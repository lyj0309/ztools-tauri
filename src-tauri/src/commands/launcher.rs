use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(not(target_os = "linux"))]
use tauri::LogicalSize;
use tauri::{AppHandle, Manager, Monitor, PhysicalPosition, State, Webview};
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use crate::{
    desktop, launcher,
    models::{ClipboardEntry, LauncherSettings, LauncherSnapshot, LocalShortcut},
    state::AppState,
};

/// 从共享状态和 SQLite 生成供前端渲染的完整快照。
fn snapshot(state: &AppState) -> Result<LauncherSnapshot, String> {
    let mut apps = state
        .apps
        .lock()
        .map_err(|_| "应用缓存锁已损坏".to_owned())?
        .clone();
    let store = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?;
    let local_shortcuts = store.local_shortcuts()?;
    apps.extend(local_shortcuts.iter().map(launcher::local_shortcut_entry));
    Ok(LauncherSnapshot {
        apps,
        history: store.history(30)?,
        recent_plugin_usages: store.plugin_usage(30)?,
        clipboard: store.clipboard_history(100)?,
        local_shortcuts,
        pinned_ids: store.pinned_ids()?,
        settings: store.settings()?,
        sync_status: state.sync_status(),
    })
}

/// 首次加载应用列表并返回启动器完整状态。
#[tauri::command]
pub(crate) fn bootstrap_launcher(state: State<'_, AppState>) -> Result<LauncherSnapshot, String> {
    let needs_scan = state
        .apps
        .lock()
        .map_err(|_| "应用缓存锁已损坏".to_owned())?
        .is_empty();
    if needs_scan {
        // 扫描在首次界面加载时执行，结果随后保存在进程内存中。
        let apps = launcher::scan_applications();
        *state
            .apps
            .lock()
            .map_err(|_| "应用缓存锁已损坏".to_owned())? = apps;
    }
    snapshot(&state)
}

/// 强制重新扫描操作系统应用入口并返回新快照。
#[tauri::command]
pub(crate) fn refresh_applications(state: State<'_, AppState>) -> Result<LauncherSnapshot, String> {
    let apps = launcher::scan_applications();
    *state
        .apps
        .lock()
        .map_err(|_| "应用缓存锁已损坏".to_owned())? = apps;
    snapshot(&state)
}

/// 按应用标识启动可信扫描结果、写入历史并隐藏启动窗口。
#[tauri::command]
pub(crate) fn launch_application(
    app_id: String,
    state: State<'_, AppState>,
    window: Webview,
) -> Result<(), String> {
    // 只允许启动宿主扫描到的记录，避免前端传入任意命令行。
    let app = state
        .apps
        .lock()
        .map_err(|_| "应用缓存锁已损坏".to_owned())?
        .iter()
        .find(|app| app.id == app_id)
        .cloned();
    let app = match app {
        Some(app) => app,
        None => state
            .store
            .lock()
            .map_err(|_| "数据库锁已损坏".to_owned())?
            .local_shortcuts()?
            .iter()
            .map(launcher::local_shortcut_entry)
            .find(|app| app.id == app_id)
            .ok_or_else(|| "应用不存在，请刷新列表后重试".to_owned())?,
    };
    if app.source.starts_with("local-") {
        desktop::open_path(app.path.clone())?;
    } else {
        launcher::launch(&app)?;
    }

    let timestamp = current_timestamp()?;
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .record_launch(&app, timestamp)?;

    // 启动成功后释放前台焦点，让目标应用获得窗口焦点。
    window.window().hide().map_err(|error| error.to_string())
}

/// 更新应用收藏状态并返回最新收藏顺序。
#[tauri::command]
pub(crate) fn set_application_pinned(
    app_id: String,
    pinned: bool,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let app = state
        .apps
        .lock()
        .map_err(|_| "应用缓存锁已损坏".to_owned())?
        .iter()
        .find(|app| app.id == app_id)
        .cloned();
    let app = match app {
        Some(app) => app,
        None => state
            .store
            .lock()
            .map_err(|_| "数据库锁已损坏".to_owned())?
            .local_shortcuts()?
            .iter()
            .map(launcher::local_shortcut_entry)
            .find(|app| app.id == app_id)
            .ok_or_else(|| "应用不存在，请刷新列表后重试".to_owned())?,
    };
    let store = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?;
    store.set_pinned(&app, pinned)?;
    store.pinned_ids()
}

/// 清空启动历史记录。
#[tauri::command]
pub(crate) fn clear_launch_history(state: State<'_, AppState>) -> Result<(), String> {
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .clear_history()
}

/// 捕获当前纯文本剪贴板内容并返回最新历史。
#[tauri::command]
pub(crate) fn capture_clipboard(state: State<'_, AppState>) -> Result<Vec<ClipboardEntry>, String> {
    if let Some(content) = desktop::read_clipboard_text()? {
        if content.len() <= 2_000_000 {
            // 超大剪贴板不进入历史，避免界面唤起造成意外内存增长。
            state
                .store
                .lock()
                .map_err(|_| "数据库锁已损坏".to_owned())?
                .capture_clipboard(&content, current_timestamp()?)?;
        }
    }
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .clipboard_history(100)
}

/// 把历史文本写入系统剪贴板，并按设置隐藏窗口后模拟粘贴。
#[tauri::command]
pub(crate) async fn copy_clipboard_text(
    content: String,
    window: Webview,
    state: State<'_, AppState>,
) -> Result<(), String> {
    desktop::write_clipboard_text(content)?;
    let auto_paste = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .settings()?
        .auto_paste;
    if auto_paste {
        // 先释放 command 执行线程，让窗口管理器有机会恢复原应用焦点。
        window.window().hide().map_err(|error| error.to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            std::thread::sleep(std::time::Duration::from_millis(120));
            desktop::simulate_paste()
        })
        .await
        .map_err(|error| format!("自动粘贴任务异常结束：{error}"))??;
    }
    Ok(())
}

/// 删除单条剪贴板历史并返回剩余记录。
#[tauri::command]
pub(crate) fn delete_clipboard_entry(
    id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<ClipboardEntry>, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?;
    store.delete_clipboard_entry(id)?;
    store.clipboard_history(100)
}

/// 清空全部剪贴板历史。
#[tauri::command]
pub(crate) fn clear_clipboard_history(state: State<'_, AppState>) -> Result<(), String> {
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .clear_clipboard_history()
}

/// 打开由系统拖放事件提供的文件或目录路径。
#[tauri::command]
pub(crate) fn open_dropped_path(path: String) -> Result<(), String> {
    desktop::open_path(path)
}

/// 在系统文件管理器中定位由系统拖放事件提供的路径。
#[tauri::command]
pub(crate) fn reveal_dropped_path(path: String) -> Result<(), String> {
    desktop::reveal_path(path)
}

/// 把已拖入的有效路径加入启动器并返回最新快照。
#[tauri::command]
pub(crate) fn add_local_shortcut(
    path: String,
    state: State<'_, AppState>,
) -> Result<LauncherSnapshot, String> {
    let canonical = desktop::canonical_path(path)?;
    let path = canonical.to_string_lossy().into_owned();
    let name = canonical
        .file_stem()
        .or_else(|| canonical.file_name())
        .and_then(|value| value.to_str())
        .unwrap_or("本地项目")
        .to_owned();
    let kind = if canonical.is_dir() {
        if cfg!(target_os = "macos") && canonical.extension().is_some_and(|value| value == "app") {
            "app"
        } else {
            "folder"
        }
    } else if cfg!(target_os = "windows")
        && canonical
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "exe" | "lnk"))
    {
        "app"
    } else {
        "file"
    };
    let shortcut = LocalShortcut {
        id: launcher::local_shortcut_id(&path),
        name,
        alias: String::new(),
        path,
        kind: kind.to_owned(),
        added_at: current_timestamp()?,
    };
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .add_local_shortcut(&shortcut)?;
    snapshot(&state)
}

/// 更新本地启动项别名并返回最新快照。
#[tauri::command]
pub(crate) fn update_local_shortcut_alias(
    id: String,
    alias: String,
    state: State<'_, AppState>,
) -> Result<LauncherSnapshot, String> {
    if alias.chars().count() > 80 {
        return Err("别名不能超过 80 个字符".to_owned());
    }
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .update_local_shortcut_alias(&id, alias.trim())?;
    snapshot(&state)
}

/// 删除本地启动项并一并清理对应收藏。
#[tauri::command]
pub(crate) fn delete_local_shortcut(
    id: String,
    state: State<'_, AppState>,
) -> Result<LauncherSnapshot, String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?;
    store.delete_local_shortcut(&id)?;
    store.remove_pinned_id(&id)?;
    drop(store);
    snapshot(&state)
}

/// 应用并持久化快捷键、开机启动和界面设置。
#[tauri::command]
pub(crate) fn update_launcher_settings(
    settings: LauncherSettings,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<LauncherSettings, String> {
    if settings.shortcut.trim().is_empty() {
        return Err("全局快捷键不能为空".to_owned());
    }
    if !(4..=50).contains(&settings.max_results) {
        return Err("结果数量必须在 4 到 50 之间".to_owned());
    }
    if !matches!(settings.theme.as_str(), "system" | "light" | "dark") {
        return Err("主题只能是 system、light 或 dark".to_owned());
    }
    if !is_hex_color(&settings.accent_color) {
        return Err("主题色必须是 #RRGGBB 格式".to_owned());
    }
    if !(1..=3650).contains(&settings.clipboard_retention_days) {
        return Err("剪贴板保留天数必须在 1 到 3650 之间".to_owned());
    }
    if !(1..=1440).contains(&settings.sync_interval_minutes) {
        return Err("同步间隔必须在 1 到 1440 分钟之间".to_owned());
    }
    if settings.sync_enabled {
        let sync_path = std::path::Path::new(settings.sync_directory.trim());
        if settings.sync_directory.trim().is_empty() || !sync_path.is_absolute() {
            return Err("启用同步时必须填写绝对目录".to_owned());
        }
    }

    let previous = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .settings()?;

    // 先验证并切换系统级资源，失败时恢复旧快捷键。
    app.global_shortcut()
        .unregister_all()
        .map_err(|error| error.to_string())?;
    if let Err(error) = app.global_shortcut().register(settings.shortcut.as_str()) {
        let _ = app.global_shortcut().register(previous.shortcut.as_str());
        return Err(format!("快捷键不可用：{error}"));
    }

    let autolaunch = app.autolaunch();
    let autostart_result = if settings.autostart {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };
    if let Err(error) = autostart_result {
        // 开机启动失败时回滚刚刚切换的全局快捷键。
        let _ = app.global_shortcut().unregister_all();
        let _ = app.global_shortcut().register(previous.shortcut.as_str());
        return Err(format!("开机启动设置失败：{error}"));
    }

    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .save_settings(&settings)?;
    Ok(settings)
}

/// 判断设置页提交的颜色是否为六位十六进制颜色。
fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

/// 隐藏当前主窗口。
#[tauri::command]
pub(crate) fn hide_main_window(window: Webview) -> Result<(), String> {
    window.window().hide().map_err(|error| error.to_string())
}

/**
 * 按搜索结果或插件工作区高度调整主窗口，兼容多 Webview 窗口。
 * @param height 目标客户区高度。
 * @param webview 发起请求的主视图。
 * @returns 主窗口尺寸更新结果。
 */
#[tauri::command]
pub(crate) fn resize_main_window(height: u32, webview: Webview) -> Result<(), String> {
    if webview.label() != "main" || !(61..=600).contains(&height) {
        return Err("主窗口尺寸请求无效".to_owned());
    }
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;

        let parent = webview.window();
        let task_window = parent.clone();
        parent
            .run_on_main_thread(move || {
                // WebKitGTK 多视图关闭后会缓存自然高度；GDK 按实际内容尺寸更新窗口。
                if let Ok(gtk_window) = task_window.gtk_window() {
                    if let Some(gdk_window) = gtk_window.window() {
                        gdk_window.resize(800, height as i32);
                    }
                }
            })
            .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    webview
        .window()
        .set_size(LogicalSize::new(800.0, f64::from(height)))
        .map_err(|error| error.to_string())
}

/// 显示主窗口、在鼠标所在屏幕顶部六分之一处水平居中并把焦点交给搜索页。
pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_window("main") {
        // 先显示以便窗口管理器提交最终尺寸，随后再计算跨屏物理位置。
        let _ = window.show();
        // 使用物理坐标匹配鼠标所在显示器，兼容负坐标与不同缩放比例。
        let positioned = app
            .cursor_position()
            .ok()
            .and_then(|cursor| monitor_containing_cursor(app, cursor.x, cursor.y))
            .and_then(|monitor| {
                let origin = monitor.position();
                let monitor_size = monitor.size();
                let size = window.outer_size().ok()?;
                let x = origin.x
                    + (i64::from(monitor_size.width) - i64::from(size.width)).max(0) as i32 / 2;
                // 原版启动器固定在屏幕上方区域，窗口向下展开时仍完整留在当前显示器内。
                let y = origin.y + i64::from(monitor_size.height).max(0) as i32 / 6;
                window.set_position(PhysicalPosition::new(x, y)).ok()
            })
            .is_some();
        if !positioned {
            // 无法读取显示器信息时仍提供系统默认的居中行为。
            let _ = window.center();
        }
        let _ = window.set_focus();
    }
}

/// 从有效显示器边界中定位光标，避开部分 Linux 后端返回的零尺寸结果。
fn monitor_containing_cursor(app: &AppHandle, x: f64, y: f64) -> Option<Monitor> {
    let monitors = app.available_monitors().ok()?;
    monitors.into_iter().find(|monitor| {
        let origin = monitor.position();
        let size = monitor.size();
        size.width > 0
            && size.height > 0
            && x >= f64::from(origin.x)
            && y >= f64::from(origin.y)
            && x < f64::from(origin.x) + f64::from(size.width)
            && y < f64::from(origin.y) + f64::from(size.height)
    })
}

/// 切换主窗口的显示状态，供全局快捷键处理器调用。
pub(crate) fn toggle_main_window(app: &AppHandle) {
    if let Some(window) = app.get_window("main") {
        match window.is_visible() {
            Ok(true) => {
                let _ = window.hide();
            }
            _ => show_main_window(app),
        }
    }
}

/// 返回 Unix 毫秒时间戳供历史表统一排序。
pub(crate) fn current_timestamp() -> Result<i64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .map_err(|error| error.to_string())
}
