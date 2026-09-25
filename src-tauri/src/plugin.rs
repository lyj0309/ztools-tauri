use std::{
    collections::HashMap,
    fs,
    io::{self, Cursor},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use base64::Engine;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{
    http, window::WindowBuilder, AppHandle, Emitter, LogicalPosition, LogicalSize, Manager,
    Monitor, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use walkdir::WalkDir;

use crate::state::AppState;

const MAX_PLUGIN_FILES: usize = 5_000;
const MAX_PLUGIN_BYTES: u64 = 100 * 1024 * 1024;
const MAX_MARKET_ARCHIVE_BYTES: usize = 50 * 1024 * 1024;
const MARKET_API_BASE: &str = "https://z.zosen.link/api/market";
type BundledPluginFile = (&'static str, &'static [u8]);
type BundledPlugin = (&'static str, &'static [BundledPluginFile]);
const BUNDLED_PLUGIN_FILES: &[BundledPlugin] = &[
    (
        "clipboard",
        &[
            (
                "plugin.json",
                include_bytes!("../resources/default-plugins/clipboard/plugin.json"),
            ),
            (
                "index.html",
                include_bytes!("../resources/default-plugins/clipboard/index.html"),
            ),
            (
                "preload.js",
                include_bytes!("../resources/default-plugins/clipboard/preload.js"),
            ),
            (
                "logo.png",
                include_bytes!("../resources/default-plugins/clipboard/logo.png"),
            ),
            (
                "README.md",
                include_bytes!("../resources/default-plugins/clipboard/README.md"),
            ),
            (
                "LICENSE.upstream",
                include_bytes!("../resources/default-plugins/clipboard/LICENSE.upstream"),
            ),
            (
                "assets/index-CgQaXsPA.css",
                include_bytes!("../resources/default-plugins/clipboard/assets/index-CgQaXsPA.css"),
            ),
            (
                "assets/index-D6-6Wwwu.js",
                include_bytes!("../resources/default-plugins/clipboard/assets/index-D6-6Wwwu.js"),
            ),
        ],
    ),
    (
        "translation-wy",
        &[
            (
                "plugin.json",
                include_bytes!("../resources/default-plugins/translation-wy/plugin.json"),
            ),
            (
                "index.html",
                include_bytes!("../resources/default-plugins/translation-wy/index.html"),
            ),
            (
                "logo.png",
                include_bytes!("../resources/default-plugins/translation-wy/logo.png"),
            ),
            (
                "LICENSE.upstream",
                include_bytes!("../resources/default-plugins/translation-wy/LICENSE.upstream"),
            ),
        ],
    ),
    (
        "break-reminder",
        &[
            (
                "plugin.json",
                include_bytes!("../resources/default-plugins/break-reminder/plugin.json"),
            ),
            (
                "index.html",
                include_bytes!("../resources/default-plugins/break-reminder/index.html"),
            ),
            (
                "reminder.css",
                include_bytes!("../resources/default-plugins/break-reminder/reminder.css"),
            ),
            (
                "reminder.js",
                include_bytes!("../resources/default-plugins/break-reminder/reminder.js"),
            ),
        ],
    ),
    (
        "baidu-translate",
        &[
            (
                "plugin.json",
                include_bytes!("../resources/default-plugins/baidu-translate/plugin.json"),
            ),
            (
                "index.html",
                include_bytes!("../resources/default-plugins/baidu-translate/index.html"),
            ),
        ],
    ),
    (
        "setting",
        &[
            (
                "plugin.json",
                include_bytes!("../resources/default-plugins/setting/plugin.json"),
            ),
            (
                "index.html",
                include_bytes!("../resources/default-plugins/setting/index.html"),
            ),
        ],
    ),
    (
        "system",
        &[
            (
                "plugin.json",
                include_bytes!("../resources/default-plugins/system/plugin.json"),
            ),
            (
                "index.html",
                include_bytes!("../resources/default-plugins/system/index.html"),
            ),
        ],
    ),
    (
        "screenshot",
        &[
            (
                "plugin.json",
                include_bytes!("../resources/default-plugins/screenshot/plugin.json"),
            ),
            (
                "logo.png",
                include_bytes!("../resources/default-plugins/screenshot/logo.png"),
            ),
            (
                "index.html",
                include_bytes!("../resources/default-plugins/screenshot/index.html"),
            ),
            (
                "editor.html",
                include_bytes!("../resources/default-plugins/screenshot/editor.html"),
            ),
            (
                "editor-adapter.js",
                include_bytes!("../resources/default-plugins/screenshot/editor-adapter.js"),
            ),
            (
                "editor-adapter.css",
                include_bytes!("../resources/default-plugins/screenshot/editor-adapter.css"),
            ),
            (
                "assets/index-CRZJr_0k.js",
                include_bytes!("../resources/default-plugins/screenshot/assets/index-CRZJr_0k.js"),
            ),
            (
                "assets/index-NfvmJyAk.css",
                include_bytes!("../resources/default-plugins/screenshot/assets/index-NfvmJyAk.css"),
            ),
            (
                "LICENSE.upstream",
                include_bytes!("../resources/default-plugins/screenshot/LICENSE.upstream"),
            ),
            (
                "README.upstream.md",
                include_bytes!("../resources/default-plugins/screenshot/README.upstream.md"),
            ),
            (
                "screenshot.js",
                include_bytes!("../resources/default-plugins/screenshot/screenshot.js"),
            ),
            (
                "screenshot.css",
                include_bytes!("../resources/default-plugins/screenshot/screenshot.css"),
            ),
            (
                "pin.html",
                include_bytes!("../resources/default-plugins/screenshot/pin.html"),
            ),
            (
                "pin.js",
                include_bytes!("../resources/default-plugins/screenshot/pin.js"),
            ),
            (
                "pin.css",
                include_bytes!("../resources/default-plugins/screenshot/pin.css"),
            ),
            (
                "history.html",
                include_bytes!("../resources/default-plugins/screenshot/history.html"),
            ),
            (
                "history.js",
                include_bytes!("../resources/default-plugins/screenshot/history.js"),
            ),
            (
                "history.css",
                include_bytes!("../resources/default-plugins/screenshot/history.css"),
            ),
        ],
    ),
];

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginManifest {
    pub(crate) name: String,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) description: String,
    pub(crate) version: String,
    pub(crate) main: String,
    #[serde(default)]
    pub(crate) logo: String,
    #[serde(default)]
    pub(crate) preload: String,
    #[serde(default)]
    pub(crate) features: Vec<PluginFeature>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginFeature {
    pub(crate) code: String,
    #[serde(default)]
    pub(crate) explain: String,
    #[serde(default)]
    pub(crate) cmds: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledPlugin {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) version: String,
    pub(crate) logo_url: String,
    pub(crate) features: Vec<PluginFeature>,
    pub(crate) compatibility: String,
    pub(crate) compatibility_notes: Vec<String>,
    pub(crate) development: bool,
    pub(crate) built_in: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MarketPlugin {
    pub(crate) name: String,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) description: String,
    pub(crate) version: String,
    #[serde(default)]
    pub(crate) author: String,
    #[serde(default)]
    pub(crate) category_title: String,
    #[serde(default)]
    pub(crate) download_count: u64,
    #[serde(default)]
    pub(crate) size: u64,
    #[serde(default)]
    pub(crate) updated_at: u64,
    #[serde(default)]
    pub(crate) published_at: u64,
    #[serde(default)]
    pub(crate) logo: String,
    #[serde(default)]
    pub(crate) homepage: String,
    #[serde(default)]
    pub(crate) source_label: String,
}

#[derive(Debug, Deserialize)]
struct MarketCategory {
    #[serde(default)]
    plugins: Vec<MarketPlugin>,
}

#[derive(Debug, Deserialize)]
struct MarketCatalog {
    #[serde(default)]
    categories: Vec<MarketCategory>,
    #[serde(default)]
    latest: Vec<MarketPlugin>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketDownload {
    #[serde(default)]
    download_url: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MarketInstallProgress {
    plugin_name: String,
    phase: String,
    received_bytes: u64,
    total_bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MarketInstallReceipt<'a> {
    schema_version: u8,
    source: &'a str,
    plugin_name: &'a str,
    plugin_version: &'a str,
    archive_sha256: String,
    downloaded_at: u128,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginEnterAction {
    pub(crate) code: String,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) payload: serde_json::Value,
}

pub(crate) struct PluginRuntime {
    root: PathBuf,
    instances: Mutex<HashMap<String, String>>,
    path_grants: Mutex<HashMap<String, Vec<PathBuf>>>,
    restore_main_on_close: Mutex<HashMap<String, bool>>,
    market_installs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    development_watchers: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl PluginRuntime {
    /// 创建使用指定插件根目录的运行时，并确保目录已经存在。
    pub(crate) fn new(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(|error| format!("无法创建插件目录：{error}"))?;
        Ok(Self {
            root,
            instances: Mutex::new(HashMap::new()),
            path_grants: Mutex::new(HashMap::new()),
            restore_main_on_close: Mutex::new(HashMap::new()),
            market_installs: Mutex::new(HashMap::new()),
            development_watchers: Mutex::new(HashMap::new()),
        })
    }

    /// 把编译进可执行文件的默认插件发布到用户插件目录，并修复缺失或损坏的内置文件。
    pub(crate) fn ensure_bundled_plugins(&self) -> Result<(), String> {
        for (plugin_name, files) in BUNDLED_PLUGIN_FILES {
            let directory = self.root.join(plugin_name);
            fs::create_dir_all(&directory)
                .map_err(|error| format!("无法创建内置插件 {plugin_name}：{error}"))?;

            // 每次启动都对照内嵌资源，确保升级和误删后能恢复到当前宿主版本。
            for (relative, contents) in *files {
                write_bundled_file(&directory.join(relative), contents)?;
            }
            let manifest = read_manifest(&directory)?;
            if manifest.name != *plugin_name {
                return Err(format!("内置插件 {plugin_name} 的 manifest 名称不一致"));
            }
        }
        Ok(())
    }

    /// 注册一项市场安装，并返回供下载循环检查的取消标记。
    fn begin_market_install(&self, plugin_name: &str) -> Result<Arc<AtomicBool>, String> {
        let mut installs = self
            .market_installs
            .lock()
            .map_err(|_| "市场安装状态锁已损坏".to_owned())?;
        if installs.contains_key(plugin_name) {
            return Err("该插件已经在安装中".to_owned());
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        installs.insert(plugin_name.to_owned(), cancelled.clone());
        Ok(cancelled)
    }

    /// 标记指定市场安装已取消；未在安装时返回 false。
    pub(crate) fn cancel_market_install(&self, plugin_name: &str) -> Result<bool, String> {
        let installs = self
            .market_installs
            .lock()
            .map_err(|_| "市场安装状态锁已损坏".to_owned())?;
        Ok(installs.get(plugin_name).is_some_and(|cancelled| {
            cancelled.store(true, Ordering::Relaxed);
            true
        }))
    }

    /// 清理安装状态，使成功、失败或取消后都可以再次安装。
    fn finish_market_install(&self, plugin_name: &str) {
        if let Ok(mut installs) = self.market_installs.lock() {
            installs.remove(plugin_name);
        }
    }

    /// 注册开发目录并启动轮询监听，文件稳定变化后同步到隔离安装副本。
    pub(crate) fn register_development_directory(
        &self,
        app: &AppHandle,
        source: &Path,
    ) -> Result<InstalledPlugin, String> {
        let source = source
            .canonicalize()
            .map_err(|error| format!("插件开发目录不存在：{error}"))?;
        let manifest = read_manifest(&source)?;
        let installed = self.install_from_directory(&source)?;
        let stop = Arc::new(AtomicBool::new(false));
        let mut watchers = self
            .development_watchers
            .lock()
            .map_err(|_| "插件开发监听锁已损坏".to_owned())?;
        if let Some(previous) = watchers.insert(manifest.name.clone(), stop.clone()) {
            previous.store(true, Ordering::Relaxed);
        }
        drop(watchers);

        let app = app.clone();
        let plugin_name = manifest.name;
        std::thread::Builder::new()
            .name(format!("ztools-plugin-dev-{plugin_name}"))
            .spawn(move || watch_development_directory(app, plugin_name, source, stop))
            .map_err(|error| format!("无法启动插件开发监听：{error}"))?;
        Ok(InstalledPlugin {
            development: true,
            ..installed
        })
    }

    /// 停止指定插件的开发目录监听，保留最后一次同步的安装副本。
    pub(crate) fn stop_development_watch(&self, plugin_name: &str) -> Result<bool, String> {
        let removed = self
            .development_watchers
            .lock()
            .map_err(|_| "插件开发监听锁已损坏".to_owned())?
            .remove(plugin_name);
        if let Some(stop) = removed {
            stop.store(true, Ordering::Relaxed);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 停止全部开发监听线程，供桌面事件循环退出时释放后台资源。
    pub(crate) fn stop_all_development_watches(&self) {
        if let Ok(mut watchers) = self.development_watchers.lock() {
            for (_, stop) in watchers.drain() {
                stop.store(true, Ordering::Relaxed);
            }
        }
    }

    /// 判断插件当前是否由开发目录监听器维护。
    fn is_development_plugin(&self, plugin_name: &str) -> bool {
        self.development_watchers
            .lock()
            .is_ok_and(|watchers| watchers.contains_key(plugin_name))
    }

    /// 返回当前插件根目录，供协议处理器和安装器使用。
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// 扫描插件目录并返回所有通过 manifest 校验的插件。
    pub(crate) fn installed_plugins(&self) -> Result<Vec<InstalledPlugin>, String> {
        let mut plugins = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                continue;
            }
            let directory = entry.path();
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            match read_manifest(&directory) {
                Ok(manifest) => {
                    let mut summary = plugin_summary(&manifest, &directory);
                    summary.development = self.is_development_plugin(&manifest.name);
                    plugins.push(summary);
                }
                Err(error) => eprintln!("[plugin] skip invalid {}: {error}", directory.display()),
            }
        }
        plugins.sort_by(|left, right| left.title.cmp(&right.title));
        Ok(plugins)
    }

    /// 将本地插件目录安全复制到 staging，再原子替换正式版本。
    pub(crate) fn install_from_directory(&self, source: &Path) -> Result<InstalledPlugin, String> {
        let source = source
            .canonicalize()
            .map_err(|error| format!("插件目录不存在：{error}"))?;
        if !source.is_dir() {
            return Err("当前安装器只接受已经解压的插件目录".to_owned());
        }
        let manifest = read_manifest(&source)?;
        if is_bundled_plugin(&manifest.name) {
            return Err("内置插件由 ZTools 随包维护，不能从本地或市场覆盖".to_owned());
        }
        let nonce = now_millis();
        let staging = self
            .root
            .join(format!(".staging-{}-{nonce}", manifest.name));
        let target = self.root.join(&manifest.name);
        let backup = self.root.join(format!(".backup-{}-{nonce}", manifest.name));

        // 安装前清理同名临时目录，避免上次异常退出污染本次事务。
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
        }
        copy_plugin_tree(&source, &staging)?;
        let staged_manifest = match read_manifest(&staging) {
            Ok(manifest) => manifest,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
        };
        if staged_manifest.name != manifest.name || staged_manifest.version != manifest.version {
            let _ = fs::remove_dir_all(&staging);
            return Err("插件复制后的 manifest 与源文件不一致".to_owned());
        }

        // 升级时先保留旧目录，只有新目录发布成功后才清除备份。
        if target.exists() {
            fs::rename(&target, &backup).map_err(|error| format!("无法暂存旧插件：{error}"))?;
        }
        if let Err(error) = fs::rename(&staging, &target) {
            if backup.exists() {
                let _ = fs::rename(&backup, &target);
            }
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("无法发布插件：{error}"));
        }
        if backup.exists() {
            // 新版本已经发布，旧备份清理失败只记录告警，不能把成功升级误报为失败。
            if let Err(error) = fs::remove_dir_all(&backup) {
                eprintln!(
                    "[plugin] unable to remove backup {}: {error}",
                    backup.display()
                );
            }
        }
        Ok(plugin_summary(&staged_manifest, &target))
    }

    /// 删除指定插件目录，拒绝不符合 manifest 名称规则的路径输入。
    pub(crate) fn uninstall(&self, plugin_name: &str) -> Result<(), String> {
        validate_plugin_name(plugin_name)?;
        if is_bundled_plugin(plugin_name) {
            return Err("内置插件不能卸载".to_owned());
        }
        let target = self.root.join(plugin_name);
        if !target.is_dir() {
            return Err("插件不存在".to_owned());
        }
        fs::remove_dir_all(target).map_err(|error| format!("卸载插件失败：{error}"))?;
        let webview_data = self.root.join(".webview-data").join(plugin_name);
        if webview_data.exists() {
            fs::remove_dir_all(webview_data)
                .map_err(|error| format!("插件已卸载，但清理 Webview 数据失败：{error}"))?;
        }
        Ok(())
    }

    /// 记录窗口和插件身份映射，使插件 API 能验证真实调用来源。
    fn register_instance(&self, label: &str, plugin_name: &str) -> Result<(), String> {
        self.instances
            .lock()
            .map_err(|_| "插件实例锁已损坏".to_owned())?
            .insert(label.to_owned(), plugin_name.to_owned());
        self.path_grants
            .lock()
            .map_err(|_| "插件路径授权锁已损坏".to_owned())?
            .entry(label.to_owned())
            .or_default();
        self.restore_main_on_close
            .lock()
            .map_err(|_| "插件窗口恢复状态锁已损坏".to_owned())?
            .insert(label.to_owned(), true);
        Ok(())
    }

    /// 移除已经销毁的插件窗口身份，防止旧窗口标签继续获得权限。
    pub(crate) fn unregister_instance(&self, label: &str) {
        if let Ok(mut instances) = self.instances.lock() {
            instances.remove(label);
        }
        if let Ok(mut grants) = self.path_grants.lock() {
            grants.remove(label);
        }
        if let Ok(mut restore) = self.restore_main_on_close.lock() {
            restore.remove(label);
        }
    }

    /// 校验调用窗口是已注册插件实例，并返回它的插件名称。
    pub(crate) fn plugin_for_window(&self, label: &str) -> Result<String, String> {
        self.instances
            .lock()
            .map_err(|_| "插件实例锁已损坏".to_owned())?
            .get(label)
            .cloned()
            .ok_or_else(|| "当前窗口没有插件 API 权限".to_owned())
    }

    /// 设置插件销毁后是否恢复主启动器，支持打开外部目标后保持宿主隐藏。
    pub(crate) fn set_restore_main_on_close(
        &self,
        label: &str,
        restore: bool,
    ) -> Result<(), String> {
        self.plugin_for_window(label)?;
        self.restore_main_on_close
            .lock()
            .map_err(|_| "插件窗口恢复状态锁已损坏".to_owned())?
            .insert(label.to_owned(), restore);
        Ok(())
    }

    /// 读取插件销毁时的主窗口恢复策略，未设置时采用恢复主窗口的安全默认值。
    fn restore_main_after_close(&self, label: &str) -> bool {
        self.restore_main_on_close
            .lock()
            .ok()
            .and_then(|restore| restore.get(label).copied())
            .unwrap_or(true)
    }

    /// 把用户明确选择或拖入的现有路径加入指定插件窗口的临时授权集。
    pub(crate) fn grant_paths<I>(&self, label: &str, paths: I) -> Result<Vec<PathBuf>, String>
    where
        I: IntoIterator<Item = PathBuf>,
    {
        self.plugin_for_window(label)?;
        let mut accepted = Vec::new();
        for path in paths {
            let canonical = path
                .canonicalize()
                .map_err(|error| format!("授权路径不存在：{error}"))?;
            if !accepted.contains(&canonical) {
                accepted.push(canonical);
            }
        }
        let mut grants = self
            .path_grants
            .lock()
            .map_err(|_| "插件路径授权锁已损坏".to_owned())?;
        let current = grants.entry(label.to_owned()).or_default();
        for path in &accepted {
            if !current.contains(path) {
                current.push(path.clone());
            }
        }
        Ok(accepted)
    }

    /// 解析并校验插件传入的现有路径是否位于用户授权范围内。
    pub(crate) fn authorized_existing_path(
        &self,
        label: &str,
        candidate: &Path,
    ) -> Result<PathBuf, String> {
        let plugin_name = self.plugin_for_window(label)?;
        let canonical = candidate
            .canonicalize()
            .map_err(|error| format!("文件路径不存在：{error}"))?;
        self.ensure_path_authorized(label, &plugin_name, &canonical)?;
        Ok(canonical)
    }

    /// 校验尚未创建的目标路径，其父目录必须来自用户授权的重命名范围。
    pub(crate) fn authorized_new_path(
        &self,
        label: &str,
        candidate: &Path,
    ) -> Result<PathBuf, String> {
        let plugin_name = self.plugin_for_window(label)?;
        let file_name = candidate
            .file_name()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "目标路径缺少文件名".to_owned())?;
        let parent = candidate
            .parent()
            .ok_or_else(|| "目标路径缺少父目录".to_owned())?
            .canonicalize()
            .map_err(|error| format!("目标父目录不存在：{error}"))?;
        let resolved = parent.join(file_name);
        self.ensure_path_authorized(label, &plugin_name, &resolved)?;
        Ok(resolved)
    }

    /// 检查规范路径是否属于插件私有目录、已授权目录或已授权文件的同级目录。
    fn ensure_path_authorized(
        &self,
        label: &str,
        plugin_name: &str,
        candidate: &Path,
    ) -> Result<(), String> {
        let private_root = self
            .root
            .join(plugin_name)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if candidate.starts_with(&private_root) {
            return Ok(());
        }
        let grants = self
            .path_grants
            .lock()
            .map_err(|_| "插件路径授权锁已损坏".to_owned())?;
        let allowed = grants.get(label).is_some_and(|paths| {
            paths.iter().any(|granted| {
                (granted.is_dir() && candidate.starts_with(granted))
                    || candidate == granted
                    || (granted.is_file() && candidate.parent() == granted.parent())
            })
        });
        if allowed {
            Ok(())
        } else {
            Err("插件没有访问该路径的权限，请先由启动器拖入或通过选择框授权".to_owned())
        }
    }
}

/// 返回正式插件目录；测试模式可通过隔离环境变量覆盖。
pub(crate) fn plugin_root() -> PathBuf {
    if std::env::var("ZTOOLS_E2E").as_deref() == Ok("1") {
        if let Some(root) = std::env::var_os("ZTOOLS_PLUGIN_ROOT") {
            return PathBuf::from(root);
        }
    }
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("top.ztools.launcher")
        .join("plugins")
}

/// 判断名称是否属于随可执行文件分发且由宿主维护的默认插件。
pub(crate) fn is_bundled_plugin(plugin_name: &str) -> bool {
    BUNDLED_PLUGIN_FILES
        .iter()
        .any(|(name, _)| *name == plugin_name)
}

/// 校验一个插件根目录中的全部正式插件并返回有效插件数量。
pub(crate) fn validate_plugin_root(root: &Path) -> Result<usize, String> {
    if !root.exists() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            return Err(format!(
                "插件根目录包含未知文件：{}",
                entry.path().display()
            ));
        }
        let manifest = read_manifest(&entry.path())?;
        if entry.file_name().to_string_lossy() != manifest.name {
            return Err(format!(
                "插件目录名 {} 与 manifest 名称 {} 不一致",
                entry.file_name().to_string_lossy(),
                manifest.name
            ));
        }
        count += 1;
    }
    Ok(count)
}

/// 从官方插件市场匿名读取当前平台目录并去除分类间的重复插件。
pub(crate) async fn fetch_market_plugins() -> Result<Vec<MarketPlugin>, String> {
    let platform = match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        value => value,
    };
    let url = format!("{MARKET_API_BASE}/plugins?limit=200&platform={platform}");
    let response = market_client()?
        .get(url)
        .send()
        .await
        .map_err(|error| format!("无法连接插件市场：{error}"))?
        .error_for_status()
        .map_err(|error| format!("插件市场返回错误：{error}"))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("无法读取插件市场响应：{error}"))?;
    if bytes.len() > 5 * 1024 * 1024 {
        return Err("插件市场目录超过 5 MB 限制".to_owned());
    }
    let catalog: MarketCatalog =
        serde_json::from_slice(&bytes).map_err(|error| format!("插件市场数据无效：{error}"))?;
    let mut unique = HashMap::new();
    for plugin in catalog
        .categories
        .into_iter()
        .flat_map(|category| category.plugins)
        .chain(catalog.latest)
    {
        if validate_plugin_name(&plugin.name).is_ok()
            && semver::Version::parse(&plugin.version).is_ok()
        {
            unique.entry(plugin.name.clone()).or_insert(plugin);
        }
    }
    let mut plugins: Vec<_> = unique.into_values().collect();
    plugins.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.title.cmp(&right.title))
    });
    Ok(plugins)
}

/// 从官方市场下载 ZIP、在隔离目录安全解压并通过安装事务发布插件。
pub(crate) async fn install_market_plugin(
    app: &AppHandle,
    runtime: &PluginRuntime,
    plugin_name: &str,
) -> Result<InstalledPlugin, String> {
    validate_plugin_name(plugin_name)?;
    let cancelled = runtime.begin_market_install(plugin_name)?;
    let result = install_market_plugin_inner(app, runtime, plugin_name, &cancelled).await;
    runtime.finish_market_install(plugin_name);
    result
}

/// 执行市场安装事务，并在下载、校验、安装和完成阶段发布状态。
async fn install_market_plugin_inner(
    app: &AppHandle,
    runtime: &PluginRuntime,
    plugin_name: &str,
    cancelled: &AtomicBool,
) -> Result<InstalledPlugin, String> {
    emit_market_install_progress(app, plugin_name, "resolving", 0, None);
    let endpoint = format!("{MARKET_API_BASE}/plugins/download?name={plugin_name}");
    let download: MarketDownload = market_client()?
        .get(endpoint)
        .send()
        .await
        .map_err(|error| format!("无法解析插件下载地址：{error}"))?
        .error_for_status()
        .map_err(|error| format!("插件下载地址返回错误：{error}"))?
        .json()
        .await
        .map_err(|error| format!("插件下载地址数据无效：{error}"))?;
    validate_market_download_url(&download.download_url)?;
    if cancelled.load(Ordering::Relaxed) {
        return Err("插件安装已取消".to_owned());
    }
    let response = market_client()?
        .get(&download.download_url)
        .send()
        .await
        .map_err(|error| format!("插件下载失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("插件下载返回错误：{error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MARKET_ARCHIVE_BYTES as u64)
    {
        return Err("插件下载包超过 50 MB 限制".to_owned());
    }
    let total_bytes = response.content_length();
    let mut bytes = Vec::with_capacity(
        total_bytes
            .unwrap_or_default()
            .min(MAX_MARKET_ARCHIVE_BYTES as u64) as usize,
    );
    let mut stream = response.bytes_stream();
    emit_market_install_progress(app, plugin_name, "downloading", 0, total_bytes);
    while let Some(chunk) = stream.next().await {
        if cancelled.load(Ordering::Relaxed) {
            return Err("插件安装已取消".to_owned());
        }
        let chunk = chunk.map_err(|error| format!("无法读取插件下载包：{error}"))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_MARKET_ARCHIVE_BYTES {
            return Err("插件下载包超过 50 MB 限制".to_owned());
        }
        bytes.extend_from_slice(&chunk);
        emit_market_install_progress(
            app,
            plugin_name,
            "downloading",
            bytes.len() as u64,
            total_bytes,
        );
    }
    emit_market_install_progress(
        app,
        plugin_name,
        "verifying",
        bytes.len() as u64,
        total_bytes,
    );
    let archive_sha256 = format!("{:x}", Sha256::digest(&bytes));

    // 下载内容先进入插件根目录下的短期工作区，任何失败都不会覆盖已安装版本。
    let extracted = runtime
        .root()
        .join(format!(".market-{plugin_name}-{}", now_millis()));
    let operation = (|| {
        extract_plugin_zip(&bytes, &extracted)?;
        let source = locate_extracted_plugin(&extracted)?;
        let manifest = read_manifest(&source)?;
        if manifest.name != plugin_name {
            return Err("下载包 manifest 名称与市场插件不一致".to_owned());
        }
        let receipt = MarketInstallReceipt {
            schema_version: 1,
            source: "official-market",
            plugin_name,
            plugin_version: &manifest.version,
            archive_sha256,
            downloaded_at: now_millis(),
        };
        let receipt_bytes =
            serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?;
        fs::write(source.join(".ztools-install.json"), receipt_bytes)
            .map_err(|error| format!("无法写入插件安装凭据：{error}"))?;
        if cancelled.load(Ordering::Relaxed) {
            return Err("插件安装已取消".to_owned());
        }
        emit_market_install_progress(
            app,
            plugin_name,
            "installing",
            bytes.len() as u64,
            total_bytes,
        );
        close_plugin_window(app, plugin_name)?;
        runtime.install_from_directory(&source)
    })();
    let _ = fs::remove_dir_all(&extracted);
    if operation.is_ok() {
        emit_market_install_progress(
            app,
            plugin_name,
            "completed",
            bytes.len() as u64,
            total_bytes,
        );
    }
    operation
}

/// 向主窗口发布市场安装阶段和字节进度，窗口关闭时忽略事件投递失败。
fn emit_market_install_progress(
    app: &AppHandle,
    plugin_name: &str,
    phase: &str,
    received_bytes: u64,
    total_bytes: Option<u64>,
) {
    let _ = app.emit(
        "plugin-market-install-progress",
        MarketInstallProgress {
            plugin_name: plugin_name.to_owned(),
            phase: phase.to_owned(),
            received_bytes,
            total_bytes,
        },
    );
}

/// 轮询开发目录指纹，在一次稳定间隔后同步插件并刷新活动 Webview。
fn watch_development_directory(
    app: AppHandle,
    plugin_name: String,
    source: PathBuf,
    stop: Arc<AtomicBool>,
) {
    let mut fingerprint = directory_fingerprint(&source).ok();
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let Ok(next) = directory_fingerprint(&source) else {
            let _ = app.emit(
                "plugin-development-status",
                serde_json::json!({
                    "pluginName": plugin_name,
                    "state": "error",
                    "message": "插件开发目录暂时不可读"
                }),
            );
            continue;
        };
        if fingerprint.as_ref() == Some(&next) {
            continue;
        }

        // 等待构建工具完成一批写入，避免同步到半写入目录。
        std::thread::sleep(std::time::Duration::from_millis(300));
        let Ok(stable) = directory_fingerprint(&source) else {
            continue;
        };
        if stable != next {
            fingerprint = Some(stable);
            continue;
        }
        let runtime = app.state::<PluginRuntime>();
        match runtime.install_from_directory(&source) {
            Ok(_) => {
                fingerprint = Some(stable);
                if let Some(webview) = app.get_webview(&plugin_window_label(&plugin_name)) {
                    let _ = webview.eval("globalThis.location.reload()");
                }
                let _ = app.emit(
                    "plugin-development-status",
                    serde_json::json!({
                        "pluginName": plugin_name,
                        "state": "reloaded",
                        "message": "开发目录已同步并刷新插件"
                    }),
                );
            }
            Err(error) => {
                let _ = app.emit(
                    "plugin-development-status",
                    serde_json::json!({
                        "pluginName": plugin_name,
                        "state": "error",
                        "message": error
                    }),
                );
            }
        }
    }
}

/// 计算开发目录的稳定元数据指纹，不读取用户文件正文。
fn directory_fingerprint(directory: &Path) -> Result<[u8; 32], String> {
    let mut entries = WalkDir::new(directory)
        .follow_links(false)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by(|left, right| left.path().cmp(right.path()));
    let mut digest = Sha256::new();
    for entry in entries {
        if entry.path() == directory {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(directory)
            .map_err(|error| error.to_string())?;
        digest.update(relative.to_string_lossy().as_bytes());
        let metadata = entry.metadata().map_err(|error| error.to_string())?;
        digest.update(metadata.len().to_le_bytes());
        let modified = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        digest.update(modified.to_le_bytes());
    }
    Ok(digest.finalize().into())
}

/// 创建带固定超时和应用标识的市场 HTTP 客户端。
fn market_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("ZTools-Tauri/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| error.to_string())
}

/// 只允许官方 HTTPS CDN 作为市场下载源，阻止目录响应触发任意地址请求。
fn validate_market_download_url(value: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(value).map_err(|_| "插件下载地址格式无效".to_owned())?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "https"
        || !(host == "zosen.link" || host.ends_with(".zosen.link"))
        || url.username() != ""
        || url.password().is_some()
    {
        return Err("插件下载地址不属于受信任的官方 HTTPS 域名".to_owned());
    }
    Ok(())
}

/// 安全解压市场 ZIP，限制路径、符号链接、文件数量和解压后体积。
fn extract_plugin_zip(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("插件包不是有效 ZIP：{error}"))?;
    if archive.len() > MAX_PLUGIN_FILES {
        return Err("插件包超过 5000 个文件限制".to_owned());
    }
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    let mut total_bytes = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("无法读取 ZIP 条目：{error}"))?;
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| "插件 ZIP 包含越界路径".to_owned())?
            .to_owned();
        validate_relative_path(&relative)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            // npm 的 .bin 链接只用于命令行构建；插件运行不需要，跳过且不在磁盘创建链接。
            if relative.parent() == Some(Path::new("preload/node_modules/.bin")) {
                continue;
            }
            return Err("插件 ZIP 不能包含符号链接".to_owned());
        }
        total_bytes = total_bytes.saturating_add(entry.size());
        if total_bytes > MAX_PLUGIN_BYTES {
            return Err("插件解压后超过 100 MB 限制".to_owned());
        }
        let output = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(output).map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut file = fs::File::create(output).map_err(|error| error.to_string())?;
        io::copy(&mut entry, &mut file).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// 找到 ZIP 根目录或唯一一级子目录中的 plugin.json。
fn locate_extracted_plugin(extracted: &Path) -> Result<PathBuf, String> {
    if extracted.join("plugin.json").is_file() {
        return Ok(extracted.to_path_buf());
    }
    let candidates: Vec<_> = fs::read_dir(extracted)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("plugin.json").is_file())
        .collect();
    if candidates.len() != 1 {
        return Err("插件 ZIP 根目录中找不到唯一 plugin.json".to_owned());
    }
    Ok(candidates[0].clone())
}

/**
 * 创建或复用插件窗口，并向插件派发本次进入动作。
 * @param app 桌面应用句柄。
 * @param runtime 插件运行时与隔离数据目录。
 * @param plugin_name 已安装插件名称。
 * @param action 本次进入的功能与数据。
 * @returns 插件窗口显示完成，失败时返回原因。
 */
pub(crate) fn launch_plugin(
    app: &AppHandle,
    runtime: &PluginRuntime,
    plugin_name: &str,
    action: PluginEnterAction,
) -> Result<(), String> {
    validate_plugin_name(plugin_name)?;
    let plugin_directory = runtime.root().join(plugin_name);
    let manifest = read_manifest(&plugin_directory)?;
    // 百度页面由网站自身处理匿名请求，不向外部网页注入本地插件桥。
    if plugin_name == "baidu-translate" {
        return launch_baidu_site(app, runtime, &manifest, &action);
    }
    if plugin_name == "translation-wy" {
        return launch_youdao_site(app, runtime, &manifest, &action);
    }
    let preload_adapter = preload_adapter_script(&manifest, &plugin_directory)?;
    let label = plugin_window_label(plugin_name);
    let action_json = serde_json::to_string(&action).map_err(|error| error.to_string())?;
    let restore_main_on_close = !(plugin_name == "break-reminder" && action.code == "break");
    let embed_in_main = restore_main_on_close
        && !(std::env::var("ZTOOLS_E2E").as_deref() == Ok("1")
            && std::env::var_os("ZTOOLS_E2E_PLUGIN_NAME").is_some()
            && std::env::var_os("ZTOOLS_E2E_EMBED_PLUGIN").is_none());

    // 普通插件优先复用主窗口搜索框下方的子 Webview，避免重复创建弹窗。
    if let Some(webview) = app.get_webview(&label) {
        if webview.window().label() == "main" {
            if !embed_in_main {
                // 同一插件从设置页进入定时提醒时，先释放嵌入视图的单例标签。
                close_plugin_window(app, plugin_name)?;
            } else {
                runtime.set_restore_main_on_close(&label, true)?;
                let payload_paths = plugin_payload_paths(&action.payload);
                if !payload_paths.is_empty() {
                    runtime.grant_paths(&label, payload_paths)?;
                }
                webview
                    .eval(format!("window.__ztoolsDispatchEnter?.({action_json})"))
                    .map_err(|error| error.to_string())?;
                show_embedded_plugin(app, &webview, &manifest.title)?;
                return Ok(());
            }
        } else if webview.window().label().starts_with("detached-") {
            // 分离视图仍是同一个插件实例；再次启动时直接派发进入动作并聚焦它。
            let payload_paths = plugin_payload_paths(&action.payload);
            if !payload_paths.is_empty() {
                runtime.grant_paths(&label, payload_paths)?;
            }
            webview
                .eval(format!("window.__ztoolsDispatchEnter?.({action_json})"))
                .map_err(|error| error.to_string())?;
            let detached = webview.window();
            detached.show().map_err(|error| error.to_string())?;
            detached.set_focus().map_err(|error| error.to_string())?;
            return Ok(());
        }
    }

    if let Some(window) = app.get_webview_window(&label) {
        if embed_in_main {
            // 关闭旧独立视图后再创建子视图，避免设置页沿用提醒弹窗。
            window.close().map_err(|error| error.to_string())?;
            runtime.unregister_instance(&label);
        } else {
            // 复用单例窗口时先派发新的进入动作，再恢复窗口焦点。
            runtime.set_restore_main_on_close(&label, restore_main_on_close)?;
            let payload_paths = plugin_payload_paths(&action.payload);
            if !payload_paths.is_empty() {
                runtime.grant_paths(&label, payload_paths)?;
            }
            window
                .eval(format!("window.__ztoolsDispatchEnter?.({action_json})"))
                .map_err(|error| error.to_string())?;
            hide_main_window(app)?;
            window.show().map_err(|error| error.to_string())?;
            window.set_focus().map_err(|error| error.to_string())?;
            return Ok(());
        }
    }

    let main_url = format!(
        "ztools-plugin://localhost/{}/{}",
        manifest.name,
        encode_relative_url_path(&manifest.main)
    );
    let url =
        tauri::Url::parse(&main_url).map_err(|error| format!("插件入口 URL 无效：{error}"))?;
    let documents = app
        .state::<AppState>()
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .plugin_documents(plugin_name)?;
    let storage = app
        .state::<AppState>()
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .plugin_storage(plugin_name)?;
    let dynamic_features = app
        .state::<AppState>()
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .plugin_features(plugin_name)?;
    let attachments = app
        .state::<AppState>()
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .plugin_attachments(plugin_name)?;
    if attachments
        .iter()
        .map(|(_, _, data)| data.len())
        .sum::<usize>()
        > 32 * 1024 * 1024
    {
        return Err("插件同步附件镜像总量超过 32 MB，请清理数据或改用异步附件 API".to_owned());
    }
    let mut init_script = compatibility_script(
        app,
        &manifest,
        &action,
        &documents,
        &storage,
        &dynamic_features,
        &attachments,
    )?;
    init_script.push_str(&preload_adapter);
    let data_directory = runtime.root().join(".webview-data").join(&manifest.name);
    fs::create_dir_all(&data_directory).map_err(|error| error.to_string())?;

    // 完成所有可能失败的资源准备后，再发布插件窗口身份和路径权限。
    runtime.register_instance(&label, plugin_name)?;
    if let Err(error) = runtime.set_restore_main_on_close(&label, restore_main_on_close) {
        runtime.unregister_instance(&label);
        return Err(error);
    }
    let payload_paths = plugin_payload_paths(&action.payload);
    if !payload_paths.is_empty() {
        if let Err(error) = runtime.grant_paths(&label, payload_paths) {
            runtime.unregister_instance(&label);
            return Err(error);
        }
    }
    if embed_in_main {
        // 子 Webview 与主窗口共享外壳，但继续使用独立标签校验插件 API 和私有协议。
        let main = app
            .get_window("main")
            .ok_or_else(|| "主启动器窗口不存在".to_owned())?;
        let builder = tauri::webview::WebviewBuilder::new(&label, WebviewUrl::External(url))
            .data_directory(data_directory)
            .initialization_script(init_script);
        let webview = match main.add_child(
            builder,
            LogicalPosition::new(0.0, 61.0),
            LogicalSize::new(800.0, 539.0),
        ) {
            Ok(webview) => webview,
            Err(error) => {
                runtime.unregister_instance(&label);
                return Err(format!("无法在主窗口加载插件：{error}"));
            }
        };
        if let Err(error) = show_embedded_plugin(app, &webview, &manifest.title) {
            let _ = webview.close();
            runtime.unregister_instance(&label);
            return Err(error);
        }
        return Ok(());
    }
    // WebView2 可能需要几秒初始化，创建完成前保留主窗口供用户查看和操作。
    let build_result = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title(&manifest.title)
        .inner_size(720.0, 560.0)
        .min_inner_size(420.0, 280.0)
        .always_on_top(true)
        .center()
        .data_directory(data_directory)
        .initialization_script(init_script)
        .build();
    let window = match build_result {
        Ok(window) => window,
        Err(error) => {
            runtime.unregister_instance(&label);
            let _ = restore_main_window(app);
            return Err(format!("无法创建插件窗口：{error}"));
        }
    };
    window.on_window_event({
        let app = app.clone();
        let label = label.clone();
        move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                let runtime = app.state::<PluginRuntime>();
                let restore_main = runtime.restore_main_after_close(&label);
                runtime.unregister_instance(&label);
                if restore_main {
                    let _ = restore_main_window(&app);
                }
            }
        }
    });
    if let Err(error) = hide_main_window(app)
        .and_then(|_| window.show().map_err(|error| error.to_string()))
        .and_then(|_| window.set_focus().map_err(|error| error.to_string()))
    {
        // 创建成功但切换窗口失败时关闭孤儿窗口，并恢复可操作的主启动器。
        let _ = window.close();
        runtime.unregister_instance(&label);
        let _ = restore_main_window(app);
        return Err(format!("无法显示插件窗口：{error}"));
    }
    Ok(())
}

/**
 * 把主窗口内运行的插件 Webview 原位迁移到独立窗口，保留页面内存状态。
 * @param app 桌面应用句柄。
 * @param runtime 插件运行时与身份表。
 * @param plugin_name 要分离的插件名称。
 * @returns 独立窗口显示完成，失败时返回原因。
 */
pub(crate) fn detach_embedded_plugin(
    app: &AppHandle,
    runtime: &PluginRuntime,
    plugin_name: &str,
) -> Result<(), String> {
    validate_plugin_name(plugin_name)?;
    let label = plugin_window_label(plugin_name);
    runtime.plugin_for_window(&label)?;
    let webview = app
        .get_webview(&label)
        .ok_or_else(|| "插件页面不存在".to_owned())?;
    if webview.window().label() != "main" {
        return Err("插件已经在独立窗口中".to_owned());
    }
    let manifest = read_manifest(&runtime.root().join(plugin_name))?;
    let detached_label = format!("detached-{plugin_name}");
    if app.get_window(&detached_label).is_some() {
        return Err("插件独立窗口已经存在".to_owned());
    }

    // 先建空宿主窗口，再迁移原有 Webview；失败时销毁空窗口，保留原插件页面。
    let detached = WindowBuilder::new(app, &detached_label)
        .title(&manifest.title)
        .inner_size(800.0, 600.0)
        .min_inner_size(420.0, 280.0)
        .always_on_top(true)
        .center()
        .visible(false)
        .build()
        .map_err(|error| format!("无法创建插件独立窗口：{error}"))?;
    let main = app
        .get_window("main")
        .ok_or_else(|| "主启动器窗口不存在".to_owned())?;
    let transition = (|| -> Result<(), String> {
        webview
            .reparent(&detached)
            .map_err(|error| format!("无法移动插件页面：{error}"))?;
        // 分离窗口接管完整内容区；主窗口保留搜索栏并恢复普通结果布局。
        webview
            .set_position(LogicalPosition::new(0.0, 0.0))
            .map_err(|error| error.to_string())?;
        webview
            .set_size(LogicalSize::new(800.0, 600.0))
            .map_err(|error| error.to_string())?;
        webview
            .set_auto_resize(true)
            .map_err(|error| error.to_string())?;
        reset_embedded_layout(app)?;
        detached.show().map_err(|error| error.to_string())?;
        detached.set_focus().map_err(|error| error.to_string())?;
        webview.set_focus().map_err(|error| error.to_string())?;
        app.emit_to("main", "plugin-panel-closed", plugin_name)
            .map_err(|error| error.to_string())
    })();
    if let Err(error) = transition {
        // 任一步骤失败都把同一个页面放回搜索栏下方，避免留下不可见的插件窗口。
        if webview.window().label() == detached_label {
            let _ = webview.set_auto_resize(false);
            let _ = webview.reparent(&main);
            let _ = show_embedded_plugin(app, &webview, &manifest.title);
        }
        let _ = detached.close();
        let _ = main.show();
        return Err(error);
    }
    // 成功后才安装销毁回调，回滚空窗口时不能撤销仍在运行的插件身份。
    detached.on_window_event({
        let app = app.clone();
        let label = label.clone();
        move |event| {
            if matches!(event, WindowEvent::Destroyed) {
                app.state::<PluginRuntime>().unregister_instance(&label);
            }
        }
    });
    Ok(())
}

/**
 * 把插件 Webview 显示在主搜索框下方，并收起先前的内嵌插件。
 * @param app 桌面应用句柄。
 * @param webview 要显示的插件子 Webview。
 * @param title 插件标题。
 * @returns 主窗口与插件内容显示完成，失败时返回原因。
 */
fn show_embedded_plugin(
    app: &AppHandle,
    webview: &tauri::Webview,
    title: &str,
) -> Result<(), String> {
    // 单次只保留一个插件 Webview；释放旧实例后，布局可在关闭时还原成普通搜索框。
    for (label, other) in app.webviews() {
        if label != webview.label()
            && label.starts_with("plugin-")
            && other.window().label() == "main"
        {
            other.close().map_err(|error| error.to_string())?;
            app.state::<PluginRuntime>().unregister_instance(&label);
        }
    }
    let main = app
        .get_window("main")
        .ok_or_else(|| "主启动器窗口不存在".to_owned())?;
    main.set_size(LogicalSize::new(800.0, 600.0))
        .map_err(|error| error.to_string())?;
    // GTK/WebView2 首次创建时主窗口仍可能是搜索结果高度，扩展后重设子视图边界。
    webview
        .set_position(LogicalPosition::new(0.0, 61.0))
        .map_err(|error| error.to_string())?;
    webview
        .set_size(LogicalSize::new(800.0, 539.0))
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "linux")]
    arrange_linux_embedded_views(&main)?;
    main.show().map_err(|error| error.to_string())?;
    app.emit_to(
        "main",
        "plugin-panel-open",
        serde_json::json!({
            "name": webview.label().trim_start_matches("plugin-"),
            "title": title,
        }),
    )
    .map_err(|error| error.to_string())?;
    webview.show().map_err(|error| error.to_string())?;
    webview.set_focus().map_err(|error| error.to_string())
}

/**
 * 把 Linux 默认纵向 GtkBox 中的 Webview 放进固定布局，使插件真正覆盖搜索栏下方。
 * @param main 主窗口句柄。
 * @returns GTK 主线程布局任务投递结果。
 */
#[cfg(target_os = "linux")]
fn arrange_linux_embedded_views(main: &tauri::Window) -> Result<(), String> {
    use gtk::prelude::*;

    let window = main.clone();
    main.run_on_main_thread(move || {
        let Ok(box_layout) = window.default_vbox() else {
            return;
        };
        // Wry 默认将子 Webview 追加到纵向 Box；转成 Fixed 才能按像素叠放。
        let existing_fixed = box_layout
            .children()
            .into_iter()
            .find_map(|widget| widget.downcast::<gtk::Fixed>().ok());
        let had_fixed = existing_fixed.is_some();
        let fixed = existing_fixed.unwrap_or_else(|| {
            let fixed = gtk::Fixed::new();
            fixed.set_size_request(800, 600);
            box_layout.pack_start(&fixed, true, true, 0);
            fixed.show();
            fixed
        });
        fixed.set_size_request(800, 600);
        if had_fixed {
            if let Some(primary) = fixed.children().first() {
                primary.set_size_request(800, 61);
            }
        }
        let children = box_layout
            .children()
            .into_iter()
            .filter(|widget| widget != &fixed.clone().upcast::<gtk::Widget>())
            .collect::<Vec<_>>();
        for (index, widget) in children.into_iter().enumerate() {
            let is_main = !had_fixed && index == 0;
            box_layout.remove(&widget);
            widget.set_size_request(800, if is_main { 61 } else { 539 });
            fixed.put(&widget, 0, if is_main { 0 } else { 61 });
            if is_main {
                widget.show();
            }
        }
    })
    .map_err(|error| error.to_string())
}

/**
 * 关闭内嵌插件后把主 Webview 放回默认 GtkBox，恢复搜索结果的动态高度。
 * @param _app 桌面应用句柄；非 Linux 平台无需重排原生视图。
 * @returns 布局复位任务投递结果。
 */
pub(crate) fn reset_embedded_layout(_app: &AppHandle) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;

        if let Some(main) = _app.get_window("main") {
            let window = main.clone();
            main.run_on_main_thread(move || {
                let Ok(box_layout) = window.default_vbox() else {
                    return;
                };
                if let Some(fixed) = box_layout
                    .children()
                    .into_iter()
                    .find_map(|widget| widget.downcast::<gtk::Fixed>().ok())
                {
                    if let Some(primary) = fixed.children().first().cloned() {
                        fixed.remove(&primary);
                        box_layout.remove(&fixed);
                        primary.set_size_request(-1, 1);
                        box_layout.pack_start(&primary, true, true, 0);
                        primary.show();
                        if let Ok(gtk_window) = window.gtk_window() {
                            box_layout.queue_resize();
                            gtk::glib::timeout_add_local_once(
                                std::time::Duration::from_millis(50),
                                move || {
                                    // GDK 层可越过 WebKit 仍缓存的 600 像素自然高度。
                                    if let Some(gdk_window) = gtk_window.window() {
                                        gdk_window.resize(800, 61);
                                    }
                                },
                            );
                        }
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/**
 * 把百度翻译默认插件的功能限制到两个固定的官方页面。
 * @param code 插件功能代码。
 * @returns 对应页面的固定地址；其他功能返回错误。
 */
fn baidu_site_url(code: &str) -> Result<tauri::Url, String> {
    let address = match code {
        "text" => "https://fanyi.baidu.com/mtpe-individual/transText#/",
        "image" => "https://fanyi.baidu.com/mtpe-individual/transImg",
        _ => return Err("未知的百度翻译功能".to_owned()),
    };
    tauri::Url::parse(address).map_err(|error| format!("百度翻译地址无效：{error}"))
}

/**
 * 在主窗口的独立子 Webview 中打开百度官网，让站点自行处理匿名翻译。
 * @param app 桌面应用句柄。
 * @param runtime 插件运行时与隔离数据目录。
 * @param manifest 内置百度翻译插件声明。
 * @param action 用户选择的文字或图片入口。
 * @returns 官网内容显示完成，失败时返回原因。
 */
fn launch_baidu_site(
    app: &AppHandle,
    runtime: &PluginRuntime,
    manifest: &PluginManifest,
    action: &PluginEnterAction,
) -> Result<(), String> {
    // 只接受内置 manifest 声明的功能，且外部 URL 由宿主固定映射。
    if !manifest
        .features
        .iter()
        .any(|feature| feature.code == action.code)
    {
        return Err("百度翻译插件未声明该功能".to_owned());
    }
    let url = baidu_site_url(&action.code)?;
    let label = plugin_window_label(&manifest.name);
    if let Some(webview) = app.get_webview(&label) {
        if webview.window().label() == "main" {
            // 在已有子 Webview 中切换文字或图片翻译，保留站点会话。
            webview.navigate(url).map_err(|error| error.to_string())?;
            return show_embedded_plugin(app, &webview, &manifest.title);
        } else if webview.window().label().starts_with("detached-") {
            // 已分离的官网视图继续复用同一页面与登录会话。
            webview.navigate(url).map_err(|error| error.to_string())?;
            let detached = webview.window();
            detached.show().map_err(|error| error.to_string())?;
            detached.set_focus().map_err(|error| error.to_string())?;
            return Ok(());
        }
    }
    let data_directory = runtime.root().join(".webview-data").join(&manifest.name);
    fs::create_dir_all(&data_directory).map_err(|error| error.to_string())?;
    // 完成资源准备后再登记窗口身份；创建失败时撤销登记。
    runtime.register_instance(&label, &manifest.name)?;
    let main = app
        .get_window("main")
        .ok_or_else(|| "主启动器窗口不存在".to_owned())?;
    let builder = tauri::webview::WebviewBuilder::new(&label, WebviewUrl::External(url))
        .data_directory(data_directory.clone());
    let webview = match main.add_child(
        builder,
        LogicalPosition::new(0.0, 61.0),
        LogicalSize::new(800.0, 539.0),
    ) {
        Ok(webview) => webview,
        Err(error) => {
            runtime.unregister_instance(&label);
            return Err(format!("无法在主窗口加载百度翻译：{error}"));
        }
    };
    if let Err(error) = show_embedded_plugin(app, &webview, &manifest.title) {
        let _ = webview.close();
        runtime.unregister_instance(&label);
        return Err(error);
    }
    Ok(())
}

/**
 * 把选中的文字填入有道网页的原版输入框，并在页面延迟渲染时重试。
 * @param action 插件启动动作及选中文字。
 * @returns 可注入官网 Webview 的脚本。
 */
fn youdao_input_script(action: &PluginEnterAction) -> Result<String, String> {
    let payload = action.payload.as_str().unwrap_or_default();
    let quoted = serde_json::to_string(payload).map_err(|error| error.to_string())?;
    Ok(format!(
        r#"(() => {{
  const text = {quoted};
  if (!text) return;
  let attempts = 0;
  const timer = setInterval(() => {{
    const candidates = document.querySelectorAll('#js_fanyi_input, textarea, [contenteditable="true"]');
    const input = [...candidates].find((element) => {{
      const box = element.getBoundingClientRect();
      return box.width > 120 && box.height > 20 && box.left > 150 && box.top > 150
        && getComputedStyle(element).visibility !== 'hidden';
    }});
    // 官网页面异步装载较慢；只给可见的翻译输入区填值，避免过早命中隐藏编辑器。
    if (!input && ++attempts < 300) return;
    clearInterval(timer);
    if (!input) return;
    input.focus();
    if (input instanceof HTMLTextAreaElement || input instanceof HTMLInputElement) {{
      const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(input), 'value')?.set;
      setter?.call(input, text);
    }} else {{
      input.textContent = text;
    }}
    input.dispatchEvent(new InputEvent('input', {{ bubbles: true, data: text, inputType: 'insertText' }}));
  }}, 200);
}})();"#
    ))
}

/**
 * 在主窗口搜索框下方打开有道官网，保留原版网页翻译界面。
 * @param app 桌面应用句柄。
 * @param runtime 插件运行时与隔离数据目录。
 * @param manifest 内置有道翻译插件声明。
 * @param action 本次进入的功能与选中文字。
 * @returns 网页显示结果。
 */
fn launch_youdao_site(
    app: &AppHandle,
    runtime: &PluginRuntime,
    manifest: &PluginManifest,
    action: &PluginEnterAction,
) -> Result<(), String> {
    if action.code != "fanyi" {
        return Err("未知的有道翻译功能".to_owned());
    }
    let url = tauri::Url::parse("https://fanyi.youdao.com/#/TextTranslate")
        .map_err(|error| error.to_string())?;
    let label = plugin_window_label(&manifest.name);
    let input_script = youdao_input_script(action)?;
    // 复用内嵌或分离视图，避免重复建立网站会话。
    if let Some(webview) = app.get_webview(&label) {
        webview
            .eval(&input_script)
            .map_err(|error| error.to_string())?;
        if webview.window().label() == "main" {
            return show_embedded_plugin(app, &webview, &manifest.title);
        }
        let detached = webview.window();
        detached.show().map_err(|error| error.to_string())?;
        detached.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }
    let data_directory = runtime.root().join(".webview-data").join(&manifest.name);
    fs::create_dir_all(&data_directory).map_err(|error| error.to_string())?;
    runtime.register_instance(&label, &manifest.name)?;
    let main = app
        .get_window("main")
        .ok_or_else(|| "主启动器窗口不存在".to_owned())?;
    let builder = tauri::webview::WebviewBuilder::new(&label, WebviewUrl::External(url))
        .data_directory(data_directory)
        .initialization_script(input_script);
    let webview = match main.add_child(
        builder,
        LogicalPosition::new(0.0, 61.0),
        LogicalSize::new(800.0, 539.0),
    ) {
        Ok(webview) => webview,
        Err(error) => {
            runtime.unregister_instance(&label);
            return Err(format!("无法在主窗口加载有道翻译：{error}"));
        }
    };
    if let Err(error) = show_embedded_plugin(app, &webview, &manifest.title) {
        let _ = webview.close();
        runtime.unregister_instance(&label);
        return Err(error);
    }
    Ok(())
}

/// 临时取消主启动器置顶并隐藏它，确保插件置顶窗口能获得真实前台层级。
fn hide_main_window(app: &AppHandle) -> Result<(), String> {
    if let Some(main) = app.get_window("main") {
        main.set_always_on_top(false)
            .map_err(|error| error.to_string())?;
        main.hide().map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// 恢复主启动器的置顶属性、可见性和焦点，供插件退出及创建失败回滚使用。
pub(crate) fn restore_main_window(app: &AppHandle) -> Result<(), String> {
    if let Some(main) = app.get_window("main") {
        main.set_always_on_top(true)
            .map_err(|error| error.to_string())?;
        main.show().map_err(|error| error.to_string())?;
        main.set_focus().map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// 关闭插件的活动窗口，使升级或卸载不会保留被占用的资源。
pub(crate) fn close_plugin_window(app: &AppHandle, plugin_name: &str) -> Result<(), String> {
    validate_plugin_name(plugin_name)?;
    let label = plugin_window_label(plugin_name);
    if let Some(webview) = app.get_webview(&label) {
        if webview.window().label() == "main" {
            webview.close().map_err(|error| error.to_string())?;
            app.state::<PluginRuntime>().unregister_instance(&label);
            reset_embedded_layout(app)?;
            app.emit_to("main", "plugin-panel-closed", plugin_name)
                .map_err(|error| error.to_string())?;
            return Ok(());
        }
        // 分离窗口使用不同于插件 Webview 的标签，须按真实父窗口关闭。
        webview
            .window()
            .close()
            .map_err(|error| error.to_string())?;
        app.state::<PluginRuntime>().unregister_instance(&label);
        return Ok(());
    }
    if let Some(window) = app.get_webview_window(&label) {
        window.close().map_err(|error| error.to_string())?;
    }
    Ok(())
}

/**
 * 从私有协议读取当前插件资源，另向各插件提供宿主内置的只读样式令牌。
 * @param root 插件安装根目录。
 * @param webview_label 请求来源窗口标识。
 * @param request 当前资源请求。
 * @returns 资源响应或未找到响应。
 */
pub(crate) fn serve_plugin_asset(
    root: &Path,
    webview_label: &str,
    request: http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    // 所有插件只共享宿主内置的只读 UI 样式，不开放其他插件的私有资源。
    if request.uri().path() == "/_ui/theme.css" {
        return http::Response::builder()
            .status(http::StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "text/css; charset=utf-8")
            .header("X-Content-Type-Options", "nosniff")
            .body(include_bytes!("../resources/ui-theme.css").to_vec())
            .unwrap_or_else(|_| http::Response::new(Vec::new()));
    }
    match resolve_plugin_asset(root, webview_label, request.uri().path())
        .and_then(|path| fs::read(&path).map(|bytes| (path, bytes)).map_err(|error| error.to_string()))
    {
        Ok((path, bytes)) => http::Response::builder()
            .status(http::StatusCode::OK)
            .header(http::header::CONTENT_TYPE, content_type_for(&path))
            .header("X-Content-Type-Options", "nosniff")
            .header(
                http::header::CONTENT_SECURITY_POLICY,
                "default-src 'self' data: blob:; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self' ipc: http://ipc.localhost https://ipc.localhost",
            )
            .body(bytes)
            .unwrap_or_else(|_| http::Response::new(Vec::new())),
        Err(error) => http::Response::builder()
            .status(http::StatusCode::NOT_FOUND)
            .header(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(error.into_bytes())
            .unwrap_or_else(|_| http::Response::new(Vec::new())),
    }
}

/**
 * 解析插件资源并验证窗口身份，包括截图插件的贴图和历史窗口。
 * @param root 插件根目录。
 * @param webview_label 请求资源的 Webview 标识。
 * @param uri_path 协议资源路径。
 * @returns 通过身份与目录边界校验的路径或错误。
 */
fn resolve_plugin_asset(
    root: &Path,
    webview_label: &str,
    uri_path: &str,
) -> Result<PathBuf, String> {
    let mut components = uri_path.trim_start_matches('/').split('/');
    let plugin_name = components.next().unwrap_or_default();
    validate_plugin_name(plugin_name)?;
    let base_label = plugin_window_label(plugin_name);
    let is_screenshot_pin = plugin_name == "screenshot"
        && (webview_label.starts_with("plugin-screenshot-pin-")
            || webview_label == "plugin-screenshot-history");
    let relative = components.collect::<PathBuf>();
    validate_relative_path(&relative)?;
    let plugin_root = root
        .join(plugin_name)
        .canonicalize()
        .map_err(|_| "插件目录不存在".to_owned())?;
    if webview_label != base_label && !is_screenshot_pin {
        // 主启动器只可读取 manifest 明确声明的图标，插件 HTML/JS 继续隔离。
        let logo = read_manifest(&plugin_root)?.logo;
        if webview_label != "main" || logo.is_empty() || relative != logo {
            return Err("插件窗口与资源身份不匹配".to_owned());
        }
    }
    let target = plugin_root
        .join(relative)
        .canonicalize()
        .map_err(|_| "插件资源不存在".to_owned())?;
    if !target.starts_with(&plugin_root) || !target.is_file() {
        return Err("插件资源越过安装目录".to_owned());
    }
    Ok(target)
}

/// 读取并校验插件 manifest 及其入口文件。
fn read_manifest(directory: &Path) -> Result<PluginManifest, String> {
    let manifest_path = directory.join("plugin.json");
    let bytes =
        fs::read(&manifest_path).map_err(|error| format!("无法读取 plugin.json：{error}"))?;
    if bytes.len() > 512 * 1024 {
        return Err("plugin.json 不能超过 512 KB".to_owned());
    }
    let manifest: PluginManifest =
        serde_json::from_slice(&bytes).map_err(|error| format!("plugin.json 格式错误：{error}"))?;
    validate_plugin_name(&manifest.name)?;
    if manifest.title.trim().is_empty() || manifest.title.chars().count() > 100 {
        return Err("插件标题不能为空且不能超过 100 个字符".to_owned());
    }
    semver::Version::parse(&manifest.version).map_err(|error| format!("插件版本无效：{error}"))?;
    let main = PathBuf::from(&manifest.main);
    validate_relative_path(&main)?;
    let canonical_directory = directory
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let canonical_main = directory
        .join(&main)
        .canonicalize()
        .map_err(|_| "插件入口文件不存在".to_owned())?;
    if !canonical_main.starts_with(&canonical_directory) || !canonical_main.is_file() {
        return Err("插件入口必须是安装目录内的文件".to_owned());
    }
    for feature in &manifest.features {
        if feature.code.trim().is_empty() || feature.code.chars().count() > 160 {
            return Err("插件 feature code 不能为空且不能超过 160 个字符".to_owned());
        }
    }
    Ok(manifest)
}

/// 将插件 manifest 转换为前端安全展示的数据，并标记已知兼容风险。
fn plugin_summary(manifest: &PluginManifest, directory: &Path) -> InstalledPlugin {
    let mut notes = Vec::new();
    let compatibility = if manifest.preload.trim().is_empty() {
        "native-webview"
    } else if preload_adapter_script(manifest, directory).is_ok() {
        notes.push("Electron preload 已由内置 Rust 兼容桥适配，不加载 Node".to_owned());
        "adapted"
    } else {
        notes.push("包含 Node/Electron preload，当前无法启动".to_owned());
        "needs-adaptation"
    };
    let logo_url = if manifest.logo.trim().is_empty() {
        String::new()
    } else {
        // WebView2 只把导航 URL 自动映射到 Windows 私有协议域名；img.src 需要直接使用该域名。
        let origin = if cfg!(target_os = "windows") {
            "http://ztools-plugin.localhost"
        } else {
            "ztools-plugin://localhost"
        };
        format!(
            "{origin}/{}/{}",
            manifest.name,
            encode_relative_url_path(&manifest.logo)
        )
    };
    InstalledPlugin {
        name: manifest.name.clone(),
        title: manifest.title.clone(),
        description: manifest.description.clone(),
        version: manifest.version.clone(),
        logo_url,
        features: manifest.features.clone(),
        compatibility: compatibility.to_owned(),
        compatibility_notes: notes,
        development: false,
        built_in: is_bundled_plugin(&manifest.name),
    }
}

/**
 * 仅在内容变化时原子发布内置插件文件，并创建资源子目录。
 * @param path 插件目录中的目标文件。
 * @param contents 编译进宿主的目标字节。
 * @returns 写入或跳过后的结果。
 */
fn write_bundled_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    if fs::read(path).is_ok_and(|current| current == contents) {
        return Ok(());
    }

    // 先写同目录临时文件，完整落盘后再发布，避免留下半写入的插件入口。
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("无法创建内置插件资源目录：{error}"))?;
    }
    let temporary = path.with_extension(format!(
        "{}.bundled-tmp",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("file")
    ));
    fs::write(&temporary, contents)
        .map_err(|error| format!("无法写入内置插件临时文件 {}：{error}", temporary.display()))?;
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("无法更新内置插件文件 {}：{error}", path.display()))?;
    }
    fs::rename(&temporary, path)
        .map_err(|error| format!("无法发布内置插件文件 {}：{error}", path.display()))?;
    Ok(())
}

/// 复制插件文件树，并限制符号链接、文件数量和解压后总体积。
fn copy_plugin_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    let mut files = 0_usize;
    let mut bytes = 0_u64;
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.path() == source {
            continue;
        }
        if entry.file_type().is_symlink() {
            let _ = fs::remove_dir_all(destination);
            return Err("插件包不能包含符号链接".to_owned());
        }
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(|error| error.to_string())?;
        validate_relative_path(relative)?;
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|error| error.to_string())?;
            continue;
        }
        files += 1;
        bytes = bytes.saturating_add(entry.metadata().map_err(|error| error.to_string())?.len());
        if files > MAX_PLUGIN_FILES || bytes > MAX_PLUGIN_BYTES {
            let _ = fs::remove_dir_all(destination);
            return Err("插件超过 5000 个文件或 100 MB 安装限制".to_owned());
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::copy(entry.path(), target).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// 为已审计的旧 preload 生成无 Node 浏览器桥，未知 preload 明确拒绝启动。
fn preload_adapter_script(manifest: &PluginManifest, directory: &Path) -> Result<String, String> {
    if manifest.preload.trim().is_empty() {
        return Ok(String::new());
    }
    let preload_path = PathBuf::from(&manifest.preload);
    validate_relative_path(&preload_path)?;
    let canonical_directory = directory
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let canonical_preload = directory
        .join(preload_path)
        .canonicalize()
        .map_err(|_| "插件 preload 文件不存在".to_owned())?;
    if !canonical_preload.starts_with(&canonical_directory) || !canonical_preload.is_file() {
        return Err("插件 preload 必须是安装目录内的文件".to_owned());
    }
    let preload = fs::read_to_string(canonical_preload)
        .map_err(|error| format!("无法读取插件 preload：{error}"))?;
    if preload.len() > 512 * 1024 {
        return Err("插件 preload 不能超过 512 KB".to_owned());
    }

    // 网页快开只使用 Node HTTP 下载图标，映射到仅允许固定 HTTPS 域名的 Rust 网络桥。
    if manifest.name == "web-quick-open"
        && manifest.preload == "preload.js"
        && preload.contains("require('node:http')")
        && preload.contains("window.webQuickOpen")
    {
        return Ok(web_quick_open_adapter(&preload));
    }

    // 文件重命名插件只暴露已审计的六个服务，原 Node preload 本身不会执行。
    if manifest.name == "file-renamer"
        && manifest.preload == "preload/services.js"
        && preload.contains("window.services")
        && preload.contains("fs.rename")
        && preload.contains("fs.promises.readdir")
    {
        return Ok(
            r#"(() => {
  const invoke = (command, args = {}) => window.__TAURI_INTERNALS__.invoke(command, args);
  const normalizeStats = (stats) => ({
    ...stats,
    mtimeMs: stats.modifiedAt,
    ctimeMs: stats.createdAt,
    birthtimeMs: stats.createdAt
  });
  const services = Object.freeze({
    rename: (oldPath, newPath) => invoke('plugin_rename_path', { oldPath, newPath }),
    exists: (path) => invoke('plugin_path_exists', { path }),
    getStats: async (path) => normalizeStats(await invoke('plugin_file_stats', { path })),
    readDirectory: async (path) => (await invoke('plugin_read_directory', { path })).map(normalizeStats),
    getClipboardFilePaths: async () => [],
    writeClipboardText: (text) => invoke('plugin_copy_text', { content: String(text ?? ''), shouldPaste: false }),
    getPathForFile: (file) => typeof file?.path === 'string' ? file.path : ''
  });
  Object.defineProperty(window, 'services', { value: services, configurable: false, writable: false });
})();"#
                .to_owned(),
        );
    }

    // UUID 插件仅把 Electron 剪贴板当可选后备；让 require 明确失败后会走页面内置兼容路径。
    if manifest.name == "uuid-kit"
        && manifest.preload == "preload.js"
        && preload.contains("window.__nativeCopy")
        && preload.contains("require('electron')")
    {
        return Ok(format!(
            "(() => {{ const require = () => {{ throw new Error('Node runtime is unavailable'); }};\n{preload}\n}})();"
        ));
    }

    // 官方剪贴板插件的 Electron 对象未实际使用，文件 stat 只服务图片元数据并映射到授权文件桥。
    if manifest.name == "clipboard"
        && manifest.preload == "preload.js"
        && preload.contains("window.ztools.clipboard.getHistory")
        && preload.contains("window.ztools.registerTool")
    {
        return Ok(format!(
            r#"(() => {{
  const invoke = (command, args = {{}}) => window.__TAURI_INTERNALS__.invoke(command, args);
  const require = (identifier) => {{
    if (identifier === 'electron') return {{}};
    if (identifier === 'fs/promises') return {{ stat: (path) => invoke('plugin_file_stats', {{ path: String(path ?? '') }}) }};
    throw new Error(`Unsupported preload module: ${{identifier}}`);
  }};
{preload}
}})();"#
        ));
    }

    // 不引用 Node/Electron 全局的 preload 本质是普通浏览器初始化脚本，可直接在隔离 Webview 中执行。
    let forbidden_tokens = [
        "require(",
        "process.",
        "__dirname",
        "__filename",
        "module.exports",
        "node:",
        "electron",
    ];
    if forbidden_tokens
        .iter()
        .all(|token| !preload.contains(token))
    {
        return Ok(format!("(() => {{\n{preload}\n}})();"));
    }

    Err("该插件包含尚未审计的 Electron/Node preload；无 Node 宿主不会执行它，请使用异步 ZTools API 或提供 Rust 适配器".to_owned())
}

/// 将网页快开的 Node HTTP 调用映射为最小事件接口和受限 Rust 请求命令。
fn web_quick_open_adapter(preload: &str) -> String {
    let prefix = r#"(() => {
  const invoke = (command, args = {}) => window.__TAURI_INTERNALS__.invoke(command, args);
  class CompatBuffer extends Uint8Array {
    static from(value) {
      if (value instanceof Uint8Array) return new CompatBuffer(value);
      if (value instanceof ArrayBuffer) return new CompatBuffer(new Uint8Array(value));
      if (Array.isArray(value)) return new CompatBuffer(value);
      if (typeof value === 'string') return new CompatBuffer(new TextEncoder().encode(value));
      throw new TypeError('Unsupported Buffer input');
    }
    static concat(values) {
      const length = values.reduce((total, value) => total + value.length, 0);
      const result = new CompatBuffer(length);
      let offset = 0;
      for (const value of values) { result.set(value, offset); offset += value.length; }
      return result;
    }
    toString(encoding = 'utf8') {
      if (encoding === 'base64') {
        let binary = '';
        for (let offset = 0; offset < this.length; offset += 0x8000) {
          binary += String.fromCharCode(...this.subarray(offset, offset + 0x8000));
        }
        return btoa(binary);
      }
      return new TextDecoder(encoding === 'utf16le' ? 'utf-16le' : 'utf-8').decode(this);
    }
  }
  const request = (url, options, callback) => {
    const requestListeners = new Map();
    let started = false;
    let destroyed = false;
    const emitRequest = (name, value) => requestListeners.get(name)?.forEach((listener) => listener(value));
    const requestObject = {
      on(name, listener) {
        const listeners = requestListeners.get(name) || [];
        listeners.push(listener);
        requestListeners.set(name, listeners);
        return requestObject;
      },
      destroy() { destroyed = true; },
      end() {
        if (started) return;
        started = true;
        invoke('plugin_http_request', {
          url: String(url),
          headers: options?.headers || {},
          maxBytes: 1024 * 1024
        }).then((result) => {
          if (destroyed) return;
          const responseListeners = new Map();
          const responseObject = {
            statusCode: result.statusCode,
            headers: result.headers,
            on(name, listener) {
              const listeners = responseListeners.get(name) || [];
              listeners.push(listener);
              responseListeners.set(name, listeners);
              return responseObject;
            }
          };
          callback(responseObject);
          queueMicrotask(() => {
            if (destroyed) return;
            const body = CompatBuffer.from(result.body);
            responseListeners.get('data')?.forEach((listener) => listener(body));
            responseListeners.get('end')?.forEach((listener) => listener());
          });
        }).catch((error) => emitRequest('error', error));
      }
    };
    return requestObject;
  };
  const require = (identifier) => {
    if (identifier === 'node:http' || identifier === 'node:https') return { request };
    throw new Error(`Unsupported preload module: ${identifier}`);
  };
  const Buffer = CompatBuffer;
"#;
    let suffix = "\n})();";
    [prefix, preload, suffix].concat()
}

/// 把 Tauri 显示器结构转换为兼容旧插件的可序列化屏幕快照。
fn monitor_snapshot(monitor: &Monitor) -> serde_json::Value {
    let position = monitor.position();
    let size = monitor.size();
    serde_json::json!({
        "id": monitor.name().map_or("display", String::as_str),
        "label": monitor.name().map_or("Display", String::as_str),
        "scaleFactor": monitor.scale_factor(),
        "bounds": {
            "x": position.x,
            "y": position.y,
            "width": size.width,
            "height": size.height
        },
        "workArea": {
            "x": position.x,
            "y": position.y,
            "width": size.width,
            "height": size.height
        },
        "size": { "width": size.width, "height": size.height },
        "workAreaSize": { "width": size.width, "height": size.height }
    })
}

/// 生成插件初始化脚本，在页面业务脚本前建立首批兼容 API。
fn compatibility_script(
    app: &AppHandle,
    manifest: &PluginManifest,
    action: &PluginEnterAction,
    documents: &[serde_json::Value],
    storage: &[(String, serde_json::Value)],
    dynamic_features: &[serde_json::Value],
    attachments: &[(String, String, Vec<u8>)],
) -> Result<String, String> {
    let plugin_json = serde_json::to_string(&manifest.name).map_err(|error| error.to_string())?;
    let version_json = serde_json::to_string(&app.package_info().version.to_string())
        .map_err(|error| error.to_string())?;
    let action_json = serde_json::to_string(action).map_err(|error| error.to_string())?;
    let documents_json = serde_json::to_string(documents).map_err(|error| error.to_string())?;
    let storage_json = serde_json::to_string(storage).map_err(|error| error.to_string())?;
    let dynamic_features_json =
        serde_json::to_string(dynamic_features).map_err(|error| error.to_string())?;
    let attachment_values = attachments
        .iter()
        .map(|(id, content_type, data)| {
            serde_json::json!({
                "id": id,
                "contentType": content_type,
                "base64": base64::engine::general_purpose::STANDARD.encode(data)
            })
        })
        .collect::<Vec<_>>();
    let attachments_json =
        serde_json::to_string(&attachment_values).map_err(|error| error.to_string())?;
    let platform_json =
        serde_json::to_string(std::env::consts::OS).map_err(|error| error.to_string())?;
    let displays = app
        .available_monitors()
        .map_err(|error| error.to_string())?
        .iter()
        .map(monitor_snapshot)
        .collect::<Vec<_>>();
    let displays_json = serde_json::to_string(&displays).map_err(|error| error.to_string())?;
    let primary_display = app
        .primary_monitor()
        .map_err(|error| error.to_string())?
        .as_ref()
        .map(monitor_snapshot)
        .unwrap_or(serde_json::Value::Null);
    let primary_display_json =
        serde_json::to_string(&primary_display).map_err(|error| error.to_string())?;
    let cursor = app.cursor_position().ok();
    let cursor_json = serde_json::to_string(
        &cursor.map(|position| serde_json::json!({ "x": position.x, "y": position.y })),
    )
    .map_err(|error| error.to_string())?;
    Ok(format!(
        r#"(() => {{
  const pluginName = {plugin_json};
  const appVersion = {version_json};
  const platform = {platform_json};
  const displays = {displays_json};
  const primaryDisplay = {primary_display_json};
  const cursorScreenPoint = {cursor_json};
  const pendingEnter = [{action_json}];
  const initialDocuments = {documents_json};
  const initialStorage = {storage_json};
  const initialFeatures = {dynamic_features_json};
  const initialAttachments = {attachments_json};
  const initialGrantedPaths = Array.isArray(pendingEnter[0]?.payload)
    ? pendingEnter[0].payload.filter((value) => typeof value === 'string')
    : [];
  let enterHandler = null;
  let outHandler = null;
  let pendingOperations = Promise.resolve();
  const invoke = (command, args = {{}}) => window.__TAURI_INTERNALS__.invoke(command, args);
  const enqueue = (command, args = {{}}) => {{
    const operation = pendingOperations.then(() => invoke(command, args));
    pendingOperations = operation.catch((error) => console.error(`[ztools:${{command}}]`, error));
    return operation;
  }};
  const dispatchEnter = (action) => {{
    if (typeof enterHandler === 'function') Promise.resolve().then(() => enterHandler(action));
    else pendingEnter.splice(0, pendingEnter.length, action);
  }};
  const registerEnter = (callback) => {{
    enterHandler = typeof callback === 'function' ? callback : null;
    if (enterHandler) pendingEnter.splice(0).forEach(dispatchEnter);
  }};
  const cloneValue = (value) => JSON.parse(JSON.stringify(value));
  const toBytes = (value) => {{
    if (value instanceof Uint8Array) return new Uint8Array(value);
    if (value instanceof ArrayBuffer) return new Uint8Array(value.slice(0));
    if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength));
    if (Array.isArray(value)) return Uint8Array.from(value);
    throw new Error('附件必须是 ArrayBuffer、TypedArray 或字节数组');
  }};
  const decodeBase64 = (value) => Uint8Array.from(atob(value), (character) => character.charCodeAt(0));
  const documents = new Map(initialDocuments.map((document) => [document._id, document]));
  const validateDocument = (document) => {{
    if (!document || typeof document !== 'object' || typeof document._id !== 'string' || !document._id)
      throw new Error('插件文档必须包含字符串 _id');
    return cloneValue(document);
  }};
  const db = Object.freeze({{
    allDocs: (prefix = '') => Array.from(documents.values())
      .filter((document) => !prefix || document._id.startsWith(prefix))
      .map(cloneValue),
    get: (documentId) => documents.has(documentId) ? cloneValue(documents.get(documentId)) : null,
    put: (document) => {{
      const value = validateDocument(document);
      documents.set(value._id, value);
      enqueue('plugin_db_put', {{ document: value }});
      return {{ ok: true, id: value._id }};
    }},
    remove: (document) => {{
      const documentId = typeof document === 'string' ? document : document?._id;
      if (typeof documentId !== 'string' || !documentId) throw new Error('删除插件文档需要 _id');
      documents.delete(documentId);
      enqueue('plugin_db_remove', {{ documentId }});
      return {{ ok: true, id: documentId }};
    }},
    bulkDocs: (values) => {{
      if (!Array.isArray(values)) throw new Error('bulkDocs 需要文档数组');
      return values.map((value) => db.put(value));
    }}
  }});
  const dbPromises = Object.freeze({{
    allDocs: async (prefix = '') => db.allDocs(prefix),
    get: async (documentId) => db.get(documentId),
    put: async (document) => {{
      const value = validateDocument(document);
      documents.set(value._id, value);
      return enqueue('plugin_db_put', {{ document: value }});
    }},
    remove: async (document) => {{
      const documentId = typeof document === 'string' ? document : document?._id;
      if (typeof documentId !== 'string' || !documentId) throw new Error('删除插件文档需要 _id');
      documents.delete(documentId);
      return enqueue('plugin_db_remove', {{ documentId }});
    }},
    bulkDocs: async (values) => {{
      if (!Array.isArray(values)) throw new Error('bulkDocs 需要文档数组');
      return Promise.all(values.map((value) => dbPromises.put(value)));
    }}
  }});
  const attachments = new Map(initialAttachments.map((attachment) => [attachment.id, {{
    contentType: attachment.contentType,
    data: decodeBase64(attachment.base64)
  }}]));
  const postAttachment = (attachmentId, attachment, contentType = 'application/octet-stream') => {{
    const id = String(attachmentId ?? '');
    const data = toBytes(attachment);
    const type = String(contentType || 'application/octet-stream');
    attachments.set(id, {{ contentType: type, data }});
    enqueue('plugin_attachment_put', {{ attachmentId: id, contentType: type, data: Array.from(data) }});
    return {{ ok: true, id }};
  }};
  const getAttachment = (attachmentId) => {{
    const value = attachments.get(String(attachmentId ?? ''));
    return value ? new Uint8Array(value.data) : null;
  }};
  const getAttachmentType = (attachmentId) => attachments.get(String(attachmentId ?? ''))?.contentType ?? null;
  const attachmentPromises = Object.freeze({{
    postAttachment: async (attachmentId, attachment, contentType = 'application/octet-stream') => {{
      const id = String(attachmentId ?? '');
      const data = toBytes(attachment);
      const type = String(contentType || 'application/octet-stream');
      attachments.set(id, {{ contentType: type, data }});
      return enqueue('plugin_attachment_put', {{ attachmentId: id, contentType: type, data: Array.from(data) }});
    }},
    getAttachment: async (attachmentId) => {{
      const response = await invoke('plugin_attachment_get', {{ attachmentId: String(attachmentId ?? '') }});
      if (!response) return null;
      const data = Uint8Array.from(response.data);
      attachments.set(response.id, {{ contentType: response.contentType, data }});
      return data;
    }},
    getAttachmentType: async (attachmentId) => {{
      const response = await invoke('plugin_attachment_get', {{ attachmentId: String(attachmentId ?? '') }});
      return response?.contentType ?? null;
    }}
  }});
  const storageValues = new Map(initialStorage);
  const dbStorage = Object.freeze({{
    getItem: (key) => storageValues.has(String(key)) ? cloneValue(storageValues.get(String(key))) : null,
    setItem: (key, value) => {{
      const storageKey = String(key);
      const storedValue = cloneValue(value);
      storageValues.set(storageKey, storedValue);
      enqueue('plugin_storage_set', {{ key: storageKey, value: storedValue }});
      return undefined;
    }},
    removeItem: (key) => {{
      const storageKey = String(key);
      storageValues.delete(storageKey);
      enqueue('plugin_storage_remove', {{ key: storageKey }});
      return undefined;
    }}
  }});
  const dynamicFeatures = new Map(initialFeatures.map((feature) => [feature.code, feature]));
  const getFeatures = (codes) => {{
    const values = Array.from(dynamicFeatures.values()).map(cloneValue);
    if (!Array.isArray(codes)) return values;
    const accepted = new Set(codes.map(String));
    return values.filter((feature) => accepted.has(feature.code));
  }};
  const setFeature = (feature) => {{
    const value = cloneValue(feature);
    if (!value || typeof value.code !== 'string' || !value.code) throw new Error('动态 feature 必须包含 code');
    dynamicFeatures.set(value.code, value);
    enqueue('plugin_feature_set', {{ feature: value }});
    return {{ ok: true, code: value.code }};
  }};
  const removeFeature = (code) => {{
    const featureCode = String(code ?? '');
    if (!featureCode) throw new Error('删除动态 feature 需要 code');
    dynamicFeatures.delete(featureCode);
    enqueue('plugin_feature_remove', {{ code: featureCode }});
    return {{ ok: true, code: featureCode }};
  }};
  let clipboardPollTimer = null;
  const clipboard = Object.freeze({{
    writeContent: async (data, shouldPaste = false) => {{
      if (data?.type === 'image' && /^image-\d+$/.test(String(data.content ?? '')))
        return invoke('plugin_clipboard_write_history', {{ id: String(data.content), shouldPaste: Boolean(shouldPaste) }});
      if (data?.type === 'file' && Array.isArray(data.content))
        return invoke('plugin_clipboard_write_files', {{ paths: data.content.map(String), shouldPaste: Boolean(shouldPaste) }});
      if (!data || data.type !== 'text' || typeof data.content !== 'string')
        throw new Error('当前版本 clipboard.writeContent 支持历史图片和文本');
      await enqueue('plugin_copy_text', {{ content: data.content, shouldPaste: Boolean(shouldPaste) }});
      return true;
    }},
    write: async (content, shouldPaste = false) => {{
      if ((typeof content === 'number' && Number.isInteger(content)) || (typeof content === 'string' && /^(?:(?:image|files)-)?\d+$/.test(content))) {{
        return invoke('plugin_clipboard_write_history', {{ id: String(content), shouldPaste: Boolean(shouldPaste) }});
      }}
      const text = typeof content === 'string' ? content : content?.content;
      if (typeof text !== 'string') throw new Error('clipboard.write 需要文本内容');
      await enqueue('plugin_copy_text', {{ content: text, shouldPaste: Boolean(shouldPaste) }});
      return true;
    }},
    getHistory: async (page = 1, pageSize = 50, kind) => invoke('plugin_clipboard_get_history', {{ page, pageSize, kind }}),
    search: async (query) => invoke('plugin_clipboard_search', {{ query: String(query ?? '') }}),
    delete: async (id) => invoke('plugin_clipboard_delete', {{ id: String(id) }}),
    clear: async () => invoke('plugin_clipboard_clear'),
    onChange: (callback) => {{
      if (typeof callback !== 'function') throw new TypeError('clipboard.onChange 需要回调函数');
      let lastId;
      if (clipboardPollTimer) clearInterval(clipboardPollTimer);
      const check = async () => {{
        const page = await invoke('plugin_clipboard_get_history', {{ page: 1, pageSize: 1 }});
        const nextId = page.items[0]?.id ?? null;
        if (lastId !== undefined && nextId !== lastId) callback(page.items[0] ?? null);
        lastId = nextId;
      }};
      void check();
      clipboardPollTimer = setInterval(() => void check(), 800);
      return () => {{ clearInterval(clipboardPollTimer); clipboardPollTimer = null; }};
    }}
  }});
  const dialog = Object.freeze({{
    open: async (options = {{}}) => invoke('plugin_dialog_open', {{ options }}),
    save: async (options = {{}}) => invoke('plugin_dialog_save', {{ options }})
  }});
  const file = Object.freeze({{
    exists: async (path) => invoke('plugin_path_exists', {{ path: String(path ?? '') }}),
    stat: async (path) => invoke('plugin_file_stats', {{ path: String(path ?? '') }}),
    readDirectory: async (path) => invoke('plugin_read_directory', {{ path: String(path ?? '') }}),
    readFile: async (path) => Uint8Array.from(await invoke('plugin_read_file', {{ path: String(path ?? '') }})),
    writeFile: async (path, data) => invoke('plugin_write_file', {{ path: String(path ?? ''), data: Array.from(toBytes(data)) }}),
    rename: async (oldPath, newPath) => invoke('plugin_rename_path', {{ oldPath: String(oldPath ?? ''), newPath: String(newPath ?? '') }}),
    copy: async (sourcePath, targetPath) => invoke('plugin_copy_path', {{ sourcePath: String(sourcePath ?? ''), targetPath: String(targetPath ?? '') }}),
    createDirectory: async (path) => invoke('plugin_create_directory', {{ path: String(path ?? '') }})
  }});
  const pluginWindow = Object.freeze({{
    setSize: async (width, height) => invoke('plugin_window_set_size', {{ width, height }}),
    setPosition: async (x, y) => invoke('plugin_window_set_position', {{ x, y }}),
    center: async () => invoke('plugin_window_center'),
    setAlwaysOnTop: async (enabled) => invoke('plugin_window_set_always_on_top', {{ enabled: Boolean(enabled) }}),
    setMinimized: async (minimized = true) => invoke('plugin_window_set_minimized', {{ minimized: Boolean(minimized) }}),
    setMaximized: async (maximized = true) => invoke('plugin_window_set_maximized', {{ maximized: Boolean(maximized) }}),
    setFullscreen: async (fullscreen = true) => invoke('plugin_window_set_fullscreen', {{ fullscreen: Boolean(fullscreen) }})
  }});
  const screen = Object.freeze({{
    getPrimaryDisplay: () => cloneValue(primaryDisplay),
    getAllDisplays: () => cloneValue(displays),
    getCursorScreenPoint: () => cloneValue(cursorScreenPoint),
    capture: async () => Uint8Array.from(await invoke('plugin_screen_capture'))
  }});
  const input = Object.freeze({{
    typeText: async (content, targetExternal = false) => invoke('plugin_input_type_text', {{ content: String(content ?? ''), targetExternal: Boolean(targetExternal) }}),
    tapKey: async (key, targetExternal = false) => invoke('plugin_input_tap_key', {{ key: String(key ?? ''), targetExternal: Boolean(targetExternal) }}),
    pasteText: async (content) => invoke('plugin_copy_text', {{ content: String(content ?? ''), shouldPaste: true }})
  }});
  let subInputHandler = null;
  let subInputValue = '';
  const filterClipboardDom = () => {{
    if (pluginName !== 'clipboard') return;
    const needle = subInputValue.trim().toLocaleLowerCase();
    let visible = 0;
    document.querySelectorAll('.clipboard-item').forEach((item) => {{
      const matches = !needle || item.textContent.toLocaleLowerCase().includes(needle);
      if (matches) visible += 1;
      if (matches) item.style.removeProperty('display');
      else item.style.setProperty('display', 'none', 'important');
    }});
    const empty = document.querySelector('[data-ztools-clipboard-search-empty]');
    if ((!needle || visible > 0) && empty) empty.remove();
    if (needle && visible === 0 && !empty && document.querySelector('.clipboard-app')) {{
      const hint = document.createElement('div');
      hint.dataset.ztoolsClipboardSearchEmpty = '';
      hint.textContent = '没有匹配的剪贴板记录';
      hint.style.cssText = 'padding:36px 16px;text-align:center;color:#999;font-size:13px';
      document.querySelector('.clipboard-app').append(hint);
    }}
  }};
  if (pluginName === 'clipboard') {{
    window.addEventListener('DOMContentLoaded', () => {{
      new MutationObserver(() => requestAnimationFrame(filterClipboardDom))
        .observe(document.body, {{ childList: true, subtree: true }});
    }}, {{ once: true }});
  }}
  const setSubInput = async (callback) => {{ subInputHandler = typeof callback === 'function' ? callback : null; return true; }};
  const setSubInputValue = (value) => {{
    subInputValue = String(value ?? '');
    if (pluginName === 'clipboard') requestAnimationFrame(filterClipboardDom);
    else subInputHandler?.({{ text: subInputValue }});
    return true;
  }};
  Object.defineProperty(window, '__ztoolsSubInputFromHost', {{
    value: (value) => {{
      subInputValue = String(value ?? '');
      if (pluginName === 'clipboard') requestAnimationFrame(filterClipboardDom);
      else setTimeout(() => subInputHandler?.({{ text: subInputValue }}), 0);
    }},
    configurable: false
  }});
  if (pluginName === 'clipboard') {{
    window.addEventListener('DOMContentLoaded', () => {{
      // 主视图与插件视图分开读取同一搜索状态，避免 GTK 在输入回调中调整子视图层级。
      setInterval(() => void invoke('plugin_get_sub_input').then((value) => {{
        if (value !== subInputValue) window.__ztoolsSubInputFromHost(value);
      }}).catch((error) => console.error('[ztools:sub-input]', error)), 180);
    }}, {{ once: true }});
  }}
  const api = Object.freeze({{
    getAppName: () => 'ZTools',
    getAppVersion: () => appVersion,
    getWindowType: () => 'plugin',
    isWindows: () => platform === 'windows',
    isLinux: () => platform === 'linux',
    isMacOS: () => platform === 'macos',
    isMacOs: () => platform === 'macos',
    isDev: () => false,
    hasCapability: (name) => ['lifecycle', 'clipboard:text', 'notification', 'db', 'db.promises', 'db.attachments', 'dbStorage', 'feature', 'file:granted', 'shell:granted', 'dialog:async', 'dialog:seeded', 'screen:metadata', 'screen:capture', 'input:basic', 'window:self'].includes(name),
    onPluginEnter: registerEnter,
    onPluginReady: registerEnter,
    onPluginOut: (callback) => {{ outHandler = typeof callback === 'function' ? callback : null; }},
    onMainPush: () => undefined,
    outPlugin: async () => {{
      await pendingOperations;
      return invoke('plugin_out');
    }},
    hideMainWindow: async (isRestorePreWindow = true) => {{
      await pendingOperations;
      return invoke('plugin_hide_main_window', {{ isRestorePreWindow }});
    }},
    showNotification: async (body) => invoke('plugin_show_notification', {{ body: String(body ?? '') }}),
    showToast: async (message) => invoke('plugin_show_notification', {{ body: String(message ?? '') }}),
    setSubInput,
    setSubInputValue,
    subInputFocus: () => {{ void invoke('plugin_focus_sub_input'); return true; }},
    registerTool: () => false,
    copyText: (text) => {{
      enqueue('plugin_copy_text', {{ content: String(text ?? ''), shouldPaste: false }});
      return true;
    }},
    getPathForFile: (file) => typeof file?.path === 'string' ? file.path : '',
    getPrimaryDisplay: () => cloneValue(primaryDisplay),
    getAllDisplays: () => cloneValue(displays),
    getCursorScreenPoint: () => cloneValue(cursorScreenPoint),
    setExpendHeight: async (height) => pluginWindow.setSize(globalThis.innerWidth, Math.max(160, Number(height) || 160)),
    showOpenDialog: () => initialGrantedPaths.slice(),
    showOpenDialogAsync: dialog.open,
    showSaveDialogAsync: dialog.save,
    shellOpenExternal: async (url) => invoke('plugin_shell_open', {{ target: String(url ?? '') }}),
    shellOpenPath: async (path) => invoke('plugin_shell_open', {{ target: String(path ?? '') }}),
    shellShowItemInFolder: async (path) => invoke('plugin_shell_reveal', {{ path: String(path ?? '') }}),
    db: Object.freeze(Object.assign({{}}, db, {{
      postAttachment,
      getAttachment,
      getAttachmentType,
      promises: Object.freeze(Object.assign({{}}, dbPromises, attachmentPromises))
    }})),
    dbStorage,
    getFeatures,
    setFeature,
    removeFeature,
    clipboard,
    dialog,
    file,
    screen,
    input,
    window: pluginWindow
  }});
  Object.defineProperty(window, 'ztools', {{ value: api, configurable: false, writable: false }});
  Object.defineProperty(window, '__ztoolsPluginName', {{ value: pluginName, configurable: false }});
  Object.defineProperty(window, '__ztoolsDispatchEnter', {{ value: dispatchEnter, configurable: false }});
  window.addEventListener('beforeunload', () => {{
    if (clipboardPollTimer) clearInterval(clipboardPollTimer);
    if (outHandler) outHandler();
  }}, {{ once: true }});
}})();"#
    ))
}

/// 校验插件名称可安全用作目录名、URL 分段和窗口标签。
pub(crate) fn validate_plugin_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || !name.as_bytes()[0].is_ascii_alphanumeric()
    {
        return Err(
            "插件名称只能包含字母、数字、短横线和下划线，且必须以字母或数字开头".to_owned(),
        );
    }
    Ok(())
}

/// 拒绝绝对路径、父目录、平台前缀和空入口路径。
fn validate_relative_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("插件路径必须位于安装目录内".to_owned());
    }
    Ok(())
}

/// 从 files 类型进入参数中提取现有绝对路径，普通文本 payload 不产生文件授权。
fn plugin_payload_paths(payload: &serde_json::Value) -> Vec<PathBuf> {
    let values = match payload {
        serde_json::Value::Array(values) => values.as_slice(),
        serde_json::Value::Object(object) => object
            .get("files")
            .and_then(serde_json::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default(),
        _ => &[],
    };
    values
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.exists())
        .collect()
}

/// 对 manifest 中的相对路径进行保守 URL 编码，保留目录分隔符。
fn encode_relative_url_path(path: &str) -> String {
    path.replace('\\', "/").replace(' ', "%20")
}

/// 为单实例插件生成稳定且可从协议请求反查身份的窗口标签。
fn plugin_window_label(plugin_name: &str) -> String {
    format!("plugin-{plugin_name}")
}

/// 根据文件扩展名返回常见 Web 资源 MIME 类型。
fn content_type_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// 返回当前 Unix 毫秒时间，用于安装事务临时目录去重。
fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        extract_plugin_zip, http, plugin_summary, preload_adapter_script, read_manifest,
        resolve_plugin_asset, serve_plugin_asset, validate_market_download_url, PluginRuntime,
    };
    use std::{
        fs,
        io::{Cursor, Write},
        path::{Path, PathBuf},
        time::SystemTime,
    };
    use zip::{write::SimpleFileOptions, ZipWriter};

    /// 创建唯一临时目录，避免插件安装测试写入真实应用数据。
    fn fixture_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ztools-plugin-{name}-{}",
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be valid")
                .as_nanos()
        ))
    }

    /// 写入最小插件目录，供 manifest 和安装事务测试复用。
    fn write_fixture(root: &Path, name: &str, version: &str) {
        fs::create_dir_all(root.join("assets")).expect("fixture directory should exist");
        fs::write(
            root.join("index.html"),
            "<script src='assets/main.js'></script>",
        )
        .expect("fixture html should be writable");
        fs::write(root.join("assets/main.js"), "window.fixture = true")
            .expect("fixture script should be writable");
        fs::write(
            root.join("plugin.json"),
            format!(
                r#"{{"name":"{name}","title":"Fixture","description":"test","version":"{version}","main":"index.html","features":[{{"code":"fixture","explain":"Fixture","cmds":["fixture"]}}]}}"#
            ),
        )
        .expect("fixture manifest should be writable");
    }

    /**
     * 验证空数据目录获得全部默认插件，损坏文件能够自动恢复。
     * @returns 无返回值。
     */
    #[test]
    fn installs_and_repairs_bundled_plugins() {
        let root = fixture_root("bundled-root");
        let runtime = PluginRuntime::new(root.clone()).expect("runtime should open");
        runtime
            .ensure_bundled_plugins()
            .expect("bundled plugins should install");

        let plugins = runtime
            .installed_plugins()
            .expect("bundled plugins should list");
        assert_eq!(plugins.len(), super::BUNDLED_PLUGIN_FILES.len());
        assert!(plugins.iter().all(|plugin| plugin.built_in));
        assert!(plugins.iter().any(|plugin| plugin.name == "setting"));
        assert!(plugins.iter().any(|plugin| plugin.name == "system"));
        assert!(plugins.iter().any(|plugin| plugin.name == "screenshot"));
        assert!(plugins.iter().any(|plugin| plugin.name == "break-reminder"));
        assert!(plugins
            .iter()
            .any(|plugin| plugin.name == "baidu-translate"));
        assert!(runtime.uninstall("setting").is_err());

        // 模拟用户目录中的 manifest 被截断，再次启动应恢复编译时版本。
        fs::write(root.join("system/plugin.json"), b"broken")
            .expect("bundled manifest should be mutable in fixture");
        runtime
            .ensure_bundled_plugins()
            .expect("bundled plugins should repair");
        assert_eq!(
            read_manifest(&root.join("system"))
                .expect("repaired manifest should load")
                .title,
            "系统"
        );
        fs::remove_dir_all(root).expect("runtime fixture should clean up");
    }

    /**
     * 确认百度翻译功能仅能导航到已审查的官网页面。
     * @returns 测试断言通过后结束。
     */
    #[test]
    fn baidu_site_urls_are_fixed() {
        assert_eq!(
            super::baidu_site_url("text")
                .expect("text should resolve")
                .as_str(),
            "https://fanyi.baidu.com/mtpe-individual/transText#/"
        );
        assert_eq!(
            super::baidu_site_url("image")
                .expect("image should resolve")
                .as_str(),
            "https://fanyi.baidu.com/mtpe-individual/transImg"
        );
        assert!(super::baidu_site_url("https://example.com").is_err());
    }

    /// 验证目录安装使用 staging 发布，并允许新版本原子替换旧版本。
    #[test]
    fn installs_and_upgrades_plugin_directory() {
        let root = fixture_root("install-root");
        let source = fixture_root("install-source");
        write_fixture(&source, "fixture", "1.0.0");
        let runtime = PluginRuntime::new(root.clone()).expect("runtime should open");
        let installed = runtime
            .install_from_directory(&source)
            .expect("fixture should install");
        assert_eq!(installed.version, "1.0.0");
        assert_eq!(installed.compatibility, "native-webview");
        write_fixture(&source, "fixture", "1.1.0");
        let upgraded = runtime
            .install_from_directory(&source)
            .expect("fixture should upgrade");
        assert_eq!(upgraded.version, "1.1.0");
        assert_eq!(
            read_manifest(&root.join("fixture"))
                .expect("installed manifest should load")
                .version,
            "1.1.0"
        );
        fs::remove_dir_all(root).expect("runtime fixture should clean up");
        fs::remove_dir_all(source).expect("source fixture should clean up");
    }

    /**
     * 验证资源协议权限和主窗口图标 URL 符合各平台 Webview 的实际协议。
     * @returns 无返回值。
     */
    #[test]
    fn confines_protocol_assets_to_matching_plugin_window() {
        let root = fixture_root("asset-root");
        let plugin = root.join("fixture");
        write_fixture(&plugin, "fixture", "1.0.0");
        let path = resolve_plugin_asset(&root, "plugin-fixture", "/fixture/assets/main.js")
            .expect("owned asset should resolve");
        assert!(path.ends_with("assets/main.js"));
        assert!(
            resolve_plugin_asset(&root, "plugin-fixture-secondary-1", "/fixture/index.html")
                .is_err()
        );
        assert!(resolve_plugin_asset(&root, "plugin-other", "/fixture/index.html").is_err());
        assert!(resolve_plugin_asset(&root, "plugin-fixture", "/fixture/../plugin.json").is_err());
        // 主搜索页只获得 manifest 声明的图标，不能借资源协议读取插件代码。
        fs::write(plugin.join("logo.png"), b"fixture logo").expect("logo should exist");
        fs::write(
            plugin.join("plugin.json"),
            r#"{"name":"fixture","title":"Fixture","version":"1.0.0","main":"index.html","logo":"logo.png"}"#,
        )
        .expect("manifest with logo should exist");
        assert!(resolve_plugin_asset(&root, "main", "/fixture/logo.png").is_ok());
        let manifest = read_manifest(&plugin).expect("manifest with logo should load");
        let summary = plugin_summary(&manifest, &plugin);
        let expected_origin = if cfg!(target_os = "windows") {
            "http://ztools-plugin.localhost"
        } else {
            "ztools-plugin://localhost"
        };
        assert_eq!(
            summary.logo_url,
            format!("{expected_origin}/fixture/logo.png")
        );
        assert!(resolve_plugin_asset(&root, "main", "/fixture/index.html").is_err());
        assert!(resolve_plugin_asset(&root, "main", "/fixture/assets/main.js").is_err());

        // 只有宿主创建的截图贴图子窗口可以读取同一个内置插件目录。
        let screenshot = root.join("screenshot");
        write_fixture(&screenshot, "screenshot", "1.0.0");
        assert!(
            resolve_plugin_asset(&root, "plugin-screenshot-pin-1", "/screenshot/index.html")
                .is_ok()
        );
        fs::remove_dir_all(root).expect("asset fixture should clean up");
    }

    /**
     * 验证共享样式只公开宿主内置 CSS，插件私有资源仍按原有边界隔离。
     * @returns 无返回值。
     */
    #[test]
    fn serves_shared_ui_theme_without_plugin_files() {
        let root = fixture_root("theme-root");
        fs::create_dir_all(&root).expect("theme fixture should exist");
        let request = http::Request::builder()
            .uri("/_ui/theme.css")
            .body(Vec::new())
            .unwrap();
        let response = serve_plugin_asset(&root, "plugin-fixture", request);
        assert_eq!(response.status(), http::StatusCode::OK);
        assert!(String::from_utf8_lossy(response.body()).contains("--z-ui-accent"));
        fs::remove_dir_all(root).expect("theme fixture should clean up");
    }

    /// 验证市场目录不能把下载器重定向到明文、本机或相似域名。
    #[test]
    fn accepts_only_official_https_market_downloads() {
        assert!(
            validate_market_download_url("https://ztools.zosen.link/timestamp-1.0.0.zip").is_ok()
        );
        assert!(validate_market_download_url("http://ztools.zosen.link/plugin.zip").is_err());
        assert!(
            validate_market_download_url("https://zosen.link.evil.example/plugin.zip").is_err()
        );
        assert!(validate_market_download_url("https://127.0.0.1/plugin.zip").is_err());
    }

    /// 验证市场包里的 npm 命令链接可忽略，而其他符号链接仍会阻止安装。
    #[test]
    fn ignores_only_npm_command_symlinks_in_market_archive() {
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        archive
            .start_file("plugin.json", options)
            .expect("manifest entry should start");
        archive
            .write_all(
                br#"{"name":"fixture","title":"Fixture","version":"1.0.0","main":"index.html"}"#,
            )
            .expect("manifest should be written");
        archive
            .start_file("index.html", options)
            .expect("entry should start");
        archive
            .write_all(b"<html></html>")
            .expect("entry should be written");
        archive
            .add_symlink(
                "preload/node_modules/.bin/acorn",
                "../acorn/bin/acorn",
                options,
            )
            .expect("npm command link should be written");
        let bytes = archive
            .finish()
            .expect("archive should finish")
            .into_inner();
        let root = fixture_root("market-bin-link");
        extract_plugin_zip(&bytes, &root).expect("npm command link should be ignored");
        assert!(root.join("plugin.json").is_file());
        assert!(!root.join("preload/node_modules/.bin/acorn").exists());
        fs::remove_dir_all(root).expect("fixture should clean up");

        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        archive
            .add_symlink("preload/runtime.js", "../secret", options)
            .expect("runtime link should be written");
        let bytes = archive
            .finish()
            .expect("archive should finish")
            .into_inner();
        let root = fixture_root("market-runtime-link");
        assert!(extract_plugin_zip(&bytes, &root).is_err());
        fs::remove_dir_all(root).expect("fixture should clean up");
    }

    /// 验证取消标记能被下载循环观察，并在任务结束后释放名称占用。
    #[test]
    fn cancels_and_releases_market_install_state() {
        let root = fixture_root("market-cancel-root");
        let runtime = PluginRuntime::new(root.clone()).expect("runtime should open");
        let cancelled = runtime
            .begin_market_install("fixture")
            .expect("install should start");
        assert!(runtime.begin_market_install("fixture").is_err());
        assert!(runtime
            .cancel_market_install("fixture")
            .expect("cancel should be accepted"));
        assert!(cancelled.load(std::sync::atomic::Ordering::Relaxed));
        runtime.finish_market_install("fixture");
        assert!(!runtime
            .cancel_market_install("fixture")
            .expect("completed install should be absent"));
        fs::remove_dir_all(root).expect("runtime fixture should clean up");
    }

    /// 验证普通浏览器 preload 可执行，而未知 Node preload 会在启动前被拒绝。
    #[test]
    fn classifies_browser_and_unknown_node_preloads() {
        let root = fixture_root("preload-root");
        write_fixture(&root, "fixture", "1.0.0");
        fs::write(
            root.join("preload.js"),
            "window.fixtureBridge = Object.freeze({});",
        )
        .expect("browser preload should be writable");
        let mut manifest = read_manifest(&root).expect("manifest should load");
        manifest.preload = "preload.js".to_owned();
        assert!(preload_adapter_script(&manifest, &root).is_ok());
        fs::write(
            root.join("preload.js"),
            "const fs = require('node:fs'); window.fixtureBridge = fs;",
        )
        .expect("node preload should be writable");
        assert!(preload_adapter_script(&manifest, &root).is_err());
        fs::remove_dir_all(root).expect("preload fixture should clean up");
    }

    /// 验证文件权限只绑定当前插件窗口，并在窗口注销时失效。
    #[test]
    fn confines_user_paths_to_registered_plugin_window() {
        let root = fixture_root("grant-root");
        let plugin = root.join("fixture");
        write_fixture(&plugin, "fixture", "1.0.0");
        let selected_root = fixture_root("grant-selected");
        let unrelated_root = fixture_root("grant-unrelated");
        fs::create_dir_all(&selected_root).expect("selected fixture should exist");
        fs::create_dir_all(&unrelated_root).expect("unrelated fixture should exist");
        let selected = selected_root.join("selected.txt");
        let sibling = selected_root.join("renamed.txt");
        let unrelated = unrelated_root.join("secret.txt");
        fs::write(&selected, "selected").expect("selected file should exist");
        fs::write(&unrelated, "secret").expect("unrelated file should exist");

        let runtime = PluginRuntime::new(root.clone()).expect("runtime should open");
        runtime
            .register_instance("plugin-fixture", "fixture")
            .expect("instance should register");
        runtime
            .grant_paths("plugin-fixture", [selected.clone()])
            .expect("selected path should be granted");
        assert!(runtime
            .authorized_existing_path("plugin-fixture", &selected)
            .is_ok());
        assert!(runtime
            .authorized_new_path("plugin-fixture", &sibling)
            .is_ok());
        assert!(runtime
            .authorized_existing_path("plugin-fixture", &unrelated)
            .is_err());
        runtime.unregister_instance("plugin-fixture");
        assert!(runtime
            .authorized_existing_path("plugin-fixture", &selected)
            .is_err());

        fs::remove_dir_all(root).expect("runtime fixture should clean up");
        fs::remove_dir_all(selected_root).expect("selected fixture should clean up");
        fs::remove_dir_all(unrelated_root).expect("unrelated fixture should clean up");
    }
}
