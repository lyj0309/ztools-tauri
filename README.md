# ZTools Tauri

ZTools 桌面宿主的 Tauri 2 重构版，使用 Rust、Vue 3、TypeScript 和 Vite。发布程序不包含 Electron 或 Node.js 运行时；旧插件 preload 仅在确认可作为浏览器脚本运行或存在内置 Rust 适配器时加载。

## 已实现

- 扫描 Windows 开始菜单、macOS 应用包和 Linux Desktop Entry；支持中文、全拼、拼音首字母、关键字和路径检索。
- 启动系统应用、本地文件和目录；支持拖放添加、别名、收藏、最近使用和自动应用重扫。
- SQLite WAL 持久化设置、收藏、历史、本地启动项、插件数据，以及文本、图片和 Windows 文件剪贴板历史；支持数据库与已安装插件的完整备份恢复。
- 只读导入 Electron ZTools 的 LMDB v1/v2 数据，迁移宿主设置、收藏、历史和本地启动项，不修改源库。
- 全局快捷键、单实例、托盘、开机启动、关闭隐藏、失焦隐藏，以及按鼠标所在显示器居中。
- Rust 后台剪贴板监控、自动粘贴、文件打开/定位、受约束 HTTP 请求、系统通知、屏幕抓取和图片剪贴板。
- 共享目录双向同步，适用于 NAS、Syncthing 和网盘目录；同步文件使用临时文件原子替换。
- GitHub Release/Tauri JSON 更新检查，以及 Tauri minisign 公钥验签、下载、安装和重启。
- 通用、外观、数据和服务设置页；设置保存后即时应用并在重启后恢复。
- `设置`、`系统`、`截图`、`剪贴板`、`有道翻译`、`百度翻译` 与 `休息提醒` 七个默认插件直接编译进可执行文件，首次启动和版本升级时自动安装或修复，便携版也无需外置资源目录。
- `截图` 指令先选区，再打开基于原版快捷截图插件资源的紧凑标注窗口，支持复制、保存、本地 OCR 和贴图；独立的 `贴图` 指令从本应用记录的剪贴板历史图片中选择原图，生成可拖动、滚轮等比缩放的悬浮贴图。
- `剪贴板` 使用原版插件页面展示文本、图片与 Windows 文件历史，保留分类、收藏、多选和主搜索框筛选；`有道翻译` 在搜索框下方打开有道官网，并可将选中文字填入翻译框。原版资源的许可与来源说明保留在各插件目录。
- 图片历史随剪贴板监控开关启停，保留最近 50 张且原图合计不超过 128 MiB，并遵循历史保留天数；清空剪贴板历史时也会清空图片。升级前未记录的图片不会自动补回。
- 截图默认插件支持鼠标所在显示器选区、原版标注工具栏、撤销、保存、图片剪贴板和可缩放置顶贴图。
- 第三方插件目录安装、升级和卸载；`plugin.json` 校验、独立 Webview、私有资源协议与窗口权限隔离。
- 官方插件市场匿名目录、分类、搜索、详情元数据、进度、取消和 ZIP 安装；下载源、压缩包路径、文件数、体积和归档哈希均由 Rust 处理。
- `window.ztools` 兼容 API：生命周期、设备快照、`db`/附件、dbStorage、动态 feature、文本剪贴板、通知、文件、异步对话框、Shell、屏幕截图、基础输入、当前窗口控制和能力检测。
- 应用、插件 feature、固定系统指令和 HTTP(S) 网址统一搜索；上述 7 个代表插件已在 Linux 开发环境验证，Windows/macOS 的实际交互仍需单独验收。
- 插件开发目录注册、隔离副本同步和活动窗口热重载。
- 无 Node 插件类型声明、开发说明和最小示例；参见[插件开发文档](docs/plugin-development.md)。
- GitHub Actions 在 Linux 执行完整检查，并从版本标签构建 Windows、Linux、macOS Intel/Apple Silicon 原生包。

核心插件迁移仍在进行：跨平台文件剪贴板、通用插件图片/文件 API、插件后台模式、基础子窗口、崩溃恢复、旧插件数据迁移和三平台真机验收尚未完成。超级面板、悬浮球、AI、支付和浏览器自动化不在当前核心范围。详细现状见[迁移状态](docs/migration-status.md)，逐项 API 状态见[兼容台账](docs/plugin-api-compatibility.json)，范围与验收门槛见[核心功能迁移方案](docs/core-migration-plan.md)。

## 运行

```bash
pnpm install
pnpm dev
```

Node.js 和 pnpm 仅用于开发时构建 Vue 页面。发布后的桌面进程是 Tauri/Rust 程序，不携带 Node 或 Electron 运行时。

Linux 开发需要 Tauri 2 的 WebKitGTK 4.1、GTK 3、AppIndicator、Rsvg、OpenSSL 和 xdo 开发包。截图优先调用 `grim`、`gnome-screenshot`、`spectacle` 或 `flameshot`，X11 环境可回退到 `ffmpeg`。自动粘贴在 Linux 使用 X11 输入后端；Wayland 是否允许模拟输入取决于桌面和权限配置。

## 验证

```bash
pnpm typecheck
pnpm build:web
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

发布更新前必须在设置或发行配置中填入实际更新地址和 minisign 公钥，并由发布流水线生成已签名的 Tauri 更新包。

### 百度翻译与本地 OCR

- 默认插件「百度翻译」提供文字和图片两个入口，直接在独立窗口打开百度翻译官网，无需填写 APPID 或密钥。网页自行提交翻译请求；文字翻译首次使用时选择「机翻 · 通用领域」，即可匿名翻译。
- 图片翻译由百度网页提供上传与识别。站点若要求登录或安全验证，应在网页中正常完成；本应用不代替用户处理验证。百度网页不会获得 ZTools 插件兼容层或本地文件 API。
- 截图选区后点击「OCR」或按 `O`，可在原界面查看、编辑和复制识别结果，截图不上传。Windows 如已安装含 Windows ML 的 Windows App Runtime，首次识别时从 Paddle 官方下载约 30 MB 的 PP-OCRv6 small 模型到用户缓存，之后离线复用；运行时不可用、下载或推理失败时回退到系统 WinRT OCR（依赖已安装的文字识别语言包）。其他平台调用本机 Tesseract（需自行安装及安装语言包）。便携版内含 Microsoft Windows App SDK bootstrap DLL，其许可见 `src-tauri/resources/windows/LICENSE.txt`。

### 休息提醒

- 在启动器搜索「休息提醒」打开内置插件，设置提醒间隔（默认 20 分钟）、休息页显示时间（默认 10 秒）并启用。默认关闭。
- 启用后由 Rust 后台服务计时，设置页关闭后仍会提醒；休息开始前 10 秒发送系统通知，到点打开休息页。可在休息页点击「跳过本次提醒」，下一轮按设置间隔继续。系统通知需要操作系统授予通知权限。
