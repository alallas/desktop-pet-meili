//! TTS sidecar（M1-3.5b）：Python Kokoro 常驻合成服务的 Rust 侧封装。
//!
//! 线程模型与 llm::sidecar 一致：进程常驻、stdin 喂文本、stdout 回收 wav 路径。
//! Kokoro 模型加载 ~2s（坑 16 懒加载补丁），逐句 spawn 不可接受，故常驻。
//! 合成本身阻塞 ~1–3s（M0 实测 RTF 0.3），由引擎线程调用（与 ASR/LLM 阻塞同级，
//! 期间唤醒容忍漏听，采集通道有 2s 缓冲，见 engine.rs 注释）。
//!
//! stdout 读取：std 管道读无超时原语，故 spawn 时挂一个常驻读线程把响应行进
//! mpsc 通道，synth() 用 recv_timeout 拿到真超时——sidecar 卡死不会拖死引擎线程。
//!
//! 协议（行式）：入 = 单行文本；出 = `OK\t<wav 路径>\t<样本数>` 或 `ERR\t<消息>`。
//! parse_response 是纯函数，独立单测。

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

/// READY 等待上限：冷机加载（torch import + Kokoro + 预热）实测逼近 120s，放宽到 180s
const READY_TIMEOUT: Duration = Duration::from_secs(180);
/// 单句合成上限：M0 实测 RTF ~0.3，60 字 ≈ 15s 音频 ≈ 5s 合成，12 倍余量
const SYNTH_TIMEOUT: Duration = Duration::from_secs(60);

pub struct TtsSidecar {
    child: Child,
    stdin: ChildStdin,
    /// 读线程转发 sidecar stdout 响应行（就绪后首行必为 READY）
    rx: Receiver<std::io::Result<String>>,
}

impl TtsSidecar {
    /// 启动 Python sidecar 并等待 READY。失败返回 Err（调用方降级：无声音）。
    pub fn spawn() -> Result<Self, String> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let script = root.join("../scripts/tts_sidecar.py");
        if !script.exists() {
            return Err(format!("tts_sidecar.py 不存在: {}", script.display()));
        }
        let mut child = Command::new(find_python()?)
            .arg(&script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // stderr 继承（开发期看报错；落盘策略与坑 36 后的 rust 日志对齐，产品化再改）
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("spawn tts sidecar 失败: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("取 sidecar stdin 失败")?;
        let stdout = child.stdout.take().ok_or("取 sidecar stdout 失败")?;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break, // EOF：sidecar 退出
                    Ok(_) => {
                        if tx.send(Ok(line)).is_err() {
                            break; // 接收方已销毁
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
        });

        // 首行必须是 READY（模型加载 + 预热完成）；之前的依赖告警行跳过
        let t0 = std::time::Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(Ok(line)) => {
                    if line.trim() == "READY" {
                        break;
                    }
                }
                Ok(Err(e)) => {
                    let _ = child.kill();
                    return Err(format!("读 tts sidecar 失败: {e}"));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if t0.elapsed() > READY_TIMEOUT {
                        let _ = child.kill();
                        return Err("tts sidecar READY 超时（120s）".into());
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = child.kill();
                    return Err("tts sidecar 启动即退出（stderr 见控制台）".into());
                }
            }
        }
        tracing::info!("tts sidecar: ready");
        let _ = &mut stdin;
        Ok(Self { child, stdin, rx })
    }

    /// 合成一句：返回 wav 路径（24kHz 16-bit PCM 单声道）。失败 Err。
    pub fn synth(&mut self, text: &str) -> Result<PathBuf, String> {
        let clean: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        if clean.trim().is_empty() {
            return Err("合成文本为空".into());
        }
        self.stdin
            .write_all(clean.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("写 sidecar stdin 失败: {e}"))?;

        match self.rx.recv_timeout(SYNTH_TIMEOUT) {
            Ok(Ok(line)) => match parse_response(line.trim()) {
                Some(Ok(path)) => Ok(path),
                Some(Err(msg)) => Err(format!("sidecar 合成失败: {msg}")),
                None => Err(format!("sidecar 响应无法解析: {line:?}")),
            },
            Ok(Err(e)) => Err(format!("读 sidecar 响应失败: {e}")),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                Err(format!("sidecar 合成超时（{}s）", SYNTH_TIMEOUT.as_secs()))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err("tts sidecar 已退出".into())
            }
        }
    }
}

impl Drop for TtsSidecar {
    fn drop(&mut self) {
        if let Err(e) = self.child.kill() {
            let _ = writeln!(std::io::stderr(), "tts sidecar kill 失败: {e}");
        }
        let _ = self.child.wait();
    }
}

/// 探测 Python 解释器：env 覆盖（测试/打包注入）> 托管 venv（本机开发环境）> PATH。
/// 打包期固化 venv 是待办 #14 同类，届时 find_python 整体换实现。
fn find_python() -> Result<String, String> {
    if let Ok(p) = std::env::var("CHAOJIANDAI_PYTHON") {
        if std::path::Path::new(&p).exists() {
            return Ok(p);
        }
    }
    let managed = r"D:\KimiData\daimon-share\daimon\runtime\python\.venv\Scripts\python.exe";
    if std::path::Path::new(managed).exists() {
        return Ok(managed.into());
    }
    Ok("python".into())
}

/// 解析 sidecar 响应行：`OK<TAB>路径[\t样本数]` / `ERR<TAB>消息`。纯函数独立单测。
fn parse_response(line: &str) -> Option<Result<PathBuf, String>> {
    if let Some(rest) = line.strip_prefix("OK\t") {
        let path = rest.split('\t').next().unwrap_or(rest).trim();
        return Some(Ok(PathBuf::from(path)));
    }
    if let Some(msg) = line.strip_prefix("ERR\t") {
        return Some(Err(msg.trim().into()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ok_with_sample_count() {
        let r = parse_response("OK\tC:\\tmp\\chaojiandai-tts\\tts_1.wav\t38400");
        assert!(matches!(r, Some(Ok(_))));
        if let Some(Ok(p)) = r {
            assert_eq!(p.to_str().unwrap(), "C:\\tmp\\chaojiandai-tts\\tts_1.wav");
        }
    }

    #[test]
    fn parse_ok_plain() {
        assert!(matches!(
            parse_response("OK\t/tmp/chaojiandai-tts/a.wav"),
            Some(Ok(_))
        ));
    }

    #[test]
    fn parse_err() {
        let r = parse_response("ERR\tRuntimeError: 模型未加载");
        assert!(matches!(r, Some(Err(_))));
    }

    #[test]
    fn parse_garbage_is_none() {
        assert!(parse_response("READY").is_none());
        assert!(parse_response("").is_none());
        assert!(parse_response("ok\tlower-case").is_none());
    }
}
