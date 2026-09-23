const invoke = window.__TAURI_INTERNALS__.invoke;
const root = document.querySelector('#screenshot-editor');
if (!root)
    throw new Error('截图编辑器根节点不存在');
root.innerHTML = `
  <canvas class="capture-canvas" aria-label="截图选区画布"></canvas>
  <div class="capture-guide">拖动鼠标选择截图区域 · Esc 取消</div>
  <div class="capture-size" hidden></div>
  <input class="capture-text-input" type="text" maxlength="200" aria-label="标注文字" placeholder="输入文字，Enter 确认" hidden />
  <div class="capture-toolbar" hidden>
    <button type="button" data-tool="select" title="重新选择区域">选区</button>
    <button type="button" data-tool="rect" title="矩形标注">矩形</button>
    <button type="button" data-tool="arrow" title="箭头标注">箭头</button>
    <button type="button" data-tool="pen" title="自由画笔">画笔</button>
    <button type="button" data-tool="text" title="文字标注">文字</button>
    <span class="capture-divider"></span>
    <button type="button" data-color="#ff3b30" class="color active" aria-label="红色"></button>
    <button type="button" data-color="#ffd60a" class="color yellow" aria-label="黄色"></button>
    <button type="button" data-color="#20c997" class="color green" aria-label="绿色"></button>
    <span class="capture-divider"></span>
    <button type="button" data-action="undo" title="撤销 Ctrl+Z">撤销</button>
    <button type="button" data-action="copy" class="primary" title="复制 Enter">复制</button>
    <button type="button" data-action="save" title="保存 S">保存</button>
    <button type="button" data-action="pin" title="贴图 P">贴图</button>
    <button type="button" data-action="cancel" title="取消 Esc">×</button>
  </div>
  <div class="capture-message" hidden></div>
`;
const canvas = root.querySelector('.capture-canvas');
const context = canvas.getContext('2d');
const toolbar = root.querySelector('.capture-toolbar');
const guide = root.querySelector('.capture-guide');
const sizeLabel = root.querySelector('.capture-size');
const textInput = root.querySelector('.capture-text-input');
const message = root.querySelector('.capture-message');
let sourceImage = null;
let sourceUrl = '';
let selection = null;
let annotations = [];
let draft = null;
let dragStart = null;
let activeTool = 'select';
let activeColor = '#ff3b30';
let busy = false;
let pendingTextPoint = null;
/**
 * 把 Tauri 原始响应统一转换为字节数组。
 * @param value IPC 返回的 ArrayBuffer 或数字数组。
 * @returns 可构造 PNG Blob 的字节数组。
 */
function responseBytes(value) {
    return value instanceof ArrayBuffer ? new Uint8Array(value) : Uint8Array.from(value);
}
/**
 * 把指针的 CSS 坐标映射为源截图像素并限制在画布内。
 * @param event 当前指针事件。
 * @returns 源图像素坐标。
 */
function eventPoint(event) {
    const bounds = canvas.getBoundingClientRect();
    return {
        x: Math.min(canvas.width, Math.max(0, ((event.clientX - bounds.left) / bounds.width) * canvas.width)),
        y: Math.min(canvas.height, Math.max(0, ((event.clientY - bounds.top) / bounds.height) * canvas.height))
    };
}
/**
 * 用两个点生成方向无关的标准矩形。
 * @param start 起点。
 * @param end 终点。
 * @returns 左上角和正宽高表示的矩形。
 */
function normalizedRect(start, end) {
    return {
        x: Math.min(start.x, end.x),
        y: Math.min(start.y, end.y),
        width: Math.abs(end.x - start.x),
        height: Math.abs(end.y - start.y)
    };
}
/**
 * 判断点是否位于当前选区内。
 * @param point 要检查的源图坐标。
 * @returns 点是否在选区边界内。
 */
function pointInsideSelection(point) {
    return Boolean(selection &&
        point.x >= selection.x &&
        point.y >= selection.y &&
        point.x <= selection.x + selection.width &&
        point.y <= selection.y + selection.height);
}
/**
 * 把标注点限制在当前选区，防止输出画布外内容影响裁剪。
 * @param point 原始源图坐标。
 * @returns 限制后的坐标。
 */
function clampToSelection(point) {
    if (!selection)
        return point;
    return {
        x: Math.min(selection.x + selection.width, Math.max(selection.x, point.x)),
        y: Math.min(selection.y + selection.height, Math.max(selection.y, point.y))
    };
}
/**
 * 返回当前显示比例下对应三个 CSS 像素的源图线宽。
 * @returns 绘制到源像素画布的线宽。
 */
function annotationLineWidth() {
    return Math.max(2, (canvas.width / Math.max(window.innerWidth, 1)) * 3);
}
/**
 * 绘制单条矩形、箭头、画笔或文字标注。
 * @param target 绘制目标上下文。
 * @param annotation 标注数据。
 * @returns 无返回值。
 */
function drawAnnotation(target, annotation) {
    const lineWidth = annotationLineWidth();
    target.save();
    target.strokeStyle = annotation.color;
    target.fillStyle = annotation.color;
    target.lineWidth = lineWidth;
    target.lineCap = 'round';
    target.lineJoin = 'round';
    if (annotation.kind === 'rect') {
        const rect = normalizedRect(annotation.start, annotation.end);
        target.strokeRect(rect.x, rect.y, rect.width, rect.height);
    }
    else if (annotation.kind === 'arrow') {
        const dx = annotation.end.x - annotation.start.x;
        const dy = annotation.end.y - annotation.start.y;
        const angle = Math.atan2(dy, dx);
        const head = lineWidth * 5;
        target.beginPath();
        target.moveTo(annotation.start.x, annotation.start.y);
        target.lineTo(annotation.end.x, annotation.end.y);
        target.moveTo(annotation.end.x, annotation.end.y);
        target.lineTo(annotation.end.x - head * Math.cos(angle - Math.PI / 6), annotation.end.y - head * Math.sin(angle - Math.PI / 6));
        target.moveTo(annotation.end.x, annotation.end.y);
        target.lineTo(annotation.end.x - head * Math.cos(angle + Math.PI / 6), annotation.end.y - head * Math.sin(angle + Math.PI / 6));
        target.stroke();
    }
    else if (annotation.kind === 'pen') {
        if (annotation.points.length >= 2) {
            target.beginPath();
            target.moveTo(annotation.points[0].x, annotation.points[0].y);
            for (const point of annotation.points.slice(1))
                target.lineTo(point.x, point.y);
            target.stroke();
        }
    }
    else {
        target.font = `600 ${Math.max(18, lineWidth * 6)}px -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`;
        target.textBaseline = 'top';
        target.fillText(annotation.text, annotation.point.x, annotation.point.y);
    }
    target.restore();
}
/**
 * 重绘截图背景、遮罩、选区和全部标注。
 * @returns 无返回值。
 */
function redraw() {
    if (!sourceImage)
        return;
    context.clearRect(0, 0, canvas.width, canvas.height);
    context.drawImage(sourceImage, 0, 0, canvas.width, canvas.height);
    if (!selection) {
        context.fillStyle = 'rgba(0, 0, 0, 0.42)';
        context.fillRect(0, 0, canvas.width, canvas.height);
        return;
    }
    // 只压暗选区外区域，保证选区内保持截图原始亮度。
    context.fillStyle = 'rgba(0, 0, 0, 0.48)';
    context.fillRect(0, 0, canvas.width, selection.y);
    context.fillRect(0, selection.y, selection.x, selection.height);
    context.fillRect(selection.x + selection.width, selection.y, canvas.width - selection.x - selection.width, selection.height);
    context.fillRect(0, selection.y + selection.height, canvas.width, canvas.height - selection.y - selection.height);
    context.save();
    context.beginPath();
    context.rect(selection.x, selection.y, selection.width, selection.height);
    context.clip();
    for (const annotation of annotations)
        drawAnnotation(context, annotation);
    if (draft)
        drawAnnotation(context, draft);
    context.restore();
    context.strokeStyle = '#20c997';
    context.lineWidth = Math.max(1, canvas.width / Math.max(window.innerWidth, 1));
    context.setLineDash([6, 4]);
    context.strokeRect(selection.x, selection.y, selection.width, selection.height);
    context.setLineDash([]);
}
/**
 * 根据源图选区更新工具栏和尺寸提示位置。
 * @returns 无返回值。
 */
function positionControls() {
    if (!selection || !sourceImage) {
        toolbar.hidden = true;
        sizeLabel.hidden = true;
        guide.hidden = false;
        return;
    }
    const scaleX = window.innerWidth / canvas.width;
    const scaleY = window.innerHeight / canvas.height;
    const left = selection.x * scaleX;
    const top = selection.y * scaleY;
    const width = selection.width * scaleX;
    const height = selection.height * scaleY;
    toolbar.hidden = false;
    sizeLabel.hidden = false;
    guide.hidden = true;
    sizeLabel.textContent = `${Math.round(selection.width)} × ${Math.round(selection.height)}`;
    sizeLabel.style.left = `${Math.max(8, left)}px`;
    sizeLabel.style.top = `${Math.max(8, top - 31)}px`;
    // 优先放在选区下方，空间不足时移到选区上方。
    const toolbarWidth = toolbar.offsetWidth || 620;
    const toolbarHeight = toolbar.offsetHeight || 44;
    const toolbarLeft = Math.max(8, Math.min(Math.max(8, left + width - toolbarWidth), window.innerWidth - toolbarWidth - 8));
    const below = top + height + 8;
    const toolbarTop = below + toolbarHeight <= window.innerHeight ? below : Math.max(8, top - toolbarHeight - 8);
    toolbar.style.left = `${toolbarLeft}px`;
    toolbar.style.top = `${toolbarTop}px`;
}
/**
 * 切换当前绘图工具并同步按钮状态。
 * @param tool 新工具。
 * @returns 无返回值。
 */
function setTool(tool) {
    cancelTextInput();
    activeTool = tool;
    toolbar.querySelectorAll('[data-tool]').forEach((button) => {
        button.classList.toggle('active', button.dataset.tool === tool);
    });
    canvas.style.cursor = tool === 'text' ? 'text' : 'crosshair';
}
/**
 * 在点击位置显示编辑层内文字输入框，避免打开阻塞式系统弹窗。
 * @param point 文字标注的源图坐标。
 * @returns 无返回值。
 */
function openTextInput(point) {
    if (!selection)
        return;
    pendingTextPoint = clampToSelection(point);
    const bounds = canvas.getBoundingClientRect();
    const left = bounds.left + (pendingTextPoint.x / canvas.width) * bounds.width;
    const top = bounds.top + (pendingTextPoint.y / canvas.height) * bounds.height;
    // 输入框限制在可视区域，防止副屏边缘点击后无法确认文字。
    textInput.style.left = `${Math.min(window.innerWidth - 230, Math.max(8, left))}px`;
    textInput.style.top = `${Math.min(window.innerHeight - 42, Math.max(8, top))}px`;
    textInput.value = '';
    textInput.hidden = false;
    textInput.focus();
}
/**
 * 提交编辑层中的文字标注并恢复画布焦点。
 * @returns 无返回值。
 */
function commitTextInput() {
    const text = textInput.value.trim();
    if (text && pendingTextPoint) {
        annotations.push({ kind: 'text', point: pendingTextPoint, text, color: activeColor });
    }
    cancelTextInput();
    redraw();
}
/**
 * 关闭文字输入框并清除尚未提交的位置。
 * @returns 无返回值。
 */
function cancelTextInput() {
    pendingTextPoint = null;
    textInput.hidden = true;
    textInput.value = '';
}
/**
 * 开始选区或标注手势。
 * @param event 指针按下事件。
 * @returns 无返回值。
 */
function handlePointerDown(event) {
    if (busy || event.button !== 0)
        return;
    const point = eventPoint(event);
    if (activeTool !== 'select' && !pointInsideSelection(point))
        return;
    if (activeTool === 'text') {
        openTextInput(point);
        return;
    }
    canvas.setPointerCapture(event.pointerId);
    dragStart = activeTool === 'select' ? point : clampToSelection(point);
    if (activeTool === 'select') {
        selection = { x: point.x, y: point.y, width: 0, height: 0 };
        annotations = [];
    }
    else if (activeTool === 'rect') {
        draft = { kind: 'rect', start: dragStart, end: dragStart, color: activeColor };
    }
    else if (activeTool === 'arrow') {
        draft = { kind: 'arrow', start: dragStart, end: dragStart, color: activeColor };
    }
    else if (activeTool === 'pen') {
        draft = { kind: 'pen', points: [dragStart], color: activeColor };
    }
    redraw();
    positionControls();
}
/**
 * 更新正在拖动的选区或标注草稿。
 * @param event 指针移动事件。
 * @returns 无返回值。
 */
function handlePointerMove(event) {
    if (!dragStart)
        return;
    const point = activeTool === 'select' ? eventPoint(event) : clampToSelection(eventPoint(event));
    if (activeTool === 'select') {
        selection = normalizedRect(dragStart, point);
    }
    else if (draft?.kind === 'rect' || draft?.kind === 'arrow') {
        draft.end = point;
    }
    else if (draft?.kind === 'pen') {
        draft.points.push(point);
    }
    redraw();
    positionControls();
}
/**
 * 提交当前选区或标注手势。
 * @param event 指针抬起事件。
 * @returns 无返回值。
 */
function handlePointerUp(event) {
    if (!dragStart)
        return;
    if (canvas.hasPointerCapture(event.pointerId))
        canvas.releasePointerCapture(event.pointerId);
    if (activeTool === 'select') {
        if (!selection || selection.width < 5 || selection.height < 5)
            selection = null;
    }
    else if (draft) {
        annotations.push(draft);
    }
    dragStart = null;
    draft = null;
    redraw();
    positionControls();
}
/**
 * 把当前选区和标注渲染成独立 PNG。
 * @returns PNG 字节及选区在截图源图中的物理像素位置和尺寸。
 * @throws 当前没有有效选区或浏览器无法编码 PNG 时抛错。
 */
async function renderSelection() {
    if (!sourceImage || !selection || selection.width < 1 || selection.height < 1) {
        throw new Error('请先选择截图区域');
    }
    const output = document.createElement('canvas');
    output.width = Math.max(1, Math.round(selection.width));
    output.height = Math.max(1, Math.round(selection.height));
    const outputContext = output.getContext('2d');
    outputContext.drawImage(sourceImage, selection.x, selection.y, selection.width, selection.height, 0, 0, output.width, output.height);
    outputContext.save();
    outputContext.translate(-selection.x, -selection.y);
    for (const annotation of annotations)
        drawAnnotation(outputContext, annotation);
    outputContext.restore();
    const blob = await new Promise((resolve, reject) => {
        output.toBlob((value) => (value ? resolve(value) : reject(new Error('无法编码截图 PNG'))), 'image/png');
    });
    return {
        bytes: new Uint8Array(await blob.arrayBuffer()),
        x: selection.x,
        y: selection.y,
        width: output.width,
        height: output.height
    };
}
/**
 * 显示短暂状态或错误信息。
 * @param text 要展示的文字。
 * @param error 是否采用错误样式。
 * @returns 无返回值。
 */
function showMessage(text, error = false) {
    message.textContent = text;
    message.classList.toggle('error', error);
    message.hidden = false;
    window.setTimeout(() => {
        message.hidden = true;
    }, 2200);
}
/**
 * 执行复制、保存或贴图动作并防止重复提交。
 * @param action 输出动作。
 * @returns 操作完成后的 Promise。
 */
async function exportSelection(action) {
    if (busy)
        return;
    busy = true;
    toolbar.classList.add('busy');
    showMessage(action === 'pin' ? '正在创建贴图…' : '正在处理截图…');
    try {
        const rendered = await renderSelection();
        if (action === 'copy') {
            await invoke('screenshot_copy', rendered.bytes);
        }
        else if (action === 'save') {
            await invoke('screenshot_save', rendered.bytes);
        }
        else {
            // 贴图 PNG 使用原始二进制 IPC，选区元数据通过小型请求头传递。
            await invoke('screenshot_pin', rendered.bytes, {
                headers: {
                    'x-ztools-selection-x': String(rendered.x),
                    'x-ztools-selection-y': String(rendered.y),
                    'x-ztools-selection-width': String(rendered.width),
                    'x-ztools-selection-height': String(rendered.height)
                }
            });
        }
    }
    catch (error) {
        busy = false;
        toolbar.classList.remove('busy');
        showMessage(String(error), true);
    }
}
/**
 * 处理工具栏工具、颜色和输出按钮。
 * @param event 工具栏点击事件。
 * @returns 无返回值。
 */
function handleToolbarClick(event) {
    const button = event.target.closest('button');
    if (!button || busy)
        return;
    const tool = button.dataset.tool;
    if (tool) {
        setTool(tool);
        return;
    }
    const color = button.dataset.color;
    if (color) {
        activeColor = color;
        toolbar.querySelectorAll('[data-color]').forEach((candidate) => {
            candidate.classList.toggle('active', candidate === button);
        });
        return;
    }
    const action = button.dataset.action;
    if (action === 'undo') {
        annotations.pop();
        redraw();
    }
    else if (action === 'copy' || action === 'save' || action === 'pin') {
        void exportSelection(action);
    }
    else if (action === 'cancel') {
        void invoke('screenshot_cancel');
    }
}
/**
 * 提供取消、撤销和快速输出键盘操作。
 * @param event 全局键盘事件。
 * @returns 无返回值。
 */
function handleKeyboard(event) {
    if (event.target === textInput) {
        if (event.key === 'Enter') {
            event.preventDefault();
            commitTextInput();
        }
        else if (event.key === 'Escape') {
            event.preventDefault();
            cancelTextInput();
        }
        return;
    }
    if (event.key === 'Escape') {
        event.preventDefault();
        void invoke('screenshot_cancel');
    }
    else if ((event.ctrlKey || event.metaKey) && event.key.toLocaleLowerCase() === 'z') {
        event.preventDefault();
        annotations.pop();
        redraw();
    }
    else if (event.key === 'Enter' && selection) {
        event.preventDefault();
        void exportSelection('copy');
    }
    else if (event.key.toLocaleLowerCase() === 's' && selection) {
        event.preventDefault();
        void exportSelection('save');
    }
    else if (event.key.toLocaleLowerCase() === 'p' && selection) {
        event.preventDefault();
        void exportSelection('pin');
    }
}
/**
 * 在窗口尺寸变化后重新定位浮动控件并重绘。
 * @returns 无返回值。
 */
function handleResize() {
    redraw();
    positionControls();
}
/**
 * 读取宿主截图并初始化与源像素一一对应的编辑画布。
 * @returns 初始化完成后的 Promise。
 */
async function initialize() {
    try {
        const [raw, info] = await Promise.all([
            invoke('screenshot_editor_source'),
            invoke('screenshot_editor_info')
        ]);
        const bytes = responseBytes(raw);
        sourceUrl = URL.createObjectURL(new Blob([new Uint8Array(bytes).buffer], { type: 'image/png' }));
        const image = new Image();
        image.src = sourceUrl;
        await image.decode();
        sourceImage = image;
        // 宿主尺寸和浏览器解码结果必须一致，否则坐标映射可能产生错误裁剪。
        if (sourceImage.naturalWidth !== info.width || sourceImage.naturalHeight !== info.height) {
            throw new Error('截图尺寸校验失败');
        }
        canvas.width = info.width;
        canvas.height = info.height;
        redraw();
        // 首帧准备完成后再发布原生窗口，避免用户看到黑色或白色加载闪烁。
        await invoke('screenshot_editor_ready');
    }
    catch (error) {
        // 初始化异常时仍显示编辑器内错误，用户可按 Esc 安全退出并恢复启动器。
        await invoke('screenshot_editor_ready').catch(() => undefined);
        showMessage(`截图编辑器加载失败：${String(error)}`, true);
    }
}
canvas.addEventListener('pointerdown', handlePointerDown);
canvas.addEventListener('pointermove', handlePointerMove);
canvas.addEventListener('pointerup', handlePointerUp);
canvas.addEventListener('pointercancel', handlePointerUp);
toolbar.addEventListener('click', handleToolbarClick);
textInput.addEventListener('blur', () => {
    if (!textInput.hidden)
        commitTextInput();
});
window.addEventListener('keydown', handleKeyboard);
window.addEventListener('resize', handleResize);
window.addEventListener('beforeunload', () => {
    if (sourceUrl)
        URL.revokeObjectURL(sourceUrl);
});
setTool('select');
void initialize();
