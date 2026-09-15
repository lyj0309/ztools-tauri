# 无 Node 插件开发

ZTools Tauri 插件仍然使用 HTML、CSS 和 JavaScript。页面运行在独立系统 WebView 中，发布版没有 Node、Electron 或后台 JavaScript sidecar。需要系统能力时调用启动前注入的 `window.ztools`；类型声明位于 [`plugin-sdk/index.d.ts`](../plugin-sdk/index.d.ts)。

最小目录：

```text
hello-plugin/
├── plugin.json
├── index.html
└── app.js
```

`plugin.json`：

```json
{
  "name": "hello-plugin",
  "title": "Hello Plugin",
  "description": "无 Node 插件示例",
  "version": "1.0.0",
  "main": "index.html",
  "features": [{ "code": "hello", "explain": "打招呼", "cmds": ["hello", "你好"] }]
}
```

插件在页面脚本执行前就能使用 `window.ztools`：

```js
window.ztools.onPluginEnter((action) => {
  document.querySelector('#payload').textContent = String(action.payload ?? '')
})
```

文件 API 使用授权路径。插件只能访问用户在启动器拖入、files feature 传入或 `await window.ztools.dialog.open()` 选择的路径。同步旧 API 只用于内存快照；文件、对话框、Shell 和网络副作用均使用 Promise。

本地调试时，在设置 → 插件中填写包含 `plugin.json` 的目录并安装。修改页面后重新安装即可刷新副本；开发构建需要 Node/pnpm 编译 Vue 主界面，发布应用和插件运行期不需要 Node。

完整示例位于 [`plugin-examples/hello-plugin`](../plugin-examples/hello-plugin)。

