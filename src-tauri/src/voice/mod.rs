//! 语音管线：状态机（IDLE/LISTENING/…）+ kws/vad/asr/tts/intent 各组件。
//! 组件接口 trait 化；M0-3 Python demo 参数在此产品化（M1-3）。

pub mod kws;
