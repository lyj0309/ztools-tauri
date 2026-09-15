let latestPayload = ''

/**
 * 显示宿主派发的进入动作，并保存可复制的文本快照。
 * @param {{ code: string, type: string, payload: unknown }} action 插件进入动作。
 * @returns 无返回值。
 */
function handlePluginEnter(action) {
  // 先转换成稳定文本，避免把对象隐式渲染成无意义字符串。
  latestPayload = typeof action.payload === 'string' ? action.payload : JSON.stringify(action.payload)
  document.querySelector('#payload').textContent = `${action.code} (${action.type})\n${latestPayload}`
}

/**
 * 将当前参数写入系统剪贴板。
 * @returns 无返回值。
 */
function copyPayload() {
  // 剪贴板副作用由 Rust 宿主执行，页面本身没有 Node 权限。
  window.ztools.copyText(latestPayload)
}

/**
 * 等待持久化队列完成后关闭插件窗口。
 * @returns 操作完成后结束的 Promise。
 */
async function closePlugin() {
  // 退出 API 会等待本窗口已经排队的数据库和剪贴板操作。
  await window.ztools.outPlugin()
}

window.ztools.onPluginEnter(handlePluginEnter)
document.querySelector('#copy').addEventListener('click', copyPayload)
document.querySelector('#close').addEventListener('click', closePlugin)
