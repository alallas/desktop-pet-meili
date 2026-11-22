//! TTS sidecar（M1-3.5b + #35 预热缓存）：Python Kokoro 常驻合成服务的 Rust 侧封装。
//!
//! 线程模型与 llm::sidecar 一致：进程常驻、stdin 喂文本、stdout 回收 wav 路径。
//! Kokoro 模型加载 ~2s（坑 16 懒加载补丁），逐句 spawn 不可接受，故常驻。
//! 合成本身阻塞 ~1–3s（M0 实测 RTF 0.3），由引擎线程调用（与 ASR/LLM 阻塞同级，
//! 期间唤醒容忍漏听，采集通道有 2s 缓冲，见 engine.rs 注释）。
//!
//! #35 预热缓存（2026-11-22）：行为评论的台词全部来自 map.toml 的有限模板变体——
//! 启动后后台线程逐条预合成进 sidecar 缓存，之后这些台词零合成延迟直接播放；
//! 对话回复文本不可预知，仍走实时合成（延迟归 #23 族）。
//!
//! 并发模型：sidecar 为 `Arc<TtsSidecar>`，引擎线程与预热线程共享（`synth` 内部
//! 加锁、按 seq 关联响应）——协议升级出 seq 就是为了这条：否则两线程的请求/响应
//! 会在 stdout 管道里错位。
//!
//! stdout 读取：std 管道读无超时原语，故 spawn 时挂一个常驻读线程把响应行进
//! mpsc 通道，synth() 用 recv_timeout 拿到真超时——sidecar 卡死不会拖死调用方。
//!
//! 协议（行式）：入 = `<seq>\t<文本>`；出 = `OK\t<seq>\t<wav 路径>\t<样本数>`
//! 或 `ERR\t<seq>\t<消息>`。parse_response 是纯函数，独立单测。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// READY 等待上限：冷机加载（torch import + Kokoro + 预热）实测逼近 120s，放宽到 180s
const READY_TIMEOUT: Duration = Duration::from_secs(180);
/// 单句合成上限：M0 实测 RTF ~0.3，60 字 ≈ 15s 音频 ≈ 5s 合成，12 倍余量
const SYNTH_TIMEOUT: Duration = Duration::from_secs(60);

/// 共享句柄：引擎线程 + 预热线程并发用（#35）
pub type Shared = Arc<TtsSidecar>;

pub struct TtsSidecar {
    inner: Mutex<SidecarInner>,
}

struct SidecarInner {
    child: Child,
    stdin: ChildStdin,
    /// 读线程转发 sidecar stdout 响应行（就绪后首行必为 READY）
    rx: Receiver<std::io::Result<String>>,
    /// 请求序号：响应按它关联（并发安全的前提）
    seq: u64,
    /// #35：文本 → wav 缓存。Python 侧也有缓存——这里是第二道，只为避免
    /// 把请求发给 sidecar 的进程内往返（同文本本会话内反复出现时省 IPC）
    cache: HashMap<String, PathBuf>,
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
        let t0 = Instant::now();
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
        Ok(Self {
            inner: Mutex::new(SidecarInner {
                child,
                stdin,
                rx,
                seq: 0,
                cache: HashMap::new(),
            }),
        })
    }

    /// 合成一句：返回 wav 路径（24kHz 16-bit PCM 单声道）。失败 Err。
    /// 并发安全：内部加锁；响应按 seq 关联，跳过不属于本请求的响应行。
    pub fn synth(&self, text: &str) -> Result<PathBuf, String> {
        let clean: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        if clean.trim().is_empty() {
            return Err("合成文本为空".into());
        }
        let mut g = self
            .inner
            .lock()
            .map_err(|e| format!("tts 锁 poisoned: {e}"))?;
        if let Some(path) = g.cache.get(&clean) {
            tracing::info!(text = %clean, "tts cache hit (rust-side)");
            return Ok(path.clone());
        }
        g.seq += 1;
        let my_seq = g.seq;
        g.stdin
            .write_all(format!("{my_seq}\t").as_bytes())
            .and_then(|_| g.stdin.write_all(clean.as_bytes()))
            .and_then(|_| g.stdin.write_all(b"\n"))
            .and_then(|_| g.stdin.flush())
            .map_err(|e| format!("写 sidecar stdin 失败: {e}"))?;

        loop {
            match g.rx.recv_timeout(SYNTH_TIMEOUT) {
                Ok(Ok(line)) => match parse_response(line.trim()) {
                    Some(Ok((seq, path))) => {
                        if seq == my_seq {
                            g.cache.insert(clean.clone(), path.clone());
                            return Ok(path);
                        }
                        // 其他请求的响应（预热线程的）：跳过
                    }
                    Some(Err((seq, msg))) => {
                        if seq == my_seq {
                            return Err(format!("sidecar 合成失败: {msg}"));
                        }
                    }
                    None => tracing::warn!(line = %line, "sidecar 响应无法解析，跳过"),
                },
                Ok(Err(e)) => return Err(format!("读 sidecar 响应失败: {e}")),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    return Err(format!("sidecar 合成超时（{}s）", SYNTH_TIMEOUT.as_secs()));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("tts sidecar 已退出".into());
                }
            }
        }
    }
}

impl Drop for SidecarInner {
    fn drop(&mut self) {
        if let Err(e) = self.child.kill() {
            let _ = writeln!(std::io::stderr(), "tts sidecar kill 失败: {e}");
        }
        let _ = self.child.wait();
    }
}

/// 后台预热（#35）：把模板台词逐条送合成进缓存。与引擎线程共享 sidecar；
/// 预热期间用户请求正常服务（锁竞争最坏多等一个合成时长）。空行/带槽位
/// 的模板（`{{field}}`）由调用方过滤——槽位台词首次真实出现时才合成并缓存。
pub fn prewarm(sidecar: Shared, lines: Vec<String>) {
    if lines.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        let total = lines.len();
        let t0 = Instant::now();
        let mut ok = 0usize;
        for line in lines {
            if sidecar.synth(&line).is_ok() {
                ok += 1;
            }
        }
        tracing::info!(
            ok,
            total,
            elapsed_s = t0.elapsed().as_secs(),
            "tts prewarm done"
        );
    });
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
    if std::path::Path::new(&managed).exists() {
        return Ok(managed.into());
    }
    Ok("python".into())
}

/// 解析 sidecar 响应行：`OK\t<seq>\t<路径>[\t样本数]` / `ERR\t<seq>\t<消息>`。
/// 纯函数独立单测。
fn parse_response(line: &str) -> Option<Result<(u64, PathBuf), (u64, String)>> {
    let mut parts = line.split('\t');
    let tag = parts.next()?;
    let seq: u64 = parts.next()?.parse().ok()?;
    match tag {
        "OK" => {
            let path = parts.next()?.trim();
            Some(Ok((seq, PathBuf::from(path))))
        }
        "ERR" => Some(Err((seq, parts.next().unwrap_or("").trim().into()))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ok_with_sample_count() {
        let r = parse_response("OK\t7\tC:\\tmp\\chaojiandai-tts\\tts_1.wav\t38400");
        assert!(matches!(r, Some(Ok((7, _)))));
    }

    #[test]
    fn parse_ok_plain() {
        assert!(matches!(
            parse_response("OK\t1\t/tmp/chaojiandai-tts/a.wav"),
            Some(Ok((1, _)))
        ));
    }

    #[test]
    fn parse_err_carries_seq() {
        let r = parse_response("ERR\t3\tRuntimeError: 模型未加载");
        match r {
            Some(Err((3, msg))) => assert!(msg.contains("模型未加载")),
            _ => panic!("ERR 解析失败"),
        }
    }

    #[test]
    fn parse_garbage_is_none() {
        assert!(parse_response("READY").is_none());
        assert!(parse_response("").is_none());
        assert!(parse_response("ok\t1\tlower-case").is_none());
        assert!(parse_response("OK\tnan\tpath").is_none()); // seq 必须数字
        assert!(parse_response("OK\t1").is_none()); // OK 缺路径
    }
}
