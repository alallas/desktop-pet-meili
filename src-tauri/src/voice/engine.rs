//! 语音常驻引擎（M1-3.2）：麦克风采集（cpal）→ 重采样 16k → KWS 唤醒 → 事件总线。
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

use super::kws::KeywordSpotter;

/// 喂给 KWS 的块大小（0.1s @16k；M0 Python demo 用 0.2s，流式减半降低命中延迟）
const CHUNK_SAMPLES: usize = 1600;
/// 唤醒命中后的冷却：reset 后残余音频可能二次触发，冷却窗口内不再发事件
const WAKE_COOLDOWN: Duration = Duration::from_millis(1500);
/// 采集回调 → 引擎线程的缓冲块数（0.1s/块；20 块 = 2s，背压上限）
const CHANNEL_BLOCKS: usize = 20;

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

    consume(bus, &mut spotter, rx);
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
    let mut resampler = Resampler::new(in_rate);
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

/// 引擎常驻线程：攒 0.1s 块喂 KWS，命中 → 冷却 → WakeWord 事件。
fn consume(bus: EventBus, spotter: &mut KeywordSpotter, rx: Receiver<Vec<f32>>) {
    let mut buf = Vec::with_capacity(CHUNK_SAMPLES);
    // 初始即允许首次命中（now() - cooldown）
    let mut last_wake = Instant::now() - WAKE_COOLDOWN;
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(chunk) => buf.extend_from_slice(&chunk),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        while buf.len() >= CHUNK_SAMPLES {
            let chunk: Vec<f32> = buf.drain(..CHUNK_SAMPLES).collect();
            if let Some(hit) = spotter.accept_waveform(&chunk) {
                if last_wake.elapsed() >= WAKE_COOLDOWN {
                    last_wake = Instant::now();
                    let mut event = bus.event(
                        EventSource::Voice,
                        EventKind::WakeWord,
                        serde_json::json!({ "keyword": hit.keyword }),
                    );
                    // 唤醒事件本体不含语音内容且全程本地处理：隐私档 Minimal
                    event.privacy_tier = PrivacyTier::Minimal;
                    bus.publish(event);
                }
            }
        }
    }
}

/// 线性插值重采样器（设备原生率 → 16k）。
///
/// 相位连续：全局输出序号 `next_out` + 基址 `base`（carry[0] 的全局输入序号），
/// 分块喂与整段喂结果逐采样一致（跨回调连续性有测试钉死）。
/// 输入端本身就是 16k 时退化为直通（尾部留 1 采样延迟，可忽略）。
struct Resampler {
    ratio: f64,      // 输入率 / 16000
    carry: Vec<f32>, // 未消费的尾部采样
    base: u64,       // carry[0] 对应的全局输入采样序号
    next_out: u64,   // 下一个输出序号（全局相位，跨回调连续）
}

impl Resampler {
    fn new(in_rate: f64) -> Self {
        Self {
            ratio: in_rate / 16000.0,
            carry: Vec::new(),
            base: 0,
            next_out: 0,
        }
    }

    /// 喂入一段单声道采样，返回本段可产出的 16k 采样。
    fn push(&mut self, input: &[f32]) -> Vec<f32> {
        self.carry.extend_from_slice(input);
        let n = self.carry.len();
        let mut out = Vec::new();
        if n < 2 || self.ratio <= 0.0 {
            return out;
        }
        loop {
            // 第 next_out 个输出的全局输入位置；插值需要 pos 与 pos+1 两个采样都在 carry 内
            let pos = self.next_out as f64 * self.ratio;
            let idx_global = pos.floor() as u64;
            let idx = idx_global.saturating_sub(self.base) as usize;
            if idx + 1 >= n {
                break;
            }
            let frac = (pos - idx_global as f64) as f32;
            let a = self.carry[idx];
            let b = self.carry[idx + 1];
            out.push(a + (b - a) * frac);
            self.next_out += 1;
        }
        // 消费到最后一个已用输入采样，之前的不再参与后续插值
        if self.next_out > 0 {
            let last_used = ((self.next_out - 1) as f64 * self.ratio).floor() as u64;
            let drop = (last_used + 1 - self.base) as usize;
            self.carry.drain(..drop);
            self.base = last_used + 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16k 直通：正弦一个周期（16 采样）原样通过，无增益无移位
    #[test]
    fn resample_passthrough_at_16k() {
        let mut r = Resampler::new(16000.0);
        let input: Vec<f32> = (0..16)
            .map(|i| (2.0 * std::f64::consts::PI * i as f64 / 16.0).sin() as f32)
            .collect();
        let out = r.push(&input);
        assert_eq!(out.len(), 15); // 留 1 采样给下一次插值
        for (a, b) in out.iter().zip(input.iter()) {
            assert!((a - b).abs() < 1e-6, "16k 直通不应改变采样");
        }
    }

    /// 48k → 16k 三分降采样：输出长度 = 输入/3，能量保持（正弦幅度不塌）
    #[test]
    fn resample_48k_to_16k() {
        let mut r = Resampler::new(48000.0);
        let input: Vec<f32> = (0..4800)
            .map(|i| (2.0 * std::f64::consts::PI * i as f64 / 4800.0 * 8.0).sin() as f32)
            .collect();
        let out = r.push(&input);
        assert_eq!(
            out.len(),
            1600,
            "48k 4800 采样（1s）应产出 1600（0.25s 音频）"
        );
        let peak = out.iter().map(|v| v.abs()).fold(0.0_f32, f32::max);
        assert!(
            peak > 0.9,
            "8 周期正弦峰值应保留（线性插值不破幅度）: {peak}"
        );
    }

    /// 跨回调相位连续：两块分别喂与合并喂，输出逐采样一致（base/next_out 工作正常）
    #[test]
    fn resample_across_callbacks() {
        let full: Vec<f32> = (0..960).map(|i| i as f32 / 960.0).collect();
        let (a, b) = full.split_at(480);
        let mut r_split = Resampler::new(48000.0);
        let out_split = {
            let mut o = r_split.push(a);
            o.extend(r_split.push(b));
            o
        };
        let mut r_full = Resampler::new(48000.0);
        let out_full = r_full.push(&full);
        assert_eq!(out_full.len(), out_split.len(), "分块喂不应改变总输出长度");
        for (i, (f, s)) in out_full.iter().zip(out_split.iter()).enumerate() {
            assert!(
                (f - s).abs() < 1e-4,
                "第 {i} 个采样不一致: full={f} split={s}"
            );
        }
    }
}
