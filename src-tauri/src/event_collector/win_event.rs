//! 前台窗口采集器：SetWinEventHook 挂「前台切换 + 窗口位置变化」。
//!
//! - M0-9 已验证钩子投递延迟 0–16ms（消息循环线程），满足 <40ms 行为预算；
//! - 事件常量本文件自带数值（0x0003 / 0x800B / OBJID_WINDOW=0）：winuser.h 的常量分散在
//!   windows-sys 多个 feature 模块里且随版本漂移，本地定义避免 import 路径不确定；
//! - 不挂 WINEVENT_SKIPOWNPROCESS：自窗口的 LOCATIONCHANGE 也要收（SelfWindowMoved 事件），
//!   拖动洪峰由总线传输级削峰兜住。

use std::sync::atomic::{AtomicIsize, Ordering};

use tauri::{AppHandle, Manager};
use windows_sys::Win32::Foundation::{CloseHandle, HWND};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetMessageW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, MSG,
};

use crate::event_bus::EventBus;
use crate::ipc::{EventKind, EventSource};

const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
const EVENT_OBJECT_LOCATIONCHANGE: u32 = 0x800B;
const OBJID_WINDOW: i32 = 0;
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;

/// 回调共享上下文。windows-sys 0.61 的 WINEVENTPROC **不含 dwContext 参数**
///（只有 7 参的文档签名），无法经钩子安装传参，改为进程内单例 static。
/// 全局只有一个采集器实例，OnceLock 语义恰好吻合；进程退出钩子自动失效。
struct Ctx {
    bus: EventBus,
    self_hwnd: AtomicIsize,
}

static CTX: std::sync::OnceLock<Ctx> = std::sync::OnceLock::new();

pub struct WinEventCollector {
    self_hwnd: isize,
}

impl WinEventCollector {
    pub fn new(app: AppHandle) -> Self {
        let self_hwnd = app
            .get_webview_window("main")
            .and_then(|w| w.hwnd().map(|h| h.0 as isize).ok())
            .unwrap_or(0);
        Self { self_hwnd }
    }
}

impl super::Collector for WinEventCollector {
    fn start(&self, bus: EventBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = CTX.get_or_init(|| Ctx {
            bus,
            self_hwnd: AtomicIsize::new(self.self_hwnd),
        });
        std::thread::spawn(move || unsafe {
            let hooks = [
                SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    std::ptr::null_mut(),
                    Some(callback),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                ),
                SetWinEventHook(
                    EVENT_OBJECT_LOCATIONCHANGE,
                    EVENT_OBJECT_LOCATIONCHANGE,
                    std::ptr::null_mut(),
                    Some(callback),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                ),
            ];
            if hooks.iter().any(|h| h.is_null()) {
                tracing::error!("SetWinEventHook failed: {hooks:?}");
            }
            // OUTOFCONTEXT 钩子的回调在**本线程的消息循环**上执行：必须保活 GetMessage。
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
            for h in hooks {
                if !h.is_null() {
                    UnhookWinEvent(h);
                }
            }
        });
        Ok(())
    }

    fn name(&self) -> &'static str {
        "win_event"
    }
}

/// WINEVENTPROC（windows-sys 0.61 的 7 参签名；真 ABI 会多塞一个 dwContext，
/// 按 7 参编译是安全的——多出的实参被忽略）。
unsafe extern "system" fn callback(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if hwnd.is_null() || id_object != OBJID_WINDOW {
        return;
    }
    let Some(ctx) = CTX.get() else {
        return;
    };
    match event {
        EVENT_SYSTEM_FOREGROUND => {
            let payload = serde_json::json!({
                "app": window_app_name(hwnd),
                "title": window_title(hwnd),
            });
            ctx.bus.publish(ctx.bus.event(
                EventSource::ForegroundWindow,
                EventKind::ForegroundSwitched,
                payload,
            ));
        }
        EVENT_OBJECT_LOCATIONCHANGE => {
            let self_hwnd = ctx.self_hwnd.load(Ordering::Relaxed);
            if hwnd as isize == self_hwnd {
                let mut rect = windows_sys::Win32::Foundation::RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                if GetWindowRect(hwnd, &mut rect) != 0 {
                    ctx.bus.publish(ctx.bus.event(
                        EventSource::WindowTrack,
                        EventKind::SelfWindowMoved,
                        serde_json::json!({ "x": rect.left, "y": rect.top }),
                    ));
                }
            } else if hwnd == GetForegroundWindow() {
                let mut rect = windows_sys::Win32::Foundation::RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                if GetWindowRect(hwnd, &mut rect) != 0 {
                    ctx.bus.publish(ctx.bus.event(
                        EventSource::WindowTrack,
                        EventKind::ForegroundMoved,
                        serde_json::json!({
                            "app": window_app_name(hwnd),
                            "rect": {
                                "left": rect.left,
                                "top": rect.top,
                                "right": rect.right,
                                "bottom": rect.bottom,
                            },
                        }),
                    ));
                }
            }
            // 其余 LOCATIONCHANGE（非前台、非自窗口）静默丢弃：桌面有几十个可见窗口，
            // 全发会把事件总线变成噪音源。
        }
        _ => {}
    }
}

/// 进程 exe 的短名（WeChat.exe → wechat）。打不开进程时返回 "unknown"。
unsafe fn window_app_name(hwnd: HWND) -> String {
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if proc.is_null() {
        return "unknown".to_owned();
    }
    let mut buf = [0u16; 260];
    let mut len = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(proc, 0, buf.as_mut_ptr(), &mut len);
    CloseHandle(proc);
    if ok == 0 || len == 0 {
        return "unknown".to_owned();
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_owned()
}

unsafe fn window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}
