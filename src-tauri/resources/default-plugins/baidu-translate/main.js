const invoke = window.__TAURI_INTERNALS__.invoke;
const api = window.ztools;
const ui = Object.fromEntries([...document.querySelectorAll('[id]')].map(element => [element.id, element]));
const languages = [['auto', '自动检测'], ['zh', '简体中文'], ['en', 'English'], ['cht', '繁体中文'], ['jp', '日本語'], ['kor', '한국어'], ['fra', 'Français'], ['de', 'Deutsch'], ['ru', 'Русский'], ['spa', 'Español'], ['pt', 'Português'], ['it', 'Italiano']];
let mode = 'text';
let busy = false;
let imageLoading = false;
let imageBytes = null;
let previewUrl = '';
let imageRevision = 0;
for (const [value, label] of languages) {
    ui.from.add(new Option(label, value));
    if (value !== 'auto') ui.to.add(new Option(label, value));
}
ui.to.value = 'zh';
const saved = api.dbStorage.getItem('baidu-api') || {};
ui.appid.value = saved.appid || '';
ui['api-key'].value = saved.key || '';
ui.settings.hidden = Boolean(saved.appid && saved.key);
/**
 * 在页面底部显示操作反馈，避免额外弹窗。
 * @param text 提示内容。
 * @param error 是否错误提示。
 * @returns 无返回值。
 */
function status(text, error = false) {
    ui.status.textContent = text;
    ui.status.classList.toggle('error', error);
}
/**
 * 切换文字或图片入口，保留尚未提交的图片供再次使用。
 * @param next 目标模式。
 * @returns 无返回值。
 */
function setMode(next) {
    if (busy) return;
    if (imageLoading && next === 'text') { imageRevision++; imageLoading = false; ui.translate.disabled = false; }
    mode = next;
    ui['image-input'].hidden = mode !== 'image';
    ui.source.readOnly = mode === 'image';
    ui.source.placeholder = mode === 'image' ? '翻译后显示图片中的原文' : '输入文字，Ctrl+Enter 翻译';
    ui['text-tab'].classList.toggle('active', mode === 'text');
    ui['image-tab'].classList.toggle('active', mode === 'image');
    ui.result.value = '';
    ui.copy.disabled = true;
}
/**
 * 生成可直接提交官方 API 的凭据及语种参数。
 * @returns 当前设置。
 * @throws 未填写 APPID 或密钥时抛出错误。
 */
function settings() {
    const appid = ui.appid.value.trim();
    const key = ui['api-key'].value.trim();
    if (!appid || !key) {
        ui.settings.hidden = false;
        throw new Error('请先填写百度翻译 APPID 和密钥，并开通对应服务');
    }
    return { appid, key, from: ui.from.value, to: ui.to.value };
}
/**
 * 调用官方文字或图片翻译，并防止重复提交和迟到结果覆盖。
 * @returns 翻译结束后的 Promise。
 */
async function translate() {
    if (busy || imageLoading) return;
    try {
        const input = settings();
        if (mode === 'text' && !ui.source.value.trim()) throw new Error('请先输入需要翻译的文字');
        if (mode === 'text' && new TextEncoder().encode(ui.source.value).length > 6000) throw new Error('单次原文最多 6000 字节，请分段翻译');
        if (mode === 'image' && !imageBytes) throw new Error('请先选择或粘贴图片');
        // 在发出请求前冻结界面，避免服务返回时写入另一张图片的结果。
        busy = true;
        ui.translate.disabled = true;
        ui.source.readOnly = true;
        ui.from.disabled = true;
        ui.to.disabled = true;
        ui.file.disabled = true;
        ui['clear-image'].disabled = true;
        ui.result.value = '';
        ui.copy.disabled = true;
        status('正在通过百度 API 翻译…');
        const result = mode === 'text'
            ? await invoke('baidu_translate_text', { input: { ...input, text: ui.source.value } })
            : await invoke('baidu_translate_image', imageBytes, { headers: { 'x-ztools-baidu-appid': input.appid, 'x-ztools-baidu-key': input.key, 'x-ztools-baidu-from': input.from, 'x-ztools-baidu-to': input.to } });
        ui.result.value = result.text;
        if (mode === 'image') ui.source.value = result.source;
        ui.copy.disabled = !result.text;
        status(`翻译完成 · ${result.from || input.from} → ${result.to || input.to}`);
    } catch (error) { status(String(error), true); }
    finally {
        busy = false;
        ui.translate.disabled = false;
        ui.source.readOnly = mode === 'image';
        ui.from.disabled = false;
        ui.to.disabled = false;
        ui.file.disabled = false;
        ui['clear-image'].disabled = false;
    }
}
/**
 * 解码图片并按官方尺寸、比例和体积要求补白及压缩为 PNG。
 * @param file 用户选择或粘贴的图片文件。
 * @returns 处理完成后的 Promise。
 */
async function loadImage(file) {
    if (busy || !file) return;
    const revision = ++imageRevision;
    imageLoading = true;
    imageBytes = null;
    ui.translate.disabled = true;
    setMode('image');
    status('正在读取图片…');
    let sourceUrl = '';
    try {
        if (!['image/png', 'image/jpeg', 'image/webp'].includes(file.type)) throw new Error('请选择 PNG、JPEG 或 WebP 图片');
        if (file.size > 16 * 1024 * 1024) throw new Error('原图不能超过 16 MB');
        sourceUrl = URL.createObjectURL(file);
        const image = new Image();
        image.src = sourceUrl;
        await image.decode();
        if (!image.naturalWidth || !image.naturalHeight || image.naturalWidth * image.naturalHeight > 32_000_000) throw new Error('图片像素过大，请先缩小图片');
        // 补白避免改变文字比例，最大边控制在 API 的 4096 像素以内。
        const paddedWidth = Math.max(30, image.naturalWidth, Math.ceil(image.naturalHeight / 3));
        const paddedHeight = Math.max(30, image.naturalHeight, Math.ceil(image.naturalWidth / 3));
        const canvas = document.createElement('canvas');
        let blob;
        for (const maximum of [4096, 2560, 1600, 1024]) {
            const scale = Math.min(1, maximum / Math.max(paddedWidth, paddedHeight));
            canvas.width = Math.max(30, Math.ceil(paddedWidth * scale));
            canvas.height = Math.max(30, Math.ceil(paddedHeight * scale));
            const context = canvas.getContext('2d');
            context.fillStyle = '#fff';
            context.fillRect(0, 0, canvas.width, canvas.height);
            context.drawImage(image, (canvas.width - image.naturalWidth * scale) / 2, (canvas.height - image.naturalHeight * scale) / 2, image.naturalWidth * scale, image.naturalHeight * scale);
            blob = await new Promise(resolve => canvas.toBlob(resolve, 'image/png'));
            if (blob && blob.size <= 4 * 1024 * 1024) break;
        }
        if (!blob || blob.size > 4 * 1024 * 1024) throw new Error('图片处理后仍超过 4 MB，请缩小图片');
        const bytes = new Uint8Array(await blob.arrayBuffer());
        if (revision !== imageRevision) return;
        imageBytes = bytes;
        if (previewUrl) URL.revokeObjectURL(previewUrl);
        previewUrl = URL.createObjectURL(blob);
        ui.preview.src = previewUrl;
        ui.preview.hidden = false;
        setMode('image');
        ui.source.value = '';
        status(`图片已就绪 · ${canvas.width} × ${canvas.height}，点击翻译上传到百度`);
    } catch (error) {
        if (revision === imageRevision) { clearImage(); status(String(error), true); }
    } finally {
        if (sourceUrl) URL.revokeObjectURL(sourceUrl);
        if (revision === imageRevision) { imageLoading = false; ui.translate.disabled = false; }
    }
}
/**
 * 清除图片、待处理任务及旧翻译，释放预览资源。
 * @returns 无返回值。
 */
function clearImage() {
    if (busy) return;
    imageRevision++;
    imageLoading = false;
    ui.translate.disabled = false;
    imageBytes = null;
    if (previewUrl) URL.revokeObjectURL(previewUrl);
    previewUrl = '';
    ui.preview.removeAttribute('src');
    ui.preview.hidden = true;
    ui.file.value = '';
    ui.source.value = '';
    ui.result.value = '';
    ui.copy.disabled = true;
    status('请选择或粘贴图片');
}
/**
 * 保存用户填写的 API 配置到当前插件自己的存储空间。
 * @returns 无返回值。
 */
function saveSettings() {
    try { const { appid, key } = settings(); api.dbStorage.setItem('baidu-api', { appid, key }); ui.settings.hidden = true; status('接口设置已保存'); }
    catch (error) { status(String(error), true); }
}
/**
 * 复制译文到剪贴板，写入失败时保留结果并反馈。
 * @returns 复制完成后的 Promise。
 */
async function copyTranslation() {
    try { await invoke('plugin_ocr_copy_text', { text: ui.result.value }); status('译文已复制'); }
    catch (error) { status(String(error), true); }
}
ui['text-tab'].addEventListener('click', () => setMode('text'));
ui['image-tab'].addEventListener('click', () => setMode('image'));
ui.translate.addEventListener('click', translate);
ui.copy.addEventListener('click', copyTranslation);
ui['settings-toggle'].addEventListener('click', () => { ui.settings.hidden = !ui.settings.hidden; });
ui['save-settings'].addEventListener('click', saveSettings);
ui['clear-settings'].addEventListener('click', () => { api.dbStorage.removeItem('baidu-api'); ui.appid.value = ''; ui['api-key'].value = ''; status('接口设置已清除'); });
ui['api-docs'].addEventListener('click', () => { void invoke('plugin_shell_open', { target: 'https://api.fanyi.baidu.com/manage/developer' }).catch(error => status(String(error), true)); });
ui.file.addEventListener('change', () => { void loadImage(ui.file.files[0]); });
ui['clear-image'].addEventListener('click', clearImage);
ui['paste-image'].addEventListener('click', pasteImage);
document.addEventListener('paste', event => {
    const file = [...(event.clipboardData?.files || [])].find(item => item.type.startsWith('image/'));
    if (file) { event.preventDefault(); void loadImage(file); }
});
document.addEventListener('keydown', event => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'v' && mode === 'image' && event.target !== ui.appid && event.target !== ui['api-key']) {
        event.preventDefault(); void pasteImage(); return;
    }
    if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') { event.preventDefault(); void translate(); }
});
api.onPluginEnter(action => { setMode(action.code === 'image' ? 'image' : 'text'); });
window.addEventListener('beforeunload', () => { if (previewUrl) URL.revokeObjectURL(previewUrl); });

/**
 * 从原生剪贴板读取图片，兼容不能向网页派发图片粘贴事件的 Webview。
 * @returns 图片读取与预览完成后的 Promise。
 */
async function pasteImage() {
    if (busy || imageLoading) return;
    try {
        const response = await invoke('plugin_read_clipboard_image');
        const bytes = response instanceof ArrayBuffer ? new Uint8Array(response) : Uint8Array.from(response);
        await loadImage(new File([bytes], 'clipboard.png', { type: 'image/png' }));
    } catch (error) { status(String(error), true); }
}
