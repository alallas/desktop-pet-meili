//! 主动性四道闸（产品 §5.2；参数 2026-11-21 定案，见技术架构 M1-5 定案条目）。
//!
//! 事件 → 台词前逐闸检查，任一不过 → 进待讲队列（或沉淀）：
//! ① 打字密集（10 分钟击键 > 600）→ 只放行 P0，其余排队
//! ② 全局冷却未到（默契档 30min / 多聊档 12min / 沉默档 ∞）→ 排队
//! ③ 内容门槛（默契档放行 P0+P1，多聊档全放行，P2 永远沉淀；沉默档全沉淀）
//! ④ 心情信号差（深夜 23:00–7:00 **且** CPU>80% 持续 10min）→ 静默（小动作待 M1-6）
//!
//! 待讲队列上限 3 条；释放时带"今天还有件事："式来历交代（产品定稿）。
//! 频率档 v1 固定默契档——语音调节指令（"今天别吵我"/"多陪我聊聊"）是下一步。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::ipc::StdEvent;
use crate::perception::local_hour;

use super::map::{fill_template, BehaviorMap, MapEntry};

/// 频率档（产品 §5.1 存在感光谱的开口频率维度；语音调节指令消费，见 voice/intent.rs）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrequencyTier {
    /// 别吵我：全沉淀，不放行
    Silent,
    /// 默认：P0+P1 放行，全局冷却 30min
    Balanced,
    /// 多陪我聊聊：全放行，全局冷却 12min
    Chatty,
}

impl FrequencyTier {
    fn global_cooldown(self) -> Option<Duration> {
        match self {
            FrequencyTier::Silent => None, // ∞：永不放行
            FrequencyTier::Balanced => Some(Duration::from_secs(30 * 60)),
            FrequencyTier::Chatty => Some(Duration::from_secs(12 * 60)),
        }
    }
    /// 放行优先级 < floor；MAX = 全沉淀
    fn priority_floor(self) -> u8 {
        match self {
            FrequencyTier::Silent => 0,
            FrequencyTier::Balanced => 2, // 放行 P0+P1
            FrequencyTier::Chatty => 3,   // 全放行（priority 最大 2）
        }
    }
}

/// 闸①打字密集判定的滑动窗与阈值（定案初值）
const TYPING_WINDOW: Duration = Duration::from_secs(10 * 60);
const TYPING_DENSE_KEYS: usize = 600;
/// 待讲队列上限（定案）
const QUEUE_CAP: usize = 3;

/// 闸④心情信号参数（定案：深夜且高负载才静默，两个条件都要）
/// 深夜 = 23:00–次日 7:00（小时 23,0..6）
const LATE_NIGHT_HOURS: [u32; 8] = [23, 0, 1, 2, 3, 4, 5, 6];
const CPU_HIGH_PERCENT: f64 = 80.0;
/// CPU 滑窗：5s 一个样本（采样节拍由行为引擎主循环定），10min 窗口 = 120 个
const CPU_WINDOW_SAMPLES: usize = 120;

/// 闸内决定
pub enum GateDecision {
    /// 现在就说（文本已填好槽位）
    Speak(String),
    /// 进待讲队列
    Queue,
    /// 沉淀/静默（不产出台词）
    Drop,
}

/// 排队中的一句台词
struct Queued {
    text: String,
    priority: u8,
    since: Instant,
}

/// CPU 占用采样器：GetSystemTimes 差值算全局 CPU%，滑窗均值
struct CpuMeter {
    last: Option<(u64, u64)>, // (idle, total)
    samples: VecDeque<f64>,
}

impl CpuMeter {
    fn new() -> Self {
        Self {
            last: None,
            samples: VecDeque::with_capacity(CPU_WINDOW_SAMPLES),
        }
    }

    /// 每 CPU_TICK 调一次，推一个样本进滑窗
    fn tick(&mut self) {
        if let Some((idle, total)) = crate::perception::cpu_times_raw() {
            if let Some((p_idle, p_total)) = self.last {
                let d_idle = idle.saturating_sub(p_idle);
                let d_total = total.saturating_sub(p_total);
                if d_total > 0 {
                    let pct = 100.0 * (1.0 - d_idle as f64 / d_total as f64);
                    self.samples.push_back(pct.clamp(0.0, 100.0));
                    while self.samples.len() > CPU_WINDOW_SAMPLES {
                        self.samples.pop_front();
                    }
                }
            }
            self.last = Some((idle, total));
        }
    }

    fn window_avg(&self) -> Option<f64> {
        (self.samples.len() >= CPU_WINDOW_SAMPLES / 2)
            .then(|| self.samples.iter().sum::<f64>() / self.samples.len() as f64)
    }
}

/// 环境探针：闸①打字密集与闸④深夜判定的外部状态注入点。
/// 测试传固定值，生产用 [`real_probes`]——避免单元测试依赖真实键盘/时钟
pub struct Probes {
    pub typing_dense: fn() -> bool,
    pub late_night: fn() -> bool,
}

/// 生产探针：读真实击键窗口与本地时刻
pub fn real_probes() -> Probes {
    Probes {
        typing_dense,
        late_night,
    }
}

pub struct GateEngine {
    tier: FrequencyTier,
    /// 上次开口（含队列释放）
    last_spoken: Option<Instant>,
    /// 条目级冷却生效时刻（下标 = BehaviorMap.entries 下标）
    entry_cooled_at: Vec<Instant>,
    queue: VecDeque<Queued>,
    cpu: CpuMeter,
    probes: Probes,
    /// 变体轮换序号（无 rand 依赖的简单取模）
    variant_seq: u64,
}

impl GateEngine {
    pub fn new(map_len: usize, tier: FrequencyTier, probes: Probes) -> Self {
        Self {
            tier,
            last_spoken: None,
            entry_cooled_at: vec![Instant::now(); map_len],
            queue: VecDeque::new(),
            cpu: CpuMeter::new(),
            probes,
            variant_seq: 0,
        }
    }

    /// 周期采样：行为引擎主循环每 CPU_TICK 调一次（CPU 滑窗推进）
    pub fn tick(&mut self) {
        self.cpu.tick();
    }

    /// 事件入口：逐闸判定。每道闸的判定留 info 级日志——场景模拟器测试期的
    /// 排障刚需（"她为什么没说话"必须能从日志直接读到闸门结论）
    pub fn on_event(&mut self, event: &StdEvent, map: &BehaviorMap) -> GateDecision {
        let Some((idx, entry)) = map.find(event, |i| self.entry_cooldown_done(i)) else {
            // 无匹配 / 条目冷却中：正常沉默（两者同一出口，payload 条件不满足也走这里）
            tracing::info!(kind = ?event.kind, "gate: drop (no match / entry cooldown)");
            return GateDecision::Drop;
        };
        // 闸③内容门槛（P2 永远沉淀；沉默档全沉淀）
        if entry.priority >= self.tier.priority_floor() {
            tracing::info!(kind = ?event.kind, priority = entry.priority, "gate③: priority floor drop");
            return GateDecision::Drop;
        }
        // 台词：变体轮换，取第一条槽位可填的
        let Some(text) = self.fill_any_variant(entry, &event.payload) else {
            tracing::info!(kind = ?event.kind, "gate: drop (no fillable variant / missing slot)");
            return GateDecision::Drop;
        };
        // 闸④心情差：降级静默小动作（v1 静默；动作待 M1-6）
        if self.mood_bad() {
            tracing::info!(kind = ?event.kind, priority = entry.priority, "gate④ mood bad, silent gesture");
            return GateDecision::Drop;
        }
        // 闸①打字密集：非 P0 排队（P0 极重要放行）
        if entry.priority > 0 && (self.probes.typing_dense)() {
            tracing::info!(kind = ?event.kind, "gate① typing dense → queue");
            return self.enqueue(text, entry.priority);
        }
        // 闸②全局冷却：排队
        if !self.global_cooldown_done() {
            tracing::info!(kind = ?event.kind, "gate② global cooldown → queue");
            return self.enqueue(text, entry.priority);
        }
        self.mark_spoken(idx, entry);
        GateDecision::Speak(text)
    }

    /// 队列释放检查（每轮主循环调用）：冷却过、心情好、不在打字 → 挑最重要一条
    pub fn release_queue(&mut self) -> Option<String> {
        if self.queue.is_empty() || self.tier == FrequencyTier::Silent {
            return None;
        }
        if self.mood_bad() || (self.probes.typing_dense)() || !self.global_cooldown_done() {
            return None;
        }
        // 最高优先级；同级取最早入队
        let best = self
            .queue
            .iter()
            .enumerate()
            .min_by_key(|(_, q)| (q.priority, q.since))
            .map(|(i, _)| i)?;
        let q = self.queue.remove(best)?;
        self.last_spoken = Some(Instant::now());
        // 交代来历（产品定稿）：还剩多条/一条不同措辞
        let intro = if self.queue.len() >= 2 {
            "今天还有件事："
        } else if self.queue.len() == 1 {
            "对了，还有："
        } else {
            ""
        };
        Some(format!("{intro}{}", q.text))
    }

    fn fill_any_variant(
        &mut self,
        entry: &MapEntry,
        payload: &serde_json::Value,
    ) -> Option<String> {
        if entry.say.is_empty() {
            return None;
        }
        let start = (self.variant_seq as usize) % entry.say.len();
        self.variant_seq += 1;
        (0..entry.say.len())
            .map(|k| &entry.say[(start + k) % entry.say.len()])
            .find_map(|t| fill_template(t, payload))
    }

    fn enqueue(&mut self, text: String, priority: u8) -> GateDecision {
        if self.queue.len() >= QUEUE_CAP {
            // 满员：新条目优先级必须低于队中最高才可能挤入——挤掉最低最老的；
            // 不更低则沉淀
            let Some(max_idx) = self
                .queue
                .iter()
                .enumerate()
                .max_by_key(|(_, q)| (q.priority, std::cmp::Reverse(q.since)))
                .map(|(i, _)| i)
            else {
                return GateDecision::Drop;
            };
            if priority >= self.queue[max_idx].priority {
                return GateDecision::Drop;
            }
            self.queue.remove(max_idx);
        }
        self.queue.push_back(Queued {
            text,
            priority,
            since: Instant::now(),
        });
        GateDecision::Queue
    }

    fn mark_spoken(&mut self, idx: usize, entry: &MapEntry) {
        self.last_spoken = Some(Instant::now());
        self.entry_cooled_at[idx] = Instant::now() + Duration::from_secs(entry.cooldown_min * 60);
    }

    fn entry_cooldown_done(&self, i: usize) -> bool {
        Instant::now() >= self.entry_cooled_at[i]
    }

    fn global_cooldown_done(&self) -> bool {
        match (self.tier.global_cooldown(), self.last_spoken) {
            (None, _) => false, // 沉默档 ∞
            (Some(cd), Some(t)) => t.elapsed() >= cd,
            (Some(_), None) => true,
        }
    }

    /// 闸④：深夜且持续高负载（两个条件都要，2026-11-21 定案）
    fn mood_bad(&self) -> bool {
        let late = (self.probes.late_night)();
        let loaded = self
            .cpu
            .window_avg()
            .is_some_and(|avg| avg >= CPU_HIGH_PERCENT);
        late && loaded
    }

    /// 调试重置（场景模拟器配套）：回到"刚开口前"状态——清全局冷却、条目冷却
    /// 与待讲队列；打字/CPU 滑窗是行为统计不是冷却，保留。
    pub fn reset_cooldowns(&mut self) {
        self.last_spoken = None;
        self.entry_cooled_at
            .iter_mut()
            .for_each(|t| *t = Instant::now());
        self.queue.clear();
        tracing::info!("behavior gates reset (debug)");
    }

    /// 语音调节指令入口："今天别吵我"/"多陪我聊聊" → 切频率档
    pub fn set_tier(&mut self, tier: FrequencyTier) {
        if self.tier != tier {
            tracing::info!(from = ?self.tier, to = ?tier, "behavior tier switched (voice command)");
        }
        self.tier = tier;
    }
}

/// 击键密度（闸①，真实探针）：10 分钟滑动窗计数 ≥ 阈值
fn typing_dense() -> bool {
    crate::event_collector::keyboard::keystrokes_within(TYPING_WINDOW) >= TYPING_DENSE_KEYS
}

/// 深夜时段（闸④，真实探针）：23:00–次日 7:00
fn late_night() -> bool {
    local_hour().is_some_and(|h| LATE_NIGHT_HOURS.contains(&h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::map::BehaviorMap;
    use crate::ipc::{EventKind, EventSource, PrivacyTier};

    fn evt(kind: EventKind) -> StdEvent {
        StdEvent {
            id: "t".into(),
            ts: 0,
            source: EventSource::Test,
            kind,
            payload: serde_json::json!({}),
            privacy_tier: PrivacyTier::Minimal,
        }
    }

    /// 无环境阻碍的探针（测试默认）：不打字密集、不是深夜
    fn clear_probes() -> Probes {
        Probes {
            typing_dense: || false,
            late_night: || false,
        }
    }

    fn map_one(text: &str) -> BehaviorMap {
        BehaviorMap::parse(&format!(
            r#"[[map]]
on = "booted"
say = ["{text}"]
"#
        ))
    }

    #[test]
    fn first_event_speaks_second_kind_queued_by_global_cooldown() {
        // 两条不同事件：条目级冷却互不干扰，第二事件走全局冷却 → 排队
        let map = BehaviorMap::parse(
            r#"[[map]]
on = "booted"
say = ["你好"]

[[map]]
on = "ping"
say = ["在的"]
"#,
        );
        let mut g = GateEngine::new(map.entries.len(), FrequencyTier::Balanced, clear_probes());
        match g.on_event(&evt(EventKind::Booted), &map) {
            GateDecision::Speak(t) => assert_eq!(t, "你好"),
            _ => panic!("首事件应直接放行"),
        }
        match g.on_event(&evt(EventKind::Ping), &map) {
            GateDecision::Queue => {}
            _ => panic!("全局冷却内第二事件应排队"),
        }
        assert!(g.release_queue().is_none(), "全局冷却内队列不得释放");
    }

    #[test]
    fn same_entry_twice_dropped_by_entry_cooldown() {
        // 同类事件冷却内重复触发 = 正常沉默（不进队列；闸②管的是跨事件全局冷却）
        let map = map_one("你好");
        let mut g = GateEngine::new(map.entries.len(), FrequencyTier::Balanced, clear_probes());
        assert!(matches!(
            g.on_event(&evt(EventKind::Booted), &map),
            GateDecision::Speak(_)
        ));
        assert!(matches!(
            g.on_event(&evt(EventKind::Booted), &map),
            GateDecision::Drop
        ));
    }

    #[test]
    fn silent_tier_drops_everything() {
        let map = map_one("你好");
        let mut g = GateEngine::new(map.entries.len(), FrequencyTier::Silent, clear_probes());
        assert!(matches!(
            g.on_event(&evt(EventKind::Booted), &map),
            GateDecision::Drop
        ));
        assert!(g.release_queue().is_none());
    }

    #[test]
    fn p2_always_drops_in_balanced() {
        let map = BehaviorMap::parse(
            r#"[[map]]
on = "booted"
priority = 2
say = ["背景话"]
"#,
        );
        let mut g = GateEngine::new(map.entries.len(), FrequencyTier::Balanced, clear_probes());
        assert!(matches!(
            g.on_event(&evt(EventKind::Booted), &map),
            GateDecision::Drop
        ));
    }

    #[test]
    fn chatty_tier_allows_p2() {
        let map = BehaviorMap::parse(
            r#"[[map]]
on = "booted"
priority = 2
say = ["背景话"]
"#,
        );
        let mut g = GateEngine::new(map.entries.len(), FrequencyTier::Chatty, clear_probes());
        assert!(matches!(
            g.on_event(&evt(EventKind::Booted), &map),
            GateDecision::Speak(_)
        ));
    }

    #[test]
    fn queue_cap_and_priority_eviction() {
        let map = map_one("低");
        let mut g = GateEngine::new(map.entries.len(), FrequencyTier::Balanced, clear_probes());
        // 手动灌满低优先级队列（绕过 on_event，直接测队列语义）
        for _ in 0..3 {
            assert!(matches!(g.enqueue("低".into(), 1), GateDecision::Queue));
        }
        assert!(
            matches!(g.enqueue("低".into(), 1), GateDecision::Drop),
            "同优先级满员即沉淀"
        );
        assert!(
            matches!(g.enqueue("高优".into(), 0), GateDecision::Queue),
            "更高优先级应挤掉队中最低"
        );
    }

    #[test]
    fn variant_rotation_fills_slots() {
        let map = BehaviorMap::parse(
            r#"[[map]]
on = "foreground_switched"
say = ["换到{{app}}了", "{{title}}哦"]
"#,
        );
        let mut g = GateEngine::new(map.entries.len(), FrequencyTier::Balanced, clear_probes());
        let payload = serde_json::json!({"app": "Code"}); // 无 title：第二条变体不可用
                                                          // 轮换两轮都应跳过缺槽变体、落到唯一可填的
        for _ in 0..2 {
            assert_eq!(
                g.fill_any_variant(&map.entries[0].1, &payload).as_deref(),
                Some("换到Code了")
            );
        }
    }

    #[test]
    fn reset_cooldowns_allows_speaking_again() {
        // 场景模拟器配套：开口后全局冷却未到 → 他事件排队；重置后应立即可说
        let map = BehaviorMap::parse(
            r#"[[map]]
on = "booted"
say = ["你好"]

[[map]]
on = "ping"
say = ["在的"]
"#,
        );
        let mut g = GateEngine::new(map.entries.len(), FrequencyTier::Balanced, clear_probes());
        assert!(matches!(
            g.on_event(&evt(EventKind::Booted), &map),
            GateDecision::Speak(_)
        ));
        assert!(matches!(
            g.on_event(&evt(EventKind::Ping), &map),
            GateDecision::Queue
        ));
        g.reset_cooldowns();
        assert!(matches!(
            g.on_event(&evt(EventKind::Ping), &map),
            GateDecision::Speak(t) if t == "在的"
        ));
        assert!(g.release_queue().is_none(), "重置应连待讲队列一起清掉");
    }
}
