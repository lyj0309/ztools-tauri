<script setup lang="ts">
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { pinyin } from 'pinyin-pro'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import ztoolsLogo from '../icons/icon.png'
import {
  addLocalShortcut,
  bootstrapLauncher,
  cancelPluginMarketInstall,
  captureClipboard,
  checkForUpdates,
  clearClipboardHistory,
  clearLaunchHistory,
  closeEmbeddedPlugin,
  detachEmbeddedPlugin,
  copyClipboardText,
  createBackup,
  deleteLocalShortcut,
  deleteClipboardEntry,
  detectLegacyData,
  fetchPluginMarket,
  getInstalledPluginDetail,
  hideMainWindow,
  importLegacyData,
  installPluginDirectory,
  installPluginFromMarket,
  installUpdate,
  launchPluginFeature,
  listPlugins,
  listRunningPlugins,
  launchApplication,
  openDroppedPath,
  openExternalUrl,
  refreshApplications,
  registerPluginDevelopment,
  revealPluginDirectory,
  revealDroppedPath,
  restoreBackup,
  resizeMainWindow,
  runSystemCommand,
  sendTestNotification,
  setApplicationPinned,
  setPluginSubInput,
  syncNow,
  stopPluginDevelopment,
  updateLocalShortcutAlias,
  updateLauncherSettings,
  uninstallPlugin
} from './api'
import type {
  AppEntry,
  ClipboardEntry,
  LauncherSettings,
  LauncherSnapshot,
  LegacyImportReport,
  InstalledPlugin,
  InstalledPluginDetail,
  MarketPlugin,
  MarketInstallProgress,
  SyncStatus,
  UpdateInfo
} from './types'

const searchInput = ref<HTMLInputElement | null>(null)
const query = ref('')
const selectedIndex = ref(0)
const loading = ref(true)
const refreshing = ref(false)
const launchingId = ref<string | null>(null)
const errorMessage = ref('')
const settingsOpen = ref(false)
const activePlugin = ref<{ name: string; title: string; logoUrl: string } | null>(null)
let unlistenPluginPanelOpen: UnlistenFn | undefined
let unlistenPluginPanelClosed: UnlistenFn | undefined
type SettingsSection = 'general' | 'appearance' | 'data' | 'plugins' | 'market' | 'services'
const settingsSection = ref<SettingsSection>('general')
const activeMode = ref<'apps' | 'plugins' | 'clipboard' | 'files'>('apps')
const droppedPaths = ref<string[]>([])
const dragActive = ref(false)
let unlistenDragDrop: (() => void) | undefined
let unlistenClipboard: UnlistenFn | undefined
let unlistenSync: UnlistenFn | undefined
let unlistenApplications: UnlistenFn | undefined
let unlistenPluginFeatures: UnlistenFn | undefined
let unlistenMarketProgress: UnlistenFn | undefined
let unlistenPluginDevelopment: UnlistenFn | undefined
let unlistenOpenSettings: UnlistenFn | undefined
let unlistenScreenshotFinished: UnlistenFn | undefined
let settingsSaveTimer: ReturnType<typeof setTimeout> | undefined
const serviceBusy = ref('')
const serviceMessage = ref('')
const updateInfo = ref<UpdateInfo | null>(null)
const localAliases = ref<Record<string, string>>({})
const legacyPath = ref('')
const legacyReport = ref<LegacyImportReport | null>(null)
const plugins = ref<InstalledPlugin[]>([])
type RecentPluginUsage = { pluginName: string; featureCode: string; usedAt: number }
const recentPluginUsages = ref<RecentPluginUsage[]>([])
const failedPluginLogos = ref(new Set<string>())
const pluginInstallPath = ref('')
const pluginBusy = ref('')
const pluginFilter = ref<'all' | 'running' | 'updates'>('all')
const pluginSearch = ref('')
const selectedPlugin = ref<InstalledPlugin | null>(null)
const selectedPluginDetail = ref<InstalledPluginDetail | null>(null)
const selectedPluginDetailLoading = ref(false)
const pluginDetailTab = ref<'detail' | 'commands' | 'data'>('detail')
const runningPluginNames = ref<string[]>([])
const pinnedPluginNames = ref<string[]>([])
const installedMoreOpen = ref(false)
const localInstallOpen = ref(false)
const marketPlugins = ref<MarketPlugin[]>([])
const marketQuery = ref('')
const marketLoading = ref(false)
const marketCategory = ref('全部')
const marketProgress = ref<Record<string, MarketInstallProgress>>({})
const snapshot = ref<LauncherSnapshot>({
  apps: [],
  history: [],
  clipboard: [],
  pinnedIds: [],
  settings: {
    shortcut: 'Alt+Z',
    autostart: false,
    hideOnBlur: false,
    maxResults: 12,
    theme: 'system',
    accentColor: '#059669',
    showRecent: true,
    clipboardMonitoring: true,
    autoPaste: false,
    clipboardRetentionDays: 180,
    syncEnabled: false,
    syncDirectory: '',
    syncIntervalMinutes: 30,
    autoCheckUpdates: true,
    updateFeedUrl: 'https://github.com/lyj0309/ztools-tauri/releases/latest/download/latest.json',
    updatePublicKey: 'dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEFFRDdCN0VDNUUyMDcwMTUKUldRVmNDQmU3TGZYcmxYSEF3cmNaTkp4MlltZEd2Y2V6VmhrSFJFNjBRMGVLNEcwMzRxSThGeVcK'
  },
  localShortcuts: [],
  syncStatus: { state: 'disabled', message: '同步未启用', lastSyncedAt: null }
})
const settingsDraft = ref<LauncherSettings>({ ...snapshot.value.settings })

const pinnedSet = computed(() => new Set(snapshot.value.pinnedIds))
const historyRank = computed(
  () => new Map(snapshot.value.history.map((entry, index) => [entry.appId, index]))
)
const localPathSet = computed(() => new Set(snapshot.value.localShortcuts.map((item) => item.path)))

const visibleApps = computed(() => {
  const normalized = query.value.trim().toLocaleLowerCase()
  return snapshot.value.apps
    .map((app) => ({ app, score: scoreApplication(app, normalized) }))
    .filter((candidate) => candidate.score > Number.NEGATIVE_INFINITY)
    .sort((left, right) => right.score - left.score || left.app.name.localeCompare(right.app.name))
    .slice(0, snapshot.value.settings.maxResults)
    .map((candidate) => candidate.app)
})

type UnifiedLauncherResult =
  | { kind: 'app'; key: string; app: AppEntry }
  | { kind: 'plugin'; key: string; plugin: InstalledPlugin; featureCode: string; explain: string; recent?: boolean }
  | { kind: 'system'; key: string; commandId: string; title: string; description: string }
  | { kind: 'url'; key: string; url: string; title: string }

const systemCommands = [
  {
    commandId: 'open-home',
    title: '打开主目录',
    description: '在文件管理器中打开用户主目录',
    keywords: 'home 用户 文件夹'
  },
  {
    commandId: 'open-downloads',
    title: '打开下载目录',
    description: '在文件管理器中打开系统下载目录',
    keywords: 'downloads 下载 文件夹'
  },
  {
    commandId: 'open-app-data',
    title: '打开 ZTools 数据目录',
    description: '查看 SQLite 数据和本机配置目录',
    keywords: 'data 数据 sqlite 配置'
  },
  {
    commandId: 'open-plugins',
    title: '打开插件目录',
    description: '查看当前安装的插件文件',
    keywords: 'plugin plugins 插件 扩展'
  },
  {
    commandId: 'open-temp',
    title: '打开临时目录',
    description: '在文件管理器中打开系统临时目录',
    keywords: 'temp tmp 临时 缓存'
  },
  {
    commandId: 'lock-screen',
    title: '锁定屏幕',
    description: '立即锁定当前桌面会话',
    keywords: 'lock screen 锁屏 锁定'
  }
] as const

const matchingSystemActions = computed<UnifiedLauncherResult[]>(() => {
  const normalized = query.value.trim().toLocaleLowerCase()
  if (!normalized) return []
  const commands: UnifiedLauncherResult[] = systemCommands
    .filter((command) =>
      `${command.title} ${command.description} ${command.keywords}`
        .toLocaleLowerCase()
        .includes(normalized)
    )
    .map((command) => ({
      kind: 'system',
      key: `system:${command.commandId}`,
      commandId: command.commandId,
      title: command.title,
      description: command.description
    }))
  const url = normalizedUrl(query.value)
  if (url) {
    commands.unshift({ kind: 'url', key: `url:${url}`, url, title: `打开 ${url}` })
  }
  return commands
})

const matchingPluginActions = computed<UnifiedLauncherResult[]>(() => {
  const rawQuery = query.value.trim()
  if (!rawQuery) return []
  const normalized = rawQuery.toLocaleLowerCase()
  return plugins.value.flatMap((plugin) => {
    const matchingFeatures = plugin.features.filter((feature) =>
      feature.cmds.some((command) => commandMatchesQuery(command, rawQuery))
    )
    const features = matchingFeatures.length
      ? matchingFeatures
      : [plugin.name, plugin.title, plugin.description]
            .join(' ')
            .toLocaleLowerCase()
            .includes(normalized)
        ? plugin.features.slice(0, 1)
        : []
    return features.map((feature) => ({
      kind: 'plugin' as const,
      key: `plugin:${plugin.name}:${feature.code}`,
      plugin,
      featureCode: feature.code,
      explain: feature.explain || plugin.description || plugin.name
    }))
  })
})

const recentPluginResults = computed<UnifiedLauncherResult[]>(() =>
  recentPluginUsages.value.flatMap((usage) => {
    const plugin = plugins.value.find((candidate) => candidate.name === usage.pluginName)
    const feature = plugin?.features.find((candidate) => candidate.code === usage.featureCode)
    return plugin && feature
      ? [{
          kind: 'plugin' as const,
          key: `recent-plugin:${plugin.name}:${feature.code}`,
          plugin,
          featureCode: feature.code,
          explain: feature.explain || plugin.description || plugin.name,
          recent: true
        }]
      : []
  })
)

const visibleLauncherResults = computed<UnifiedLauncherResult[]>(() => {
  if (!query.value.trim()) {
    return (snapshot.value.settings.showRecent ? recentPluginResults.value : []).slice(
      0,
      snapshot.value.settings.maxResults
    )
  }
  return [
    ...matchingSystemActions.value,
    ...matchingPluginActions.value,
    ...visibleApps.value.map((app) => ({ kind: 'app' as const, key: `app:${app.id}`, app }))
  ].slice(0, snapshot.value.settings.maxResults)
})

/**
 * 持久化最近启动的插件功能，并让重复启动项回到列表首位。
 * @param pluginName 插件 manifest 名称。
 * @param featureCode 插件功能编码。
 * @returns 无返回值。
 */
function rememberRecentPluginAction(pluginName: string, featureCode: string): void {
  // 同一插件功能只保留最新一次，避免最近列表被重复启动记录占满。
  recentPluginUsages.value = [
    { pluginName, featureCode, usedAt: Date.now() },
    ...recentPluginUsages.value.filter(
      (entry) => entry.pluginName !== pluginName || entry.featureCode !== featureCode
    )
  ].slice(0, 30)
  // 本地记录沿用已安装插件置顶项的存储位置，不要求插件或 Rust 侧增加专用接口。
  try {
    localStorage.setItem('recent-plugin-usages', JSON.stringify(recentPluginUsages.value))
  } catch {
    // 隐私模式禁止写入时仍在当前窗口显示最近使用项。
  }
}

/**
 * 启动插件功能，并在宿主接受请求后写入最近使用列表。
 * @param pluginName 插件 manifest 名称。
 * @param featureCode 插件功能编码。
 * @param payload 传给插件的文本、文件路径或其他进入数据。
 * @returns 插件启动请求完成后的 Promise。
 * @throws Rust 宿主无法创建或唤起插件窗口时拒绝。
 */
async function launchPluginAction(
  pluginName: string,
  featureCode: string,
  payload: unknown = null
): Promise<void> {
  await launchPluginFeature(pluginName, featureCode, payload)
  rememberRecentPluginAction(pluginName, featureCode)
}

/**
 * 判断插件图标是否存在且尚未报告加载失败。
 * @param plugin 包含名称和图标地址的插件摘要。
 * @returns 可加载图标存在时为 true。
 */
function hasPluginLogo(plugin: { name: string; logoUrl: string }): boolean {
  return Boolean(plugin.logoUrl && !failedPluginLogos.value.has(plugin.name))
}

/**
 * 记录插件图标加载失败，使界面显示名称首字母作为备用图标。
 * @param pluginName 插件 manifest 名称。
 * @returns 无返回值。
 */
function markPluginLogoFailed(pluginName: string): void {
  failedPluginLogos.value = new Set([...failedPluginLogos.value, pluginName])
}

/**
 * 把明确的 HTTP 地址或域名查询转换为可安全打开的 URL。
 * @param value 用户搜索框中的原始文本。
 * @returns 可打开的 HTTP URL；普通搜索词返回 null。
 */
function normalizedUrl(value: string): string | null {
  const raw = value.trim()
  if (!raw || /\s/.test(raw)) return null
  const candidate = /^https?:\/\//i.test(raw)
    ? raw
    : /^[a-z0-9](?:[a-z0-9-]*\.)+[a-z]{2,}(?:[/:?#].*)?$/i.test(raw)
      ? `https://${raw}`
      : ''
  if (!candidate) return null
  try {
    const parsed = new URL(candidate)
    return ['http:', 'https:'].includes(parsed.protocol) ? parsed.toString() : null
  } catch {
    return null
  }
}

const visibleClipboard = computed(() => {
  const normalized = query.value.trim().toLocaleLowerCase()
  return snapshot.value.clipboard.filter(
    (entry) => !normalized || entry.content.toLocaleLowerCase().includes(normalized)
  )
})

const visibleDroppedPaths = computed(() => {
  const normalized = query.value.trim().toLocaleLowerCase()
  return droppedPaths.value.filter(
    (path) => !normalized || path.toLocaleLowerCase().includes(normalized)
  )
})

const visiblePlugins = computed(() => {
  const rawQuery = query.value.trim()
  const normalized = rawQuery.toLocaleLowerCase()
  return plugins.value.filter((plugin) => {
    if (!normalized) return true
    const metadataMatches = [plugin.name, plugin.title, plugin.description]
      .join(' ')
      .toLocaleLowerCase()
      .includes(normalized)
    return (
      metadataMatches ||
      plugin.features.some((feature) =>
        feature.cmds.some((command) => commandMatchesQuery(command, rawQuery))
      )
    )
  })
})

const filePluginActions = computed(() =>
  plugins.value.flatMap((plugin) => {
    const feature = plugin.features.find((candidate) =>
      candidate.cmds.some((command) => fileCommandMatches(command, droppedPaths.value))
    )
    return feature ? [{ plugin, featureCode: feature.code }] : []
  })
)

const installedPluginVersions = computed(
  () => new Map(plugins.value.map((plugin) => [plugin.name, plugin.version]))
)

const visibleMarketPlugins = computed(() => {
  const normalized = marketQuery.value.trim().toLocaleLowerCase()
  return marketPlugins.value
    .filter((plugin) => {
      if (marketCategory.value !== '全部' && plugin.categoryTitle !== marketCategory.value) {
        return false
      }
      return (
        !normalized ||
        [plugin.name, plugin.title, plugin.description, plugin.author, plugin.categoryTitle]
          .join(' ')
          .toLocaleLowerCase()
          .includes(normalized)
      )
    })
    .slice(0, 80)
})

const marketCategories = computed(() => [
  '全部',
  ...new Set(marketPlugins.value.map((plugin) => plugin.categoryTitle).filter(Boolean))
])

/**
 * 判断插件命令声明是否接受当前查询，支持文本命令和市场 manifest 的正则命令。
 * @param command feature.cmds 中的单条命令声明。
 * @param rawQuery 用户尚未标准化的查询文本。
 * @returns 命令是否匹配查询。
 */
function commandMatchesQuery(command: unknown, rawQuery: string): boolean {
  if (typeof command === 'string') {
    return command.toLocaleLowerCase().includes(rawQuery.toLocaleLowerCase())
  }
  if (!command || typeof command !== 'object') return false
  const candidate = command as Record<string, unknown>
  if (candidate.type !== 'regex' || typeof candidate.match !== 'string') return false
  if (typeof candidate.minLength === 'number' && rawQuery.length < candidate.minLength) return false
  if (typeof candidate.maxLength === 'number' && rawQuery.length > candidate.maxLength) return false

  // 市场格式把正则保存成 /pattern/flags，解析失败时仅忽略该条声明。
  const match = candidate.match.match(/^\/(.*)\/([a-z]*)$/i)
  if (!match) return false
  try {
    return new RegExp(match[1], match[2]).test(rawQuery)
  } catch {
    return false
  }
}

/**
 * 判断 files 类型命令是否接受当前拖入路径集合。
 * @param command feature.cmds 中的命令声明。
 * @param paths 操作系统拖放事件提供的绝对路径集合。
 * @returns 文件数量和扩展名是否符合命令约束。
 */
function fileCommandMatches(command: unknown, paths: string[]): boolean {
  if (!command || typeof command !== 'object' || paths.length === 0) return false
  const candidate = command as Record<string, unknown>
  const match =
    candidate.match && typeof candidate.match === 'object'
      ? (candidate.match as Record<string, unknown>)
      : candidate
  if (candidate.type !== 'files' && match.type !== 'files') return false
  const minimum = typeof candidate.minLength === 'number' ? candidate.minLength : 1
  const maximum = typeof candidate.maxLength === 'number' ? candidate.maxLength : Number.MAX_SAFE_INTEGER
  if (paths.length < minimum || paths.length > maximum) return false
  const extensions = Array.isArray(match.extensions)
    ? match.extensions.map((value) => String(value).toLocaleLowerCase())
    : []
  if (extensions.length === 0) return true
  return paths.every((path) => extensions.includes(path.split('.').at(-1)?.toLocaleLowerCase() || ''))
}

/**
 * 根据当前查询选择插件最合适的 feature，未匹配时回退到首个 feature。
 * @param plugin 要启动的插件。
 * @param rawQuery 当前搜索输入。
 * @returns 要传给 onPluginEnter 的 feature 编码。
 */
function pluginFeatureCode(plugin: InstalledPlugin, rawQuery: string): string {
  return (
    plugin.features.find((feature) =>
      feature.cmds.some((command) => commandMatchesQuery(command, rawQuery))
    )?.code ||
    plugin.features[0]?.code ||
    plugin.name
  )
}

const searchPlaceholder = computed(() => {
  if (settingsOpen.value && settingsSection.value === 'plugins') return '搜索已安装插件...'
  if (activeMode.value === 'files') return '筛选已拖入文件'
  return '搜索应用和指令 / 粘贴文件或图片'
})

const hasLauncherContent = computed(
  () =>
    Boolean(query.value.trim() || errorMessage.value) ||
    (activeMode.value === 'apps' && snapshot.value.settings.showRecent && recentPluginResults.value.length > 0) ||
    (activeMode.value === 'files' && droppedPaths.value.length > 0)
)

/**
 * 判断市场版本是否高于本地插件版本。
 * @param installed 本地版本号。
 * @param available 市场版本号。
 * @returns 市场存在更新时为 true。
 */
function isPluginUpdateAvailable(installed: string, available: string): boolean {
  const local = installed.split('.').map((part) => Number.parseInt(part, 10) || 0)
  const remote = available.split('.').map((part) => Number.parseInt(part, 10) || 0)
  for (let index = 0; index < Math.max(local.length, remote.length); index += 1) {
    if ((remote[index] || 0) !== (local[index] || 0)) {
      return (remote[index] || 0) > (local[index] || 0)
    }
  }
  return false
}

const matchingInstalledPlugins = computed(() => {
  const search = pluginSearch.value.trim().toLocaleLowerCase()
  const list = plugins.value.filter((plugin) =>
    `${plugin.title} ${plugin.name} ${plugin.description}`.toLocaleLowerCase().includes(search)
  )
  const filtered = list.filter((plugin) =>
    pluginFilter.value === 'running'
      ? runningPluginNames.value.includes(plugin.name)
      : pluginFilter.value === 'updates'
        ? marketPlugins.value.some((market) => market.name === plugin.name && isPluginUpdateAvailable(plugin.version, market.version))
        : true
  )
  return filtered.sort((left, right) =>
    Number(pinnedPluginNames.value.includes(right.name)) - Number(pinnedPluginNames.value.includes(left.name))
  )
})
const installedUpdates = computed(() =>
  plugins.value.filter((plugin) =>
    marketPlugins.value.some((market) => market.name === plugin.name && isPluginUpdateAvailable(plugin.version, market.version))
  )
)
const selectedMarketPlugin = computed(() =>
  marketPlugins.value.find((plugin) => plugin.name === selectedPlugin.value?.name)
)

/**
 * 按原版 ZTools 的固定宽度和内容高度调整主窗口。
 * @returns 窗口尺寸更新完成后的 Promise。
 */
async function resizeLauncherWindow(): Promise<void> {
  // 设置页和普通插件共用原版 800×600 工作区；空搜索只保留顶部栏。
  let height = 61
  if (settingsOpen.value || activePlugin.value) {
    height = 600
  } else if (hasLauncherContent.value) {
    await nextTick()
    // Grid 容器会拉伸到旧窗口高度；用实际结果项底边计算内容高度才能收起插件工作区。
    const content = document.querySelector<HTMLElement>('.launcher-content')
    const contentTop = content?.getBoundingClientRect().top ?? 0
    const items = content?.querySelectorAll<HTMLElement>('.results-panel > *, .error-banner') ?? []
    const contentHeight = Math.ceil(
      Math.max(0, ...Array.from(items, (item) => item.getBoundingClientRect().bottom - contentTop)) + 8
    )
    height = Math.min(Math.max(61 + contentHeight, 145), 600)
  }
  try {
    // 开发热更新期间窗口可能正在销毁，尺寸同步失败不应中断界面渲染。
    await resizeMainWindow(height)
  } catch (error) {
    console.warn('同步启动器窗口尺寸失败', error)
  }
}

/**
 * 计算应用对当前查询的排序得分，并融入收藏和最近使用权重。
 * @param app 候选应用。
 * @param normalized 已标准化的小写查询。
 * @returns 匹配得分；不匹配时返回负无穷。
 */
function scoreApplication(app: AppEntry, normalized: string): number {
  const name = app.name.toLocaleLowerCase()
  const path = app.path.toLocaleLowerCase()
  const keywords = app.keywords.join(' ').toLocaleLowerCase()
  const pinyinFull = pinyin(app.name, { toneType: 'none', type: 'array' })
    .join('')
    .toLocaleLowerCase()
  const pinyinAbbr = pinyin(app.name, { pattern: 'first', toneType: 'none' })
    .replaceAll(' ', '')
    .toLocaleLowerCase()
  let score = 0

  if (normalized) {
    if (name === normalized) score += 1000
    else if (name.startsWith(normalized)) score += 600
    else if (name.includes(normalized)) score += 300
    else if (keywords.includes(normalized)) score += 160
    else if (pinyinFull.includes(normalized)) score += 150
    else if (pinyinAbbr.includes(normalized)) score += 140
    else if (path.includes(normalized)) score += 80
    else return Number.NEGATIVE_INFINITY
  }
  if (pinnedSet.value.has(app.id)) score += 100
  const rank = historyRank.value.get(app.id)
  if (snapshot.value.settings.showRecent && rank !== undefined) score += Math.max(40 - rank, 1)
  return score
}

/**
 * 从 Rust 宿主加载启动器首屏状态。
 * @returns 加载完成后的 Promise。
 */
async function loadLauncher(): Promise<void> {
  loading.value = true
  errorMessage.value = ''
  try {
    const [launcherSnapshot, installedPlugins] = await Promise.all([
      bootstrapLauncher(),
      listPlugins()
    ])
    snapshot.value = launcherSnapshot
    plugins.value = installedPlugins
    settingsDraft.value = { ...snapshot.value.settings }
    localAliases.value = Object.fromEntries(
      snapshot.value.localShortcuts.map((shortcut) => [shortcut.id, shortcut.alias])
    )
    if (snapshot.value.settings.autoCheckUpdates) void checkUpdates(false)
    void detectLegacyData().then((paths) => {
      if (!legacyPath.value && paths[0]) legacyPath.value = paths[0]
    })
  } catch (error) {
    errorMessage.value = String(error)
  } finally {
    loading.value = false
    await focusSearch()
  }
}

/**
 * 重新扫描系统应用，同时保留当前查询。
 * @returns 刷新完成后的 Promise。
 */
async function refresh(): Promise<void> {
  refreshing.value = true
  errorMessage.value = ''
  try {
    snapshot.value = await refreshApplications()
    selectedIndex.value = 0
  } catch (error) {
    errorMessage.value = String(error)
  } finally {
    refreshing.value = false
  }
}

/**
 * 启动目标应用并在界面缓存中更新最近记录。
 * @param app 要启动的应用。
 * @returns 启动请求完成后的 Promise。
 */
async function launch(app: AppEntry): Promise<void> {
  launchingId.value = app.id
  errorMessage.value = ''
  try {
    await launchApplication(app.id)
    const now = Date.now()
    snapshot.value.history = [
      { appId: app.id, name: app.name, path: app.path, launchedAt: now },
      ...snapshot.value.history.filter((entry) => entry.appId !== app.id)
    ].slice(0, 30)
    query.value = ''
  } catch (error) {
    errorMessage.value = String(error)
  } finally {
    launchingId.value = null
  }
}

/**
 * 切换应用收藏状态并使用数据库返回的顺序刷新界面。
 * @param app 要切换收藏状态的应用。
 * @returns 收藏操作完成后的 Promise。
 */
async function togglePinned(app: AppEntry): Promise<void> {
  const nextPinned = !pinnedSet.value.has(app.id)
  errorMessage.value = ''
  try {
    snapshot.value.pinnedIds = await setApplicationPinned(app.id, nextPinned)
  } catch (error) {
    errorMessage.value = String(error)
  }
}

/**
 * 自动保存设置并采用宿主返回的最终配置，保持原版设置页的即时生效行为。
 * @returns 设置保存完成后的 Promise。
 */
async function saveSettings(): Promise<void> {
  errorMessage.value = ''
  try {
    const saved = await updateLauncherSettings(settingsDraft.value)
    snapshot.value.settings = saved
  } catch (error) {
    errorMessage.value = String(error)
  }
}

/**
 * 合并连续表单输入，停止编辑后再提交一次完整设置。
 * @returns 无返回值。
 */
function scheduleSettingsSave(): void {
  if (!settingsOpen.value) return
  // 文本输入会连续触发更新，延迟提交可避免频繁写库和重复注册快捷键。
  if (settingsSaveTimer) clearTimeout(settingsSaveTimer)
  settingsSaveTimer = setTimeout(() => {
    settingsSaveTimer = undefined
    void saveSettings()
  }, 300)
}

/**
 * 清除数据库中的应用历史和当前界面持久化的插件最近使用项。
 * @returns 清理完成后的 Promise。
 */
async function clearHistory(): Promise<void> {
  errorMessage.value = ''
  try {
    await clearLaunchHistory()
    snapshot.value.history = []
    recentPluginUsages.value = []
    // 清除插件最近使用缓存失败时，也不回滚已完成的数据库清理。
    try {
      localStorage.removeItem('recent-plugin-usages')
    } catch {
      // 当前窗口中的列表仍已清空，浏览器存储限制只影响下次启动恢复。
    }
  } catch (error) {
    errorMessage.value = String(error)
  }
}

/**
 * 捕获当前剪贴板，系统暂时不可用时保留已有历史。
 * @returns 捕获操作完成后的 Promise。
 */
async function refreshClipboard(): Promise<void> {
  try {
    snapshot.value.clipboard = await captureClipboard()
  } catch {
    // Wayland 或无图形会话可能暂时没有剪贴板，不能影响启动器唤起。
  }
}

/**
 * 复制历史文本并隐藏启动器，让用户返回原应用粘贴。
 * @param content 要复制的剪贴板文本。
 * @returns 复制完成后的 Promise。
 */
async function copyClipboard(content: string): Promise<void> {
  errorMessage.value = ''
  try {
    await copyClipboardText(content)
    await hideMainWindow()
  } catch (error) {
    errorMessage.value = String(error)
  }
}

/**
 * 删除指定剪贴板历史记录。
 * @param id 数据库记录标识。
 * @returns 删除完成后的 Promise。
 */
async function removeClipboard(id: number): Promise<void> {
  errorMessage.value = ''
  try {
    snapshot.value.clipboard = await deleteClipboardEntry(id)
  } catch (error) {
    errorMessage.value = String(error)
  }
}

/**
 * 清空全部剪贴板历史和当前界面缓存。
 * @returns 清理完成后的 Promise。
 */
async function clearClipboard(): Promise<void> {
  errorMessage.value = ''
  try {
    await clearClipboardHistory()
    snapshot.value.clipboard = []
  } catch (error) {
    errorMessage.value = String(error)
  }
}

/**
 * 从设置页填写的目录安装插件，并采用宿主返回的最终插件列表。
 * @returns 安装和列表刷新完成后的 Promise。
 */
async function installLocalPlugin(): Promise<void> {
  pluginBusy.value = 'install'
  serviceMessage.value = ''
  try {
    plugins.value = await installPluginDirectory(pluginInstallPath.value.trim())
    pluginInstallPath.value = ''
    serviceMessage.value = '插件已安装；可在主窗口的插件页运行'
  } catch (error) {
    serviceMessage.value = String(error)
  } finally {
    pluginBusy.value = ''
  }
}

/**
 * 注册设置页中的本地目录作为开发插件并启动热重载监听。
 * @returns 初次同步和监听注册完成后的 Promise。
 */
async function registerDevelopmentPlugin(): Promise<void> {
  pluginBusy.value = 'development'
  serviceMessage.value = ''
  try {
    plugins.value = await registerPluginDevelopment(pluginInstallPath.value.trim())
    pluginInstallPath.value = ''
    serviceMessage.value = '开发目录已注册；页面文件变化会自动同步并刷新活动插件'
  } catch (error) {
    serviceMessage.value = String(error)
  } finally {
    pluginBusy.value = ''
  }
}

/**
 * 停止指定插件的开发目录监听并刷新插件状态。
 * @param pluginName 插件稳定名称。
 * @returns 停止监听完成后的 Promise。
 */
async function stopDevelopmentPlugin(pluginName: string): Promise<void> {
  pluginBusy.value = `development:${pluginName}`
  serviceMessage.value = ''
  try {
    plugins.value = await stopPluginDevelopment(pluginName)
    serviceMessage.value = `${pluginName} 已停止开发监听`
  } catch (error) {
    serviceMessage.value = String(error)
  } finally {
    pluginBusy.value = ''
  }
}

/**
 * 从 Rust 宿主加载官方市场目录，重复打开时复用当前结果。
 * @param selectSection 是否切换到市场页。
 * @returns 市场目录加载完成后的 Promise。
 */
async function loadPluginMarket(selectSection = true): Promise<void> {
  if (selectSection) settingsSection.value = 'market'
  if (marketPlugins.value.length || marketLoading.value) return
  marketLoading.value = true
  serviceMessage.value = ''
  try {
    marketPlugins.value = await fetchPluginMarket()
  } catch (error) {
    serviceMessage.value = String(error)
  } finally {
    marketLoading.value = false
  }
}

/**
 * 下载并安装市场插件，再根据兼容状态提示是否能够启动。
 * @param pluginName 市场插件名称。
 * @returns 下载、校验和原子安装完成后的 Promise。
 */
async function installMarketPlugin(pluginName: string): Promise<void> {
  pluginBusy.value = `market:${pluginName}`
  serviceMessage.value = ''
  try {
    plugins.value = await installPluginFromMarket(pluginName)
    if (selectedPlugin.value?.name === pluginName) {
      selectedPlugin.value = plugins.value.find((plugin) => plugin.name === pluginName) || null
    }
    // 安装成功不等于旧 Node preload 已适配，避免把暂不能运行的插件误报为可用。
    const installed = plugins.value.find((plugin) => plugin.name === pluginName)
    serviceMessage.value =
      installed?.compatibility === 'needs-adaptation'
        ? `${installed.title} 已安装，但此版本含未适配的 Node/Electron preload，暂不能运行`
        : `${installed?.title || pluginName} 已安装`
  } catch (error) {
    serviceMessage.value = String(error)
  } finally {
    pluginBusy.value = ''
  }
}

/**
 * 顺序升级已安装插件，避免并行替换多个插件目录造成界面状态混乱。
 * @returns 全部更新任务结束后的 Promise。
 */
async function updateAllInstalledPlugins(): Promise<void> {
  const targets = [...installedUpdates.value]
  if (!targets.length || pluginBusy.value) return
  installedMoreOpen.value = false
  let updated = 0
  const failures: string[] = []
  // 每个插件使用现有的市场下载与原子安装流程，单项失败不阻断其他更新。
  for (const plugin of targets) {
    pluginBusy.value = `market:${plugin.name}`
    try {
      plugins.value = await installPluginFromMarket(plugin.name)
      updated += 1
    } catch {
      failures.push(plugin.title)
    }
  }
  pluginBusy.value = ''
  serviceMessage.value = failures.length
    ? `已更新 ${updated} 个插件；失败：${failures.join('、')}`
    : `已更新 ${updated} 个插件`
}

/**
 * 请求 Rust 下载循环取消当前市场安装。
 * @param pluginName 正在安装的插件名称。
 * @returns 取消请求完成后的 Promise。
 */
async function cancelMarketInstall(pluginName: string): Promise<void> {
  try {
    const cancelled = await cancelPluginMarketInstall(pluginName)
    serviceMessage.value = cancelled ? `${pluginName} 正在取消…` : `${pluginName} 当前没有安装任务`
  } catch (error) {
    serviceMessage.value = String(error)
  }
}

/**
 * 将安装阶段和字节数转换成市场按钮旁的短状态。
 * @param pluginName 市场插件名称。
 * @returns 当前安装阶段和下载百分比。
 */
function marketProgressLabel(pluginName: string): string {
  const progress = marketProgress.value[pluginName]
  if (!progress) return '处理中…'
  if (progress.phase === 'resolving') return '解析地址…'
  if (progress.phase === 'verifying') return '校验中…'
  if (progress.phase === 'installing') return '安装中…'
  if (progress.phase === 'completed') return '已完成'
  if (!progress.totalBytes) return `${formatPluginSize(progress.receivedBytes)}…`
  return `${Math.min(Math.round((progress.receivedBytes / progress.totalBytes) * 100), 100)}%`
}

/**
 * 使用宿主校验后的系统浏览器打开插件主页。
 * @param homepage 市场返回的 HTTPS 项目主页。
 * @returns 打开请求完成后的 Promise。
 */
async function openPluginHomepage(homepage: string): Promise<void> {
  if (!homepage) return
  try {
    await openExternalUrl(homepage)
  } catch (error) {
    serviceMessage.value = String(error)
  }
}

/**
 * 根据本地版本返回市场操作文字。
 * @param plugin 市场插件元数据。
 * @returns 安装、升级或已安装状态文字。
 */
function marketActionLabel(plugin: MarketPlugin): string {
  const installedVersion = installedPluginVersions.value.get(plugin.name)
  if (!installedVersion) return '安装'
  if (installedVersion === plugin.version) return '已安装'
  return `升级 ${plugin.version}`
}

/**
 * 把市场字节数格式化为紧凑的人类可读文本。
 * @param bytes 插件包标称字节数。
 * @returns KB 或 MB 文本。
 */
function formatPluginSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  return `${Math.max(bytes / 1024, 0).toFixed(0)} KB`
}

/**
 * 启动插件的首个功能，并把当前查询作为文本进入动作。
 * @param plugin 要启动的插件。
 * @returns 插件窗口显示完成后的 Promise。
 */
async function runPlugin(plugin: InstalledPlugin): Promise<void> {
  pluginBusy.value = plugin.name
  errorMessage.value = ''
  try {
    const featureCode = pluginFeatureCode(plugin, query.value.trim())
    await launchPluginAction(plugin.name, featureCode, query.value || null)
  } catch (error) {
    errorMessage.value = String(error)
  } finally {
    pluginBusy.value = ''
  }
}

/**
 * 关闭当前内嵌插件并回到主搜索界面。
 * @returns 插件 Webview 关闭后的 Promise。
 */
async function closeActivePlugin(): Promise<void> {
  const plugin = activePlugin.value
  if (!plugin) return
  // Rust 关闭视图并撤销插件身份后，由关闭事件同步界面状态。
  await closeEmbeddedPlugin(plugin.name)
  activePlugin.value = null
  runningPluginNames.value = runningPluginNames.value.filter((name) => name !== plugin.name)
  await resizeLauncherWindow()
  void focusSearch()
}

/**
 * 双击顶部搜索栏时把当前插件移到独立窗口，主窗口回到搜索。
 * @returns 插件分离完成后的 Promise。
 */
async function detachActivePlugin(): Promise<void> {
  const plugin = activePlugin.value
  if (!plugin) return
  // Rust 负责迁移原 Webview 并发出关闭工作区事件，前端等待事件同步布局。
  try {
    await detachEmbeddedPlugin(plugin.name)
  } catch (error) {
    errorMessage.value = `分离插件失败：${String(error)}`
    console.error('分离插件失败', error)
  }
}

/**
 * 在剪贴板插件工作区把查询发送给原版列表，其他页面仍回到启动器搜索。
 * @returns 无返回值。
 */
function onSearchInput(): void {
  selectedIndex.value = 0
  if (settingsOpen.value && settingsSection.value === 'plugins') pluginSearch.value = query.value
  if (activePlugin.value?.name === 'clipboard') {
    // 查询变化由下方响应式监听传给插件；这里仅保留原版内嵌工作区。
    return
  }
  if (activePlugin.value) void closeActivePlugin()
}

/**
 * 切换已安装插件的置顶状态并保存到当前应用配置。
 * @param pluginName 插件 manifest 名称。
 * @returns 无返回值。
 */
function toggleInstalledPin(pluginName: string): void {
  // 新置顶项排在最前，取消置顶时只移除当前名称。
  pinnedPluginNames.value = pinnedPluginNames.value.includes(pluginName)
    ? pinnedPluginNames.value.filter((name) => name !== pluginName)
    : [pluginName, ...pinnedPluginNames.value]
  localStorage.setItem('installed-plugin-pins', JSON.stringify(pinnedPluginNames.value))
}

/**
 * 从已安装插件详情返回列表。
 * @returns 无返回值。
 */
function closeInstalledDetail(): void {
  selectedPlugin.value = null
  selectedPluginDetail.value = null
}

/**
 * 打开已安装插件的内嵌详情，并恢复原版默认详情页。
 * @param plugin 要查看的插件。
 * @returns 插件说明和数据目录加载后的 Promise。
 */
async function openInstalledDetail(plugin: InstalledPlugin): Promise<void> {
  selectedPlugin.value = plugin
  pluginDetailTab.value = 'detail'
  selectedPluginDetail.value = null
  selectedPluginDetailLoading.value = true
  try {
    // 只在用户打开详情时读取 README 和数据键，避免拖慢插件列表。
    selectedPluginDetail.value = await getInstalledPluginDetail(plugin.name)
  } catch (error) {
    serviceMessage.value = String(error)
  } finally {
    selectedPluginDetailLoading.value = false
  }
}

/**
 * 停止当前运行的插件 Webview。
 * @param pluginName 插件 manifest 名称。
 * @returns 插件关闭后的 Promise。
 */
async function stopRunningPlugin(pluginName: string): Promise<void> {
  try {
    await closeEmbeddedPlugin(pluginName)
    runningPluginNames.value = runningPluginNames.value.filter((name) => name !== pluginName)
    if (activePlugin.value?.name === pluginName) activePlugin.value = null
  } catch (error) {
    serviceMessage.value = String(error)
  }
}

/**
 * 从 Rust 当前 Webview 实例同步已安装插件的运行状态。
 * @returns 状态查询完成后的 Promise。
 */
async function refreshInstalledRunning(): Promise<void> {
  try {
    runningPluginNames.value = await listRunningPlugins()
  } catch (error) {
    serviceMessage.value = String(error)
  }
}

/**
 * 打开指定插件的安装目录并在页面中显示失败原因。
 * @param pluginName 插件 manifest 名称。
 * @returns 文件管理器调用完成后的 Promise。
 */
async function openInstalledFolder(pluginName: string): Promise<void> {
  try {
    await revealPluginDirectory(pluginName)
  } catch (error) {
    serviceMessage.value = String(error)
  }
}

/**
 * 启动统一搜索结果中的系统应用或精确插件 feature。
 * @param result 统一结果模型。
 * @returns 启动请求完成后的 Promise。
 */
async function launchUnifiedResult(result: UnifiedLauncherResult): Promise<void> {
  if (result.kind === 'app') {
    await launch(result.app)
    return
  }
  if (result.kind === 'system') {
    try {
      await runSystemCommand(result.commandId)
      await hideMainWindow()
    } catch (error) {
      errorMessage.value = String(error)
    }
    return
  }
  if (result.kind === 'url') {
    try {
      await openExternalUrl(result.url)
      await hideMainWindow()
    } catch (error) {
      errorMessage.value = String(error)
    }
    return
  }
  pluginBusy.value = result.plugin.name
  errorMessage.value = ''
  try {
    await launchPluginAction(result.plugin.name, result.featureCode, query.value.trim())
  } catch (error) {
    errorMessage.value = String(error)
  } finally {
    pluginBusy.value = ''
  }
}

/**
 * 使用当前拖入路径启动 files 类型插件 feature，并由 Rust 授予本窗口临时路径权限。
 * @param plugin 要启动的插件。
 * @param featureCode files 类型 feature 编码。
 * @returns 插件窗口显示完成后的 Promise。
 */
async function runPluginWithFiles(plugin: InstalledPlugin, featureCode: string): Promise<void> {
  pluginBusy.value = plugin.name
  errorMessage.value = ''
  try {
    await launchPluginAction(plugin.name, featureCode, [...droppedPaths.value])
  } catch (error) {
    errorMessage.value = String(error)
  } finally {
    pluginBusy.value = ''
  }
}

/**
 * 关闭活动窗口并卸载指定插件。
 * @param pluginName 要卸载的 manifest 名称。
 * @param removeData 是否同时清除插件私有数据。
 * @returns 卸载和列表刷新完成后的 Promise。
 */
async function removePlugin(pluginName: string, removeData = false): Promise<void> {
  pluginBusy.value = pluginName
  serviceMessage.value = ''
  try {
    plugins.value = await uninstallPlugin(pluginName, removeData)
    if (selectedPlugin.value?.name === pluginName) selectedPlugin.value = null
    serviceMessage.value = removeData ? '插件及其私有数据已删除' : '插件已卸载，私有数据已保留'
  } catch (error) {
    serviceMessage.value = String(error)
  } finally {
    pluginBusy.value = ''
  }
}

/**
 * 在用户二次确认后卸载插件并永久清除它的私有数据。
 * @param pluginName 要卸载的 manifest 名称。
 * @returns 用户取消时直接结束，否则等待卸载完成。
 */
async function removePluginWithData(pluginName: string): Promise<void> {
  if (!window.confirm(`确定卸载 ${pluginName} 并永久删除它的全部私有数据吗？`)) return
  await removePlugin(pluginName, true)
}

/**
 * 使用系统默认程序打开拖入的文件或目录。
 * @param path 要打开的绝对路径。
 * @returns 打开完成后的 Promise。
 */
async function openFile(path: string): Promise<void> {
  errorMessage.value = ''
  try {
    await openDroppedPath(path)
    await hideMainWindow()
  } catch (error) {
    errorMessage.value = String(error)
  }
}

/**
 * 在系统文件管理器中定位拖入的路径。
 * @param path 要定位的绝对路径。
 * @returns 定位完成后的 Promise。
 */
async function revealFile(path: string): Promise<void> {
  errorMessage.value = ''
  try {
    await revealDroppedPath(path)
  } catch (error) {
    errorMessage.value = String(error)
  }
}

/**
 * 将拖入路径保存为长期本地启动项。
 * @param path 要保存的绝对路径。
 * @returns 保存完成后的 Promise。
 */
async function pinDroppedPath(path: string): Promise<void> {
  errorMessage.value = ''
  try {
    snapshot.value = await addLocalShortcut(path)
    serviceMessage.value = '已加入本地启动项'
  } catch (error) {
    errorMessage.value = String(error)
  }
}

/**
 * 保存本地启动项别名并刷新应用搜索数据。
 * @param id 本地启动项标识。
 * @returns 保存完成后的 Promise。
 */
async function saveLocalAlias(id: string): Promise<void> {
  try {
    snapshot.value = await updateLocalShortcutAlias(id, localAliases.value[id] || '')
    serviceMessage.value = '别名已保存'
  } catch (error) {
    errorMessage.value = String(error)
  }
}

/**
 * 删除本地启动项并刷新应用搜索数据。
 * @param id 本地启动项标识。
 * @returns 删除完成后的 Promise。
 */
async function removeLocalShortcut(id: string): Promise<void> {
  try {
    snapshot.value = await deleteLocalShortcut(id)
    delete localAliases.value[id]
  } catch (error) {
    errorMessage.value = String(error)
  }
}

/**
 * 从 Electron 版 LMDB 只读导入仍属于宿主的数据。
 * @returns 导入和界面刷新完成后的 Promise。
 */
async function runLegacyImport(): Promise<void> {
  serviceBusy.value = 'legacy'
  serviceMessage.value = ''
  try {
    legacyReport.value = await importLegacyData(legacyPath.value)
    snapshot.value = await bootstrapLauncher()
    settingsDraft.value = { ...snapshot.value.settings }
    localAliases.value = Object.fromEntries(
      snapshot.value.localShortcuts.map((shortcut) => [shortcut.id, shortcut.alias])
    )
    serviceMessage.value = '旧数据导入完成，原目录未改动'
  } catch (error) {
    serviceMessage.value = String(error)
  } finally {
    serviceBusy.value = ''
  }
}

/**
 * 让用户选择目标文件并创建包含数据库与插件的完整备份。
 * @returns 备份流程结束后的 Promise。
 */
async function runCreateBackup(): Promise<void> {
  serviceBusy.value = 'backup'
  serviceMessage.value = ''
  try {
    const report = await createBackup()
    if (report) {
      serviceMessage.value = `备份完成：${report.pluginCount} 个插件，${formatPluginSize(report.totalBytes)}`
    }
  } catch (error) {
    serviceMessage.value = String(error)
  } finally {
    serviceBusy.value = ''
  }
}

/**
 * 让用户选择备份文件，恢复后重新加载启动器与插件状态。
 * @returns 恢复流程结束后的 Promise。
 */
async function runRestoreBackup(): Promise<void> {
  serviceBusy.value = 'restore'
  serviceMessage.value = ''
  try {
    const report = await restoreBackup()
    if (report) {
      snapshot.value = await bootstrapLauncher()
      settingsDraft.value = { ...snapshot.value.settings }
      plugins.value = await listPlugins()
      localAliases.value = Object.fromEntries(
        snapshot.value.localShortcuts.map((shortcut) => [shortcut.id, shortcut.alias])
      )
      serviceMessage.value = `恢复完成：${report.pluginCount} 个插件，${report.pluginFileCount} 个文件`
    }
  } catch (error) {
    serviceMessage.value = String(error)
  } finally {
    serviceBusy.value = ''
  }
}

/**
 * 执行共享目录同步并把状态显示在设置页。
 * @returns 同步完成后的 Promise。
 */
async function runSync(): Promise<void> {
  serviceBusy.value = 'sync'
  serviceMessage.value = ''
  try {
    snapshot.value.syncStatus = await syncNow()
    snapshot.value = await bootstrapLauncher()
    serviceMessage.value = '同步完成'
  } catch (error) {
    snapshot.value.syncStatus = { state: 'error', message: String(error), lastSyncedAt: null }
    serviceMessage.value = String(error)
  } finally {
    serviceBusy.value = ''
  }
}

/**
 * 先保存服务设置，再立即执行一次同步。
 * @returns 保存和同步均完成后的 Promise。
 */
async function saveAndSync(): Promise<void> {
  try {
    snapshot.value.settings = await updateLauncherSettings(settingsDraft.value)
    await runSync()
  } catch (error) {
    serviceMessage.value = String(error)
  }
}

/**
 * 先保存更新源，再检查最新版本。
 * @returns 保存和检查均完成后的 Promise。
 */
async function saveAndCheckUpdates(): Promise<void> {
  try {
    snapshot.value.settings = await updateLauncherSettings(settingsDraft.value)
    await checkUpdates(true)
  } catch (error) {
    serviceMessage.value = String(error)
  }
}

/**
 * 检查远端发布源并选择是否展示失败信息。
 * @param showErrors 是否把错误显示到设置页。
 * @returns 检查完成后的 Promise。
 */
async function checkUpdates(showErrors = true): Promise<void> {
  serviceBusy.value = 'update'
  if (showErrors) serviceMessage.value = ''
  try {
    updateInfo.value = await checkForUpdates()
    if (showErrors) {
      serviceMessage.value = updateInfo.value.updateAvailable ? '发现新版本' : '当前已是最新版本'
    }
  } catch (error) {
    if (showErrors) serviceMessage.value = String(error)
  } finally {
    serviceBusy.value = ''
  }
}

/**
 * 请求通知权限并发送宿主测试通知。
 * @returns 通知测试完成后的 Promise。
 */
async function testNotification(): Promise<void> {
  serviceBusy.value = 'notification'
  serviceMessage.value = ''
  try {
    await sendTestNotification()
    serviceMessage.value = '测试通知已发送'
  } catch (error) {
    serviceMessage.value = String(error)
  } finally {
    serviceBusy.value = ''
  }
}

/**
 * 打开原生全屏选区截图与标注编辑器。
 * @returns 编辑器窗口创建完成后的 Promise。
 */
async function runScreenCapture(): Promise<void> {
  serviceBusy.value = 'screenshot'
  serviceMessage.value = ''
  try {
    await launchPluginAction('screenshot', 'capture', null)
    serviceMessage.value = '请选择截图区域，可标注后复制、保存或贴图'
  } catch (error) {
    serviceMessage.value = String(error)
  } finally {
    serviceBusy.value = ''
  }
}

/**
 * 在系统浏览器打开已校验的更新发布页。
 * @returns 浏览器启动完成后的 Promise。
 */
async function openUpdatePage(): Promise<void> {
  if (!updateInfo.value?.releaseUrl) return
  try {
    await openExternalUrl(updateInfo.value.releaseUrl)
  } catch (error) {
    serviceMessage.value = String(error)
  }
}

/**
 * 下载并安装已由 Tauri 公钥验证的更新。
 * @returns 安装或失败反馈完成后的 Promise。
 */
async function runUpdateInstall(): Promise<void> {
  serviceBusy.value = 'install-update'
  serviceMessage.value = '正在下载并验证更新…'
  try {
    await installUpdate()
  } catch (error) {
    serviceMessage.value = String(error)
    serviceBusy.value = ''
  }
}

/**
 * 注册 Rust 后台服务向当前窗口发送的状态事件。
 * @returns 后台服务和内置插件事件订阅都完成后的 Promise。
 */
async function registerServiceEvents(): Promise<void> {
  unlistenClipboard = await listen<ClipboardEntry[]>('clipboard-history-updated', (event) => {
    snapshot.value.clipboard = event.payload
  })
  unlistenSync = await listen<SyncStatus>('sync-status-updated', (event) => {
    snapshot.value.syncStatus = event.payload
  })
  unlistenApplications = await listen<AppEntry[]>('applications-updated', (event) => {
    // 后台事件只携带系统应用，本地启动项继续使用当前快照中的记录。
    const localApps = snapshot.value.apps.filter((app) => app.source.startsWith('local-'))
    snapshot.value.apps = [...event.payload, ...localApps]
  })
  unlistenPluginFeatures = await listen<string>('plugin-features-changed', async () => {
    // 动态 feature 已在 Rust 持久化，重新读取合并结果以刷新搜索索引。
    plugins.value = await listPlugins()
  })
  unlistenMarketProgress = await listen<MarketInstallProgress>(
    'plugin-market-install-progress',
    (event) => {
      marketProgress.value = {
        ...marketProgress.value,
        [event.payload.pluginName]: event.payload
      }
    }
  )
  unlistenPluginDevelopment = await listen<{
    pluginName: string
    state: 'reloaded' | 'error'
    message: string
  }>('plugin-development-status', (event) => {
    serviceMessage.value = `${event.payload.pluginName}: ${event.payload.message}`
  })
  unlistenOpenSettings = await listen<SettingsSection>('open-settings-section', (event) => {
    // Rust 已校验分页标识，前端只负责切换现有设置界面。
    void openSettingsSection(event.payload)
  })
  unlistenPluginPanelOpen = await listen<{ name: string; title: string }>(
    'plugin-panel-open',
    (event) => {
      // 顶栏图标从已安装摘要取回，Rust 事件只传插件身份和标题。
      const plugin = plugins.value.find((entry) => entry.name === event.payload.name)
      activePlugin.value = { ...event.payload, logoUrl: plugin?.logoUrl || '' }
      // 插件内容占用搜索框下方的工作区，宿主只保留顶栏和关闭入口。
      settingsOpen.value = false
      if (event.payload.name === 'clipboard') query.value = ''
      if (event.payload.name === 'clipboard') {
        void setPluginSubInput('clipboard', '').catch((error) => { errorMessage.value = String(error) })
      }
      if (!runningPluginNames.value.includes(event.payload.name)) {
        runningPluginNames.value = [...runningPluginNames.value, event.payload.name]
      }
    }
  )
  unlistenPluginPanelClosed = await listen<string>('plugin-panel-closed', async (event) => {
    if (activePlugin.value?.name === event.payload) activePlugin.value = null
    // 分离只关闭主窗口中的工作区；仍在独立窗口运行的插件要保留运行标记。
    try {
      runningPluginNames.value = await listRunningPlugins()
    } catch (error) {
      console.warn('刷新运行中插件失败', error)
    }
    // Linux 在销毁 GtkFixed 后才解除最小高度，下一帧重新计算搜索结果高度。
    window.setTimeout(() => void resizeLauncherWindow(), 80)
  })
  unlistenScreenshotFinished = await listen<{ action: string; path?: string | null }>(
    'screenshot-finished',
    (event) => {
      // 编辑器结束后在设置页显示最终动作，便于确认复制、保存或贴图结果。
      const { action, path } = event.payload
      if (action === 'saved') serviceMessage.value = `截图已保存：${path ?? ''}`
      else if (action === 'copied') serviceMessage.value = '截图已复制到剪贴板'
      else if (action === 'pinned') serviceMessage.value = '截图已创建为悬浮贴图'
      else serviceMessage.value = '已取消截图'
    }
  )
}

/**
 * 根据键盘输入按原版九列网格移动选择、启动结果或关闭窗口。
 * @param event 搜索框键盘事件。
 * @returns 无返回值。
 */
function handleKeyboard(event: KeyboardEvent): void {
  // 和原版一样，在插件工作区用 Ctrl/Cmd+D 触发独立窗口分离。
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'd' && activePlugin.value) {
    event.preventDefault()
    void detachActivePlugin()
    return
  }
  const resultCount =
    activeMode.value === 'apps'
      ? visibleLauncherResults.value.length
      : activeMode.value === 'plugins'
        ? visiblePlugins.value.length
      : activeMode.value === 'clipboard'
        ? visibleClipboard.value.length
        : filePluginActions.value.length + visibleDroppedPaths.value.length
  const gridStep = activeMode.value === 'apps' ? 9 : 1
  if (event.key === 'ArrowDown') {
    event.preventDefault()
    selectedIndex.value = Math.min(selectedIndex.value + gridStep, Math.max(resultCount - 1, 0))
  } else if (event.key === 'ArrowUp') {
    event.preventDefault()
    selectedIndex.value = Math.max(selectedIndex.value - gridStep, 0)
  } else if (event.key === 'ArrowRight' || event.key === 'Tab') {
    event.preventDefault()
    selectedIndex.value = resultCount ? (selectedIndex.value + 1) % resultCount : 0
  } else if (event.key === 'ArrowLeft') {
    event.preventDefault()
    selectedIndex.value = resultCount
      ? (selectedIndex.value - 1 + resultCount) % resultCount
      : 0
  } else if (event.key === 'Enter') {
    if (activeMode.value === 'apps') {
      const result = visibleLauncherResults.value[selectedIndex.value]
      if (result) void launchUnifiedResult(result)
    } else if (activeMode.value === 'plugins') {
      const plugin = visiblePlugins.value[selectedIndex.value]
      if (plugin) void runPlugin(plugin)
    } else if (activeMode.value === 'clipboard') {
      const entry = visibleClipboard.value[selectedIndex.value]
      if (entry) void copyClipboard(entry.content)
    } else {
      const action = filePluginActions.value[selectedIndex.value]
      if (action) void runPluginWithFiles(action.plugin, action.featureCode)
      else {
        const path = visibleDroppedPaths.value[selectedIndex.value - filePluginActions.value.length]
        if (path) void openFile(path)
      }
    }
  } else if (event.key === 'Escape') {
    if (settingsOpen.value) settingsOpen.value = false
    else if (activePlugin.value) void closeActivePlugin()
    else void hideMainWindow()
  }
}

/**
 * 切换结果类别并重置查询与键盘位置。
 * @param mode 要显示的结果类别。
 * @returns 无返回值。
 */
function selectMode(mode: 'apps' | 'plugins' | 'clipboard' | 'files'): void {
  activeMode.value = mode
  query.value = ''
  selectedIndex.value = 0
  if (mode === 'clipboard') void refreshClipboard()
  void focusSearch()
}

/**
 * 打开设置面板并复制当前已保存配置作为草稿。
 * @returns 无返回值。
 */
function openSettings(): void {
  if (activePlugin.value) void closeActivePlugin()
  settingsDraft.value = { ...snapshot.value.settings }
  settingsOpen.value = true
}

/**
 * 响应内置设置插件的 feature，打开宿主设置面板中的对应分页。
 * @param section 要显示的设置分页。
 * @returns 市场分页完成首次目录加载后结束的 Promise。
 */
async function openSettingsSection(section: SettingsSection): Promise<void> {
  // 打开面板前重新复制已保存值，避免保留上次取消编辑的草稿。
  if (activePlugin.value) await closeActivePlugin()
  settingsDraft.value = { ...snapshot.value.settings }
  settingsSection.value = section
  settingsOpen.value = true
  if (section === 'plugins') {
    query.value = ''
    pluginSearch.value = ''
    await refreshInstalledRunning()
  }
  if (section === 'market') await loadPluginMarket()
  if (section === 'plugins') void loadPluginMarket(false)
}

/**
 * 重置查询和键盘选择位置。
 * @returns 无返回值。
 */
function clearQuery(): void {
  query.value = ''
  selectedIndex.value = 0
  void focusSearch()
}

/**
 * 等待 DOM 更新后聚焦并选中搜索框内容。
 * @returns 聚焦完成后的 Promise。
 */
async function focusSearch(): Promise<void> {
  await nextTick()
  searchInput.value?.focus()
  searchInput.value?.select()
}

/**
 * 从路径提取简短来源文字，避免结果行被完整路径挤占。
 * @param app 当前应用记录。
 * @returns 用于副标题的目录或来源。
 */
function appSubtitle(app: AppEntry): string {
  const separators = /[/\\]/
  const parts = app.path.split(separators)
  return parts.length > 2 ? parts.slice(-2).join('/') : app.source
}

/**
 * 返回文件路径的末级名称作为结果标题。
 * @param path 文件或目录绝对路径。
 * @returns 可展示的文件名。
 */
function fileName(path: string): string {
  return path.split(/[/\\]/).filter(Boolean).at(-1) || path
}

/**
 * 响应宿主窗口重新获得焦点并刷新剪贴板。
 * @returns 无返回值。
 */
function handleWindowFocus(): void {
  void focusSearch()
  void refreshClipboard()
}

/**
 * 注册 Tauri 原生文件拖放事件并保存清理函数。
 * @returns 注册完成后的 Promise。
 */
async function registerDragAndDrop(): Promise<void> {
  unlistenDragDrop = await getCurrentWebviewWindow().onDragDropEvent((event) => {
    if (event.payload.type === 'over') {
      dragActive.value = true
    } else if (event.payload.type === 'leave') {
      dragActive.value = false
    } else {
      // 只接受系统事件提供的绝对路径，具体存在性仍由 Rust 二次校验。
      droppedPaths.value = [...new Set(event.payload.paths)]
      dragActive.value = false
      selectMode('files')
    }
  })
}

watch(
  [
    query,
    settingsOpen,
    activePlugin,
    activeMode,
    loading,
    errorMessage,
    () => visibleLauncherResults.value.length,
    () => droppedPaths.value.length
  ],
  () => {
    // 插件工作区已由 Rust 固定为 800×600；输入搜索词时重复 GTK resize 会把子 Webview 挤走。
    if (activePlugin.value) return
    void resizeLauncherWindow()
  },
  { flush: 'post' }
)

watch(settingsDraft, scheduleSettingsSave, { deep: true, flush: 'sync' })
watch(query, (value) => {
  if (activePlugin.value?.name !== 'clipboard') return
  // 输入、清空和程序设置查询均同步到原版剪贴板的子输入接口。
  void setPluginSubInput('clipboard', value).catch((error) => {
    errorMessage.value = String(error)
  })
})

onMounted(() => {
  // 首帧先恢复原版单行启动器尺寸，再异步加载内容。
  try {
    const savedPins = JSON.parse(localStorage.getItem('installed-plugin-pins') || '[]')
    pinnedPluginNames.value = Array.isArray(savedPins)
      ? savedPins.filter((name): name is string => typeof name === 'string')
      : []
  } catch {
    pinnedPluginNames.value = []
  }
  try {
    const savedRecentPlugins = JSON.parse(localStorage.getItem('recent-plugin-usages') || '[]')
    recentPluginUsages.value = Array.isArray(savedRecentPlugins)
      ? savedRecentPlugins.filter(
          (entry): entry is RecentPluginUsage =>
            entry !== null &&
            typeof entry === 'object' &&
            typeof entry.pluginName === 'string' &&
            typeof entry.featureCode === 'string' &&
            typeof entry.usedAt === 'number'
        )
      : []
  } catch {
    recentPluginUsages.value = []
  }
  void resizeLauncherWindow()
  void loadLauncher()
  void registerDragAndDrop()
  void registerServiceEvents()
  window.addEventListener('focus', handleWindowFocus)
})

onUnmounted(() => {
  // 页面销毁时释放原生事件订阅、待提交设置和浏览器焦点监听。
  unlistenDragDrop?.()
  unlistenClipboard?.()
  unlistenSync?.()
  unlistenApplications?.()
  unlistenPluginFeatures?.()
  unlistenMarketProgress?.()
  unlistenPluginDevelopment?.()
  unlistenOpenSettings?.()
  unlistenPluginPanelOpen?.()
  unlistenPluginPanelClosed?.()
  unlistenScreenshotFinished?.()
  if (settingsSaveTimer) clearTimeout(settingsSaveTimer)
  window.removeEventListener('focus', handleWindowFocus)
})
</script>

<template>
  <main
    class="launcher-shell"
    :data-theme="snapshot.settings.theme"
    :style="{ '--accent-color': snapshot.settings.accentColor }"
    @keydown="handleKeyboard"
  >
    <section class="search-panel" @dblclick="detachActivePlugin">
      <div class="search-field">
        <div v-if="activePlugin" class="active-plugin-tag">
          <img
            v-if="hasPluginLogo(activePlugin)"
            :src="activePlugin.logoUrl"
            alt=""
            draggable="false"
            @error="markPluginLogoFailed(activePlugin.name)"
          />
          <span v-else class="active-plugin-fallback">{{ activePlugin.title.slice(0, 1).toLocaleUpperCase() }}</span>
          <span class="active-plugin-title">{{ activePlugin.title }}</span>
          <button type="button" class="active-plugin-close" title="关闭插件" @click.stop="closeActivePlugin">×</button>
        </div>
        <input
          ref="searchInput"
          v-model="query"
          type="text"
          autocomplete="off"
          spellcheck="false"
          :placeholder="searchPlaceholder"
          :aria-label="searchPlaceholder"
          @input="onSearchInput"
        />
      </div>
      <span v-if="query" class="tab-hint">切换选中 <kbd>Tab</kbd></span>
      <button class="profile-button" :title="settingsOpen ? '返回搜索' : '设置'" @click="settingsOpen ? settingsOpen = false : openSettings()">
        <img :src="ztoolsLogo" alt="ZTools" />
      </button>
    </section>

    <section v-if="hasLauncherContent && !settingsOpen && !activePlugin" class="launcher-content" aria-live="polite">
      <p v-if="errorMessage" class="error-banner">{{ errorMessage }}</p>
      <div class="results-panel">
      <div v-if="loading" class="empty-state">
        <span class="loader"></span>
        <p>正在读取系统应用…</p>
      </div>
      <div v-else-if="activeMode === 'apps' && visibleLauncherResults.length === 0" class="empty-state">
        <strong>没有找到应用或插件指令</strong>
        <p>换一个名称、命令或路径关键字试试</p>
      </div>
      <template v-else-if="activeMode === 'apps'">
        <div class="section-caption">{{ query.trim() ? '最佳搜索结果' : '最近使用' }}</div>
        <button
          v-for="(result, index) in visibleLauncherResults"
          :key="result.key"
          class="result-row"
          :class="{ 'plugin-row': result.kind === 'plugin', selected: index === selectedIndex }"
          @mouseenter="selectedIndex = index"
          @click="selectedIndex = index; launchUnifiedResult(result)"
        >
          <template v-if="result.kind === 'app'">
            <span class="app-icon app-icon-placeholder">{{ result.app.name.slice(0, 1).toLocaleUpperCase() }}</span>
            <span class="app-copy">
              <strong>{{ result.app.name }}</strong>
              <small>{{ appSubtitle(result.app) }}</small>
            </span>
            <span v-if="launchingId === result.app.id" class="row-status">启动中</span>
            <span
              v-else-if="snapshot.settings.showRecent && historyRank.has(result.app.id)"
              class="row-status"
              >最近使用</span
            >
            <span
              class="pin-button"
              :class="{ pinned: pinnedSet.has(result.app.id) }"
              role="button"
              :aria-label="pinnedSet.has(result.app.id) ? '取消收藏' : '收藏'"
              @click.stop="togglePinned(result.app)"
            >
              ★</span
            >
          </template>
          <template v-else-if="result.kind === 'plugin'">
            <span class="app-icon plugin-icon">
              <img v-if="hasPluginLogo(result.plugin)" :src="result.plugin.logoUrl" alt="" @error="markPluginLogoFailed(result.plugin.name)" />
              <template v-else>{{ result.plugin.title.slice(0, 1).toLocaleUpperCase() }}</template>
            </span>
            <span class="app-copy">
              <strong>{{ result.plugin.name === 'screenshot' && result.featureCode === 'pin' ? '贴图' : result.plugin.title }}</strong>
              <small>{{ result.explain }} · {{ result.featureCode }}</small>
            </span>
            <span class="row-status">{{ result.recent ? '最近使用' : '插件指令' }}</span>
          </template>
          <template v-else>
            <span class="app-icon">{{ result.kind === 'url' ? '↗' : '⌘' }}</span>
            <span class="app-copy">
              <strong>{{ result.title }}</strong>
              <small>{{ result.kind === 'url' ? result.url : result.description }}</small>
            </span>
            <span class="row-status">{{ result.kind === 'url' ? '网址' : '系统指令' }}</span>
          </template>
          <kbd v-if="index === selectedIndex">↵</kbd>
        </button>
      </template>

      <div v-else-if="activeMode === 'plugins' && visiblePlugins.length === 0" class="empty-state">
        <strong>还没有可运行插件</strong>
        <p>在设置的插件页安装包含 plugin.json 的目录</p>
      </div>
      <template v-else-if="activeMode === 'plugins'">
        <div class="section-caption">
          <span>{{ query ? `“${query}” 的插件` : '插件' }}</span>
          <span>{{ visiblePlugins.length }} / {{ plugins.length }}</span>
        </div>
        <button
          v-for="(plugin, index) in visiblePlugins"
          :key="plugin.name"
          class="result-row plugin-row"
          :class="{ selected: index === selectedIndex }"
          @mouseenter="selectedIndex = index"
          @dblclick="runPlugin(plugin)"
          @click="selectedIndex = index"
        >
          <span class="app-icon plugin-icon">
            <img v-if="hasPluginLogo(plugin)" :src="plugin.logoUrl" alt="" @error="markPluginLogoFailed(plugin.name)" />
            <template v-else>{{ plugin.title.slice(0, 1).toLocaleUpperCase() }}</template>
          </span>
          <span class="app-copy">
            <strong>{{ plugin.title }}</strong>
            <small>{{ plugin.features[0]?.explain || plugin.description || plugin.name }}</small>
          </span>
          <span
            class="compatibility-badge"
            :class="{ adapting: plugin.compatibility === 'needs-adaptation' }"
            >{{ plugin.compatibility === 'native-webview' ? '可直接运行' : '需适配 preload' }}</span
          >
          <span v-if="pluginBusy === plugin.name" class="row-status">启动中</span>
          <kbd v-else-if="index === selectedIndex">↵</kbd>
        </button>
      </template>

      <div v-else-if="activeMode === 'clipboard' && visibleClipboard.length === 0" class="empty-state">
        <strong>没有剪贴板历史</strong>
        <p>复制文本后重新唤起 ZTools 即可捕获</p>
      </div>
      <template v-else-if="activeMode === 'clipboard'">
        <div class="section-caption">
          <span>纯文本历史</span>
          <button class="caption-action" @click="clearClipboard">清空</button>
        </div>
        <button
          v-for="(entry, index) in visibleClipboard"
          :key="entry.id"
          class="result-row clipboard-row"
          :class="{ selected: index === selectedIndex }"
          @mouseenter="selectedIndex = index"
          @dblclick="copyClipboard(entry.content)"
          @click="selectedIndex = index"
        >
          <span class="app-icon clipboard-icon">⌘</span>
          <span class="app-copy">
            <strong>{{ entry.content }}</strong>
            <small>{{ new Date(entry.capturedAt).toLocaleString() }}</small>
          </span>
          <span></span>
          <span class="remove-button" role="button" @click.stop="removeClipboard(entry.id)">×</span>
          <kbd v-if="index === selectedIndex">复制</kbd>
        </button>
      </template>

      <div v-else-if="visibleDroppedPaths.length === 0" class="empty-state file-drop-empty">
        <strong>把文件或目录拖到这里</strong>
        <p>路径由 Rust 校验后交给系统默认应用打开</p>
      </div>
      <template v-else>
        <template v-if="filePluginActions.length">
          <div class="section-caption">
            <span>可处理这些文件的插件</span>
            <span>{{ filePluginActions.length }}</span>
          </div>
          <button
            v-for="(action, index) in filePluginActions"
            :key="`${action.plugin.name}:${action.featureCode}`"
            class="result-row plugin-row"
            :class="{ selected: index === selectedIndex }"
            @mouseenter="selectedIndex = index"
            @dblclick="runPluginWithFiles(action.plugin, action.featureCode)"
            @click="selectedIndex = index"
          >
            <span class="app-icon plugin-icon">
              <img v-if="hasPluginLogo(action.plugin)" :src="action.plugin.logoUrl" alt="" @error="markPluginLogoFailed(action.plugin.name)" />
              <template v-else>{{ action.plugin.title.slice(0, 1) }}</template>
            </span>
            <span class="app-copy">
              <strong>{{ action.plugin.title }}</strong>
              <small>{{ action.featureCode }} · {{ droppedPaths.length }} 个路径</small>
            </span>
            <span v-if="pluginBusy === action.plugin.name" class="row-status">启动中</span>
            <kbd v-else-if="index === selectedIndex">运行</kbd>
          </button>
        </template>
        <div class="section-caption">
          <span>已拖入路径</span>
          <button class="caption-action" @click="droppedPaths = []">清空</button>
        </div>
        <button
          v-for="(path, index) in visibleDroppedPaths"
          :key="path"
          class="result-row file-row"
          :class="{ selected: index + filePluginActions.length === selectedIndex }"
          @mouseenter="selectedIndex = index + filePluginActions.length"
          @dblclick="openFile(path)"
          @click="selectedIndex = index + filePluginActions.length"
        >
          <span class="app-icon file-icon">↗</span>
          <span class="app-copy">
            <strong>{{ fileName(path) }}</strong>
            <small>{{ path }}</small>
          </span>
          <span
            class="add-button"
            :class="{ added: localPathSet.has(path) }"
            role="button"
            @click.stop="pinDroppedPath(path)"
            >{{ localPathSet.has(path) ? '已加入' : '加入' }}</span
          >
          <span class="reveal-button" role="button" @click.stop="revealFile(path)">定位</span>
          <kbd v-if="index + filePluginActions.length === selectedIndex">打开</kbd>
        </button>
      </template>
      </div>
    </section>

    <div v-if="dragActive" class="drop-overlay">
      <strong>松开以添加文件</strong>
    </div>

    <div v-if="settingsOpen" class="settings-workspace">
      <form class="settings-card" @submit.prevent="saveSettings">
        <nav class="settings-tabs" aria-label="设置类别">
          <button
            type="button"
            :class="{ active: settingsSection === 'general' }"
            @click="settingsSection = 'general'"
            ><span>⚙</span>通用设置</button
          >
          <button
            type="button"
            :class="{ active: settingsSection === 'appearance' }"
            @click="settingsSection = 'appearance'"
            ><span>◈</span>外观设置</button
          >
          <button
            type="button"
            :class="{ active: settingsSection === 'data' }"
            @click="settingsSection = 'data'"
            ><span>▤</span>我的数据</button
          >
          <button
            type="button"
            :class="{ active: settingsSection === 'plugins' }"
            @click="settingsSection = 'plugins'; query = ''; pluginSearch = ''; refreshInstalledRunning(); loadPluginMarket(false)"
            ><span>⌘</span>已安装插件</button
          >
          <button
            type="button"
            :class="{ active: settingsSection === 'market' }"
            @click="loadPluginMarket()"
            ><span>▣</span>插件市场</button
          >
          <button
            type="button"
            :class="{ active: settingsSection === 'services' }"
            @click="settingsSection = 'services'"
            ><span>☁</span>数据同步</button
          >
        </nav>

        <section v-if="settingsSection === 'general'" class="settings-body">
          <h3 class="settings-section-title">基础</h3>
          <label class="field-row">
            <span>全局快捷键</span>
            <input v-model.trim="settingsDraft.shortcut" placeholder="Alt+Z" />
          </label>
          <label class="field-row">
            <span>最多显示结果</span>
            <input v-model.number="settingsDraft.maxResults" type="number" min="4" max="50" />
          </label>
          <label class="switch-row">
            <span><strong>开机自动启动</strong><small>在后台等待全局快捷键</small></span>
            <input v-model="settingsDraft.autostart" type="checkbox" />
          </label>
          <label class="switch-row">
            <span><strong>失去焦点时隐藏</strong><small>切到其他窗口后自动收起</small></span>
            <input v-model="settingsDraft.hideOnBlur" type="checkbox" />
          </label>
        </section>

        <section v-else-if="settingsSection === 'appearance'" class="settings-body">
          <h3 class="settings-section-title">外观</h3>
          <label class="field-row">
            <span>主题</span>
            <select v-model="settingsDraft.theme">
              <option value="system">跟随系统</option>
              <option value="light">浅色</option>
              <option value="dark">深色</option>
            </select>
          </label>
          <label class="field-row">
            <span>主题色</span>
            <input v-model="settingsDraft.accentColor" type="color" />
          </label>
          <label class="switch-row">
            <span><strong>使用最近记录排序</strong><small>常用应用排在更靠前的位置</small></span>
            <input v-model="settingsDraft.showRecent" type="checkbox" />
          </label>
        </section>

        <section v-else-if="settingsSection === 'data'" class="settings-body">
          <h3 class="settings-section-title">我的数据</h3>
          <label class="switch-row">
            <span><strong>持续记录剪贴板</strong><small>本地保存文本和图片历史；文本最多 2 MB</small></span>
            <input v-model="settingsDraft.clipboardMonitoring" type="checkbox" />
          </label>
          <label class="switch-row">
            <span><strong>选择后自动粘贴</strong><small>隐藏窗口后由 Rust 模拟系统粘贴快捷键</small></span>
            <input v-model="settingsDraft.autoPaste" type="checkbox" />
          </label>
          <label class="field-row">
            <span>剪贴板保留天数</span>
            <input
              v-model.number="settingsDraft.clipboardRetentionDays"
              type="number"
              min="1"
              max="3650"
            />
          </label>
          <div class="settings-subtitle">本地启动项</div>
          <div v-if="snapshot.localShortcuts.length === 0" class="settings-empty">
            把文件拖入主窗口后点击“加入”
          </div>
          <div
            v-for="shortcut in snapshot.localShortcuts"
            :key="shortcut.id"
            class="shortcut-editor"
          >
            <span :title="shortcut.path">{{ shortcut.name }}</span>
            <input v-model="localAliases[shortcut.id]" placeholder="别名" />
            <button type="button" @click="saveLocalAlias(shortcut.id)">保存</button>
            <button type="button" class="danger-text" @click="removeLocalShortcut(shortcut.id)">
              删除
            </button>
          </div>
          <div class="inline-actions">
            <button type="button" class="danger-button" @click="clearHistory">清空启动历史</button>
            <button type="button" class="danger-button" @click="clearClipboard">
              清空剪贴板
            </button>
          </div>
          <div class="settings-subtitle">完整备份与恢复</div>
          <div class="service-row">
            <span>
              <strong>数据库和插件</strong>
              <small>使用 SQLite 一致性快照；恢复前校验归档路径、体积和数据库哈希</small>
            </span>
            <div class="service-buttons">
              <button type="button" :disabled="Boolean(serviceBusy)" @click="runCreateBackup">
                {{ serviceBusy === 'backup' ? '备份中…' : '创建备份' }}
              </button>
              <button
                type="button"
                class="danger-text"
                :disabled="Boolean(serviceBusy)"
                @click="runRestoreBackup"
              >
                {{ serviceBusy === 'restore' ? '恢复中…' : '恢复备份' }}
              </button>
            </div>
          </div>
          <div class="settings-subtitle">Electron 旧数据迁移</div>
          <label class="field-row field-row-wide">
            <span>旧数据目录</span>
            <input v-model.trim="legacyPath" placeholder="~/.ztools 或 Electron userData" />
          </label>
          <div class="service-row">
            <span>
              <strong>只读导入 LMDB</strong>
              <small>迁移设置、应用收藏、启动历史和本地启动项；不导入插件</small>
            </span>
            <button type="button" :disabled="serviceBusy === 'legacy'" @click="runLegacyImport">
              {{ serviceBusy === 'legacy' ? '导入中…' : '开始导入' }}
            </button>
          </div>
          <p v-if="legacyReport" class="service-message">
            设置 {{ legacyReport.importedSettings ? '1' : '0' }} 项，收藏
            {{ legacyReport.importedPins }} 项，历史 {{ legacyReport.importedHistory }} 项，本地启动项
            {{ legacyReport.importedShortcuts }} 项。
          </p>
          <p v-if="serviceMessage" class="service-message">{{ serviceMessage }}</p>
        </section>

        <section v-else-if="settingsSection === 'plugins'" class="settings-body installed-plugins-page">
          <template v-if="!selectedPlugin">
            <div class="installed-toolbar">
              <div class="installed-filters" role="tablist" aria-label="插件状态">
                <button type="button" :class="{ active: pluginFilter === 'all' }" @click="pluginFilter = 'all'">
                  全部 <span>{{ plugins.length }}</span>
                </button>
                <button type="button" :class="{ active: pluginFilter === 'running' }" @click="pluginFilter = 'running'">
                  运行中 <span>{{ runningPluginNames.length }}</span>
                </button>
                <button type="button" :class="{ active: pluginFilter === 'updates' }" @click="pluginFilter = 'updates'">
                  更新 <span :class="{ 'has-updates': installedUpdates.length > 0 }">{{ installedUpdates.length }}</span>
                </button>
              </div>
              <div class="installed-more-wrap">
                <button type="button" class="installed-more-button" @click="installedMoreOpen = !installedMoreOpen">更多 <span>⌄</span></button>
                <div v-if="installedMoreOpen" class="installed-more-menu">
                  <button type="button" @click="localInstallOpen = true; installedMoreOpen = false">▣　导入本地插件</button>
                  <button type="button" :disabled="installedUpdates.length === 0 || !!pluginBusy" @click="updateAllInstalledPlugins">↻　全部更新 ({{ installedUpdates.length }})</button>
                  <button type="button" :disabled="runningPluginNames.length === 0" @click="runningPluginNames.forEach((name) => stopRunningPlugin(name)); installedMoreOpen = false">■　停止所有插件</button>
                </div>
              </div>
            </div>
            <div v-if="localInstallOpen" class="installed-import-panel">
              <div class="installed-import-heading"><strong>导入本地插件</strong><button type="button" @click="localInstallOpen = false">×</button></div>
              <input v-model.trim="pluginInstallPath" placeholder="包含 plugin.json 的插件目录绝对路径" data-testid="plugin-install-path" />
              <div class="installed-import-actions">
                <button type="button" :disabled="!pluginInstallPath || !!pluginBusy" data-testid="plugin-install-button" @click="installLocalPlugin">{{ pluginBusy === 'install' ? '安装中…' : '安装插件' }}</button>
                <button type="button" :disabled="!pluginInstallPath || !!pluginBusy" @click="registerDevelopmentPlugin">{{ pluginBusy === 'development' ? '注册中…' : '注册开发目录' }}</button>
              </div>
            </div>
            <div class="installed-list">
              <div
                v-for="plugin in matchingInstalledPlugins"
                :key="plugin.name"
                class="installed-plugin-card"
                :title="plugin.description"
                @click="openInstalledDetail(plugin)"
              >
                <div class="installed-icon-wrap">
                  <img v-if="hasPluginLogo(plugin)" :src="plugin.logoUrl" class="installed-plugin-icon" alt="插件图标" draggable="false" @error="markPluginLogoFailed(plugin.name)" />
                  <div v-else class="installed-plugin-placeholder">{{ plugin.title.slice(0, 1).toLocaleUpperCase() }}</div>
                  <span v-if="plugin.development" class="installed-dev-badge">DEV</span>
                  <span v-if="installedUpdates.some((entry) => entry.name === plugin.name)" class="installed-update-dot"></span>
                </div>
                <div class="installed-plugin-info">
                  <div class="installed-plugin-name">{{ plugin.title || plugin.name }} <span class="installed-version">v{{ plugin.version }}</span>
                    <span v-if="runningPluginNames.includes(plugin.name)" class="installed-running"><i></i>运行中</span>
                  </div>
                  <div class="installed-plugin-description">{{ plugin.description || '暂无描述' }}</div>
                </div>
                <div class="installed-plugin-actions">
                  <button type="button" title="打开插件" :disabled="plugin.compatibility === 'needs-adaptation'" @click.stop="runPlugin(plugin)">▶</button>
                  <button v-if="runningPluginNames.includes(plugin.name)" type="button" title="终止运行" @click.stop="stopRunningPlugin(plugin.name)">■</button>
                  <button type="button" title="打开插件目录" @click.stop="openInstalledFolder(plugin.name)">▣</button>
                  <button type="button" :title="pinnedPluginNames.includes(plugin.name) ? '取消置顶' : '置顶'" :class="{ pinned: pinnedPluginNames.includes(plugin.name) }" @click.stop="toggleInstalledPin(plugin.name)">♙</button>
                </div>
              </div>
              <div v-if="matchingInstalledPlugins.length === 0" class="installed-empty">
                <div>🧩</div>
                <strong>{{ plugins.length === 0 ? '暂无插件' : pluginFilter === 'updates' ? '全部插件均为最新版本' : pluginFilter === 'running' ? '没有运行中的插件' : '未找到匹配的插件' }}</strong>
                <small>{{ plugins.length === 0 ? '点击“导入本地插件”来安装你的第一个插件' : pluginFilter === 'updates' ? '有新版本的插件会出现在这里' : '尝试使用其他关键词搜索' }}</small>
              </div>
            </div>
          </template>
          <template v-else>
            <div class="installed-detail-topbar">
              <button type="button" class="installed-back" @click="closeInstalledDetail">‹ <span>插件详情</span></button>
              <div class="installed-detail-actions">
                <button type="button" title="打开" :disabled="selectedPlugin.compatibility === 'needs-adaptation'" @click="runPlugin(selectedPlugin)">▶</button>
                <button v-if="runningPluginNames.includes(selectedPlugin.name)" type="button" title="终止运行" @click="stopRunningPlugin(selectedPlugin.name)">■</button>
                <button type="button" title="打开插件目录" @click="openInstalledFolder(selectedPlugin.name)">▣</button>
                <button type="button" :title="pinnedPluginNames.includes(selectedPlugin.name) ? '取消置顶' : '置顶'" @click="toggleInstalledPin(selectedPlugin.name)">♙</button>
                <button v-if="!selectedPlugin.builtIn" type="button" title="卸载" @click="removePlugin(selectedPlugin.name)">▤</button>
              </div>
            </div>
            <div class="installed-detail-content">
              <div class="installed-detail-heading">
                <img v-if="hasPluginLogo(selectedPlugin)" :src="selectedPlugin.logoUrl" alt="插件图标" draggable="false" @error="markPluginLogoFailed(selectedPlugin.name)" />
                <div v-else class="installed-detail-placeholder">{{ selectedPlugin.title.slice(0, 1).toLocaleUpperCase() }}</div>
                <div><h3>{{ selectedPlugin.title }}</h3><p>{{ selectedPlugin.description || '暂无描述' }}</p></div>
                <button v-if="selectedMarketPlugin && isPluginUpdateAvailable(selectedPlugin.version, selectedMarketPlugin.version)" type="button" @click="installMarketPlugin(selectedPlugin.name)">更新</button>
              </div>
              <div class="installed-detail-meta">
                <div><small>开发者</small><strong>{{ selectedMarketPlugin?.author || '-' }}</strong></div>
                <div><small>版本</small><strong>{{ selectedPlugin.version }}</strong></div>
                <div><small>状态</small><strong>{{ runningPluginNames.includes(selectedPlugin.name) ? '运行中' : '已安装' }}</strong></div>
              </div>
              <div class="installed-detail-tabs">
                <button type="button" :class="{ active: pluginDetailTab === 'detail' }" @click="pluginDetailTab = 'detail'">详情</button>
                <button type="button" :class="{ active: pluginDetailTab === 'commands' }" @click="pluginDetailTab = 'commands'">指令</button>
                <button type="button" :class="{ active: pluginDetailTab === 'data' }" @click="pluginDetailTab = 'data'">数据</button>
              </div>
              <div v-if="pluginDetailTab === 'detail'" class="installed-detail-tab-content">
                <p v-if="selectedPluginDetailLoading">加载中...</p>
                <pre v-else-if="selectedPluginDetail?.readme" class="installed-readme">{{ selectedPluginDetail.readme }}</pre>
                <p v-else>{{ selectedPlugin.description || '该插件暂无详情说明' }}</p>
                <small v-for="note in selectedPlugin.compatibilityNotes" :key="note">{{ note }}</small>
                <button v-if="selectedPlugin.development" type="button" @click="stopDevelopmentPlugin(selectedPlugin.name)">停止开发目录监听</button>
              </div>
              <div v-else-if="pluginDetailTab === 'commands'" class="installed-detail-tab-content">
                <div v-for="feature in selectedPlugin.features" :key="feature.code" class="installed-feature-card">
                  <div><strong>{{ feature.explain || feature.code }}</strong><small>{{ feature.code }}</small></div>
                  <button type="button" :disabled="selectedPlugin.compatibility === 'needs-adaptation'" @click="launchPluginAction(selectedPlugin.name, feature.code)">打开</button>
                </div>
                <p v-if="selectedPlugin.features.length === 0">该插件暂无指令</p>
              </div>
              <div v-else class="installed-detail-tab-content">
                <p v-if="selectedPluginDetailLoading">加载中...</p>
                <template v-else-if="selectedPluginDetail?.data.length">
                  <div v-for="item in selectedPluginDetail.data" :key="`${item.kind}:${item.key}`" class="installed-data-row">
                    <span><strong>{{ item.key }}</strong><small>{{ item.kind === 'document' ? '文档' : item.kind === 'storage' ? '键值存储' : '附件' }}</small></span>
                    <small>{{ formatPluginSize(item.bytes) }}</small>
                  </div>
                </template>
                <p v-else>该插件暂无数据</p>
              </div>
              <div v-if="!selectedPlugin.builtIn" class="installed-detail-footer">
                <button type="button" @click="removePluginWithData(selectedPlugin.name)">卸载并删除插件数据</button>
              </div>
            </div>
          </template>
          <p v-if="serviceMessage" class="service-message installed-feedback">{{ serviceMessage }}</p>
        </section>

        <section v-else-if="settingsSection === 'market'" class="settings-body">
          <h3 class="settings-section-title">插件市场</h3>
          <div class="market-filters">
            <label class="field-row field-row-wide">
              <span>搜索市场</span>
              <input v-model.trim="marketQuery" placeholder="名称、作者或分类" />
            </label>
            <label class="field-row field-row-wide">
              <span>分类</span>
              <select v-model="marketCategory">
                <option v-for="category in marketCategories" :key="category" :value="category">
                  {{ category }}
                </option>
              </select>
            </label>
          </div>
          <div v-if="marketLoading" class="settings-empty">正在读取官方插件市场…</div>
          <div v-else-if="visibleMarketPlugins.length === 0" class="settings-empty">
            {{ marketPlugins.length ? '没有匹配插件' : '市场目录尚未加载' }}
          </div>
          <div v-for="plugin in visibleMarketPlugins" :key="plugin.name" class="market-plugin-row">
            <span class="market-plugin-icon">
              <img v-if="plugin.logo" :src="plugin.logo" alt="" />
              <template v-else>{{ plugin.title.slice(0, 1).toLocaleUpperCase() }}</template>
            </span>
            <span class="market-plugin-copy">
              <strong>
                {{ plugin.title }} <small>v{{ plugin.version }}</small>
              </strong>
              <small>{{ plugin.description || plugin.name }}</small>
              <small>
                {{ plugin.categoryTitle || '未分类' }} · {{ plugin.author || '未知作者' }} ·
                {{ plugin.sourceLabel || '市场插件' }} · {{ formatPluginSize(plugin.size) }} ·
                {{ plugin.downloadCount }} 次下载
              </small>
              <button
                v-if="plugin.homepage"
                class="market-homepage"
                type="button"
                @click="openPluginHomepage(plugin.homepage)"
                >项目主页</button
              >
            </span>
            <span class="market-actions">
              <button
                type="button"
                :disabled="
                  pluginBusy === `market:${plugin.name}` ||
                  installedPluginVersions.get(plugin.name) === plugin.version
                "
                @click="installMarketPlugin(plugin.name)"
                >{{
                  pluginBusy === `market:${plugin.name}`
                    ? marketProgressLabel(plugin.name)
                    : marketActionLabel(plugin)
                }}</button
              >
              <button
                v-if="pluginBusy === `market:${plugin.name}`"
                type="button"
                class="danger-text"
                @click="cancelMarketInstall(plugin.name)"
                >取消</button
              >
            </span>
          </div>
          <p v-if="serviceMessage" class="service-message">{{ serviceMessage }}</p>
        </section>

        <section v-else class="settings-body">
          <h3 class="settings-section-title">数据同步</h3>
          <label class="switch-row">
            <span><strong>共享目录同步</strong><small>可放在 NAS、Syncthing 或网盘目录</small></span>
            <input v-model="settingsDraft.syncEnabled" type="checkbox" />
          </label>
          <label class="field-row field-row-wide">
            <span>同步目录</span>
            <input v-model.trim="settingsDraft.syncDirectory" placeholder="/绝对路径/ZTools-Sync" />
          </label>
          <label class="field-row">
            <span>同步间隔（分钟）</span>
            <input
              v-model.number="settingsDraft.syncIntervalMinutes"
              type="number"
              min="1"
              max="1440"
            />
          </label>
          <div class="service-row">
            <span><strong>同步状态</strong><small>{{ snapshot.syncStatus.message }}</small></span>
            <button type="button" :disabled="serviceBusy === 'sync'" @click="saveAndSync">
              {{ serviceBusy === 'sync' ? '同步中…' : '保存并同步' }}
            </button>
          </div>
          <label class="switch-row">
            <span><strong>启动后检查更新</strong><small>只读取发布信息，不静默安装</small></span>
            <input v-model="settingsDraft.autoCheckUpdates" type="checkbox" />
          </label>
          <label class="field-row field-row-wide">
            <span>发布源</span>
            <input v-model.trim="settingsDraft.updateFeedUrl" />
          </label>
          <label class="field-row field-row-wide">
            <span>Tauri 签名公钥</span>
            <input v-model.trim="settingsDraft.updatePublicKey" placeholder="发布前配置 minisign 公钥" />
          </label>
          <div class="service-row">
            <span>
              <strong>应用更新</strong>
              <small v-if="updateInfo">当前 {{ updateInfo.currentVersion }} · 最新 {{ updateInfo.latestVersion }}</small>
              <small v-else>支持 GitHub Release 或 Tauri JSON</small>
            </span>
            <button type="button" :disabled="serviceBusy === 'update'" @click="saveAndCheckUpdates">
              检查更新
            </button>
          </div>
          <div v-if="updateInfo?.updateAvailable && updateInfo.releaseUrl" class="service-row">
            <span><strong>发现 {{ updateInfo.latestVersion }}</strong><small>签名公钥已配置时可直接安装</small></span>
            <div class="service-buttons">
              <button type="button" @click="openUpdatePage">打开下载地址</button>
              <button
                v-if="settingsDraft.updatePublicKey"
                type="button"
                :disabled="serviceBusy === 'install-update'"
                @click="runUpdateInstall"
                >验签安装</button
              >
            </div>
          </div>
          <div class="service-row">
            <span><strong>系统通知</strong><small>验证原生通知权限和投递</small></span>
            <button
              type="button"
              :disabled="serviceBusy === 'notification'"
              @click="testNotification"
              >发送测试通知</button
            >
          </div>
          <div class="service-row">
            <span><strong>截图与贴图</strong><small>选区截图后可标注、复制、保存或创建悬浮贴图</small></span>
            <button
              type="button"
              :disabled="serviceBusy === 'screenshot'"
              @click="runScreenCapture"
              >{{ serviceBusy === 'screenshot' ? '启动中…' : '开始截图' }}</button
            >
          </div>
          <p v-if="serviceMessage" class="service-message">{{ serviceMessage }}</p>
        </section>

      </form>
    </div>
  </main>
</template>
