// M1-3.5b 排查工具：用产品同一份代码（parse_pcm16_wav 等价实现 + Resampler）
// 把 Kokoro 24kHz wav 重采样到 48kHz 并落盘，供"系统播放器 vs 应用内播放"对照。
// 用法: cargo run --example resample_wav <in.wav> <out.wav>
// 注意：parse_pcm16_wav 是 player.rs 私有函数，此处复制实现（排查工具允许重复，
// 若修复后保留此工具，应将其提为 pub 或移入 resample 流程共享）。

use std::env;
use std::io::Write;

#[path = "../src/voice/resample.rs"]
mod resample;
use resample::Resampler;

fn parse_pcm16_wav(bytes: &[u8]) -> Result<(f64, Vec<f32>), String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("非 RIFF/WAVE".into());
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
                fmt = u16::from_le_bytes(bytes[body_start..body_start + 2].try_into().unwrap());
                rate =
                    u32::from_le_bytes(bytes[body_start + 4..body_start + 8].try_into().unwrap());
                bits =
                    u16::from_le_bytes(bytes[body_start + 14..body_start + 16].try_into().unwrap());
            }
            b"data" => pcm = Some(body_start..body_end),
            _ => {}
        }
        pos = body_end + (size & 1);
    }
    if fmt != 1 || bits != 16 {
        return Err(format!("非 PCM16: fmt={fmt} bits={bits}"));
    }
    let raw = &bytes[pcm.ok_or("缺 data chunk")?];
    let n = raw.len() / 2;
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        let v = i16::from_le_bytes(raw[2 * i..2 * i + 2].try_into().unwrap());
        samples.push(v as f32 / 32768.0);
    }
    Ok((rate as f64, samples))
}

fn write_pcm16_wav(path: &str, rate: u32, samples: &[f32]) -> std::io::Result<()> {
    let mut b = Vec::new();
    let data_bytes = samples.len() * 2;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_bytes as u32).to_le_bytes());
    b.extend_from_slice(b"WAVE");
    b.extend_from_slice(b"fmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    for s in samples {
        b.extend_from_slice(&((*s * 32767.0) as i16).to_le_bytes());
    }
    std::fs::File::create(path)?.write_all(&b)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let (input, output) = (args.get(1).expect("in.wav"), args.get(2).expect("out.wav"));
    let bytes = std::fs::read(input).expect("读入失败");
    let (in_rate, mono) = parse_pcm16_wav(&bytes).expect("解析失败");
    let mut r = Resampler::new(in_rate, 48000.0);
    let out = r.push(&mono);
    write_pcm16_wav(output, 48000, &out).expect("写出失败");
    println!(
        "in: {in_rate}Hz {}采样 → out: 48000Hz {}采样（{:.1}s）",
        mono.len(),
        out.len(),
        out.len() as f64 / 48000.0
    );
}
