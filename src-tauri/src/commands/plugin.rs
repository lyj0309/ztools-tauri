use std::{fs, path::PathBuf};

use tauri::{AppHandle, Emitter, Manager, State, Webview};
use tauri_plugin_notification::{NotificationExt, PermissionState};

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginFileStats {
    name: String,
    path: String,
    is_file: bool,
    is_directory: bool,
    size: u64,
    modified_at: u64,
    created_at: u64,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginHttpResponse {
    status_code: u16,
    headers: std::collections::HashMap<String, String>,
    body: Vec<u8>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct PluginDialogOptions {
    title: String,
    default_path: String,
    default_name: String,
    properties: Vec<String>,
    filters: Vec<PluginDialogFilter>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
pub(crate) struct PluginDialogFilter {
    name: String,
    extensions: Vec<String>,
}

use crate::{
    desktop,
    models::ClipboardEntry,
    plugin::{
        self, InstalledPlugin, MarketPlugin, PluginEnterAction, PluginFeature, PluginRuntime,
    },
    state::AppState,
    storage::PluginDataItem,
};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledPluginDetail {
    readme: String,
    data: Vec<PluginDataItem>,
}

/// 把宿主纯文本记录转换为旧剪贴板插件可识别的项目结构。
fn clipboard_item(entry: ClipboardEntry) -> serde_json::Value {
    let preview = entry.content.clone();
    serde_json::json!({
        "id": entry.id.to_string(),
        "type": "text",
        "content": entry.content,
        "preview": preview,
        "timestamp": entry.captured_at,
        "appName": null
    })
}

/// 返回已经安装且 manifest 有效的插件列表。
#[tauri::command]
pub(crate) fn list_plugins(
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<Vec<InstalledPlugin>, String> {
    list_installed_plugins(&runtime, &state)
}

/// 匿名读取官方插件市场的当前平台目录。
#[tauri::command]
pub(crate) async fn fetch_plugin_market() -> Result<Vec<MarketPlugin>, String> {
    plugin::fetch_market_plugins().await
}

/// 从官方市场下载、安全解压并安装或升级插件。
#[tauri::command]
pub(crate) async fn install_plugin_from_market(
    plugin_name: String,
    app: AppHandle,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<Vec<InstalledPlugin>, String> {
    plugin::install_market_plugin(&app, &runtime, &plugin_name).await?;
    list_installed_plugins(&runtime, &state)
}

/// 取消正在解析地址或下载中的市场插件安装。
#[tauri::command]
pub(crate) fn cancel_plugin_market_install(
    plugin_name: String,
    runtime: State<'_, PluginRuntime>,
) -> Result<bool, String> {
    plugin::validate_plugin_name(&plugin_name)?;
    runtime.cancel_market_install(&plugin_name)
}

/// 从已经解压的本地目录安装或升级插件，并返回最新插件列表。
#[tauri::command]
pub(crate) fn install_plugin_directory(
    path: String,
    app: AppHandle,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<Vec<InstalledPlugin>, String> {
    let source = PathBuf::from(path);
    let source_manifest = source.join("plugin.json");
    let plugin_name = std::fs::read(&source_manifest)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| {
            value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    if let Some(plugin_name) = plugin_name {
        // 升级前释放现有 Webview 对插件文件和缓存的占用。
        plugin::close_plugin_window(&app, &plugin_name)?;
    }
    runtime.install_from_directory(&source)?;
    list_installed_plugins(&runtime, &state)
}

/// 注册插件开发目录，先同步隔离副本，再监听文件变化并自动刷新活动页面。
#[tauri::command]
pub(crate) fn register_plugin_development(
    path: String,
    app: AppHandle,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<Vec<InstalledPlugin>, String> {
    runtime.register_development_directory(&app, &PathBuf::from(path))?;
    list_installed_plugins(&runtime, &state)
}

/// 停止指定插件的开发目录监听，并返回最新插件列表。
#[tauri::command]
pub(crate) fn stop_plugin_development(
    plugin_name: String,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<Vec<InstalledPlugin>, String> {
    plugin::validate_plugin_name(&plugin_name)?;
    runtime.stop_development_watch(&plugin_name)?;
    list_installed_plugins(&runtime, &state)
}

/// 关闭并删除指定插件，然后返回剩余插件列表。
#[tauri::command]
pub(crate) fn uninstall_plugin(
    plugin_name: String,
    remove_data: bool,
    app: AppHandle,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<Vec<InstalledPlugin>, String> {
    plugin::close_plugin_window(&app, &plugin_name)?;
    runtime.stop_development_watch(&plugin_name)?;
    runtime.uninstall(&plugin_name)?;
    if remove_data {
        state
            .store
            .lock()
            .map_err(|_| "数据库锁已损坏".to_owned())?
            .clear_plugin_data(&plugin_name)?;
    }
    list_installed_plugins(&runtime, &state)
}

/**
 * 在异步线程中启动插件指令，截图与历史贴图使用各自入口。
 * @param plugin_name 插件名称。
 * @param feature_code 功能标识。
 * @param payload 可选的启动参数。
 * @param app 桌面宿主句柄。
 * @param runtime 插件运行时状态。
 * @returns 插件功能启动结果。
 */
#[tauri::command]
pub(crate) async fn launch_plugin_feature(
    plugin_name: String,
    feature_code: String,
    payload: Option<serde_json::Value>,
    app: AppHandle,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    if plugin_name == "setting" {
        let section = match feature_code.as_str() {
            "general" | "appearance" | "data" | "plugins" | "market" | "services" => feature_code,
            _ => return Err("未知的内置设置功能".to_owned()),
        };
        // 设置插件复用主窗口原生设置页，避免再装一份前端和 Node 依赖。
        app.emit_to("main", "open-settings-section", section)
            .map_err(|error| error.to_string())?;
        crate::commands::launcher::show_main_window(&app);
        return Ok(());
    }
    if plugin_name == "screenshot" {
        if feature_code != "capture" && feature_code != "pin" {
            return Err("未知的截图插件功能".to_owned());
        }
        let main = app
            .get_window("main")
            .ok_or_else(|| "主启动器窗口不存在".to_owned())?;
        // 默认截图插件由宿主先隐藏启动器并抓取屏幕，再加载插件自带编辑界面。
        if feature_code == "pin" {
            crate::screenshot::start_history(app.clone(), main).await?;
        } else {
            crate::screenshot::start_editor(app.clone(), main).await?;
        }
        return Ok(());
    }
    if plugin_name == "system" {
        // 系统插件只转发 manifest 中声明的固定命令，由 Rust 白名单完成最终校验。
        crate::commands::system::run_system_command(feature_code, app.clone())?;
        if let Some(window) = app.get_window("main") {
            window.hide().map_err(|error| error.to_string())?;
        }
        return Ok(());
    }
    let kind = match payload.as_ref() {
        Some(serde_json::Value::Array(_)) => "files",
        Some(serde_json::Value::String(value)) if !value.is_empty() => "regex",
        _ => "text",
    };
    let action = PluginEnterAction {
        code: feature_code,
        kind: kind.to_owned(),
        payload: payload.unwrap_or(serde_json::Value::Null),
    };
    plugin::launch_plugin(&app, &runtime, &plugin_name, action)
}

/// 校验插件窗口身份后把文本写入系统剪贴板。
#[tauri::command]
pub(crate) async fn plugin_copy_text(
    content: String,
    should_paste: bool,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    let plugin_name = runtime.plugin_for_window(window.label())?;
    desktop::write_clipboard_text(content).map_err(|error| {
        eprintln!("[plugin] clipboard write failed for {plugin_name}: {error}");
        error
    })?;
    eprintln!("[plugin] clipboard write succeeded for {plugin_name}");
    if should_paste {
        // 隐藏插件窗口并稍候，让原前台应用恢复后再发送粘贴组合键。
        window.window().hide().map_err(|error| error.to_string())?;
        tauri::async_runtime::spawn_blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(120));
            desktop::simulate_paste()
        })
        .await
        .map_err(|error| format!("插件粘贴任务异常结束：{error}"))??;
    }
    Ok(())
}

/// 返回当前宿主的纯文本剪贴板历史，并按插件请求分页和筛选。
#[tauri::command]
pub(crate) fn plugin_clipboard_get_history(
    page: usize,
    page_size: usize,
    kind: Option<String>,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    runtime.plugin_for_window(window.label())?;
    if page == 0 || page_size == 0 || page_size > 100 {
        return Err("剪贴板分页参数超出范围".to_owned());
    }
    let history = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .clipboard_history(100)?;
    let accepted = kind
        .as_deref()
        .map_or(true, |value| matches!(value, "all" | "text"));
    let total = if accepted { history.len() } else { 0 };
    let start = (page - 1).saturating_mul(page_size).min(total);
    let items = if accepted {
        history
            .into_iter()
            .skip(start)
            .take(page_size)
            .map(clipboard_item)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    Ok(serde_json::json!({ "items": items, "total": total }))
}

/// 在当前宿主的纯文本剪贴板历史中执行不区分大小写的包含搜索。
#[tauri::command]
pub(crate) fn plugin_clipboard_search(
    query: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    runtime.plugin_for_window(window.label())?;
    if query.chars().count() > 500 {
        return Err("剪贴板搜索词不能超过 500 个字符".to_owned());
    }
    let normalized = query.to_lowercase();
    let history = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .clipboard_history(100)?;
    Ok(history
        .into_iter()
        .filter(|entry| entry.content.to_lowercase().contains(&normalized))
        .map(clipboard_item)
        .collect())
}

/// 删除一条宿主剪贴板历史，并通知主窗口刷新列表。
#[tauri::command]
pub(crate) fn plugin_clipboard_delete(
    id: i64,
    window: Webview,
    app: AppHandle,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    runtime.plugin_for_window(window.label())?;
    let store = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?;
    store.delete_clipboard_entry(id)?;
    let history = store.clipboard_history(100)?;
    app.emit("clipboard-history-updated", history)
        .map_err(|error| error.to_string())?;
    Ok(true)
}

/// 清空宿主剪贴板历史，并通知主窗口刷新列表。
#[tauri::command]
pub(crate) fn plugin_clipboard_clear(
    window: Webview,
    app: AppHandle,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    runtime.plugin_for_window(window.label())?;
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .clear_clipboard_history()?;
    app.emit("clipboard-history-updated", Vec::<serde_json::Value>::new())
        .map_err(|error| error.to_string())?;
    Ok(true)
}

/// 将指定历史文本重新写入剪贴板，并可隐藏插件后粘贴到原前台应用。
#[tauri::command]
pub(crate) async fn plugin_clipboard_write_history(
    id: i64,
    should_paste: bool,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    runtime.plugin_for_window(window.label())?;
    let content = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .clipboard_history(100)?
        .into_iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.content)
        .ok_or_else(|| "剪贴板历史记录不存在".to_owned())?;
    desktop::write_clipboard_text(content)?;
    if should_paste {
        window.window().hide().map_err(|error| error.to_string())?;
        tauri::async_runtime::spawn_blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(120));
            desktop::simulate_paste()
        })
        .await
        .map_err(|error| format!("插件粘贴任务异常结束：{error}"))??;
    }
    Ok(true)
}

/// 校验插件窗口身份并发送系统原生通知。
#[tauri::command]
pub(crate) fn plugin_show_notification(
    body: String,
    window: Webview,
    app: AppHandle,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    let plugin_name = runtime.plugin_for_window(window.label())?;
    if body.chars().count() > 500 {
        return Err("插件通知正文不能超过 500 个字符".to_owned());
    }
    let notification = app.notification();
    let permission = notification
        .permission_state()
        .map_err(|error| error.to_string())?;
    let permission = if matches!(
        permission,
        PermissionState::Prompt | PermissionState::PromptWithRationale
    ) {
        notification
            .request_permission()
            .map_err(|error| error.to_string())?
    } else {
        permission
    };
    if permission != PermissionState::Granted {
        return Err("系统通知权限未授予".to_owned());
    }
    notification
        .builder()
        .title(format!("ZTools · {plugin_name}"))
        .body(body)
        .show()
        .map_err(|error| error.to_string())
}

/**
 * 退出插件：内嵌视图只关闭子 Webview，独立插件则关闭整个外层窗口。
 * @param window 发起退出的插件 Webview。
 * @param runtime 插件身份注册表。
 * @param app 桌面应用句柄。
 * @returns 视图或窗口关闭完成，失败时返回原因。
 */
#[tauri::command]
pub(crate) fn plugin_out(
    window: Webview,
    runtime: State<'_, PluginRuntime>,
    app: AppHandle,
) -> Result<(), String> {
    runtime.plugin_for_window(window.label())?;
    let embedded = window.window().label() == "main";
    let plugin_name = window.label().trim_start_matches("plugin-").to_owned();
    if embedded {
        // 内嵌插件共享主窗口，只能移除自己的 Webview 并恢复搜索结果布局。
        window.close().map_err(|error| error.to_string())?;
        runtime.unregister_instance(&format!("plugin-{plugin_name}"));
        plugin::reset_embedded_layout(&app)?;
        app.emit_to("main", "plugin-panel-closed", plugin_name)
            .map_err(|error| error.to_string())?;
    } else {
        // 独立插件必须销毁顶层窗口，避免 Webview 关闭后留下空白弹窗。
        window.window().close().map_err(|error| error.to_string())?;
    }
    Ok(())
}

/**
 * 由主启动器关闭一个内嵌插件，并撤销它的 API 身份。
 * @param plugin_name 插件 manifest 名称。
 * @param window 发起请求的主 Webview。
 * @param app 桌面宿主句柄。
 * @returns 子 Webview 关闭后的结果。
 */
#[tauri::command]
pub(crate) fn close_embedded_plugin(
    plugin_name: String,
    window: Webview,
    app: AppHandle,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("只有主启动器可以关闭内嵌插件".to_owned());
    }
    plugin::close_plugin_window(&app, &plugin_name)
}

/**
 * 由主启动器把当前插件页面移到独立窗口。
 * @param plugin_name 插件 manifest 名称。
 * @param window 发起请求的主 Webview。
 * @param runtime 插件身份注册表。
 * @param app 桌面应用句柄。
 * @returns 原 Webview 分离完成，失败时返回原因。
 */
#[tauri::command]
pub(crate) fn detach_embedded_plugin(
    plugin_name: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
    app: AppHandle,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("只有主启动器可以分离插件".to_owned());
    }
    plugin::detach_embedded_plugin(&app, &runtime, &plugin_name)
}

/**
 * 在系统文件管理器中定位已安装插件目录。
 * @param plugin_name 插件 manifest 名称。
 * @param window 发起请求的主 Webview。
 * @param runtime 插件运行时根目录。
 * @returns 文件管理器打开后的结果。
 */
#[tauri::command]
pub(crate) fn reveal_plugin_directory(
    plugin_name: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("只有主启动器可以打开插件目录".to_owned());
    }
    plugin::validate_plugin_name(&plugin_name)?;
    let directory = runtime.root().join(plugin_name);
    if !directory.is_dir() {
        return Err("插件目录不存在".to_owned());
    }
    desktop::open_path(directory.to_string_lossy().into_owned())
}

/**
 * 读取已安装插件的说明和私有数据目录，只返回数据键及大小。
 * @param plugin_name 插件 manifest 名称。
 * @param window 发起请求的主 Webview。
 * @param runtime 插件安装根目录。
 * @param state SQLite 数据状态。
 * @returns README 文本与数据元信息。
 */
#[tauri::command]
pub(crate) fn get_installed_plugin_detail(
    plugin_name: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<InstalledPluginDetail, String> {
    if window.label() != "main" {
        return Err("只有主启动器可以查看插件详情".to_owned());
    }
    plugin::validate_plugin_name(&plugin_name)?;
    let directory = runtime
        .root()
        .join(&plugin_name)
        .canonicalize()
        .map_err(|_| "插件目录不存在".to_owned())?;
    // README 只允许读取安装目录内不超过 1 MB 的普通文件。
    let readme = ["README.md", "readme.md", "README.markdown"]
        .into_iter()
        .find_map(|name| {
            let path = directory.join(name).canonicalize().ok()?;
            let metadata = fs::metadata(&path).ok()?;
            if !path.starts_with(&directory) || !metadata.is_file() || metadata.len() > 1024 * 1024
            {
                return None;
            }
            fs::read_to_string(path).ok()
        })
        .unwrap_or_default();
    let data = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .plugin_data_items(&plugin_name)?;
    Ok(InstalledPluginDetail { readme, data })
}

/**
 * 返回当前已经创建的插件 Webview 所属插件名称。
 * @param window 发起请求的主 Webview。
 * @param app 桌面宿主句柄。
 * @param runtime 插件身份映射。
 * @returns 去重后的运行中插件名称。
 */
#[tauri::command]
pub(crate) fn list_running_plugins(
    window: Webview,
    app: AppHandle,
    runtime: State<'_, PluginRuntime>,
) -> Result<Vec<String>, String> {
    if window.label() != "main" {
        return Err("只有主启动器可以查看运行状态".to_owned());
    }
    let mut names = app
        .webviews()
        .into_keys()
        .filter_map(|label| runtime.plugin_for_window(&label).ok())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    Ok(names)
}

/// 隐藏插件宿主窗口，并记录随后退出时是否恢复此前的主启动器。
#[tauri::command]
pub(crate) fn plugin_hide_main_window(
    is_restore_pre_window: Option<bool>,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    runtime.set_restore_main_on_close(window.label(), is_restore_pre_window.unwrap_or(true))?;
    window.window().hide().map_err(|error| error.to_string())
}

/// 校验插件窗口身份并持久化一条插件私有 JSON 文档。
#[tauri::command]
pub(crate) fn plugin_db_put(
    document: serde_json::Value,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let plugin_name = runtime.plugin_for_window(window.label())?;
    let document_id = plugin_document_id(&document)?;
    let encoded = serde_json::to_vec(&document).map_err(|error| error.to_string())?;
    if encoded.len() > 2 * 1024 * 1024 {
        return Err("单条插件文档不能超过 2 MB".to_owned());
    }
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .put_plugin_document(&plugin_name, &document_id, &document)?;
    Ok(serde_json::json!({ "ok": true, "id": document_id }))
}

/// 校验插件窗口身份并删除一条插件私有 JSON 文档。
#[tauri::command]
pub(crate) fn plugin_db_remove(
    document_id: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let plugin_name = runtime.plugin_for_window(window.label())?;
    validate_plugin_document_id(&document_id)?;
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .remove_plugin_document(&plugin_name, &document_id)?;
    Ok(serde_json::json!({ "ok": true, "id": document_id }))
}

/// 校验插件窗口身份并保存一条插件私有键值数据。
#[tauri::command]
pub(crate) fn plugin_storage_set(
    key: String,
    value: serde_json::Value,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let plugin_name = runtime.plugin_for_window(window.label())?;
    validate_storage_key(&key)?;
    let encoded = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    if encoded.len() > 2 * 1024 * 1024 {
        return Err("单条插件键值数据不能超过 2 MB".to_owned());
    }
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .set_plugin_storage(&plugin_name, &key, &value)
}

/// 校验插件窗口身份并删除一条插件私有键值数据。
#[tauri::command]
pub(crate) fn plugin_storage_remove(
    key: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let plugin_name = runtime.plugin_for_window(window.label())?;
    validate_storage_key(&key)?;
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .remove_plugin_storage(&plugin_name, &key)
}

/// 校验并保存插件动态 feature，然后通知主启动器刷新搜索索引。
#[tauri::command]
pub(crate) fn plugin_feature_set(
    feature: serde_json::Value,
    window: Webview,
    app: AppHandle,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let plugin_name = runtime.plugin_for_window(window.label())?;
    let parsed = validate_dynamic_feature(&feature)?;
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .set_plugin_feature(&plugin_name, &parsed.code, &feature)?;
    app.emit("plugin-features-changed", &plugin_name)
        .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({ "ok": true, "code": parsed.code }))
}

/// 删除插件动态 feature，然后通知主启动器刷新搜索索引。
#[tauri::command]
pub(crate) fn plugin_feature_remove(
    code: String,
    window: Webview,
    app: AppHandle,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let plugin_name = runtime.plugin_for_window(window.label())?;
    validate_feature_code(&code)?;
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .remove_plugin_feature(&plugin_name, &code)?;
    app.emit("plugin-features-changed", &plugin_name)
        .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({ "ok": true, "code": code }))
}

/// 校验插件窗口身份并保存一份二进制附件。
#[tauri::command]
pub(crate) fn plugin_attachment_put(
    attachment_id: String,
    content_type: String,
    data: Vec<u8>,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let plugin_name = runtime.plugin_for_window(window.label())?;
    validate_plugin_document_id(&attachment_id)?;
    validate_attachment(&content_type, &data)?;
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .put_plugin_attachment(&plugin_name, &attachment_id, &content_type, &data)?;
    Ok(serde_json::json!({ "ok": true, "id": attachment_id }))
}

/// 校验插件窗口身份并读取一份二进制附件。
#[tauri::command]
pub(crate) fn plugin_attachment_get(
    attachment_id: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<Option<serde_json::Value>, String> {
    let plugin_name = runtime.plugin_for_window(window.label())?;
    validate_plugin_document_id(&attachment_id)?;
    let attachment = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .plugin_attachments(&plugin_name)?
        .into_iter()
        .find(|(id, _, _)| id == &attachment_id);
    Ok(attachment.map(|(id, content_type, data)| {
        serde_json::json!({ "id": id, "contentType": content_type, "data": data })
    }))
}

/// 校验插件窗口身份并删除一份二进制附件。
#[tauri::command]
pub(crate) fn plugin_attachment_remove(
    attachment_id: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let plugin_name = runtime.plugin_for_window(window.label())?;
    validate_plugin_document_id(&attachment_id)?;
    state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?
        .remove_plugin_attachment(&plugin_name, &attachment_id)?;
    Ok(serde_json::json!({ "ok": true, "id": attachment_id }))
}

/// 返回用户已授权路径的文件元数据。
#[tauri::command]
pub(crate) fn plugin_file_stats(
    path: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<PluginFileStats, String> {
    let path = runtime.authorized_existing_path(window.label(), &PathBuf::from(path))?;
    file_stats(&path)
}

/// 枚举用户已授权目录的直接子项，不递归读取整棵目录树。
#[tauri::command]
pub(crate) fn plugin_read_directory(
    path: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<Vec<PluginFileStats>, String> {
    let path = runtime.authorized_existing_path(window.label(), &PathBuf::from(path))?;
    if !path.is_dir() {
        return Err("读取目录 API 需要目录路径".to_owned());
    }
    let mut entries = fs::read_dir(path)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .filter_map(|entry| file_stats(&entry.path()).ok())
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .is_directory
            .cmp(&left.is_directory)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(entries)
}

/// 判断授权范围内的路径是否存在；不存在的候选路径按其父目录校验。
#[tauri::command]
pub(crate) fn plugin_path_exists(
    path: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<bool, String> {
    let path = PathBuf::from(path);
    if path.exists() {
        runtime.authorized_existing_path(window.label(), &path)?;
        Ok(true)
    } else {
        runtime.authorized_new_path(window.label(), &path)?;
        Ok(false)
    }
}

/// 在同一已授权目录中重命名文件或目录，并拒绝覆盖已有目标。
#[tauri::command]
pub(crate) fn plugin_rename_path(
    old_path: String,
    new_path: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    let source = runtime.authorized_existing_path(window.label(), &PathBuf::from(old_path))?;
    let target = runtime.authorized_new_path(window.label(), &PathBuf::from(new_path))?;
    if target.exists() {
        return Err("重命名目标已经存在".to_owned());
    }
    if source.parent() != target.parent() {
        return Err("重命名只能在原目录内进行".to_owned());
    }
    fs::rename(source, target).map_err(|error| error.to_string())
}

/// 读取用户授权的普通文件，并限制单次返回体积。
#[tauri::command]
pub(crate) fn plugin_read_file(
    path: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<Vec<u8>, String> {
    let path = runtime.authorized_existing_path(window.label(), &PathBuf::from(path))?;
    let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("读取文件 API 不接受目录".to_owned());
    }
    if metadata.len() > 16 * 1024 * 1024 {
        return Err("单次读取文件不能超过 16 MB".to_owned());
    }
    fs::read(path).map_err(|error| error.to_string())
}

/// 写入用户授权范围内的普通文件，并限制单次写入体积。
#[tauri::command]
pub(crate) fn plugin_write_file(
    path: String,
    data: Vec<u8>,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    if data.len() > 16 * 1024 * 1024 {
        return Err("单次写入文件不能超过 16 MB".to_owned());
    }
    let candidate = PathBuf::from(path);
    let path = if candidate.exists() {
        runtime.authorized_existing_path(window.label(), &candidate)?
    } else {
        runtime.authorized_new_path(window.label(), &candidate)?
    };
    if path.is_dir() {
        return Err("写入文件 API 不接受目录".to_owned());
    }
    fs::write(path, data).map_err(|error| error.to_string())
}

/// 在用户授权范围内复制普通文件，并拒绝覆盖已有目标。
#[tauri::command]
pub(crate) fn plugin_copy_path(
    source_path: String,
    target_path: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    let source = runtime.authorized_existing_path(window.label(), &PathBuf::from(source_path))?;
    if !source.is_file() {
        return Err("当前复制 API 只支持普通文件".to_owned());
    }
    let target = runtime.authorized_new_path(window.label(), &PathBuf::from(target_path))?;
    if target.exists() {
        return Err("复制目标已经存在".to_owned());
    }
    fs::copy(source, target)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// 在用户授权范围内创建目录，父目录必须已经存在且获得授权。
#[tauri::command]
pub(crate) fn plugin_create_directory(
    path: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    let candidate = PathBuf::from(path);
    if candidate.exists() {
        let existing = runtime.authorized_existing_path(window.label(), &candidate)?;
        return if existing.is_dir() {
            Ok(())
        } else {
            Err("目标已经存在且不是目录".to_owned())
        };
    }
    let target = runtime.authorized_new_path(window.label(), &candidate)?;
    fs::create_dir(target).map_err(|error| error.to_string())
}

/// 使用系统默认应用打开已授权文件、目录或 HTTP 地址。
#[tauri::command]
pub(crate) fn plugin_shell_open(
    target: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    if target.starts_with("https://") || target.starts_with("http://") {
        return desktop::open_url(target);
    }
    let path = runtime.authorized_existing_path(window.label(), &PathBuf::from(target))?;
    desktop::open_path(path.to_string_lossy().into_owned())
}

/// 在系统文件管理器中定位已授权文件或目录。
#[tauri::command]
pub(crate) fn plugin_shell_reveal(
    path: String,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    let path = runtime.authorized_existing_path(window.label(), &PathBuf::from(path))?;
    desktop::reveal_path(path.to_string_lossy().into_owned())
}

/// 为已审计的网页快开适配器请求受限图标资源，拒绝其他插件和域名。
#[tauri::command]
pub(crate) async fn plugin_http_request(
    url: String,
    headers: std::collections::HashMap<String, String>,
    max_bytes: usize,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<PluginHttpResponse, String> {
    let plugin_name = runtime.plugin_for_window(window.label())?;
    if plugin_name != "web-quick-open" {
        return Err("当前插件没有网络桥权限".to_owned());
    }
    if max_bytes == 0 || max_bytes > 2 * 1024 * 1024 {
        return Err("网络响应体限制必须介于 1 字节和 2 MB 之间".to_owned());
    }
    let parsed = reqwest::Url::parse(&url).map_err(|error| format!("URL 无效：{error}"))?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("fav.lee.cm")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("网页快开网络桥只允许访问 https://fav.lee.cm".to_owned());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.get(parsed);
    for (name, value) in headers {
        let normalized = name.to_ascii_lowercase();
        if !matches!(
            normalized.as_str(),
            "accept" | "accept-language" | "user-agent" | "accept-encoding"
        ) {
            return Err(format!("网络桥不允许请求头 {name}"));
        }
        request = request.header(name, value);
    }
    let mut response = request.send().await.map_err(|error| error.to_string())?;
    let status_code = response.status().as_u16();
    let response_headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err("网络响应超过插件声明的体积限制".to_owned());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(PluginHttpResponse {
        status_code,
        headers: response_headers,
        body,
    })
}

/// 显示系统原生文件或目录选择框，并把用户选择结果加入当前插件路径授权。
#[tauri::command]
pub(crate) async fn plugin_dialog_open(
    options: PluginDialogOptions,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<Vec<String>, String> {
    runtime.plugin_for_window(window.label())?;
    let label = window.label().to_owned();
    let selected = tauri::async_runtime::spawn_blocking(move || open_dialog(options))
        .await
        .map_err(|error| format!("文件选择任务异常结束：{error}"))??;
    let granted = runtime.grant_paths(&label, selected)?;
    Ok(granted
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect())
}

/// 显示系统原生保存选择框，并授权插件写入用户确认的目标目录。
#[tauri::command]
pub(crate) async fn plugin_dialog_save(
    options: PluginDialogOptions,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<Option<String>, String> {
    runtime.plugin_for_window(window.label())?;
    let label = window.label().to_owned();
    let selected = tauri::async_runtime::spawn_blocking(move || save_dialog(options))
        .await
        .map_err(|error| format!("保存选择任务异常结束：{error}"))??;
    let Some(path) = selected else {
        return Ok(None);
    };
    let parent = path
        .parent()
        .ok_or_else(|| "保存路径缺少父目录".to_owned())?
        .to_path_buf();
    runtime.grant_paths(&label, [parent])?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/**
 * 调整独立插件窗口客户区尺寸；内嵌插件保持主搜索工作区的固定布局。
 * @param width 请求的客户区宽度。
 * @param height 请求的客户区高度。
 * @param window 发起请求的插件 Webview。
 * @param runtime 插件身份注册表。
 * @returns 尺寸更新结果，内嵌视图返回成功而不改变宿主窗口。
 */
#[tauri::command]
pub(crate) fn plugin_window_set_size(
    width: f64,
    height: f64,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    runtime.plugin_for_window(window.label())?;
    if !(240.0..=3840.0).contains(&width) || !(160.0..=2160.0).contains(&height) {
        return Err("插件窗口尺寸超出 240×160 到 3840×2160 的范围".to_owned());
    }
    // 内嵌视图的尺寸由宿主统一控制，不能让插件覆盖搜索框或缩小自身视口。
    if window.window().label() == "main" {
        return Ok(());
    }
    window
        .window()
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|error| error.to_string())
}

/**
 * 移动独立插件窗口；内嵌插件始终固定在主搜索框下方。
 * @param x 请求的横坐标。
 * @param y 请求的纵坐标。
 * @param window 发起请求的插件 Webview。
 * @param runtime 插件身份注册表。
 * @returns 位置更新结果，内嵌视图返回成功而不改变宿主布局。
 */
#[tauri::command]
pub(crate) fn plugin_window_set_position(
    x: f64,
    y: f64,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    runtime.plugin_for_window(window.label())?;
    if !x.is_finite() || !y.is_finite() {
        return Err("插件窗口坐标必须是有限数值".to_owned());
    }
    // 主窗口由启动器控制位置，插件只能移动自己的独立窗口。
    if window.window().label() == "main" {
        return Ok(());
    }
    window
        .window()
        .set_position(tauri::LogicalPosition::new(x, y))
        .map_err(|error| error.to_string())
}

/// 把当前插件窗口移动到所在桌面的中心。
#[tauri::command]
pub(crate) fn plugin_window_center(
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    runtime.plugin_for_window(window.label())?;
    window.window().center().map_err(|error| error.to_string())
}

/// 设置当前插件窗口置顶状态。
#[tauri::command]
pub(crate) fn plugin_window_set_always_on_top(
    enabled: bool,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    runtime.plugin_for_window(window.label())?;
    window
        .window()
        .set_always_on_top(enabled)
        .map_err(|error| error.to_string())
}

/// 设置当前插件窗口最小化状态。
#[tauri::command]
pub(crate) fn plugin_window_set_minimized(
    minimized: bool,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    runtime.plugin_for_window(window.label())?;
    if minimized {
        window
            .window()
            .minimize()
            .map_err(|error| error.to_string())
    } else {
        window
            .window()
            .unminimize()
            .map_err(|error| error.to_string())
    }
}

/// 设置当前插件窗口最大化状态。
#[tauri::command]
pub(crate) fn plugin_window_set_maximized(
    maximized: bool,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    runtime.plugin_for_window(window.label())?;
    if maximized {
        window
            .window()
            .maximize()
            .map_err(|error| error.to_string())
    } else {
        window
            .window()
            .unmaximize()
            .map_err(|error| error.to_string())
    }
}

/// 设置当前插件窗口全屏状态。
#[tauri::command]
pub(crate) fn plugin_window_set_fullscreen(
    fullscreen: bool,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    runtime.plugin_for_window(window.label())?;
    window
        .window()
        .set_fullscreen(fullscreen)
        .map_err(|error| error.to_string())
}

/// 校验插件身份后截取当前鼠标所在显示器，并返回 PNG 字节。
#[tauri::command]
pub(crate) async fn plugin_screen_capture(
    window: Webview,
    app: AppHandle,
    runtime: State<'_, PluginRuntime>,
) -> Result<Vec<u8>, String> {
    runtime.plugin_for_window(window.label())?;
    let capture_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path = desktop::capture_screen(&capture_handle)?;
        let result = (|| {
            let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
            if metadata.len() > 32 * 1024 * 1024 {
                return Err("插件截图超过 32 MB 限制".to_owned());
            }
            fs::read(&path).map_err(|error| error.to_string())
        })();
        // 临时截图已经编码进返回值，清理失败不覆盖读取结果。
        let _ = fs::remove_file(path);
        result
    })
    .await
    .map_err(|error| format!("插件截图任务异常结束：{error}"))?
}

/// 校验插件身份后输入有限长度文本，可先隐藏插件以把输入送回原前台窗口。
#[tauri::command]
pub(crate) async fn plugin_input_type_text(
    content: String,
    target_external: bool,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    runtime.plugin_for_window(window.label())?;
    if content.chars().count() > 100_000 {
        return Err("单次模拟输入不能超过 100000 个字符".to_owned());
    }
    if target_external {
        window.window().hide().map_err(|error| error.to_string())?;
    }
    tauri::async_runtime::spawn_blocking(move || {
        if target_external {
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        desktop::type_text(&content)
    })
    .await
    .map_err(|error| format!("插件文本输入任务异常结束：{error}"))?
}

/// 校验插件身份后点击一枚受白名单约束的基础键。
#[tauri::command]
pub(crate) async fn plugin_input_tap_key(
    key: String,
    target_external: bool,
    window: Webview,
    runtime: State<'_, PluginRuntime>,
) -> Result<(), String> {
    runtime.plugin_for_window(window.label())?;
    if target_external {
        window.window().hide().map_err(|error| error.to_string())?;
    }
    tauri::async_runtime::spawn_blocking(move || {
        if target_external {
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        desktop::tap_key(&key)
    })
    .await
    .map_err(|error| format!("插件按键任务异常结束：{error}"))?
}

/// 根据插件选项构造阻塞式原生打开对话框，供异步 command 在线程池调用。
fn open_dialog(options: PluginDialogOptions) -> Result<Vec<PathBuf>, String> {
    let dialog = configure_dialog(&options);
    let directory = options
        .properties
        .iter()
        .any(|property| property == "openDirectory");
    let multiple = options
        .properties
        .iter()
        .any(|property| property == "multiSelections");
    let selected = match (directory, multiple) {
        (true, true) => dialog.pick_folders().unwrap_or_default(),
        (true, false) => dialog.pick_folder().into_iter().collect(),
        (false, true) => dialog.pick_files().unwrap_or_default(),
        (false, false) => dialog.pick_file().into_iter().collect(),
    };
    Ok(selected)
}

/// 根据插件选项构造阻塞式原生保存对话框，取消时返回空结果。
fn save_dialog(options: PluginDialogOptions) -> Result<Option<PathBuf>, String> {
    let mut dialog = configure_dialog(&options);
    if !options.default_name.trim().is_empty() {
        dialog = dialog.set_file_name(options.default_name);
    }
    Ok(dialog.save_file())
}

/// 把标题、初始目录和扩展名过滤器应用到原生文件对话框。
fn configure_dialog(options: &PluginDialogOptions) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new();
    if !options.title.trim().is_empty() {
        dialog = dialog.set_title(&options.title);
    }
    if !options.default_path.trim().is_empty() {
        dialog = dialog.set_directory(&options.default_path);
    }
    for filter in &options.filters {
        if !filter.name.trim().is_empty() && !filter.extensions.is_empty() {
            dialog = dialog.add_filter(&filter.name, &filter.extensions);
        }
    }
    dialog
}

/// 把文件系统元数据转换为插件可序列化的稳定结构。
fn file_stats(path: &std::path::Path) -> Result<PluginFileStats, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    let to_millis = |value: Result<std::time::SystemTime, std::io::Error>| {
        value
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default()
    };
    Ok(PluginFileStats {
        name: path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        path: path.to_string_lossy().into_owned(),
        is_file: metadata.is_file(),
        is_directory: metadata.is_dir(),
        size: metadata.len(),
        modified_at: to_millis(metadata.modified()),
        created_at: to_millis(metadata.created()),
    })
}

/// 合并插件 manifest 和 SQLite 中的动态 feature，供启动器建立完整搜索索引。
fn list_installed_plugins(
    runtime: &PluginRuntime,
    state: &AppState,
) -> Result<Vec<InstalledPlugin>, String> {
    let mut plugins = runtime.installed_plugins()?;
    let store = state
        .store
        .lock()
        .map_err(|_| "数据库锁已损坏".to_owned())?;
    for installed in &mut plugins {
        for value in store.plugin_features(&installed.name)? {
            let feature = validate_dynamic_feature(&value)?;
            if let Some(position) = installed
                .features
                .iter()
                .position(|candidate| candidate.code == feature.code)
            {
                installed.features[position] = feature;
            } else {
                installed.features.push(feature);
            }
        }
    }
    Ok(plugins)
}

/// 校验插件键值存储键，避免无界键名和控制字符进入数据库。
fn validate_storage_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.chars().count() > 256 || key.chars().any(char::is_control) {
        return Err("插件存储键不能为空、不能超过 256 字符或包含控制字符".to_owned());
    }
    Ok(())
}

/// 把动态 feature JSON 解码为受约束模型，并验证可进入启动器索引的字段。
fn validate_dynamic_feature(feature: &serde_json::Value) -> Result<PluginFeature, String> {
    let encoded = serde_json::to_vec(feature).map_err(|error| error.to_string())?;
    if encoded.len() > 256 * 1024 {
        return Err("动态 feature 不能超过 256 KB".to_owned());
    }
    let parsed: PluginFeature = serde_json::from_value(feature.clone())
        .map_err(|error| format!("动态 feature 格式无效：{error}"))?;
    validate_feature_code(&parsed.code)?;
    if parsed.cmds.len() > 100 {
        return Err("单个动态 feature 不能声明超过 100 条命令".to_owned());
    }
    Ok(parsed)
}

/// 校验动态 feature code 的长度和字符边界。
fn validate_feature_code(code: &str) -> Result<(), String> {
    if code.trim().is_empty() || code.chars().count() > 160 || code.chars().any(char::is_control) {
        return Err("动态 feature code 不能为空、不能超过 160 字符或包含控制字符".to_owned());
    }
    Ok(())
}

/// 限制附件类型和体积，防止同步镜像在插件启动时无界占用内存。
fn validate_attachment(content_type: &str, data: &[u8]) -> Result<(), String> {
    if content_type.is_empty()
        || content_type.chars().count() > 128
        || content_type.chars().any(char::is_control)
    {
        return Err("附件 MIME 类型不能为空、不能超过 128 字符或包含控制字符".to_owned());
    }
    if data.len() > 16 * 1024 * 1024 {
        return Err("单份插件附件不能超过 16 MB".to_owned());
    }
    Ok(())
}

/// 从插件文档对象提取并校验 `_id` 字段。
fn plugin_document_id(document: &serde_json::Value) -> Result<String, String> {
    let document_id = document
        .as_object()
        .and_then(|object| object.get("_id"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "插件文档必须包含字符串 _id".to_owned())?;
    validate_plugin_document_id(document_id)?;
    Ok(document_id.to_owned())
}

/// 限制插件文档标识长度和控制字符，避免异常索引键进入 SQLite。
fn validate_plugin_document_id(document_id: &str) -> Result<(), String> {
    if document_id.is_empty()
        || document_id.len() > 512
        || document_id.chars().any(char::is_control)
    {
        return Err("插件文档 _id 必须为 1 到 512 字节且不能包含控制字符".to_owned());
    }
    Ok(())
}
