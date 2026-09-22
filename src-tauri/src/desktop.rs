use std::{
    borrow::Cow,
    fs,
    path::PathBuf,
    process::Command,
    sync::{Mutex, MutexGuard},
};

use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use tauri::{AppHandle, Manager};

static SYSTEM_CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

/// 获取进程级剪贴板句柄，使 Linux 的 X11/Wayland 选择所有权在写入后继续存活。
fn system_clipboard() -> Result<MutexGuard<'static, Option<arboard::Clipboard>>, String> {
    let mut clipboard = SYSTEM_CLIPBOARD
        .lock()
        .map_err(|_| "剪贴板锁已损坏".to_owned())?;
    if clipboard.is_none() {
        // 初始化失败时保留空状态，后续桌面会话恢复后仍可重试。
        *clipboard = Some(arboard::Clipboard::new().map_err(|error| error.to_string())?);
    }
    Ok(clipboard)
}

/// 读取系统剪贴板中的纯文本，非文本内容返回空结果。
pub(crate) fn read_clipboard_text() -> Result<Option<String>, String> {
    let mut clipboard = system_clipboard()?;
    let clipboard = clipboard
        .as_mut()
        .ok_or_else(|| "系统剪贴板不可用".to_owned())?;
    match clipboard.get_text() {
        Ok(content) if !content.trim().is_empty() => Ok(Some(content)),
        Ok(_) => Ok(None),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// 把纯文本写入操作系统剪贴板。
pub(crate) fn write_clipboard_text(content: String) -> Result<(), String> {
    if content.len() > 2_000_000 {
        return Err("剪贴板文本不能超过 2 MB".to_owned());
    }
    let mut clipboard = system_clipboard()?;
    clipboard
        .as_mut()
        .ok_or_else(|| "系统剪贴板不可用".to_owned())?
        .set_text(content)
        .map_err(|error| error.to_string())
}

/// 解码 PNG 并把 RGBA 像素写入系统图片剪贴板。
pub(crate) fn write_clipboard_image_png(data: &[u8]) -> Result<(), String> {
    let image = image::load_from_memory_with_format(data, image::ImageFormat::Png)
        .map_err(|error| format!("截图 PNG 无效：{error}"))?
        .into_rgba8();
    let (width, height) = image.dimensions();
    let mut clipboard = system_clipboard()?;
    clipboard
        .as_mut()
        .ok_or_else(|| "系统剪贴板不可用".to_owned())?
        .set_image(arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: Cow::Owned(image.into_raw()),
        })
        .map_err(|error| error.to_string())
}

/// 使用原生输入后端发送当前平台的粘贴组合键。
pub(crate) fn simulate_paste() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|error| error.to_string())?;
    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;

    // 分开按下和释放修饰键，失败时 Enigo 的 Drop 会释放仍保持的按键。
    enigo
        .key(modifier, Direction::Press)
        .map_err(|error| error.to_string())?;
    let click_result = enigo
        .key(Key::Unicode('v'), Direction::Click)
        .map_err(|error| error.to_string());
    let release_result = enigo
        .key(modifier, Direction::Release)
        .map_err(|error| error.to_string());
    click_result.and(release_result)
}

/// 使用原生输入后端输入一段 Unicode 文本。
pub(crate) fn type_text(content: &str) -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|error| error.to_string())?;
    enigo.text(content).map_err(|error| error.to_string())
}

/// 点击一枚受支持的基础键，拒绝任意原始键码。
pub(crate) fn tap_key(name: &str) -> Result<(), String> {
    let key = match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => Key::Return,
        "escape" | "esc" => Key::Escape,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "arrowup" | "up" => Key::UpArrow,
        "arrowdown" | "down" => Key::DownArrow,
        "arrowleft" | "left" => Key::LeftArrow,
        "arrowright" | "right" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        _ => return Err("只支持 Enter、Escape、Tab、编辑键、方向键和翻页键".to_owned()),
    };
    let mut enigo = Enigo::new(&Settings::default()).map_err(|error| error.to_string())?;
    enigo
        .key(key, Direction::Click)
        .map_err(|error| error.to_string())
}

/// 截取鼠标所在显示器并把 PNG 保存到系统图片目录。
pub(crate) fn capture_screen(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .picture_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    fs::create_dir_all(&directory).map_err(|error| format!("无法创建截图目录：{error}"))?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let output = directory.join(format!("ZTools-Screenshot-{timestamp}.png"));

    capture_screen_to(app, &output)?;
    Ok(output)
}

/// 把鼠标所在显示器截取到宿主生成的目标路径。
pub(crate) fn capture_screen_to(app: &AppHandle, output: &PathBuf) -> Result<(), String> {
    // 平台实现只接收宿主生成的输出路径，不拼接用户输入或交给 shell 解析。
    capture_screen_platform(app, output)?;
    if !output.is_file() || fs::metadata(output).map_or(0, |metadata| metadata.len()) == 0 {
        return Err("截图工具没有生成有效图片".to_owned());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
/// 在 Linux 上优先使用桌面截图工具，X11 环境回退到 FFmpeg x11grab。
fn capture_screen_platform(app: &AppHandle, output: &PathBuf) -> Result<(), String> {
    let candidates: [(&str, &[&str]); 4] = [
        ("grim", &[]),
        ("gnome-screenshot", &["-f"]),
        ("spectacle", &["-b", "-n", "-o"]),
        ("flameshot", &["full", "-p"]),
    ];
    for (program, arguments) in candidates {
        let result = Command::new(program).args(arguments).arg(output).status();
        if result.as_ref().is_ok_and(|status| status.success()) {
            return Ok(());
        }
    }

    let cursor = app.cursor_position().map_err(|error| error.to_string())?;
    let monitor = app
        .monitor_from_point(cursor.x, cursor.y)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "无法确定鼠标所在显示器".to_owned())?;
    let display = std::env::var("DISPLAY").map_err(|_| {
        "当前 Linux 会话缺少可用截图门户或 DISPLAY；请安装 grim、gnome-screenshot、spectacle 或 flameshot"
            .to_owned()
    })?;
    let position = monitor.position();
    let size = monitor.size();
    let input = format!("{display}{:+},{}", position.x, position.y);
    command_result(
        Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "x11grab",
                "-video_size",
                &format!("{}x{}", size.width, size.height),
                "-i",
                &input,
                "-frames:v",
                "1",
                "-y",
            ])
            .arg(output)
            .status(),
    )
}

#[cfg(target_os = "macos")]
/// 使用 macOS 自带交互式截图工具保存 PNG。
fn capture_screen_platform(_app: &AppHandle, output: &PathBuf) -> Result<(), String> {
    command_result(
        Command::new("screencapture")
            .args(["-i", "-r"])
            .arg(output)
            .status(),
    )
}

#[cfg(target_os = "windows")]
/// 使用 Windows 系统程序集截取鼠标所在显示器并保存 PNG。
fn capture_screen_platform(_app: &AppHandle, output: &PathBuf) -> Result<(), String> {
    const SCRIPT: &str = "Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $o=[Environment]::GetEnvironmentVariable('ZTOOLS_SCREENSHOT_OUTPUT'); $b=[System.Windows.Forms.Screen]::FromPoint([System.Windows.Forms.Cursor]::Position).Bounds; $i=New-Object System.Drawing.Bitmap $b.Width,$b.Height; $g=[System.Drawing.Graphics]::FromImage($i); $g.CopyFromScreen($b.Location,[System.Drawing.Point]::Empty,$b.Size); $i.Save($o,[System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $i.Dispose()";
    command_result(
        Command::new("powershell")
            // 通过子进程环境传入路径，避免 -Command 把 Windows 路径继续解析为脚本源码。
            .env("ZTOOLS_SCREENSHOT_OUTPUT", output)
            .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
            .status(),
    )
}

/// 校验用户拖入的路径并交给当前操作系统打开。
pub(crate) fn open_path(path: String) -> Result<(), String> {
    let path = canonical_path(path)?;
    run_open_command(&path, false)
}

/// 校验路径并在系统文件管理器中定位它。
pub(crate) fn reveal_path(path: String) -> Result<(), String> {
    let path = canonical_path(path)?;
    run_open_command(&path, true)
}

/// 使用系统默认浏览器打开经过校验的 HTTP 或 HTTPS 地址。
pub(crate) fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("只允许打开 HTTP 或 HTTPS 地址".to_owned());
    }
    #[cfg(target_os = "linux")]
    let result = Command::new("gio").arg("open").arg(&url).status();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(&url).status();
    #[cfg(target_os = "windows")]
    let result = Command::new("cmd").args(["/C", "start", "", &url]).status();
    command_result(result)
}

/// 规范化外部路径，拒绝不存在的目标。
pub(crate) fn canonical_path(path: String) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(path);
    if !candidate.exists() {
        return Err("文件或目录不存在".to_owned());
    }
    candidate.canonicalize().map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
/// 使用 Linux 桌面默认应用打开目标或其父目录。
fn run_open_command(path: &PathBuf, reveal: bool) -> Result<(), String> {
    let target = if reveal && path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    command_result(Command::new("gio").arg("open").arg(target).status())
}

#[cfg(target_os = "macos")]
/// 使用 macOS Launch Services 打开或定位目标。
fn run_open_command(path: &PathBuf, reveal: bool) -> Result<(), String> {
    let mut command = Command::new("open");
    if reveal {
        command.arg("-R");
    }
    command_result(command.arg(path).status())
}

#[cfg(target_os = "windows")]
/// 使用 Windows Shell 打开或定位目标。
fn run_open_command(path: &PathBuf, reveal: bool) -> Result<(), String> {
    let mut command = Command::new("explorer");
    if reveal && path.is_file() {
        command.arg(format!("/select,{}", path.display()));
    } else {
        command.arg(path);
    }
    command_result(command.status())
}

/// 将系统命令退出状态转换为可展示的结果。
fn command_result(result: std::io::Result<std::process::ExitStatus>) -> Result<(), String> {
    let status = result.map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("系统打开操作失败，返回 {status}"))
    }
}
