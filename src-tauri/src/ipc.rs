//! IPC 命令定义（工程规范 §1：全部 IPC 集中于此，specta 导出源）。
//! 协议改动必须人审（工程规范 §5.4）。
//!
//! M1-1 骨架：先收纳 M0 遗产命令；ping-pong 样例与 specta 类型导出在协议定版时补齐。

/// 点击穿透开关（M0 验证命令，产品化后由窗口管理器接管）
#[tauri::command]
pub fn set_click_through(through: bool, window: tauri::Window) {
    let _ = window.set_ignore_cursor_events(through);
}

/// 点击穿透下页面收不到鼠标事件，由 Rust 侧查询全局光标位置（M0-2）
#[tauri::command]
pub fn get_cursor_position(app: tauri::AppHandle) -> Option<(f64, f64)> {
    app.cursor_position().ok().map(|p| (p.x, p.y))
}

/// 临时调试：前端状态写文件（截图不可靠时排查用，日志落 `%TEMP%\chaojiandai-debug.log`）
#[tauri::command]
pub fn debug_log(msg: String) {
    use std::io::Write;
    let path = std::env::temp_dir().join("chaojiandai-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{}", msg);
    }
}
