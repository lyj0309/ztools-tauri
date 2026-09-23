use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    models::{AppEntry, ClipboardEntry, HistoryEntry, LauncherSettings, LocalShortcut, SyncStatus},
    state::AppState,
};

const SYNC_FILE_NAME: &str = "ztools-sync-v1.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncDocument {
    pub(crate) schema_version: u32,
    pub(crate) source_device_id: String,
    pub(crate) modified_at: i64,
    pub(crate) settings: LauncherSettings,
    pub(crate) pinned_apps: Vec<AppEntry>,
    pub(crate) history: Vec<HistoryEntry>,
    pub(crate) clipboard: Vec<ClipboardEntry>,
    pub(crate) local_shortcuts: Vec<LocalShortcut>,
}

/// 使用共享目录执行一次双向数据同步并返回最终状态。
pub(crate) fn perform_sync(state: &AppState) -> Result<SyncStatus, String> {
    state.set_sync_status(SyncStatus {
        state: "syncing".to_owned(),
        message: "正在同步".to_owned(),
        last_synced_at: state.sync_status().last_synced_at,
    });

    let result = perform_sync_inner(state);
    let status = match result {
        Ok(timestamp) => SyncStatus {
            state: "success".to_owned(),
            message: "数据已同步".to_owned(),
            last_synced_at: Some(timestamp),
        },
        Err(error) => {
            let status = SyncStatus {
                state: "error".to_owned(),
                message: error.clone(),
                last_synced_at: state.sync_status().last_synced_at,
            };
            state.set_sync_status(status);
            return Err(error);
        }
    };
    state.set_sync_status(status.clone());
    Ok(status)
}

/// 完成文件读取、合并、数据库替换和原子发布。
fn perform_sync_inner(state: &AppState) -> Result<i64, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?;
    let settings = store.settings()?;
    if !settings.sync_enabled {
        return Err("请先在设置中启用同步".to_owned());
    }
    if settings.sync_directory.trim().is_empty() {
        return Err("同步目录不能为空".to_owned());
    }

    let directory = absolute_directory(&settings.sync_directory)?;
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建同步目录：{error}"))?;
    let path = directory.join(SYNC_FILE_NAME);
    let local = export_document(&store)?;
    let merged = if path.exists() {
        let raw =
            fs::read_to_string(&path).map_err(|error| format!("无法读取同步文件：{error}"))?;
        let remote: SyncDocument =
            serde_json::from_str(&raw).map_err(|error| format!("同步文件格式无效：{error}"))?;
        if remote.schema_version != 1 {
            return Err(format!("不支持同步格式版本 {}", remote.schema_version));
        }
        merge_documents(local, remote)
    } else {
        local
    };

    // 数据库和同步文件都更新为同一个合并结果，避免下一轮产生回声冲突。
    store.apply_sync_document(&merged)?;
    write_document_atomically(&path, &merged)?;
    store.set_metadata("last_sync_at", &merged.modified_at.to_string())?;
    Ok(merged.modified_at)
}

/// 从 SQLite 导出一个不含账号凭据的同步文档。
fn export_document(store: &crate::storage::Store) -> Result<SyncDocument, String> {
    let device_id = match store.metadata("device_id")? {
        Some(value) => value,
        None => {
            let value = format!(
                "device-{:016x}",
                stable_hash(&format!(
                    "{}:{}:{}",
                    std::env::consts::OS,
                    std::process::id(),
                    now_millis()
                ))
            );
            store.set_metadata("device_id", &value)?;
            value
        }
    };
    let mut modified_at = store.modified_at()?;
    if modified_at == 0 {
        // 首次同步必须把基准时间写回数据库，否则每次导出都会伪装成新的本地修改。
        modified_at = now_millis();
        store.set_metadata("modified_at", &modified_at.to_string())?;
    }
    Ok(SyncDocument {
        schema_version: 1,
        source_device_id: device_id,
        modified_at,
        settings: store.settings()?,
        pinned_apps: store.pinned_apps()?,
        history: store.history(500)?,
        clipboard: store.clipboard_history(100)?,
        local_shortcuts: store.local_shortcuts()?,
    })
}

/// 合并两个设备快照；配置类数据采用较新快照，历史类数据做并集。
fn merge_documents(local: SyncDocument, remote: SyncDocument) -> SyncDocument {
    let local_is_newer = local.modified_at >= remote.modified_at;
    let (winner, older) = if local_is_newer {
        (local, remote)
    } else {
        (remote, local)
    };
    let mut result = winner.clone();
    result.history = merge_history(winner.history, older.history);
    result.clipboard = merge_clipboard(winner.clipboard, older.clipboard);
    result.modified_at = winner.modified_at.max(older.modified_at);
    result
}

/// 按应用合并最近启动记录并保留最新 500 项。
fn merge_history(primary: Vec<HistoryEntry>, secondary: Vec<HistoryEntry>) -> Vec<HistoryEntry> {
    let mut entries = HashMap::new();
    for entry in primary.into_iter().chain(secondary) {
        entries
            .entry(entry.app_id.clone())
            .and_modify(|current: &mut HistoryEntry| {
                if entry.launched_at > current.launched_at {
                    *current = entry.clone();
                }
            })
            .or_insert(entry);
    }
    let mut values: Vec<_> = entries.into_values().collect();
    values.sort_by_key(|entry| std::cmp::Reverse(entry.launched_at));
    values.truncate(500);
    values
}

/// 按文本内容合并剪贴板记录并保留最新 100 项。
fn merge_clipboard(
    primary: Vec<ClipboardEntry>,
    secondary: Vec<ClipboardEntry>,
) -> Vec<ClipboardEntry> {
    let mut entries = HashMap::new();
    for entry in primary.into_iter().chain(secondary) {
        let key = stable_hash(&entry.content);
        entries
            .entry(key)
            .and_modify(|current: &mut ClipboardEntry| {
                if entry.captured_at > current.captured_at {
                    *current = entry.clone();
                }
            })
            .or_insert(entry);
    }
    let mut values: Vec<_> = entries.into_values().collect();
    values.sort_by_key(|entry| std::cmp::Reverse(entry.captured_at));
    values.truncate(100);
    values
}

/// 将同步文档写入临时文件后原子替换正式文件。
fn write_document_atomically(path: &Path, document: &SyncDocument) -> Result<(), String> {
    let raw = serde_json::to_vec_pretty(document).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, raw).map_err(|error| format!("无法写入同步临时文件：{error}"))?;
    fs::rename(&temporary, path).map_err(|error| format!("无法发布同步文件：{error}"))?;
    Ok(())
}

/// 解析并校验用户配置的绝对同步目录。
fn absolute_directory(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value.trim());
    if !path.is_absolute() {
        return Err("同步目录必须使用绝对路径".to_owned());
    }
    if path.exists() && !path.is_dir() {
        return Err("同步路径不是目录".to_owned());
    }
    Ok(path)
}

/// 返回当前 Unix 毫秒时间戳。
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

/// 使用稳定 FNV-1a 算法生成跨设备内容标识。
fn stable_hash(value: &str) -> u64 {
    value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

#[cfg(test)]
mod tests {
    use super::{merge_documents, perform_sync, SyncDocument, SYNC_FILE_NAME};
    use crate::models::{ClipboardEntry, HistoryEntry, LauncherSettings};
    use crate::{state::AppState, storage::Store};
    use std::fs;

    /// 验证双向合并保留两台设备各自新增的历史，同时使用较新设置。
    #[test]
    fn merges_history_and_uses_newer_preferences() {
        let older_settings = LauncherSettings {
            theme: "light".to_owned(),
            ..LauncherSettings::default()
        };
        let newer_settings = LauncherSettings {
            theme: "dark".to_owned(),
            ..LauncherSettings::default()
        };
        let older = document(100, older_settings, "old-app", "old text");
        let newer = document(200, newer_settings, "new-app", "new text");

        let merged = merge_documents(older, newer);
        assert_eq!(merged.settings.theme, "dark");
        assert_eq!(merged.history.len(), 2);
        assert_eq!(merged.clipboard.len(), 2);
    }

    /**
     * 验证独立数据库的双向同步，并在清理前释放 Windows 文件句柄。
     * @returns 无返回值。
     */
    #[test]
    fn synchronizes_two_device_databases() {
        let root = std::env::temp_dir().join(format!(
            "ztools-sync-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let shared = root.join("shared");
        let state_a = state_for_sync(&root.join("a.sqlite3"), &shared);
        let state_b = state_for_sync(&root.join("b.sqlite3"), &shared);

        // 两台设备先后写入不同内容，再让第一台拉取第二台发布的合并快照。
        {
            let store = state_a.store.lock().expect("device A store should lock");
            store
                .capture_clipboard("from device A", 100)
                .expect("device A clipboard should save");
            store
                .touch_modified(100)
                .expect("device A timestamp should save");
        }
        perform_sync(&state_a).expect("device A should publish");
        {
            let store = state_b.store.lock().expect("device B store should lock");
            store
                .capture_clipboard("from device B", 200)
                .expect("device B clipboard should save");
            store
                .touch_modified(200)
                .expect("device B timestamp should save");
        }
        perform_sync(&state_b).expect("device B should merge and publish");
        perform_sync(&state_a).expect("device A should pull merged snapshot");

        for state in [&state_a, &state_b] {
            let contents: Vec<_> = state
                .store
                .lock()
                .expect("store should lock")
                .clipboard_history(10)
                .expect("clipboard should load")
                .into_iter()
                .map(|entry| entry.content)
                .collect();
            assert!(contents.contains(&"from device A".to_owned()));
            assert!(contents.contains(&"from device B".to_owned()));
        }
        assert!(shared.join(SYNC_FILE_NAME).is_file());
        // Windows 不允许删除仍被 SQLite 持有的数据库文件。
        drop(state_a);
        drop(state_b);
        fs::remove_dir_all(root).expect("sync fixture should be removable");
    }

    /// 创建启用共享目录同步的独立设备状态。
    fn state_for_sync(database: &std::path::Path, shared: &std::path::Path) -> AppState {
        let store = Store::open(database).expect("sync database should open");
        let settings = LauncherSettings {
            sync_enabled: true,
            sync_directory: shared.to_string_lossy().into_owned(),
            ..LauncherSettings::default()
        };
        store
            .save_settings(&settings)
            .expect("sync settings should save");
        AppState::new(store)
    }

    /// 构造最小同步文档供合并边界测试使用。
    fn document(
        modified_at: i64,
        settings: LauncherSettings,
        app_id: &str,
        clipboard: &str,
    ) -> SyncDocument {
        SyncDocument {
            schema_version: 1,
            source_device_id: app_id.to_owned(),
            modified_at,
            settings,
            pinned_apps: Vec::new(),
            history: vec![HistoryEntry {
                app_id: app_id.to_owned(),
                name: app_id.to_owned(),
                path: format!("/{app_id}"),
                launched_at: modified_at,
            }],
            clipboard: vec![ClipboardEntry {
                id: modified_at,
                content: clipboard.to_owned(),
                captured_at: modified_at,
            }],
            local_shortcuts: Vec::new(),
        }
    }
}
