# ZTools Tauri 2 架构

新项目以 Tauri command 和 event 作为唯一前后端边界。Vue 负责显示、表单、拼音索引和结果排序；Rust 负责系统资源、数据、进程和后台任务。主窗口使用类型化 API，插件窗口由宿主注入无 Node 的 `window.ztools` 兼容层。

## 代码边界

| 位置 | 职责 |
| --- | --- |
| `src/App.vue` | 启动器、剪贴板、拖放文件和四类设置界面 |
| `src/api.ts` | 类型化 Tauri command 调用 |
| `src-tauri/src/launcher.rs` | 三平台应用扫描、稳定 ID 和可信启动 |
| `src-tauri/src/storage.rs` | SQLite schema、迁移后的宿主数据和同步事务 |
| `src-tauri/src/backup.rs` | SQLite 在线快照、插件归档、完整性校验和恢复回滚 |
| `src-tauri/src/legacy*.rs` | LMDB v1/v2 只读解析与一次性数据转换 |
| `src-tauri/src/desktop.rs` | 剪贴板、输入模拟、截图、文件和外部地址 |
| `src-tauri/src/sync.rs` | 共享文件双向合并和原子发布 |
| `src-tauri/src/services.rs` | 可取消的剪贴板、同步和应用扫描线程 |
| `src-tauri/src/commands` | 参数校验、系统操作和前端可见错误 |
| `src-tauri/src/lib.rs` | Tauri 生命周期、快捷键、托盘、单实例和窗口行为 |

前端启动应用时只提交扫描结果的 ID。Rust 从进程内缓存或 SQLite 本地启动项还原路径，拒绝 WebView 提交任意命令行。外部 URL 限制为 HTTP/HTTPS；HTTP 请求具有重定向、超时、请求体和响应体上限。

## 数据与同步

主数据位于 Tauri 应用数据目录 `top.ztools.launcher/ztools.sqlite3`，使用 SQLite WAL。同步文档不含账号凭据，配置类数据采用较新的完整快照，历史和剪贴板按内容合并；快捷键、开机启动、同步目录和更新密钥等设备级设置留在本机。完整备份通过 SQLite 在线备份 API 固定数据库，再连同正式插件目录写入受路径、数量和体积约束的归档。

旧数据导入器检测 `~/.ztools` 和 Electron `userData` 的常见布局。标准 LMDB v1 由 `heed` 读取；原项目 npm `lmdb` 写出的扩展 v2 格式由 `vendor/lmdb-v2` 中带命名空间的只读兼容库读取。导入过程使用 `MDB_RDONLY | MDB_NOLOCK`，不写锁文件，也不修改旧数据。

## 生命周期

后台服务共享停止标记。应用退出时设置停止状态并等待三个工作线程释放数据库和桌面句柄；长周期休眠被拆成最多一秒的片段。截图和自动粘贴放到阻塞线程并异步等待，使窗口隐藏与焦点切换能先由桌面事件循环提交。

应用更新使用 Tauri updater 的运行时 endpoint 和 minisign 公钥。检查更新只读取元数据；安装路径必须完成签名验证才会下载、安装和重启。
