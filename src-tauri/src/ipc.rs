//! IPC 协议（v1，2026-11-11 人审定案）。
//!
//! - 全部命令经 specta 标注，构建时导出 `src/ipc/types.ts`（CI 校验新鲜度，工程规范 §3）。
//! - Rust→WebView 事件骨架：`StdEvent { id, ts, source, kind, payload, privacy_tier }`；
//!   `source`/`kind` 是封闭枚举，M1-2 采集器落地时逐源扩枚举（加变体必须全消费方编译通过）。
//! - 协议改动必须人审（工程规范 §5.4）。

use serde::{Deserialize, Serialize};
use specta::Type;

// ---------- 事件骨架（协议核心，勿随意改） ----------

/// 隐私三档（产品 §6，档位语义见 settings）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
pub enum PrivacyTier {
    Minimal,
    Balanced,
    Companion,
}

/// 事件来源（封闭集；每个采集器一个变体，新增须同步映射表与文档）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
pub enum EventSource {
    System,
    ForegroundWindow, // 前台窗口变化（SetWinEventHook EVENT_SYSTEM_FOREGROUND）
    WindowTrack,      // 窗口位置跟踪（同钩子的 LOCATIONCHANGE）
    Idle,             // 空闲时长（GetLastInputInfo ≤1Hz 轮询）
    Mouse,            // 全局鼠标按键（WH_MOUSE_LL 低级钩子）
    Power,            // 电源状态（GetSystemPowerStatus 1Hz 轮询）
    Network,          // 网络连通性（NotifyAddrChange 事件 + 网关探测）
    Session,          // 会话状态（WTS 锁屏/解锁事件）
    Media,            // SMTC 媒体会话（WinRT，"你在听/看什么"双通道之一）
    Voice,            // 语音管线（本地 KWS/VAD 处理，唤醒事件本体不外流）
    Test,
}

/// 事件类型（封闭集；随采集器扩展）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub enum EventKind {
    /// 应用启动完成（M1-1 ping-pong 样例用的载具）
    Booted,
    Ping,
    /// 前台应用切换。payload: {app, title}
    ForegroundSwitched,
    /// 前台窗口被移动/改变大小。payload: {app, rect}
    ForegroundMoved,
    /// 自窗口被移动（外部拖动或别的程序挪了我们）。payload: {x, y}
    SelfWindowMoved,
    /// 空闲状态翻转。payload: {idle_seconds, is_idle}
    IdleStateChanged,
    /// 全局鼠标按键按下（左/右/中）。payload: {button, x, y}
    MouseClicked,
    /// 电源状态变化（插电/用电池/充电翻转/电量档位跳变）。payload: {ac_online, charging, battery_percent}
    PowerStateChanged,
    /// 网络连通性变化（有无可上网网关的网卡）。payload: {connected}
    NetworkChanged,
    /// 锁屏/解锁。payload: {locked}
    LockStateChanged,
    /// 媒体播放变化（SMTC：状态/曲目/歌手）。payload: {app, title, artist, state}
    MediaPlaybackChanged,
    /// 唤醒词命中（本地 KWS，音频不出本机）。payload: {keyword}
    WakeWord,
    /// 一句指令语音结束（本地 VAD 断句；音频本体留 Rust 侧给 ASR 消费）。
    /// payload: {duration_ms}
    UtteranceEnded,
    /// 本地 ASR（SenseVoice）转写完成。payload: {text, duration_ms}。
    /// 文本是语音内容本体：隐私档 Balanced
    TranscriptReady,
}

/// 标准化事件：Rust→WebView 事件总线的唯一格式
#[derive(Debug, Clone, Serialize, Type)]
pub struct StdEvent {
    pub id: String,
    /// 秒级 Unix epoch
    pub ts: i64,
    pub source: EventSource,
    pub kind: EventKind,
    pub payload: serde_json::Value,
    pub privacy_tier: PrivacyTier,
}

impl StdEvent {
    pub fn new(source: EventSource, kind: EventKind, payload: serde_json::Value) -> Self {
        Self {
            id: uuid(),
            ts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or_default(),
            source,
            kind,
            payload,
            privacy_tier: PrivacyTier::Balanced,
        }
    }
}

/// 启动完成事件（Rust→WebView，展示事件通道）
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
pub struct Booted(pub StdEvent);

/// 事件总线对外流（Rust→WebView，事件监视器调试页订阅）
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
pub struct EventStream(pub StdEvent);

fn uuid() -> String {
    // M1-1：无 uuid 依赖前的临时 id（进程内唯一即可）；repo.rs 落地时换正式实现
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!(
        "evt-{}-{:016x}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

// ---------- 命令（WebView → Rust） ----------

/// ping-pong 样例：IPC 两端联通性验证（M1-1 交付项）
#[tauri::command]
#[specta::specta]
pub fn ping(msg: String) -> String {
    format!("pong: {msg}")
}

/// 点击穿透开关（M0 验证命令，产品化后由窗口管理器接管）
#[tauri::command]
#[specta::specta]
pub fn set_click_through(through: bool, window: tauri::Window) {
    let _ = window.set_ignore_cursor_events(through);
}

/// 点击穿透下页面收不到鼠标事件，由 Rust 侧查询全局光标位置（M0-2）
#[tauri::command]
#[specta::specta]
pub fn get_cursor_position(app: tauri::AppHandle) -> Option<(f64, f64)> {
    app.cursor_position().ok().map(|p| (p.x, p.y))
}

/// 临时调试：前端状态写文件（截图不可靠时排查用，日志落 `%TEMP%\chaojiandai-debug.log`）
#[tauri::command]
#[specta::specta]
pub fn debug_log(msg: String) {
    use std::io::Write;
    let path = std::env::temp_dir().join("chaojiandai-debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{}", msg);
    }
}
