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
/// 空唤醒抑制：唤醒后 12s 内没有任何转写 → 10s 不再响应唤醒，压住连续自触发
/// 风暴。曾设 30s，实测把真人的连续呼喊整段吞掉（2026-11-19 用户投诉灵敏度）。
/// 空唤醒现在会出声回应（WAKE_NUDGE），用户能感知"她在"，10s 不应期可接受。
const EMPTY_WAKE_SUPPRESS: Duration = Duration::from_secs(10);
/// 空唤醒回应话术：喊了名字没接话时的短应答（不走 LLM，零延迟）。产品语义：
/// 喊名字必须有回应，否则用户以为没听到（多次"喊了没反应"投诉的根因）
const WAKE_NUDGE: &str = "嗯？怎么啦？";
/// 碎语回应话术：唤醒被接受、ASR 只转出单音节噪声时的追问（不走 LLM，零延迟）。
/// 2026-11-19 排查实锤：三次"喊了没反应"均因旧行为"短转写静默丢弃"——唤醒已被
/// 接受（用户看到跳跃气泡）却没有任何声音反馈。唤醒被接受后不允许零反馈。
const SHORT_NUDGE: &str = "嗯？我没太听清，再说一遍？";
/// VAD 语音确认窗：命中前的最近 3s 内必须有 VAD 判定的语音，否则视为脉冲噪声。
/// 曾用 1.5s + RMS 能量门双判：实测真人快喊 0.00991 RMS 被能量门误杀（缝隙块），
/// 且个别快喊 VAD 起始帧概率爬升慢、1.5s 窗 edge case 漏 latch——能量门取消
/// （安静噪声同样过不了 VAD 确认，职责重复），窗口放宽 3s 覆盖 KWS 检测延迟。
const SPEECH_CONFIRM_WINDOW: Duration = Duration::from_secs(3);

/// 块 RMS（16k 单声道 f32）
fn chunk_rms(chunk: &[f32]) -> f32 {
    let sq: f64 = chunk.iter().map(|v| (*v as f64) * (*v as f64)).sum();
    (sq / chunk.len().max(1) as f64).sqrt() as f32
}

/// 启动语音引擎（装配层唯一入口，进程常驻）。失败只记日志不 panic——
/// 无麦克风的环境桌宠其余功能不受影响（装配层不重试）。
/// behavior_ctrl：语音调节指令（voice/intent.rs）发往行为引擎的控制通道
pub fn start(
    bus: EventBus,
    comments: Receiver<String>,
    behavior_ctrl: SyncSender<crate::behavior::Control>,
) {
    thread::spawn(move || {
        if let Err(e) = run(bus, comments, behavior_ctrl) {
            tracing::error!("voice engine stopped: {e:#}");
        }
    });
}

fn run(
    bus: EventBus,
    comments: Receiver<String>,
    behavior_ctrl: SyncSender<crate::behavior::Control>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
    let mut capture = CaptureHandle::new(device, config, tx)?;
    capture.rebuild()?;
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
        tts: &mut tts,
        player: &mut player,
    };
    consume(bus, &mut parts, rx, &mut capture, comments, &behavior_ctrl);
    Ok(())
}

/// 采集流句柄（待办 #27 看门狗）：持有重建所需的一切参数，断流时引擎线程
/// 可自行重建（无需重启应用）。Windows 会在设备重平衡时静默暂停采集流
/// （12:08 实机：唤醒后 1.3s 断流 ~10s，无错误回调）。
struct CaptureHandle {
    device: cpal::Device,
    config: cpal::StreamConfig,
    format: cpal::SampleFormat,
    channels: usize,
    in_rate: f64,
    tx: SyncSender<Vec<f32>>,
    /// None = 已停止/重建中；drop Stream 即停采集（cpal 保活语义）
    stream: Option<cpal::Stream>,
}

impl CaptureHandle {
    fn new(
        device: cpal::Device,
        config: cpal::SupportedStreamConfig,
        tx: SyncSender<Vec<f32>>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let handle = Self {
            channels: config.channels() as usize,
            in_rate: config.sample_rate().0 as f64,
            format: config.sample_format(),
            config: config.config(),
            device,
            tx,
            stream: None,
        };
        Ok(handle)
    }

    /// 停旧流建新流。失败保留旧流（若还在）或返回 Err（调用方记日志，下轮重试）
    fn rebuild(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.stream = None; // 先停：同一个设备上重复建流可能冲突
        let stream = match self.format {
            cpal::SampleFormat::F32 => build_stream::<f32>(
                &self.device,
                &self.config,
                self.channels,
                self.tx.clone(),
                self.in_rate,
            )?,
            cpal::SampleFormat::I16 => build_stream::<i16>(
                &self.device,
                &self.config,
                self.channels,
                self.tx.clone(),
                self.in_rate,
            )?,
            other => return Err(format!("不支持的采样格式 {other:?}").into()),
        };
        stream.play()?;
        self.stream = Some(stream);
        tracing::info!("voice capture: stream rebuilt");
        Ok(())
    }
}
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

/// 引擎常驻线程的可变组件集合（consume 参数打包，避免长参数列表）。
/// tts/player 以 Option 所有权传入：输出设备可能在使用中恢复（耳机插拔），
/// player 缺失时 speak 前 lazy 重建（见 consume 内注释）。
struct EngineParts<'a> {
    spotter: &'a mut KeywordSpotter,
    vad: &'a mut Vad,
    asr: &'a Recognizer,
    thinker: Option<&'a mut crate::llm::Thinker>,
    tts: &'a mut Option<super::tts::TtsSidecar>,
    player: &'a mut Option<super::player::Player>,
}

/// 看门狗阈值：超过此时长没收到任何采集块 = 断流（正常节奏是 0.1s 一块，
/// recv 超时仅 200ms）。引擎阻塞期（ASR/LLM 思考最长 ~30s）不算断流，
/// 由 in_blocking 豁免——否则思考中会被误报重建。
const CAPTURE_STALL: Duration = Duration::from_secs(5);
/// 唤醒回填环容量（8 × 0.1s = 0.8s）：待办 #12，KWS 命中瞬间 vad.reset()
/// 会清掉唤醒词起音所在的更早音频，转写首字丢失；环里留最近音频，命中后
/// 把尾巴回填给 VAD。
const RING_CAPACITY: usize = 8;
/// 回填长度：命中块之前 0.4s（唤醒词起音最多落在这么远）
const REPLAY_CHUNKS: usize = 4;

/// 引擎常驻线程：状态机分发 0.1s 块——IDLE/SPEAKING 喂 KWS，LISTENING 喂 VAD。
fn consume(
    bus: EventBus,
    parts: &mut EngineParts,
    rx: Receiver<Vec<f32>>,
    capture: &mut CaptureHandle,
    comments: Receiver<String>,
    behavior_ctrl: &SyncSender<crate::behavior::Control>,
) {
    let spotter = &mut *parts.spotter;
    let vad = &mut *parts.vad;
    let asr = parts.asr;
    let thinker = &mut parts.thinker;
    let tts = &mut *parts.tts;
    let player = &mut *parts.player;
    let mut buf = Vec::with_capacity(CHUNK_SAMPLES);
    // VAD 侧对齐缓冲：0.1s 块（1600）非 512 倍数，攒到 4×512 再喂
    let mut vad_buf: Vec<f32> = Vec::new();
    // 初始即允许首次命中（now() - cooldown）
    let mut last_wake = Instant::now() - WAKE_COOLDOWN;
    // 最近一次 VAD 判定为语音的时间（None = 尚无）
    let mut last_speech: Option<Instant> = None;
    // 空唤醒抑制截止戳：None = 未抑制（初始不抑制，避免启动后 30s 不应期）
    let mut suppress_until: Option<Instant> = None;
    let mut state = State::Idle;
    let mut meter = LevelMeter::new();
    // 看门狗状态：上次收到采集块的时间。引擎阻塞期（ASR/LLM 思考）本就不在
    // recv 上，看门狗无从误报——回到 recv 时会先收到通道里积压的块， Healthy
    // 流不会触发；真断流则任何时刻回到 recv 都会立刻暴露（stalled 日志=此刻往前
    // 都没块，含阻塞期，重建无害——流是真死了）
    let mut last_chunk_at = Instant::now();
    // #12 唤醒回填环：最近 RING_CAPACITY 个 IDLE/SPEAKING 态的块
    let mut ring: std::collections::VecDeque<Vec<f32>> = std::collections::VecDeque::new();
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(chunk) => {
                last_chunk_at = Instant::now();
                buf.extend_from_slice(&chunk);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // 看门狗：超过 CAPTURE_STALL 收不到块 = 采集被系统静默暂停
                // （坑：无错误回调）。单线程模型下引擎阻塞期本就不在 recv 上，
                // 无需豁免标志；重建流自愈，失败按阈值间隔重试
                if last_chunk_at.elapsed() >= CAPTURE_STALL {
                    tracing::error!(
                        stalled_ms = last_chunk_at.elapsed().as_millis(),
                        "voice capture stalled, rebuilding stream"
                    );
                    if let Err(e) = capture.rebuild() {
                        tracing::error!("voice capture rebuild failed: {e:#}");
                    }
                    last_chunk_at = Instant::now();
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        // 行为引擎台词注入（M1-5）：仅 IDLE 态接收——不打断倾听与对话，
        // 说正中（SPEAKING）的评论由行为侧闸控丢弃（通道满即丢）
        if matches!(state, State::Idle) {
            if let Ok(text) = comments.try_recv() {
                ensure_player(player);
                match speak_reply(tts.as_mut(), player.as_mut(), &text) {
                    Ok(()) => {
                        // 复用 ReplyReady 让前端/调试页看到台词文本（M1-6 气泡 UI 吃同一事件）
                        publish_reply(&bus, &text, 0);
                        tracing::info!("behavior comment speaking");
                        state = State::Speaking;
                    }
                    Err(e) => tracing::warn!("behavior comment speak failed: {e:#}"),
                }
            }
        }
        while buf.len() >= CHUNK_SAMPLES {
            let chunk: Vec<f32> = buf.drain(..CHUNK_SAMPLES).collect();
            match state {
                State::Idle | State::Speaking => {
                    // 注意：曾在此加"SMTC 播放中暂停 KWS"（commit e526cbb），被用户否决
                    // （2026-11-19："播放的时候就先那样"）——放歌时唤醒照常工作，
                    // 歌词谐音误唤醒靠门控兜着，正经解法是 AEC 回声消除（待办 #11）。
                    //
                    // VAD 常喂（IDLE/SPEAKING 也喂）：KWS 命中必须落在"语音上下文"
                    // 里才被接受——真人喊"美丽"必被 VAD 判为语音，而键盘敲击/
                    // 椅子声等脉冲噪声的能量虽高（实测 RMS 0.06-0.09 也触发 KWS），
                    // VAD 不认为是语音。段在此态只是副产品，直接丢弃。
                    feed_vad(vad, &mut vad_buf, &chunk);
                    if vad.in_speech() {
                        last_speech = Some(Instant::now());
                    }
                    while vad.pop_segment().is_some() {}
                    // #12 回填环：只积累 IDLE/SPEAKING 态的块（LISTENING 的音频
                    // 属已唤醒会话，不该回填进下一轮）。push 在命中判定之前：
                    // 命中块本身不进环（该轮唤醒里 ring.clear() 会清掉它）。
                    ring.push_back(chunk.clone());
                    if ring.len() > RING_CAPACITY {
                        ring.pop_front();
                    }
                    let suppressed = suppress_until.is_some_and(|t| Instant::now() < t);
                    if let Some(hit) = spotter.accept_waveform(&chunk) {
                        let rms = chunk_rms(&chunk);
                        let speech_ago_ms = last_speech.map(|t| t.elapsed().as_millis() as u64);
                        let in_speech_ctx = speech_ago_ms
                            .is_some_and(|ms| ms <= SPEECH_CONFIRM_WINDOW.as_millis() as u64);
                        if !in_speech_ctx {
                            // 非语音上下文命中：脉冲噪声（敲击/碰撞）能量高但不是话。
                            // 带 VAD latch 年龄诊断：真人喊也被拦说明 VAD 漏判快喊，
                            // 下一步调 silero threshold（0.25 → 0.2 方向）
                            tracing::info!(
                                rms = format_args!("{:.5}", rms),
                                speech_ago_ms = speech_ago_ms.unwrap_or(u64::MAX),
                                "wake hit rejected: no speech context"
                            );
                            last_wake = Instant::now();
                        } else if suppressed {
                            tracing::info!(
                                rms = format_args!("{:.5}", rms),
                                "wake hit suppressed: empty-wake quiet period"
                            );
                            last_wake = Instant::now();
                        } else if last_wake.elapsed() >= WAKE_COOLDOWN {
                            last_wake = Instant::now();
                            // 打断（barge-in）：播放中命中唤醒词 → 停播并通知前端收嘴
                            if matches!(state, State::Speaking) {
                                if let Some(p) = player.as_mut() {
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
                            // 命中块尾部（唤醒词收音）也进 VAD：min_speech 会滤掉过短残渣。
                            // #12 回填：reset 会清掉唤醒词起音所在的更早音频（起音在
                            // 命中块之前的块里），不回填则转写首字丢失——把环里命中前
                            // 0.4s 的块重喂给 VAD（这些块 reset 前已喂过，回放无副作用）
                            vad.reset();
                            vad_buf.clear();
                            let keep = ring.len().saturating_sub(REPLAY_CHUNKS);
                            for old in ring.iter().skip(keep) {
                                vad_buf.extend_from_slice(old);
                            }
                            ring.clear();
                            tracing::info!(
                                "wake hit keyword={:?}, entering listening",
                                hit.keyword
                            );
                            state = State::Listening {
                                since: Instant::now(),
                            };
                            feed_vad(vad, &mut vad_buf, &chunk);
                        }
                    }
                    // 播完检测：输出回调把"队列播空"翻成 !is_playing，这里接成事件
                    if matches!(state, State::Speaking) {
                        let finished = match player.as_mut() {
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
                        let transcribe_result = asr.transcribe(&seg.samples);
                        match transcribe_result {
                            Ok(text) => {
                                tracing::info!(
                                    elapsed_ms = t0.elapsed().as_millis(),
                                    text = %text,
                                    "asr transcript"
                                );
                                publish_transcript(&bus, &text, duration_ms);
                                // 碎语门：环境噪声凑出的单音节转写（"I."/"。"等）不触发
                                // 思考与出声——否则静音环境下桌宠会"自言自语"（2026-11-19 用户投诉）
                                let meaningful =
                                    text.chars().filter(|c| c.is_alphanumeric()).count() >= 2;
                                if !meaningful {
                                    // 与空唤醒超时 nudge 同机制：出声追问 + 10s 抑制压
                                    // 噪声风暴。抑制无论 TTS 成败都生效（播放器缺失时
                                    // 也要压住连续自触发）
                                    tracing::info!(
                                        text = %text,
                                        "transcript too short, nudge instead of silent skip"
                                    );
                                    suppress_until = Some(Instant::now() + EMPTY_WAKE_SUPPRESS);
                                }
                                // 流式出声（#23-b + #32 逗号软切）：LLM 边生成边按
                                // 气口合成入队，首音 ≈ 口癖尾巴生成+合成（~1.5–3s）
                                // 而非整句完（8–17s）。期间引擎线程在流内，KWS 暂停
                                // ——打断窗口只有 SPEAKING 态（同非流式思考阻塞期，
                                // barge-in 语义不回归）。
                                let mut audio_enqueued = false;
                                let mut first_audio_ms: Option<u128> = None;
                                if meaningful {
                                    ensure_player(player);
                                }
                                // M1-5 语音调节指令：命中即拦截——不送 LLM 闲聊，
                                // 切行为档位 + 模板确认台词（"今天别吵我"/"多陪我聊聊"）
                                let mut tier_command = false;
                                if meaningful {
                                    if let Some((tier, ack)) =
                                        super::intent::match_tier_command(&text)
                                    {
                                        tier_command = true;
                                        if let Err(e) = behavior_ctrl
                                            .try_send(crate::behavior::Control::SetTier(tier))
                                        {
                                            tracing::error!(
                                                "tier command control send failed: {e}"
                                            );
                                        }
                                        tracing::info!(
                                            ?tier,
                                            text = %text,
                                            "tier command matched, intercept before llm"
                                        );
                                        if speak_reply(tts.as_mut(), player.as_mut(), ack).is_ok() {
                                            audio_enqueued = true;
                                        }
                                    }
                                }
                                if meaningful && !tier_command {
                                    if let Some(thinker) = thinker.as_deref_mut() {
                                        let t1 = Instant::now();
                                        let think_result =
                                            thinker.think_streaming(&text, |sentence| {
                                                match speak_reply(
                                                    tts.as_mut(),
                                                    player.as_mut(),
                                                    sentence,
                                                ) {
                                                    Ok(()) => {
                                                        if first_audio_ms.is_none() {
                                                            first_audio_ms =
                                                                Some(t1.elapsed().as_millis());
                                                            tracing::info!(
                                                                elapsed_ms =
                                                                    t1.elapsed().as_millis(),
                                                                "tts first audio enqueued"
                                                            );
                                                        }
                                                        audio_enqueued = true;
                                                    }
                                                    Err(e) => {
                                                        tracing::error!(
                                                        "tts speak failed, skip sentence: {e:#}"
                                                    )
                                                    }
                                                }
                                            });
                                        match think_result {
                                            Ok(reply) => {
                                                tracing::info!(
                                                    latency_ms = t1.elapsed().as_millis(),
                                                    text = %reply,
                                                    "llm reply (streamed)"
                                                );
                                                publish_reply(
                                                    &bus,
                                                    &reply,
                                                    t1.elapsed().as_millis(),
                                                );
                                            }
                                            Err(e) => {
                                                // 流式失败兜底：整句兜底话术走一次性合成
                                                tracing::error!("think failed: {e:#}");
                                                let fallback =
                                                    crate::llm::FALLBACK_REPLY.to_string();
                                                if speak_reply(
                                                    tts.as_mut(),
                                                    player.as_mut(),
                                                    &fallback,
                                                )
                                                .is_ok()
                                                {
                                                    audio_enqueued = true;
                                                }
                                                publish_reply(
                                                    &bus,
                                                    &fallback,
                                                    t1.elapsed().as_millis(),
                                                );
                                            }
                                        }
                                    }
                                }
                                // 碎语：出声追问（用户可再喊一次名字重试）；
                                // 正常：有音频则播完回 IDLE，无音频（合成失败）直接回
                                state = if !meaningful {
                                    if speak_reply(tts.as_mut(), player.as_mut(), SHORT_NUDGE)
                                        .is_ok()
                                    {
                                        State::Speaking
                                    } else {
                                        State::Idle
                                    }
                                } else if audio_enqueued {
                                    State::Speaking
                                } else {
                                    State::Idle
                                };
                            }
                            Err(e) => tracing::error!("asr failed: {e:#}"),
                        }
                    } else if since.elapsed() >= MAX_LISTENING {
                        // 唤醒后 12s 无转写：大概率是"只喊了名字没接话"（真实用法，
                        // 用户想确认她听到了）——出声短应答 + 10s 抑制压住噪声风暴。
                        // 播放器不可用时静默回 IDLE（老行为）。
                        tracing::info!(
                            "listening timeout, nudge (empty wake, suppress {:?})",
                            EMPTY_WAKE_SUPPRESS
                        );
                        suppress_until = Some(Instant::now() + EMPTY_WAKE_SUPPRESS);
                        state = if speak_reply(tts.as_mut(), player.as_mut(), WAKE_NUDGE).is_ok() {
                            State::Speaking
                        } else {
                            State::Idle
                        };
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

/// 播放器 lazy 重建（耳机插拔/音频栈自愈后输出设备可用）：对话路径与
/// 行为评论注入共用——player 缺失时重建一次，不重启应用
fn ensure_player(player: &mut Option<super::player::Player>) {
    if player.is_none() {
        match super::player::Player::new() {
            Ok(p) => {
                tracing::info!("tts player: late bind ok");
                *player = Some(p);
            }
            Err(e) => tracing::warn!("tts player: late bind failed, silent reply: {e:#}"),
        }
    }
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
