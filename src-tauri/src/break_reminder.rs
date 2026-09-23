use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde::Deserialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::{NotificationExt, PermissionState};

use crate::{plugin, state::AppState};

const PLUGIN_NAME: &str = "break-reminder";
const NOTIFICATION_LEAD: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
struct ReminderSettings {
    enabled: bool,
    interval_minutes: u32,
    duration_seconds: u32,
}

impl Default for ReminderSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: 20,
            duration_seconds: 10,
        }
    }
}

impl ReminderSettings {
    /**
     * 限制插件设置的数值范围，使损坏或手工改写的数据不会形成忙循环。
     * @returns 已限制范围的设置。
     */
    fn normalized(self) -> Self {
        Self {
            enabled: self.enabled,
            interval_minutes: self.interval_minutes.clamp(1, 480),
            duration_seconds: self.duration_seconds.clamp(1, 600),
        }
    }
}

#[derive(Default)]
struct ReminderClock {
    settings: Option<ReminderSettings>,
    due_at: Option<Instant>,
    notified: bool,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct ReminderTick {
    notify: bool,
    show_break: bool,
}

impl ReminderClock {
    /**
     * 根据持久化设置与当前单调时间推进提醒，设置变化时重新起算间隔。
     * @param settings 当前插件设置。
     * @param now 本次轮询的单调时间。
     * @returns 本轮需要发送的通知和休息弹窗动作。
     */
    fn tick(&mut self, settings: ReminderSettings, now: Instant) -> ReminderTick {
        let settings = settings.normalized();
        if self.settings != Some(settings) {
            // 启用或修改设置时从当前时间起算，禁用时清除未完成的提醒。
            self.settings = Some(settings);
            self.due_at = settings
                .enabled
                .then(|| now + Duration::from_secs(u64::from(settings.interval_minutes) * 60));
            self.notified = false;
        }
        let Some(due_at) = self.due_at else {
            return ReminderTick::default();
        };
        if now >= due_at {
            // 一次休息结束后才开始下一轮工作间隔，避免长提醒连续覆盖弹窗。
            self.due_at = Some(
                now + Duration::from_secs(u64::from(settings.duration_seconds))
                    + Duration::from_secs(u64::from(settings.interval_minutes) * 60),
            );
            self.notified = false;
            return ReminderTick {
                notify: false,
                show_break: true,
            };
        }
        if !self.notified && now + NOTIFICATION_LEAD >= due_at {
            self.notified = true;
            return ReminderTick {
                notify: true,
                show_break: false,
            };
        }
        ReminderTick::default()
    }
}

/**
 * 从插件私有存储读取休息提醒配置；尚未配置时保持默认关闭。
 * @param app 桌面应用句柄。
 * @returns 当前合法配置，数据库不可读时返回错误。
 */
fn load_settings(app: &AppHandle) -> Result<ReminderSettings, String> {
    let state = app.state::<AppState>();
    let values = state
        .store
        .lock()
        .map_err(|_| "休息提醒数据库锁已损坏".to_owned())?
        .plugin_storage(PLUGIN_NAME)?;
    Ok(values
        .into_iter()
        .find(|(key, _)| key == "settings")
        .and_then(|(_, value)| serde_json::from_value::<ReminderSettings>(value).ok())
        .unwrap_or_default()
        .normalized())
}

/**
 * 在休息开始前发送系统通知；通知权限未获授权时不阻断后续弹窗。
 * @param app 桌面应用句柄。
 * @returns 通知处理结果，系统拒绝时返回原因。
 */
fn send_notification(app: &AppHandle) -> Result<(), String> {
    let notification = app.notification();
    let permission = notification
        .permission_state()
        .map_err(|error| error.to_string())?;
    let permission = if matches!(
        permission,
        PermissionState::Prompt | PermissionState::PromptWithRationale
    ) {
        notification
            .request_permission()
            .map_err(|error| error.to_string())?
    } else {
        permission
    };
    if permission != PermissionState::Granted {
        return Err("系统通知权限未授予".to_owned());
    }
    notification
        .builder()
        .title("ZTools · 休息提醒")
        .body("10 秒后该休息一下了")
        .show()
        .map_err(|error| error.to_string())
}

/**
 * 在主线程打开内置插件休息页，保证 WebView 创建遵循桌面事件循环约束。
 * @param app 桌面应用句柄。
 * @param duration_seconds 本次休息弹窗持续秒数。
 * @returns 任务提交结果。
 */
fn show_break(app: AppHandle, duration_seconds: u32) -> Result<(), String> {
    app.clone()
        .run_on_main_thread(move || {
            let runtime = app.state::<plugin::PluginRuntime>();
            let action = plugin::PluginEnterAction {
                code: "break".to_owned(),
                kind: "text".to_owned(),
                payload: serde_json::json!({ "durationSeconds": duration_seconds }),
            };
            if let Err(error) = plugin::launch_plugin(&app, &runtime, PLUGIN_NAME, action) {
                eprintln!("[break-reminder] unable to show break: {error}");
            }
        })
        .map_err(|error| error.to_string())
}

/**
 * 启动常驻休息提醒计时器，关闭应用时由宿主退出信号停止。
 * @param app 桌面应用句柄。
 * @param stop 后台服务统一退出信号。
 * @returns 后台工作线程句柄。
 */
pub(crate) fn start(app: AppHandle, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("ztools-break-reminder".to_owned())
        .spawn(move || {
            let mut clock = ReminderClock::default();
            while !stop.load(Ordering::Acquire) {
                match load_settings(&app) {
                    Ok(settings) => {
                        let tick = clock.tick(settings, Instant::now());
                        if tick.notify {
                            let notification_app = app.clone();
                            if let Err(error) = app.run_on_main_thread(move || {
                                if let Err(error) = send_notification(&notification_app) {
                                    eprintln!("[break-reminder] notification unavailable: {error}");
                                }
                            }) {
                                eprintln!("[break-reminder] notification dispatch failed: {error}");
                            }
                        }
                        if tick.show_break {
                            if let Err(error) = show_break(app.clone(), settings.duration_seconds) {
                                eprintln!("[break-reminder] break dispatch failed: {error}");
                            }
                        }
                    }
                    Err(error) => eprintln!("[break-reminder] unable to read settings: {error}"),
                }
                // 一秒轮询只读一项插件设置，不依赖隐藏 WebView 的 JS 定时器。
                for _ in 0..10 {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        })
        .expect("failed to start break reminder")
}

#[cfg(test)]
mod tests {
    use super::{ReminderClock, ReminderSettings, ReminderTick};
    use std::time::{Duration, Instant};

    /**
     * 验证提前通知、到时弹窗和下一轮间隔不会连续触发。
     * @returns 无返回值。
     */
    #[test]
    fn reminder_sequence_respects_duration_and_interval() {
        let start = Instant::now();
        let settings = ReminderSettings {
            enabled: true,
            interval_minutes: 1,
            duration_seconds: 10,
        };
        let mut clock = ReminderClock::default();
        assert_eq!(clock.tick(settings, start), ReminderTick::default());
        assert_eq!(
            clock.tick(settings, start + Duration::from_secs(50)),
            ReminderTick {
                notify: true,
                show_break: false
            }
        );
        assert_eq!(
            clock.tick(settings, start + Duration::from_secs(60)),
            ReminderTick {
                notify: false,
                show_break: true
            }
        );
        assert_eq!(
            clock.tick(settings, start + Duration::from_secs(119)),
            ReminderTick::default()
        );
        assert_eq!(
            clock.tick(settings, start + Duration::from_secs(120)),
            ReminderTick {
                notify: true,
                show_break: false
            }
        );
        assert_eq!(
            clock.tick(settings, start + Duration::from_secs(130)),
            ReminderTick {
                notify: false,
                show_break: true
            }
        );
    }

    /**
     * 验证关闭及重新启用提醒会清除旧周期并重新起算。
     * @returns 无返回值。
     */
    #[test]
    fn disabling_and_reenabling_resets_due_time() {
        let start = Instant::now();
        let enabled = ReminderSettings {
            enabled: true,
            interval_minutes: 1,
            duration_seconds: 10,
        };
        let mut clock = ReminderClock::default();
        clock.tick(enabled, start);
        clock.tick(
            ReminderSettings {
                enabled: false,
                ..enabled
            },
            start + Duration::from_secs(55),
        );
        assert_eq!(
            clock.tick(enabled, start + Duration::from_secs(56)),
            ReminderTick::default()
        );
        assert_eq!(
            clock.tick(enabled, start + Duration::from_secs(106)),
            ReminderTick {
                notify: true,
                show_break: false
            }
        );
    }
}
