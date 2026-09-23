mod backup;
mod clipboard_images;
mod commands;
mod desktop;
mod launcher;
mod legacy;
mod legacy_lmdb_v2;
mod models;
mod ocr;
mod plugin;
mod screenshot;
mod services;
mod state;
mod storage;
mod sync;
#[cfg(target_os = "windows")]
mod windows_ml;

use std::{io, path::PathBuf};

use commands::launcher::{show_main_window, toggle_main_window};
use state::AppState;
use storage::Store;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, RunEvent, WindowEvent,
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// 注册启动器服务并运行 Tauri 桌面事件循环。
pub fn run() {
    #[cfg(target_os = "windows")]
    windows_ml::initialize();
    let plugin_root = plugin::plugin_root();
    let protocol_root = plugin_root.clone();
    let application = tauri::Builder::default()
        .register_uri_scheme_protocol("ztools-plugin", move |context, request| {
            plugin::serve_plugin_asset(&protocol_root, context.webview_label(), request)
        })
        // 单实例插件必须先处理第二次启动，再初始化其他桌面资源。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_main_window(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            commands::launcher::bootstrap_launcher,
            commands::launcher::refresh_applications,
            commands::launcher::launch_application,
            commands::launcher::set_application_pinned,
            commands::launcher::clear_launch_history,
            commands::launcher::capture_clipboard,
            commands::launcher::copy_clipboard_text,
            commands::launcher::delete_clipboard_entry,
            commands::launcher::clear_clipboard_history,
            commands::launcher::open_dropped_path,
            commands::launcher::reveal_dropped_path,
            commands::launcher::add_local_shortcut,
            commands::launcher::update_local_shortcut_alias,
            commands::launcher::delete_local_shortcut,
            commands::launcher::update_launcher_settings,
            commands::launcher::hide_main_window,
            commands::system::sync_now,
            commands::system::get_sync_status,
            commands::system::send_test_notification,
            commands::system::capture_screen,
            ocr::plugin_ocr,
            ocr::plugin_ocr_copy_text,
            screenshot::screenshot_editor_source,
            screenshot::screenshot_editor_info,
            screenshot::screenshot_editor_ready,
            screenshot::screenshot_save,
            screenshot::screenshot_copy,
            screenshot::screenshot_pin,
            screenshot::screenshot_pin_ready,
            screenshot::screenshot_pin_resize,
            screenshot::screenshot_history_list,
            screenshot::screenshot_history_pin,
            screenshot::screenshot_history_close,
            screenshot::screenshot_cancel,
            screenshot::screenshot_pin_source,
            screenshot::screenshot_pin_start_dragging,
            screenshot::screenshot_pin_copy,
            screenshot::screenshot_pin_save,
            screenshot::screenshot_pin_close,
            commands::system::check_for_updates,
            commands::system::install_update,
            commands::system::open_external_url,
            commands::system::run_system_command,
            commands::system::http_request,
            commands::system::detect_legacy_data,
            commands::system::import_legacy_data,
            commands::system::create_backup,
            commands::system::restore_backup,
            commands::plugin::list_plugins,
            commands::plugin::fetch_plugin_market,
            commands::plugin::install_plugin_from_market,
            commands::plugin::cancel_plugin_market_install,
            commands::plugin::install_plugin_directory,
            commands::plugin::register_plugin_development,
            commands::plugin::stop_plugin_development,
            commands::plugin::uninstall_plugin,
            commands::plugin::launch_plugin_feature,
            commands::plugin::plugin_copy_text,
            commands::plugin::plugin_clipboard_get_history,
            commands::plugin::plugin_clipboard_search,
            commands::plugin::plugin_clipboard_delete,
            commands::plugin::plugin_clipboard_clear,
            commands::plugin::plugin_clipboard_write_history,
            commands::plugin::plugin_show_notification,
            commands::plugin::plugin_out,
            commands::plugin::plugin_hide_main_window,
            commands::plugin::plugin_db_put,
            commands::plugin::plugin_db_remove,
            commands::plugin::plugin_storage_set,
            commands::plugin::plugin_storage_remove,
            commands::plugin::plugin_feature_set,
            commands::plugin::plugin_feature_remove,
            commands::plugin::plugin_attachment_put,
            commands::plugin::plugin_attachment_get,
            commands::plugin::plugin_attachment_remove,
            commands::plugin::plugin_file_stats,
            commands::plugin::plugin_read_directory,
            commands::plugin::plugin_path_exists,
            commands::plugin::plugin_rename_path,
            commands::plugin::plugin_read_file,
            commands::plugin::plugin_write_file,
            commands::plugin::plugin_copy_path,
            commands::plugin::plugin_create_directory,
            commands::plugin::plugin_shell_open,
            commands::plugin::plugin_shell_reveal,
            commands::plugin::plugin_http_request,
            commands::plugin::plugin_dialog_open,
            commands::plugin::plugin_dialog_save,
            commands::plugin::plugin_window_set_size,
            commands::plugin::plugin_window_set_position,
            commands::plugin::plugin_window_center,
            commands::plugin::plugin_window_set_always_on_top,
            commands::plugin::plugin_window_set_minimized,
            commands::plugin::plugin_window_set_maximized,
            commands::plugin::plugin_window_set_fullscreen,
            commands::plugin::plugin_screen_capture,
            commands::plugin::plugin_input_type_text,
            commands::plugin::plugin_input_tap_key
        ])
        .setup(move |app| {
            // 新项目使用独立 SQLite 文件，不读取或覆盖 Electron 的 LMDB 数据。
            let database_path = application_data_root(app)?.join("ztools.sqlite3");
            let store = Store::open(&database_path).map_err(io::Error::other)?;
            let settings = store.settings().map_err(io::Error::other)?;
            app.manage(AppState::new(store));
            app.manage(screenshot::ScreenshotRuntime::new());
            let plugin_runtime =
                plugin::PluginRuntime::new(plugin_root.clone()).map_err(io::Error::other)?;
            // 用户首次启动或升级后先发布随包默认插件，再开放前端插件列表查询。
            plugin_runtime
                .ensure_bundled_plugins()
                .map_err(io::Error::other)?;
            app.manage(plugin_runtime);
            app.manage(services::BackgroundServices::start(app.handle().clone()));

            // 启动时让系统注册状态与持久化设置重新一致。
            let autolaunch = app.autolaunch();
            if settings.autostart {
                let _ = autolaunch.enable();
            } else {
                let _ = autolaunch.disable();
            }

            // 注册保存的快捷键；无效的旧配置回退到默认值，不阻断应用启动。
            if app
                .global_shortcut()
                .register(settings.shortcut.as_str())
                .is_err()
            {
                let _ = app.global_shortcut().register("Alt+Z");
            }

            // 托盘只保留高频窗口控制和显式退出动作。
            let show_item = MenuItem::with_id(app, "show", "显示 ZTools", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;
            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "linux")]
                configure_linux_launcher_minimum_height(&window);
                // 关闭按钮与失焦行为只隐藏常驻启动器，显式退出由托盘负责。
                let app_handle = app.handle().clone();
                window.on_window_event(move |event| match event {
                    WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        if let Some(window) = app_handle.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    }
                    WindowEvent::Focused(false) => {
                        let should_hide = app_handle
                            .state::<AppState>()
                            .store
                            .lock()
                            .ok()
                            .and_then(|store| store.settings().ok())
                            .is_some_and(|settings| settings.hide_on_blur);
                        if should_hide {
                            if let Some(window) = app_handle.get_webview_window("main") {
                                let _ = window.hide();
                            }
                        }
                    }
                    _ => {}
                });
            }

            // --hidden 供系统开机启动使用，人工启动则立即展示搜索窗口。
            let e2e_plugin_launch = std::env::var("ZTOOLS_E2E").as_deref() == Ok("1")
                && std::env::var_os("ZTOOLS_E2E_PLUGIN_NAME").is_some();
            if !std::env::args().any(|argument| argument == "--hidden") && !e2e_plugin_launch {
                show_main_window_after_ready(app.handle().clone());
            }
            if e2e_plugin_launch {
                launch_e2e_plugin(app.handle())?;
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the ZTools Tauri host");

    // 退出事件循环前显式停止并回收所有后台线程。
    application.run(|app, event| {
        if matches!(event, RunEvent::Exit) {
            app.state::<screenshot::ScreenshotRuntime>().cleanup();
            app.state::<plugin::PluginRuntime>()
                .stop_all_development_watches();
            app.state::<services::BackgroundServices>().stop();
        }
    });
}

/// 降低 WebKitGTK 子控件的默认高度请求，使 Linux 启动器能收起到原版 61 像素。
#[cfg(target_os = "linux")]
fn configure_linux_launcher_minimum_height(window: &tauri::WebviewWindow) {
    use gtk::prelude::{ContainerExt, WidgetExt};

    // WebKitGTK 默认请求 200 像素高度；覆盖子控件请求后仍由窗口约束保证宽度。
    if let Ok(container) = window.default_vbox() {
        container.set_size_request(-1, 1);
        for child in container.children() {
            child.set_size_request(-1, 1);
        }
    }
}

/**
 * 在隔离测试模式下启动指定插件，截图与历史贴图走正式入口。
 * @param app 桌面宿主句柄。
 * @returns 启动任务提交结果。
 */
fn launch_e2e_plugin(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let Some(plugin_name) = std::env::var_os("ZTOOLS_E2E_PLUGIN_NAME") else {
        return Ok(());
    };
    let plugin_name = plugin_name.to_string_lossy().into_owned();
    let feature_code =
        std::env::var("ZTOOLS_E2E_PLUGIN_FEATURE").unwrap_or_else(|_| plugin_name.clone());
    let payload = std::env::var("ZTOOLS_E2E_PLUGIN_PAYLOAD")
        .ok()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or(serde_json::Value::Null);
    eprintln!("[e2e] launching plugin {plugin_name}:{feature_code}");
    if plugin_name == "screenshot" && (feature_code == "capture" || feature_code == "pin") {
        let screenshot_app = app.clone();
        let main = app
            .get_webview_window("main")
            .ok_or_else(|| io::Error::other("main window is unavailable"))?;
        // 测试使用正式异步入口创建截图编辑器或历史图片选择页。
        tauri::async_runtime::spawn(async move {
            let result = if feature_code == "pin" {
                screenshot::start_history(screenshot_app, main).await
            } else {
                screenshot::start_editor(screenshot_app, main).await
            };
            match result {
                Ok(()) => eprintln!("[e2e] screenshot plugin launch completed"),
                Err(error) => eprintln!("[e2e] screenshot plugin launch failed: {error}"),
            }
        });
        return Ok(());
    }
    let runtime = app.state::<plugin::PluginRuntime>();
    if let Some(source) = std::env::var_os("ZTOOLS_E2E_PLUGIN_DEV_SOURCE") {
        runtime
            .register_development_directory(app, &PathBuf::from(source))
            .map_err(io::Error::other)?;
        eprintln!("[e2e] registered development directory for {plugin_name}");
    }
    let action = plugin::PluginEnterAction {
        code: feature_code,
        kind: if payload.is_array() { "files" } else { "text" }.to_owned(),
        payload,
    };
    plugin::launch_plugin(app, &runtime, &plugin_name, action).map_err(io::Error::other)?;
    eprintln!("[e2e] plugin launch completed");
    Ok(())
}

/// 返回应用数据根目录；端到端验证仅在显式测试模式下接受隔离目录。
fn application_data_root<R: tauri::Runtime>(app: &tauri::App<R>) -> Result<PathBuf, io::Error> {
    if std::env::var("ZTOOLS_E2E").as_deref() == Ok("1") {
        if let Some(root) = std::env::var_os("ZTOOLS_DATA_ROOT") {
            return Ok(PathBuf::from(root));
        }
    }
    app.path()
        .app_data_dir()
        .map_err(|error| io::Error::other(error.to_string()))
}

/// 等待首个窗口完成布局后再显示，避免隐藏窗口报告临时尺寸导致定位偏移。
fn show_main_window_after_ready(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("ztools-initial-window".to_owned())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(180));
            let main_thread_handle = app.clone();
            let _ = app.run_on_main_thread(move || show_main_window(&main_thread_handle));
            // 首次映射后窗口管理器才提供最终外框尺寸，再定位一次消除启动竞态。
            std::thread::sleep(std::time::Duration::from_millis(120));
            let positioned_handle = app.clone();
            let _ = app.run_on_main_thread(move || show_main_window(&positioned_handle));
        })
        .expect("failed to schedule initial window");
}
