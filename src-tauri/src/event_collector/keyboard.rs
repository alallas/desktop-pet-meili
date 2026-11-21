//! 击键计数采集器：闸①"打字密集"的输入源（WH_KEYBOARD_LL 低级钩子）。
//!
//! **隐私红线：只计数，不上报事件。** 键值/扫描码/目标窗口一概不记录——
//! 钩子回调里不解引用 KBDLLHOOKSTRUCT 任何字段，本模块也不 publish 事件
//! （行为引擎直接读计数窗口，事件总线与日志里都看不到击键痕迹）。
//! 结构同 mouse.rs：消息常量本地定义（WH_KEYBOARD_LL=13 / WM_KEYDOWN 0x100 /
//! WM_SYSKEYDOWN 0x104），只报按下、消息循环不做事。

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, MSG,
};

use super::Collector;

const WH_KEYBOARD_LL: i32 = 13;
const WM_KEYDOWN: u32 = 0x0100;
const WM_SYSKEYDOWN: u32 = 0x0104;

/// 击键时间戳滑动窗（容量自然受 10min 窗约束；闸①查询时顺带清理过期项）
static WINDOW: OnceLock<Mutex<VecDeque<Instant>>> = OnceLock::new();

fn window() -> &'static Mutex<VecDeque<Instant>> {
    WINDOW.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// 最近 `within` 时长内的击键次数（闸①用）。顺带弹出窗口外旧样本
pub fn keystrokes_within(within: Duration) -> usize {
    let Ok(mut w) = window().lock() else { return 0 };
    let cutoff = Instant::now() - within;
    while w.front().is_some_and(|t| *t < cutoff) {
        w.pop_front();
    }
    w.len()
}

fn count_keydown() {
    if let Ok(mut w) = window().lock() {
        w.push_back(Instant::now());
    }
}

pub struct KeyboardCollector;

impl Collector for KeyboardCollector {
    fn start(
        &self,
        _bus: crate::event_bus::EventBus,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        std::thread::spawn(move || unsafe {
            let hook: HHOOK =
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(callback), std::ptr::null_mut(), 0);
            if hook.is_null() {
                tracing::error!("SetWindowsHookExW(WH_KEYBOARD_LL) failed");
                return;
            }
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
            UnhookWindowsHookEx(hook);
        });
        Ok(())
    }

    fn name(&self) -> &'static str {
        "keyboard(count-only)"
    }
}

unsafe extern "system" fn callback(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if ncode >= 0 && matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
        count_keydown();
    }
    CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam)
}
