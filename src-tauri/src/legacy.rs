use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use heed::{types::Str, Database, EnvFlags, EnvOpenOptions};
use serde::Serialize;
use serde_json::Value;

use crate::{
    launcher,
    models::{AppEntry, LauncherSettings, LocalShortcut},
    state::AppState,
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LegacyImportReport {
    pub(crate) source_paths: Vec<String>,
    pub(crate) imported_settings: bool,
    pub(crate) imported_pins: usize,
    pub(crate) imported_history: usize,
    pub(crate) imported_shortcuts: usize,
    pub(crate) skipped_documents: usize,
}

/// 返回当前系统中存在的 Electron ZTools LMDB 候选目录。
pub(crate) fn detect_candidates() -> Vec<String> {
    default_roots()
        .into_iter()
        .filter(|path| !resolve_lmdb_sources(path).is_empty())
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

/// 只读扫描旧 LMDB 并把宿主设置、收藏、历史和本地启动项导入 SQLite。
pub(crate) fn import(path: String, state: &AppState) -> Result<LegacyImportReport, String> {
    let root = PathBuf::from(path.trim());
    if !root.is_absolute() || !root.exists() {
        return Err("旧数据路径必须是存在的绝对目录".to_owned());
    }
    let sources = resolve_lmdb_sources(&root);
    if sources.is_empty() {
        return Err("目录中没有找到 LMDB data.mdb".to_owned());
    }

    let mut documents = HashMap::new();
    let mut skipped_documents = 0;
    for source in &sources {
        for (key, value) in read_main_database(source)? {
            match serde_json::from_str::<Value>(&value) {
                Ok(document) => {
                    documents.insert(key, document);
                }
                Err(_) => skipped_documents += 1,
            }
        }
    }

    let store = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?;
    let imported_settings = if let Some(data) = document_data(&documents, "ZTOOLS/settings-general")
    {
        let settings = migrate_settings(store.settings()?, data);
        store.save_settings(&settings)?;
        true
    } else {
        false
    };

    let imported_shortcuts = document_data(&documents, "ZTOOLS/local-shortcuts")
        .and_then(Value::as_array)
        .map(|items| import_shortcuts(&store, items))
        .transpose()?
        .unwrap_or_default();
    let imported_pins = document_data(&documents, "ZTOOLS/pinned-commands")
        .and_then(Value::as_array)
        .map(|items| import_pins(&store, items))
        .transpose()?
        .unwrap_or_default();
    let imported_history = document_data(&documents, "ZTOOLS/command-history")
        .and_then(Value::as_array)
        .map(|items| import_history(&store, items))
        .transpose()?
        .unwrap_or_default();

    Ok(LegacyImportReport {
        source_paths: sources
            .into_iter()
            .map(|source| source.to_string_lossy().into_owned())
            .collect(),
        imported_settings,
        imported_pins,
        imported_history,
        imported_shortcuts,
        skipped_documents,
    })
}

/// 打开单个 LMDB 环境并读取 main 命名数据库中的字符串键值。
fn read_main_database(path: &PathBuf) -> Result<Vec<(String, String)>, String> {
    match read_lmdb_v1(path) {
        Ok(values) => Ok(values),
        Err(v1_error) => crate::legacy_lmdb_v2::read_main_database(path).map_err(|v2_error| {
            format!(
                "无法读取 {}（LMDB v1: {v1_error}；LMDB v2: {v2_error}）",
                path.display()
            )
        }),
    }
}

/// 使用标准 LMDB v1 读取器遍历 main 命名数据库。
fn read_lmdb_v1(path: &PathBuf) -> Result<Vec<(String, String)>, String> {
    let environment = unsafe {
        EnvOpenOptions::new()
            .max_dbs(16)
            .flags(EnvFlags::READ_ONLY)
            .open(path)
    }
    .map_err(|error| format!("无法只读打开 {}：{error}", path.display()))?;
    let transaction = environment
        .read_txn()
        .map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
    let database: Database<Str, Str> = environment
        .open_database(&transaction, Some("main"))
        .map_err(|error| format!("无法打开 main 数据库：{error}"))?
        .ok_or_else(|| format!("{} 缺少 main 数据库", path.display()))?;
    let iterator = database
        .iter(&transaction)
        .map_err(|error| format!("无法遍历旧数据库：{error}"))?;
    iterator
        .map(|item| {
            item.map(|(key, value)| (key.to_owned(), value.to_owned()))
                .map_err(|error| error.to_string())
        })
        .collect()
}

/// 解析用户选择目录下所有可能的新旧版 LMDB 数据空间。
fn resolve_lmdb_sources(root: &Path) -> Vec<PathBuf> {
    let candidates = [
        root.to_path_buf(),
        root.join("lmdb"),
        root.join("lmdb/device"),
        root.join("lmdb/accounts/default"),
        root.join("device"),
        root.join("accounts/default"),
    ];
    let mut result = Vec::new();
    for candidate in candidates {
        if candidate.join("data.mdb").is_file() && !result.contains(&candidate) {
            result.push(candidate);
        }
    }
    result
}

/// 生成各平台 Electron 版的默认数据目录候选。
fn default_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    #[cfg(target_os = "macos")]
    let roots = vec![
        home.join(".ztools"),
        home.join("Library/Application Support/ZTools"),
    ];
    #[cfg(target_os = "windows")]
    let roots = vec![
        home.join(".ztools"),
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
            .join("ZTools"),
    ];
    #[cfg(target_os = "linux")]
    let roots = vec![home.join(".ztools"), home.join(".config/ZTools")];
    roots
}

/// 获取旧文档 data 字段，兼容裸值测试夹具。
fn document_data<'a>(documents: &'a HashMap<String, Value>, key: &str) -> Option<&'a Value> {
    documents
        .get(key)
        .and_then(|document| document.get("data").or(Some(document)))
}

/// 将旧版通用设置的可复用字段映射到新模型。
fn migrate_settings(mut settings: LauncherSettings, data: &Value) -> LauncherSettings {
    if let Some(value) = data.get("theme").and_then(Value::as_str) {
        if matches!(value, "system" | "light" | "dark") {
            settings.theme = value.to_owned();
        }
    }
    if let Some(value) = data.get("customColor").and_then(Value::as_str) {
        if is_hex_color(value) {
            settings.accent_color = value.to_owned();
        }
    }
    if let Some(value) = data.get("showRecentInSearch").and_then(Value::as_bool) {
        settings.show_recent = value;
    }
    if let Some(value) = data.get("autoCheckUpdate").and_then(Value::as_bool) {
        settings.auto_check_updates = value;
    }
    if let Some(value) = data.get("clipboardRetentionDays").and_then(Value::as_u64) {
        settings.clipboard_retention_days = value.clamp(1, 3650) as u32;
    }
    settings
}

/// 导入旧版本地启动项并保留稳定路径标识。
fn import_shortcuts(store: &crate::storage::Store, items: &[Value]) -> Result<usize, String> {
    let mut imported = 0;
    for item in items {
        let Some(path) = item.get("path").and_then(Value::as_str) else {
            continue;
        };
        if !PathBuf::from(path).exists() {
            continue;
        }
        let shortcut = LocalShortcut {
            id: launcher::local_shortcut_id(path),
            name: item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("本地项目")
                .to_owned(),
            alias: item
                .get("alias")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            path: path.to_owned(),
            kind: item
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("file")
                .to_owned(),
            added_at: item
                .get("addedAt")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
        };
        store.add_local_shortcut(&shortcut)?;
        imported += 1;
    }
    Ok(imported)
}

/// 导入仍可对应文件路径的旧版收藏项。
fn import_pins(store: &crate::storage::Store, items: &[Value]) -> Result<usize, String> {
    let mut imported = 0;
    let local_paths: std::collections::HashSet<_> = store
        .local_shortcuts()?
        .into_iter()
        .map(|shortcut| shortcut.path)
        .collect();
    for item in items {
        let Some(path) = item.get("path").and_then(Value::as_str) else {
            continue;
        };
        if item.get("type").and_then(Value::as_str) == Some("plugin") {
            continue;
        }
        let app = AppEntry {
            id: if local_paths.contains(path) {
                launcher::local_shortcut_id(path)
            } else {
                legacy_app_id(path)
            },
            name: item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("应用")
                .to_owned(),
            path: path.to_owned(),
            keywords: Vec::new(),
            source: "legacy-import".to_owned(),
        };
        store.set_pinned(&app, true)?;
        imported += 1;
    }
    Ok(imported)
}

/// 导入旧版启动历史并跳过插件指令。
fn import_history(store: &crate::storage::Store, items: &[Value]) -> Result<usize, String> {
    let mut imported = 0;
    let local_paths: std::collections::HashSet<_> = store
        .local_shortcuts()?
        .into_iter()
        .map(|shortcut| shortcut.path)
        .collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default();
    for (index, item) in items.iter().rev().enumerate() {
        let Some(path) = item.get("path").and_then(Value::as_str) else {
            continue;
        };
        if item.get("type").and_then(Value::as_str) == Some("plugin") {
            continue;
        }
        let app = AppEntry {
            id: if local_paths.contains(path) {
                launcher::local_shortcut_id(path)
            } else {
                legacy_app_id(path)
            },
            name: item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("应用")
                .to_owned(),
            path: path.to_owned(),
            keywords: Vec::new(),
            source: "legacy-import".to_owned(),
        };
        store.record_launch(&app, now.saturating_sub(index as i64))?;
        imported += 1;
    }
    Ok(imported)
}

/// 为旧版应用路径生成与新扫描器相同形式的稳定标识。
fn legacy_app_id(path: &str) -> String {
    let hash = path
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("app-{hash:016x}")
}

/// 判断旧设置的颜色是否为可迁移的六位十六进制颜色。
fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::import;
    use crate::{launcher, legacy_lmdb_v2, state::AppState, storage::Store};
    use serde_json::json;
    use std::fs;

    /// 验证真实 LMDB v2 文件能完整迁移宿主设置、快捷项、收藏和历史。
    #[test]
    fn imports_v2_host_documents_without_touching_source() {
        let root = std::env::temp_dir().join(format!(
            "ztools-legacy-v2-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let legacy_path = root.join("legacy/lmdb");
        let target_path = root.join("new/ztools.sqlite3");
        let shortcut_path = root.join("legacy-file.txt");
        fs::create_dir_all(&root).expect("legacy fixture root should exist");
        fs::write(&shortcut_path, "legacy data").expect("legacy shortcut should exist");
        let shortcut = shortcut_path.to_string_lossy().into_owned();
        let settings = json!({
            "data": {
                "theme": "light",
                "customColor": "#336699",
                "showRecentInSearch": false,
                "clipboardRetentionDays": 45
            }
        })
        .to_string();
        let shortcuts = json!({
            "data": [{"name": "Legacy File", "alias": "旧文件", "path": shortcut, "type": "file", "addedAt": 12}]
        })
        .to_string();
        let pins = json!({
            "data": [{"name": "Legacy File", "path": shortcut_path, "type": "file"}]
        })
        .to_string();
        let history = json!({
            "data": [{"name": "Legacy File", "path": shortcut_path, "type": "file"}]
        })
        .to_string();
        legacy_lmdb_v2::write_test_database(
            &legacy_path,
            &[
                ("ZTOOLS/settings-general", &settings),
                ("ZTOOLS/local-shortcuts", &shortcuts),
                ("ZTOOLS/pinned-commands", &pins),
                ("ZTOOLS/command-history", &history),
            ],
        )
        .expect("LMDB v2 fixture should be created");
        let source_size = fs::metadata(legacy_path.join("data.mdb"))
            .expect("source database should exist")
            .len();

        let state = AppState::new(Store::open(&target_path).expect("target store should open"));
        let report = import(root.join("legacy").to_string_lossy().into_owned(), &state)
            .expect("legacy import should succeed");
        assert!(report.imported_settings);
        assert_eq!(report.imported_shortcuts, 1);
        assert_eq!(report.imported_pins, 1);
        assert_eq!(report.imported_history, 1);
        let store = state.store.lock().expect("target store should lock");
        assert_eq!(
            store.settings().expect("settings should load").theme,
            "light"
        );
        assert_eq!(
            store.pinned_ids().expect("pins should load"),
            [launcher::local_shortcut_id(&shortcut)]
        );
        assert_eq!(
            store.history(1).expect("history should load")[0].app_id,
            launcher::local_shortcut_id(&shortcut)
        );
        assert_eq!(
            fs::metadata(legacy_path.join("data.mdb"))
                .expect("source database should remain")
                .len(),
            source_size
        );
        drop(store);
        drop(state);
        fs::remove_dir_all(root).expect("legacy fixture should be removable");
    }
}
