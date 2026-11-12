//! 语音管线：状态机（IDLE/LISTENING/…）+ kws/vad/asr/tts/intent 各组件。
//! 组件接口 trait 化；M0-3 Python demo 参数在此产品化（M1-3）。
//!
//! 状态机（M1-3.2 起）：
//! - IDLE：常驻，KWS 常听唤醒词；命中 → 发 WakeWord 事件，冷却后继续听；
//! - LISTENING：M1-3.3 随 VAD 断句加入（唤醒后采集指令语音）；
//! - THINKING/SPEAKING：接 ASR/LLM/TTS 后补。
//!
//! 引擎是进程生命周期常驻线程，无停止接口（与采集器线程模型一致）。

pub mod engine;
pub mod kws;
pub mod vad;
