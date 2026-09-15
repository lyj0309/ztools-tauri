use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "linux")]
use std::fs;

use walkdir::WalkDir;

use crate::models::{AppEntry, LocalShortcut};

/// 扫描当前操作系统公开的应用入口并按名称去重排序。
pub(crate) fn scan_applications() -> Vec<AppEntry> {
    let mut apps = platform_applications();
    let mut seen = HashSet::new();

    // 统一过滤重复入口，避免系统级与用户级菜单显示同一个目标。
    apps.retain(|app| seen.insert((app.name.to_lowercase(), app.path.clone())));
    apps.sort_by_key(|app| app.name.to_lowercase());
    apps
}

/// 将已验证的应用入口提交给操作系统启动器。
pub(crate) fn launch(app: &AppEntry) -> Result<(), String> {
    platform_launch(app)
}

/// 将本地文件、目录或应用启动项转换为统一搜索记录。
pub(crate) fn local_shortcut_entry(shortcut: &LocalShortcut) -> AppEntry {
    let display_name = if shortcut.alias.trim().is_empty() {
        shortcut.name.clone()
    } else {
        shortcut.alias.clone()
    };
    AppEntry {
        id: shortcut.id.clone(),
        name: display_name,
        path: shortcut.path.clone(),
        keywords: vec![shortcut.name.clone(), shortcut.alias.clone()],
        source: format!("local-{}", shortcut.kind),
    }
}

/// 根据稳定路径构造无需额外注册表的应用标识。
fn app_id(path: &str) -> String {
    // FNV-1a 参数固定，升级 Rust 工具链不会使已收藏的应用标识失效。
    let hash = path
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("app-{hash:016x}")
}

/// 根据路径生成本地启动项的稳定标识。
pub(crate) fn local_shortcut_id(path: &str) -> String {
    let hash = path
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("local-{hash:016x}")
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
/// 从路径末尾生成缺少元数据时可展示的应用名称。
fn fallback_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Application")
        .trim_end_matches(".app")
        .to_owned()
}

#[cfg(target_os = "linux")]
/// 扫描 Freedesktop 应用目录中的可见桌面入口。
fn platform_applications() -> Vec<AppEntry> {
    let mut roots = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
    ];

    // 用户应用目录应覆盖系统入口并参与同一轮去重。
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        roots.insert(0, PathBuf::from(data_home).join("applications"));
    } else if let Some(home) = dirs::home_dir() {
        roots.insert(0, home.join(".local/share/applications"));
    }

    roots
        .into_iter()
        .flat_map(|root| WalkDir::new(root).max_depth(3).into_iter().flatten())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|value| value == "desktop")
        })
        .filter_map(|entry| parse_desktop_entry(entry.path()))
        .collect()
}

#[cfg(target_os = "linux")]
/// 解析单个 Desktop Entry，并排除隐藏或不可启动的记录。
fn parse_desktop_entry(path: &Path) -> Option<AppEntry> {
    let content = fs::read_to_string(path).ok()?;
    let mut in_desktop_section = false;
    let mut name = None;
    let mut exec = None;
    let mut keywords = Vec::new();
    let mut hidden = false;
    let mut no_display = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') {
            in_desktop_section = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_section || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "Name" => name = Some(value.trim().to_owned()),
            "Exec" => exec = Some(value.trim().to_owned()),
            "Keywords" => {
                keywords.extend(
                    value
                        .split(';')
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(str::to_owned),
                );
            }
            "Hidden" => hidden = value.eq_ignore_ascii_case("true"),
            "NoDisplay" => no_display = value.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }

    // 必须同时具有展示名称和执行声明，隐藏菜单项不进入启动器。
    let name = name?;
    exec?;
    if hidden || no_display {
        return None;
    }
    let path = path.to_string_lossy().into_owned();
    Some(AppEntry {
        id: app_id(&path),
        name,
        path,
        keywords,
        source: "desktop-entry".to_owned(),
    })
}

#[cfg(target_os = "linux")]
/// 使用桌面环境自身的解析器启动 Desktop Entry。
fn platform_launch(app: &AppEntry) -> Result<(), String> {
    let status = Command::new("gio")
        .args(["launch", &app.path])
        .status()
        .map_err(|error| format!("无法启动 {}：{error}", app.name))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} 启动失败，系统返回 {status}", app.name))
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::parse_desktop_entry;
    use std::{fs, path::PathBuf};

    /// 创建隔离 Desktop Entry 并验证可见应用元数据解析。
    #[test]
    fn parses_visible_desktop_entry() {
        let path = fixture_path("visible");
        fs::write(
            &path,
            "[Desktop Entry]\nName=Demo App\nExec=demo %U\nKeywords=demo;sample;\n",
        )
        .expect("fixture should be writable");

        let app = parse_desktop_entry(&path).expect("entry should be visible");
        assert_eq!(app.name, "Demo App");
        assert_eq!(app.keywords, ["demo", "sample"]);
        assert_eq!(app.source, "desktop-entry");

        fs::remove_file(path).expect("fixture should be removable");
    }

    /// 验证 NoDisplay 入口不会泄露到启动器结果。
    #[test]
    fn skips_hidden_desktop_entry() {
        let path = fixture_path("hidden");
        fs::write(
            &path,
            "[Desktop Entry]\nName=Hidden App\nExec=hidden\nNoDisplay=true\n",
        )
        .expect("fixture should be writable");

        assert!(parse_desktop_entry(&path).is_none());

        fs::remove_file(path).expect("fixture should be removable");
    }

    /// 为并行测试构造进程内唯一的临时文件路径。
    fn fixture_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ztools-{name}-{}-{:?}.desktop",
            std::process::id(),
            std::thread::current().id()
        ))
    }
}

#[cfg(target_os = "macos")]
/// 扫描 macOS 标准应用目录中的应用包。
fn platform_applications() -> Vec<AppEntry> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }

    roots
        .into_iter()
        .flat_map(|root| WalkDir::new(root).max_depth(3).into_iter().flatten())
        .filter(|entry| entry.path().extension().is_some_and(|value| value == "app"))
        .map(|entry| {
            let path = entry.path().to_string_lossy().into_owned();
            AppEntry {
                id: app_id(&path),
                name: fallback_name(entry.path()),
                path,
                keywords: Vec::new(),
                source: "application-bundle".to_owned(),
            }
        })
        .collect()
}

#[cfg(target_os = "macos")]
/// 使用 Launch Services 打开 macOS 应用包。
fn platform_launch(app: &AppEntry) -> Result<(), String> {
    let status = Command::new("open")
        .args(["-a", &app.path])
        .status()
        .map_err(|error| format!("无法启动 {}：{error}", app.name))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} 启动失败，系统返回 {status}", app.name))
    }
}

#[cfg(target_os = "windows")]
/// 扫描 Windows 当前用户和所有用户的开始菜单快捷方式。
fn platform_applications() -> Vec<AppEntry> {
    let mut roots = Vec::new();
    if let Some(data) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(data).join("Microsoft/Windows/Start Menu/Programs"));
    }
    if let Some(data) = std::env::var_os("PROGRAMDATA") {
        roots.push(PathBuf::from(data).join("Microsoft/Windows/Start Menu/Programs"));
    }

    roots
        .into_iter()
        .flat_map(|root| WalkDir::new(root).max_depth(8).into_iter().flatten())
        .filter(|entry| entry.path().extension().is_some_and(|value| value == "lnk"))
        .map(|entry| {
            let path = entry.path().to_string_lossy().into_owned();
            AppEntry {
                id: app_id(&path),
                name: fallback_name(entry.path()),
                path,
                keywords: Vec::new(),
                source: "start-menu".to_owned(),
            }
        })
        .collect()
}

#[cfg(target_os = "windows")]
/// 通过 Windows Shell 打开开始菜单快捷方式。
fn platform_launch(app: &AppEntry) -> Result<(), String> {
    let status = Command::new("cmd")
        .args(["/C", "start", "", &app.path])
        .status()
        .map_err(|error| format!("无法启动 {}：{error}", app.name))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} 启动失败，系统返回 {status}", app.name))
    }
}
