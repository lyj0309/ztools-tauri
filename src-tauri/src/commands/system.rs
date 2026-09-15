use std::{collections::HashMap, fs};

use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::{plugin::PermissionState, AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;

use crate::{
    backup::{self, BackupReport, RestoreReport},
    desktop,
    legacy::{self, LegacyImportReport},
    models::{SyncStatus, UpdateInfo},
    plugin::PluginRuntime,
    state::AppState,
    sync,
};

/// 显示原生保存对话框并创建包含数据库和插件的完整备份。
#[tauri::command]
pub(crate) async fn create_backup(
    app: AppHandle,
    runtime: State<'_, PluginRuntime>,
) -> Result<Option<BackupReport>, String> {
    let selected = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("备份 ZTools 数据")
            .set_file_name("ztools-backup.ztools-backup")
            .add_filter("ZTools 备份", &["ztools-backup"])
            .save_file()
    })
    .await
    .map_err(|error| format!("备份文件选择任务异常结束：{error}"))?;
    let Some(destination) = selected else {
        return Ok(None);
    };
    let plugin_root = runtime.root().to_path_buf();
    let backup_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = backup_app.state::<AppState>();
        let store = state
            .store
            .lock()
            .map_err(|_| "数据库锁已损坏".to_owned())?;
        backup::create(&destination, &store, &plugin_root).map(Some)
    })
    .await
    .map_err(|error| format!("备份任务异常结束：{error}"))?
}

/// 显示原生文件对话框，校验归档后恢复数据库和已安装插件。
#[tauri::command]
pub(crate) async fn restore_backup(
    app: AppHandle,
    runtime: State<'_, PluginRuntime>,
) -> Result<Option<RestoreReport>, String> {
    let selected = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("恢复 ZTools 数据")
            .add_filter("ZTools 备份", &["ztools-backup"])
            .pick_file()
    })
    .await
    .map_err(|error| format!("恢复文件选择任务异常结束：{error}"))?;
    let Some(source) = selected else {
        return Ok(None);
    };

    // 恢复前停止开发监听并关闭插件窗口，避免旧资源继续写入被替换的数据。
    runtime.stop_all_development_watches();
    for (label, window) in app.webview_windows() {
        if label.starts_with("plugin-") {
            let _ = window.close();
        }
    }
    let plugin_root = runtime.root().to_path_buf();
    let restore_app = app.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        let state = restore_app.state::<AppState>();
        let mut store = state
            .store
            .lock()
            .map_err(|_| "数据库锁已损坏".to_owned())?;
        backup::restore(&source, &mut store, &plugin_root)
    })
    .await
    .map_err(|error| format!("恢复任务异常结束：{error}"))??;

    // 旧备份可能不含当前版本默认插件，恢复完成后立即补齐内嵌副本。
    runtime.ensure_bundled_plugins()?;

    // 数据恢复后同步当前进程持有的全局快捷键和开机启动状态。
    let settings = app
        .state::<AppState>()
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .settings()?;
    app.global_shortcut()
        .unregister_all()
        .map_err(|error| error.to_string())?;
    if app
        .global_shortcut()
        .register(settings.shortcut.as_str())
        .is_err()
    {
        app.global_shortcut()
            .register("Alt+Z")
            .map_err(|error| format!("数据已恢复，但快捷键注册失败：{error}"))?;
    }
    if settings.autostart {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    }
    .map_err(|error| format!("数据已恢复，但开机启动设置失败：{error}"))?;
    let _ = app.emit("backup-restored", &report);
    Ok(Some(report))
}

/// 返回当前系统中检测到的 Electron ZTools 数据目录。
#[tauri::command]
pub(crate) fn detect_legacy_data() -> Vec<String> {
    legacy::detect_candidates()
}

/// 从用户指定的旧数据目录执行只读 LMDB 导入。
#[tauri::command]
pub(crate) fn import_legacy_data(
    path: String,
    state: State<'_, AppState>,
) -> Result<LegacyImportReport, String> {
    legacy::import(path, &state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HttpRequest {
    method: String,
    url: String,
    headers: HashMap<String, String>,
    body: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HttpResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
}

/// 立即执行一次文件夹双向同步并通知当前界面。
#[tauri::command]
pub(crate) fn sync_now(app: AppHandle, state: State<'_, AppState>) -> Result<SyncStatus, String> {
    let result = sync::perform_sync(&state);
    let status = state.sync_status();
    let _ = app.emit("sync-status-updated", status);
    result
}

/// 返回后台同步服务最近状态。
#[tauri::command]
pub(crate) fn get_sync_status(state: State<'_, AppState>) -> SyncStatus {
    state.sync_status()
}

/// 请求系统通知权限并发送一条可见测试通知。
#[tauri::command]
pub(crate) fn send_test_notification(app: AppHandle) -> Result<(), String> {
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
        .title("ZTools")
        .body("Tauri 2 原生通知工作正常")
        .show()
        .map_err(|error| error.to_string())
}

/// 隐藏启动器后截取鼠标所在屏幕，并恢复窗口与焦点。
#[tauri::command]
pub(crate) async fn capture_screen(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<String, String> {
    window.hide().map_err(|error| error.to_string())?;
    let capture_handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        // 让窗口管理器先完成隐藏，再调用可能阻塞的系统截图工具。
        std::thread::sleep(std::time::Duration::from_millis(220));
        desktop::capture_screen(&capture_handle).map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|error| format!("截图任务异常结束：{error}"))?;

    // 无论截图成功或失败都恢复设置窗口，避免用户失去错误反馈入口。
    let _ = window.show();
    let _ = window.set_focus();
    result
}

/// 请求发布源并判断当前程序是否有新版本。
#[tauri::command]
pub(crate) async fn check_for_updates(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UpdateInfo, String> {
    let feed_url = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .settings()?
        .update_feed_url;
    let url = reqwest::Url::parse(&feed_url).map_err(|error| format!("更新地址无效：{error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("更新地址只支持 HTTP 或 HTTPS".to_owned());
    }

    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?
        .get(url)
        .header("User-Agent", "ZTools-Tauri-Updater")
        .send()
        .await
        .map_err(|error| format!("检查更新失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("更新服务返回错误：{error}"))?;
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("更新响应格式无效：{error}"))?;
    let latest = value
        .get("tag_name")
        .or_else(|| value.get("version"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "更新响应缺少版本号".to_owned())?
        .trim_start_matches('v')
        .to_owned();
    let release_url = value
        .get("html_url")
        .or_else(|| value.get("url"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let notes = value
        .get("body")
        .or_else(|| value.get("notes"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let current = app.package_info().version.to_string();
    let update_available = Version::parse(&latest)
        .ok()
        .zip(Version::parse(&current).ok())
        .is_some_and(|(latest, current)| latest > current);
    Ok(UpdateInfo {
        current_version: current,
        latest_version: latest,
        update_available,
        release_url,
        notes,
    })
}

/// 下载并安装经过 Tauri 签名验证的更新，然后重启应用。
#[tauri::command]
pub(crate) async fn install_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let settings = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .settings()?;
    if settings.update_public_key.trim().is_empty() {
        return Err("安装更新前必须配置 Tauri 签名公钥".to_owned());
    }
    let endpoint = reqwest::Url::parse(&settings.update_feed_url)
        .map_err(|error| format!("更新地址无效：{error}"))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|error| error.to_string())?
        .pubkey(settings.update_public_key)
        .build()
        .map_err(|error| error.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|error| format!("检查签名更新失败：{error}"))?
        .ok_or_else(|| "当前已是最新版本".to_owned())?;
    let progress_app = app.clone();
    update
        .download_and_install(
            move |chunk_length, content_length| {
                let _ = progress_app.emit(
                    "update-progress",
                    serde_json::json!({
                        "chunkLength": chunk_length,
                        "contentLength": content_length
                    }),
                );
            },
            {
                let app = app.clone();
                move || {
                    let _ = app.emit("update-progress", serde_json::json!({ "finished": true }));
                }
            },
        )
        .await
        .map_err(|error| format!("下载或安装更新失败：{error}"))?;
    app.restart();
}

/// 在系统浏览器打开更新发布页。
#[tauri::command]
pub(crate) fn open_external_url(url: String) -> Result<(), String> {
    desktop::open_url(url)
}

/// 执行启动器内置的固定系统命令，不接受任意程序或参数。
#[tauri::command]
pub(crate) fn run_system_command(command_id: String, app: AppHandle) -> Result<(), String> {
    let directory = match command_id.as_str() {
        "open-home" => Some(dirs::home_dir().ok_or_else(|| "无法定位用户主目录".to_owned())?),
        "open-downloads" => {
            Some(dirs::download_dir().ok_or_else(|| "无法定位下载目录".to_owned())?)
        }
        "open-app-data" => Some(
            app.path()
                .app_data_dir()
                .map_err(|error| error.to_string())?,
        ),
        "open-plugins" => Some(crate::plugin::plugin_root()),
        "open-temp" => Some(std::env::temp_dir()),
        "lock-screen" => {
            lock_screen()?;
            None
        }
        _ => return Err("未知系统命令".to_owned()),
    };
    if let Some(directory) = directory {
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        desktop::open_path(directory.to_string_lossy().into_owned())?;
    }
    Ok(())
}

/// 调用当前平台固定的锁屏程序，并拒绝通过 Shell 拼接输入。
fn lock_screen() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let candidates: &[(&str, &[&str])] = &[("rundll32.exe", &["user32.dll,LockWorkStation"])];
    #[cfg(target_os = "macos")]
    let candidates: &[(&str, &[&str])] = &[(
        "/System/Library/CoreServices/Menu Extras/User.menu/Contents/Resources/CGSession",
        &["-suspend"],
    )];
    #[cfg(target_os = "linux")]
    let candidates: &[(&str, &[&str])] = &[
        ("loginctl", &["lock-session"]),
        ("xdg-screensaver", &["lock"]),
    ];
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let candidates: &[(&str, &[&str])] = &[];

    for (program, arguments) in candidates {
        match std::process::Command::new(program).args(*arguments).spawn() {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("无法执行锁屏：{error}")),
        }
    }
    Err("当前系统没有可用的锁屏命令".to_owned())
}

/// 执行受大小和超时约束的通用 HTTP 请求。
#[tauri::command]
pub(crate) async fn http_request(request: HttpRequest) -> Result<HttpResponse, String> {
    let url = reqwest::Url::parse(&request.url).map_err(|error| format!("URL 无效：{error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("网络请求只支持 HTTP 或 HTTPS".to_owned());
    }
    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .map_err(|error| format!("HTTP 方法无效：{error}"))?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|error| error.to_string())?;
    let mut builder = client.request(method, url);
    for (name, value) in request.headers {
        builder = builder.header(name, value);
    }
    if let Some(body) = request.body {
        if body.len() > 2_000_000 {
            return Err("请求正文不能超过 2 MB".to_owned());
        }
        builder = builder.body(body);
    }
    let response = builder
        .send()
        .await
        .map_err(|error| format!("网络请求失败：{error}"))?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.to_string(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取响应失败：{error}"))?;
    if bytes.len() > 10_000_000 {
        return Err("响应正文超过 10 MB".to_owned());
    }
    Ok(HttpResponse {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::{http_request, HttpRequest};
    use std::{
        collections::HashMap,
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    /// 验证受约束 HTTP 客户端能读取本地服务的状态、响应头和正文。
    #[test]
    fn performs_bounded_http_request() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("HTTP fixture should bind");
        let address = listener.local_addr().expect("fixture address should exist");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("fixture should accept request");
            let mut buffer = [0_u8; 2048];
            let bytes = stream
                .read(&mut buffer)
                .expect("request should be readable");
            assert!(String::from_utf8_lossy(&buffer[..bytes]).starts_with("GET /health HTTP/1.1"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                )
                .expect("fixture response should be writable");
        });
        let response = tauri::async_runtime::block_on(http_request(HttpRequest {
            method: "GET".to_owned(),
            url: format!("http://{address}/health"),
            headers: HashMap::new(),
            body: None,
        }))
        .expect("HTTP request should succeed");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, "ok");
        assert_eq!(
            response.headers.get("content-type").map(String::as_str),
            Some("text/plain")
        );
        server.join().expect("HTTP fixture should stop");
    }
}
