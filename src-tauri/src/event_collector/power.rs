//! 电源采集器：GetSystemPowerStatus 1Hz 轮询（架构 §3.1 明确允许），变化才发事件。
//!
//! 变化定义：(ac_online, charging, 电量十档) 三元组跳变——电量按 10% 分档防止
//! 1% 抖动刷事件。启动先报一次基线（行为引擎要知道"我是插着电的"）。

use std::time::Duration;

use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

use super::Collector;
use crate::event_bus::EventBus;
use crate::ipc::{EventKind, EventSource};

const POLL_INTERVAL: Duration = Duration::from_secs(1);

const BATTERY_FLAG_CHARGING: u8 = 0x08;
const BATTERY_FLAG_NO_BATTERY: u8 = 0x80;

pub struct PowerCollector;

impl Collector for PowerCollector {
    fn start(&self, bus: EventBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        std::thread::spawn(move || {
            let mut prev: Option<(bool, bool, u8)> = None;
            loop {
                let cur = power_snapshot();
                if cur.is_some() && cur != prev {
                    prev = cur;
                    if let Some((ac, charging, percent)) = cur {
                        bus.publish(bus.event(
                            EventSource::Power,
                            EventKind::PowerStateChanged,
                            serde_json::json!({
                                "ac_online": ac,
                                "charging": charging,
                                "battery_percent": percent,
                            }),
                        ));
                    }
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        });
        Ok(())
    }

    fn name(&self) -> &'static str {
        "power"
    }
}

/// (是否插电, 是否充电中, 电量十档 0-10；无电池时 percent=None)
fn power_snapshot() -> Option<(bool, bool, u8)> {
    unsafe {
        let mut s: SYSTEM_POWER_STATUS = std::mem::zeroed();
        if GetSystemPowerStatus(&mut s) == 0 {
            return None;
        }
        let ac = s.ACLineStatus == 1;
        let charging = s.BatteryFlag & BATTERY_FLAG_CHARGING != 0;
        // 无电池 / 电量未知 → 电量维度视为常数（0 档），只看 ac/charging 变化
        let percent = if s.BatteryFlag & BATTERY_FLAG_NO_BATTERY != 0 || s.BatteryLifePercent == 255
        {
            0
        } else {
            s.BatteryLifePercent / 10
        };
        Some((ac, charging, percent))
    }
}
