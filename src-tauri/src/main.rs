#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 启动 ZTools 的 Tauri 桌面宿主。
fn main() {
    ztools_tauri_lib::run()
}
