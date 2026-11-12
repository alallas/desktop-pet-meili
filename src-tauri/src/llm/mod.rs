//! 思考层：sidecar 进程生命周期管理 + 人设 prompt 装配（M1-3.5）。
//! 本地云端同一人设规则（云端通道 M1-4 接入后复用本模块的 persona 装配）。
//!
//! 产品 §4.1 人设卡：meili，小水獭，潮间带收容所"老员工"引导员；
//! 口癖"然后就是"（产品设计 L369 定稿，不刷屏）；元气、话多、爱收藏石头；
//! 温暖陪伴调性。M0 实测：Qwen3-4B 关思考（enable_thinking=false）+ 短回复
//! prompt，persona 表现合格。

pub mod sidecar;

use std::collections::VecDeque;

use sidecar::Sidecar;

/// 人设 system prompt（产品 §4.1；改口癖/性格只改这里）
const SYSTEM_PROMPT: &str = "你是「美丽」（meili），「潮间带」海洋收容所里的一只小水獭，\
收容所的引导员老员工。口癖是\"然后就是\"——朋友聊天式的口头垫词，自然地挂在嘴边，\
每两三句里最多出现一次，不刻意刷屏。性格元气、好奇、话多、爱收藏，\
口袋里总有几块最喜欢的石头。你在主人的电脑桌面上陪伴他、观察他的生活。\
每次回复不超过两句话，语气温暖，像朋友而不是助理。";

/// 短上下文轮数（每轮 = 用户一句 + 美丽一句）
const HISTORY_TURNS: usize = 4;

/// 生成上限：M0 实测美美短回复 10–20 tokens；90 留足余量又不至于失控
const MAX_TOKENS: u32 = 90;

/// 思考层不可用时的话术兜底（保住"喊了有回应"的闭环）
pub const FALLBACK_REPLY: &str = "然后就是我刚刚没太听清，你可以再说一遍吗？";

/// 对话思考器：持有 sidecar 生命周期 + 短期记忆。
pub struct Thinker {
    _sidecar: Sidecar,
    history: VecDeque<(String, String)>,
}

impl Thinker {
    /// 启动本地 Qwen3-4B sidecar 并构造思考器。失败返回 Err（调用方降级）。
    pub fn start() -> Result<Self, String> {
        Ok(Self {
            _sidecar: Sidecar::spawn()?,
            history: VecDeque::new(),
        })
    }

    /// 一轮对话：transcript → 美丽口吻回复。阻塞 ~2–5s（M0 实测 2.4–4.4s）。
    pub fn think(&mut self, transcript: &str) -> Result<String, String> {
        let messages = Self::assemble_messages(&self.history, transcript);
        let reply = self.chat(&messages)?;
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

    /// OpenAI 兼容 /v1/chat/completions（llama-server）
    fn chat(&self, messages: &serde_json::Value) -> Result<String, String> {
        let body = serde_json::json!({
            "messages": messages,
            "max_tokens": MAX_TOKENS,
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
            arr[0]["content"].as_str().unwrap().contains("然后就是"),
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
}
