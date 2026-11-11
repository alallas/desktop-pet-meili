#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! 装配层 only（工程规范 §1）：plugin/路由注册与启动顺序，不放逻辑。

mod cloud;
mod db;
mod debounce;
mod event_bus;
mod event_collector;
mod hooks;
mod ipc;
mod llm;
mod perception;
mod settings;
mod voice;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let win = app.get_webview_window("main").unwrap();
            // 窗口铺满主显示器（避免写死分辨率，兼容不同屏幕/DPI）
            if let Ok(Some(monitor)) = app.primary_monitor() {
                let size = monitor.size();
                let pos = monitor.position();
                let _ = win.set_size(tauri::Size::Physical(*size));
                let _ = win.set_position(tauri::Position::Physical(*pos));
            }
            // M0 验证：默认点击穿透，悬浮层不阻塞桌面操作
            let _ = win.set_ignore_cursor_events(true);
            // M1-1：存档库（八张表，架构 §3.8；目录为 %APPDATA%/<identifier>/saves/）
            let save_dir = app.path().app_data_dir()?.join("saves");
            std::fs::create_dir_all(&save_dir).expect("create save dir");
            let conn = db::open(&save_dir.join("save.db")).expect("open save db");
            // rusqlite::Connection 是 Send 但非 Sync，Mutex 包装后共享（IPC 多线程访问的正确姿势）
            app.manage(std::sync::Mutex::new(conn));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::set_click_through,
            ipc::get_cursor_position,
            ipc::debug_log
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
