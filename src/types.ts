export interface AppEntry {
  id: string
  name: string
  path: string
  keywords: string[]
  source: string
}

export interface HistoryEntry {
  appId: string
  name: string
  path: string
  launchedAt: number
}

export interface ClipboardEntry {
  id: number
  content: string
  capturedAt: number
}

export interface LocalShortcut {
  id: string
  name: string
  alias: string
  path: string
  kind: 'file' | 'folder' | 'app'
  addedAt: number
}

export interface LauncherSettings {
  shortcut: string
  autostart: boolean
  hideOnBlur: boolean
  maxResults: number
  theme: 'system' | 'light' | 'dark'
  accentColor: string
  showRecent: boolean
  clipboardMonitoring: boolean
  autoPaste: boolean
  clipboardRetentionDays: number
  syncEnabled: boolean
  syncDirectory: string
  syncIntervalMinutes: number
  autoCheckUpdates: boolean
  updateFeedUrl: string
  updatePublicKey: string
}

export interface SyncStatus {
  state: 'disabled' | 'idle' | 'syncing' | 'success' | 'error'
  message: string
  lastSyncedAt: number | null
}

export interface UpdateInfo {
  currentVersion: string
  latestVersion: string
  updateAvailable: boolean
  releaseUrl: string
  notes: string
}

export interface LegacyImportReport {
  sourcePaths: string[]
  importedSettings: boolean
  importedPins: number
  importedHistory: number
  importedShortcuts: number
  skippedDocuments: number
}

export interface BackupReport {
  path: string
  createdAt: number
  pluginCount: number
  pluginFileCount: number
  totalBytes: number
  archiveSha256: string
}

export interface RestoreReport {
  path: string
  restoredAt: number
  pluginCount: number
  pluginFileCount: number
}

export interface PluginFeature {
  code: string
  explain: string
  cmds: unknown[]
}

export interface InstalledPlugin {
  name: string
  title: string
  description: string
  version: string
  logoUrl: string
  features: PluginFeature[]
  compatibility: 'native-webview' | 'adapted' | 'needs-adaptation'
  compatibilityNotes: string[]
  development: boolean
}

export interface MarketPlugin {
  name: string
  title: string
  description: string
  version: string
  author: string
  categoryTitle: string
  downloadCount: number
  size: number
  updatedAt: number
  publishedAt: number
  logo: string
  homepage: string
  sourceLabel: string
}

export interface MarketInstallProgress {
  pluginName: string
  phase: 'resolving' | 'downloading' | 'verifying' | 'installing' | 'completed'
  receivedBytes: number
  totalBytes: number | null
}

export interface LauncherSnapshot {
  apps: AppEntry[]
  history: HistoryEntry[]
  clipboard: ClipboardEntry[]
  localShortcuts: LocalShortcut[]
  pinnedIds: string[]
  settings: LauncherSettings
  syncStatus: SyncStatus
}
