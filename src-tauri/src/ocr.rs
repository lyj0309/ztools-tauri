use std::io::Cursor;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{
    ipc::{InvokeBody, Request},
    Manager, WebviewWindow,
};

static OCR_BUSY: Mutex<()> = Mutex::new(());

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OcrResult {
    text: String,
    language: String,
    engine: &'static str,
}

/**
 * 仅接受截图编辑器或已注册插件的本地图像识别调用。
 * @param window 发起调用的 Webview。
 * @returns 校验结果。
 */
fn require_plugin(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "plugin-screenshot" {
        return Ok(());
    }
    window
        .app_handle()
        .state::<crate::plugin::PluginRuntime>()
        .plugin_for_window(window.label())
        .map(|_| ())
}

/**
 * 在工作线程识别二进制 PNG，避免系统 OCR 阻塞窗口事件循环。
 * @param request PNG 请求体，可通过 x-ztools-ocr-language 指定语言。
 * @param window 发起识别的插件窗口。
 * @returns 识别文本、实际语言及本地引擎名称。
 */
#[tauri::command]
pub(crate) async fn plugin_ocr(
    request: Request<'_>,
    window: WebviewWindow,
) -> Result<OcrResult, String> {
    require_plugin(&window)?;
    let data = match request.body() {
        InvokeBody::Raw(data) if !data.is_empty() && data.len() <= 32 * 1024 * 1024 => data.clone(),
        _ => return Err("OCR 需要不超过 32 MB 的 PNG 图片".to_owned()),
    };
    let language = request
        .headers()
        .get("x-ztools-ocr-language")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("auto")
        .to_owned();
    if !matches!(
        language.as_str(),
        "auto" | "zh-Hans" | "zh-Hant" | "en-US" | "ja-JP" | "ko-KR"
    ) {
        return Err("OCR 语言无效".to_owned());
    }
    tauri::async_runtime::spawn_blocking(move || {
        // 串行限制识别任务；不让连续点击或多个插件堆积大图解码。
        let _guard = OCR_BUSY
            .try_lock()
            .map_err(|_| "正在识别其他图片，请稍后再试".to_owned())?;
        let image = decode_image(&data)?;
        recognize(image, &language)
    })
    .await
    .map_err(|error| error.to_string())?
}

/**
 * 校验 PNG 尺寸后解码，防止压缩小图展开成超大像素缓冲区。
 * @param data 待识别 PNG。
 * @returns 已限制内存和尺寸的图像。
 */
fn decode_image(data: &[u8]) -> Result<image::DynamicImage, String> {
    let reader = image::ImageReader::with_format(Cursor::new(data), image::ImageFormat::Png);
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| format!("图片无效：{error}"))?;
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || u64::from(width) * u64::from(height) > 32_000_000
    {
        return Err("OCR 图片过大，请缩小选区后重试".to_owned());
    }
    image::load_from_memory_with_format(data, image::ImageFormat::Png)
        .map_err(|error| error.to_string())
}

/**
 * 复制用户编辑后的识别结果，保留截图界面以便继续操作。
 * @param text 需要复制的识别文本。
 * @param window 发起复制的插件窗口。
 * @returns 剪贴板写入结果。
 */
#[tauri::command]
pub(crate) async fn plugin_ocr_copy_text(
    text: String,
    window: WebviewWindow,
) -> Result<(), String> {
    require_plugin(&window)?;
    if text.len() > 1024 * 1024 {
        return Err("识别文本过长".to_owned());
    }
    tauri::async_runtime::spawn_blocking(move || crate::desktop::write_clipboard_text(text))
        .await
        .map_err(|error| error.to_string())?
}

#[cfg(target_os = "windows")]
/**
 * 使用系统 WinRT OCR 和已安装语言包完成离线识别。
 * @param image 已校验的截图。
 * @param language auto 或 Windows 语言标签。
 * @returns 本地识别结果，未安装语言时给出可操作错误。
 */
fn recognize(image: image::DynamicImage, language: &str) -> Result<OcrResult, String> {
    use windows::{
        core::HSTRING,
        Globalization::Language,
        Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
        Media::Ocr::OcrEngine,
        Storage::Streams::DataWriter,
        Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
    };
    // 专用工作线程按 MTA 初始化并在所有 COM 对象释放后配对清理。
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(|error| error.to_string())?;
    let result = (|| -> windows::core::Result<OcrResult> {
        let engine = if language == "auto" {
            OcrEngine::TryCreateFromUserProfileLanguages()?
        } else {
            OcrEngine::TryCreateFromLanguage(&Language::CreateLanguage(&HSTRING::from(language))?)?
        };
        let max = OcrEngine::MaxImageDimension()?;
        let image = if image.width() > max || image.height() > max {
            image.thumbnail(max, max)
        } else {
            image
        };
        let pixels = image.to_luma8();
        let writer = DataWriter::new()?;
        writer.WriteBytes(pixels.as_raw())?;
        let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
            &writer.DetachBuffer()?,
            BitmapPixelFormat::Gray8,
            pixels.width() as i32,
            pixels.height() as i32,
            BitmapAlphaMode::Ignore,
        )?;
        let recognized = engine.RecognizeAsync(&bitmap)?.get()?;
        let mut lines = Vec::new();
        for line in recognized.Lines()? {
            lines.push(line.Text()?.to_string());
        }
        Ok(OcrResult {
            text: lines.join("\n"),
            language: engine.RecognizerLanguage()?.LanguageTag()?.to_string(),
            engine: "Windows OCR",
        })
    })();
    unsafe {
        RoUninitialize();
    }
    result.map_err(|error| format!("Windows OCR 无法识别：{error}。请在 Windows 设置中安装对应语言的文字识别功能，或切换识别语言。"))
}

#[cfg(not(target_os = "windows"))]
/**
 * 使用本机 Tesseract 离线识别，不附带大型模型或常驻运行时。
 * @param image 已校验的截图。
 * @param language auto 或界面语言标签。
 * @returns 本地识别结果；缺少依赖或超时返回明确错误。
 */
fn recognize(image: image::DynamicImage, language: &str) -> Result<OcrResult, String> {
    use std::{
        io::Write,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let langs = Command::new("tesseract")
        .arg("--list-langs")
        .output()
        .map_err(|_| {
            "当前系统未安装 Tesseract。请安装 tesseract 及所需语言包后使用本地 OCR。".to_owned()
        })?;
    let available = String::from_utf8_lossy(&langs.stdout);
    let language = match language {
        "zh-Hans" => "chi_sim",
        "zh-Hant" => "chi_tra",
        "ja-JP" => "jpn",
        "ko-KR" => "kor",
        "en-US" => "eng",
        _ => {
            if available.lines().any(|line| line == "chi_sim") {
                "chi_sim+eng"
            } else {
                "eng"
            }
        }
    };
    for part in language.split('+') {
        if !available.lines().any(|line| line == part) {
            return Err(format!("请安装 Tesseract 的 {part} 语言包"));
        }
    }
    let mut png = Cursor::new(Vec::new());
    image
        .thumbnail(4096, 4096)
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    let mut child = Command::new("tesseract")
        .args(["stdin", "stdout", "-l", language, "--psm", "11"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "无法打开 OCR 输入管道".to_owned())?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法打开 OCR 输出管道".to_owned())?;
    // 并发消费管道，避免大图写入或长文本输出堵塞超时检测。
    let input = std::thread::spawn(move || stdin.write_all(&png.into_inner()));
    let output = std::thread::spawn(move || {
        use std::io::Read;
        let mut data = String::new();
        stdout.read_to_string(&mut data).map(|_| data)
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break Ok(status);
        }
        if started.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            break Err("识别超时，请缩小选区后重试".to_owned());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let _ = input.join();
    let text = output
        .join()
        .map_err(|_| "OCR 输出线程异常".to_owned())?
        .map_err(|error| error.to_string())?;
    if !status?.success() {
        return Err("识别失败，请检查 Tesseract 语言包".to_owned());
    }
    Ok(OcrResult {
        text: text.trim().to_owned(),
        language: language.to_owned(),
        engine: "Tesseract",
    })
}

#[cfg(test)]
mod tests {
    /**
     * 在装有本地 OCR 的真实系统上识别固定文字，验证图像格式和语言接入。
     * @returns 无返回值。
     */
    #[test]
    #[ignore = "requires an installed local English OCR language pack"]
    fn recognizes_local_fixture() {
        let image = super::decode_image(include_bytes!("../tests/fixtures/ocr.png")).unwrap();
        let result = super::recognize(image, "en-US").unwrap();
        assert!(result.text.contains("ZTOOLS"), "{}", result.text);
        assert!(result.text.contains("2026"), "{}", result.text);
    }

    /**
     * 拒绝无效输入并检查正常 PNG 可以进入识别流程。
     * @returns 无返回值。
     */
    #[test]
    fn validates_ocr_images() {
        assert!(super::decode_image(b"invalid").is_err());
        let mut data = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(20, 10)
            .write_to(&mut data, image::ImageFormat::Png)
            .unwrap();
        assert_eq!(super::decode_image(data.get_ref()).unwrap().width(), 20);
    }
}
