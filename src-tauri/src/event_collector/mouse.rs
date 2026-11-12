//! 全局鼠标采集器：WH_MOUSE_LL 低级钩子，只报按键按下。
//!
//! - 光标位置本身由前端轮询 get_cursor_position（M0-2 已验证），本采集器的增量信息是
//!   "用户在哪点了鼠标"（行为引擎的"试图捕捉光标"/点击桌宠交互的输入源）；
//! - 消息常量本地定义（WH_MOUSE_LL=14 / WM_*BUTTONDOWN），winuser.h 常量在 windows-sys
//!   模块间分布随版本漂移（同 win_event.rs 的理由）；
//! - 架构 §9 风险表：低级钩子是杀软敏感点，只挂不按频率上报（不监听 MOUSEMOVE），
//!   消息循环不做事，降低被误判风险。

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, MSG,
};

use super::Collector;
use crate::event_bus::EventBus;
use crate::ipc::{EventKind, EventSource};

const WH_MOUSE_LL: i32 = 14;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_MBUTTONDOWN: u32 = 0x0207;

/// MSLLHOOKSTRUCT（winuser.h）：低级鼠标钩子的 lparam 指向结构。
/// 本地定义只取前两个字段，布局稳定。
#[repr(C)]
struct Msllhookstruct {
    pt_x: i32,
    pt_y: i32,
    _rest: [u8; 32], // mouseData(4) + flags(4) + time(4) + dwExtraInfo(usize=8) + 对齐余量
}

static BUS: std::sync::OnceLock<EventBus> = std::sync::OnceLock::new();

pub struct MouseCollector;

impl Collector for MouseCollector {
    fn start(&self, bus: EventBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = BUS.get_or_init(|| bus);
        std::thread::spawn(move || unsafe {
            let hook: HHOOK =
                SetWindowsHookExW(WH_MOUSE_LL, Some(callback), std::ptr::null_mut(), 0);
            if hook.is_null() {
                tracing::error!("SetWindowsHookExW(WH_MOUSE_LL) failed");
                return;
            }
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
            UnhookWindowsHookEx(hook);
        });
        Ok(())
    }

    fn name(&self) -> &'static str {
        "mouse"
    }
}

unsafe extern "system" fn callback(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if ncode >= 0 {
        let button = match wparam as u32 {
            WM_LBUTTONDOWN => Some("left"),
            WM_RBUTTONDOWN => Some("right"),
            WM_MBUTTONDOWN => Some("middle"),
            _ => None,
        };
        if let Some(button) = button {
            let info = &*(lparam as *const Msllhookstruct);
            if let Some(bus) = BUS.get() {
                bus.publish(bus.event(
                    EventSource::Mouse,
                    EventKind::MouseClicked,
                    serde_json::json!({ "button": button, "x": info.pt_x, "y": info.pt_y }),
                ));
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam)
}
