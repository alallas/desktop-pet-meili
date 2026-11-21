//! 调试面板全局热键（M1-5 场景模拟器配套）。
//!
//! 背景：主窗口是 skipTaskbar + 不聚焦 + 点击穿透的全屏覆盖层（产品形态定的），
//! 键盘焦点永远到不了 WebView——前端所有按键快捷键（T/M/Q/N）都不可达。
//! 调试工具必须能从任何焦点状态呼出，所以热键只能注册在 Rust 侧
//! （RegisterHotKey 系统级，不经窗口焦点）。
//!
//! Ctrl+Alt+N（刻意避开 Ctrl+Shift+N——那是 VS Code 新窗口 / Chrome 无痕的快捷键）。
//! 状态权威在 Rust（AtomicBool），前端只按事件同步显示，避免两边漂移。

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{Emitter, Manager};
use tauri_specta::Event as _; // SimPanel::NAME
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, MOD_ALT, MOD_CONTROL};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

use crate::ipc::SimPanel;

/// 面板开关状态（true = 面板开着、点击穿透关）
static PANEL_OPEN: AtomicBool = AtomicBool::new(false);

pub fn start(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let ok = unsafe { RegisterHotKey(None, 1, MOD_CONTROL | MOD_ALT, u32::from('N')) };
        if ok.is_err() {
            tracing::error!("debug hotkey register failed (Ctrl+Alt+N 可能被占用)");
            return;
        }
        tracing::info!("debug hotkey registered: Ctrl+Alt+N");
        let mut msg = MSG::default();
        loop {
            // 阻塞取消息；进程退出时线程随主进程终止
            if unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 <= 0 {
                break;
            }
            if msg.message != WM_HOTKEY || msg.wParam.0 != 1 {
                continue;
            }
            let open = !PANEL_OPEN.fetch_xor(true, Ordering::SeqCst);
            let win = app.get_webview_window("main");
            if let Some(win) = win {
                let _ = win.set_ignore_cursor_events(!open);
            }
            // 不用 tauri_specta 的 Event::emit（与 Booted 同款路径，实机验证这两种
            // specta 事件都到不了前端，而 EventStream 的裸 Emitter::emit 正常——
            // 故走与 EventStream 完全相同的裸发路径，事件名与载荷格式保持一致）
            if let Err(e) = app.emit(SimPanel::NAME, open) {
                tracing::warn!("sim panel event emit failed: {e}");
            }
            tracing::info!(open, "sim panel toggled via hotkey");
        }
    });
}
