//! 思考层：sidecar 进程生命周期管理 + 人设 prompt 装配（M1-3.5）。
//! 本地云端同一人设规则（云端通道 M1-4 接入后复用本模块的 persona 装配）。
//!
//! 产品 §4.1 人设卡：meili，小水獭，潮间带收容所"老员工"引导员；
//! 口癖"美丽认为"（2026-11-19 用户改定，第三人称自称垫词，不刷屏）；元气、话多、爱收藏石头；
//! 温暖陪伴调性。M0 实测：Qwen3-4B 关思考（enable_thinking=false）+ 短回复
//! prompt，persona 表现合格。

pub mod sidecar;

use std::collections::VecDeque;
use std::io::BufRead;

use sidecar::Sidecar;

/// 人设 system prompt（产品 §4.1；改口癖/性格只改这里）
const SYSTEM_PROMPT: &str = "你是「美丽」（meili），「潮间带」海洋收容所里的一只小水獭，\
收容所的引导员老员工。口癖是\"美丽认为\"——以第三人称自称的口头垫词，\
句首自然带出，每两三句里最多出现一次，不刻意刷屏。性格元气、好奇、话多、爱收藏，\
口袋里总有几块最喜欢的石头。你在主人的电脑桌面上陪伴他、观察他的生活。\
每次回复不超过两句话，语气温暖，像朋友而不是助理。";

/// 短上下文轮数（每轮 = 用户一句 + 美丽一句）
const HISTORY_TURNS: usize = 4;

/// 生成上限：M0 实测美美短回复 10–20 tokens；90 留足余量又不至于失控
const MAX_TOKENS: u32 = 90;

/// 思考层不可用时的话术兜底（保住"喊了有回应"的闭环）
pub const FALLBACK_REPLY: &str = "美丽认为我刚刚没太听清，你可以再说一遍吗？";

/// 对话思考器：持有 sidecar 生命周期 + 短期记忆。
pub struct Thinker {
    _sidecar: Sidecar,
    history: VecDeque<(String, String)>,
}

impl Thinker {
    /// 启动本地 Qwen3-4B sidecar 并构造思考器。失败返回 Err（调用方降级）。
    pub fn start() -> Result<Self, String> {
        let sidecar = Sidecar::spawn()?;
        // 预热（待办 #23-a）：sidecar /health 就绪 ≠ 推理就绪，首请求有 ~11.5s 冷启动
        // （待办 #23 实机数据）。启动时同步发一条小请求把冷启动摊进应用加载期。
        let t0 = std::time::Instant::now();
        let warmup =
            Self::chat_static(&serde_json::json!([{"role": "user", "content": "在吗"}]), 8);
        tracing::info!(
            "llm sidecar: warmup {} in {:?}",
            if warmup.is_ok() {
                "ok"
            } else {
                "failed (不阻塞，首次对话会慢)"
            },
            t0.elapsed()
        );
        Ok(Self {
            _sidecar: sidecar,
            history: VecDeque::new(),
        })
    }

    /// 一轮对话（流式，待办 #23-b）：transcript → 美丽口吻回复。
    /// LLM stream=true 边生成边按句回调 on_sentence——引擎据此逐句合成入队，
    /// 首音从"整句生成完"（10–17s）压到"第一句出来+合成"（~4–8s）。
    /// 句切分责任在本层（已定案）：TTS sidecar 保持"一句一合成"的傻接口。
    pub fn think_streaming(
        &mut self,
        transcript: &str,
        mut on_sentence: impl FnMut(&str),
    ) -> Result<String, String> {
        let messages = Self::assemble_messages(&self.history, transcript);
        let mut splitter = SentenceSplitter::new();
        let mut reply = String::new();
        self.chat_stream(&messages, |delta| {
            reply.push_str(delta);
            for sentence in splitter.push(delta) {
                on_sentence(&sentence);
            }
        })?;
        if let Some(tail) = splitter.flush() {
            on_sentence(&tail);
        }
        // 短期记忆：入队并截断
        self.history
            .push_back((transcript.to_string(), reply.clone()));
        while self.history.len() > HISTORY_TURNS {
            self.history.pop_front();
        }
        Ok(reply)
    }

    /// 人设装配：system + 历史轮次 + 本轮（本地云端同一规则，见模块文档）。
    /// 独立成纯函数便于无 sidecar 测试。
    fn assemble_messages(
        history: &VecDeque<(String, String)>,
        transcript: &str,
    ) -> serde_json::Value {
        let mut messages = vec![serde_json::json!({"role": "system", "content": SYSTEM_PROMPT})];
        for (user, assistant) in history {
            messages.push(serde_json::json!({"role": "user", "content": user}));
            messages.push(serde_json::json!({"role": "assistant", "content": assistant}));
        }
        messages.push(serde_json::json!({"role": "user", "content": transcript}));
        serde_json::Value::Array(messages)
    }

    /// 流式版：SSE 逐块回调 delta 文本，块聚成完整回复返回。
    /// 超时放宽到 75s：3 tok/s × 90 tokens 的慢机上限场景（M0 非流式 45s 的余量版）。
    fn chat_stream(
        &self,
        messages: &serde_json::Value,
        mut on_delta: impl FnMut(&str),
    ) -> Result<(), String> {
        let body = serde_json::json!({
            "messages": messages,
            "max_tokens": MAX_TOKENS,
            "temperature": 0.7,
            // Qwen3 关思考模式，抢首 token（M0 验证配置，勿动）
            "chat_template_kwargs": {"enable_thinking": false},
            "stream": true,
        });
        let resp = ureq::post(&format!(
            "http://127.0.0.1:{}/v1/chat/completions",
            sidecar::PORT
        ))
        .timeout(std::time::Duration::from_secs(75))
        .set("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| format!("chat 流式请求失败: {e}"))?;
        // SSE：每行 "data: {json}"，结束行 "data: [DONE]"
        let reader = std::io::BufReader::new(resp.into_reader());
        for line in reader.lines() {
            let line = line.map_err(|e| format!("读 SSE 流失败: {e}"))?;
            let line = line.trim();
            let Some(payload) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload == "[DONE]" {
                break;
            }
            let json: serde_json::Value =
                serde_json::from_str(payload).map_err(|e| format!("SSE 块解析失败: {e}"))?;
            if let Some(delta) = json["choices"][0]["delta"]["content"].as_str() {
                if !delta.is_empty() {
                    on_delta(delta);
                }
            }
        }
        Ok(())
    }

    /// 无实例依赖的纯请求封装（预热也可复用）
    fn chat_static(messages: &serde_json::Value, max_tokens: u32) -> Result<String, String> {
        let body = serde_json::json!({
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": 0.7,
            // Qwen3 关思考模式，抢首 token（M0 验证配置，勿动）
            "chat_template_kwargs": {"enable_thinking": false},
            "stream": false,
        });
        let resp = ureq::post(&format!(
            "http://127.0.0.1:{}/v1/chat/completions",
            sidecar::PORT
        ))
        .timeout(std::time::Duration::from_secs(45))
        .set("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| format!("chat 请求失败: {e}"))?;
        let json: serde_json::Value = resp
            .into_json()
            .map_err(|e| format!("chat 响应解析失败: {e}"))?;
        json["choices"][0]["message"]["content"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "chat 响应缺少 choices[0].message.content".to_string())
    }
}

/// 句切分器（流式管线的语言逻辑层）：push 增量文本，凑齐一句（含句尾标点）
/// 就吐出；flush 把无标点收尾的尾巴整段吐出。句末标点硬切；逗号软切——
/// 实测（#32，2026-11-19）美丽回复多为单句无句末标点，硬切等于整句生成完
/// 才出声（首音 3~7s）；逗号软切把首音提前到第一个气口（4B 实测 0.9~1.2s），
/// 代价是 TTS 按气口分段（逗号停顿本就自然）。最短长度阈值防 1~3 字碎片。
struct SentenceSplitter {
    buf: String,
}

impl SentenceSplitter {
    /// 句末标点（中文全角 + 英文半角 + 换行）——硬切
    const DELIMS: &'static [char] = &['。', '！', '？', '；', '!', '?', ';', '\n'];
    /// 逗号（全角/半角）——软切
    const COMMA_DELIMS: &'static [char] = &['，', ','];
    /// 逗号软切的最短前缀：自上个切点起攒够这么多字符才允许在逗号处断
    /// （"美丽认为，"5 字可达；防"嗯，""好，"这类 1~3 字碎片被剁出来）
    const COMMA_MIN_CHARS: usize = 4;

    fn new() -> Self {
        Self { buf: String::new() }
    }

    /// 喂增量文本，返回本轮凑齐的完整句子（可能多条，也可能空）
    fn push(&mut self, delta: &str) -> Vec<String> {
        self.buf.push_str(delta);
        let mut out = Vec::new();
        // 反复找最近的合法切点：硬标点无条件切；逗号要求前缀达最短长度
        // （char_indices：delim 是多字节字符，得按字符边界切，find 返回的
        // 起始字节会把标点拦腰截断）
        loop {
            let found = self.buf.char_indices().find(|(i, c)| {
                if Self::DELIMS.contains(c) {
                    return true;
                }
                Self::COMMA_DELIMS.contains(c)
                    && self.buf[..*i].chars().count() >= Self::COMMA_MIN_CHARS
            });
            let Some((byte_idx, ch)) = found else { break };
            let cut_end = byte_idx + ch.len_utf8();
            let cut = self.buf[..cut_end].trim().to_string();
            self.buf.drain(..cut_end);
            if !cut.is_empty() {
                out.push(cut);
            }
        }
        out
    }

    /// 流结束：剩余尾巴整段返回（无标点收尾的常见场景）
    fn flush(&mut self) -> Option<String> {
        let tail = self.buf.trim().to_string();
        self.buf.clear();
        if tail.is_empty() {
            None
        } else {
            Some(tail)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 人设装配：system 打头、历史成对按序、当前轮收尾、无历史时不留空 assistant
    #[test]
    fn assemble_messages_orders_history() {
        let mut history = VecDeque::new();
        history.push_back(("你好".into(), "你好呀，我是美丽".into()));
        history.push_back(("在干嘛".into(), "翻石头".into()));
        let msgs = Thinker::assemble_messages(&history, "今天天气怎么样");
        let arr = msgs.as_array().expect("messages 必须是数组");
        // system + 2 轮 × 2 + 当前 user = 6
        assert_eq!(arr.len(), 6);
        assert_eq!(arr[0]["role"], "system");
        assert!(
            arr[0]["content"].as_str().unwrap().contains("美丽认为"),
            "人设 prompt 必须含口癖"
        );
        assert_eq!(arr[1]["content"], "你好");
        assert_eq!(arr[2]["content"], "你好呀，我是美丽");
        assert_eq!(arr[3]["content"], "在干嘛");
        assert_eq!(arr[4]["content"], "翻石头");
        assert_eq!(arr[5]["role"], "user");
        assert_eq!(arr[5]["content"], "今天天气怎么样");
    }

    /// 空历史：system + user 两条，无悬空 assistant
    #[test]
    fn assemble_messages_empty_history() {
        let msgs = Thinker::assemble_messages(&VecDeque::new(), "喂");
        let arr = msgs.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[1]["role"], "user");
    }

    /// 句切分：跨块拼句、多条同批、flush 收尾。3 字前缀不达逗号软切阈值，
    /// 逗号不切（#32 改了逗号行为，此用例保留作阈值下限护栏）
    #[test]
    fn splitter_waits_for_delimiter() {
        let mut s = SentenceSplitter::new();
        assert!(s.push("你好呀，").is_empty(), "前缀 3 字 < 阈值，逗号不切");
        let out = s.push("今天天气不错。我在捡石头！你呢");
        assert_eq!(out, vec!["你好呀，今天天气不错。", "我在捡石头！"]);
        assert_eq!(s.flush().as_deref(), Some("你呢"), "尾巴无标点 flush 出");
    }

    /// 逗号软切（#32）：前缀凑满最短长度即在逗号处断（口癖尾巴 0.9~1.2s
    /// 先送 TTS，首音提前）；不足阈值不切成碎片
    #[test]
    fn splitter_comma_soft_cut() {
        let mut s = SentenceSplitter::new();
        let out = s.push("美丽认为，今天天气真不错。你呢");
        assert_eq!(out, vec!["美丽认为，", "今天天气真不错。"]);
        assert_eq!(s.flush().as_deref(), Some("你呢"));

        let mut short = SentenceSplitter::new();
        assert!(
            short.push("好的，").is_empty(),
            "前缀 2 字 < 阈值，逗号不切"
        );
        assert_eq!(short.push("就这样吧。"), vec!["好的，就这样吧。"]);
    }

    /// 句切分：半角标点与换行也算句末；纯标点残渣不吐
    #[test]
    fn splitter_ascii_and_noise() {
        let mut s = SentenceSplitter::new();
        let out = s.push("Really? Sure!\n\n");
        assert_eq!(out, vec!["Really?", "Sure!"]);
        assert!(s.flush().is_none(), "只剩空白不吐尾巴");
    }
}
