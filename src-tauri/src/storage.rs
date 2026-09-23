use std::{fs, path::Path, time::Duration};

use rusqlite::{backup::Backup, params, Connection};

use crate::models::{AppEntry, ClipboardEntry, HistoryEntry, LauncherSettings, LocalShortcut};
use crate::sync::SyncDocument;

const CLIPBOARD_IMAGE_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS clipboard_images (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   content_hash TEXT NOT NULL UNIQUE,
                   width INTEGER NOT NULL,
                   height INTEGER NOT NULL,
                   png BLOB NOT NULL,
                   thumbnail TEXT NOT NULL,
                   captured_at INTEGER NOT NULL
                 );";

pub(crate) struct Store {
    connection: Connection,
}

impl Store {
    /**
     * 打开 SQLite 数据库并补建文本和图片历史等应用表。
     * @param path 数据库文件路径。
     * @returns 可用的数据存储或初始化错误。
     */
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        // 确保首次启动时应用数据目录已经存在。
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS launch_history (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   app_id TEXT NOT NULL,
                   name TEXT NOT NULL,
                   path TEXT NOT NULL,
                   launched_at INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_launch_history_time
                   ON launch_history(launched_at DESC);
                 CREATE TABLE IF NOT EXISTS pinned_apps (
                   app_id TEXT PRIMARY KEY,
                   name TEXT NOT NULL,
                   path TEXT NOT NULL,
                   position INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS settings (
                   id INTEGER PRIMARY KEY CHECK (id = 1),
                   value TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS clipboard_history (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   content_hash TEXT NOT NULL UNIQUE,
                   content TEXT NOT NULL,
                   captured_at INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS local_shortcuts (
                   id TEXT PRIMARY KEY,
                   name TEXT NOT NULL,
                   alias TEXT NOT NULL DEFAULT '',
                   path TEXT NOT NULL UNIQUE,
                   kind TEXT NOT NULL,
                   added_at INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS metadata (
                   key TEXT PRIMARY KEY,
                   value TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS plugin_documents (
                   plugin_name TEXT NOT NULL,
                   document_id TEXT NOT NULL,
                   value TEXT NOT NULL,
                   updated_at INTEGER NOT NULL,
                   PRIMARY KEY(plugin_name, document_id)
                 );
                 CREATE TABLE IF NOT EXISTS plugin_storage (
                   plugin_name TEXT NOT NULL,
                   storage_key TEXT NOT NULL,
                   value TEXT NOT NULL,
                   updated_at INTEGER NOT NULL,
                   PRIMARY KEY(plugin_name, storage_key)
                 );
                 CREATE TABLE IF NOT EXISTS plugin_features (
                   plugin_name TEXT NOT NULL,
                   feature_code TEXT NOT NULL,
                   value TEXT NOT NULL,
                   updated_at INTEGER NOT NULL,
                   PRIMARY KEY(plugin_name, feature_code)
                 );
                 CREATE TABLE IF NOT EXISTS plugin_attachments (
                   plugin_name TEXT NOT NULL,
                   attachment_id TEXT NOT NULL,
                   content_type TEXT NOT NULL,
                   data BLOB NOT NULL,
                   updated_at INTEGER NOT NULL,
                   PRIMARY KEY(plugin_name, attachment_id)
                 );",
            )
            .map_err(|error| error.to_string())?;

        connection
            .execute_batch(CLIPBOARD_IMAGE_SCHEMA)
            .map_err(|error| error.to_string())?;
        Ok(Self { connection })
    }

    /// 把当前在线数据库复制为可独立恢复的一致性 SQLite 文件。
    pub(crate) fn backup_to(&self, path: &Path) -> Result<(), String> {
        if path.exists() {
            fs::remove_file(path).map_err(|error| error.to_string())?;
        }
        let mut destination = Connection::open(path).map_err(|error| error.to_string())?;
        let backup =
            Backup::new(&self.connection, &mut destination).map_err(|error| error.to_string())?;
        backup
            .run_to_completion(128, Duration::from_millis(2), None)
            .map_err(|error| error.to_string())
    }

    /**
     * 恢复已校验的数据库，并补建旧版本缺少的图片历史表。
     * @param path 备份数据库路径。
     * @returns 恢复与兼容迁移结果。
     */
    pub(crate) fn restore_from(&mut self, path: &Path) -> Result<(), String> {
        let source = Connection::open(path).map_err(|error| error.to_string())?;
        source
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .eq("ok")
            .then_some(())
            .ok_or_else(|| "备份数据库完整性检查失败".to_owned())?;
        let backup =
            Backup::new(&source, &mut self.connection).map_err(|error| error.to_string())?;
        backup
            .run_to_completion(128, Duration::from_millis(2), None)
            .map_err(|error| error.to_string())?;
        // 先释放 SQLite 备份句柄，再补建旧备份中不存在的新表。
        drop(backup);
        self.connection
            .execute_batch(CLIPBOARD_IMAGE_SCHEMA)
            .map_err(|error| error.to_string())
    }

    /// 读取启动器设置，数据库尚未保存时返回默认配置。
    pub(crate) fn settings(&self) -> Result<LauncherSettings, String> {
        let mut statement = self
            .connection
            .prepare("SELECT value FROM settings WHERE id = 1")
            .map_err(|error| error.to_string())?;
        let value = statement.query_row([], |row| row.get::<_, String>(0));

        match value {
            Ok(json) => serde_json::from_str(&json).map_err(|error| error.to_string()),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(LauncherSettings::default()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// 原子保存完整启动器设置。
    pub(crate) fn save_settings(&self, settings: &LauncherSettings) -> Result<(), String> {
        let value = serde_json::to_string(settings).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO settings(id, value) VALUES(1, ?1)
                 ON CONFLICT(id) DO UPDATE SET value = excluded.value",
                [value],
            )
            .map_err(|error| error.to_string())?;
        self.touch_modified(now_millis())
    }

    /// 返回全部本地文件、目录和应用启动项。
    pub(crate) fn local_shortcuts(&self) -> Result<Vec<LocalShortcut>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, name, alias, path, kind, added_at
                 FROM local_shortcuts ORDER BY added_at, name",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(LocalShortcut {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    alias: row.get(2)?,
                    path: row.get(3)?,
                    kind: row.get(4)?,
                    added_at: row.get(5)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    /// 新增本地启动项，路径重复时返回现有记录。
    pub(crate) fn add_local_shortcut(
        &self,
        shortcut: &LocalShortcut,
    ) -> Result<LocalShortcut, String> {
        self.connection
            .execute(
                "INSERT INTO local_shortcuts(id, name, alias, path, kind, added_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(path) DO UPDATE SET
                   name = excluded.name,
                   kind = excluded.kind",
                params![
                    shortcut.id,
                    shortcut.name,
                    shortcut.alias,
                    shortcut.path,
                    shortcut.kind,
                    shortcut.added_at
                ],
            )
            .map_err(|error| error.to_string())?;
        self.touch_modified(shortcut.added_at)?;
        self.local_shortcuts()?
            .into_iter()
            .find(|item| item.path == shortcut.path)
            .ok_or_else(|| "本地启动项保存后未找到".to_owned())
    }

    /// 修改本地启动项的用户别名。
    pub(crate) fn update_local_shortcut_alias(&self, id: &str, alias: &str) -> Result<(), String> {
        let changed = self
            .connection
            .execute(
                "UPDATE local_shortcuts SET alias = ?2 WHERE id = ?1",
                params![id, alias],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err("本地启动项不存在".to_owned());
        }
        self.touch_modified(now_millis())
    }

    /// 删除指定本地启动项。
    pub(crate) fn delete_local_shortcut(&self, id: &str) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM local_shortcuts WHERE id = ?1", [id])
            .map_err(|error| error.to_string())?;
        self.touch_modified(now_millis())
    }

    /// 读取内部元数据，不存在时返回空结果。
    pub(crate) fn metadata(&self, key: &str) -> Result<Option<String>, String> {
        match self
            .connection
            .query_row("SELECT value FROM metadata WHERE key = ?1", [key], |row| {
                row.get(0)
            }) {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    /// 写入内部元数据。
    pub(crate) fn set_metadata(&self, key: &str, value: &str) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO metadata(key, value) VALUES(?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// 返回用户数据最后一次实质修改时间。
    pub(crate) fn modified_at(&self) -> Result<i64, String> {
        self.metadata("modified_at")?
            .and_then(|value| value.parse().ok())
            .map_or_else(|| Ok(0), Ok)
    }

    /// 更新用户数据修改时间，供文件同步判定新旧版本。
    pub(crate) fn touch_modified(&self, timestamp: i64) -> Result<(), String> {
        self.set_metadata("modified_at", &timestamp.to_string())
    }

    /// 记录一次成功提交给操作系统的应用启动请求。
    pub(crate) fn record_launch(&self, app: &AppEntry, timestamp: i64) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO launch_history(app_id, name, path, launched_at)
                 VALUES(?1, ?2, ?3, ?4)",
                params![app.id, app.name, app.path, timestamp],
            )
            .map_err(|error| error.to_string())?;
        // 控制历史表体积，保留最近 500 次启动即可支持排序。
        self.connection
            .execute(
                "DELETE FROM launch_history WHERE id NOT IN (
                   SELECT id FROM launch_history ORDER BY launched_at DESC LIMIT 500
                 )",
                [],
            )
            .map_err(|error| error.to_string())?;
        self.touch_modified(timestamp)
    }

    /// 返回去重后的最近应用启动记录。
    pub(crate) fn history(&self, limit: usize) -> Result<Vec<HistoryEntry>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT app_id, name, path, MAX(launched_at) AS last_launch
                 FROM launch_history
                 GROUP BY app_id, name, path
                 ORDER BY last_launch DESC
                 LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([limit as i64], |row| {
                Ok(HistoryEntry {
                    app_id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    launched_at: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    /// 清空全部本地启动历史。
    pub(crate) fn clear_history(&self) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM launch_history", [])
            .map_err(|error| error.to_string())?;
        self.touch_modified(now_millis())
    }

    /// 保存剪贴板文本并把重复内容移动到历史首位。
    pub(crate) fn capture_clipboard(&self, content: &str, timestamp: i64) -> Result<(), String> {
        let content_hash = stable_text_hash(content);
        self.connection
            .execute(
                "INSERT INTO clipboard_history(content_hash, content, captured_at)
                 VALUES(?1, ?2, ?3)
                 ON CONFLICT(content_hash) DO UPDATE SET
                   content = excluded.content,
                   captured_at = excluded.captured_at",
                params![content_hash, content, timestamp],
            )
            .map_err(|error| error.to_string())?;
        // 剪贴板可能包含大文本，严格限制本地历史条数。
        self.connection
            .execute(
                "DELETE FROM clipboard_history WHERE id NOT IN (
                   SELECT id FROM clipboard_history ORDER BY captured_at DESC LIMIT 100
                 )",
                [],
            )
            .map_err(|error| error.to_string())?;
        self.touch_modified(timestamp)?;
        Ok(())
    }

    /// 返回最近捕获的纯文本剪贴板记录。
    pub(crate) fn clipboard_history(&self, limit: usize) -> Result<Vec<ClipboardEntry>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, content, captured_at FROM clipboard_history
                 ORDER BY captured_at DESC LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([limit as i64], |row| {
                Ok(ClipboardEntry {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    captured_at: row.get(2)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    /**
     * 保存去重图片并限制历史条数及总图片体积。
     * @param image 已编码的原图、缩略图和内容哈希。
     * @param timestamp 捕获时间戳。
     * @returns 写入成功返回 Ok，否则返回数据库错误。
     */
    pub(crate) fn capture_clipboard_image(
        &self,
        image: &crate::clipboard_images::CapturedImage,
        timestamp: i64,
    ) -> Result<(), String> {
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO clipboard_images(content_hash,width,height,png,thumbnail,captured_at)
             VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(content_hash) DO UPDATE SET captured_at=excluded.captured_at",
            params![
                image.hash,
                image.width,
                image.height,
                image.png,
                image.thumbnail,
                timestamp
            ],
        )
        .map_err(|e| e.to_string())?;
        // 最多保存 50 张、合计 128 MiB 原图，避免持续截图使数据库无限增长。
        tx.execute(
            "DELETE FROM clipboard_images WHERE id NOT IN (
               SELECT id FROM (
                 SELECT id, ROW_NUMBER() OVER (ORDER BY captured_at DESC,id DESC) AS rank,
                 SUM(length(png)) OVER (ORDER BY captured_at DESC,id DESC) AS bytes
                 FROM clipboard_images
               ) WHERE rank <= 50 AND bytes <= 134217728
             )",
            [],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    /**
     * 返回图片历史元数据与小尺寸预览，原图留在数据库内。
     * @returns 按最近复制时间排序的图片列表或数据库错误。
     */
    pub(crate) fn clipboard_images(
        &self,
    ) -> Result<Vec<crate::models::ClipboardImageEntry>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id,width,height,captured_at,thumbnail FROM clipboard_images
             ORDER BY captured_at DESC,id DESC LIMIT 50",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(crate::models::ClipboardImageEntry {
                    id: row.get(0)?,
                    width: row.get(1)?,
                    height: row.get(2)?,
                    captured_at: row.get(3)?,
                    thumbnail: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /**
     * 读取指定历史图片的原始 PNG，供悬浮贴图使用。
     * @param id 图片历史的数据库标识。
     * @returns PNG 字节，记录不存在时返回错误。
     */
    pub(crate) fn clipboard_image_png(&self, id: i64) -> Result<Vec<u8>, String> {
        self.connection
            .query_row(
                "SELECT png FROM clipboard_images WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(|e| format!("历史图片不存在或已清理：{e}"))
    }

    /// 删除单条剪贴板历史记录。
    pub(crate) fn delete_clipboard_entry(&self, id: i64) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM clipboard_history WHERE id = ?1", [id])
            .map_err(|error| error.to_string())?;
        self.touch_modified(now_millis())
    }

    /**
     * 同时清空文本与图片剪贴板历史。
     * @returns 清理成功返回 Ok，否则返回数据库错误。
     */
    pub(crate) fn clear_clipboard_history(&self) -> Result<(), String> {
        // 图片和文本使用同一清理入口，避免用户清空历史后图片仍被保留。
        self.connection
            .execute_batch("DELETE FROM clipboard_history; DELETE FROM clipboard_images;")
            .map_err(|error| error.to_string())?;
        self.touch_modified(now_millis())
    }

    /// 返回指定插件的全部 JSON 文档，数据始终按插件身份隔离。
    pub(crate) fn plugin_documents(
        &self,
        plugin_name: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT value FROM plugin_documents
                 WHERE plugin_name = ?1 ORDER BY updated_at, document_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([plugin_name], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        rows.map(|row| {
            let value = row.map_err(|error| error.to_string())?;
            serde_json::from_str(&value).map_err(|error| error.to_string())
        })
        .collect()
    }

    /// 新增或替换指定插件的 JSON 文档。
    pub(crate) fn put_plugin_document(
        &self,
        plugin_name: &str,
        document_id: &str,
        document: &serde_json::Value,
    ) -> Result<(), String> {
        let value = serde_json::to_string(document).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO plugin_documents(plugin_name, document_id, value, updated_at)
                 VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(plugin_name, document_id) DO UPDATE SET
                   value = excluded.value,
                   updated_at = excluded.updated_at",
                params![plugin_name, document_id, value, now_millis()],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// 删除指定插件身份下的一条 JSON 文档。
    pub(crate) fn remove_plugin_document(
        &self,
        plugin_name: &str,
        document_id: &str,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "DELETE FROM plugin_documents WHERE plugin_name = ?1 AND document_id = ?2",
                params![plugin_name, document_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// 返回指定插件的全部键值存储，并保持 JSON 值类型不变。
    pub(crate) fn plugin_storage(
        &self,
        plugin_name: &str,
    ) -> Result<Vec<(String, serde_json::Value)>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT storage_key, value FROM plugin_storage
                 WHERE plugin_name = ?1 ORDER BY storage_key",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([plugin_name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| {
            let (key, value) = row.map_err(|error| error.to_string())?;
            Ok((
                key,
                serde_json::from_str(&value).map_err(|error| error.to_string())?,
            ))
        })
        .collect()
    }

    /// 新增或替换指定插件的一条键值存储。
    pub(crate) fn set_plugin_storage(
        &self,
        plugin_name: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        let encoded = serde_json::to_string(value).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO plugin_storage(plugin_name, storage_key, value, updated_at)
                 VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(plugin_name, storage_key) DO UPDATE SET
                   value = excluded.value,
                   updated_at = excluded.updated_at",
                params![plugin_name, key, encoded, now_millis()],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// 删除指定插件的一条键值存储。
    pub(crate) fn remove_plugin_storage(&self, plugin_name: &str, key: &str) -> Result<(), String> {
        self.connection
            .execute(
                "DELETE FROM plugin_storage WHERE plugin_name = ?1 AND storage_key = ?2",
                params![plugin_name, key],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// 返回指定插件持久化的动态 feature 声明。
    pub(crate) fn plugin_features(
        &self,
        plugin_name: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT value FROM plugin_features
                 WHERE plugin_name = ?1 ORDER BY updated_at, feature_code",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([plugin_name], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        rows.map(|row| {
            let value = row.map_err(|error| error.to_string())?;
            serde_json::from_str(&value).map_err(|error| error.to_string())
        })
        .collect()
    }

    /// 新增或替换指定插件的一条动态 feature。
    pub(crate) fn set_plugin_feature(
        &self,
        plugin_name: &str,
        feature_code: &str,
        feature: &serde_json::Value,
    ) -> Result<(), String> {
        let encoded = serde_json::to_string(feature).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO plugin_features(plugin_name, feature_code, value, updated_at)
                 VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(plugin_name, feature_code) DO UPDATE SET
                   value = excluded.value,
                   updated_at = excluded.updated_at",
                params![plugin_name, feature_code, encoded, now_millis()],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// 删除指定插件的一条动态 feature。
    pub(crate) fn remove_plugin_feature(
        &self,
        plugin_name: &str,
        feature_code: &str,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "DELETE FROM plugin_features WHERE plugin_name = ?1 AND feature_code = ?2",
                params![plugin_name, feature_code],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// 返回指定插件的全部附件，用于插件启动时构造同步兼容镜像。
    pub(crate) fn plugin_attachments(
        &self,
        plugin_name: &str,
    ) -> Result<Vec<(String, String, Vec<u8>)>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT attachment_id, content_type, data FROM plugin_attachments
                 WHERE plugin_name = ?1 ORDER BY attachment_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([plugin_name], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    /// 新增或替换指定插件的一份二进制附件。
    pub(crate) fn put_plugin_attachment(
        &self,
        plugin_name: &str,
        attachment_id: &str,
        content_type: &str,
        data: &[u8],
    ) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO plugin_attachments(plugin_name, attachment_id, content_type, data, updated_at)
                 VALUES(?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(plugin_name, attachment_id) DO UPDATE SET
                   content_type = excluded.content_type,
                   data = excluded.data,
                   updated_at = excluded.updated_at",
                params![plugin_name, attachment_id, content_type, data, now_millis()],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// 删除指定插件的一份二进制附件。
    pub(crate) fn remove_plugin_attachment(
        &self,
        plugin_name: &str,
        attachment_id: &str,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "DELETE FROM plugin_attachments
                 WHERE plugin_name = ?1 AND attachment_id = ?2",
                params![plugin_name, attachment_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// 在单一事务中删除指定插件的文档、键值、动态功能和附件数据。
    pub(crate) fn clear_plugin_data(&self, plugin_name: &str) -> Result<(), String> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        for table in [
            "plugin_documents",
            "plugin_storage",
            "plugin_features",
            "plugin_attachments",
        ] {
            transaction
                .execute(
                    &format!("DELETE FROM {table} WHERE plugin_name = ?1"),
                    [plugin_name],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        self.touch_modified(now_millis())
    }

    /**
     * 按同一保留期限清理文本和图片历史。
     * @param retention_days 保留天数。
     * @param now 当前时间戳。
     * @returns 删除记录总数或数据库错误。
     */
    pub(crate) fn prune_clipboard(&self, retention_days: u32, now: i64) -> Result<usize, String> {
        let cutoff = now.saturating_sub(i64::from(retention_days) * 86_400_000);
        let changed = self
            .connection
            .execute(
                "DELETE FROM clipboard_history WHERE captured_at < ?1",
                [cutoff],
            )
            .map_err(|error| error.to_string())?;
        let images = self
            .connection
            .execute(
                "DELETE FROM clipboard_images WHERE captured_at < ?1",
                [cutoff],
            )
            .map_err(|error| error.to_string())?;
        Ok(changed + images)
    }

    /// 返回按用户位置排序的收藏应用标识。
    pub(crate) fn pinned_ids(&self) -> Result<Vec<String>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT app_id FROM pinned_apps ORDER BY position, rowid")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    /// 返回同步文件需要的完整收藏记录。
    pub(crate) fn pinned_apps(&self) -> Result<Vec<AppEntry>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT app_id, name, path FROM pinned_apps ORDER BY position, rowid")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(AppEntry {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    keywords: Vec::new(),
                    source: "synced-pin".to_owned(),
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    /// 在单个事务中应用同步合并结果，同时保留本机级配置。
    pub(crate) fn apply_sync_document(&self, document: &SyncDocument) -> Result<(), String> {
        let mut merged_settings = document.settings.clone();
        let local_settings = self.settings()?;
        // 快捷键、开机启动、失焦行为和同步位置只对当前设备有效。
        merged_settings.shortcut = local_settings.shortcut;
        merged_settings.autostart = local_settings.autostart;
        merged_settings.hide_on_blur = local_settings.hide_on_blur;
        merged_settings.sync_enabled = local_settings.sync_enabled;
        merged_settings.sync_directory = local_settings.sync_directory;
        merged_settings.sync_interval_minutes = local_settings.sync_interval_minutes;
        merged_settings.update_feed_url = local_settings.update_feed_url;
        merged_settings.update_public_key = local_settings.update_public_key;

        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO settings(id, value) VALUES(1, ?1)
                 ON CONFLICT(id) DO UPDATE SET value = excluded.value",
                [serde_json::to_string(&merged_settings).map_err(|error| error.to_string())?],
            )
            .map_err(|error| error.to_string())?;

        transaction
            .execute("DELETE FROM pinned_apps", [])
            .map_err(|error| error.to_string())?;
        for (position, app) in document.pinned_apps.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO pinned_apps(app_id, name, path, position) VALUES(?1, ?2, ?3, ?4)",
                    params![app.id, app.name, app.path, position as i64],
                )
                .map_err(|error| error.to_string())?;
        }

        transaction
            .execute("DELETE FROM local_shortcuts", [])
            .map_err(|error| error.to_string())?;
        for shortcut in &document.local_shortcuts {
            transaction
                .execute(
                    "INSERT INTO local_shortcuts(id, name, alias, path, kind, added_at)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        shortcut.id,
                        shortcut.name,
                        shortcut.alias,
                        shortcut.path,
                        shortcut.kind,
                        shortcut.added_at
                    ],
                )
                .map_err(|error| error.to_string())?;
        }

        transaction
            .execute("DELETE FROM launch_history", [])
            .map_err(|error| error.to_string())?;
        for entry in &document.history {
            transaction
                .execute(
                    "INSERT INTO launch_history(app_id, name, path, launched_at)
                     VALUES(?1, ?2, ?3, ?4)",
                    params![entry.app_id, entry.name, entry.path, entry.launched_at],
                )
                .map_err(|error| error.to_string())?;
        }

        transaction
            .execute("DELETE FROM clipboard_history", [])
            .map_err(|error| error.to_string())?;
        for entry in &document.clipboard {
            transaction
                .execute(
                    "INSERT INTO clipboard_history(content_hash, content, captured_at)
                     VALUES(?1, ?2, ?3)",
                    params![
                        stable_text_hash(&entry.content),
                        entry.content,
                        entry.captured_at
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction
            .execute(
                "INSERT INTO metadata(key, value) VALUES('modified_at', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [document.modified_at.to_string()],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    }

    /// 新增或删除一条应用收藏记录。
    pub(crate) fn set_pinned(&self, app: &AppEntry, pinned: bool) -> Result<(), String> {
        if pinned {
            self.connection
                .execute(
                    "INSERT INTO pinned_apps(app_id, name, path, position)
                     VALUES(?1, ?2, ?3, COALESCE((SELECT MAX(position) + 1 FROM pinned_apps), 0))
                     ON CONFLICT(app_id) DO UPDATE SET name = excluded.name, path = excluded.path",
                    params![app.id, app.name, app.path],
                )
                .map_err(|error| error.to_string())?;
        } else {
            self.connection
                .execute("DELETE FROM pinned_apps WHERE app_id = ?1", [&app.id])
                .map_err(|error| error.to_string())?;
        }
        self.touch_modified(now_millis())
    }

    /// 按标识清理失效收藏记录。
    pub(crate) fn remove_pinned_id(&self, app_id: &str) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM pinned_apps WHERE app_id = ?1", [app_id])
            .map_err(|error| error.to_string())?;
        self.touch_modified(now_millis())
    }
}

/// 返回 Unix 毫秒时间戳供存储变更统一排序。
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

/// 使用固定 FNV-1a 算法为剪贴板内容生成去重键。
fn stable_text_hash(content: &str) -> String {
    let hash = content
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::Store;
    use crate::models::{AppEntry, LauncherSettings};
    use std::fs;

    /**
     * 验证图片历史去重排序、跨连接持久化、数量限制和统一清理。
     * @returns 无返回值。
     */
    #[test]
    fn persists_and_bounds_clipboard_images() {
        let directory = std::env::temp_dir().join(format!(
            "ztools-image-history-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("state.sqlite3");
        let store = Store::open(&path).unwrap();
        let mut image = crate::clipboard_images::CapturedImage {
            hash: "first".to_owned(),
            width: 1600,
            height: 900,
            png: vec![1, 2, 3],
            thumbnail: "preview".to_owned(),
        };
        store.capture_clipboard_image(&image, 100).unwrap();
        let first = store.clipboard_images().unwrap()[0].id;
        store.capture_clipboard_image(&image, 200).unwrap();
        assert_eq!(store.clipboard_images().unwrap().len(), 1);
        assert_eq!(store.clipboard_images().unwrap()[0].id, first);
        assert_eq!(store.clipboard_images().unwrap()[0].captured_at, 200);
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.clipboard_image_png(first).unwrap(), vec![1, 2, 3]);
        for index in 0..51 {
            image.hash = format!("image-{index}");
            store.capture_clipboard_image(&image, 300 + index).unwrap();
        }
        assert_eq!(store.clipboard_images().unwrap().len(), 50);
        assert!(store.clipboard_image_png(first).is_err());
        store.prune_clipboard(1, 86_400_340).unwrap();
        assert_eq!(store.clipboard_images().unwrap().len(), 11);
        store.clear_clipboard_history().unwrap();
        assert!(store.clipboard_images().unwrap().is_empty());
        drop(store);
        let _ = fs::remove_dir_all(directory);
    }

    /// 验证设置、收藏与历史能够跨 SQLite 连接持久化。
    #[test]
    fn persists_launcher_state() {
        let directory = std::env::temp_dir().join(format!(
            "ztools-store-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("state.sqlite3");
        let store = Store::open(&path).expect("store should open");
        let settings = LauncherSettings {
            shortcut: "Alt+X".to_owned(),
            autostart: true,
            hide_on_blur: false,
            max_results: 20,
            ..LauncherSettings::default()
        };
        let app = AppEntry {
            id: "app-demo".to_owned(),
            name: "Demo".to_owned(),
            path: "/tmp/demo.desktop".to_owned(),
            keywords: Vec::new(),
            source: "test".to_owned(),
        };

        store
            .save_settings(&settings)
            .expect("settings should save");
        store.set_pinned(&app, true).expect("pin should save");
        store
            .record_launch(&app, 1234)
            .expect("history should save");
        store
            .capture_clipboard("copied text", 2345)
            .expect("clipboard should save");
        store
            .put_plugin_document(
                "fixture",
                "paper/1",
                &serde_json::json!({ "_id": "paper/1", "value": 42 }),
            )
            .expect("plugin document should save");
        store
            .set_plugin_storage(
                "fixture",
                "preferences",
                &serde_json::json!({ "compact": true }),
            )
            .expect("plugin storage should save");
        store
            .set_plugin_feature(
                "fixture",
                "dynamic-search",
                &serde_json::json!({
                    "code": "dynamic-search",
                    "explain": "Dynamic search",
                    "cmds": ["dynamic"]
                }),
            )
            .expect("plugin feature should save");
        store
            .put_plugin_attachment("fixture", "image/1", "image/png", &[1, 2, 3, 4])
            .expect("plugin attachment should save");
        drop(store);

        let reopened = Store::open(&path).expect("store should reopen");
        assert_eq!(
            reopened.settings().expect("settings should load").shortcut,
            "Alt+X"
        );
        assert_eq!(
            reopened.pinned_ids().expect("pins should load"),
            ["app-demo"]
        );
        assert_eq!(reopened.history(10).expect("history should load").len(), 1);
        let clipboard = reopened
            .clipboard_history(10)
            .expect("clipboard should load");
        assert_eq!(clipboard.len(), 1);
        assert_eq!(clipboard[0].content, "copied text");
        let documents = reopened
            .plugin_documents("fixture")
            .expect("plugin documents should load");
        assert_eq!(documents[0]["value"], 42);
        assert!(reopened
            .plugin_documents("other-plugin")
            .expect("other plugin documents should load")
            .is_empty());
        let storage = reopened
            .plugin_storage("fixture")
            .expect("plugin storage should load");
        assert_eq!(storage[0].0, "preferences");
        assert_eq!(storage[0].1["compact"], true);
        assert!(reopened
            .plugin_storage("other-plugin")
            .expect("other plugin storage should load")
            .is_empty());
        let features = reopened
            .plugin_features("fixture")
            .expect("plugin features should load");
        assert_eq!(features[0]["code"], "dynamic-search");
        let attachments = reopened
            .plugin_attachments("fixture")
            .expect("plugin attachments should load");
        assert_eq!(attachments[0].0, "image/1");
        assert_eq!(attachments[0].1, "image/png");
        assert_eq!(attachments[0].2, [1, 2, 3, 4]);
        assert!(reopened
            .plugin_attachments("other-plugin")
            .expect("other plugin attachments should load")
            .is_empty());
        reopened
            .remove_plugin_document("fixture", "paper/1")
            .expect("plugin document should delete");
        reopened
            .remove_plugin_storage("fixture", "preferences")
            .expect("plugin storage should delete");
        reopened
            .remove_plugin_feature("fixture", "dynamic-search")
            .expect("plugin feature should delete");
        reopened
            .remove_plugin_attachment("fixture", "image/1")
            .expect("plugin attachment should delete");
        reopened
            .delete_clipboard_entry(clipboard[0].id)
            .expect("clipboard entry should delete");
        assert!(reopened
            .clipboard_history(10)
            .expect("clipboard should reload")
            .is_empty());
        drop(reopened);

        fs::remove_dir_all(directory).expect("fixture directory should be removable");
    }
}
