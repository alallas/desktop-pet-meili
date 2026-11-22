#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! 装配层 only（工程规范 §1）：plugin/路由注册与启动顺序，不放逻辑。

mod behavior;
mod cloud;
mod db;
mod debounce;
mod dev_hotkey;
mod event_bus;
mod event_collector;
mod hooks;
mod ipc;
mod llm;
mod perception;
mod settings;
mod voice;

use tauri::Manager;
use tauri_specta::Event as _; // Booted::emit trait 方法

/// specta 构建器：命令/事件/类型的唯一集散地（导出与运行时共用，保证一致）
fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::new()
        .error_handling(tauri_specta::ErrorHandlingMode::Result)
        // Unix 秒级时间戳走 f64 无损（21 亿年内安全），允许 i64 → number
        .dangerously_cast_bigints_to_number()
        .commands(tauri_specta::collect_commands![
            ipc::ping,
            ipc::set_click_through,
            ipc::get_cursor_position,
            ipc::debug_log,
            ipc::debug_emit_event,
            ipc::debug_reset_behavior,
        ])
        .events(tauri_specta::collect_events![
            ipc::Booted,
            ipc::EventStream,
            ipc::SimPanel
        ])
        .typ::<ipc::StdEvent>()
        .typ::<ipc::EventSource>()
        .typ::<ipc::EventKind>()
        .typ::<ipc::PrivacyTier>()
}

fn main() {
    // tracing 落盘：桌面 exe 无控制台，不初始化 collector/语音引擎的日志全丢。
    // 每次启动追加 %TEMP%\chaojiandai-rust.log（与前端状态日志分文件，职责不同）。
    tracing_subscriber::fmt()
        .with_writer(|| -> std::fs::File {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(std::env::temp_dir().join("chaojiandai-rust.log"))
                .expect("open rust log")
        })
        .init();
    // panic 落盘：GUI 程序（windows_subsystem）无控制台，默认 hook 的 panic 信息直接丢。
    // 工作线程（语音引擎/采集器）panic = 功能静默消失，必须留下遗言。
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = format!("thread panic: {info}");
        tracing::error!("{msg}");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::temp_dir().join("chaojiandai-rust.log"))
        {
            use std::io::Write;
            let _ = writeln!(f, "{msg}");
        }
        default_hook(info);
    }));
    let builder = specta_builder();
    export_typescript_bindings(&builder);
    // 返回的闭包自持 Arc（'static），builder 本体随后 move 进 setup 挂事件
    let handler = builder.invoke_handler();

    tauri::Builder::default()
        .setup(move |app| {
            let win = app.get_webview_window("main").unwrap();
            // 小窗形态（§3.5 M1-1 定案）：主窗 = 300×400 透明小窗（tauri.conf），
            // 不再铺满屏幕。初始定位主显示器右下角（不挡工作区）；
            // 任务栏避让/多显示器/拖拽跟随随 M1-6 窗口管理细化
            if let Ok(Some(monitor)) = app.primary_monitor() {
                if let Ok(win_size) = win.outer_size() {
                    let margin = (24.0 * monitor.scale_factor()) as i32;
                    let x = monitor.position().x + monitor.size().width as i32
                        - win_size.width as i32
                        - margin;
                    let y = monitor.position().y + monitor.size().height as i32
                        - win_size.height as i32
                        - margin;
                    let _ = win.set_position(tauri::Position::Physical(
                        tauri::PhysicalPosition::new(x, y),
                    ));
                }
            }
            // M0 验证：默认点击穿透，悬浮层不阻塞桌面操作
            let _ = win.set_ignore_cursor_events(true);
            // M1-1：存档库（八张表，架构 §3.8；目录为 %APPDATA%/<identifier>/saves/）
            let save_dir = app.path().app_data_dir()?.join("saves");
            std::fs::create_dir_all(&save_dir).expect("create save dir");
            let conn = db::open(&save_dir.join("save.db")).expect("open save db");
            // rusqlite::Connection 是 Send 但非 Sync，Mutex 包装后共享（IPC 多线程访问的正确姿势）
            app.manage(std::sync::Mutex::new(conn));
            // M1-2：系统层事件采集（前台窗口钩子 + 空闲轮询），统一投事件总线；
            // M1-5 修正：总线单实例注入（与语音/行为引擎同一份），事件总线注释见 event_bus.rs
            let bus = crate::event_bus::EventBus::new(app.handle().clone());
            event_collector::start_all(app.handle().clone(), bus.clone());
            // 总线与行为引擎控制通道挂 State：场景模拟器调试命令（debug_emit_event /
            // debug_reset_behavior）经 tauri::State 取用
            app.manage(bus.clone());
            let (control_tx, control_rx) = std::sync::mpsc::sync_channel::<behavior::Control>(8);
            app.manage(control_tx.clone());
            // M1-3.2 + M1-5：语音引擎（常驻线程）+ 行为引擎（事件 → 四道闸 → 台词）。
            // 台词通道：行为引擎产、语音引擎 IDLE 态消费（满员即丢，闸已限频）
            let (comment_tx, comment_rx) = std::sync::mpsc::sync_channel::<String>(8);
            // #35：行为评论模板台词预热——台词全部来自 map.toml 有限变体
            let prewarm_lines: Vec<String> = behavior::map::BehaviorMap::load("meili")
                .entries
                .iter()
                .flat_map(|(_, e)| e.say.iter().cloned())
                .filter(|s| !s.contains("{{")) // 槽位台词真实出现时才合成（槽位未填会合成出字面 {{app}}）
                .collect();
            voice::engine::start(bus.clone(), comment_rx, control_tx.clone(), prewarm_lines);
            behavior::start(bus.clone(), control_rx, comment_tx);
            // M1-5 场景模拟器：全局热键 Ctrl+Alt+N（主窗口不聚焦，热键只能 Rust 侧注册）
            dev_hotkey::start(app.handle().clone());
            // M1-1：事件通道样例——启动完成事件（ping-pong 的另一半在前端 listen）。
            // 延迟 1s 发射：setup 阶段 WebView 尚未加载，立即发会被错过（事件不排队）。
            builder.mount_events(app);
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let event = ipc::StdEvent::new(
                    ipc::EventSource::System,
                    ipc::EventKind::Booted,
                    serde_json::json!({ "version": env!("CARGO_PKG_VERSION") }),
                );
                // M1-1 前端契约保留（Booted 事件）；同时投总线——Rust 侧订阅者
                // （行为引擎）只认 publish 的扇出，直接 emit 它们永远收不到
                let _ = ipc::Booted(event.clone()).emit(&app_handle);
                bus.publish(event);
            });
            Ok(())
        })
        .invoke_handler(handler)
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 导出 TS 绑定到 `src/ipc/types.ts`（不手改；CI 用 tests::bindings_fresh 校验新鲜度）
fn export_typescript_bindings(builder: &tauri_specta::Builder<tauri::Wry>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/ipc/types.ts");
    builder
        .export(specta_typescript::Typescript::default(), &path)
        .expect("export typescript bindings");
}

#[cfg(test)]
mod tests {
    /// IPC 生成物新鲜度校验（工程规范 §6）：Rust 侧改了协议忘重新生成时，这里红。
    #[test]
    fn bindings_fresh() {
        let tmp = std::env::temp_dir().join("chaojiandai-types-test.ts");
        super::specta_builder()
            .export(specta_typescript::Typescript::default(), &tmp)
            .expect("export to temp");
        let fresh = std::fs::read_to_string(&tmp).expect("read fresh bindings");
        let checked_in_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/ipc/types.ts");
        let Ok(checked_in) = std::fs::read_to_string(&checked_in_path) else {
            // 首次引导：文件尚不存在，直接生成入库（此后本测试只剩比对职责）
            std::fs::copy(&tmp, &checked_in_path).expect("bootstrap types.ts");
            std::fs::remove_file(&tmp).ok();
            return;
        };
        std::fs::remove_file(&tmp).ok();
        assert_eq!(
            fresh.trim_end(),
            checked_in.trim_end(),
            "src/ipc/types.ts 与 Rust 协议不一致：改了 ipc.rs 后重新 cargo test 生成并提交"
        );
    }
}
