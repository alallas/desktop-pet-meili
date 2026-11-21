//! 生物人设档案（PersonaSpec）：一动物一份 TOML（resources/creatures/<id>.toml），
//! 启动时读取——改人设/口癖不用重新编译。文件缺失/损坏回退编译期内置副本。
//! M2 多动物：按 id 各加载一份；运行时状态（亲密度、事件流水）仍归 SQLite，
//! 本模块只管"内容"。

use serde::Deserialize;

/// 内置副本 = 同一份 meili.toml（include_str 双保险：文件被误删/打包路径
/// 异常时人设不丢，行为退化为该次编译时的版本）。M2 多动物时每个 id 各嵌一份。
const EMBEDDED_MEILI: &str = include_str!("../../resources/creatures/meili.toml");

/// 「潮间带」生物人设档案（resources/creatures/<id>.toml）。
/// quirk 三字段全缺 = 无口癖动物，装配时整个口癖段落省略。
#[derive(Debug, Clone, Deserialize)]
pub struct PersonaSpec {
    /// 唤醒/事件用 id（M2 时也是档案文件名）
    pub id: String,
    /// 名字（prompt 与 TTS 称呼）
    pub name: String,
    pub species: String,
    pub role: String,
    /// 口癖（可选）：无口癖动物在 TOML 里删掉 quirk 三行
    pub quirk: Option<String>,
    /// 口癖风格描述（如"以第三人称自称的口头垫词"）
    pub quirk_desc: Option<String>,
    /// 口癖后带逗号停顿（锚定流式逗号软切的首音，见 SentenceSplitter）
    #[serde(default)]
    pub quirk_comma_habit: bool,
    pub personality: String,
    pub setting: String,
    #[serde(default = "default_max_sentences")]
    pub max_sentences: u32,
}

fn default_max_sentences() -> u32 {
    2
}

impl PersonaSpec {
    /// 装配 system prompt。无口癖动物（quirk/quirk_desc 任一缺失）不出口癖段落。
    pub fn system_prompt(&self) -> String {
        let mut s = format!(
            "你是「{}」（{}），「潮间带」海洋收容所里的一只{}，{}。",
            self.name, self.id, self.species, self.role
        );
        if let (Some(quirk), Some(desc)) = (&self.quirk, &self.quirk_desc) {
            let comma = if self.quirk_comma_habit {
                "，口癖后习惯带逗号停顿"
            } else {
                ""
            };
            s.push_str(&format!(
                "口癖是\"{quirk}\"——{desc}，句首自然带出{comma}，\
                 每两三句里最多出现一次，不刻意刷屏。"
            ));
        }
        s.push_str(&format!(
            "性格{}。{}。每次回复不超过{}句话，语气温暖，像朋友而不是助理。",
            self.personality, self.setting, self.max_sentences
        ));
        s
    }

    /// 从 creatures 目录按 id 加载档案；文件缺失/解析失败回退内置副本并告警。
    /// M2 多动物前只有 meili 有内置副本，其他 id 失败也回退 meili（人设降级
    /// 优于无回应），记 error 日志提示配置错误。
    pub fn load(id: &str) -> Self {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources/creatures")
            .join(format!("{id}.toml"));
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(spec) => return spec,
                Err(e) => {
                    tracing::warn!("人设档案解析失败 {}，回退内置副本: {e}", path.display())
                }
            },
            Err(e) => tracing::warn!("人设档案缺失 {}，回退内置副本: {e}", path.display()),
        }
        if id != "meili" {
            tracing::error!("档案 {id} 无内置副本可回退，人设降级为 meili");
        }
        toml::from_str(EMBEDDED_MEILI).expect("内置 meili 档案必须可解析")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 有口癖动物的装配：口癖段落完整、关键信息齐全
    #[test]
    fn meili_prompt_has_quirk() {
        let p = PersonaSpec::load("meili");
        let s = p.system_prompt();
        assert!(s.contains("美丽认为"), "口癖必须来自档案");
        assert!(s.contains("口癖后习惯带逗号停顿"), "逗号习惯锚定必须生效");
        assert!(s.contains("不超过2句话"));
        assert!(s.starts_with("你是「美丽」（meili）"));
    }

    /// 无口癖动物（quirk 三字段全缺）：不出口癖段落
    #[test]
    fn quirkless_animal_omits_quirk_clause() {
        let p: PersonaSpec = toml::from_str(
            r#"
id = "tangtang"
name = "糖糖"
species = "小寄居蟹"
role = "所里的邮差"
personality = "慢吞吞、认真"
setting = "你总在沙滩和礁石之间送信"
"#,
        )
        .unwrap();
        let s = p.system_prompt();
        assert!(!s.contains("口癖"), "无口癖动物不得出口癖段落");
        assert!(s.contains("糖糖"));
        assert!(s.contains("不超过2句话"), "max_sentences 走默认值");
    }
}
