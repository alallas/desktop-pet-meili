//! 事件总线：标准化事件 `{id, ts, source, kind, payload, privacy_tier}` 的唯一集散地。
//! M1-2 实现：采集器投递 → 防抖 → 订阅分发（WebView 与行为引擎）。
