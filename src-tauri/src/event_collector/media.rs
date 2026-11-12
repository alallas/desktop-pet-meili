//! SMTC 音乐采集器：WinRT GlobalSystemMediaTransportControlsSession（架构 §3.1 事件驱动）。
//!
//! - "你在听/看什么"双通道之一（另一条是窗口标题）——M0-4 已验证 SMTC 能拿到 Chrome 等
//!   浏览器视频标签页的媒体会话，基础版"看视频给评论"不依赖浏览器扩展；
//! - 事件：当前会话切换 / 播放状态变化 / 曲目信息变化，三者都重新读取并发布；
//! - 缩略图（专辑封面）留给 M1-5 感知管道，这里只要元数据（隐私三档里的 Minimal 线）。

use std::sync::{Mutex, OnceLock};

use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};

use super::Collector;
use crate::event_bus::EventBus;
use crate::ipc::{EventKind, EventSource};

/// 极简 block_on：WinRT 异步操作的同步等待（windows-future 0.3 起移除了阻塞式 get()，
/// 只留 std feature 的 IntoFuture；采集器线程非 UI 线程，park/unpark 阻塞正合适）
fn block_on<F: std::future::IntoFuture>(fut: F) -> F::Output {
    use std::future::Future;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    struct ThreadWaker(std::thread::Thread);
    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker: Waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = Box::pin(fut.into_future());
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park(),
        }
    }
}

static BUS: OnceLock<EventBus> = OnceLock::new();
/// 当前媒体会话（WinRT 对象要求保持在存活，事件注册才不会失效）
static SESSION: Mutex<Option<GlobalSystemMediaTransportControlsSession>> = Mutex::new(None);

pub struct MediaCollector;

impl Collector for MediaCollector {
    fn start(&self, bus: EventBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = BUS.get_or_init(|| bus);
        std::thread::spawn(move || {
            if let Err(e) = run() {
                // 无 SMTC 服务的精简系统会走到这；记 error 退出（不影响其他采集器）
                tracing::error!("SMTC collector exited: {e}");
            }
        });
        Ok(())
    }

    fn name(&self) -> &'static str {
        "media"
    }
}

fn run() -> windows::core::Result<()> {
    // 双层 Result：RequestAsync 返回 Result<IAsyncOperation>，IntoFuture 的 Output 还是 Result<T>
    let manager = block_on(GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?)?;
    // 当前会话切换（开了新播放器 / 关了播放器）
    manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
        let manager =
            GlobalSystemMediaTransportControlsSessionManager::RequestAsync().and_then(block_on);
        if let Ok(manager) = manager {
            set_session(manager.GetCurrentSession().ok());
        }
        Ok(())
    }))?;
    // 基线 + 常驻（事件回调在 WinRT 线程池，本线程只负责保活）
    set_session(manager.GetCurrentSession().ok());
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

/// 换会话：更新静态引用、挂事件、报基线
fn set_session(session: Option<GlobalSystemMediaTransportControlsSession>) {
    if let Some(s) = &session {
        let info_session = s.clone();
        let _ = s.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
            publish(&info_session);
            Ok(())
        }));
        let prop_session = s.clone();
        let _ = s.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
            publish(&prop_session);
            Ok(())
        }));
    }
    if let Ok(mut guard) = SESSION.lock() {
        *guard = session;
    }
    if let Ok(guard) = SESSION.lock() {
        if let Some(s) = guard.clone() {
            publish(&s);
        } else {
            // 无会话也报一次（"音乐停了"本身是信息，且证明采集器活着）
            if let Some(bus) = BUS.get() {
                bus.publish(bus.event(
                    EventSource::Media,
                    EventKind::MediaPlaybackChanged,
                    serde_json::json!({ "app": "", "title": "", "artist": "", "state": "none" }),
                ));
            }
        }
    }
}

/// 读当前播放状态 + 曲目信息，发事件
fn publish(session: &GlobalSystemMediaTransportControlsSession) {
    let Some(bus) = BUS.get() else { return };
    let state = session
        .GetPlaybackInfo()
        .map(|info| {
            info.PlaybackStatus()
                .unwrap_or(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Stopped)
        })
        .unwrap_or(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Stopped);
    let state = match state {
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing => "playing",
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused => "paused",
        _ => "stopped",
    };
    // 曲目信息异步获取；个别会话会失败（无媒体属性），降级为空串
    let (title, artist) = session
        .TryGetMediaPropertiesAsync()
        .and_then(block_on)
        .map(|p| {
            (
                p.Title().unwrap_or_default().to_string(),
                p.Artist().unwrap_or_default().to_string(),
            )
        })
        .unwrap_or_default();
    let app = session
        .SourceAppUserModelId()
        .map(|id| id.to_string())
        .unwrap_or_default();
    bus.publish(bus.event(
        EventSource::Media,
        EventKind::MediaPlaybackChanged,
        serde_json::json!({ "app": app, "title": title, "artist": artist, "state": state }),
    ));
}
