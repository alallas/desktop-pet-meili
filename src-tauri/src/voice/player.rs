//! TTS 播放（M1-3.5b）：cpal 输出流 + 共享队列。
//!
//! 架构：cpal 输出只有回调式 API（Windows WASAPI 后端），故"播放"=
//! 把重采样后的采样推进共享队列，输出回调从队列 drain、不足补静音。
//! 引擎线程在 SPEAKING 态继续跑 KWS（打断 barge-in 的前提：命中即 stop() 清队列）。
//!
//! 播完检测：回调发现队列空且 playing → 置 playing=false；引擎线程轮询
//! is_playing() 发现翻转后发 SpeakEnded。事件延迟 ≤ 回调周期，可忽略。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::resample::Resampler;

/// 播放共享状态（输出回调与引擎线程之间）
struct PlayState {
    /// 已按输出格式展开的采样帧（mono 已复制为 N 声道）
    queue: VecDeque<f32>,
    playing: bool,
}

pub struct Player {
    #[allow(dead_code)]
    stream: cpal::Stream, // 保活：drop 即停输出
    state: Arc<Mutex<PlayState>>,
    out_rate: f64,
    channels: usize,
}

impl Player {
    /// 打开输出设备并启动输出流。失败返回 Err（调用方降级：无声音）。
    ///
    /// 双重候选遍历（坑 39）：实测 default_output_config 对输出设备会"虚报"
    /// （按其 build 流必失败 0x8889000F），且默认设备本身可能是失效端点（无线
    /// 耳机连接后扬声器被禁用）——故外层遍历全部输出设备，内层对每个设备先
    /// 试默认配置、失败后遍历 supported_output_configs，第一个建流成功者启用。
    pub fn new() -> Result<Self, String> {
        let host = cpal::default_host();
        let state = Arc::new(Mutex::new(PlayState {
            queue: VecDeque::new(),
            playing: false,
        }));

        let mut tried = 0usize;
        for device in host
            .output_devices()
            .map_err(|e| format!("枚举输出设备失败: {e}"))?
        {
            let dev_name = device.name().unwrap_or_default();
            // 内层候选：默认配置优先，其后枚举设备支持的全部配置
            let mut attempts: Vec<(cpal::StreamConfig, cpal::SampleFormat)> = Vec::new();
            if let Ok(def) = device.default_output_config() {
                attempts.push((def.config(), def.sample_format()));
            }
            if let Ok(ranges) = device.supported_output_configs() {
                for range in ranges {
                    attempts.push((range.with_max_sample_rate().config(), range.sample_format()));
                }
            }
            for (cfg, fmt) in &attempts {
                let res = match fmt {
                    cpal::SampleFormat::F32 => build_output::<f32>(&device, cfg, &state),
                    cpal::SampleFormat::I16 => build_output::<i16>(&device, cfg, &state),
                    _ => continue,
                };
                tried += 1;
                if let Ok(stream) = res {
                    tracing::info!(
                        device = %dev_name,
                        rate = cfg.sample_rate.0,
                        channels = cfg.channels,
                        format = ?fmt,
                        "tts player: output stream up"
                    );
                    stream.play().map_err(|e| format!("播放流启动失败: {e}"))?;
                    return Ok(Self {
                        stream,
                        state,
                        out_rate: cfg.sample_rate.0 as f64,
                        channels: cfg.channels as usize,
                    });
                }
            }
        }
        Err(format!(
            "无任何输出设备可建流（{tried} 个配置尝试均失败）；若刚连接耳机，稍后重试"
        ))
    }

    /// 播放一段 16-bit PCM WAV（Kokoro sidecar 产物，24kHz 单声道）。
    /// 重采样 + 声道展开 + 入队，立即返回；播完由 is_playing() 翻转感知。
    pub fn play_wav(&mut self, wav_bytes: &[u8]) -> Result<(), String> {
        let (in_rate, mono) = parse_pcm16_wav(wav_bytes)?;
        // 每次播放新建重采样器：相位从 0 开始，与上次播放解耦
        let mut resampler = Resampler::new(in_rate, self.out_rate);
        let resampled = resampler.push(&mono);
        // 尾部未消费采样 ≤1 个输入采样，对整句播放可忽略（与 16k 直通同款约定）

        let mut state = self
            .state
            .lock()
            .map_err(|e| format!("播放锁 poison: {e}"))?;
        for s in resampled {
            for _ in 0..self.channels {
                state.queue.push_back(s);
            }
        }
        state.playing = true;
        Ok(())
    }

    /// 打断：清队列 + 立即停（barge-in：播放中命中唤醒词，待办 #11 MVP）
    pub fn stop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.queue.clear();
            state.playing = false;
        }
    }

    /// 是否还在播（引擎线程轮询；回调把"播完"翻成 false）
    pub fn is_playing(&self) -> bool {
        self.state.lock().map(|s| s.playing).unwrap_or(false)
    }
}

/// cpal 输出回调：从共享队列 drain 采样帧，不足补静音；队列空且 playing → 播完翻转。
/// 回调内不分配大块内存（队列预留在 play_wav 侧完成，这里逐帧 pop）。
fn build_output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: &Arc<Mutex<PlayState>>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let state = Arc::clone(state);
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _| {
                let mut st = match state.lock() {
                    Ok(s) => s,
                    Err(_) => return,
                };
                // 队列与 out 同为声道交错布局（play_wav 按 L,R,L,R 展开），逐槽弹一个。
                // 注意：不能按"每帧弹一个再复制到各声道"——入队量是出队量的 channels 倍，
                // 会把播放拖慢 channels 倍（0.5 倍速 + 降调，M1-3.5b 实机踩过）。
                for slot in out.iter_mut() {
                    *slot = T::from_sample(st.queue.pop_front().unwrap_or(0.0));
                }
                if st.playing && st.queue.is_empty() {
                    st.playing = false; // 播完翻转：is_playing() 将返回 false
                }
            },
            |e| tracing::error!("tts playback error: {e}"),
            None,
        )
        .map_err(|e| format!("{e}"))
}

/// 解析 16-bit PCM WAV（RIFF 遍历）。返回 (采样率, f32 单声道采样 [-1,1])。
/// 纯函数独立单测；只支持 PCM（format 1），其他压缩格式显式拒绝。
fn parse_pcm16_wav(bytes: &[u8]) -> Result<(f64, Vec<f32>), String> {
    let err = |m: &str| m.to_string();
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(err("非 RIFF/WAVE 文件"));
    }
    let mut pos = 12;
    let (mut rate, mut bits, mut fmt, mut pcm) = (0u32, 0u16, 0u16, None);
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body_start = pos + 8;
        let body_end = body_start.saturating_add(size).min(bytes.len());
        match id {
            b"fmt " => {
                if body_end - body_start < 16 {
                    return Err(err("fmt chunk 过短"));
                }
                fmt = u16::from_le_bytes(bytes[body_start..body_start + 2].try_into().unwrap());
                rate =
                    u32::from_le_bytes(bytes[body_start + 4..body_start + 8].try_into().unwrap());
                bits =
                    u16::from_le_bytes(bytes[body_start + 14..body_start + 16].try_into().unwrap());
            }
            b"data" => pcm = Some(body_start..body_end),
            _ => {}
        }
        pos = body_end + (size & 1); // chunk 按 2 字节对齐
    }
    if fmt != 1 {
        return Err(format!("非 PCM 格式（fmt={fmt}）"));
    }
    if bits != 16 {
        return Err(format!("非 16-bit（bits={bits}）"));
    }
    let range = pcm.ok_or("缺 data chunk")?;
    let raw = &bytes[range];
    let n = raw.len() / 2;
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        let v = i16::from_le_bytes(raw[2 * i..2 * i + 2].try_into().unwrap());
        samples.push(v as f32 / 32768.0);
    }
    Ok((rate as f64, samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造最小合法 WAV：16-bit PCM 单声道
    fn make_wav(rate: u32, samples: &[i16]) -> Vec<u8> {
        let data_bytes = samples.len() * 2;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_bytes as u32).to_le_bytes());
        b.extend_from_slice(b"WAVE");
        b.extend_from_slice(b"fmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes()); // PCM
        b.extend_from_slice(&1u16.to_le_bytes()); // mono
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
        b.extend_from_slice(&2u16.to_le_bytes()); // block align
        b.extend_from_slice(&16u16.to_le_bytes()); // bits
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data_bytes as u32).to_le_bytes());
        for s in samples {
            b.extend_from_slice(&s.to_le_bytes());
        }
        b
    }

    #[test]
    fn parse_wav_roundtrip() {
        let src = [0i16, 16384, -16384, 32767, -32768];
        let wav = make_wav(24000, &src);
        let (rate, f) = parse_pcm16_wav(&wav).expect("应解析成功");
        assert_eq!(rate, 24000.0);
        assert_eq!(f.len(), 5);
        assert!((f[1] - 0.5).abs() < 1e-3, "16384/32768 ≈ 0.5: {}", f[1]);
        assert!(f[4] < -0.999, "-32768 → ≈ -1: {}", f[4]);
    }

    #[test]
    fn parse_rejects_non_pcm() {
        let mut wav = make_wav(24000, &[0i16; 4]);
        wav[20] = 3; // fmt = IEEE float
        assert!(parse_pcm16_wav(&wav).is_err());
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_pcm16_wav(b"not a wav").is_err());
        assert!(parse_pcm16_wav(b"").is_err());
    }
}
