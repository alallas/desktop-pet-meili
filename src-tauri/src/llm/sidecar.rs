//! llama-server sidecar 生命周期管理（M1-3.5）。
//!
//! M0 §10-7 已验证的配置原样产品化：llama-server b12283（官方 CPU 版）+
//! Qwen3-4B Q4_K_M（2.4GB），16 线程，-c 4096 -np 1。
//! 启动参数与 `scripts/llm_latency_demo.py` 保持一致——那是实测过首 token
//! 0.31–0.36s 的配置，不要随手改。
//!
//! 日志不再丢（坑 36 教训）：stdout/stderr 落 `%TEMP%\chaojiandai-llama.log`。

use std::io::Write;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// 固定回环端口（llama-server OpenAI 兼容端点）
pub const PORT: u16 = 11599;

pub struct Sidecar {
    child: Child,
}

impl Sidecar {
    /// 启动 llama-server 并等待 /health 就绪。模型加载约 5s（M0 实测）。
    /// 任何一步失败都返回 Err（调用方降级：无思考层，语音链路照常）。
    pub fn spawn() -> Result<Self, String> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let dir = root.join("../tools/llama.cpp");
        let exe = dir.join("llama-server.exe");
        let model = root.join("../models/qwen3-4b/Qwen3-4B-Q4_K_M.gguf");
        if !exe.exists() {
            return Err(format!("llama-server 不存在: {}", exe.display()));
        }
        if !model.exists() {
            return Err(format!("模型不存在: {}", model.display()));
        }

        let log_path = std::env::temp_dir().join("chaojiandai-llama.log");
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| format!("打开 llama 日志失败: {e}"))?;
        let log_err = log
            .try_clone()
            .map_err(|e| format!("克隆 llama 日志句柄失败: {e}"))?;

        tracing::info!("llm sidecar: spawning llama-server (model 2.4GB, ~5s load)");
        let child = Command::new(&exe)
            .args([
                "-m",
                model.to_str().ok_or("模型路径非 UTF-8")?,
                "--port",
                &PORT.to_string(),
                "-t",
                "16",
                "-c",
                "4096",
                "-np",
                "1",
            ])
            .current_dir(&dir) // ggml-*.dll 在同目录，必须以此为 cwd
            .stdout(log)
            .stderr(log_err)
            .spawn()
            .map_err(|e| format!("spawn llama-server 失败: {e}"))?;

        let mut sidecar = Self { child };
        sidecar.wait_ready(Duration::from_secs(120))?;
        tracing::info!("llm sidecar: ready on 127.0.0.1:{PORT}");
        Ok(sidecar)
    }

    /// 轮询 /health 直到就绪。llama-server 在模型加载完才开端口，连接拒绝是常态。
    fn wait_ready(&mut self, timeout: Duration) -> Result<(), String> {
        let t0 = Instant::now();
        loop {
            if let Ok(resp) = ureq::get(&format!("http://127.0.0.1:{PORT}/health")).call() {
                if resp.status() == 200 {
                    return Ok(());
                }
            }
            if t0.elapsed() > timeout {
                let _ = self.child.kill();
                return Err(format!(
                    "llama-server 就绪超时（{}s），日志见 %TEMP%\\chaojiandai-llama.log",
                    timeout.as_secs()
                ));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        // 子进程树整体结束：llama-server 无孙进程，kill 即可
        if let Err(e) = self.child.kill() {
            let _ = writeln!(std::io::stderr(), "llama-server kill 失败: {e}");
        }
        let _ = self.child.wait();
    }
}
