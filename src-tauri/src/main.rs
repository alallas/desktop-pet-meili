#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![set_click_through, get_cursor_position, debug_log])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[tauri::command]
fn set_click_through(through: bool, window: tauri::Window) {
    let _ = window.set_ignore_cursor_events(through);
}

// M0-2：点击穿透下页面收不到鼠标事件，由 Rust 侧轮询全局光标位置
#[tauri::command]
fn get_cursor_position(app: tauri::AppHandle) -> Option<(f64, f64)> {
    app.cursor_position().ok().map(|p| (p.x, p.y))
}

// 临时调试：前端状态写文件（截图不可靠时排查用）
#[tauri::command]
fn debug_log(msg: String) {
    use std::io::Write;
    let path = std::env::temp_dir().join("chaojiandai-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}", msg);
    }
}
