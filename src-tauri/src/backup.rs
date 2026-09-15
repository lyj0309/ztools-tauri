use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

use crate::{plugin, storage::Store};

const BACKUP_SCHEMA_VERSION: u32 = 1;
const MAX_BACKUP_FILES: usize = 25_000;
const MAX_BACKUP_BYTES: u64 = 1_024 * 1_024 * 1_024;
const DATABASE_ENTRY: &str = "database/ztools.sqlite3";
const MANIFEST_ENTRY: &str = "manifest.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest {
    schema_version: u32,
    host_version: String,
    created_at: i64,
    database_sha256: String,
    plugin_count: usize,
    plugin_file_count: usize,
    plugin_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackupReport {
    pub(crate) path: String,
    pub(crate) created_at: i64,
    pub(crate) plugin_count: usize,
    pub(crate) plugin_file_count: usize,
    pub(crate) total_bytes: u64,
    pub(crate) archive_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RestoreReport {
    pub(crate) path: String,
    pub(crate) restored_at: i64,
    pub(crate) plugin_count: usize,
    pub(crate) plugin_file_count: usize,
}

/// 创建包含一致性 SQLite 快照和正式插件文件的可恢复归档。
pub(crate) fn create(
    destination: &Path,
    store: &Store,
    plugin_root: &Path,
) -> Result<BackupReport, String> {
    let destination = absolute_backup_path(destination)?;
    let parent = destination
        .parent()
        .ok_or_else(|| "备份文件缺少父目录".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建备份目录：{error}"))?;
    let nonce = now_millis();
    let database_copy = parent.join(format!(".ztools-database-{nonce}.tmp"));
    let archive_copy = parent.join(format!(".ztools-backup-{nonce}.tmp"));

    let result = (|| {
        // 先通过 SQLite 在线备份接口固定数据库快照，避免直接复制 WAL 数据库。
        store.backup_to(&database_copy)?;
        let database_hash = hash_file(&database_copy)?;
        let database_bytes = fs::metadata(&database_copy)
            .map_err(|error| error.to_string())?
            .len();
        let plugin_files = collect_plugin_files(plugin_root)?;
        let plugin_bytes = plugin_files.iter().try_fold(0_u64, |total, (path, _)| {
            let length = fs::metadata(path).map_err(|error| error.to_string())?.len();
            total
                .checked_add(length)
                .ok_or_else(|| "插件备份体积溢出".to_owned())
        })?;
        if database_bytes.saturating_add(plugin_bytes) > MAX_BACKUP_BYTES {
            return Err("备份内容超过 1 GB 限制".to_owned());
        }
        let plugin_count = plugin::validate_plugin_root(plugin_root)?;
        let manifest = BackupManifest {
            schema_version: BACKUP_SCHEMA_VERSION,
            host_version: env!("CARGO_PKG_VERSION").to_owned(),
            created_at: nonce,
            database_sha256: database_hash,
            plugin_count,
            plugin_file_count: plugin_files.len(),
            plugin_bytes,
        };

        // 归档先写入同目录临时文件，全部写完并同步后才发布正式文件。
        write_archive(&archive_copy, &manifest, &database_copy, &plugin_files)?;
        if destination.exists() {
            fs::remove_file(&destination).map_err(|error| format!("无法覆盖旧备份：{error}"))?;
        }
        fs::rename(&archive_copy, &destination)
            .map_err(|error| format!("无法发布备份文件：{error}"))?;
        let archive_sha256 = hash_file(&destination)?;
        let total_bytes = fs::metadata(&destination)
            .map_err(|error| error.to_string())?
            .len();
        Ok(BackupReport {
            path: destination.to_string_lossy().into_owned(),
            created_at: manifest.created_at,
            plugin_count,
            plugin_file_count: plugin_files.len(),
            total_bytes,
            archive_sha256,
        })
    })();

    // 成功和失败都清理未发布的中间文件。
    let _ = fs::remove_file(database_copy);
    let _ = fs::remove_file(archive_copy);
    result
}

/// 校验并恢复备份；插件目录和数据库任一失败时回滚两者。
pub(crate) fn restore(
    source: &Path,
    store: &mut Store,
    plugin_root: &Path,
) -> Result<RestoreReport, String> {
    let source = source
        .canonicalize()
        .map_err(|error| format!("备份文件不存在：{error}"))?;
    if !source.is_file() {
        return Err("备份路径不是文件".to_owned());
    }
    let parent = plugin_root
        .parent()
        .ok_or_else(|| "插件目录缺少父目录".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let nonce = now_millis();
    let staging = parent.join(format!(".restore-staging-{nonce}"));
    let previous_plugins = parent.join(format!(".restore-plugins-{nonce}"));
    let previous_database = parent.join(format!(".restore-database-{nonce}.sqlite3"));

    let result = (|| {
        fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
        let manifest = extract_archive(&source, &staging)?;
        let database = staging.join(DATABASE_ENTRY);
        if hash_file(&database)? != manifest.database_sha256 {
            return Err("备份数据库哈希不匹配".to_owned());
        }
        let staged_plugins = staging.join("plugins");
        fs::create_dir_all(&staged_plugins).map_err(|error| error.to_string())?;
        let plugin_count = plugin::validate_plugin_root(&staged_plugins)?;
        if plugin_count != manifest.plugin_count {
            return Err("备份插件数量与清单不一致".to_owned());
        }

        // 在修改任何正式数据前保存恢复点。
        store.backup_to(&previous_database)?;
        if plugin_root.exists() {
            fs::rename(plugin_root, &previous_plugins)
                .map_err(|error| format!("无法暂存当前插件：{error}"))?;
        }
        if let Err(error) = fs::rename(&staged_plugins, plugin_root) {
            if previous_plugins.exists() {
                let _ = fs::rename(&previous_plugins, plugin_root);
            }
            return Err(format!("无法恢复插件目录：{error}"));
        }

        if let Err(error) = store.restore_from(&database) {
            // 数据库恢复失败时同时撤销插件目录替换。
            let _ = fs::remove_dir_all(plugin_root);
            if previous_plugins.exists() {
                let _ = fs::rename(&previous_plugins, plugin_root);
            }
            let rollback = store.restore_from(&previous_database);
            return match rollback {
                Ok(()) => Err(format!("恢复数据库失败，已回滚：{error}")),
                Err(rollback_error) => Err(format!(
                    "恢复数据库失败且回滚失败：{error}；回滚错误：{rollback_error}"
                )),
            };
        }
        if previous_plugins.exists() {
            if let Err(error) = fs::remove_dir_all(&previous_plugins) {
                eprintln!(
                    "[backup] unable to remove previous plugin directory {}: {error}",
                    previous_plugins.display()
                );
            }
        }
        Ok(RestoreReport {
            path: source.to_string_lossy().into_owned(),
            restored_at: now_millis(),
            plugin_count,
            plugin_file_count: manifest.plugin_file_count,
        })
    })();

    // staging 和数据库恢复点不进入正式应用目录。
    let _ = fs::remove_dir_all(staging);
    let _ = fs::remove_file(previous_database);
    result
}

/// 收集非隐藏正式插件文件并生成使用正斜线的归档路径。
fn collect_plugin_files(root: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| error.to_string())?;
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| error.to_string())?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        if relative
            .components()
            .next()
            .is_some_and(|component| component.as_os_str().to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if entry.file_type().is_symlink() {
            return Err(format!(
                "插件备份不接受符号链接：{}",
                entry.path().display()
            ));
        }
        if entry.file_type().is_file() {
            let relative = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            files.push((entry.path().to_path_buf(), format!("plugins/{relative}")));
            if files.len() > MAX_BACKUP_FILES {
                return Err(format!("备份文件数不能超过 {MAX_BACKUP_FILES}"));
            }
        }
    }
    files.sort_by(|left, right| left.1.cmp(&right.1));
    Ok(files)
}

/// 写入备份清单、SQLite 快照和插件文件。
fn write_archive(
    path: &Path,
    manifest: &BackupManifest,
    database: &Path,
    plugin_files: &[(PathBuf, String)],
) -> Result<(), String> {
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut archive = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o600);
    archive
        .start_file(MANIFEST_ENTRY, options)
        .map_err(|error| error.to_string())?;
    archive
        .write_all(&serde_json::to_vec_pretty(manifest).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    append_file(&mut archive, DATABASE_ENTRY, database, options)?;
    for (source, name) in plugin_files {
        append_file(&mut archive, name, source, options)?;
    }
    let file = archive.finish().map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

/// 把一个磁盘文件流式追加到 ZIP，避免整份插件读入内存。
fn append_file(
    archive: &mut ZipWriter<File>,
    name: &str,
    source: &Path,
    options: SimpleFileOptions,
) -> Result<(), String> {
    archive
        .start_file(name, options)
        .map_err(|error| error.to_string())?;
    let mut input = File::open(source).map_err(|error| error.to_string())?;
    std::io::copy(&mut input, archive).map_err(|error| error.to_string())?;
    Ok(())
}

/// 安全解压恢复归档并返回已校验清单。
fn extract_archive(source: &Path, staging: &Path) -> Result<BackupManifest, String> {
    let file = File::open(source).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|error| format!("备份格式无效：{error}"))?;
    if archive.len() > MAX_BACKUP_FILES + 2 {
        return Err("备份包含过多文件".to_owned());
    }
    let mut total = 0_u64;
    let mut plugin_file_count = 0_usize;
    let mut plugin_bytes = 0_u64;
    let mut names = HashSet::new();
    let mut manifest = None;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let name = entry.name().to_owned();
        if !names.insert(name.clone()) {
            return Err(format!("备份包含重复路径：{name}"));
        }
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| format!("备份包含不安全路径：{name}"))?
            .to_path_buf();
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(format!("备份包含符号链接：{name}"));
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| "备份解压体积溢出".to_owned())?;
        if total > MAX_BACKUP_BYTES {
            return Err("备份解压后超过 1 GB 限制".to_owned());
        }
        if name == MANIFEST_ENTRY {
            let mut raw = Vec::new();
            entry
                .read_to_end(&mut raw)
                .map_err(|error| error.to_string())?;
            let value: BackupManifest =
                serde_json::from_slice(&raw).map_err(|error| format!("备份清单无效：{error}"))?;
            if value.schema_version != BACKUP_SCHEMA_VERSION {
                return Err(format!("不支持备份格式版本 {}", value.schema_version));
            }
            manifest = Some(value);
            continue;
        }
        if name != DATABASE_ENTRY && !name.starts_with("plugins/") {
            return Err(format!("备份包含未知文件：{name}"));
        }
        if name.starts_with("plugins/") && !entry.is_dir() {
            plugin_file_count += 1;
            plugin_bytes = plugin_bytes
                .checked_add(entry.size())
                .ok_or_else(|| "插件解压体积溢出".to_owned())?;
        }
        let target = staging.join(enclosed);
        if entry.is_dir() {
            fs::create_dir_all(target).map_err(|error| error.to_string())?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            let mut output = File::create(target).map_err(|error| error.to_string())?;
            std::io::copy(&mut entry, &mut output).map_err(|error| error.to_string())?;
        }
    }
    if !staging.join(DATABASE_ENTRY).is_file() {
        return Err("备份缺少数据库快照".to_owned());
    }
    let manifest = manifest.ok_or_else(|| "备份缺少清单".to_owned())?;
    if plugin_file_count != manifest.plugin_file_count || plugin_bytes != manifest.plugin_bytes {
        return Err("备份插件文件统计与清单不一致".to_owned());
    }
    Ok(manifest)
}

/// 规范用户选择的备份目标并补全默认扩展名。
fn absolute_backup_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("备份路径必须是绝对路径".to_owned());
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err("备份路径不能包含相对跳转".to_owned());
    }
    let mut value = path.to_path_buf();
    if value.extension().and_then(|extension| extension.to_str()) != Some("ztools-backup") {
        value.set_extension("ztools-backup");
    }
    Ok(value)
}

/// 计算文件 SHA-256，供数据库和最终归档核验。
fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let length = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// 返回当前 Unix 毫秒时间戳供临时目录和报告使用。
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{create, restore};
    use crate::{models::LauncherSettings, storage::Store};
    use std::fs;

    /// 验证完整备份能恢复 SQLite 内容和被替换的插件目录。
    #[test]
    fn restores_database_and_plugins() {
        let root = std::env::temp_dir().join(format!(
            "ztools-backup-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let plugin_root = root.join("plugins");
        let plugin = plugin_root.join("fixture");
        fs::create_dir_all(&plugin).expect("plugin directory should exist");
        fs::write(
            plugin.join("plugin.json"),
            r#"{
              "name": "fixture",
              "title": "Fixture",
              "description": "Backup fixture",
              "version": "1.0.0",
              "main": "index.html",
              "features": []
            }"#,
        )
        .expect("manifest should save");
        fs::write(plugin.join("index.html"), "before backup").expect("plugin page should save");
        let database = root.join("ztools.sqlite3");
        let mut store = Store::open(&database).expect("store should open");
        let settings = LauncherSettings {
            shortcut: "Alt+X".to_owned(),
            ..LauncherSettings::default()
        };
        store
            .save_settings(&settings)
            .expect("settings should save");
        store
            .capture_clipboard("backup content", 1234)
            .expect("clipboard should save");
        let archive = root.join("fixture.ztools-backup");

        let report = create(&archive, &store, &plugin_root).expect("backup should succeed");
        assert_eq!(report.plugin_count, 1);
        store
            .save_settings(&LauncherSettings::default())
            .expect("settings should mutate");
        fs::remove_dir_all(&plugin_root).expect("plugin should be removed");

        let restored = restore(&archive, &mut store, &plugin_root).expect("restore should succeed");
        assert_eq!(restored.plugin_count, 1);
        assert_eq!(
            store.settings().expect("settings should load").shortcut,
            "Alt+X"
        );
        assert_eq!(
            store.clipboard_history(10).expect("clipboard should load")[0].content,
            "backup content"
        );
        assert_eq!(
            fs::read_to_string(plugin_root.join("fixture/index.html"))
                .expect("plugin should restore"),
            "before backup"
        );
        drop(store);
        fs::remove_dir_all(root).expect("test directory should be removable");
    }
}
