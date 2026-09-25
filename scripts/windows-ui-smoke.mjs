import { spawn } from 'node:child_process'
import { createWriteStream, mkdirSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'

const executable = resolve(process.argv[2] || '')
const artifactRoot = resolve(process.argv[3] || 'artifacts')
const testRoot = join(process.env.RUNNER_TEMP || artifactRoot, 'ztools-windows-ui-smoke')
mkdirSync(artifactRoot, { recursive: true })
mkdirSync(testRoot, { recursive: true })

/**
 * 重试异步检查，等待 Windows WebView2 页面和资源完成渲染。
 * @param {() => Promise<unknown>} check 返回真值表示成功的检查。
 * @param {string} label 超时时显示的检查名称。
 * @param {number} timeoutMs 最长等待时间。
 * @returns {Promise<unknown>} 首个通过检查的结果。
 * @throws 超时或测试进程退出时抛出错误。
 */
async function waitFor(check, label, timeoutMs = 20000) {
  const deadline = Date.now() + timeoutMs
  let lastError = ''
  while (Date.now() < deadline) {
    try {
      const result = await check()
      if (result) return result
    } catch (error) {
      lastError = String(error)
    }
    await delay(300)
  }
  throw new Error(`${label} timed out${lastError ? `: ${lastError}` : ''}`)
}

/**
 * 向 WebView2 调试目标发送一条 CDP 指令并取回对应结果。
 * @param {string} socketUrl 页面调试 WebSocket 地址。
 * @param {string} method CDP 方法名。
 * @param {object} params 方法参数。
 * @returns {Promise<object>} CDP 响应的 result 对象。
 * @throws 连接失败、协议错误或超时时抛出错误。
 */
async function cdp(socketUrl, method, params = {}) {
  const socket = new WebSocket(socketUrl)
  try {
    await Promise.race([
      new Promise((resolveOpen, rejectOpen) => {
        socket.addEventListener('open', resolveOpen, { once: true })
        socket.addEventListener('error', rejectOpen, { once: true })
      }),
      delay(5000).then(() => { throw new Error('WebView2 CDP connection timed out') })
    ])
    return await Promise.race([
      new Promise((resolveResponse, rejectResponse) => {
        socket.addEventListener('message', (event) => {
          const response = JSON.parse(event.data)
          if (response.id !== 1) return
          if (response.error) rejectResponse(new Error(JSON.stringify(response.error)))
          else resolveResponse(response.result)
        })
        socket.send(JSON.stringify({ id: 1, method, params }))
      }),
      delay(5000).then(() => { throw new Error(`${method} timed out`) })
    ])
  } finally {
    socket.close()
  }
}

/**
 * 在主启动器页面执行 JS 并取得可序列化的结果。
 * @param {string} socketUrl 启动器 CDP WebSocket 地址。
 * @param {string} expression 要执行的 JavaScript 表达式。
 * @returns {Promise<unknown>} 页面表达式返回值。
 * @throws 页面执行异常时抛出错误。
 */
async function evaluate(socketUrl, expression) {
  const response = await cdp(socketUrl, 'Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true
  })
  if (response.exceptionDetails) throw new Error(JSON.stringify(response.exceptionDetails))
  return response.result.value
}

/**
 * 找到当前便携版主 WebView 的 CDP 目标。
 * @param {number} port WebView2 调试端口。
 * @returns {Promise<string>} 主启动器 WebSocket 地址。
 */
async function launcherTarget(port) {
  return /** @type {Promise<string>} */ (waitFor(async () => {
    const response = await fetch(`http://127.0.0.1:${port}/json/list`)
    const targets = await response.json()
    for (const target of targets) {
      if (target.type !== 'page' || !target.webSocketDebuggerUrl) continue
      if (await evaluate(target.webSocketDebuggerUrl, "Boolean(document.querySelector('.search-field input'))")) {
        return target.webSocketDebuggerUrl
      }
    }
    return ''
  }, 'launcher CDP target'))
}

/**
 * 启动隔离数据目录下的 Windows 便携版，并开放本次专用的 WebView2 调试端口。
 * @param {number} port 本次测试专用调试端口。
 * @param {string} label 日志文件名称前缀。
 * @returns {import('node:child_process').ChildProcess} 被测程序进程。
 */
function startApp(port, label) {
  const child = spawn(executable, [], {
    env: {
      ...process.env,
      ZTOOLS_E2E: '1',
      ZTOOLS_DATA_ROOT: join(testRoot, 'data'),
      ZTOOLS_PLUGIN_ROOT: join(testRoot, 'plugins'),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`
    },
    stdio: ['ignore', 'pipe', 'pipe']
  })
  child.stdout.pipe(createWriteStream(join(artifactRoot, `${label}-stdout.log`)))
  child.stderr.pipe(createWriteStream(join(artifactRoot, `${label}-stderr.log`)))
  return child
}

/**
 * 关闭本次测试启动的应用进程，释放单实例锁和 WebView2 调试端口。
 * @param {import('node:child_process').ChildProcess | null} child 被测进程。
 * @returns {Promise<void>} 进程退出后的 Promise。
 */
async function stopApp(child) {
  if (!child || child.exitCode !== null) return
  child.kill()
  await Promise.race([
    new Promise((done) => child.once('exit', done)),
    delay(5000)
  ])
}

/**
 * 截取当前主 WebView 可见区域，保留 Windows 端的实际渲染证据。
 * @param {string} socketUrl 启动器 CDP WebSocket 地址。
 * @param {string} filename 输出文件名。
 * @returns {Promise<void>} 截图写入后的 Promise。
 */
async function capture(socketUrl, filename) {
  const response = await cdp(socketUrl, 'Page.captureScreenshot', { format: 'png', captureBeyondViewport: false })
  writeFileSync(join(artifactRoot, filename), Buffer.from(response.data, 'base64'))
}

let first = null
let second = null
try {
  first = startApp(9325, 'ui-first')
  const launcher = await launcherTarget(9325)
  await waitFor(() => evaluate(launcher, "!document.querySelector('.loader')"), 'launcher data')

  // 通过真实 Vue 输入和点击入口打开插件，覆盖命令执行、名称事件与图标加载。
  await evaluate(launcher, `(() => {
    const input = document.querySelector('.search-field input')
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, '剪贴板')
    input.dispatchEvent(new Event('input', { bubbles: true }))
    return true
  })()`)
  const searchLogo = await waitFor(() => evaluate(launcher, `(() => {
    const row = [...document.querySelectorAll('.result-row.plugin-row')]
      .find((element) => element.textContent.includes('剪贴板'))
    const image = row?.querySelector('img')
    return image?.naturalWidth > 0 && image.currentSrc.includes('ztools-plugin.localhost')
  })()`), 'search result plugin icon')
  await capture(launcher, '15-plugin-search.png')
  await evaluate(launcher, `(() => {
    [...document.querySelectorAll('.result-row.plugin-row')]
      .find((element) => element.textContent.includes('剪贴板')).click()
    return true
  })()`)
  const opened = await waitFor(() => evaluate(launcher, `(() => {
    const tag = document.querySelector('.active-plugin-tag')
    const title = tag?.querySelector('.active-plugin-title')
    const image = tag?.querySelector('img')
    const input = document.querySelector('.search-field input')
    return title?.textContent === '剪贴板' && image?.naturalWidth > 0 &&
      tag.getBoundingClientRect().right <= input.getBoundingClientRect().left + 1
  })()`), 'plugin icon and title left of input')
  await capture(launcher, '16-plugin-open.png')
  await evaluate(launcher, "document.querySelector('.active-plugin-close').click()")
  const recent = await waitFor(() => evaluate(launcher, `(() => {
    const row = [...document.querySelectorAll('.result-row.plugin-row')]
      .find((element) => element.textContent.includes('剪贴板'))
    return document.body.textContent.includes('最近使用') && row?.querySelector('img')?.naturalWidth > 0
  })()`), 'recently used plugin on home')
  await capture(launcher, '17-plugin-recent.png')

  // 重新打开同一隔离数据目录，确认“最近使用”不是只存在于当前页面内存中。
  await stopApp(first)
  first = null
  second = startApp(9326, 'ui-restart')
  const restartedLauncher = await launcherTarget(9326)
  const persisted = await waitFor(() => evaluate(restartedLauncher, `(() => {
    const row = [...document.querySelectorAll('.result-row.plugin-row')]
      .find((element) => element.textContent.includes('剪贴板'))
    return document.body.textContent.includes('最近使用') && row?.querySelector('img')?.naturalWidth > 0
  })()`), 'persisted recent plugin after restart')
  await capture(restartedLauncher, '18-plugin-recent-restart.png')
  writeFileSync(join(artifactRoot, 'ui-diagnostics.json'), JSON.stringify({ searchLogo, opened, recent, persisted }, null, 2))
} finally {
  await stopApp(first)
  await stopApp(second)
}
