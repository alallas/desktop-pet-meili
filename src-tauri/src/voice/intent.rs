//! 语音调节指令（产品 §5.1 存在感光谱的开口频率维度）：转写文本 → 行为档位。
//! 放 voice 域因为拦截点在语音管线（转写后、LLM 前）；语义是行为调节指令。
//! v1 关键词表匹配——封闭小集合用表（工程规范 §8），LLM 意图识别推迟。

use crate::behavior::gates::FrequencyTier;

/// 命中返回 (目标档位, 确认台词)。匹配 = 关键词子串命中（转写无标点也能中）。
/// 顺序即优先级。关键词取口语短词、宁缺勿滥——"恢复正常"比"正常"安全
/// （避免"正常吗"误切档位）。
pub fn match_tier_command(text: &str) -> Option<(FrequencyTier, &'static str)> {
    const TABLE: &[(&[&str], FrequencyTier, &str)] = &[
        (
            &["别吵", "安静一点", "别说话", "少说话", "闭嘴"],
            FrequencyTier::Silent,
            "好，那美丽安静待着，你需要时再叫我。",
        ),
        (
            &["多陪我聊聊", "多说话", "陪我说话", "热闹一点"],
            FrequencyTier::Chatty,
            "好呀，那美丽多陪你聊聊！",
        ),
        (
            &["恢复", "不多聊", "少聊一点"],
            FrequencyTier::Balanced,
            "好，恢复平时的节奏。",
        ),
    ];
    let t = text.trim();
    TABLE
        .iter()
        .find(|(keys, _, _)| keys.iter().any(|k| t.contains(k)))
        .map(|(_, tier, ack)| (*tier, *ack))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_each_tier() {
        let (tier, ack) = match_tier_command("今天别吵我").unwrap();
        assert_eq!(tier, FrequencyTier::Silent);
        assert!(!ack.is_empty());

        let (tier, _) = match_tier_command("那你多陪我聊聊吧").unwrap();
        assert_eq!(tier, FrequencyTier::Chatty);

        let (tier, _) = match_tier_command("恢复正常模式").unwrap();
        assert_eq!(tier, FrequencyTier::Balanced);
    }

    #[test]
    fn no_punctuation_still_matches() {
        // ASR 转写常无标点
        assert!(match_tier_command("别吵").is_some());
        assert!(match_tier_command("多说话").is_some());
    }

    #[test]
    fn chat_and_smalltalk_not_matched() {
        // 日常闲聊不得误触档位切换（回归守护）
        assert!(match_tier_command("今天天气怎么样").is_none());
        assert!(match_tier_command("给我讲个故事").is_none());
        assert!(match_tier_command("").is_none());
    }

    #[test]
    fn priority_silent_before_balanced() {
        // "别吵我，让我安静"同时含安静类词：先命中沉默档（表顺序）
        let (tier, _) = match_tier_command("别吵我让我安静一会").unwrap();
        assert_eq!(tier, FrequencyTier::Silent);
    }
}
