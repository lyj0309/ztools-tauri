use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tauri::{
    ipc::{InvokeBody, Request, Response},
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, Window, WindowEvent,
};

use crate::desktop;

const EDITOR_LABEL: &str = "plugin-screenshot";
const HISTORY_LABEL: &str = "plugin-screenshot-history";
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

/**
 * 无过渡隐藏启动器，截取固定显示器并打开选区编辑器。
 * @param app 桌面宿主句柄。
 * @param main 发起截图的启动器窗口。
 * @param mode 原版截图命令的普通、立即复制或立即保存模式。
 * @returns 编辑器创建结果，抓屏失败时恢复启动器。
 */
pub(crate) async fn start_editor(app: AppHandle, main: Window, mode: &str) -> Result<(), String> {
    if main.label() != "main" {
        return Err("只有主启动器可以发起截图".to_owned());
    }
    if !matches!(mode, "capture" | "copy" | "save") {
        return Err("未知的截图模式".to_owned());
    }
    if let Some(existing) = app.get_webview_window(EDITOR_LABEL) {
        let _ = existing.close();
    }

    let monitor = active_monitor(&app)?;
    let monitor_position = *monitor.position();
    let monitor_size = *monitor.size();
    // 隐藏启动器时关闭系统过渡，不再固定停顿等待动画结束。
    set_window_transitions(&main, false)?;
    let hidden = main.hide().map_err(|error| error.to_string());
    let _ = set_window_transitions(&main, true);
    hidden?;

    let runtime = app.state::<ScreenshotRuntime>();
    let source = temporary_path(&app, "editor", runtime.next_id())?;
    let capture_app = app.clone();
    let capture_source = source.clone();
    let capture_result = tauri::async_runtime::spawn_blocking(move || {
        // Windows 只等待桌面合成提交隐藏结果，不引入人为过渡时长。
        flush_capture_frame();
        desktop::capture_display_to(
            &capture_app,
            &capture_source,
            monitor_position,
            monitor_size,
        )
    })
    .await
    .map_err(|error| format!("截图任务异常结束：{error}"))?;
    if let Err(error) = capture_result {
        crate::commands::launcher::show_main_window(&app);
        return Err(error);
    }
    runtime.replace_editor_source(source)?;

    let build_result = WebviewWindowBuilder::new(
        &app,
        EDITOR_LABEL,
        plugin_url(&format!("index.html?mode={mode}"))?,
    )
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
            if matches!(event, WindowEvent::Destroyed) && cleanup_editor_source(&app) {
                // 非正常销毁仍恢复启动器；正常完成会先取走会话，因此不会重复弹窗和抢焦点。
                crate::commands::launcher::show_main_window(&app);
            }
        }
    });
    let prepare_result = editor
        .set_position(PhysicalPosition::new(
            monitor_position.x,
            monitor_position.y,
        ))
        .and_then(|_| editor.set_size(PhysicalSize::new(monitor_size.width, monitor_size.height)));
    let prepare_result = prepare_result
        .map_err(|error| error.to_string())
        .and_then(|_| set_window_transitions(&editor.as_ref().window(), false));
    if let Err(error) = prepare_result {
        // 创建后的任一步失败都销毁半成品窗口并恢复启动器。
        let _ = editor.close();
        cleanup_editor_source(&app);
        crate::commands::launcher::show_main_window(&app);
        return Err(error.to_string());
    }
    Ok(())
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

/**
 * 用用户选定的 PNG 替换临时源图，供原版截图标注界面继续编辑。
 * @param request 原始 PNG 二进制请求。
 * @param window 当前截图编辑器窗口。
 * @param runtime 截图会话运行时。
 * @returns 临时源图写入结果。
 */
#[tauri::command]
pub(crate) fn screenshot_editor_select(
    request: Request<'_>,
    window: WebviewWindow,
    runtime: tauri::State<'_, ScreenshotRuntime>,
) -> Result<(), String> {
    require_editor(&window)?;
    let data = raw_png(&request)?;
    validate_png(&data)?;
    let image = decode_png(&data)?;
    // 选区确定后不再需要全屏底图；复用会话临时文件避免把大图片放进 Web 存储。
    fs::write(runtime.editor_source()?, data).map_err(|error| error.to_string())?;

    // 原版标注器是围绕光标的紧凑窗口；在页面切换时隐藏全屏选区窗口并调整到同样的尺寸。
    let monitor = window
        .current_monitor()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "无法定位截图显示器".to_owned())?;
    let scale = window.scale_factor().map_err(|error| error.to_string())?;
    let available_width = f64::from(monitor.size().width) / scale;
    let available_height = f64::from(monitor.size().height) / scale;
    let image_width = f64::from(image.width()) / scale;
    let image_height = f64::from(image.height()) / scale;
    let content_width_limit = (available_width - 80.0)
        .min((image_height * 5.0).max(620.0))
        .max(1.0);
    let content_height_limit = (available_height - 140.0).max(1.0);
    let view_scale = (content_width_limit / image_width)
        .min(content_height_limit / image_height)
        .min(1.0);
    let width = (image_width * view_scale + 24.0)
        .max(660.0)
        .min(available_width);
    let height = (image_height * view_scale + 82.0)
        .max(260.0)
        .min(available_height);
    let cursor = window
        .app_handle()
        .cursor_position()
        .map_err(|error| error.to_string())?;
    let origin = monitor.position();
    let max_x = origin.x + monitor.size().width as i32 - (width * scale).round() as i32;
    let max_y = origin.y + monitor.size().height as i32 - (height * scale).round() as i32;
    let x = (cursor.x as i32 - (width * scale / 2.0).round() as i32).clamp(origin.x, max_x);
    let y = (cursor.y as i32 - (height * scale / 2.0).round() as i32).clamp(origin.y, max_y);
    window.hide().map_err(|error| error.to_string())?;
    window
        .set_size(LogicalSize::new(width, height))
        .and_then(|_| window.set_position(PhysicalPosition::new(x, y)))
        .map_err(|error| error.to_string())
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

/// 在源图加载并完成首帧绘制后显示截图编辑器，避免空白窗口闪烁。
#[tauri::command]
pub(crate) fn screenshot_editor_ready(window: WebviewWindow) -> Result<(), String> {
    require_editor(&window)?;
    window
        .show()
        .and_then(|_| window.set_focus())
        .map_err(|error| error.to_string())
}

/**
 * 打开原版截图的保存对话框，把标注后的 PNG 写到用户指定位置。
 * @param request 编辑器提交的原始 PNG。
 * @param app 桌面宿主句柄。
 * @param window 发起保存的截图窗口。
 * @returns 保存后的路径；取消选择时返回空字符串。
 */
#[tauri::command]
pub(crate) async fn screenshot_save(
    request: Request<'_>,
    app: AppHandle,
    window: WebviewWindow,
) -> Result<String, String> {
    require_editor(&window)?;
    let data = raw_png(&request)?;
    let pictures = app
        .path()
        .picture_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    let destination =
        tauri::async_runtime::spawn_blocking(move || -> Result<Option<PathBuf>, String> {
            // 原版保存操作由用户明确选路径；校验、对话框和磁盘写入全部留在阻塞线程。
            validate_png(&data)?;
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or_default();
            let selected = rfd::FileDialog::new()
                .set_title("保存截图")
                .set_directory(&pictures)
                .set_file_name(format!("ZTools-Screenshot-{timestamp}.png"))
                .add_filter("PNG 图片", &["png"])
                .save_file();
            if let Some(path) = &selected {
                fs::write(path, &data).map_err(|error| format!("无法保存截图：{error}"))?;
            }
            Ok(selected)
        })
        .await
        .map_err(|error| format!("截图保存任务异常结束：{error}"))??;
    let Some(destination) = destination else {
        return Ok(String::new());
    };
    finish_editor(
        &app,
        &window,
        "saved",
        Some(destination.to_string_lossy().as_ref()),
        false,
    )?;
    Ok(destination.to_string_lossy().into_owned())
}

/// 把编辑后的 PNG 写入系统剪贴板并关闭编辑器。
#[tauri::command]
pub(crate) async fn screenshot_copy(
    request: Request<'_>,
    app: AppHandle,
    window: WebviewWindow,
) -> Result<(), String> {
    require_editor(&window)?;
    let data = raw_png(&request)?;
    tauri::async_runtime::spawn_blocking(move || {
        // 图片解码和系统剪贴板写入放到阻塞线程，保持编辑器继续响应窗口事件。
        validate_png(&data)?;
        desktop::write_clipboard_image_png(&data)
    })
    .await
    .map_err(|error| format!("截图复制任务异常结束：{error}"))??;
    finish_editor(&app, &window, "copied", None, false)
}

/**
 * 将编辑器的二进制 PNG 发布为等比缩放贴图并结束选区编辑。
 * @param request PNG 请求体和物理选区坐标请求头。
 * @param app 桌面宿主句柄。
 * @param window 发起操作的编辑器窗口。
 * @returns 贴图创建与编辑器关闭的结果。
 */
#[tauri::command]
pub(crate) async fn screenshot_pin(
    request: Request<'_>,
    app: AppHandle,
    window: WebviewWindow,
) -> Result<(), String> {
    require_editor(&window)?;
    let data = raw_png(&request)?;
    let x = request_number(&request, "x-ztools-selection-x")?;
    let y = request_number(&request, "x-ztools-selection-y")?;
    let width = request_number(&request, "x-ztools-selection-width")?;
    let height = request_number(&request, "x-ztools-selection-height")?;
    if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() {
        return Err("贴图位置或尺寸无效".to_owned());
    }
    let width = width.round().clamp(1.0, 16_384.0) as u32;
    let height = height.round().clamp(1.0, 16_384.0) as u32;
    let editor_position = window.outer_position().map_err(|error| error.to_string())?;
    create_pin(
        &app,
        data,
        PhysicalPosition::new(
            editor_position.x.saturating_add(x.round() as i32),
            editor_position.y.saturating_add(y.round() as i32),
        ),
        PhysicalSize::new(width, height),
    )
    .await?;
    finish_editor(&app, &window, "pinned", None, false)
}

/**
 * 创建保持原图比例、使用滚轮缩放的悬浮窗口。
 * @param app 桌面宿主句柄。
 * @param data PNG 原图。
 * @param position 贴图左上角物理位置。
 * @param size 贴图初始物理尺寸。
 * @returns 创建成功返回 Ok，否则回收窗口并返回错误。
 */
async fn create_pin(
    app: &AppHandle,
    data: Vec<u8>,
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
) -> Result<(), String> {
    let runtime = app.state::<ScreenshotRuntime>();
    let id = runtime.next_id();
    let label = format!("{PIN_LABEL_PREFIX}{id}");
    let path = temporary_path(app, "pin", id)?;
    let write_path = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 校验和写盘在阻塞线程完成，防止大选区冻结 WebView2 窗口。
        validate_png(&data)?;
        fs::write(&write_path, &data).map_err(|error| format!("无法创建贴图文件：{error}"))
    })
    .await
    .map_err(|error| format!("贴图创建任务异常结束：{error}"))??;
    runtime.register_pin(label.clone(), path.clone())?;

    let build_result = WebviewWindowBuilder::new(app, &label, plugin_url("pin.html")?)
        .title("ZTools 贴图")
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .inner_size(f64::from(size.width), f64::from(size.height))
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
        .set_position(position)
        .and_then(|_| pin.set_size(size))
        .map_err(|error| error.to_string())
        .and_then(|_| set_window_transitions(&pin.as_ref().window(), false))
    {
        // 贴图未能显示时回收动态窗口及其临时文件。
        let _ = pin.close();
        cleanup_pin(app, &label);
        return Err(error.to_string());
    }
    Ok(())
}

/**
 * 按原图比例计算限制范围内的窗口尺寸。
 * @param width 原图宽度。
 * @param height 原图高度。
 * @param target_width 希望显示的宽度。
 * @returns 同比例物理尺寸，最短边至少一像素。
 */
fn proportional_size(width: u32, height: u32, target_width: f64) -> PhysicalSize<u32> {
    let ratio = (target_width.max(1.0) / f64::from(width.max(1)))
        .min(16384.0 / f64::from(width.max(height).max(1)));
    PhysicalSize::new(
        (f64::from(width) * ratio).round().max(1.0) as u32,
        (f64::from(height) * ratio).round().max(1.0) as u32,
    )
}

/**
 * 按滚轮倍率等比缩放当前贴图。
 * @param factor 单次缩放倍率。
 * @param app 桌面宿主句柄。
 * @param window 发起缩放的贴图窗口。
 * @returns 调整结束后的结果。
 */
#[tauri::command]
pub(crate) async fn screenshot_pin_resize(
    factor: f64,
    app: AppHandle,
    window: WebviewWindow,
) -> Result<(), String> {
    require_pin(&window)?;
    if !factor.is_finite() || !(0.5..=2.0).contains(&factor) {
        return Err("贴图缩放倍率无效".to_owned());
    }
    let path = app.state::<ScreenshotRuntime>().pin_path(window.label())?;
    let (width, height) = image::image_dimensions(path).map_err(|error| error.to_string())?;
    let current = window.inner_size().map_err(|error| error.to_string())?;
    // 只改变一个缩放倍率，宽高始终从原图尺寸推导，不积累多次缩放的比例误差。
    let size = proportional_size(
        width,
        height,
        (f64::from(current.width) * factor).clamp(32.0, 8192.0),
    );
    window.set_size(size).map_err(|error| error.to_string())
}

/**
 * 设置 Windows 窗口的系统显示隐藏过渡，其他平台保持立即显示逻辑。
 * @param window 需要调整的窗口。
 * @param enabled 是否允许系统过渡。
 * @returns 设置结果。
 */
fn set_window_transitions(window: &Window, enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::ffi::c_void;
        #[link(name = "dwmapi")]
        extern "system" {
            fn DwmSetWindowAttribute(
                hwnd: *mut c_void,
                attribute: u32,
                value: *const c_void,
                size: u32,
            ) -> i32;
        }
        let disabled = i32::from(!enabled);
        let hwnd = window.hwnd().map_err(|error| error.to_string())?;
        // DWMWA_TRANSITIONS_FORCEDISABLED=3；传入有效 HWND 和四字节 BOOL。
        unsafe {
            DwmSetWindowAttribute(hwnd.0, 3, &disabled as *const i32 as *const c_void, 4);
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (window, enabled);
    Ok(())
}

/**
 * 等待 Windows 桌面合成完成当前帧，不附加固定延迟。
 * @returns 无返回值。
 */
fn flush_capture_frame() {
    #[cfg(target_os = "windows")]
    {
        #[link(name = "dwmapi")]
        extern "system" {
            fn DwmFlush() -> i32;
        }
        // DwmFlush 不接收指针，等待合成器提交启动器隐藏这一帧。
        unsafe {
            DwmFlush();
        }
    }
}

/**
 * 打开默认截图插件内的历史图片选择页。
 * @param app 桌面宿主句柄。
 * @param main 发起操作的启动器窗口。
 * @returns 选择窗口创建完成后的结果。
 */
pub(crate) async fn start_history(app: AppHandle, main: Window) -> Result<(), String> {
    if main.label() != "main" {
        return Err("只有启动器可以打开图片历史".to_owned());
    }
    if let Some(existing) = app.get_webview_window(HISTORY_LABEL) {
        existing
            .show()
            .and_then(|_| existing.set_focus())
            .map_err(|e| e.to_string())?;
        return main.hide().map_err(|e| e.to_string());
    }
    let monitor = active_monitor(&app)?;
    let scale = monitor.scale_factor();
    let width = (680.0 * scale).min(f64::from(monitor.size().width));
    let height = (480.0 * scale).min(f64::from(monitor.size().height));
    let picker = WebviewWindowBuilder::new(&app, HISTORY_LABEL, plugin_url("history.html")?)
        .title("ZTools 贴图 · 历史图片")
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .inner_size(width / scale, height / scale)
        .build()
        .map_err(|e| e.to_string())?;
    let prepared = picker
        .set_position(PhysicalPosition::new(
            monitor.position().x + ((f64::from(monitor.size().width) - width) / 2.0) as i32,
            monitor.position().y + ((f64::from(monitor.size().height) - height) / 2.0) as i32,
        ))
        .map_err(|e| e.to_string())
        .and_then(|_| set_window_transitions(&picker.as_ref().window(), false));
    if let Err(error) = prepared {
        let _ = picker.close();
        return Err(error);
    }
    // 图片列表加载前即可显示空状态，窗口本身不做淡入淡出。
    main.hide().map_err(|e| e.to_string())?;
    if let Err(error) = picker.show().and_then(|_| picker.set_focus()) {
        let _ = picker.close();
        crate::commands::launcher::show_main_window(&app);
        return Err(error.to_string());
    }
    Ok(())
}

/**
 * 验证调用者为历史图片选择页。
 * @param window 当前调用窗口。
 * @returns 身份匹配返回 Ok，否则拒绝访问图片历史。
 */
fn require_history(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == HISTORY_LABEL {
        Ok(())
    } else {
        Err("当前窗口不是图片历史选择页".to_owned())
    }
}

/**
 * 捕获当前图片并返回本地历史预览。
 * @param app 桌面宿主句柄。
 * @param window 图片选择页窗口。
 * @returns 历史图片元数据和缩略图列表。
 */
#[tauri::command]
pub(crate) async fn screenshot_history_list(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Vec<crate::models::ClipboardImageEntry>, String> {
    require_history(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        // 临时剪贴板占用不妨碍读取已经保存的历史。
        let _ = crate::clipboard_images::capture_current(&app);
        let state = app.state::<crate::state::AppState>();
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let settings = store.settings()?;
        store.prune_clipboard(
            settings.clipboard_retention_days,
            crate::commands::launcher::current_timestamp()?,
        )?;
        store.clipboard_images()
    })
    .await
    .map_err(|e| e.to_string())?
}

/**
 * 把选中的历史原图显示为悬浮贴图，初始尺寸适配选择页所在屏幕。
 * @param id 历史图片标识。
 * @param app 桌面宿主句柄。
 * @param window 图片选择页窗口。
 * @returns 贴图窗口创建及选择页关闭后的结果。
 */
#[tauri::command]
pub(crate) async fn screenshot_history_pin(
    id: i64,
    app: AppHandle,
    window: WebviewWindow,
) -> Result<(), String> {
    require_history(&window)?;
    let read_app = app.clone();
    let (data, width, height) = tauri::async_runtime::spawn_blocking(move || {
        let data = read_app
            .state::<crate::state::AppState>()
            .store
            .lock()
            .map_err(|e| e.to_string())?
            .clipboard_image_png(id)?;
        let image = decode_png(&data)?;
        Ok::<_, String>((data, image.width(), image.height()))
    })
    .await
    .map_err(|e| e.to_string())??;
    let monitor = window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "无法获取贴图所在显示器".to_owned())?;
    let scale = (f64::from(monitor.size().width) * 0.8 / f64::from(width))
        .min(f64::from(monitor.size().height) * 0.8 / f64::from(height))
        .min(1.0);
    let size = proportional_size(width, height, f64::from(width) * scale);
    let position = PhysicalPosition::new(
        monitor.position().x + ((monitor.size().width - size.width) / 2) as i32,
        monitor.position().y + ((monitor.size().height - size.height) / 2) as i32,
    );
    create_pin(&app, data, position, size).await?;
    window.close().map_err(|e| e.to_string())
}

/**
 * 取消历史图片选择并返回启动器。
 * @param app 桌面宿主句柄。
 * @param window 图片选择页窗口。
 * @returns 关闭操作结果。
 */
#[tauri::command]
pub(crate) fn screenshot_history_close(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<(), String> {
    require_history(&window)?;
    window.close().map_err(|e| e.to_string())?;
    crate::commands::launcher::show_main_window(&app);
    Ok(())
}

/// 在贴图图片解码完成后显示窗口，避免先弹出黑色空窗。
#[tauri::command]
pub(crate) fn screenshot_pin_ready(window: WebviewWindow) -> Result<(), String> {
    require_pin(&window)?;
    window
        .show()
        .and_then(|_| window.set_focus())
        .map_err(|error| error.to_string())
}

/// 取消截图并关闭编辑器。
#[tauri::command]
pub(crate) fn screenshot_cancel(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    require_editor(&window)?;
    finish_editor(&app, &window, "cancelled", None, true)
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

/// 从 Tauri 原始二进制请求中读取 PNG，避免把每个字节编码成 JSON 数字。
fn raw_png(request: &Request<'_>) -> Result<Vec<u8>, String> {
    match request.body() {
        InvokeBody::Raw(data) => Ok(data.clone()),
        InvokeBody::Json(_) => Err("截图必须使用二进制数据传输".to_owned()),
    }
}

/// 从二进制请求头读取有限浮点数形式的选区参数。
fn request_number(request: &Request<'_>, name: &str) -> Result<f64, String> {
    let value = request
        .headers()
        .get(name)
        .ok_or_else(|| format!("截图请求缺少 {name}"))?
        .to_str()
        .map_err(|_| format!("截图请求头 {name} 无效"))?
        .parse::<f64>()
        .map_err(|_| format!("截图请求头 {name} 不是数字"))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("截图请求头 {name} 不是有限数字"))
    }
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
    restore_main: bool,
) -> Result<(), String> {
    cleanup_editor_source(app);
    let _ = app.emit_to(
        "main",
        "screenshot-finished",
        serde_json::json!({ "action": action, "path": path }),
    );
    let close_result = window.close().map_err(|error| error.to_string());
    if restore_main {
        // 用户取消时恢复原启动器；成功输出后保持收起，避免额外弹窗抢焦点。
        crate::commands::launcher::show_main_window(app);
    }
    close_result
}

/// 删除当前编辑器源图。
fn cleanup_editor_source(app: &AppHandle) -> bool {
    if let Some(path) = app.state::<ScreenshotRuntime>().take_editor_source() {
        let _ = fs::remove_file(path);
        return true;
    }
    false
}

/// 删除指定贴图的临时文件和运行时记录。
fn cleanup_pin(app: &AppHandle, label: &str) {
    if let Some(path) = app.state::<ScreenshotRuntime>().remove_pin(label) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::{proportional_size, validate_png, MAX_SCREENSHOT_BYTES};

    /**
     * 验证横图、竖图及缩小图片在缩放上限附近仍保持原图比例。
     * @returns 无返回值。
     */
    #[test]
    fn keeps_pin_aspect_ratio() {
        for (width, height, target) in [
            (1600, 900, 800.0),
            (900, 1600, 1800.0),
            (100, 400, 8192.0),
            (16, 9, 32.0),
        ] {
            let size = proportional_size(width, height, target);
            assert!(size.width > 0 && size.height > 0);
            assert!(size.width <= 16384 && size.height <= 16384);
            let error = (f64::from(size.height)
                - f64::from(size.width) * f64::from(height) / f64::from(width))
            .abs();
            assert!(error <= 1.0);
        }
    }

    /// 拒绝空数据和超过上限的截图请求。
    #[test]
    fn rejects_invalid_screenshot_payloads() {
        assert!(validate_png(&[]).is_err());
        assert!(validate_png(&vec![0; MAX_SCREENSHOT_BYTES + 1]).is_err());
    }
}
