use std::sync::Mutex;

use crate::{
    models::{AppEntry, SyncStatus},
    storage::Store,
};

pub(crate) struct AppState {
    pub(crate) apps: Mutex<Vec<AppEntry>>,
    pub(crate) store: Mutex<Store>,
    sync_status: Mutex<SyncStatus>,
}

impl AppState {
    /// 创建包含应用缓存和持久化仓库的共享状态。
    pub(crate) fn new(store: Store) -> Self {
        Self {
            apps: Mutex::new(Vec::new()),
            store: Mutex::new(store),
            sync_status: Mutex::new(SyncStatus::default()),
        }
    }

    /// 返回当前后台同步状态的只读副本。
    pub(crate) fn sync_status(&self) -> SyncStatus {
        self.sync_status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default()
    }

    /// 替换后台同步状态，锁异常时忽略状态展示更新。
    pub(crate) fn set_sync_status(&self, status: SyncStatus) {
        if let Ok(mut current) = self.sync_status.lock() {
            *current = status;
        }
    }
}
