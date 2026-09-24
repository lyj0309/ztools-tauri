const invoke = window.__TAURI_INTERNALS__.invoke;
const selection = JSON.parse(sessionStorage.getItem('ztools-original-selection') || 'null');
let completed = false;
let editorDataUrl = '';

/**
 * 将 Tauri 返回的原始响应规范化为可传给 Blob 的字节数组。
 * @param value IPC 返回的 ArrayBuffer 或数字数组。
 * @returns PNG 字节数组。
 */
function responseBytes(value) {
  return value instanceof ArrayBuffer ? new Uint8Array(value) : Uint8Array.from(value);
}

/**
 * 把 PNG 字节转换成原版编辑器读取的 data URL。
 * @param bytes 截图 PNG 字节。
 * @returns 图片加载完成后的 data URL。
 */
function pngDataUrl(bytes) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result || ''));
    reader.onerror = () => reject(reader.error || new Error('无法读取截图'));
    reader.readAsDataURL(new Blob([bytes], { type: 'image/png' }));
  });
}

/**
 * 把原版编辑器输出的 data URL 解码为 Rust 截图命令的二进制请求体。
 * @param dataUrl 原版编辑器导出的 PNG data URL。
 * @returns PNG 字节。
 * @throws 当画布输出不是 PNG 时抛出错误。
 */
async function pngFromDataUrl(dataUrl) {
  if (!String(dataUrl).startsWith('data:image/png;base64,')) throw new Error('截图输出不是 PNG');
  // WebKitGTK 不保证允许 fetch(data:)；直接解码画布产出的 base64，避免所有导出操作一起失败。
  const binary = atob(dataUrl.slice('data:image/png;base64,'.length));
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  return bytes;
}

/**
 * 读取原版画布上的最新标注，保留用户刚完成的编辑。
 * @returns 当前画布的 PNG 字节。
 * @throws 当画布尚未准备好时抛出错误。
 */
async function currentCanvasPng() {
  const canvas = document.querySelector('.editor-shell canvas.capture-canvas');
  if (!(canvas instanceof HTMLCanvasElement)) throw new Error('截图画布尚未准备好');
  return pngFromDataUrl(canvas.toDataURL('image/png'));
}

/**
 * 复制原版编辑器画布并结束截图会话。
 * @param dataUrl 原版编辑器生成的 PNG。
 * @returns Rust 写入图片剪贴板后的 Promise。
 */
async function copyImage(dataUrl) {
  const bytes = await pngFromDataUrl(dataUrl);
  completed = true;
  try { await invoke('screenshot_copy', bytes); }
  catch (error) { completed = false; throw error; }
}

/**
 * 保存原版编辑器画布，由 Rust 选择最终文件位置并结束截图会话。
 * @param dataUrl 原版编辑器生成的 PNG。
 * @param _path 原版编辑器预期的文件名占位符。
 * @returns 保存完成后的文件路径。
 */
async function saveDataUrl(dataUrl, _path) {
  const bytes = await pngFromDataUrl(dataUrl);
  const path = await invoke('screenshot_save', bytes);
  if (path) completed = true;
  return path;
}

/**
 * 关闭原版编辑器；输出成功时 Rust 已负责销毁窗口。
 * @param _restore 原版插件的恢复窗口参数。
 * @returns 无返回值。
 */
function closeWindow(_restore) {
  if (!completed) void invoke('screenshot_cancel');
}

/**
 * 从原版编辑器的画布生成原生贴图，并保留选区的物理屏幕位置。
 * @returns 创建贴图并关闭编辑器后的 Promise。
 * @throws 当选区已失效或贴图创建失败时抛出错误。
 */
async function pinCanvas() {
  if (!selection) throw new Error('截图选区已失效');
  const bytes = await currentCanvasPng();
  completed = true;
  try {
    await invoke('screenshot_pin', bytes, { headers: {
      'x-ztools-selection-x': String(selection.x),
      'x-ztools-selection-y': String(selection.y),
      'x-ztools-selection-width': String(selection.width),
      'x-ztools-selection-height': String(selection.height)
    } });
  } catch (error) { completed = false; throw error; }
}

/**
 * 在原版工具栏旁展示本地 OCR 结果，保持识别与编辑在同一窗口。
 * @returns OCR 结果面板填充完成后的 Promise。
 */
async function recognizeCanvas() {
  const panel = document.querySelector('.ztools-extra-panel');
  const status = panel.querySelector('.ztools-extra-status');
  panel.hidden = false;
  status.textContent = '正在本地识别…';
  try {
    const bytes = await currentCanvasPng();
    const result = await invoke('plugin_ocr', bytes, { headers: { 'x-ztools-ocr-language': 'auto' } });
    panel.querySelector('textarea').value = result.text || '';
    status.textContent = result.text ? `本地识别 · ${result.language}` : '没有识别到文字';
  } catch (error) { status.textContent = String(error); }
}

/**
 * 在原版标注工具栏追加此前内置截图已有的 OCR 和贴图入口。
 * @returns 工具栏与结果面板挂载完成后的 Promise。
 */
async function mountExtraActions() {
  // 原版 Vue 工具栏可能在图片解码后才出现，观察到首个 footer 再追加按钮。
  const footer = await new Promise((resolve) => {
    const existing = document.querySelector('.editor-shell footer');
    if (existing) { resolve(existing); return; }
    const observer = new MutationObserver(() => {
      const target = document.querySelector('.editor-shell footer');
      if (target) { observer.disconnect(); resolve(target); }
    });
    observer.observe(document.querySelector('#app'), { childList: true, subtree: true });
  });
  for (const [label, action] of [['OCR', recognizeCanvas], ['贴图', pinCanvas]]) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'icon-button';
    button.textContent = label;
    button.title = label;
    button.addEventListener('click', () => void action().catch((error) => {
      const status = document.querySelector('.ztools-extra-status');
      status.textContent = String(error);
      status.closest('.ztools-extra-panel').hidden = false;
    }));
    footer.insertBefore(button, footer.lastElementChild);
  }
  const panel = document.createElement('section');
  panel.className = 'ztools-extra-panel';
  panel.hidden = true;
  panel.innerHTML = '<strong>文字识别</strong><textarea aria-label="识别文字"></textarea><button type="button" data-action="copy">复制文字</button><button type="button" data-action="close">关闭</button><span class="ztools-extra-status"></span>';
  panel.querySelector('[data-action="copy"]').addEventListener('click', () => {
    void invoke('plugin_ocr_copy_text', { text: panel.querySelector('textarea').value })
      .then(() => { panel.querySelector('.ztools-extra-status').textContent = '文字已复制'; });
  });
  panel.querySelector('[data-action="close"]').addEventListener('click', () => { panel.hidden = true; });
  document.body.append(panel);
}

/**
 * 建立原版截图编辑器需要的浏览器适配层，然后加载其原始 Vue 资源。
 * @returns 原版编辑器启动完成后的 Promise。
 */
async function initializeOriginalEditor() {
  const bytes = responseBytes(await invoke('screenshot_editor_source'));
  editorDataUrl = await pngDataUrl(bytes);
  sessionStorage.removeItem('ztools-original-selection');
  // 原版只需要同步读取一次截图数据，数据从 Rust 临时文件取得且不落入 Web 存储。
  window.ztools = Object.freeze({ dbStorage: {
    getItem: () => ({ dataUrl: editorDataUrl }),
    removeItem: () => undefined
  } });
  window.shortcutCapture = Object.freeze({
    editorKeyPrefix: 'ztools-capture:',
    copyImage,
    showSaveDialog: async () => 'rust-save-dialog',
    getDefaultSavePath: () => '',
    saveDataUrl,
    shellShowItemInFolder: () => undefined,
    closeWindow
  });
  document.querySelector('#app').innerHTML = '';
  await import('./assets/index-CRZJr_0k.js');
  await invoke('screenshot_editor_ready');
  void mountExtraActions();
}

void initializeOriginalEditor().catch((error) => {
  document.querySelector('#app').textContent = `截图标注加载失败：${String(error)}`;
  void invoke('screenshot_editor_ready');
});
