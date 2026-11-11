#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let win = app.get_webview_window("main").unwrap();
            // M0 验证：默认点击穿透，悬浮层不阻塞桌面操作
            let _ = win.set_ignore_cursor_events(true);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![set_click_through])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[tauri::command]
fn set_click_through(through: bool, window: tauri::Window) {
    let _ = window.set_ignore_cursor_events(through);
}
