//! 行为引擎（M1-5）：事件总线 → 映射表（map.toml）→ 四道闸 → 台词 → 语音管线。
//!
//! 分工（定案 2026-11-21，见技术架构 M1-5 条目）：
//! - 即时反应 = 纯模板 + 槽位（零延迟零 CPU）；LLM 只用于非即时内容（待做）
//! - 台词出口唯一通道：mpsc 送语音引擎（voice::engine），由引擎在 IDLE 态出声
//! - 自己不碰 TTS/播放器（语音管线状态机归 voice 模块）
//! - 跳过 Voice/Test 来源事件：不对自己说的话再做反应（防自激回路）

pub mod gates;
pub mod map;

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::event_bus::EventBus;
use crate::ipc::{EventKind, EventSource};

use gates as gates_mod;
use gates::{FrequencyTier, GateDecision, GateEngine};
use map::BehaviorMap;

/// CPU 采样/队列释放检查的节拍（对齐闸④ 5s 采样粒度）
const TICK: Duration = Duration::from_secs(5);
/// 控制消息的轮询粒度：远小于 TICK，让调试重置近乎即时生效
const CONTROL_POLL: Duration = Duration::from_millis(200);

/// 行为引擎控制（场景模拟器 + 语音调节指令共用通道；Sender 侧经 tauri State 暴露）
pub enum Control {
    /// 四道闸冷却清零（场景模拟器；等价"回到刚启动"）
    ResetCooldowns,
    /// 切频率档（语音调节指令："今天别吵我"/"多陪我聊聊"）
    SetTier(FrequencyTier),
}

/// 启动行为引擎线程。comment_tx 送语音引擎（voice::engine::start 的入参）
pub fn start(
    bus: EventBus,
    control_rx: mpsc::Receiver<Control>,
    comment_tx: mpsc::SyncSender<String>,
) {
    thread::spawn(move || {
        tracing::info!(
            tid = crate::event_collector::tid(),
            "behavior engine thread up"
        );
        let mut behavior_map = BehaviorMap::load("meili");
        // 热更（#34）：轮询映射表 mtime，变了就严格解析重载；失败保留旧表。
        // 与 CPU 采样共用 TICK 节拍——内容工具，秒级生效足够，不添文件监听依赖
        let map_path = BehaviorMap::path("meili");
        let mut map_mtime = std::fs::metadata(&map_path).and_then(|m| m.modified()).ok();
        let mut gates = GateEngine::new(
            behavior_map.entries.len(),
            FrequencyTier::Balanced,
            gates_mod::real_probes(),
        );
        let rx = bus.subscribe();
        tracing::info!(
            entries = behavior_map.entries.len(),
            "behavior engine started"
        );
        let mut last_tick = Instant::now();
        loop {
            // 控制消息优先处理（调试工具，延迟 ≤ CONTROL_POLL）
            while let Ok(c) = control_rx.try_recv() {
                match c {
                    Control::ResetCooldowns => gates.reset_cooldowns(),
                    Control::SetTier(tier) => gates.set_tier(tier),
                }
            }
            match rx.recv_timeout(CONTROL_POLL) {
                Ok(event) => {
                    // Voice/Test 外的 Behavior 源也跳过：不对自己产出的
                    // Gesture 再反应（防自激回路）
                    if matches!(
                        event.source,
                        EventSource::Voice | EventSource::Test | EventSource::Behavior
                    ) {
                        continue;
                    }
                    let outcome = gates.on_event(&event, &behavior_map);
                    // 即时表现层反应（摸头等）：与台词解耦，直接总线直发前端
                    if let Some(anim) = outcome.react {
                        let mut e = bus.event(
                            EventSource::Behavior,
                            EventKind::Gesture,
                            serde_json::json!({ "anim": anim }),
                        );
                        e.privacy_tier = crate::ipc::PrivacyTier::Minimal;
                        bus.publish(e);
                    }
                    if let GateDecision::Speak(text) = outcome.decision {
                        send_comment(&comment_tx, text);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::error!("event bus subscribers disconnected, behavior engine exit");
                    break;
                }
            }
            // CPU 采样按 TICK 节拍推进（时间戳门控，控制轮询细化不影响粒度）
            if last_tick.elapsed() >= TICK {
                gates.tick();
                last_tick = Instant::now();
                // 映射表热更：mtime 变了 → 严格解析重载（失败保留旧表并 error）。
                // 编辑器保存即触发，秒级生效；mtime 取不到（文件短暂消失）视为无变化
                let new_mtime = std::fs::metadata(&map_path).and_then(|m| m.modified()).ok();
                if new_mtime.is_some() && new_mtime != map_mtime {
                    map_mtime = new_mtime;
                    match BehaviorMap::reload("meili") {
                        Ok(new_map) => {
                            gates.resize_entries(new_map.entries.len());
                            let n = new_map.entries.len();
                            behavior_map = new_map;
                            tracing::info!(
                                entries = n,
                                "behavior map hot reloaded (#34，保存即生效)"
                            );
                        }
                        Err(e) => {
                            tracing::error!("映射表热更失败，保留旧表: {e}");
                        }
                    }
                }
            }
            if let Some(text) = gates.release_queue() {
                send_comment(&comment_tx, text);
            }
        }
    });
}

/// 送语音管线：管线忙（通道满）就丢——闸已保证频率极低，丢一句可接受
fn send_comment(tx: &mpsc::SyncSender<String>, text: String) {
    tracing::info!(text = %text, "behavior comment");
    if let Err(e) = tx.try_send(text) {
        tracing::warn!("comment dropped (voice pipeline busy): {e}");
    }
}
