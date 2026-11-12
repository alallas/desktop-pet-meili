//! 空闲采集器：GetLastInputInfo 1Hz 轮询 + 滞回翻转。
//!
//! 架构 §3.4 明确允许「≤1Hz 轮询」，符合 <1% CPU 红线（本条线程常眠）。
//! 翻转才发事件，中间状态不发——行为引擎只看"开始闲/结束闲"两个瞬间。

use std::time::Duration;

use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

use super::Collector;
use crate::debounce::Hysteresis;
use crate::event_bus::EventBus;
use crate::ipc::{EventKind, EventSource};

const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// 滞回带 60s：≥180s 判闲，≤120s 判活，中间维持原状态，防阈值横跳。
const IDLE_HIGH_SECS: f64 = 180.0;
const IDLE_LOW_SECS: f64 = 120.0;

pub struct IdleCollector;

impl Collector for IdleCollector {
    fn start(&self, bus: EventBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        std::thread::spawn(move || {
            let mut hysteresis = Hysteresis::new(IDLE_LOW_SECS, IDLE_HIGH_SECS);
            loop {
                let idle_secs = last_input_idle_secs();
                let (flipped, is_idle) = hysteresis.update(idle_secs);
                if flipped {
                    bus.publish(bus.event(
                        EventSource::Idle,
                        EventKind::IdleStateChanged,
                        serde_json::json!({ "idle_seconds": idle_secs as u64, "is_idle": is_idle }),
                    ));
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        });
        Ok(())
    }

    fn name(&self) -> &'static str {
        "idle"
    }
}

/// 距上次键鼠输入的秒数。u32 wrapping 算术兼容 GetTickCount 49 天回绕。
fn last_input_idle_secs() -> f64 {
    unsafe {
        let mut info: LASTINPUTINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<LASTINPUTINFO>() as u32;
        if GetLastInputInfo(&mut info) == 0 {
            return 0.0;
        }
        (GetTickCount().wrapping_sub(info.dwTime) / 1000) as f64
    }
}
