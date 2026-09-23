use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use tauri::{AppHandle, Emitter, Manager};

use crate::{commands::launcher::current_timestamp, desktop, launcher, state::AppState, sync};

pub(crate) struct BackgroundServices {
    stop: Arc<AtomicBool>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl BackgroundServices {
    /// 启动可取消的剪贴板监控与周期同步后台线程。
    pub(crate) fn start(app: AppHandle) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let workers = vec![
            start_clipboard_monitor(app.clone(), stop.clone()),
            start_sync_scheduler(app.clone(), stop.clone()),
            start_application_scanner(app, stop.clone()),
        ];
        Self {
            stop,
            workers: Mutex::new(workers),
        }
    }

    /// 通知所有后台循环退出，并等待线程释放数据库和桌面句柄。
    pub(crate) fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut workers) = self.workers.lock() {
            for worker in workers.drain(..) {
                let _ = worker.join();
            }
        }
    }
}

/// 启动周期同步线程，设置关闭时保持休眠并发布禁用状态。
fn start_sync_scheduler(app: AppHandle, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("ztools-sync-scheduler".to_owned())
        .spawn(move || {
            let mut last_attempt = 0_i64;
            while !stop.load(Ordering::Acquire) {
                let state = app.state::<AppState>();
                let settings = state
                    .store
                    .lock()
                    .ok()
                    .and_then(|store| store.settings().ok());
                if let Some(settings) = settings {
                    if settings.sync_enabled {
                        let now = current_timestamp().unwrap_or_default();
                        let interval = i64::from(settings.sync_interval_minutes.max(1)) * 60_000;
                        if last_attempt == 0 || now.saturating_sub(last_attempt) >= interval {
                            last_attempt = now;
                            let _ = sync::perform_sync(&state);
                            let _ = app.emit("sync-status-updated", state.sync_status());
                        }
                    } else if state.sync_status().state != "disabled" {
                        state.set_sync_status(crate::models::SyncStatus::default());
                        let _ = app.emit("sync-status-updated", state.sync_status());
                    }
                }
                sleep_interruptibly(&stop, Duration::from_secs(5));
            }
        })
        .expect("failed to start sync scheduler")
}

impl Drop for BackgroundServices {
    fn drop(&mut self) {
        self.stop();
    }
}

/**
 * 监控文本及图片剪贴板，在内容变化时保存历史。
 * @param app 桌面宿主句柄。
 * @param stop 退出信号。
 * @returns 后台监控线程句柄。
 */
fn start_clipboard_monitor(app: AppHandle, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("ztools-clipboard-monitor".to_owned())
        .spawn(move || {
            let mut last_content = String::new();
            while !stop.load(Ordering::Acquire) {
                let state = app.state::<AppState>();
                let settings = state
                    .store
                    .lock()
                    .ok()
                    .and_then(|store| store.settings().ok());
                if !settings
                    .as_ref()
                    .is_some_and(|settings| settings.clipboard_monitoring)
                {
                    let _ = crate::clipboard_images::capture_current(&app);
                    last_content.clear();
                    sleep_interruptibly(&stop, Duration::from_millis(700));
                    continue;
                }

                // 只在文本发生变化时写库，避免轮询持续刷新排序时间。
                if let Ok(Some(content)) = desktop::read_clipboard_text() {
                    if content != last_content && content.len() <= 2_000_000 {
                        let timestamp = current_timestamp().unwrap_or_default();
                        if let Ok(store) = state.store.lock() {
                            let saved =
                                store.capture_clipboard(&content, timestamp).and_then(|_| {
                                    store.prune_clipboard(
                                        settings
                                            .as_ref()
                                            .map_or(180, |value| value.clipboard_retention_days),
                                        timestamp,
                                    )?;
                                    store.clipboard_history(100)
                                });
                            if let Ok(history) = saved {
                                let _ = app.emit("clipboard-history-updated", history);
                            }
                        }
                        last_content = content;
                    }
                }
                // 原图编码运行在后台线程，图片历史通过独立选择页按需读取。
                let _ = crate::clipboard_images::capture_current(&app);
                sleep_interruptibly(&stop, Duration::from_millis(700));
            }
        })
        .expect("failed to start clipboard monitor")
}

/// 定期重扫系统应用目录，在安装或卸载应用后刷新前端缓存。
fn start_application_scanner(app: AppHandle, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("ztools-application-scanner".to_owned())
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                sleep_interruptibly(&stop, Duration::from_secs(60));
                if stop.load(Ordering::Acquire) {
                    break;
                }

                // 扫描结果先整体替换，再发布事件，前端不会看到半成品列表。
                let apps = launcher::scan_applications();
                let state = app.state::<AppState>();
                if let Ok(mut current) = state.apps.lock() {
                    *current = apps.clone();
                    let _ = app.emit("applications-updated", apps);
                };
            }
        })
        .expect("failed to start application scanner")
}

/// 将较长休眠拆成短片段，使退出请求最多等待一秒即可生效。
fn sleep_interruptibly(stop: &AtomicBool, duration: Duration) {
    let mut remaining = duration;
    let slice = Duration::from_secs(1);
    while remaining > Duration::ZERO && !stop.load(Ordering::Acquire) {
        let current = remaining.min(slice);
        thread::sleep(current);
        remaining = remaining.saturating_sub(current);
    }
}
