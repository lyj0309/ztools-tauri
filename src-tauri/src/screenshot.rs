use std::{
    collections::HashMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tauri::{
    ipc::Response, AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, WindowEvent,
};

use crate::desktop;

const EDITOR_LABEL: &str = "plugin-screenshot";
const PIN_LABEL_PREFIX: &str = "plugin-screenshot-pin-";
const MAX_SCREENSHOT_BYTES: usize = 64 * 1024 * 1024;

/// 管理截图编辑器源图和悬浮贴图临时文件。
pub(crate) struct ScreenshotRuntime {
    next_id: AtomicU64,
    editor_source: Mutex<Option<PathBuf>>,
    pins: Mutex<HashMap<String, PathBuf>>,
}

impl ScreenshotRuntime {
    /// 创建空的截图运行时。
    pub(crate) fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            editor_source: Mutex::new(None),
            pins: Mutex::new(HashMap::new()),
        }
    }

    /// 分配进程内唯一序号。
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// 替换编辑器源图并清理上一份临时文件。
    fn replace_editor_source(&self, path: PathBuf) -> Result<(), String> {
        let mut source = self
            .editor_source
            .lock()
            .map_err(|_| "截图状态锁已损坏".to_owned())?;
        if let Some(previous) = source.replace(path) {
            let _ = fs::remove_file(previous);
        }
        Ok(())
    }

    /// 读取当前编辑器源图路径。
    fn editor_source(&self) -> Result<PathBuf, String> {
        self.editor_source
            .lock()
            .map_err(|_| "截图状态锁已损坏".to_owned())?
            .clone()
            .ok_or_else(|| "截图编辑会话已经结束".to_owned())
    }

    /// 取出编辑器源图，使后续窗口销毁回调不会重复删除。
    fn take_editor_source(&self) -> Option<PathBuf> {
        self.editor_source.lock().ok()?.take()
    }

    /// 注册悬浮贴图临时文件。
    fn register_pin(&self, label: String, path: PathBuf) -> Result<(), String> {
        self.pins
            .lock()
            .map_err(|_| "贴图状态锁已损坏".to_owned())?
            .insert(label, path);
        Ok(())
    }

    /// 返回指定悬浮贴图文件。
    fn pin_path(&self, label: &str) -> Result<PathBuf, String> {
        self.pins
            .lock()
            .map_err(|_| "贴图状态锁已损坏".to_owned())?
            .get(label)
            .cloned()
            .ok_or_else(|| "贴图已经关闭".to_owned())
    }

    /// 移除悬浮贴图记录并返回临时文件。
    fn remove_pin(&self, label: &str) -> Option<PathBuf> {
        self.pins.lock().ok()?.remove(label)
    }

    /// 应用退出前删除仍由运行时持有的截图临时文件。
    pub(crate) fn cleanup(&self) {
        if let Some(path) = self.take_editor_source() {
            let _ = fs::remove_file(path);
        }
        if let Ok(mut pins) = self.pins.lock() {
            for (_, path) in pins.drain() {
                let _ = fs::remove_file(path);
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScreenshotEditorInfo {
    width: u32,
    height: u32,
}

/// 隐藏主窗口、截取鼠标所在显示器并打开选区编辑器。
pub(crate) async fn start_editor(app: AppHandle, main: WebviewWindow) -> Result<(), String> {
    if main.label() != "main" {
        return Err("只有主启动器可以发起截图".to_owned());
    }
    if let Some(existing) = app.get_webview_window(EDITOR_LABEL) {
        let _ = existing.close();
    }

    let monitor = active_monitor(&app)?;
    let monitor_position = *monitor.position();
    let monitor_size = *monitor.size();
    main.hide().map_err(|error| error.to_string())?;

    let runtime = app.state::<ScreenshotRuntime>();
    let source = temporary_path(&app, "editor", runtime.next_id())?;
    let capture_app = app.clone();
    let capture_source = source.clone();
    let capture_result = tauri::async_runtime::spawn_blocking(move || {
        // 给窗口管理器留出一帧隐藏主窗口，避免把启动器截进背景。
        std::thread::sleep(std::time::Duration::from_millis(220));
        desktop::capture_screen_to(&capture_app, &capture_source)
    })
    .await
    .map_err(|error| format!("截图任务异常结束：{error}"))?;
    if let Err(error) = capture_result {
        crate::commands::launcher::show_main_window(&app);
        return Err(error);
    }
    runtime.replace_editor_source(source)?;

    let build_result = WebviewWindowBuilder::new(&app, EDITOR_LABEL, plugin_url("index.html")?)
        .title("ZTools 截图")
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .inner_size(
            f64::from(monitor_size.width),
            f64::from(monitor_size.height),
        )
        .build();
    let editor = match build_result {
        Ok(window) => window,
        Err(error) => {
            cleanup_editor_source(&app);
            crate::commands::launcher::show_main_window(&app);
            return Err(format!("无法创建截图编辑器：{error}"));
        }
    };
    editor.on_window_event({
        let app = app.clone();
        move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                cleanup_editor_source(&app);
                crate::commands::launcher::show_main_window(&app);
            }
        }
    });
    let prepare_result = editor
        .set_position(PhysicalPosition::new(
            monitor_position.x,
            monitor_position.y,
        ))
        .and_then(|_| editor.set_size(PhysicalSize::new(monitor_size.width, monitor_size.height)))
        .and_then(|_| editor.show())
        .and_then(|_| editor.set_focus());
    if let Err(error) = prepare_result {
        // 创建后的任一步失败都销毁半成品窗口并恢复启动器。
        let _ = editor.close();
        cleanup_editor_source(&app);
        crate::commands::launcher::show_main_window(&app);
        return Err(error.to_string());
    }
    schedule_e2e_pin_trigger(&app, &editor);
    Ok(())
}

/// 在隔离测试模式下等待外部触发文件，再点击截图编辑器的贴图按钮。
fn schedule_e2e_pin_trigger(app: &AppHandle, editor: &WebviewWindow) {
    if std::env::var("ZTOOLS_E2E").as_deref() != Ok("1") {
        return;
    }
    let Some(trigger) = std::env::var_os("ZTOOLS_E2E_SCREENSHOT_PIN_TRIGGER") else {
        return;
    };
    let trigger = PathBuf::from(trigger);
    let app = app.clone();
    let editor = editor.clone();
    std::thread::spawn(move || {
        // 等待桌面自动化完成选区，超时后退出，避免测试钩子常驻。
        for _ in 0..120 {
            if trigger.is_file() {
                let _ = fs::remove_file(&trigger);
                let result =
                    editor.eval("document.querySelector('[data-action=\"pin\"]')?.click()");
                eprintln!("[e2e] screenshot pin trigger evaluated: {result:?}");
                std::thread::sleep(std::time::Duration::from_secs(2));
                // 发布版 WebView2 可能接受但不执行后台 eval；固定调用正式命令验证原生窗口链路。
                let result = create_e2e_native_pin(&app, &editor);
                eprintln!("[e2e] screenshot native pin fallback: {result:?}");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        eprintln!("[e2e] screenshot pin trigger timed out");
    });
}

/// 从当前截图源裁剪固定测试区域并调用正式贴图命令。
fn create_e2e_native_pin(app: &AppHandle, editor: &WebviewWindow) -> Result<(), String> {
    let runtime = app.state::<ScreenshotRuntime>();
    let source = read_png(&runtime.editor_source()?)?;
    let image = decode_png(&source)?;
    let x = 120_u32.min(image.width().saturating_sub(1));
    let y = 100_u32.min(image.height().saturating_sub(1));
    let width = 700_u32.min(image.width().saturating_sub(x));
    let height = 440_u32.min(image.height().saturating_sub(y));
    let cropped = image.crop_imm(x, y, width, height);
    let mut output = Cursor::new(Vec::new());
    cropped
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|error| format!("无法编码 E2E 贴图：{error}"))?;
    let data = output.into_inner();
    let pin_app = app.clone();
    let pin_editor = editor.clone();
    app.run_on_main_thread(move || {
        let result = screenshot_pin(
            data,
            f64::from(x),
            f64::from(y),
            f64::from(width),
            f64::from(height),
            pin_app,
            pin_editor,
        );
        eprintln!("[e2e] screenshot native pin result: {result:?}");
    })
    .map_err(|error| error.to_string())
}

/// 返回当前编辑器源 PNG 的原始字节。
#[tauri::command]
pub(crate) fn screenshot_editor_source(
    window: WebviewWindow,
    runtime: tauri::State<'_, ScreenshotRuntime>,
) -> Result<Response, String> {
    require_editor(&window)?;
    read_png(&runtime.editor_source()?).map(Response::new)
}

/// 返回当前编辑器源图尺寸，供前端校验画布映射。
#[tauri::command]
pub(crate) fn screenshot_editor_info(
    window: WebviewWindow,
    runtime: tauri::State<'_, ScreenshotRuntime>,
) -> Result<ScreenshotEditorInfo, String> {
    require_editor(&window)?;
    let data = read_png(&runtime.editor_source()?)?;
    let image = decode_png(&data)?;
    Ok(ScreenshotEditorInfo {
        width: image.width(),
        height: image.height(),
    })
}

/// 把编辑后的 PNG 保存到系统图片目录并关闭编辑器。
#[tauri::command]
pub(crate) fn screenshot_save(
    data: Vec<u8>,
    app: AppHandle,
    window: WebviewWindow,
) -> Result<String, String> {
    require_editor(&window)?;
    validate_png(&data)?;
    let destination = save_png_to_pictures(&app, &data)?;
    finish_editor(
        &app,
        &window,
        "saved",
        Some(destination.to_string_lossy().as_ref()),
    )?;
    Ok(destination.to_string_lossy().into_owned())
}

/// 把编辑后的 PNG 写入系统剪贴板并关闭编辑器。
#[tauri::command]
pub(crate) fn screenshot_copy(
    data: Vec<u8>,
    app: AppHandle,
    window: WebviewWindow,
) -> Result<(), String> {
    require_editor(&window)?;
    validate_png(&data)?;
    desktop::write_clipboard_image_png(&data)?;
    finish_editor(&app, &window, "copied", None)
}

/// 用编辑后的 PNG 创建无边框置顶贴图窗口并关闭编辑器。
#[tauri::command]
pub(crate) fn screenshot_pin(
    data: Vec<u8>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    app: AppHandle,
    window: WebviewWindow,
) -> Result<(), String> {
    require_editor(&window)?;
    validate_png(&data)?;
    if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() {
        return Err("贴图位置或尺寸无效".to_owned());
    }
    let width = width.clamp(80.0, 1600.0);
    let height = height.clamp(60.0, 1200.0);
    let runtime = app.state::<ScreenshotRuntime>();
    let id = runtime.next_id();
    let label = format!("{PIN_LABEL_PREFIX}{id}");
    let path = temporary_path(&app, "pin", id)?;
    fs::write(&path, &data).map_err(|error| format!("无法创建贴图文件：{error}"))?;
    runtime.register_pin(label.clone(), path.clone())?;

    let editor_position = window.outer_position().map_err(|error| error.to_string())?;
    let scale_factor = window.scale_factor().map_err(|error| error.to_string())?;
    let pin_x = editor_position.x + (x * scale_factor).round() as i32;
    let pin_y = editor_position.y + (y * scale_factor).round() as i32;
    let build_result = WebviewWindowBuilder::new(&app, &label, plugin_url("pin.html")?)
        .title("ZTools 贴图")
        .decorations(false)
        .resizable(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .inner_size(width, height)
        .min_inner_size(80.0, 60.0)
        .build();
    let pin = match build_result {
        Ok(pin) => pin,
        Err(error) => {
            runtime.remove_pin(&label);
            let _ = fs::remove_file(path);
            return Err(format!("无法创建贴图窗口：{error}"));
        }
    };
    pin.on_window_event({
        let app = app.clone();
        let label = label.clone();
        move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                cleanup_pin(&app, &label);
            }
        }
    });
    if let Err(error) = pin
        .set_position(PhysicalPosition::new(pin_x, pin_y))
        .and_then(|_| pin.show())
    {
        // 贴图未能显示时回收动态窗口及其临时文件。
        let _ = pin.close();
        cleanup_pin(&app, &label);
        return Err(error.to_string());
    }
    finish_editor(&app, &window, "pinned", None)
}

/// 取消截图并关闭编辑器。
#[tauri::command]
pub(crate) fn screenshot_cancel(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    require_editor(&window)?;
    finish_editor(&app, &window, "cancelled", None)
}

/// 返回调用窗口对应的贴图 PNG 字节。
#[tauri::command]
pub(crate) fn screenshot_pin_source(
    window: WebviewWindow,
    runtime: tauri::State<'_, ScreenshotRuntime>,
) -> Result<Response, String> {
    require_pin(&window)?;
    read_png(&runtime.pin_path(window.label())?).map(Response::new)
}

/// 开始拖动当前无边框贴图窗口。
#[tauri::command]
pub(crate) fn screenshot_pin_start_dragging(window: WebviewWindow) -> Result<(), String> {
    require_pin(&window)?;
    window.start_dragging().map_err(|error| error.to_string())
}

/// 把当前贴图再次写入系统图片剪贴板。
#[tauri::command]
pub(crate) fn screenshot_pin_copy(
    window: WebviewWindow,
    runtime: tauri::State<'_, ScreenshotRuntime>,
) -> Result<(), String> {
    require_pin(&window)?;
    let data = read_png(&runtime.pin_path(window.label())?)?;
    desktop::write_clipboard_image_png(&data)
}

/// 把当前贴图另存到系统图片目录。
#[tauri::command]
pub(crate) fn screenshot_pin_save(
    app: AppHandle,
    window: WebviewWindow,
    runtime: tauri::State<'_, ScreenshotRuntime>,
) -> Result<String, String> {
    require_pin(&window)?;
    let data = read_png(&runtime.pin_path(window.label())?)?;
    save_png_to_pictures(&app, &data).map(|path| path.to_string_lossy().into_owned())
}

/// 关闭当前贴图窗口并删除其临时文件。
#[tauri::command]
pub(crate) fn screenshot_pin_close(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    require_pin(&window)?;
    let label = window.label().to_owned();
    window.close().map_err(|error| error.to_string())?;
    cleanup_pin(&app, &label);
    Ok(())
}

/// 获取鼠标所在显示器，无法读取鼠标位置时回退到主显示器。
fn active_monitor(app: &AppHandle) -> Result<tauri::Monitor, String> {
    if let Ok(cursor) = app.cursor_position() {
        if let Ok(Some(monitor)) = app.monitor_from_point(cursor.x, cursor.y) {
            return Ok(monitor);
        }
    }
    app.primary_monitor()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "无法确定截图显示器".to_owned())
}

/// 构造仅由宿主控制的截图临时文件路径。
fn temporary_path(app: &AppHandle, kind: &str, id: u64) -> Result<PathBuf, String> {
    let cache_root = if std::env::var("ZTOOLS_E2E").as_deref() == Ok("1") {
        // 真实桌面测试显式使用隔离根目录，不能把临时截图写进用户缓存。
        std::env::var_os("ZTOOLS_DATA_ROOT")
            .map(PathBuf::from)
            .ok_or_else(|| "截图 E2E 缺少隔离数据目录".to_owned())?
            .join("cache")
    } else {
        app.path()
            .app_cache_dir()
            .map_err(|error| error.to_string())?
    };
    let directory = cache_root.join("screenshots");
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建截图缓存目录：{error}"))?;
    Ok(directory.join(format!("{kind}-{id}.png")))
}

/// 校验调用窗口是唯一截图编辑器。
fn require_editor(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == EDITOR_LABEL {
        Ok(())
    } else {
        Err("当前窗口不是截图编辑器".to_owned())
    }
}

/// 校验调用窗口是宿主创建的截图贴图。
fn require_pin(window: &WebviewWindow) -> Result<(), String> {
    if window.label().starts_with(PIN_LABEL_PREFIX) {
        Ok(())
    } else {
        Err("当前窗口不是截图贴图".to_owned())
    }
}

/// 构造默认截图插件的私有协议资源地址。
fn plugin_url(entry: &str) -> Result<WebviewUrl, String> {
    let url = tauri::Url::parse(&format!("ztools-plugin://localhost/screenshot/{entry}"))
        .map_err(|error| format!("截图插件入口 URL 无效：{error}"))?;
    Ok(WebviewUrl::External(url))
}

/// 读取受宿主管理的 PNG 文件并限制 IPC 体积。
fn read_png(path: &Path) -> Result<Vec<u8>, String> {
    let data = fs::read(path).map_err(|error| format!("无法读取截图：{error}"))?;
    validate_png(&data)?;
    Ok(data)
}

/// 解码并校验 PNG 数据。
fn decode_png(data: &[u8]) -> Result<image::DynamicImage, String> {
    if data.is_empty() || data.len() > MAX_SCREENSHOT_BYTES {
        return Err("截图数据为空或超过 64 MB".to_owned());
    }
    image::load_from_memory_with_format(data, image::ImageFormat::Png)
        .map_err(|error| format!("截图 PNG 无效：{error}"))
}

/// 校验 PNG 数据可以安全交给剪贴板、文件和贴图窗口。
fn validate_png(data: &[u8]) -> Result<(), String> {
    let image = decode_png(data)?;
    if image.width() == 0
        || image.height() == 0
        || image.width() > 16_384
        || image.height() > 16_384
    {
        return Err("截图尺寸无效或超过 16384 像素限制".to_owned());
    }
    Ok(())
}

/// 把 PNG 写入系统图片目录并返回最终路径。
fn save_png_to_pictures(app: &AppHandle, data: &[u8]) -> Result<PathBuf, String> {
    validate_png(data)?;
    let directory = app
        .path()
        .picture_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建截图目录：{error}"))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let output = directory.join(format!("ZTools-Screenshot-{timestamp}.png"));
    fs::write(&output, data).map_err(|error| format!("无法保存截图：{error}"))?;
    Ok(output)
}

/// 结束编辑会话、通知主窗口并恢复启动器。
fn finish_editor(
    app: &AppHandle,
    window: &WebviewWindow,
    action: &str,
    path: Option<&str>,
) -> Result<(), String> {
    cleanup_editor_source(app);
    let _ = app.emit_to(
        "main",
        "screenshot-finished",
        serde_json::json!({ "action": action, "path": path }),
    );
    let close_result = window.close().map_err(|error| error.to_string());
    crate::commands::launcher::show_main_window(app);
    close_result
}

/// 删除当前编辑器源图。
fn cleanup_editor_source(app: &AppHandle) {
    if let Some(path) = app.state::<ScreenshotRuntime>().take_editor_source() {
        let _ = fs::remove_file(path);
    }
}

/// 删除指定贴图的临时文件和运行时记录。
fn cleanup_pin(app: &AppHandle, label: &str) {
    if let Some(path) = app.state::<ScreenshotRuntime>().remove_pin(label) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_png, MAX_SCREENSHOT_BYTES};

    /// 拒绝空数据和超过上限的截图请求。
    #[test]
    fn rejects_invalid_screenshot_payloads() {
        assert!(validate_png(&[]).is_err());
        assert!(validate_png(&vec![0; MAX_SCREENSHOT_BYTES + 1]).is_err());
    }
}
