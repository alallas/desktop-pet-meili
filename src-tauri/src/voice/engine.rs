//! 语音常驻引擎（M1-3.2/3.3）：麦克风采集（cpal）→ 重采样 16k →
//! 状态机（IDLE: KWS 常听唤醒 / LISTENING: VAD 断句采指令）→ 事件总线。
//!
//! 线程模型：一条常驻线程持 KWS 引擎；cpal 采集回调只把采样 push 进有界通道
//! （满则丢块，KWS 容忍少量丢帧），解码全在常驻线程做（与采集器"回调只翻译、
//! 不重活"的约定一致，事件总线传输级削峰同款思路）。
//!
//! 模型路径：dev 阶段用仓库根 `models/kws`（相对 CARGO_MANIFEST_DIR）；
//! 打包分发（tauri resources）随 M1-3 验收前补齐，见技术架构 §11 待办。

use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample; // f32::from_sample（Sample trait 方法，需显式引入）

use crate::event_bus::EventBus;
use crate::ipc::{EventKind, EventSource, PrivacyTier};

use super::asr::Recognizer;
use super::kws::KeywordSpotter;
// 线性插值重采样器已泛化为任意率对（in→out）并移至 resample.rs：
// 采集通路（设备率→16k）与 TTS 输出通路（24k→设备率）共用同一实现。
use super::resample::Resampler;
use super::vad::{SpeechSegment, Vad};

/// 喂给 KWS 的块大小（0.1s @16k；M0 Python demo 用 0.2s，流式减半降低命中延迟）
const CHUNK_SAMPLES: usize = 1600;
/// 唤醒命中后的冷却：reset 后残余音频可能二次触发，冷却窗口内不再发事件
const WAKE_COOLDOWN: Duration = Duration::from_millis(1500);
/// 采集回调 → 引擎线程的缓冲块数（0.1s/块；20 块 = 2s，背压上限）
const CHANNEL_BLOCKS: usize = 20;
/// LISTENING 态超时：唤醒后一直不说话/VAD 漏检，超时回 IDLE 重新常听
const MAX_LISTENING: Duration = Duration::from_secs(12);

/// 启动语音引擎（装配层唯一入口，进程常驻）。失败只记日志不 panic——
/// 无麦克风的环境桌宠其余功能不受影响（装配层不重试）。
pub fn start(bus: EventBus) {
    thread::spawn(move || {
        if let Err(e) = run(bus) {
            tracing::error!("voice engine stopped: {e:#}");
        }
    });
}

fn run(bus: EventBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let model_dir = root.join("../models/kws");
    let dir = model_dir.to_str().ok_or("模型路径非 UTF-8")?;
    let mut spotter = KeywordSpotter::new(dir, &format!("{dir}/keywords.txt"))?;
    let mut vad = Vad::new(&root.join("../models/silero_vad.onnx").to_string_lossy())?;
    let asr = Recognizer::new(
        &root
            .join("../models/sense-voice.int8.onnx")
            .to_string_lossy(),
        &root
            .join("../models/sense-voice-tokens.txt")
            .to_string_lossy(),
    )?;

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or("无默认输入设备（麦克风）")?;
    let config = device.default_input_config()?;
    tracing::info!(
        device = %device.name().unwrap_or_default(),
        rate = config.sample_rate().0,
        channels = config.channels(),
        format = ?config.sample_format(),
        "voice engine: input device"
    );

    let (tx, rx) = sync_channel::<Vec<f32>>(CHANNEL_BLOCKS);
    let in_rate = config.sample_rate().0 as f64;
    let channels = config.channels() as usize;
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => {
            build_stream::<f32>(&device, &config.config(), channels, tx, in_rate)?
        }
        cpal::SampleFormat::I16 => {
            build_stream::<i16>(&device, &config.config(), channels, tx, in_rate)?
        }
        other => return Err(format!("不支持的采样格式 {other:?}").into()),
    };
    stream.play()?;
    tracing::info!("voice engine: listening for wake word");

    // 思考层（本地 Qwen3-4B sidecar）：启动失败不致命——语音链路照常，只是没回复
    let mut thinker = match crate::llm::Thinker::start() {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::error!("thinker unavailable, replies disabled: {e:#}");
            None
        }
    };
    // TTS（Kokoro sidecar 合成 + cpal 播放）：启动失败不致命——降级为静默回复
    let mut tts = match super::tts::TtsSidecar::spawn() {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::error!("tts sidecar unavailable, silent replies: {e:#}");
            None
        }
    };
    let mut player = match super::player::Player::new() {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::error!("tts player unavailable, silent replies: {e:#}");
            None
        }
    };

    let mut parts = EngineParts {
        spotter: &mut spotter,
        vad: &mut vad,
        asr: &asr,
        thinker: thinker.as_mut(),
        tts: tts.as_mut(),
        player: player.as_mut(),
    };
    consume(bus, &mut parts, rx);
    Ok(())
}

/// cpal 回调与引擎线程的分界线：回调里只做 单声道化 + 线性重采样 + try_send。
fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    tx: SyncSender<Vec<f32>>,
    in_rate: f64,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample, // build_input_stream 的硬约束；SizedSample: Sample 超trait
    f32: cpal::FromSample<T>, // from_sample 的转换目标约束
{
    let mut resampler = Resampler::new(in_rate, 16000.0);
    device.build_input_stream(
        config,
        move |data: &[T], _| {
            // 多声道取平均单声道化（KWS 只吃 mono）
            let mono: Vec<f32> = if channels == 1 {
                data.iter().map(|s| f32::from_sample(*s)).collect()
            } else {
                data.chunks(channels)
                    .map(|c| c.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32)
                    .collect()
            };
            let out = resampler.push(&mono);
            // 满则丢块：KWS 容忍少量丢帧，阻塞回调会爆音甚至拖垮采集线程
            let _ = tx.try_send(out);
        },
        |e| tracing::error!("voice capture error: {e}"),
        None,
    )
}

/// LISTENING 态秒级音量探针：攒 1s 的平方和再开方，每 1s 报一次。
/// 诊断工具（真人语音 VAD 漏检排查），稳定后可降为 debug 或移除。
struct LevelMeter {
    sum_sq: f64,
    n: usize,
}

impl LevelMeter {
    fn new() -> Self {
        Self { sum_sq: 0.0, n: 0 }
    }

    fn accumulate(&mut self, samples: &[f32]) {
        self.sum_sq += samples
            .iter()
            .map(|v| (*v as f64) * (*v as f64))
            .sum::<f64>();
        self.n += samples.len();
    }

    /// 攒满 1s（16k 采样）则返回该窗口 RMS 并重置；未满返回 None
    fn tick(&mut self) -> Option<f64> {
        if self.n >= 16000 {
            let rms = (self.sum_sq / self.n as f64).sqrt();
            self.n = 0;
            self.sum_sq = 0.0;
            Some(rms)
        } else {
            None
        }
    }
}

/// 引擎状态机（M1-3.3 → M1-3.5b）：
/// - IDLE：KWS 常听唤醒词；命中 → LISTENING 采指令语音；
/// - LISTENING：VAD 断句出段 → 本地 ASR 转写 → 本地思考层 → TTS 出声 → SPEAKING；
/// - SPEAKING：KWS 继续听（打断 barge-in：命中 → 停播 → 回 LISTENING）；自然播完 → 回 IDLE。
///
/// 已知折衷（待办 #11）：播放中扬声器回声可能误触发 KWS，回声消除后续评估。
#[derive(Clone, Copy)]
enum State {
    Idle,
    Listening { since: Instant },
    Speaking,
}

/// 引擎常驻线程的可变组件集合（consume 参数打包，避免长参数列表）
struct EngineParts<'a> {
    spotter: &'a mut KeywordSpotter,
    vad: &'a mut Vad,
    asr: &'a Recognizer,
    thinker: Option<&'a mut crate::llm::Thinker>,
    tts: Option<&'a mut super::tts::TtsSidecar>,
    player: Option<&'a mut super::player::Player>,
}

/// 引擎常驻线程：状态机分发 0.1s 块——IDLE/SPEAKING 喂 KWS，LISTENING 喂 VAD。
fn consume(bus: EventBus, parts: &mut EngineParts, rx: Receiver<Vec<f32>>) {
    let EngineParts {
        spotter,
        vad,
        asr,
        thinker,
        tts,
        player,
    } = parts;
    let mut buf = Vec::with_capacity(CHUNK_SAMPLES);
    // VAD 侧对齐缓冲：0.1s 块（1600）非 512 倍数，攒到 4×512 再喂
    let mut vad_buf: Vec<f32> = Vec::new();
    // 初始即允许首次命中（now() - cooldown）
    let mut last_wake = Instant::now() - WAKE_COOLDOWN;
    let mut state = State::Idle;
    let mut meter = LevelMeter::new();
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(chunk) => buf.extend_from_slice(&chunk),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        while buf.len() >= CHUNK_SAMPLES {
            let chunk: Vec<f32> = buf.drain(..CHUNK_SAMPLES).collect();
            match state {
                State::Idle | State::Speaking => {
                    if let Some(hit) = spotter.accept_waveform(&chunk) {
                        if last_wake.elapsed() >= WAKE_COOLDOWN {
                            last_wake = Instant::now();
                            // 打断（barge-in）：播放中命中唤醒词 → 停播并通知前端收嘴
                            if matches!(state, State::Speaking) {
                                if let Some(p) = player.as_deref_mut() {
                                    p.stop();
                                }
                                publish_speak_ended(&bus, true);
                            }
                            let mut event = bus.event(
                                EventSource::Voice,
                                EventKind::WakeWord,
                                serde_json::json!({ "keyword": hit.keyword }),
                            );
                            // 唤醒事件本体不含语音内容且全程本地处理：隐私档 Minimal
                            event.privacy_tier = PrivacyTier::Minimal;
                            bus.publish(event);
                            // 命中块尾部（唤醒词收音）也进 VAD：min_speech 会滤掉过短残渣
                            vad.reset();
                            vad_buf.clear();
                            tracing::info!("wake hit, entering listening");
                            state = State::Listening {
                                since: Instant::now(),
                            };
                            feed_vad(vad, &mut vad_buf, &chunk);
                        }
                    }
                    // 播完检测：输出回调把"队列播空"翻成 !is_playing，这里接成事件
                    if matches!(state, State::Speaking) {
                        let finished = match player.as_deref_mut() {
                            Some(p) => !p.is_playing(),
                            None => true,
                        };
                        if finished {
                            publish_speak_ended(&bus, false);
                            state = State::Idle;
                        }
                    }
                }
                State::Listening { since } => {
                    feed_vad(vad, &mut vad_buf, &chunk);
                    // 诊断探针：秒级音量/VAD 状态，区分"麦没声"与"VAD 漏判"
                    meter.accumulate(&chunk);
                    if let Some(rms) = meter.tick() {
                        tracing::info!(
                            rms = format_args!("{:.5}", rms),
                            in_speech = vad.in_speech(),
                            elapsed_ms = since.elapsed().as_millis(),
                            "listening probe"
                        );
                    }
                    if let Some(seg) = drain_vad_segment(vad) {
                        let duration_ms = seg.samples.len() as u64 / 16;
                        publish_utterance(&bus, duration_ms);
                        // 本地转写（阻塞 ~0.5s；期间采集通道缓冲，唤醒容忍漏听，见 mod.rs）
                        let t0 = Instant::now();
                        let mut pending_speak: Option<String> = None;
                        match asr.transcribe(&seg.samples) {
                            Ok(text) => {
                                tracing::info!(
                                    elapsed_ms = t0.elapsed().as_millis(),
                                    text = %text,
                                    "asr transcript"
                                );
                                publish_transcript(&bus, &text, duration_ms);
                                // 思考层（阻塞 ~2–5s）：无 key 的本地路径，人设见 llm/mod.rs
                                if let Some(thinker) = thinker.as_deref_mut() {
                                    let t1 = Instant::now();
                                    let (reply, fallback) = match thinker.think(&text) {
                                        Ok(r) => (r, false),
                                        Err(e) => {
                                            tracing::error!("think failed: {e:#}");
                                            (crate::llm::FALLBACK_REPLY.to_string(), true)
                                        }
                                    };
                                    tracing::info!(
                                        latency_ms = t1.elapsed().as_millis(),
                                        fallback = fallback,
                                        text = %reply,
                                        "llm reply"
                                    );
                                    publish_reply(&bus, &reply, t1.elapsed().as_millis());
                                    pending_speak = Some(reply);
                                }
                            }
                            Err(e) => tracing::error!("asr failed: {e:#}"),
                        }
                        // TTS 出声（M1-3.5b）：合成+播放成功进 SPEAKING；失败降级静默回 IDLE
                        state = match pending_speak {
                            Some(text) => {
                                match speak_reply(tts.as_deref_mut(), player.as_deref_mut(), &text)
                                {
                                    Ok(()) => State::Speaking,
                                    Err(e) => {
                                        tracing::error!("tts speak failed, silent reply: {e:#}");
                                        State::Idle
                                    }
                                }
                            }
                            None => State::Idle,
                        };
                    } else if since.elapsed() >= MAX_LISTENING {
                        // 兜底：说了唤醒词但一直没说话（或 VAD 漏检），别永远占着 LISTENING
                        tracing::info!("listening timeout, back to idle");
                        state = State::Idle;
                    }
                }
            }
        }
    }
}

/// 把 0.1s 块喂进 VAD（内部先攒到 512 的整数倍）
fn feed_vad(vad: &mut Vad, vad_buf: &mut Vec<f32>, chunk: &[f32]) {
    const VAD_FEED: usize = 512 * 4; // 2048 采样 = 0.128s，512 对齐且不过度拖延断句
    vad_buf.extend_from_slice(chunk);
    while vad_buf.len() >= VAD_FEED {
        let feed: Vec<f32> = vad_buf.drain(..VAD_FEED).collect();
        vad.accept_waveform(&feed);
    }
}

/// VAD 出段则取时长（ms）并清空队列；音频本体留给 M1-3.4 ASR 消费
fn drain_vad_segment(vad: &mut Vad) -> Option<SpeechSegment> {
    let seg = vad.pop_segment()?;
    tracing::info!(
        start = seg.start,
        n = seg.samples.len(),
        "vad segment, utterance ended"
    );
    while vad.pop_segment().is_some() {
        // 一次只取一句；异常多段（max_speech 切分）一并清掉
    }
    Some(seg)
}

fn publish_utterance(bus: &EventBus, duration_ms: u64) {
    let mut event = bus.event(
        EventSource::Voice,
        EventKind::UtteranceEnded,
        serde_json::json!({ "duration_ms": duration_ms }),
    );
    event.privacy_tier = PrivacyTier::Minimal;
    bus.publish(event);
}

/// 转写结果事件：文本是语音内容本体，比时长敏感——隐私档 Balanced（默认档）
fn publish_transcript(bus: &EventBus, text: &str, duration_ms: u64) {
    let mut event = bus.event(
        EventSource::Voice,
        EventKind::TranscriptReady,
        serde_json::json!({ "text": text, "duration_ms": duration_ms }),
    );
    event.privacy_tier = PrivacyTier::Balanced;
    bus.publish(event);
}

/// 思考层回复事件（M1-3.5a 终点；TTS 消费见 M1-3.5b）
fn publish_reply(bus: &EventBus, text: &str, latency_ms: u128) {
    let mut event = bus.event(
        EventSource::Voice,
        EventKind::ReplyReady,
        serde_json::json!({ "text": text, "latency_ms": latency_ms }),
    );
    event.privacy_tier = PrivacyTier::Balanced;
    bus.publish(event);
}

/// 合成 + 播放一句回复（M1-3.5b）。合成阻塞 ~1–3s（与 ASR/LLM 同级，引擎线程
/// 容忍短暂漏听唤醒）；播放本身非阻塞（队列 + 输出回调，见 player.rs）。
fn speak_reply(
    tts: Option<&mut super::tts::TtsSidecar>,
    player: Option<&mut super::player::Player>,
    text: &str,
) -> Result<(), String> {
    let tts = tts.ok_or("tts sidecar 不可用")?;
    let player = player.ok_or("tts player 不可用")?;
    let t0 = Instant::now();
    let path = tts.synth(text)?;
    let bytes = std::fs::read(&path).map_err(|e| format!("读 wav 失败 {path:?}: {e}"))?;
    player.play_wav(&bytes)?;
    tracing::info!(elapsed_ms = t0.elapsed().as_millis(), "tts speak queued");
    Ok(())
}

/// 播放结束事件（自然播完 barge_in=false / 被唤醒打断 barge_in=true）。
/// 仅播放状态翻转，无内容：隐私档 Minimal。前端消费：收嘴、退出说话动画。
fn publish_speak_ended(bus: &EventBus, barge_in: bool) {
    let mut event = bus.event(
        EventSource::Voice,
        EventKind::SpeakEnded,
        serde_json::json!({ "barge_in": barge_in }),
    );
    event.privacy_tier = PrivacyTier::Minimal;
    bus.publish(event);
}

/// 线性插值重采样器（设备原生率 → 16k）已泛化为任意率对并移至 resample.rs
#[cfg(test)]
mod tests {
    use super::*;

    /// LevelMeter：满幅正弦 1s 的 RMS ≈ 0.707，1s 未满不报
    #[test]
    fn level_meter_rms() {
        let mut m = LevelMeter::new();
        let sine: Vec<f32> = (0..16000)
            .map(|i| (2.0 * std::f64::consts::PI * i as f64 / 100.0).sin() as f32)
            .collect();
        m.accumulate(&sine[..8000]);
        assert!(m.tick().is_none(), "未满 1s 不应出数");
        m.accumulate(&sine[8000..]);
        let rms = m.tick().expect("满 1s 应出数");
        assert!(
            (rms - 2.0_f64.sqrt() / 2.0).abs() < 0.01,
            "满幅正弦 RMS: {rms}"
        );
    }
}
