export interface PluginEnterAction {
  code: string
  type: 'text' | 'regex' | 'files'
  payload: unknown
}

export interface PluginDocument {
  _id: string
  [key: string]: unknown
}

export interface PluginFeature {
  code: string
  explain?: string
  cmds: unknown[]
  [key: string]: unknown
}

export interface PluginFileStats {
  name: string
  path: string
  isFile: boolean
  isDirectory: boolean
  size: number
  modifiedAt: number
  createdAt: number
}

export interface DialogOptions {
  title?: string
  defaultPath?: string
  defaultName?: string
  properties?: Array<'openFile' | 'openDirectory' | 'multiSelections'>
  filters?: Array<{ name: string; extensions: string[] }>
}

export interface ZToolsPluginApi {
  /** @returns 固定应用名 ZTools。 */
  getAppName(): string
  /** @returns 当前宿主语义化版本。 */
  getAppVersion(): string
  /** @returns 当前窗口类型 plugin。 */
  getWindowType(): 'plugin'
  /** @param name 能力标识。 @returns 当前宿主是否声明该能力。 */
  hasCapability(name: string): boolean
  /** @param callback 每次进入插件时调用的处理器。 @returns 无返回值。 */
  onPluginEnter(callback: (action: PluginEnterAction) => void | Promise<void>): void
  /** @param callback 插件窗口销毁前调用的处理器。 @returns 无返回值。 */
  onPluginOut(callback: () => void | Promise<void>): void
  /** @returns 等待已排队持久化操作后关闭插件的 Promise。 */
  outPlugin(): Promise<void>
  /** @param isRestorePreWindow 退出时是否恢复主启动器。 @returns 窗口隐藏完成后的 Promise。 */
  hideMainWindow(isRestorePreWindow?: boolean): Promise<void>
  /** @param text 要复制的文本。 @returns 已接受写入请求。 */
  copyText(text: string): boolean
  /** @param body 通知正文。 @returns 通知提交完成后的 Promise。 */
  showNotification(body: string): Promise<void>
  /** @param url HTTP 或 HTTPS 地址。 @returns 系统浏览器启动完成后的 Promise。 */
  shellOpenExternal(url: string): Promise<void>
  /** @param path 用户已授权路径。 @returns 系统默认程序启动完成后的 Promise。 */
  shellOpenPath(path: string): Promise<void>
  /** @param path 用户已授权路径。 @returns 文件管理器定位完成后的 Promise。 */
  shellShowItemInFolder(path: string): Promise<void>
  /** @returns 主显示器快照。 */
  getPrimaryDisplay(): Record<string, unknown> | null
  /** @returns 全部显示器快照。 */
  getAllDisplays(): Array<Record<string, unknown>>
  /** @returns 插件启动时的鼠标屏幕坐标。 */
  getCursorScreenPoint(): { x: number; y: number } | null
  /** @param feature 动态 feature。 @returns 同步镜像更新结果。 */
  setFeature(feature: PluginFeature): { ok: true; code: string }
  /** @param code 动态 feature 编码。 @returns 同步镜像删除结果。 */
  removeFeature(code: string): { ok: true; code: string }
  /** @param codes 可选的 feature 编码过滤器。 @returns 动态 feature 快照。 */
  getFeatures(codes?: string[]): PluginFeature[]
  db: {
    /** @param document JSON 文档。 @returns 同步镜像更新结果。 */
    put(document: PluginDocument): { ok: true; id: string }
    /** @param id 文档标识。 @returns 文档快照或空值。 */
    get(id: string): PluginDocument | null
    /** @param document 文档或文档标识。 @returns 同步镜像删除结果。 */
    remove(document: PluginDocument | string): { ok: true; id: string }
    /** @param prefix 可选文档标识前缀。 @returns 匹配文档快照。 */
    allDocs(prefix?: string): PluginDocument[]
    promises: {
      /** @param document JSON 文档。 @returns 持久化结果。 */
      put(document: PluginDocument): Promise<{ ok: true; id: string }>
      /** @param id 文档标识。 @returns 当前文档快照或空值。 */
      get(id: string): Promise<PluginDocument | null>
      /** @param document 文档或文档标识。 @returns 持久化删除结果。 */
      remove(document: PluginDocument | string): Promise<{ ok: true; id: string }>
      /** @param prefix 可选文档标识前缀。 @returns 匹配文档快照。 */
      allDocs(prefix?: string): Promise<PluginDocument[]>
    }
  }
  dbStorage: {
    /** @param key 存储键。 @returns JSON 值或空值。 */
    getItem(key: string): unknown
    /** @param key 存储键。 @param value JSON 值。 @returns 无返回值。 */
    setItem(key: string, value: unknown): void
    /** @param key 存储键。 @returns 无返回值。 */
    removeItem(key: string): void
  }
  clipboard: {
    /** @param text 文本内容。 @returns 写入完成后的 Promise。 */
    write(text: string): Promise<boolean>
    /** @param data 文本内容对象。 @param shouldPaste 是否写入后粘贴。 @returns 写入完成后的 Promise。 */
    writeContent(data: { type: 'text'; content: string }, shouldPaste?: boolean): Promise<boolean>
  }
  dialog: {
    /** @param options 打开对话框选项。 @returns 用户选择并已授权的路径。 */
    open(options?: DialogOptions): Promise<string[]>
    /** @param options 保存对话框选项。 @returns 保存路径，取消时为空。 */
    save(options?: DialogOptions): Promise<string | null>
  }
  file: {
    /** @param path 用户已授权路径。 @returns 路径是否存在。 */
    exists(path: string): Promise<boolean>
    /** @param path 用户已授权路径。 @returns 文件元数据。 */
    stat(path: string): Promise<PluginFileStats>
    /** @param path 用户已授权目录。 @returns 直接子项元数据。 */
    readDirectory(path: string): Promise<PluginFileStats[]>
    /** @param path 用户已授权文件。 @returns 文件字节。 */
    readFile(path: string): Promise<Uint8Array>
    /** @param path 用户已授权目标。 @param data 文件字节。 @returns 写入完成后的 Promise。 */
    writeFile(path: string, data: ArrayBuffer | ArrayBufferView | number[]): Promise<void>
    /** @param oldPath 原路径。 @param newPath 同目录目标路径。 @returns 重命名完成后的 Promise。 */
    rename(oldPath: string, newPath: string): Promise<void>
    /** @param sourcePath 源文件。 @param targetPath 目标文件。 @returns 复制完成后的 Promise。 */
    copy(sourcePath: string, targetPath: string): Promise<void>
    /** @param path 新目录路径。 @returns 创建完成后的 Promise。 */
    createDirectory(path: string): Promise<void>
  }
  screen: {
    /** @returns 主显示器快照。 */
    getPrimaryDisplay(): Record<string, unknown> | null
    /** @returns 全部显示器快照。 */
    getAllDisplays(): Array<Record<string, unknown>>
    /** @returns 插件启动时记录的鼠标屏幕坐标。 */
    getCursorScreenPoint(): { x: number; y: number } | null
    /** @returns 当前鼠标所在显示器的 PNG 字节。 */
    capture(): Promise<Uint8Array>
  }
  input: {
    /** @param content 要输入的文本。 @param targetExternal 是否先隐藏插件。 @returns 输入完成后的 Promise。 */
    typeText(content: string, targetExternal?: boolean): Promise<void>
    /** @param key 基础键名称。 @param targetExternal 是否先隐藏插件。 @returns 点击按键完成后的 Promise。 */
    tapKey(
      key:
        | 'Enter'
        | 'Escape'
        | 'Tab'
        | 'Backspace'
        | 'Delete'
        | 'ArrowUp'
        | 'ArrowDown'
        | 'ArrowLeft'
        | 'ArrowRight'
        | 'Home'
        | 'End'
        | 'PageUp'
        | 'PageDown',
      targetExternal?: boolean
    ): Promise<void>
    /** @param content 要复制并粘贴到原前台应用的文本。 @returns 粘贴完成后的 Promise。 */
    pasteText(content: string): Promise<void>
  }
  window: {
    /** @param width 客户区宽度。 @param height 客户区高度。 @returns 调整完成后的 Promise。 */
    setSize(width: number, height: number): Promise<void>
    /** @param x 屏幕横坐标。 @param y 屏幕纵坐标。 @returns 移动完成后的 Promise。 */
    setPosition(x: number, y: number): Promise<void>
    /** @returns 居中完成后的 Promise。 */
    center(): Promise<void>
    /** @param enabled 是否置顶。 @returns 更新完成后的 Promise。 */
    setAlwaysOnTop(enabled: boolean): Promise<void>
    /** @param minimized 是否最小化。 @returns 更新完成后的 Promise。 */
    setMinimized(minimized?: boolean): Promise<void>
    /** @param maximized 是否最大化。 @returns 更新完成后的 Promise。 */
    setMaximized(maximized?: boolean): Promise<void>
    /** @param fullscreen 是否全屏。 @returns 更新完成后的 Promise。 */
    setFullscreen(fullscreen?: boolean): Promise<void>
  }
}

declare global {
  interface Window {
    ztools: ZToolsPluginApi
  }
}

export {}
