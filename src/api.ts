import { invoke } from '@tauri-apps/api/core'
import type {
  BackupReport,
  ClipboardEntry,
  LegacyImportReport,
  LauncherSettings,
  LauncherSnapshot,
  InstalledPlugin,
  InstalledPluginDetail,
  MarketPlugin,
  RestoreReport,
  SyncStatus,
  UpdateInfo
} from './types'

/**
 * 读取 Rust 宿主准备好的启动器完整状态。
 * @returns 含应用、历史、收藏和设置的状态快照。
 */
export function bootstrapLauncher(): Promise<LauncherSnapshot> {
  return invoke<LauncherSnapshot>('bootstrap_launcher')
}

/**
 * 重新扫描操作系统的应用入口并返回最新状态。
 * @returns 刷新后的启动器完整状态。
 */
export function refreshApplications(): Promise<LauncherSnapshot> {
  return invoke<LauncherSnapshot>('refresh_applications')
}

/**
 * 启动已扫描到的应用并记录使用历史。
 * @param appId 要启动的应用标识。
 * @returns 应用启动请求完成后的 Promise。
 */
export function launchApplication(appId: string): Promise<void> {
  return invoke<void>('launch_application', { appId })
}

/**
 * 设置应用的收藏状态。
 * @param appId 要变更的应用标识。
 * @param pinned 是否收藏。
 * @returns 更新后的收藏标识列表。
 */
export function setApplicationPinned(appId: string, pinned: boolean): Promise<string[]> {
  return invoke<string[]>('set_application_pinned', { appId, pinned })
}

/**
 * 清空本地启动历史。
 * @returns 清理完成后的 Promise。
 */
export function clearLaunchHistory(): Promise<void> {
  return invoke<void>('clear_launch_history')
}

/**
 * 捕获系统当前纯文本剪贴板并返回历史。
 * @returns 最新剪贴板历史。
 */
export function captureClipboard(): Promise<ClipboardEntry[]> {
  return invoke<ClipboardEntry[]>('capture_clipboard')
}

/**
 * 将指定文本重新写入系统剪贴板。
 * @param content 要复制的文本。
 * @returns 写入完成后的 Promise。
 */
export function copyClipboardText(content: string): Promise<void> {
  return invoke<void>('copy_clipboard_text', { content })
}

/**
 * 删除单条剪贴板历史。
 * @param id 数据库记录标识。
 * @returns 删除后的剪贴板历史。
 */
export function deleteClipboardEntry(id: number): Promise<ClipboardEntry[]> {
  return invoke<ClipboardEntry[]>('delete_clipboard_entry', { id })
}

/**
 * 清空全部剪贴板历史。
 * @returns 清理完成后的 Promise。
 */
export function clearClipboardHistory(): Promise<void> {
  return invoke<void>('clear_clipboard_history')
}

/**
 * 使用系统默认应用打开拖入的文件或目录。
 * @param path 系统拖放事件提供的绝对路径。
 * @returns 打开操作完成后的 Promise。
 */
export function openDroppedPath(path: string): Promise<void> {
  return invoke<void>('open_dropped_path', { path })
}

/**
 * 在系统文件管理器中定位拖入的文件或目录。
 * @param path 系统拖放事件提供的绝对路径。
 * @returns 定位操作完成后的 Promise。
 */
export function revealDroppedPath(path: string): Promise<void> {
  return invoke<void>('reveal_dropped_path', { path })
}

/**
 * 将有效文件或目录加入本地启动项。
 * @param path 要加入启动器的绝对路径。
 * @returns 更新后的完整启动器快照。
 */
export function addLocalShortcut(path: string): Promise<LauncherSnapshot> {
  return invoke<LauncherSnapshot>('add_local_shortcut', { path })
}

/**
 * 更新本地启动项的搜索和显示别名。
 * @param id 本地启动项标识。
 * @param alias 新别名，空字符串表示恢复文件名。
 * @returns 更新后的完整启动器快照。
 */
export function updateLocalShortcutAlias(id: string, alias: string): Promise<LauncherSnapshot> {
  return invoke<LauncherSnapshot>('update_local_shortcut_alias', { id, alias })
}

/**
 * 删除本地启动项。
 * @param id 本地启动项标识。
 * @returns 更新后的完整启动器快照。
 */
export function deleteLocalShortcut(id: string): Promise<LauncherSnapshot> {
  return invoke<LauncherSnapshot>('delete_local_shortcut', { id })
}

/**
 * 保存设置并立即应用快捷键和开机启动配置。
 * @param settings 新的完整设置。
 * @returns 宿主实际保存的设置。
 */
export function updateLauncherSettings(settings: LauncherSettings): Promise<LauncherSettings> {
  return invoke<LauncherSettings>('update_launcher_settings', { settings })
}

/**
 * 隐藏主窗口。
 * @returns 隐藏操作完成后的 Promise。
 */
export function hideMainWindow(): Promise<void> {
  return invoke<void>('hide_main_window')
}

/**
 * 立即执行一次共享目录双向同步。
 * @returns 本次同步的最终状态。
 */
export function syncNow(): Promise<SyncStatus> {
  return invoke<SyncStatus>('sync_now')
}

/**
 * 发送系统原生测试通知。
 * @returns 通知提交完成后的 Promise。
 */
export function sendTestNotification(): Promise<void> {
  return invoke<void>('send_test_notification')
}

/**
 * 隐藏启动器并截取鼠标所在显示器到系统图片目录。
 * @returns 新截图文件的绝对路径。
 */
export function captureScreen(): Promise<string> {
  return invoke<string>('capture_screen')
}

/**
 * 从设置的发布源检查新版本。
 * @returns 当前版本与远端版本信息。
 */
export function checkForUpdates(): Promise<UpdateInfo> {
  return invoke<UpdateInfo>('check_for_updates')
}

/**
 * 下载、验签并安装发布源中的 Tauri 更新。
 * @returns 安装完成并触发重启前结束的 Promise。
 */
export function installUpdate(): Promise<void> {
  return invoke<void>('install_update')
}

/**
 * 使用系统默认浏览器打开外部 HTTP 地址。
 * @param url 要打开的地址。
 * @returns 系统浏览器启动完成后的 Promise。
 */
export function openExternalUrl(url: string): Promise<void> {
  return invoke<void>('open_external_url', { url })
}

/**
 * 执行宿主内置的固定系统命令。
 * @param commandId 系统命令白名单中的稳定标识。
 * @returns 系统操作提交完成后的 Promise。
 */
export function runSystemCommand(commandId: string): Promise<void> {
  return invoke<void>('run_system_command', { commandId })
}

/**
 * 检测默认位置中已有的 Electron ZTools 数据目录。
 * @returns 存在的旧数据目录绝对路径。
 */
export function detectLegacyData(): Promise<string[]> {
  return invoke<string[]>('detect_legacy_data')
}

/**
 * 从旧 LMDB 只读导入宿主数据。
 * @param path Electron ZTools 数据目录或 LMDB 目录。
 * @returns 导入统计报告。
 */
export function importLegacyData(path: string): Promise<LegacyImportReport> {
  return invoke<LegacyImportReport>('import_legacy_data', { path })
}

/**
 * 通过原生保存对话框创建数据库和插件完整备份。
 * @returns 用户取消时为 null，否则返回备份文件报告。
 */
export function createBackup(): Promise<BackupReport | null> {
  return invoke<BackupReport | null>('create_backup')
}

/**
 * 通过原生文件对话框校验并恢复数据库和插件完整备份。
 * @returns 用户取消时为 null，否则返回恢复报告。
 */
export function restoreBackup(): Promise<RestoreReport | null> {
  return invoke<RestoreReport | null>('restore_backup')
}

/**
 * 扫描 Rust 插件目录并返回 manifest 有效的插件。
 * @returns 已安装插件列表。
 */
export function listPlugins(): Promise<InstalledPlugin[]> {
  return invoke<InstalledPlugin[]>('list_plugins')
}

/**
 * 匿名读取官方插件市场中适用于当前平台的插件目录。
 * @returns 市场插件列表。
 */
export function fetchPluginMarket(): Promise<MarketPlugin[]> {
  return invoke<MarketPlugin[]>('fetch_plugin_market')
}

/**
 * 从官方市场下载并安装或升级指定插件。
 * @param pluginName 市场插件的稳定名称。
 * @returns 安装完成后的完整插件列表。
 */
export function installPluginFromMarket(pluginName: string): Promise<InstalledPlugin[]> {
  return invoke<InstalledPlugin[]>('install_plugin_from_market', { pluginName })
}

/**
 * 请求中止指定插件正在进行的市场安装。
 * @param pluginName 市场插件的稳定名称。
 * @returns 找到活动安装并设置取消标记时为 true。
 */
export function cancelPluginMarketInstall(pluginName: string): Promise<boolean> {
  return invoke<boolean>('cancel_plugin_market_install', { pluginName })
}

/**
 * 从已经解压的本地目录安装或升级插件。
 * @param path 包含 plugin.json 的插件目录绝对路径。
 * @returns 安装完成后的完整插件列表。
 */
export function installPluginDirectory(path: string): Promise<InstalledPlugin[]> {
  return invoke<InstalledPlugin[]>('install_plugin_directory', { path })
}

/**
 * 注册并监听一个本地插件开发目录。
 * @param path 包含 plugin.json 的开发目录绝对路径。
 * @returns 注册完成后的完整插件列表。
 */
export function registerPluginDevelopment(path: string): Promise<InstalledPlugin[]> {
  return invoke<InstalledPlugin[]>('register_plugin_development', { path })
}

/**
 * 停止插件的开发目录监听并保留当前隔离副本。
 * @param pluginName 插件稳定名称。
 * @returns 停止监听后的完整插件列表。
 */
export function stopPluginDevelopment(pluginName: string): Promise<InstalledPlugin[]> {
  return invoke<InstalledPlugin[]>('stop_plugin_development', { pluginName })
}

/**
 * 卸载插件并按选择保留或删除插件私有数据。
 * @param pluginName plugin.json 中的插件名称。
 * @param removeData 是否同时清除文档、键值、动态 feature 和附件。
 * @returns 卸载完成后的完整插件列表。
 */
export function uninstallPlugin(
  pluginName: string,
  removeData = false
): Promise<InstalledPlugin[]> {
  return invoke<InstalledPlugin[]>('uninstall_plugin', { pluginName, removeData })
}

/**
 * 创建或复用主窗口内嵌插件 Webview，并派发功能进入动作。
 * @param pluginName 要启动的插件名称。
 * @param featureCode 要触发的功能编码。
 * @param payload 传给 onPluginEnter 的输入数据。
 * @returns 插件页面完成显示后的 Promise。
 */
export function launchPluginFeature(
  pluginName: string,
  featureCode: string,
  payload: unknown = null
): Promise<void> {
  return invoke<void>('launch_plugin_feature', { pluginName, featureCode, payload })
}

/**
 * 关闭主窗口中的插件页面并撤销它的 API 身份。
 * @param pluginName 要关闭的插件名称。
 * @returns 页面关闭后的 Promise。
 */
export function closeEmbeddedPlugin(pluginName: string): Promise<void> {
  return invoke<void>('close_embedded_plugin', { pluginName })
}

/**
 * 按当前搜索或插件工作区高度调整主窗口。
 * @param height 61 到 600 像素的目标高度。
 * @returns 尺寸调整完成后的 Promise。
 */
export function resizeMainWindow(height: number): Promise<void> {
  return invoke<void>('resize_main_window', { height })
}

/**
 * 在系统文件管理器中打开指定插件的安装目录。
 * @param pluginName 插件 manifest 名称。
 * @returns 文件管理器打开后的 Promise。
 */
export function revealPluginDirectory(pluginName: string): Promise<void> {
  return invoke<void>('reveal_plugin_directory', { pluginName })
}

/**
 * 读取插件 README 和私有数据项的键及大小。
 * @param pluginName 插件 manifest 名称。
 * @returns 插件详情内容。
 */
export function getInstalledPluginDetail(pluginName: string): Promise<InstalledPluginDetail> {
  return invoke<InstalledPluginDetail>('get_installed_plugin_detail', { pluginName })
}

/**
 * 查询当前仍有 Webview 实例的插件名称。
 * @returns 运行中插件名称列表。
 */
export function listRunningPlugins(): Promise<string[]> {
  return invoke<string[]>('list_running_plugins')
}
