//! 采集器：每个事件源一个文件。`Collector` 是统一接口，main.rs 装配层启动全部。
//!
//! 分层原则（工程规范 §1/§8）：
//! - 采集器只做**系统 API → StdEvent** 的翻译，不含业务判断（业务冷却在 M1-5 行为引擎）；
//! - 高频率事件（移动类）直接投总线，传输级削峰由 [`crate::event_bus::EventBus`] 内建，
//!   采集器不各自造轮子；
//! - 每个采集器一个线程（或系统回调线程），互不阻塞。

use tauri::AppHandle;

use crate::event_bus::EventBus;

/// 当前线程 OS TID（#37 线程归因用：配合 scripts/profile-threads.ps1 的
/// CPU 热点 Tid 定位到代码位置）。仅 spawn 点打一次日志，零常驻开销。
pub(crate) fn tid() -> u32 {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    unsafe { GetCurrentThreadId() }
}

pub mod idle;
pub mod keyboard;
pub mod media;
pub mod mouse;
pub mod network;
pub mod power;
pub mod session;
pub mod win_event;

/// 事件源采集器统一接口。实现体是无状态/轻状态的 `struct`，启动即交出一个线程。
pub trait Collector: Send {
    /// 启动采集。实现内部自行 `thread::spawn`（或挂系统钩子），不得阻塞调用方。
    fn start(&self, bus: EventBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    /// 采集器名字（日志用）
    fn name(&self) -> &'static str;
}

/// 启动全部采集器（装配层唯一入口）。新采集器在这里加一行。
/// bus 必须由装配层注入并与语音/行为引擎同一实例——订阅者表随 clone 共享，
/// 各自 `EventBus::new` 会让 Rust 侧订阅者永远收不到事件（M1-5 实机踩过）。
pub fn start_all(app: AppHandle, bus: EventBus) {
    let collectors: Vec<Box<dyn Collector>> = vec![
        Box::new(win_event::WinEventCollector::new(app)),
        Box::new(idle::IdleCollector),
        Box::new(mouse::MouseCollector),
        Box::new(keyboard::KeyboardCollector),
        Box::new(power::PowerCollector),
        Box::new(network::NetworkCollector),
        Box::new(session::SessionCollector),
        Box::new(media::MediaCollector),
    ];
    for c in collectors {
        match c.start(bus.clone()) {
            Ok(()) => tracing::info!(collector = c.name(), "collector started"),
            Err(e) => tracing::error!(collector = c.name(), "collector failed to start: {e:#}"),
        }
    }
}
