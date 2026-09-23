use std::{io::Cursor, sync::Mutex};

use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::{commands::launcher::current_timestamp, desktop, state::AppState};

static LAST_CAPTURE: Mutex<(Option<u32>, Option<String>)> = Mutex::new((None, None));

pub(crate) struct CapturedImage {
    pub(crate) hash: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) png: Vec<u8>,
    pub(crate) thumbnail: String,
}

/**
 * 将剪贴板像素编码为原图及限尺寸预览。
 * @param rgba 从系统剪贴板读取的 RGBA 像素。
 * @param hash 像素与尺寸的 SHA256，用于去重。
 * @returns 编码后的图片或 PNG 编码错误。
 */
fn encode_image(rgba: image::RgbaImage, hash: String) -> Result<CapturedImage, String> {
    let image = image::DynamicImage::ImageRgba8(rgba);
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    if png.get_ref().len() > 64 * 1024 * 1024 {
        return Err("剪贴板图片超过 64 MB".to_owned());
    }
    // 列表仅返回缩略图，选中后再读取完整 PNG，避免一次加载所有历史原图。
    let mut preview = Cursor::new(Vec::new());
    image
        .thumbnail(240, 160)
        .write_to(&mut preview, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(CapturedImage {
        hash,
        width: image.width(),
        height: image.height(),
        png: png.into_inner(),
        thumbnail: format!(
            "data:image/png;base64,{}",
            STANDARD.encode(preview.into_inner())
        ),
    })
}

/**
 * 捕获当前剪贴板图片；重复像素不重复编码，Windows 无变化时不读取像素。
 * @param app 用于访问隔离或正式数据库的宿主句柄。
 * @returns 新图片已保存返回 true，无变化或监控关闭返回 false，失败返回错误。
 */
pub(crate) fn capture_current(app: &AppHandle) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let settings = state.store.lock().map_err(|e| e.to_string())?.settings()?;
    let mut last = LAST_CAPTURE.lock().map_err(|e| e.to_string())?;
    if !settings.clipboard_monitoring {
        *last = (None, None);
        return Ok(false);
    }
    let sequence = desktop::clipboard_sequence();
    if sequence.is_some() && sequence == last.0 {
        return Ok(false);
    }
    let Some(rgba) = desktop::read_clipboard_image()? else {
        *last = (sequence, None);
        return Ok(false);
    };
    let mut digest = Sha256::new();
    digest.update(rgba.width().to_le_bytes());
    digest.update(rgba.height().to_le_bytes());
    digest.update(rgba.as_raw());
    let hash = format!("{:x}", digest.finalize());
    if last.1.as_ref() == Some(&hash) {
        last.0 = sequence;
        return Ok(false);
    }
    let encoded = encode_image(rgba, hash.clone())?;
    let now = current_timestamp()?;
    // 编码不占数据库锁；成功提交后才记录序号，让临时剪贴板冲突可以重试。
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.capture_clipboard_image(&encoded, now)?;
    store.prune_clipboard(settings.clipboard_retention_days, now)?;
    *last = (sequence, Some(hash));
    Ok(true)
}

#[cfg(test)]
mod tests {
    /**
     * 确保历史使用原图尺寸并生成小尺寸缩略图。
     * @returns 无返回值。
     */
    #[test]
    fn preserves_original_and_limits_preview() {
        use base64::Engine;
        let pixels = image::RgbaImage::from_pixel(640, 320, image::Rgba([20, 80, 220, 255]));
        let encoded = super::encode_image(pixels, "hash".into()).unwrap();
        let original = image::load_from_memory(&encoded.png).unwrap();
        assert_eq!((original.width(), original.height()), (640, 320));
        let preview = super::STANDARD
            .decode(encoded.thumbnail.split_once(',').unwrap().1)
            .unwrap();
        let preview = image::load_from_memory(&preview).unwrap();
        assert_eq!((preview.width(), preview.height()), (240, 120));
    }
}
