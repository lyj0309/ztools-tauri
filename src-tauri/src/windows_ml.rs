use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

use image::DynamicImage;
use libloading::Library;
use ppocr_rs::OcrLite;
use serde_yaml::Value;
use sha2::{Digest, Sha256};
use tauri::{Manager, WebviewWindow};

use crate::ocr::OcrResult;

const DET_URL: &str = "https://paddle-model-ecology.bj.bcebos.com/paddlex/official_inference_model/paddle3.0.0/tmp/PP-OCRv6_tiny_det_onnx.tar";
const REC_URL: &str = "https://paddle-model-ecology.bj.bcebos.com/paddlex/official_inference_model/paddle3.0.0/tmp/PP-OCRv6_tiny_rec_0515_onnx.tar";
const DET_SHA256: &str = "220c0bd1074bd9415434f8b08339bb559c41a2ffd33c3ea34ded6a5fc63158b6";
const REC_SHA256: &str = "71a9a34ee45cb376cfdf8a849eb0cc66755e59baaccb6b94045d9ba6e5b6d012";
const DET_ONNX_SHA256: &str = "a56a3430a96a6c691f8bfbbb297208bb9573c3d70083d6382071cbd92d0a152e";
const REC_ONNX_SHA256: &str = "d5de4cb712dc90158c4f966e7ed25b87b5d16a1ab7c8e14b9f9c2b4aa269dd36";
const DET_PREFIX: &str = "PP-OCRv6_tiny_det_onnx";
const REC_PREFIX: &str = "PP-OCRv6_tiny_rec_0515_onnx";
// Windows App SDK 1.8.1 的 MSIX 运行时版本为 8000.625.330.0。
const MIN_WINDOWS_ML_RUNTIME: u64 = (8000_u64 << 48) | (625_u64 << 32) | (330_u64 << 16);
static WINDOWS_ML_READY: OnceLock<bool> = OnceLock::new();
static MODEL_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

pub(crate) struct ModelPaths {
    det: PathBuf,
    rec: PathBuf,
    dict: PathBuf,
}

/**
 * 在桌面启动前尝试挂载 Windows App SDK 1.8 运行时；失败时保留 WinRT OCR。
 * @returns 无返回值。
 */
pub(crate) fn initialize() {
    WINDOWS_ML_READY.get_or_init(|| match initialize_runtime() {
        Ok(()) => true,
        Err(error) => {
            eprintln!("[ocr] Windows ML unavailable, using WinRT OCR: {error}");
            false
        }
    });
}

/**
 * 判断共享 Windows ML 运行时是否已在当前进程中初始化。
 * @returns 运行时是否可用。
 */
pub(crate) fn is_available() -> bool {
    *WINDOWS_ML_READY.get().unwrap_or(&false)
}

/**
 * 挂载系统安装的 Windows App SDK 1.8，并验证其中包含 ONNX Runtime。
 * @returns 成功时无返回值。
 * @throws 系统未安装兼容运行时或 DLL 无法加载。
 */
fn initialize_runtime() -> Result<(), String> {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

    // Bootstrap 必须在调用 Windows App SDK API 前执行；应用主线程已有 COM 时复用其公寓。
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let bootstrap_path = bootstrap_path()?;
    let library = unsafe { Library::new(&bootstrap_path) }.map_err(|error| error.to_string())?;
    unsafe {
        type Initialize = unsafe extern "system" fn(u32, *const u16, u64, u32) -> i32;
        let initialize: libloading::Symbol<Initialize> = library
            .get(b"MddBootstrapInitialize2")
            .map_err(|error| error.to_string())?;
        // 精确要求 1.8.1 及以上，再探测共享 ONNX Runtime 是否能被加载。
        let hr = initialize(0x0001_0008, std::ptr::null(), MIN_WINDOWS_ML_RUNTIME, 0);
        if hr < 0 {
            return Err(format!("Windows App SDK 1.8 bootstrap failed: 0x{hr:08x}"));
        }
    }
    // 保持 bootstrap DLL 与动态依赖在整个进程生命周期内有效。
    std::mem::forget(library);
    let ort = unsafe { Library::new("onnxruntime.dll") }.map_err(|error| error.to_string())?;
    std::mem::forget(ort);
    Ok(())
}

/**
 * 将随便携版内嵌的 Microsoft bootstrap DLL 放到用户缓存目录。
 * @returns 可供 Windows 加载器使用的 DLL 路径。
 * @throws 缓存目录不可写。
 */
fn bootstrap_path() -> Result<PathBuf, String> {
    const BYTES: &[u8] =
        include_bytes!("../resources/windows/Microsoft.WindowsAppRuntime.Bootstrap.dll");
    let cache = if std::env::var("ZTOOLS_E2E").as_deref() == Ok("1") {
        std::env::var_os("ZTOOLS_DATA_ROOT")
            .map(PathBuf::from)
            .ok_or_else(|| "E2E Windows ML 缺少隔离数据目录".to_owned())?
            .join("cache")
    } else {
        dirs::cache_dir().ok_or_else(|| "无法定位 Windows 用户缓存目录".to_owned())?
    };
    let root = cache
        .join("ztools-tauri")
        .join("windows-ml")
        .join("bootstrap-1.8.1");
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let path = root.join("Microsoft.WindowsAppRuntime.Bootstrap.dll");
    if fs::read(&path).map(|bytes| bytes != BYTES).unwrap_or(true) {
        let pending = root.join("bootstrap.pending");
        fs::write(&pending, BYTES).map_err(|error| error.to_string())?;
        if path.exists() {
            fs::remove_file(&path).map_err(|error| error.to_string())?;
        }
        fs::rename(&pending, &path).map_err(|error| error.to_string())?;
    }
    Ok(path)
}

/**
 * 在第一次需要 Windows ML 时下载并校验官方 tiny 模型，随后复用本地缓存。
 * @param window 发起 OCR 的窗口，用于定位隔离的测试缓存。
 * @returns 检测、识别和字典的本地路径。
 * @throws 下载、校验或缓存失败。
 */
pub(crate) async fn ensure_models(window: &WebviewWindow) -> Result<ModelPaths, String> {
    let root = if std::env::var("ZTOOLS_E2E").as_deref() == Ok("1") {
        std::env::var_os("ZTOOLS_DATA_ROOT")
            .map(PathBuf::from)
            .ok_or_else(|| "E2E OCR 缺少隔离数据目录".to_owned())?
            .join("cache")
    } else {
        window
            .app_handle()
            .path()
            .app_cache_dir()
            .map_err(|error| error.to_string())?
    }
    .join("ocr")
    .join("ppocrv6-tiny");
    ensure_models_in(&root).await
}

/**
 * 在给定缓存目录准备官方 tiny 模型，供桌面调用和隔离测试复用。
 * @param root 模型缓存目录。
 * @returns 三个推理文件路径。
 * @throws 下载、解包或缓存失败。
 */
async fn ensure_models_in(root: &Path) -> Result<ModelPaths, String> {
    // 串行首次下载与缓存发布，避免多个 OCR 请求同时改写相同模型文件。
    let _guard = MODEL_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let det = root.join("det.onnx");
    let rec = root.join("rec.onnx");
    let yml = root.join("rec.yml");
    let dict = root.join("dict.txt");

    // 只复用完整缓存；下载文件使用固定哈希校验，避免损坏或被替换的模型投入推理。
    if !valid_model_cache(&det, &rec, &yml, &dict) {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(35))
            .build()
            .map_err(|error| error.to_string())?;
        let det_tar = download_archive(&client, DET_URL, DET_SHA256, 3_000_000).await?;
        let rec_tar = download_archive(&client, REC_URL, REC_SHA256, 6_000_000).await?;
        let det_onnx = extract_entry(&det_tar, DET_PREFIX, "inference.onnx")?;
        let rec_onnx = extract_entry(&rec_tar, REC_PREFIX, "inference.onnx")?;
        let rec_yml = extract_entry(&rec_tar, REC_PREFIX, "inference.yml")?;
        let dictionary = dictionary_from_yaml(&rec_yml)?;
        // 每个文件先写临时路径再发布；下次启动只接受四个文件齐全的缓存。
        publish(&det, &det_onnx)?;
        publish(&rec, &rec_onnx)?;
        publish(&yml, &rec_yml)?;
        publish(&dict, dictionary.as_bytes())?;
    }
    Ok(ModelPaths { det, rec, dict })
}

/**
 * 检查上次下载的模型是否具备可推理的基本文件集合。
 * @param det 检测模型路径。
 * @param rec 识别模型路径。
 * @param yml 识别模型配置路径。
 * @param dict 字符字典路径。
 * @returns 模型哈希匹配且配置、字典非空时为真。
 */
fn valid_model_cache(det: &Path, rec: &Path, yml: &Path, dict: &Path) -> bool {
    // 每次复用前检查模型哈希，以便自动修复不完整或被改写的缓存。
    let model_matches =
        [(det, DET_ONNX_SHA256), (rec, REC_ONNX_SHA256)]
            .iter()
            .all(|(path, expected)| {
                fs::read(path)
                    .map(|bytes| format!("{:x}", Sha256::digest(bytes)) == *expected)
                    .unwrap_or(false)
            });
    model_matches
        && [yml, dict].iter().all(|path| {
            fs::metadata(path)
                .map(|meta| meta.len() > 0)
                .unwrap_or(false)
        })
}

/**
 * 下载尺寸受限的官方模型归档并核对固定 SHA-256。
 * @param client 带超时的 HTTP 客户端。
 * @param url 官方模型地址。
 * @param expected 预期 SHA-256。
 * @param limit 最大归档字节数。
 * @returns 可信归档内容。
 * @throws 下载失败、体积超限或哈希不符。
 */
async fn download_archive(
    client: &reqwest::Client,
    url: &str,
    expected: &str,
    limit: usize,
) -> Result<Vec<u8>, String> {
    use futures_util::StreamExt;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| error.to_string())?;
        if bytes.len() + chunk.len() > limit {
            return Err("OCR 模型下载大小异常".to_owned());
        }
        bytes.extend_from_slice(&chunk);
    }
    if format!("{:x}", Sha256::digest(&bytes)) != expected {
        return Err("OCR 模型校验失败".to_owned());
    }
    Ok(bytes)
}

/**
 * 仅从官方归档中读取明确命名的模型文件，拒绝其他路径及超大展开内容。
 * @param archive 已校验的 TAR 归档。
 * @param prefix 模型目录名称。
 * @param filename 需要提取的文件名。
 * @returns 提取的模型或配置字节。
 * @throws 归档无效或目标文件缺失。
 */
fn extract_entry(archive: &[u8], prefix: &str, filename: &str) -> Result<Vec<u8>, String> {
    let target = format!("{prefix}/{filename}");
    for entry in tar::Archive::new(Cursor::new(archive))
        .entries()
        .map_err(|error| error.to_string())?
    {
        let mut entry = entry.map_err(|error| error.to_string())?;
        if entry
            .path()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            == target
        {
            if entry.size() > 8_000_000 {
                return Err("OCR 模型展开大小异常".to_owned());
            }
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            return Ok(bytes);
        }
    }
    Err(format!("OCR 模型归档缺少 {filename}"))
}

/**
 * 从识别模型配置提取与输出层一致的字符表。
 * @param yml 官方识别模型的 inference.yml 内容。
 * @returns 每行一个字符的字典。
 * @throws 配置缺少字符表。
 */
fn dictionary_from_yaml(yml: &[u8]) -> Result<String, String> {
    let value: Value = serde_yaml::from_slice(yml).map_err(|error| error.to_string())?;
    let chars = value["PostProcess"]["character_dict"]
        .as_sequence()
        .ok_or_else(|| "OCR 模型缺少字符字典".to_owned())?;
    if chars.len() < 1000 {
        return Err("OCR 字符字典长度异常".to_owned());
    }
    Ok(chars
        .iter()
        .map(|value| value.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n")
}

/**
 * 原子发布模型缓存中的单个文件。
 * @param path 最终路径。
 * @param content 已校验内容。
 * @returns 成功时无返回值。
 * @throws 写入或重命名失败。
 */
fn publish(path: &Path, content: &[u8]) -> Result<(), String> {
    let pending = path.with_extension("pending");
    fs::write(&pending, content).map_err(|error| error.to_string())?;
    if path.exists() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    fs::rename(pending, path).map_err(|error| error.to_string())
}

/**
 * 使用系统共享 ONNX Runtime 和本地 PP-OCRv6 tiny 模型识别截图。
 * @param image 已校验截图。
 * @param language 请求语言，tiny 模型以中英文识别为主。
 * @param paths 本地检测、识别和字典文件。
 * @returns 识别结果。
 * @throws 运行时或模型推理失败。
 */
pub(crate) fn recognize(
    image: DynamicImage,
    language: &str,
    paths: ModelPaths,
) -> Result<OcrResult, String> {
    let mut ocr = OcrLite::new();
    let det = paths.det.to_str().ok_or("OCR 检测模型路径无效")?;
    let rec = paths.rec.to_str().ok_or("OCR 识别模型路径无效")?;
    let dict = paths.dict.to_str().ok_or("OCR 字典路径无效")?;
    ocr.init_models_no_angle(det, rec, dict, 2)
        .map_err(|error| error.to_string())?;
    let image = image.thumbnail(4096, 4096).to_rgb8();
    let recognized = ocr
        .detect(&image, 10, 960, 0.6, 0.3, 1.6, false, false)
        .map_err(|error| error.to_string())?;
    Ok(OcrResult {
        text: recognized
            .text_blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        language: language.to_owned(),
        engine: "Windows ML · PP-OCRv6 tiny",
    })
}

#[cfg(test)]
mod tests {
    /**
     * 在真实 Windows App Runtime 上下载官方模型并识别固定截图。
     * @returns 无返回值。
     */
    #[test]
    #[ignore = "requires Windows App Runtime 1.8.1+ and first-use network download"]
    fn recognizes_with_windows_ml_fixture() {
        super::initialize();
        assert!(super::is_available(), "Windows ML runtime was not loaded");
        let root = std::env::temp_dir().join(format!("ztools-ocr-ml-test-{}", std::process::id()));
        let paths = tauri::async_runtime::block_on(super::ensure_models_in(&root)).unwrap();
        let image = image::load_from_memory(include_bytes!("../tests/fixtures/ocr.png")).unwrap();
        let result = super::recognize(image, "en-US", paths).unwrap();
        // tiny 模型对字体中的 O/0 会混淆；验证完整短语及数字，而不修改用户实际 OCR 文本。
        let normalized = result.text.to_uppercase().replace('0', "O");
        assert!(normalized.contains("ZTOOLS OCR"), "{}", result.text);
        assert!(result.text.contains("2026"), "{}", result.text);
        let cached = tauri::async_runtime::block_on(super::ensure_models_in(&root)).unwrap();
        assert!(cached.det.exists() && cached.rec.exists() && cached.dict.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
