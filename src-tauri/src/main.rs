#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! 装配层 only（工程规范 §1）：plugin/路由注册与启动顺序，不放逻辑。

mod cloud;
mod db;
mod debounce;
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
        ])
        .events(tauri_specta::collect_events![ipc::Booted, ipc::EventStream])
        .typ::<ipc::StdEvent>()
        .typ::<ipc::EventSource>()
        .typ::<ipc::EventKind>()
        .typ::<ipc::PrivacyTier>()
}

fn main() {
    let builder = specta_builder();
    export_typescript_bindings(&builder);
    // 返回的闭包自持 Arc（'static），builder 本体随后 move 进 setup 挂事件
    let handler = builder.invoke_handler();

    tauri::Builder::default()
        .setup(move |app| {
            let win = app.get_webview_window("main").unwrap();
            // 窗口铺满主显示器（避免写死分辨率，兼容不同屏幕/DPI）
            if let Ok(Some(monitor)) = app.primary_monitor() {
                let size = monitor.size();
                let pos = monitor.position();
                let _ = win.set_size(tauri::Size::Physical(*size));
                let _ = win.set_position(tauri::Position::Physical(*pos));
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
            // 其余采集器（鼠标/SMTC/电源/网络/锁屏）后续按提交增量接入 start_all。
            event_collector::start_all(app.handle().clone());
            // M1-1：事件通道样例——启动完成事件（ping-pong 的另一半在前端 listen）。
            // 延迟 1s 发射：setup 阶段 WebView 尚未加载，立即发会被错过（事件不排队）。
            builder.mount_events(app);
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let _ = ipc::Booted(ipc::StdEvent::new(
                    ipc::EventSource::System,
                    ipc::EventKind::Booted,
                    serde_json::json!({ "version": env!("CARGO_PKG_VERSION") }),
                ))
                .emit(&app_handle);
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
