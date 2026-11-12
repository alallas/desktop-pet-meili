//! 采集器：每个事件源一个文件。`Collector` 是统一接口，main.rs 装配层启动全部。
//!
//! 分层原则（工程规范 §1/§8）：
//! - 采集器只做**系统 API → StdEvent** 的翻译，不含业务判断（业务冷却在 M1-5 行为引擎）；
//! - 高频率事件（移动类）直接投总线，传输级削峰由 [`crate::event_bus::EventBus`] 内建，
//!   采集器不各自造轮子；
//! - 每个采集器一个线程（或系统回调线程），互不阻塞。

use tauri::AppHandle;

use crate::event_bus::EventBus;

pub mod idle;
pub mod win_event;

/// 事件源采集器统一接口。实现体是无状态/轻状态的 `struct`，启动即交出一个线程。
pub trait Collector: Send {
    /// 启动采集。实现内部自行 `thread::spawn`（或挂系统钩子），不得阻塞调用方。
    fn start(&self, bus: EventBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    /// 采集器名字（日志用）
    fn name(&self) -> &'static str;
}

/// 启动全部采集器（装配层唯一入口）。新采集器在这里加一行。
pub fn start_all(app: AppHandle) {
    let bus = EventBus::new(app.clone());
    let collectors: Vec<Box<dyn Collector>> = vec![
        Box::new(win_event::WinEventCollector::new(app)),
        Box::new(idle::IdleCollector),
    ];
    for c in collectors {
        match c.start(bus.clone()) {
            Ok(()) => tracing::info!(collector = c.name(), "collector started"),
            Err(e) => tracing::error!(collector = c.name(), "collector failed to start: {e:#}"),
        }
    }
}
