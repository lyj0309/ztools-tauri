use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{
    ipc::{InvokeBody, Request},
    Manager, WebviewWindow,
};

const TEXT_URL: &str = "https://fanyi-api.baidu.com/api/trans/vip/translate";
const IMAGE_URL: &str = "https://fanyi-api.baidu.com/api/trans/sdk/picture";

#[derive(Deserialize)]
pub(crate) struct TranslationInput {
    appid: String,
    key: String,
    from: String,
    to: String,
    text: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct TranslationResult {
    source: String,
    text: String,
    from: String,
    to: String,
}

/**
 * 校验百度插件身份及官方接口需要的凭据和语种。
 * @param window 发起请求的插件窗口。
 * @param input 官方接口参数。
 * @returns 校验结果。
 */
fn validate(window: &WebviewWindow, input: &TranslationInput) -> Result<(), String> {
    let name = window
        .app_handle()
        .state::<crate::plugin::PluginRuntime>()
        .plugin_for_window(window.label())?;
    if name != "baidu-translate" {
        return Err("仅百度翻译插件可调用此接口".to_owned());
    }
    if input.appid.is_empty()
        || input.appid.len() > 64
        || !input.appid.bytes().all(|c| c.is_ascii_digit())
        || input.key.is_empty()
        || input.key.len() > 256
    {
        return Err("请先在接口设置中填写正确的百度翻译 APPID 和密钥".to_owned());
    }
    let supported = [
        "auto", "zh", "en", "cht", "jp", "kor", "fra", "de", "ru", "spa", "pt", "it",
    ];
    if !supported.contains(&input.from.as_str())
        || !supported.contains(&input.to.as_str())
        || input.to == "auto"
    {
        return Err("翻译语种无效".to_owned());
    }
    Ok(())
}

/**
 * 生成百度规定的 MD5 签名，文本必须保持原始 UTF-8 而非 URL 编码。
 * @param parts 按官方顺序排列的原始字段。
 * @returns 32 位小写 MD5。
 */
fn sign(parts: &[&str]) -> String {
    format!("{:x}", Md5::digest(parts.concat()))
}

/**
 * 创建有超时且不跟随重定向的官方接口客户端和请求随机串。
 * @returns HTTPS 客户端及每次请求的盐值。
 */
fn client() -> Result<(reqwest::Client, String), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(40))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?;
    let salt = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos()
        .to_string();
    Ok((client, salt))
}

/**
 * 调用百度通用文本翻译，密钥仅用于本地签名，不发送给服务端。
 * @param input 凭据、原文及语种。
 * @param window 发起调用的百度翻译插件。
 * @returns 规范化原文与译文。
 */
#[tauri::command]
pub(crate) async fn baidu_translate_text(
    input: TranslationInput,
    window: WebviewWindow,
) -> Result<TranslationResult, String> {
    validate(&window, &input)?;
    if input.text.trim().is_empty() || input.text.len() > 6000 {
        return Err("请输入文字，单次原文最多 6000 字节".to_owned());
    }
    let (client, salt) = client()?;
    let signature = sign(&[&input.appid, &input.text, &salt, &input.key]);
    let response = client
        .post(TEXT_URL)
        .form(&[
            ("q", input.text.as_str()),
            ("from", &input.from),
            ("to", &input.to),
            ("appid", &input.appid),
            ("salt", &salt),
            ("sign", &signature),
        ])
        .send()
        .await
        .map_err(|error| format!("百度接口连接失败：{error}"))?;
    read_response(response, false).await
}

/**
 * 读取二进制图片并调用百度图片翻译接口。
 * @param request PNG 请求体，凭据及语种由 x-ztools-baidu-* 请求头携带。
 * @param window 发起调用的百度翻译插件。
 * @returns 图片中的原文与译文。
 */
#[tauri::command]
pub(crate) async fn baidu_translate_image(
    request: Request<'_>,
    window: WebviewWindow,
) -> Result<TranslationResult, String> {
    let header = |name| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned()
    };
    let input = TranslationInput {
        appid: header("x-ztools-baidu-appid"),
        key: header("x-ztools-baidu-key"),
        from: header("x-ztools-baidu-from"),
        to: header("x-ztools-baidu-to"),
        text: String::new(),
    };
    validate(&window, &input)?;
    let image = match request.body() {
        InvokeBody::Raw(data) if !data.is_empty() && data.len() <= 4 * 1024 * 1024 => data.clone(),
        _ => return Err("百度图片翻译需要不超过 4 MB 的 PNG 图片".to_owned()),
    };
    // 上传前核实真实图片头尺寸，防止无效图片占用付费接口次数。
    validate_image(&image)?;
    let (client, salt) = client()?;
    let image_hash = format!("{:x}", Md5::digest(&image));
    let signature = sign(&[
        &input.appid,
        &image_hash,
        &salt,
        "APICUID",
        "mac",
        &input.key,
    ]);
    let form = reqwest::multipart::Form::new()
        .text("appid", input.appid)
        .text("from", input.from)
        .text("to", input.to)
        .text("salt", salt)
        .text("cuid", "APICUID")
        .text("mac", "mac")
        .text("version", "3")
        .text("paste", "0")
        .text("sign", signature)
        .part(
            "image",
            reqwest::multipart::Part::bytes(image)
                .file_name("image.png")
                .mime_str("image/png")
                .map_err(|error| error.to_string())?,
        );
    let response = client
        .post(IMAGE_URL)
        .multipart(form)
        .send()
        .await
        .map_err(|error| format!("百度接口连接失败：{error}"))?;
    read_response(response, true).await
}

/**
 * 检查官方图片 API 对边长和比例的限制。
 * @param data PNG 字节。
 * @returns 可上传结果。
 */
fn validate_image(data: &[u8]) -> Result<(), String> {
    let reader =
        image::ImageReader::with_format(std::io::Cursor::new(data), image::ImageFormat::Png);
    let (w, h) = reader
        .into_dimensions()
        .map_err(|_| "图片格式无效".to_owned())?;
    if w.min(h) < 30 || w.max(h) > 4096 || w.max(h) > w.min(h) * 3 {
        return Err("图片边长需要 30–4096 像素，长宽比不超过 3:1".to_owned());
    }
    Ok(())
}

/**
 * 限制服务响应体并解析成功或错误数据。
 * @param response 百度官方 HTTPS 响应。
 * @param image 是否按图片接口读取结果。
 * @returns 原文译文或可操作错误。
 */
async fn read_response(
    mut response: reqwest::Response,
    image: bool,
) -> Result<TranslationResult, String> {
    if !response.status().is_success() {
        return Err(format!("百度接口返回 HTTP {}", response.status().as_u16()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if body.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("百度返回内容过大".to_owned());
        }
        body.extend_from_slice(&chunk);
    }
    let value = serde_json::from_slice(&body).map_err(|_| "百度返回了无法解析的内容".to_owned())?;
    parse_response(&value, image)
}

/**
 * 合并文字段落或图片原译文，并将常见服务错误转为中文提示。
 * @param value 接口 JSON。
 * @param image 是否图片接口。
 * @returns 统一翻译结果。
 */
fn parse_response(value: &Value, image: bool) -> Result<TranslationResult, String> {
    if let Some(code) = value.get("error_code") {
        let code = code
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| code.to_string());
        if code != "0" && code != "52000" {
            let hint = match code.as_str() {
                "52003" | "58002" => "请检查 APPID 并在百度控制台开通对应翻译服务",
                "54001" => "签名校验失败，请检查 APPID 和密钥是否配对",
                "54003" | "54005" => "请求过于频繁，请稍后再试",
                "54004" => "百度账户额度或余额不足",
                "58001" => "该接口不支持所选翻译方向，请切换语言",
                "90107" => "请先完成百度开发者认证",
                _ => value
                    .get("error_msg")
                    .and_then(Value::as_str)
                    .unwrap_or("服务暂不可用，请稍后重试"),
            };
            return Err(format!("百度翻译 {code}：{hint}"));
        }
    }
    let data = if image {
        value.get("data").ok_or("图片翻译响应缺少 data")?
    } else {
        value
    };
    let (source, text) = if image {
        (
            data.get("sumSrc")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            data.get("sumDst")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        )
    } else {
        let rows = data
            .get("trans_result")
            .and_then(Value::as_array)
            .ok_or("翻译响应缺少结果")?;
        let column = |key| {
            rows.iter()
                .filter_map(|row| row.get(key).and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        };
        (column("src"), column("dst"))
    };
    if text.trim().is_empty() {
        return Err("没有返回译文，请检查文字或图片是否清晰".to_owned());
    }
    Ok(TranslationResult {
        source,
        text,
        from: data
            .get("from")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        to: data
            .get("to")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
    })
}

#[cfg(test)]
mod tests {
    /**
     * 使用官方样例验证签名，并覆盖两种响应和服务拒绝。
     * @returns 无返回值。
     */
    #[test]
    fn signs_and_parses_official_contracts() {
        assert_eq!(
            super::sign(&["2015063000000001", "apple", "65478", "1234567890"]),
            "a1a7461d92e5194c5cae3182b5b24de1"
        );
        let text = serde_json::json!({"from":"en", "to":"zh", "trans_result":[{"src":"apple", "dst":"苹果"}]});
        assert_eq!(super::parse_response(&text, false).unwrap().text, "苹果");
        let image = serde_json::json!({"error_code":"0", "data":{"sumSrc":"hello", "sumDst":"你好", "from":"en", "to":"zh"}});
        assert_eq!(super::parse_response(&image, true).unwrap().source, "hello");
        assert!(
            super::parse_response(&serde_json::json!({"error_code":54001}), false)
                .unwrap_err()
                .contains("密钥")
        );
        assert!(super::parse_response(&serde_json::json!({}), true).is_err());
    }
}
