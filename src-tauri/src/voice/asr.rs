//! ASR（离线转写）：sherpa-onnx SenseVoice 的薄 FFI 封装。
//!
//! 与 kws.rs/vad.rs 同约定：布局逐字段抄自 `c-api.h`（v2.0.1），
//! 升级 sherpa-onnx 版本必须对照新头文件复核 repr(C) 布局。
//!
//! ⚠️ SherpaOnnxOfflineModelConfig 在 v2.0.1 有 26 个字段（字段顺序即模型家族
//! 枚举顺序），中间不允许压缩——拿不准就到 c-api.h 逐行核对。

use std::ffi::{c_char, c_void, CString};

#[repr(C)]
struct SherpaOnnxFeatureConfig {
    sample_rate: i32,
    feature_dim: i32,
}

// ---------- OfflineModelConfig 的子配置（未启用的模型家族全部置空） ----------

#[repr(C)]
struct OfflineTransducerModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    joiner: *const c_char,
}

#[repr(C)]
struct OfflineSingleModelConfig {
    model: *const c_char,
}

#[repr(C)]
struct OfflineWhisperModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    language: *const c_char,
    task: *const c_char,
    tail_paddings: i32,
    enable_token_timestamps: i32,
    enable_segment_timestamps: i32,
}

#[repr(C)]
struct OfflineSenseVoiceModelConfig {
    model: *const c_char,
    language: *const c_char,
    use_itn: i32,
}

#[repr(C)]
struct OfflineMoonshineModelConfig {
    preprocessor: *const c_char,
    encoder: *const c_char,
    uncached_decoder: *const c_char,
    cached_decoder: *const c_char,
    merged_decoder: *const c_char,
}

#[repr(C)]
struct OfflineFireRedAsrModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
}

#[repr(C)]
struct OfflineCanaryModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    src_lang: *const c_char,
    tgt_lang: *const c_char,
    use_pnc: i32,
}

#[repr(C)]
struct OfflineFunAsrNanoModelConfig {
    encoder_adaptor: *const c_char,
    llm: *const c_char,
    embedding: *const c_char,
    tokenizer: *const c_char,
    system_prompt: *const c_char,
    user_prompt: *const c_char,
    max_new_tokens: i32,
    temperature: f32,
    top_p: f32,
    seed: i32,
    language: *const c_char,
    itn: i32,
    hotwords: *const c_char,
}

#[repr(C)]
struct OfflineQwen3AsrModelConfig {
    conv_frontend: *const c_char,
    encoder: *const c_char,
    decoder: *const c_char,
    tokenizer: *const c_char,
    max_total_len: i32,
    max_new_tokens: i32,
    temperature: f32,
    top_p: f32,
    seed: i32,
    hotwords: *const c_char,
}

#[repr(C)]
struct OfflineCohereTranscribeModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    language: *const c_char,
    use_punct: i32,
    use_itn: i32,
}

fn null_single() -> OfflineSingleModelConfig {
    OfflineSingleModelConfig {
        model: std::ptr::null(),
    }
}

/// v2.0.1 OfflineModelConfig 全量布局（26 字段，顺序照抄 c-api.h）
#[repr(C)]
struct SherpaOnnxOfflineModelConfig {
    transducer: OfflineTransducerModelConfig,
    paraformer: OfflineSingleModelConfig,
    nemo_ctc: OfflineSingleModelConfig,
    whisper: OfflineWhisperModelConfig,
    tdnn: OfflineSingleModelConfig,
    tokens: *const c_char,
    num_threads: i32,
    debug: i32,
    provider: *const c_char,
    model_type: *const c_char,
    modeling_unit: *const c_char,
    bpe_vocab: *const c_char,
    telespeech_ctc: *const c_char,
    sense_voice: OfflineSenseVoiceModelConfig,
    moonshine: OfflineMoonshineModelConfig,
    fire_red_asr: OfflineFireRedAsrModelConfig,
    dolphin: OfflineSingleModelConfig,
    zipformer_ctc: OfflineSingleModelConfig,
    canary: OfflineCanaryModelConfig,
    wenet_ctc: OfflineSingleModelConfig,
    omnilingual: OfflineSingleModelConfig,
    medasr: OfflineSingleModelConfig,
    funasr_nano: OfflineFunAsrNanoModelConfig,
    fire_red_asr_ctc: OfflineSingleModelConfig,
    qwen3_asr: OfflineQwen3AsrModelConfig,
    cohere_transcribe: OfflineCohereTranscribeModelConfig,
}

#[repr(C)]
struct SherpaOnnxOfflineLMConfig {
    model: *const c_char,
    scale: f32,
}

#[repr(C)]
struct SherpaOnnxHomophoneReplacerConfig {
    dict_dir: *const c_char,
    lexicon: *const c_char,
    rule_fsts: *const c_char,
}

#[repr(C)]
struct SherpaOnnxOfflineCtcFstDecoderConfig {
    graph: *const c_char,
    max_active: i32,
}

#[repr(C)]
struct SherpaOnnxOfflineRecognizerConfig {
    feat_config: SherpaOnnxFeatureConfig,
    model_config: SherpaOnnxOfflineModelConfig,
    lm_config: SherpaOnnxOfflineLMConfig,
    decoding_method: *const c_char,
    max_active_paths: i32,
    hotwords_file: *const c_char,
    hotwords_score: f32,
    rule_fsts: *const c_char,
    rule_fars: *const c_char,
    blank_penalty: f32,
    hr: SherpaOnnxHomophoneReplacerConfig,
    ctc_fst_decoder_config: SherpaOnnxOfflineCtcFstDecoderConfig,
}

/// 结果结构体只声明用到的首字段（text）：repr(C) 前缀布局，读首字段是安全的
#[repr(C)]
struct SherpaOnnxOfflineRecognizerResult {
    text: *const c_char,
}

#[link(name = "sherpa-onnx-c-api")]
extern "C" {
    fn SherpaOnnxCreateOfflineRecognizer(
        config: *const SherpaOnnxOfflineRecognizerConfig,
    ) -> *mut c_void;
    fn SherpaOnnxDestroyOfflineRecognizer(recognizer: *const c_void);
    fn SherpaOnnxCreateOfflineStream(recognizer: *const c_void) -> *mut c_void;
    fn SherpaOnnxDestroyOfflineStream(stream: *const c_void);
    fn SherpaOnnxAcceptWaveformOffline(
        stream: *const c_void,
        sample_rate: i32,
        samples: *const f32,
        n: i32,
    );
    fn SherpaOnnxDecodeOfflineStream(recognizer: *const c_void, stream: *const c_void);
    fn SherpaOnnxGetOfflineStreamResult(
        stream: *const c_void,
    ) -> *const SherpaOnnxOfflineRecognizerResult;
    fn SherpaOnnxDestroyOfflineRecognizerResult(result: *const SherpaOnnxOfflineRecognizerResult);
}

// ---------- 安全封装 ----------

/// 离线识别器：每次 transcribe 新建 stream（复用 stream 会串上下文，见 M0 待办）。
pub struct Recognizer {
    recognizer: *mut c_void,
}

impl Recognizer {
    /// SenseVoice（zh/en/ja/ko/yue，auto 语言 + ITN 数字归一化）+ greedy_search。
    pub fn new(model_path: &str, tokens_path: &str) -> Result<Self, String> {
        let c = |s: &str| CString::new(s).expect("静态串/调用方路径不含 NUL");
        let model = c(model_path);
        let tokens = c(tokens_path);
        let provider = c("cpu");
        let decoding = c("greedy_search");
        let language = c("auto");

        let model_config = SherpaOnnxOfflineModelConfig {
            transducer: OfflineTransducerModelConfig {
                encoder: std::ptr::null(),
                decoder: std::ptr::null(),
                joiner: std::ptr::null(),
            },
            paraformer: null_single(),
            nemo_ctc: null_single(),
            whisper: OfflineWhisperModelConfig {
                encoder: std::ptr::null(),
                decoder: std::ptr::null(),
                language: std::ptr::null(),
                task: std::ptr::null(),
                tail_paddings: 0,
                enable_token_timestamps: 0,
                enable_segment_timestamps: 0,
            },
            tdnn: null_single(),
            tokens: tokens.as_ptr(),
            num_threads: 2,
            debug: 0,
            provider: provider.as_ptr(),
            model_type: std::ptr::null(),
            modeling_unit: std::ptr::null(),
            bpe_vocab: std::ptr::null(),
            telespeech_ctc: std::ptr::null(),
            sense_voice: OfflineSenseVoiceModelConfig {
                model: model.as_ptr(),
                language: language.as_ptr(),
                use_itn: 1,
            },
            moonshine: OfflineMoonshineModelConfig {
                preprocessor: std::ptr::null(),
                encoder: std::ptr::null(),
                uncached_decoder: std::ptr::null(),
                cached_decoder: std::ptr::null(),
                merged_decoder: std::ptr::null(),
            },
            fire_red_asr: OfflineFireRedAsrModelConfig {
                encoder: std::ptr::null(),
                decoder: std::ptr::null(),
            },
            dolphin: null_single(),
            zipformer_ctc: null_single(),
            canary: OfflineCanaryModelConfig {
                encoder: std::ptr::null(),
                decoder: std::ptr::null(),
                src_lang: std::ptr::null(),
                tgt_lang: std::ptr::null(),
                use_pnc: 0,
            },
            wenet_ctc: null_single(),
            omnilingual: null_single(),
            medasr: null_single(),
            funasr_nano: OfflineFunAsrNanoModelConfig {
                encoder_adaptor: std::ptr::null(),
                llm: std::ptr::null(),
                embedding: std::ptr::null(),
                tokenizer: std::ptr::null(),
                system_prompt: std::ptr::null(),
                user_prompt: std::ptr::null(),
                max_new_tokens: 0,
                temperature: 0.0,
                top_p: 0.0,
                seed: 0,
                language: std::ptr::null(),
                itn: 0,
                hotwords: std::ptr::null(),
            },
            fire_red_asr_ctc: null_single(),
            qwen3_asr: OfflineQwen3AsrModelConfig {
                conv_frontend: std::ptr::null(),
                encoder: std::ptr::null(),
                decoder: std::ptr::null(),
                tokenizer: std::ptr::null(),
                max_total_len: 0,
                max_new_tokens: 0,
                temperature: 0.0,
                top_p: 0.0,
                seed: 0,
                hotwords: std::ptr::null(),
            },
            cohere_transcribe: OfflineCohereTranscribeModelConfig {
                encoder: std::ptr::null(),
                decoder: std::ptr::null(),
                language: std::ptr::null(),
                use_punct: 0,
                use_itn: 0,
            },
        };
        let config = SherpaOnnxOfflineRecognizerConfig {
            feat_config: SherpaOnnxFeatureConfig {
                sample_rate: 16000,
                feature_dim: 80,
            },
            model_config,
            lm_config: SherpaOnnxOfflineLMConfig {
                model: std::ptr::null(),
                scale: 0.0,
            },
            decoding_method: decoding.as_ptr(),
            max_active_paths: 4,
            hotwords_file: std::ptr::null(),
            hotwords_score: 0.0,
            rule_fsts: std::ptr::null(),
            rule_fars: std::ptr::null(),
            blank_penalty: 0.0,
            hr: SherpaOnnxHomophoneReplacerConfig {
                dict_dir: std::ptr::null(),
                lexicon: std::ptr::null(),
                rule_fsts: std::ptr::null(),
            },
            ctc_fst_decoder_config: SherpaOnnxOfflineCtcFstDecoderConfig {
                graph: std::ptr::null(),
                max_active: 0,
            },
        };
        let recognizer = unsafe { SherpaOnnxCreateOfflineRecognizer(&config) };
        if recognizer.is_null() {
            return Err("SherpaOnnxCreateOfflineRecognizer 返回空（模型/tokens 路径错误）".into());
        }
        Ok(Self { recognizer })
    }

    /// 转写一段 16k 单声道语音（整段，离线）。空音频返回空串。
    pub fn transcribe(&self, samples: &[f32]) -> Result<String, String> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        unsafe {
            let stream = SherpaOnnxCreateOfflineStream(self.recognizer);
            if stream.is_null() {
                return Err("SherpaOnnxCreateOfflineStream 返回空".into());
            }
            SherpaOnnxAcceptWaveformOffline(stream, 16000, samples.as_ptr(), samples.len() as i32);
            SherpaOnnxDecodeOfflineStream(self.recognizer, stream);
            let result = SherpaOnnxGetOfflineStreamResult(stream);
            let text = if result.is_null() || (*result).text.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr((*result).text)
                    .to_string_lossy()
                    .into_owned()
            };
            if !result.is_null() {
                SherpaOnnxDestroyOfflineRecognizerResult(result);
            }
            SherpaOnnxDestroyOfflineStream(stream);
            Ok(text)
        }
    }
}

impl Drop for Recognizer {
    fn drop(&mut self) {
        unsafe { SherpaOnnxDestroyOfflineRecognizer(self.recognizer) }
    }
}

// Send：裸指针只在引擎自有线程消费（与 KeywordSpotter/Vad 同款约定）
unsafe impl Send for Recognizer {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::kws::KeywordSpotter;
    use std::time::Duration;

    /// M1-3.4：SenseVoice 转写 M0 唤醒录音，必须出含"美丽"的文本。
    /// 同时给转写耗时封顶（离线 int8 在 CPU 上应 <2s/秒音频）。
    #[test]
    fn asr_transcribes_m0_sample() {
        let root = env!("CARGO_MANIFEST_DIR");
        let recognizer = Recognizer::new(
            &format!("{root}/../models/sense-voice.int8.onnx"),
            &format!("{root}/../models/sense-voice-tokens.txt"),
        )
        .expect("构建识别器");
        let (samples, _sr) =
            KeywordSpotter::read_wave(&format!("{root}/../models/test_wavs/wake_meili.wav"))
                .expect("读 wake_meili.wav");

        let t0 = std::time::Instant::now();
        let text = recognizer.transcribe(&samples).expect("转写");
        let elapsed = t0.elapsed();
        tracing::info!("asr test: {text:?} in {elapsed:?}");
        assert!(
            text.contains("美丽"),
            "唤醒录音转写应含「美丽」，实际 {text:?}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "1 秒音频转写应远快于 5s: {elapsed:?}"
        );
    }
}
