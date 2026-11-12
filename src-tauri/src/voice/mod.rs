//! 语音管线：状态机（IDLE/LISTENING/…）+ kws/vad/asr/tts/intent 各组件。
//! 组件接口 trait 化；M0-3 Python demo 参数在此产品化（M1-3）。
//!
//! 状态机（M1-3.2 起）：
//! - IDLE：常驻，KWS 常听唤醒词；命中 → 发 WakeWord 事件并切 LISTENING；
//! - LISTENING：VAD 断句（M1-3.3）→ 发 UtteranceEnded → 本地 SenseVoice 转写
//!   （M1-3.4）→ 发 TranscriptReady → 回 IDLE；
//! - THINKING/SPEAKING：接 LLM/TTS 后补。
//!
//! 引擎是进程生命周期常驻线程，无停止接口（与采集器线程模型一致）。
//! 转写阻塞期间（~0.5s）引擎线程不做 KWS——采集通道有 2s 缓冲，漏听窗口可接受；
//! 持续优化方向：转写移入侧线程或 DecodeMultipleOfflineStreams 并行（优化类待办）。

pub mod asr;
pub mod engine;
pub mod kws;
pub mod vad;
