//! 会话采集器：WTSRegisterSessionNotification 收锁屏/解锁（架构 §3.1 事件驱动）。
//!
//! 需要一个 HWND 挂通知，用 message-only 窗口（不可见、不占屏幕、不收桌面输入）。
//! 锁屏事件同时在解锁瞬间补发基线（is_idle 语义由行为引擎组合，这里只发原始翻转）。

use windows_sys::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, RegisterClassW,
    TranslateMessage, MSG, WNDCLASSW,
};

use super::Collector;
use crate::event_bus::EventBus;
use crate::ipc::{EventKind, EventSource};

const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WTS_SESSION_LOCK: usize = 0x7;
const WTS_SESSION_UNLOCK: usize = 0x8;
/// NOTIFY_FOR_ALL_SESSIONS（wtsapi32 常量，稳定值 1）
const NOTIFY_FOR_ALL_SESSIONS: u32 = 1;
/// HWND_MESSAGE：message-only 窗口的父句柄
const HWND_MESSAGE: isize = -3;

fn class_name() -> Vec<u16> {
    "chaojiandai_session\0".encode_utf16().collect()
}

static BUS: std::sync::OnceLock<EventBus> = std::sync::OnceLock::new();

pub struct SessionCollector;

impl Collector for SessionCollector {
    fn start(&self, bus: EventBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = BUS.get_or_init(|| bus);
        std::thread::spawn(move || unsafe {
            let class = class_name();
            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: std::ptr::null_mut(),
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class.as_ptr(),
            };
            if RegisterClassW(&wc) == 0 {
                tracing::error!("RegisterClassW(session) failed");
                return;
            }
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE as _,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            if hwnd.is_null() {
                tracing::error!("CreateWindowExW(message-only) failed");
                return;
            }
            if WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_ALL_SESSIONS) == 0 {
                tracing::error!("WTSRegisterSessionNotification failed");
                DestroyWindow(hwnd);
                return;
            }
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            WTSUnRegisterSessionNotification(hwnd);
            DestroyWindow(hwnd);
        });
        Ok(())
    }

    fn name(&self) -> &'static str {
        "session"
    }
}

unsafe extern "system" fn wndproc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    if msg == WM_WTSSESSION_CHANGE {
        if let Some(bus) = BUS.get() {
            let locked = match wparam {
                WTS_SESSION_LOCK => Some(true),
                WTS_SESSION_UNLOCK => Some(false),
                _ => None,
            };
            if let Some(locked) = locked {
                bus.publish(bus.event(
                    EventSource::Session,
                    EventKind::LockStateChanged,
                    serde_json::json!({ "locked": locked }),
                ));
            }
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
