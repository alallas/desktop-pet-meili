//! VAD（语音活动检测）：sherpa-onnx Silero VAD 的薄 FFI 封装。
//!
//! 布局逐字段抄自 `c-api.h`（v2.0.1），与 kws.rs 同约定：升级 sherpa-onnx
//! 版本必须对照新头文件复核，Rust 侧不保证跨版本 ABI。

use std::ffi::{c_char, c_void, CString};

// ---------- FFI 结构（repr(C)，布局 = c-api.h v2.0.1 VAD 段） ----------

#[repr(C)]
struct SherpaOnnxSileroVadModelConfig {
    model: *const c_char,
    threshold: f32,
    min_silence_duration: f32,
    min_speech_duration: f32,
    window_size: i32,
    max_speech_duration: f32,
}

#[repr(C)]
struct SherpaOnnxTenVadModelConfig {
    model: *const c_char,
    threshold: f32,
    min_silence_duration: f32,
    min_speech_duration: f32,
    window_size: i32,
    max_speech_duration: f32,
}

#[repr(C)]
struct SherpaOnnxVadModelConfig {
    silero_vad: SherpaOnnxSileroVadModelConfig,
    sample_rate: i32,
    num_threads: i32,
    provider: *const c_char,
    debug: i32,
    ten_vad: SherpaOnnxTenVadModelConfig,
}

#[repr(C)]
struct SherpaOnnxSpeechSegment {
    start: i64,
    samples: *const f32,
    n: i32,
}

#[link(name = "sherpa-onnx-c-api")]
extern "C" {
    fn SherpaOnnxCreateVoiceActivityDetector(
        config: *const SherpaOnnxVadModelConfig,
        buffer_size_in_seconds: f32,
    ) -> *mut c_void;
    fn SherpaOnnxDestroyVoiceActivityDetector(vad: *const c_void);
    fn SherpaOnnxVoiceActivityDetectorAcceptWaveform(
        vad: *const c_void,
        samples: *const f32,
        n: i32,
    );
    fn SherpaOnnxVoiceActivityDetectorEmpty(vad: *const c_void) -> i32;
    fn SherpaOnnxVoiceActivityDetectorFront(vad: *const c_void) -> *const SherpaOnnxSpeechSegment;
    fn SherpaOnnxDestroySpeechSegment(segment: *const SherpaOnnxSpeechSegment);
    fn SherpaOnnxVoiceActivityDetectorPop(vad: *const c_void);
    fn SherpaOnnxVoiceActivityDetectorClear(vad: *const c_void);
    fn SherpaOnnxVoiceActivityDetectorReset(vad: *const c_void);
}

// ---------- 安全封装 ----------

/// VAD 产出的语音段（样本已拷出，归 Rust 所有）
pub struct SpeechSegment {
    /// 段起点（检测器输入流绝对序号，-1 表示无效）
    pub start: i64,
    pub samples: Vec<f32>,
}

/// 流式 VAD。一次只持有一个检测器实例（引擎线程模型：单线程消费）。
pub struct Vad {
    vad: *mut c_void,
}

impl Vad {
    /// silero_vad.onnx + 文档示例参数（c-api.h）：threshold 0.25 / min_silence 0.5 /
    /// min_speech 0.25 / max_speech 15 / window 512。
    pub fn new(model_path: &str) -> Result<Self, String> {
        let model = CString::new(model_path).map_err(|_| "模型路径含 NUL")?;
        let provider = CString::new("cpu").expect("无 NUL");
        let config = SherpaOnnxVadModelConfig {
            silero_vad: SherpaOnnxSileroVadModelConfig {
                model: model.as_ptr(),
                threshold: 0.25,
                min_silence_duration: 0.5,
                min_speech_duration: 0.25,
                window_size: 512,
                max_speech_duration: 15.0,
            },
            sample_rate: 16000,
            num_threads: 1,
            provider: provider.as_ptr(),
            debug: 0,
            ten_vad: SherpaOnnxTenVadModelConfig {
                model: std::ptr::null(),
                threshold: 0.0,
                min_silence_duration: 0.0,
                min_speech_duration: 0.0,
                window_size: 0,
                max_speech_duration: 0.0,
            },
        };
        let vad = unsafe { SherpaOnnxCreateVoiceActivityDetector(&config, 30.0) };
        if vad.is_null() {
            return Err(
                "SherpaOnnxCreateVoiceActivityDetector 返回空（模型路径或配置错误）".into(),
            );
        }
        Ok(Self { vad })
    }

    /// 喂 16k 单声道采样。n 须为 window_size(512) 整数倍，否则 sherpa 内部断言
    /// （调用方负责对齐，见 engine.rs 的 vad_buf）。
    pub fn accept_waveform(&mut self, samples: &[f32]) {
        debug_assert!(
            samples.len().is_multiple_of(512),
            "VAD 输入须 512 对齐（silero window_size）"
        );
        unsafe {
            SherpaOnnxVoiceActivityDetectorAcceptWaveform(
                self.vad,
                samples.as_ptr(),
                samples.len() as i32,
            );
        }
    }

    /// 弹出当前已完成的语音段（FIFO）。无则 None。
    pub fn pop_segment(&mut self) -> Option<SpeechSegment> {
        unsafe {
            if SherpaOnnxVoiceActivityDetectorEmpty(self.vad) == 1 {
                return None;
            }
            let seg = SherpaOnnxVoiceActivityDetectorFront(self.vad);
            if seg.is_null() {
                return None;
            }
            let s = &*seg;
            let out = if s.samples.is_null() || s.n <= 0 {
                None
            } else {
                Some(SpeechSegment {
                    start: s.start,
                    samples: std::slice::from_raw_parts(s.samples, s.n as usize).to_vec(),
                })
            };
            SherpaOnnxDestroySpeechSegment(seg);
            SherpaOnnxVoiceActivityDetectorPop(self.vad);
            out
        }
    }

    /// 新一轮对话前清空内部状态（丢弃残留分段与缓冲）
    pub fn reset(&mut self) {
        unsafe {
            SherpaOnnxVoiceActivityDetectorClear(self.vad);
            SherpaOnnxVoiceActivityDetectorReset(self.vad);
        }
    }
}

impl Drop for Vad {
    fn drop(&mut self) {
        unsafe { SherpaOnnxDestroyVoiceActivityDetector(self.vad) }
    }
}

// Send：裸指针只在引擎自有线程消费（与 KeywordSpotter 同款约定）
unsafe impl Send for Vad {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::kws::KeywordSpotter;

    /// M1-3.3：VAD FFI 链路——喂 M0 唤醒录音（含"美丽"人声）必须切出 ≥1 个语音段。
    #[test]
    fn vad_segments_speech_from_m0_sample() {
        let root = env!("CARGO_MANIFEST_DIR");
        let mut vad = Vad::new(&format!("{root}/../models/silero_vad.onnx")).expect("构建 VAD");
        let (samples, sr) =
            KeywordSpotter::read_wave(&format!("{root}/../models/test_wavs/wake_meili.wav"))
                .expect("读 wake_meili.wav");
        assert_eq!(sr, 16000);

        // 按 512 对齐整段喂入（VAD 内部缓冲足够容纳）
        for chunk in samples.chunks(512 * 8) {
            let mut buf = chunk.to_vec();
            // 尾块补齐 512 对齐（静音填充）
            while buf.len() % 512 != 0 {
                buf.push(0.0);
            }
            vad.accept_waveform(&buf);
        }
        let seg = vad.pop_segment().expect("M0 唤醒录音应切出至少一个语音段");
        assert!(
            seg.samples.len() >= 16000 / 4,
            "语音段应 ≥0.25s（min_speech），实际 {} 采样",
            seg.samples.len()
        );
    }

    /// 静音输入不产生语音段（负样本：不误切）
    #[test]
    fn vad_silent_input_yields_no_segment() {
        let root = env!("CARGO_MANIFEST_DIR");
        let mut vad = Vad::new(&format!("{root}/../models/silero_vad.onnx")).expect("构建 VAD");
        let silence = vec![0.0_f32; 16000 * 2];
        for chunk in silence.chunks(512 * 8) {
            let mut buf = chunk.to_vec();
            while buf.len() % 512 != 0 {
                buf.push(0.0);
            }
            vad.accept_waveform(&buf);
        }
        assert!(vad.pop_segment().is_none(), "纯静音不应产生语音段");
    }
}
