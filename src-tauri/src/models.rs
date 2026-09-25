use serde::{Deserialize, Serialize};

const UPDATE_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEFFRDdCN0VDNUUyMDcwMTUKUldRVmNDQmU3TGZYcmxYSEF3cmNaTkp4MlltZEd2Y2V6VmhrSFJFNjBRMGVLNEcwMzRxSThGeVcK";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppEntry {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) keywords: Vec<String>,
    pub(crate) source: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalShortcut {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) alias: String,
    pub(crate) path: String,
    pub(crate) kind: String,
    pub(crate) added_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryEntry {
    pub(crate) app_id: String,
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) launched_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginUsageEntry {
    pub(crate) plugin_name: String,
    pub(crate) feature_code: String,
    pub(crate) used_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClipboardEntry {
    pub(crate) id: i64,
    pub(crate) content: String,
    pub(crate) captured_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClipboardImageEntry {
    pub(crate) id: i64,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) captured_at: i64,
    pub(crate) thumbnail: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClipboardFileItem {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) is_directory: bool,
    pub(crate) exists: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClipboardFilesEntry {
    pub(crate) id: i64,
    pub(crate) files: Vec<ClipboardFileItem>,
    pub(crate) captured_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct LauncherSettings {
    pub(crate) shortcut: String,
    pub(crate) autostart: bool,
    pub(crate) hide_on_blur: bool,
    pub(crate) max_results: usize,
    pub(crate) theme: String,
    pub(crate) accent_color: String,
    pub(crate) show_recent: bool,
    pub(crate) clipboard_monitoring: bool,
    pub(crate) auto_paste: bool,
    pub(crate) clipboard_retention_days: u32,
    pub(crate) sync_enabled: bool,
    pub(crate) sync_directory: String,
    pub(crate) sync_interval_minutes: u32,
    pub(crate) auto_check_updates: bool,
    pub(crate) update_feed_url: String,
    pub(crate) update_public_key: String,
}

impl Default for LauncherSettings {
    fn default() -> Self {
        Self {
            shortcut: "Alt+Z".to_owned(),
            autostart: false,
            hide_on_blur: false,
            max_results: 12,
            theme: "system".to_owned(),
            accent_color: "#059669".to_owned(),
            show_recent: true,
            clipboard_monitoring: true,
            auto_paste: false,
            clipboard_retention_days: 180,
            sync_enabled: false,
            sync_directory: String::new(),
            sync_interval_minutes: 30,
            auto_check_updates: true,
            update_feed_url:
                "https://github.com/lyj0309/ztools-tauri/releases/latest/download/latest.json"
                    .to_owned(),
            update_public_key: UPDATE_PUBLIC_KEY.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncStatus {
    pub(crate) state: String,
    pub(crate) message: String,
    pub(crate) last_synced_at: Option<i64>,
}

impl Default for SyncStatus {
    fn default() -> Self {
        Self {
            state: "disabled".to_owned(),
            message: "同步未启用".to_owned(),
            last_synced_at: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateInfo {
    pub(crate) current_version: String,
    pub(crate) latest_version: String,
    pub(crate) update_available: bool,
    pub(crate) release_url: String,
    pub(crate) notes: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LauncherSnapshot {
    pub(crate) apps: Vec<AppEntry>,
    pub(crate) history: Vec<HistoryEntry>,
    pub(crate) recent_plugin_usages: Vec<PluginUsageEntry>,
    pub(crate) clipboard: Vec<ClipboardEntry>,
    pub(crate) local_shortcuts: Vec<LocalShortcut>,
    pub(crate) pinned_ids: Vec<String>,
    pub(crate) settings: LauncherSettings,
    pub(crate) sync_status: SyncStatus,
}
