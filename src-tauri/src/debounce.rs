//! 防抖器：阈值 + 滞回 + 冷却（工程规范 §1；架构 §3.4"防抖"）。
//! 纯逻辑，进 cargo test。两种形态：
//! - [`Cooling`]：离散事件的按 key 冷却（同 key N 秒内只放行一次）；
//! - [`Hysteresis`]：连续信号的阈值滞回（跨过高线触发、低于低线解除，防抖动）。

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// 按 key 冷却：同 key 在 cooldown 内重复到达直接丢弃。
/// 用于事件洪峰削峰（如 SetWinEventHook 的 LOCATIONCHANGE 拖动时每秒上百条）。
#[derive(Debug)]
pub struct Cooling {
    cooldown: Duration,
    last_fired: HashMap<String, Instant>,
}

impl Cooling {
    pub fn new(cooldown: Duration) -> Self {
        Self {
            cooldown,
            last_fired: HashMap::new(),
        }
    }

    /// true = 放行；同时刷新该 key 的冷却计时。
    /// 不做 gc：调用方的 key 集合有界（封闭枚举的 kind 组合），map 不会无限增长。
    pub fn allow(&mut self, key: &str) -> bool {
        let now = Instant::now();
        match self.last_fired.get(key) {
            Some(t) if now.duration_since(*t) < self.cooldown => false,
            _ => {
                self.last_fired.insert(key.to_owned(), now);
                true
            }
        }
    }
}

/// 阈值滞回：value ≥ high 触发（进入 active），≤ low 解除，中间维持原状态。
/// 用于空闲时长、CPU 负载等连续信号，避免在阈值附近反复横跳。
#[derive(Debug, Clone, Copy)]
pub struct Hysteresis {
    low: f64,
    high: f64,
    active: bool,
}

impl Hysteresis {
    pub fn new(low: f64, high: f64) -> Self {
        assert!(low < high, "滞回区间非法：low 必须小于 high");
        Self {
            low,
            high,
            active: false,
        }
    }

    /// 返回 (是否翻转, 当前状态)。翻转瞬间才允许发事件。
    pub fn update(&mut self, value: f64) -> (bool, bool) {
        let prev = self.active;
        if value >= self.high {
            self.active = true;
        } else if value <= self.low {
            self.active = false;
        }
        (prev != self.active, self.active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooling_blocks_repeat_within_cooldown() {
        let mut c = Cooling::new(Duration::from_millis(100));
        assert!(c.allow("fg.move"), "首次放行");
        assert!(!c.allow("fg.move"), "冷却内拦截");
        assert!(c.allow("other"), "不同 key 互不影响");
        std::thread::sleep(Duration::from_millis(110));
        assert!(c.allow("fg.move"), "冷却后放行");
    }

    #[test]
    fn hysteresis_flips_only_across_bounds() {
        let mut h = Hysteresis::new(0.2, 0.8);
        // 从低往高：跨过高线才触发
        assert_eq!(h.update(0.5), (false, false), "区间中部不翻转");
        assert_eq!(h.update(0.9), (true, true), "跨过高线触发");
        // 回落但不破低线：维持
        assert_eq!(h.update(0.5), (false, true), "滞回带内维持 active");
        // 跌破低线：解除
        assert_eq!(h.update(0.1), (true, false), "跌破低线解除");
        // 从下往上：跨过高线才再次触发，区间中部不触发
        assert_eq!(h.update(0.7), (false, false), "未跨高线不触发");
        assert_eq!(h.update(0.85), (true, true), "再次跨高线触发");
    }

    #[test]
    #[should_panic]
    fn hysteresis_rejects_inverted_bounds() {
        let _ = Hysteresis::new(0.8, 0.2);
    }
}
