//! 事件总线：采集器 → WebView/行为引擎 的唯一集散地。
//!
//! 分层职责（别把业务冷却放这里）：
//! - 本层只做**传输级削峰**——高频率 kind（窗口移动类）按 key 冷却，防 LOCATIONCHANGE
//!   拖动洪峰打爆前端；
//! - **业务级防抖**（反应向的 5min 冷却，架构 §3.4）在 M1-5 行为引擎，行为引擎消费
//!   本总线的原始流，两边互不影响（事件监视器要看的也是原始流）。
//! - 每条事件一条 tracing::debug（工程规范 §2 日志纪律）。

use std::sync::Mutex;
use std::time::Duration;

use tauri::Emitter;
use tauri_specta::Event as _; // EventStream::NAME

use crate::debounce::Cooling;
use crate::ipc::{EventKind, EventSource, EventStream, StdEvent};

/// 移动类事件的高频削峰冷却（传输级）
const MOVE_COOLDOWN: Duration = Duration::from_millis(120);

#[derive(Clone)]
pub struct EventBus {
    app: tauri::AppHandle,
    cooling: std::sync::Arc<Mutex<Cooling>>,
}

impl EventBus {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self {
            app,
            cooling: std::sync::Arc::new(Mutex::new(Cooling::new(MOVE_COOLDOWN))),
        }
    }

    /// 采集器投递入口。返回 false 表示被传输级削峰拦截。
    pub fn publish(&self, event: StdEvent) -> bool {
        self.tee_to_log(&event);
        if self.is_high_freq(event.kind) {
            let key = format!("{:?}:{:?}", event.source, event.kind);
            let allowed = self
                .cooling
                .lock()
                .map(|mut c| c.allow(&key))
                .unwrap_or(true);
            if !allowed {
                return false;
            }
        }
        tracing::debug!(
            source = ?event.source,
            kind = ?event.kind,
            id = %event.id,
            "event"
        );
        // 失败只记 warn 不 panic（WebView 未就绪等瞬态）
        if let Err(e) = self.app.emit(EventStream::NAME, &EventStream(event)) {
            tracing::warn!("event emit failed: {e}");
        }
        true
    }

    /// 调试落盘：`CHAOJIANDAI_EVENT_LOG=1` 启动时把每条过线事件追加到
    /// `%TEMP%\chaojiandai-events.log`（验证采集器用；正常启动零开销）。
    fn tee_to_log(&self, event: &StdEvent) {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !ENABLED.get_or_init(|| std::env::var_os("CHAOJIANDAI_EVENT_LOG").is_some()) {
            return;
        }
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::temp_dir().join("chaojiandai-events.log"))
        {
            let _ = writeln!(f, "{:?} {:?} {}", event.source, event.kind, event.payload);
        }
    }

    fn is_high_freq(&self, kind: EventKind) -> bool {
        matches!(
            kind,
            EventKind::ForegroundMoved | EventKind::SelfWindowMoved
        )
    }

    /// 采集器构造事件的便捷入口
    pub fn event(
        &self,
        source: EventSource,
        kind: EventKind,
        payload: serde_json::Value,
    ) -> StdEvent {
        StdEvent::new(source, kind, payload)
    }
}
