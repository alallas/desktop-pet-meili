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
        // 杀进程**树**：venv 的 python.exe 是 ~260KB 的重定向壳，真正跑脚本的是
        // 它派生的基底解释器子进程（实测父 4MB 空转 / 子 1.2GB 装模型，坑 53）——
        // 只 kill 父会把 1.2GB 的子进程 orphan 掉（历史孤儿 sidecar 的成因）。
        // taskkill /T 随父杀整棵树；taskkill 恒在（System32），失败回退单杀
        let pid = self.child.id();
        let tree_killed = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !tree_killed {
            if let Err(e) = self.child.kill() {
                let _ = writeln!(std::io::stderr(), "tts sidecar kill 失败: {e}");
            }
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
        // 两轮：首轮失败（超时）的台词收尾重试一次。失败主因是启动期 CPU
        // 竞争——预热与 llama-server 加载（16 线程）并发，个别合成被饿超时
        // （2026-11-22 实证 27/28）；重试发生在 llama 加载完成后，基本必过。
        // 仍失败的记 warn + 原文——"哪条台词没进缓存"此前不可见
        let mut failed: Vec<String> = Vec::new();
        let mut ok = 0usize;
        for line in lines {
            if sidecar.synth(&line).is_ok() {
                ok += 1;
            } else {
                failed.push(line);
            }
        }
        for line in failed {
            if sidecar.synth(&line).is_ok() {
                ok += 1;
            } else {
                tracing::warn!(text = %line, "tts prewarm 重试仍失败（该台词首用时现场合成）");
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

/// 流水线化合成（#23）：后台 worker 线程串行消费合成任务，LLM 流式出句、
/// TTS 合成、播放三者并行——多句回复不再句句串行等 1.5~3.6s 合成。
///
/// 生产者-消费者 + 代次作废（generation token）：
/// - 引擎（生产者）把句子 `send` 进有界通道（背压：worker 跟不上时阻塞 LLM
///   流读取，天然限流），`bump_gen` 开新一轮对话/打断时递增代次；
/// - worker（单消费者，保句子顺序）发现任务代次过期直接丢弃（不浪费合成），
///   合成完把 wav 字节连同代次发回；引擎 drain 时丢弃过期代次的结果。
/// - `in_flight`/`ready` 两个计数让引擎能在"队列恰好播空但后续句子还在
///   合成"时不误判回复结束（防提前 SpeakEnded → 提前回 Idle 被评论插话）。
pub struct SynthPipe {
    tx: std::sync::mpsc::SyncSender<SynthJob>,
    /// 引擎侧消费端（只有引擎线程读，故 Receiver 无需共享）
    rx: Receiver<SynthDone>,
    /// 当前代次：引擎 bump、worker 比对，打断/新轮回复时旧任务整体作废
    gen: Arc<std::sync::atomic::AtomicU64>,
    /// 已派发未完成的任务数（引擎 +1，worker 完成/丢弃 -1）
    in_flight: Arc<std::sync::atomic::AtomicUsize>,
    /// 已完成待引擎取走的结果数（worker +1，引擎 drain -1）
    ready: Arc<std::sync::atomic::AtomicUsize>,
}

pub struct SynthJob {
    pub gen: u64,
    pub text: String,
}

pub struct SynthDone {
    pub gen: u64,
    pub text: String,
    /// 合成产物（wav 全字节，引擎直接入播放器；读文件也收进 worker）
    pub wav: Result<Vec<u8>, String>,
}

/// 背压上限：在途 + 待取结果合计不超过 8 句（LLM 流读取被自然阻塞）
const PIPE_CAP: usize = 8;

impl SynthPipe {
    /// 启动 worker 线程。sidecar 由 pipe 共享持有——引擎线程另有 Arc 引用
    /// （预热也持有），pipe 只为 worker 续命。
    pub fn spawn(sidecar: Shared) -> Self {
        let (tx, rx_jobs) = std::sync::mpsc::sync_channel::<SynthJob>(PIPE_CAP);
        let (tx_done, rx) = channel::<SynthDone>();
        let gen = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let in_flight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let ready = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        {
            let gen = Arc::clone(&gen);
            let in_flight = Arc::clone(&in_flight);
            let ready = Arc::clone(&ready);
            std::thread::spawn(move || {
                use std::sync::atomic::Ordering::Relaxed;
                while let Ok(job) = rx_jobs.recv() {
                    // 代次作废：引擎已开新轮/打断，旧句子不值得再合成
                    if job.gen != gen.load(Relaxed) {
                        in_flight.fetch_sub(1, Relaxed);
                        continue;
                    }
                    let wav = sidecar.synth(&job.text).and_then(|p| {
                        std::fs::read(&p).map_err(|e| format!("读 wav 失败 {p:?}: {e}"))
                    });
                    ready.fetch_add(1, Relaxed);
                    // 引擎已退出则发送失败：计数留着无妨（进程收尾）
                    let _ = tx_done.send(SynthDone {
                        gen: job.gen,
                        text: job.text,
                        wav,
                    });
                    in_flight.fetch_sub(1, Relaxed);
                }
            });
        }
        Self {
            tx,
            rx,
            gen,
            in_flight,
            ready,
        }
    }

    /// 新一轮对话：代次 +1（旧在途任务与结果整体作废）。返回新代次。
    pub fn bump_gen(&self) -> u64 {
        self.gen.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
    }

    /// 派发一句合成。在途计数 +1（worker 完成/作废时 -1）。
    pub fn send(&self, gen: u64, text: &str) -> Result<(), String> {
        use std::sync::atomic::Ordering::Relaxed;
        self.in_flight.fetch_add(1, Relaxed);
        self.tx
            .try_send(SynthJob {
                gen,
                text: text.to_string(),
            })
            .map_err(|e| {
                self.in_flight.fetch_sub(1, Relaxed);
                format!("合成队列不可用: {e}")
            })
    }

    /// 取一个有效结果：弹出一条已完成合成，过期代次返回 None（已消费且
    /// ready 计数已扣）。引擎循环 while let Some(d) = pop_valid()。
    pub fn pop_valid(&self) -> Option<SynthDone> {
        use std::sync::atomic::Ordering::Relaxed;
        let done = self.rx.try_recv().ok()?;
        self.ready.fetch_sub(1, Relaxed);
        if done.gen != self.gen.load(Relaxed) {
            return None; // 过期：丢弃，继续取
        }
        Some(done)
    }

    /// 是否还有未完成的回复音频（在途 + 待取）。引擎"播完"判定用——
    /// 队列恰好播空但后续句子仍在合成时不得提前判结束。
    pub fn has_pending(&self) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        self.in_flight.load(Relaxed) > 0 || self.ready.load(Relaxed) > 0
    }
}

/// 探测 TTS sidecar 的 Python 解释器（#14 固化后的优先级）：
/// env 覆盖（测试/打包注入）> **仓库自有 venv**（tools/tts-venv，
/// scripts/setup-tts-venv.ps1 产物，锁定版本 + 懒加载补丁，打包即带走）>
/// 本机托管 venv（开发机遗留兜底）> PATH 上的 python（报错信息可见，不静默）
fn find_python() -> Result<String, String> {
    if let Ok(p) = std::env::var("CHAOJIANDAI_PYTHON") {
        if std::path::Path::new(&p).exists() {
            return Ok(p);
        }
    }
    let repo_venv = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tools/tts-venv/Scripts/python.exe");
    if repo_venv.exists() {
        return Ok(repo_venv.to_string_lossy().into_owned());
    }
    // 开发机兜底：Kimi 托管 venv（坑 16 打过补丁的那份）。新机器应跑
    // setup-tts-venv.ps1 生成仓库 venv，不走这里
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

    /// 手工构造 pipe（不起 worker）：验证代次作废过滤与计数语义
    fn make_pipe() -> (
        SynthPipe,
        std::sync::mpsc::Sender<SynthDone>,
        std::sync::mpsc::Receiver<SynthJob>,
    ) {
        let (tx, rx_jobs) = std::sync::mpsc::sync_channel(4);
        let (tx_done, rx) = channel();
        let pipe = SynthPipe {
            tx,
            rx,
            gen: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            in_flight: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            ready: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        };
        (pipe, tx_done, rx_jobs)
    }

    fn done(gen: u64) -> SynthDone {
        SynthDone {
            gen,
            text: format!("句{gen}"),
            wav: Ok(vec![1, 2, 3]),
        }
    }

    #[test]
    fn pipe_stale_gen_results_dropped_fifo_order_kept() {
        use std::sync::atomic::Ordering::Relaxed;
        let (pipe, tx_done, _rx_jobs) = make_pipe();
        // 代次 0 的两条 + 新代次 1 的一条（bump 后 0 即过期）
        tx_done.send(done(0)).unwrap();
        tx_done.send(done(0)).unwrap();
        pipe.ready.store(3, Relaxed);
        tx_done.send(done(1)).unwrap();
        assert_eq!(pipe.bump_gen(), 1);
        assert!(pipe.pop_valid().is_none(), "过期代次第一条丢弃");
        assert!(pipe.pop_valid().is_none(), "过期代次第二条丢弃");
        let d = pipe.pop_valid().expect("新代次应取到");
        assert_eq!(d.gen, 1);
        assert!(pipe.pop_valid().is_none(), "取空");
    }

    #[test]
    fn pipe_pending_reflects_in_flight_and_ready() {
        use std::sync::atomic::Ordering::Relaxed;
        let (pipe, tx_done, _rx_jobs) = make_pipe();
        assert!(!pipe.has_pending());
        pipe.in_flight.store(1, Relaxed);
        assert!(pipe.has_pending(), "在途即 pending");
        pipe.in_flight.store(0, Relaxed);
        tx_done.send(done(0)).unwrap();
        pipe.ready.store(1, Relaxed);
        assert!(pipe.has_pending(), "待取即 pending");
        assert!(pipe.pop_valid().is_some());
        assert!(!pipe.has_pending(), "取空后释放");
    }

    #[test]
    fn pipe_send_backpressure_full_channel_errs() {
        use std::sync::atomic::Ordering::Relaxed;
        let (pipe, _tx_done, rx_jobs) = make_pipe(); // 容量 4
        for _ in 0..4 {
            pipe.send(0, "句子").unwrap();
        }
        assert_eq!(pipe.in_flight.load(Relaxed), 4);
        // 队列满：派发失败且在途计数回滚
        assert!(pipe.send(0, "溢出").is_err());
        assert_eq!(pipe.in_flight.load(Relaxed), 4, "失败派发不得漏计数");
        drop(rx_jobs);
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
