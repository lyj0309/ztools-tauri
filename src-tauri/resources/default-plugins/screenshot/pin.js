const invoke = window.__TAURI_INTERNALS__.invoke;
const root = document.querySelector('#screenshot-pin');
if (!root)
    throw new Error('贴图窗口根节点不存在');
root.innerHTML = `
  <img class="pin-image" alt="悬浮截图" draggable="false" />
  <div class="pin-controls">
    <button type="button" data-action="copy" title="复制 Ctrl+C">复制</button>
    <button type="button" data-action="save" title="保存 Ctrl+S">保存</button>
    <button type="button" data-action="close" title="关闭 Esc">×</button>
  </div>
  <div class="pin-message" hidden></div>
`;
const image = root.querySelector('.pin-image');
const controls = root.querySelector('.pin-controls');
const message = root.querySelector('.pin-message');
let sourceUrl = '';
let busy = false;
/**
 * 把 Tauri 原始响应统一转换为图片字节。
 * @param value IPC 返回的 ArrayBuffer 或数字数组。
 * @returns 可供 Blob 使用的字节数组。
 */
function responseBytes(value) {
    return value instanceof ArrayBuffer ? new Uint8Array(value) : Uint8Array.from(value);
}
/**
 * 在贴图窗口中央短暂显示操作结果。
 * @param text 要展示的状态文字。
 * @param error 是否显示错误样式。
 * @returns 无返回值。
 */
function showMessage(text, error = false) {
    message.textContent = text;
    message.classList.toggle('error', error);
    message.hidden = false;
    window.setTimeout(() => {
        message.hidden = true;
    }, 1800);
}
/**
 * 执行贴图复制、保存或关闭命令，并避免按钮重复提交。
 * @param action 要执行的贴图操作。
 * @returns 操作完成后的 Promise。
 */
async function runAction(action) {
    if (busy)
        return;
    busy = true;
    controls.classList.add('busy');
    try {
        if (action === 'copy') {
            await invoke('screenshot_pin_copy');
            showMessage('已复制到剪贴板');
        }
        else if (action === 'save') {
            const path = await invoke('screenshot_pin_save');
            showMessage(`已保存：${path}`);
        }
        else {
            await invoke('screenshot_pin_close');
        }
    }
    catch (error) {
        showMessage(String(error), true);
    }
    finally {
        busy = false;
        controls.classList.remove('busy');
    }
}
/**
 * 处理贴图工具栏按钮。
 * @param event 工具栏点击事件。
 * @returns 无返回值。
 */
function handleControlsClick(event) {
    const button = event.target.closest('button');
    const action = button?.dataset.action;
    if (action === 'copy' || action === 'save' || action === 'close')
        void runAction(action);
}
/**
 * 在图片区域按下主键时交给系统拖动无边框窗口。
 * @param event 指针按下事件。
 * @returns 无返回值。
 */
function handlePointerDown(event) {
    if (event.button !== 0 || event.target.closest('.pin-controls'))
        return;
    event.preventDefault();
    void invoke('screenshot_pin_start_dragging').catch((error) => showMessage(String(error), true));
}
/**
 * 提供贴图复制、保存和关闭快捷键。
 * @param event 全局键盘事件。
 * @returns 无返回值。
 */
function handleKeyboard(event) {
    if (event.key === 'Escape') {
        event.preventDefault();
        void runAction('close');
    }
    else if ((event.ctrlKey || event.metaKey) && event.key.toLocaleLowerCase() === 'c') {
        event.preventDefault();
        void runAction('copy');
    }
    else if ((event.ctrlKey || event.metaKey) && event.key.toLocaleLowerCase() === 's') {
        event.preventDefault();
        void runAction('save');
    }
}
/**
 * 读取宿主持有的临时 PNG 并显示为贴图内容。
 * @returns 初始化完成后的 Promise。
 */
async function initialize() {
    try {
        const raw = await invoke('screenshot_pin_source');
        const bytes = responseBytes(raw);
        sourceUrl = URL.createObjectURL(new Blob([new Uint8Array(bytes).buffer], { type: 'image/png' }));
        image.src = sourceUrl;
        await image.decode();
        // 图片首帧就绪后再显示原生贴图窗口，避免空白窗口抢焦点。
        await invoke('screenshot_pin_ready');
    }
    catch (error) {
        showMessage(`贴图加载失败：${String(error)}`, true);
    }
}
root.addEventListener('pointerdown', handlePointerDown);
controls.addEventListener('click', handleControlsClick);
window.addEventListener('keydown', handleKeyboard);
window.addEventListener('contextmenu', (event) => event.preventDefault());
window.addEventListener('beforeunload', () => {
    if (sourceUrl)
        URL.revokeObjectURL(sourceUrl);
});
void initialize();
