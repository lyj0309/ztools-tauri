const invoke = window.__TAURI_INTERNALS__.invoke;
const grid = document.querySelector('#images');
const message = document.querySelector('#message');
const pinButton = document.querySelector('#pin');
const refreshButton = document.querySelector('#refresh');
let selectedId = null;
let busy = false;

/**
 * 更新图片选择状态和确认按钮。
 * @param id 历史图片标识。
 * @returns 无返回值。
 */
function selectImage(id) {
    selectedId = id;
    for (const card of grid.querySelectorAll('button')) {
        const selected = Number(card.dataset.id) === id;
        card.classList.toggle('selected', selected);
        card.setAttribute('aria-pressed', String(selected));
    }
    pinButton.disabled = selectedId === null || busy;
}

/**
 * 读取历史预览并保留仍然存在的选择。
 * @returns 列表刷新结束后的 Promise。
 */
async function loadHistory() {
    if (busy) return;
    busy = true;
    refreshButton.disabled = true;
    pinButton.disabled = true;
    try {
        const entries = await invoke('screenshot_history_list');
        // 仅插入宿主提供的缩略图，所有文字使用 textContent 防止被当成 HTML。
        grid.replaceChildren();
        for (const entry of entries) {
            const card = document.createElement('button');
            card.type = 'button';
            card.className = 'image-card';
            card.dataset.id = String(entry.id);
            const image = document.createElement('img');
            image.src = entry.thumbnail;
            image.alt = `${entry.width} × ${entry.height} 图片`;
            image.draggable = false;
            const caption = document.createElement('span');
            caption.textContent = `${entry.width} × ${entry.height} · ${new Date(entry.capturedAt).toLocaleString()}`;
            card.append(image, caption);
            grid.append(card);
        }
        message.classList.remove('error');
        message.textContent = '还没有历史图片。开启剪贴板监控后复制图片，再点击刷新。';
        message.hidden = entries.length > 0;
        selectedId = entries.some((entry) => entry.id === selectedId) ? selectedId : (entries[0]?.id ?? null);
    } catch (error) {
        message.hidden = false;
        message.classList.add('error');
        message.textContent = `读取失败：${String(error)}`;
    } finally {
        busy = false;
        refreshButton.disabled = false;
        selectImage(selectedId);
    }
}

/**
 * 将选中历史原图交给宿主生成悬浮贴图。
 * @returns 操作完成后的 Promise。
 */
async function pinSelected() {
    if (busy || selectedId === null) return;
    busy = true;
    pinButton.disabled = true;
    refreshButton.disabled = true;
    pinButton.textContent = '正在贴图…';
    try {
        await invoke('screenshot_history_pin', { id: selectedId });
    } catch (error) {
        message.hidden = false;
        message.classList.add('error');
        message.textContent = `贴图失败：${String(error)}`;
        busy = false;
        pinButton.disabled = false;
        refreshButton.disabled = false;
        pinButton.textContent = '贴图';
    }
}

/**
 * 点击选择图片，双击则立即贴出。
 * @param event 图片列表鼠标事件。
 * @returns 无返回值。
 */
function handleImageClick(event) {
    if (busy) return;
    const card = event.target.closest('[data-id]');
    if (!card) return;
    selectImage(Number(card.dataset.id));
    if (event.detail === 2) void pinSelected();
}

/**
 * 关闭选择页并恢复启动器。
 * @returns 操作结束后的 Promise。
 */
async function closeHistory() {
    if (!busy) await invoke('screenshot_history_close');
}

/**
 * 支持键盘确认与退出。
 * @param event 键盘事件。
 * @returns 无返回值。
 */
function handleKeyboard(event) {
    if (event.key === 'Escape') {
        event.preventDefault();
        void closeHistory();
    } else if (event.key === 'Enter' && event.target.id !== 'refresh' && event.target.id !== 'close') {
        event.preventDefault();
        const card = event.target.closest('[data-id]');
        if (card) selectImage(Number(card.dataset.id));
        void pinSelected();
    }
}
grid.addEventListener('click', handleImageClick);
pinButton.addEventListener('click', pinSelected);
refreshButton.addEventListener('click', loadHistory);
document.querySelector('#close').addEventListener('click', closeHistory);
window.addEventListener('keydown', handleKeyboard);
void loadHistory();
