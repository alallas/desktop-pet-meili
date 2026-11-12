//! 唤醒词引擎（KWS）：sherpa-onnx C API 的薄 FFI 封装。
//!
//! 链接方式同 sqlite（坑 25 模式）：GitHub 官方预编译 DLL + 导入库（tools/sherpa-onnx/，
//! dev 文档负责下载），本机 gcc 5.3 编不了 sherpa-onnx 的 C++（坑 25b 同类）。
//!
//! ⚠️ FFI 结构体布局逐字段抄自 `c-api.h`（v2.0.1，tools/sherpa-onnx/.../include/）：
//! 升级 sherpa-onnx 版本必须对照新头文件复核本文件，Rust 侧不保证跨版本 ABI。
//! allow(dead_code)：FFI 结构是 c-api.h v2.0.1 全量布局，M1-3.2 接线后逐项消费；现在只填 transducer 一支。
#![allow(dead_code)]

use std::ffi::{c_char, c_void, CString};

// ---------- FFI 结构（repr(C)，布局 = c-api.h v2.0.1） ----------

#[repr(C)]
struct SherpaOnnxFeatureConfig {
    sample_rate: i32,
    feature_dim: i32,
}

#[repr(C)]
struct SherpaOnnxOnlineTransducerModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    joiner: *const c_char,
}

#[repr(C)]
struct SherpaOnnxOnlineParaformerModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
}

#[repr(C)]
struct SherpaOnnxOnlineZipformer2CtcModelConfig {
    model: *const c_char,
}

#[repr(C)]
struct SherpaOnnxOnlineNemoCtcModelConfig {
    model: *const c_char,
}

#[repr(C)]
struct SherpaOnnxOnlineToneCtcModelConfig {
    model: *const c_char,
}

#[repr(C)]
struct SherpaOnnxOnlineModelConfig {
    transducer: SherpaOnnxOnlineTransducerModelConfig,
    paraformer: SherpaOnnxOnlineParaformerModelConfig,
    zipformer2_ctc: SherpaOnnxOnlineZipformer2CtcModelConfig,
    tokens: *const c_char,
    num_threads: i32,
    provider: *const c_char,
    debug: i32,
    model_type: *const c_char,
    modeling_unit: *const c_char,
    bpe_vocab: *const c_char,
    tokens_buf: *const c_char,
    tokens_buf_size: i32,
    nemo_ctc: SherpaOnnxOnlineNemoCtcModelConfig,
    t_one_ctc: SherpaOnnxOnlineToneCtcModelConfig,
}

#[repr(C)]
struct SherpaOnnxKeywordSpotterConfig {
    feat_config: SherpaOnnxFeatureConfig,
    model_config: SherpaOnnxOnlineModelConfig,
    max_active_paths: i32,
    num_trailing_blanks: i32,
    keywords_score: f32,
    keywords_threshold: f32,
    keywords_file: *const c_char,
    keywords_buf: *const c_char,
    keywords_buf_size: i32,
}

#[repr(C)]
struct SherpaOnnxKeywordResult {
    keyword: *const c_char,
    tokens: *const c_char,
    tokens_arr: *const *const c_char,
    count: i32,
    timestamps: *const f32,
    start_time: f32,
    json: *const c_char,
}

#[repr(C)]
struct SherpaOnnxWave {
    samples: *const f32,
    sample_rate: i32,
    num_samples: i32,
}

#[link(name = "sherpa-onnx-c-api")]
extern "C" {
    fn SherpaOnnxCreateKeywordSpotter(config: *const SherpaOnnxKeywordSpotterConfig)
        -> *mut c_void;
    fn SherpaOnnxDestroyKeywordSpotter(spotter: *const c_void);
    fn SherpaOnnxCreateKeywordStream(spotter: *const c_void) -> *mut c_void;
    fn SherpaOnnxDestroyOnlineStream(stream: *const c_void);
    fn SherpaOnnxOnlineStreamAcceptWaveform(
        stream: *const c_void,
        sample_rate: i32,
        samples: *const f32,
        n: i32,
    );
    fn SherpaOnnxIsKeywordStreamReady(spotter: *const c_void, stream: *const c_void) -> i32;
    fn SherpaOnnxDecodeKeywordStream(spotter: *const c_void, stream: *const c_void);
    fn SherpaOnnxGetKeywordResult(
        spotter: *const c_void,
        stream: *const c_void,
    ) -> *const SherpaOnnxKeywordResult;
    fn SherpaOnnxDestroyKeywordResult(result: *const SherpaOnnxKeywordResult);
    fn SherpaOnnxResetKeywordStream(spotter: *const c_void, stream: *const c_void);
    fn SherpaOnnxReadWave(filename: *const c_char) -> *const SherpaOnnxWave;
    fn SherpaOnnxFreeWave(wave: *const SherpaOnnxWave);
}

// ---------- 安全封装 ----------

/// 唤醒词命中（从 SherpaOnnxKeywordResult 拷出的数据）
#[derive(Debug)]
pub struct KeywordHit {
    pub keyword: String,
    pub start_time: f32,
}

pub struct KeywordSpotter {
    spotter: *mut c_void,
    stream: *mut c_void,
}

impl KeywordSpotter {
    /// 用 kws-wenetspeech 模型（zipformer transducer）+ 关键词文件构建。
    pub fn new(model_dir: &str, keywords_file: &str) -> Result<Self, String> {
        let c = |s: &str| CString::new(s).expect("含 NUL 的路径");
        let encoder = c(&format!(
            "{model_dir}/encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx"
        ));
        let decoder = c(&format!(
            "{model_dir}/decoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx"
        ));
        let joiner = c(&format!(
            "{model_dir}/joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx"
        ));
        let tokens = c(&format!("{model_dir}/tokens.txt"));
        let keywords = c(keywords_file);
        let provider = c("cpu");

        let model_config = SherpaOnnxOnlineModelConfig {
            transducer: SherpaOnnxOnlineTransducerModelConfig {
                encoder: encoder.as_ptr(),
                decoder: decoder.as_ptr(),
                joiner: joiner.as_ptr(),
            },
            paraformer: SherpaOnnxOnlineParaformerModelConfig {
                encoder: std::ptr::null(),
                decoder: std::ptr::null(),
            },
            zipformer2_ctc: SherpaOnnxOnlineZipformer2CtcModelConfig {
                model: std::ptr::null(),
            },
            tokens: tokens.as_ptr(),
            num_threads: 1,
            provider: provider.as_ptr(),
            debug: 0,
            model_type: std::ptr::null(),
            modeling_unit: std::ptr::null(),
            bpe_vocab: std::ptr::null(),
            tokens_buf: std::ptr::null(),
            tokens_buf_size: 0,
            nemo_ctc: SherpaOnnxOnlineNemoCtcModelConfig {
                model: std::ptr::null(),
            },
            t_one_ctc: SherpaOnnxOnlineToneCtcModelConfig {
                model: std::ptr::null(),
            },
        };
        // 文档示例参数（c-api.h）：score 3.0 / threshold 0.1 / max_active_paths 4
        let config = SherpaOnnxKeywordSpotterConfig {
            feat_config: SherpaOnnxFeatureConfig {
                sample_rate: 16000,
                feature_dim: 80,
            },
            model_config,
            max_active_paths: 4,
            num_trailing_blanks: 1,
            keywords_score: 3.0,
            keywords_threshold: 0.1,
            keywords_file: keywords.as_ptr(),
            keywords_buf: std::ptr::null(),
            keywords_buf_size: 0,
        };
        let spotter = unsafe { SherpaOnnxCreateKeywordSpotter(&config) };
        if spotter.is_null() {
            return Err("SherpaOnnxCreateKeywordSpotter 返回空（模型路径或配置错误）".into());
        }
        let stream = unsafe { SherpaOnnxCreateKeywordStream(spotter) };
        if stream.is_null() {
            unsafe { SherpaOnnxDestroyKeywordSpotter(spotter) };
            return Err("SherpaOnnxCreateKeywordStream 返回空".into());
        }
        Ok(Self { spotter, stream })
    }

    /// 喂一段 16k 单声道采样，返回本轮新命中的关键词。
    /// 命中后内部自动 reset，可继续喂下一段。
    pub fn accept_waveform(&mut self, samples: &[f32]) -> Option<KeywordHit> {
        unsafe {
            SherpaOnnxOnlineStreamAcceptWaveform(
                self.stream,
                16000,
                samples.as_ptr(),
                samples.len() as i32,
            );
            while SherpaOnnxIsKeywordStreamReady(self.spotter, self.stream) == 1 {
                SherpaOnnxDecodeKeywordStream(self.spotter, self.stream);
            }
            let result = SherpaOnnxGetKeywordResult(self.spotter, self.stream);
            if result.is_null() {
                return None;
            }
            let hit = if (*result).keyword.is_null() {
                None
            } else {
                let keyword = std::ffi::CStr::from_ptr((*result).keyword)
                    .to_string_lossy()
                    .into_owned();
                // 空串 = 本轮无命中（GetKeywordResult 总是返回结果对象）
                if keyword.is_empty() {
                    None
                } else {
                    Some(KeywordHit {
                        keyword,
                        start_time: (*result).start_time,
                    })
                }
            };
            SherpaOnnxDestroyKeywordResult(result);
            if hit.is_some() {
                SherpaOnnxResetKeywordStream(self.spotter, self.stream);
            }
            hit
        }
    }

    /// 读 wav 文件（经 sherpa 的 ReadWave，单声道 16bit PCM）
    pub fn read_wave(path: &str) -> Option<(Vec<f32>, i32)> {
        let c = CString::new(path).ok()?;
        unsafe {
            let wave = SherpaOnnxReadWave(c.as_ptr());
            if wave.is_null() {
                return None;
            }
            let w = &*wave;
            let samples = std::slice::from_raw_parts(w.samples, w.num_samples as usize).to_vec();
            let sr = w.sample_rate;
            SherpaOnnxFreeWave(wave);
            Some((samples, sr))
        }
    }
}

impl Drop for KeywordSpotter {
    fn drop(&mut self) {
        unsafe {
            SherpaOnnxDestroyOnlineStream(self.stream);
            SherpaOnnxDestroyKeywordSpotter(self.spotter);
        }
    }
}

// Send：裸指针只在自有线程消费（事件总线同款约定，采集器线程模型）
unsafe impl Send for KeywordSpotter {}

#[cfg(test)]
mod tests {
    use super::*;

    /// M1-3.1 spike：Rust FFI + 预编译 DLL + M0 模型，喂"美丽"录音必须命中。
    /// 通过 = 本机链接/ABI/模型链路全通，语音管线可以在这个地基上盖。
    #[test]
    fn kws_detects_meili_in_m0_sample() {
        let root = env!("CARGO_MANIFEST_DIR");
        let mut spotter = KeywordSpotter::new(
            &format!("{root}/../models/kws"),
            &format!("{root}/../models/kws/keywords.txt"),
        )
        .expect("构建 KWS 引擎");
        let (samples, sr) =
            KeywordSpotter::read_wave(&format!("{root}/../models/test_wavs/wake_meili.wav"))
                .expect("读 wake_meili.wav");
        assert_eq!(sr, 16000, "KWS 模型要求 16k 采样");

        let mut hits = Vec::new();
        // 0.2s 一块喂（3200 采样 @16k），模拟流式
        for chunk in samples.chunks(3200) {
            if let Some(hit) = spotter.accept_waveform(chunk) {
                hits.push(hit);
            }
        }
        assert!(
            hits.iter().any(|h| h.keyword.contains("美丽")),
            "M0 录的唤醒词样本必须命中一次，实际命中: {hits:?}"
        );
    }
}
