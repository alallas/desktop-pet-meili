//! 拎起 = 拖窗口（小窗主交互，M1-6 窗口管理先遣）。
//!
//! 交互语义：小窗默认穿透 ON（不挡桌面），用户**按住宠物 = 拎起拖动**。
//! 穿透窗口收不到鼠标消息——拖动借 WH_MOUSE_LL 实现：mouse.rs 的钩子回调在
//! 系统投递消息**之前**触发，左键按下落在窗内时本模块**同步摘掉 WS_EX_TRANSPARENT**
//! （不吞消息——穿透摘掉后系统对本条按下重新做命中测试会落在自家窗口，
//! 若吞掉按下则输入管线不登记按键状态，GetAsyncKeyState 永远查不到按下、
//! 拖动立即失效，2026-11-22 实测踩过）。
//! 拖动循环由本线程 GetCursorPos + SetWindowPos 跟随光标推进，松手恢复穿透。
//!
//! 分工边界：mouse.rs 只翻译事件（采集器职责），"按下落在宠物窗内怎么办"
//! 是本模块的业务判断——由 [`on_left_button_down`] 承担。
//!
//! 已知折衷（v1）：
//! - 命中矩形 = 宠物模型的绑定姿势投影 bbox（前端 500ms 重算上报，带 6px 容差），
//!   不是逐像素 alpha 检测；蒙皮动画瞬时姿态超出绑定位的误差容差内吸收；
//! - 穿透状态权威化已落地（#41①，2026-11-22）：[`THROUGH_DESIRED`] 是唯一权威
//!   （前端 set_click_through / 模拟器热键都经 [`set_through_desired`] 写入），
//!   拖拽的"摘除/恢复"只是围绕权威的瞬时物理动作，松手恢复权威值——
//!   "面板开着拖宠物 → 松手面板失效"冲突在此根治；
//! - 点击宠物（按下即松、无位移）≠ 拖动：松手时由拖线程裁决——没拖动就
//!   发 deferred 点击事件（on_pet=true），mouse.rs 对落在宠物上的左键按下
//!   是 deferred 的（不发全局点击），"拖一下误触发点击评论"在此根治。
//!   摸头反应 = map.toml anim 字段（M1-6 表现层）

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use tauri::WebviewWindow;
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, SetWindowPos, SystemParametersInfoW,
};

/// WH_MOUSE_LL 回调（mouse.rs）→ 拖线程的通知通道。容量 4：人手连续点击
/// 不可能堆 4 条；满则丢（拖线程正忙着拖，来了新按下也处理不了）。
static DOWN_TX: OnceLock<SyncSender<(i32, i32)>> = OnceLock::new();
/// 主窗 HWND 缓存（start 时写入）。WH_MOUSE_LL 回调里要用，不能带锁。
/// 存 isize：裸 HWND(*mut c_void) 不实现 Send/Sync，static 存不了。
static SELF_HWND: OnceLock<isize> = OnceLock::new();
/// 宠物命中矩形（前端上报，窗口客户区物理像素，左上角 + 宽高）。
/// 只有落在矩形内才允许"拎起"——窗口其余透明区域点击须穿透到桌面（
/// 2026-11-22 用户实机投诉：点窗口透明部分下面的字先触发拖拽）。
/// 四元组 AtomicI32 + ready 标志：钩子回调要快，禁止带锁。
static HIT_RECT: [AtomicI32; 4] = [
    AtomicI32::new(0),
    AtomicI32::new(0),
    AtomicI32::new(0),
    AtomicI32::new(0),
];
static HIT_READY: AtomicBool = AtomicBool::new(false);
/// 事件总线（松手裁决"点击 vs 拖拽"后发 deferred 点击用；start 时注入）
static BUS: OnceLock<crate::event_bus::EventBus> = OnceLock::new();
/// 穿透期望状态的**唯一权威**（#41① 穿透状态权威化，2026-11-22）：前端
/// set_click_through 与模拟器面板热键都经 [`set_through_desired`] 改写这里；
/// 拖拽的"摘除/恢复"只是围绕权威状态的瞬时物理动作——松手恢复的是
/// 权威值而非硬编码 ON（修"面板开着拖宠物 → 松手面板失效"冲突）
static THROUGH_DESIRED: AtomicBool = AtomicBool::new(true);

/// SWP_* 常量（winuser.h 稳定值；同 win_event.rs 的本地定义约定，避免 import 漂移）
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOZORDER: u32 = 0x0004;
const SWP_NOACTIVATE: u32 = 0x0010;
/// WS_EX_NOACTIVATE（winuser.h 稳定值）：点击窗口不激活、不偷前台焦点
const WS_EX_NOACTIVATE: isize = 0x0800_0000;
/// SPI_GETWORKAREA（winuser.h 稳定值）：主显示器工作区（任务栏避让）
const SPI_GETWORKAREA: u32 = 48;

/// 启动拖线程。win 与事件总线由装配层（main.rs setup）注入
/// （总线用于松手裁决"点击 vs 拖拽"后发 deferred 点击事件）。
pub fn start(win: &WebviewWindow, bus: crate::event_bus::EventBus) {
    let hwnd = win.hwnd().map(|h| h.0 as isize).unwrap_or(0);
    if hwnd == 0 {
        tracing::error!("window_drag: main hwnd unavailable, drag disabled");
        return;
    }
    let _ = SELF_HWND.set(hwnd);
    let _ = BUS.set(bus);
    // 点击不偷焦点：拖拽按下会落到自家 WebView（穿透摘除后），没这个位会
    // 把窗口激活到前台、打断用户正在用的应用。与 M0 set_ignore_cursor_events
    // 互不冲突——穿透位由 on_left_button_down/drag_once 成对管理
    unsafe {
        let hwnd = hwnd as HWND;
        let ex = windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, -20);
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
            hwnd,
            -20,
            ex | WS_EX_NOACTIVATE,
        );
    }
    let (tx, rx) = sync_channel::<(i32, i32)>(4);
    let _ = DOWN_TX.set(tx);
    thread::spawn(move || {
        tracing::info!(tid = crate::event_collector::tid(), "window drag thread up");
        for (x, y) in rx {
            drag_once(hwnd as HWND, x, y);
        }
    });
}

/// WH_MOUSE_LL 左键按下回调入口：光标在宠物窗内 → 摘掉 WS_EX_TRANSPARENT + 通知
/// 拖线程。不吞消息——穿透在钩子回调里同步摘除后，系统对这条按下重新做命中
/// 测试会落在本窗口（后续 move/up/按键状态全部走标准管线，拖线程靠
/// GetAsyncKeyState 跟手）；若吞掉按下，输入管线不登记按键状态，拖动立即失效
/// （2026-11-22 实测踩过）。按下落到自家 WebView 无副作用（无点击处理器，
/// WS_EX_NOACTIVATE 保证不偷焦点）。快速路径 ~µs，符合钩子回调轻量要求。
pub fn on_left_button_down(x: i32, y: i32) {
    let Some(&hwnd_isize) = SELF_HWND.get() else {
        return;
    };
    let hwnd = hwnd_isize as HWND;
    if !hit_test(hwnd, x, y) {
        return; // 点在宠物外的透明区：不摘穿透，点击正常落到下层桌面窗口
    }
    unsafe {
        let style = windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, -20);
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(hwnd, -20, style & !0x20);
    }
    if let Some(tx) = DOWN_TX.get() {
        let _ = tx.try_send((x, y)); // 满则丢：拖线程正忙，无法并发拖第二轮
    }
}

/// 设置穿透期望状态（穿透权威唯一入口，ipc set_click_through 与模拟器面板热键
/// 共用）：写入权威位并立即应用到窗口。应用走 Get/SetWindowLongPtrW 而非
/// tauri 的 set_ignore_cursor_events——两者同改 WS_EX_TRANSPARENT 位，
/// 但本模块拖线程也直接操作该位，经同一对 API 读写避免状态互相覆盖
pub fn set_through_desired(through: bool) {
    THROUGH_DESIRED.store(through, Ordering::SeqCst);
    let Some(&hwnd_isize) = SELF_HWND.get() else {
        return; // 装配期（start 之前）调用：权威位已写，start 后自然生效
    };
    unsafe {
        let hwnd = hwnd_isize as HWND;
        let style = windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, -20);
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
            hwnd,
            -20,
            if through { style | 0x20 } else { style & !0x20 },
        );
    }
}

/// 前端上报命中矩形（ipc 入口）。x/y/w/h 为 CSS 逻辑像素、相对窗口客户区左上角；
/// 这里乘 scale factor 转物理像素存静态原子量。前端每 500ms 重算宠物投影 bbox、
/// 变化才上报（见 main.ts reportHitRect）。
pub fn set_hit_rect(scale: f64, x: f64, y: f64, w: f64, h: f64) {
    let s = scale.max(1.0);
    HIT_RECT[0].store((x * s).round() as i32, Ordering::Relaxed);
    HIT_RECT[1].store((y * s).round() as i32, Ordering::Relaxed);
    HIT_RECT[2].store((w * s).round() as i32, Ordering::Relaxed);
    HIT_RECT[3].store((h * s).round() as i32, Ordering::Relaxed);
    HIT_READY.store(true, Ordering::Relaxed);
}

/// 光标是否落在宠物命中矩形内（屏幕物理像素）。mouse.rs 发布点击事件时
/// 给 payload 标 on_pet 用；未就绪（前端还没报 bbox）时恒 false。
pub fn is_on_pet(x: i32, y: i32) -> bool {
    let Some(&hwnd_isize) = SELF_HWND.get() else {
        return false;
    };
    hit_test(hwnd_isize as HWND, x, y)
}

/// 点 (x,y)（屏幕物理像素）是否落在宠物命中矩形内。
/// 客户区原点用 ClientToScreen 现算——比缓存窗口偏移稳（框架内边随
/// shadow/decorations 配置会变，2026-11-22 shadow:false 后内边为 0）。
fn hit_test(hwnd: HWND, x: i32, y: i32) -> bool {
    if !HIT_READY.load(Ordering::Relaxed) {
        return false; // 前端还没报：宁可暂不可拖，也不整窗拦截
    }
    unsafe {
        let mut origin = POINT { x: 0, y: 0 };
        if ClientToScreen(hwnd, &mut origin) == 0 {
            return false;
        }
        let hx = HIT_RECT[0].load(Ordering::Relaxed);
        let hy = HIT_RECT[1].load(Ordering::Relaxed);
        let hw = HIT_RECT[2].load(Ordering::Relaxed);
        let hh = HIT_RECT[3].load(Ordering::Relaxed);
        if hw <= 0 || hh <= 0 {
            return false;
        }
        // 矩形是客户区内相对坐标，origin 即客户区左上角（物理像素）
        x >= origin.x + hx && x < origin.x + hx + hw && y >= origin.y + hy && y < origin.y + hy + hh
    }
}

/// 一轮拖动：从按下点开始跟随光标，直到左键松开。
fn drag_once(hwnd: HWND, x: i32, y: i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        if windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect) == 0 {
            return;
        }
    }
    let grab_dx = x - rect.left;
    let grab_dy = y - rect.top;
    let work = work_area();
    let start = Instant::now();
    let mut moved = false;
    let mut last_pos = (rect.left, rect.top);

    // 跟手节拍 16ms（~60Hz，与 M0 窗口移动实测的帧预算一致）；松手检测用
    // GetAsyncKeyState 而非等 WM_LBUTTONUP——拖动中窗口不一定收得到消息
    loop {
        thread::sleep(Duration::from_millis(16));
        unsafe {
            if GetAsyncKeyState(0x01) as u32 & 0x8000 == 0 {
                break; // VK_LBUTTON 松开
            }
            let mut pt = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut pt) == 0 {
                break;
            }
            // 防抖：按下后 250ms 内位移 < 6px 视为"没打算拖"，窗口不动
            let dx = pt.x - x;
            let dy = pt.y - y;
            if !moved && dx * dx + dy * dy < 36 && start.elapsed() < Duration::from_millis(250) {
                continue;
            }
            let mut new_left = pt.x - grab_dx;
            let mut new_top = pt.y - grab_dy;
            let win_w = rect.right - rect.left;
            let win_h = rect.bottom - rect.top;
            clamp_to_work_area(&mut new_left, &mut new_top, win_w, win_h, &work);
            if (new_left, new_top) == last_pos {
                continue;
            }
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                new_left,
                new_top,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            last_pos = (new_left, new_top);
            moved = true;
        }
    }

    // 恢复穿透：恢复的是权威期望状态（THROUGH_DESIRED）而非硬编码 ON——
    // "模拟器面板开着（权威=OFF）时拖宠物"松手后面板才不会失效（#41①）。
    // 移动会触发自窗口 LOCATIONCHANGE → 现有 SelfWindowMoved 事件链路，
    // 位置同步不用本模块操心
    let through = THROUGH_DESIRED.load(Ordering::SeqCst);
    unsafe {
        let style = windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, -20);
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
            hwnd,
            -20,
            if through { style | 0x20 } else { style & !0x20 },
        );
    }
    tracing::info!(moved, through, "pet drag ended");

    // 松手裁决：没拖动 = 点宠物（摸头）。按下时mouse.rs 对"落在宠物上的
    // 左键"是 deferred 的（不发全局点击），由这里补发——拖到一半松手
    // （moved=true）不发，§41"拖一下触发评论"噪音在此根治
    if !moved {
        if let Some(bus) = BUS.get() {
            bus.publish(bus.event(
                crate::ipc::EventSource::Mouse,
                crate::ipc::EventKind::MouseClicked,
                serde_json::json!({ "button": "left", "x": x, "y": y, "on_pet": true }),
            ));
        }
    }
}

/// 便捷入口（main.rs 初始定位用）：窗口所在主显示器工作区；取不到返回 None，
/// 调用方退回全屏尺寸（不避让任务栏，比不定位强）。
pub fn work_area_for(win: &WebviewWindow) -> Option<RECT> {
    win.hwnd().ok()?;
    let r = work_area();
    if r.right <= r.left || r.bottom <= r.top {
        None
    } else {
        Some(r)
    }
}

/// 主显示器工作区（任务栏避让）。坐标与 GetWindowRect（物理像素）同空间，
/// 可直接比较/夹取。单显示器假设与 main.rs 初始定位一致，
/// 多显示器随 M1-6 窗口管理细化。
fn work_area() -> RECT {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        // 不用 SystemParametersInfoForDpi：它对 SPI_GETWORKAREA 实测返回失败
        // （ok=0，2026-11-22 用 ctypes 对照验证）。本进程是 per-monitor DPI 感知
        // （GetWindowRect 实测物理像素），SystemParametersInfoW 对感知进程直接
        // 返回主显示器工作区物理坐标，与 GetWindowRect 同空间，可直接比较
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut rect as *mut RECT as *mut _, 0);
    }
    rect
}

/// 窗口位置夹取进工作区（拖出不脱屏、右下不扎进任务栏）
fn clamp_to_work_area(left: &mut i32, top: &mut i32, win_w: i32, win_h: i32, work: &RECT) {
    if *left < work.left {
        *left = work.left;
    }
    if *top < work.top {
        *top = work.top;
    }
    if *left + win_w > work.right {
        *left = work.right - win_w;
    }
    if *top + win_h > work.bottom {
        *top = work.bottom - win_h;
    }
}
