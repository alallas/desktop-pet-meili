//! 事件映射表（map.toml）：事件 → 反应。生物性格的主载体（M1-5 定案）。
//!
//! - 每生物一份 `resources/creatures/<id>.map.toml`，启动时加载；
//!   整文件缺失/损坏回退编译期内置副本（同 persona.rs 的双保险策略）
//! - 单条格式错误（on 非封闭事件名 / say 为空）→ 告警并跳过该条，不影响其他条目
//! - 台词模板 `{{field}}` 槽位只取事件 payload 的字符串字段；字段缺失则
//!   该变体不选（换下一条）。v1 不做 mood 类虚拟槽（2026-11-21 定案）
//! - anim 小动作（M1-6 落地）：随评论的 ReplyReady payload 下发前端驱动反应，
//!   封闭集合见 KNOWN_ANIMS，未知值加载期告警剥离

use serde::Deserialize;

use crate::ipc::{EventKind, StdEvent};

/// 内置副本 = 同一份 meili.map.toml
const EMBEDDED_MEILI_MAP: &str = include_str!("../../resources/creatures/meili.map.toml");

/// 一条映射：某事件（+可选 payload 条件）→ 若干台词变体
#[derive(Debug, Clone, Deserialize)]
pub struct MapEntry {
    /// 事件类型名（EventKind::as_str 封闭集合，加载时校验）
    pub on: String,
    /// payload 条件（可选）：{ "app": ["Code"] } = payload.app 命中其一（字符串相等）
    #[serde(default)]
    pub when: Option<serde_json::Value>,
    /// 重要级：0 重要 / 1 有趣 / 2 背景（默契档只放行 0+1）。默认 1
    #[serde(default = "default_priority")]
    pub priority: u8,
    /// 同类冷却（分钟）。默认 5（已定稿）
    #[serde(default = "default_cooldown_min")]
    pub cooldown_min: u64,
    /// 台词变体（至少一条，随机一条出口）
    pub say: Vec<String>,
    /// 表现层小动作（M1-6 落地）：随 ReplyReady payload 下发前端驱动反应。
    /// 封闭集合（前端 switch 消费）：petted=摸头。加载时未知值告警剥离
    #[serde(default)]
    pub anim: Option<String>,
}

/// anim 封闭集合：map.toml 写其他值 = 告警剥离（防止 content 包打错字静默失效）
const KNOWN_ANIMS: [&str; 1] = ["petted"];

fn default_priority() -> u8 {
    1
}
fn default_cooldown_min() -> u64 {
    5
}

/// 映射表文件根：`[[map]]` 数组
#[derive(Debug, Deserialize)]
struct BehaviorMapFile {
    map: Vec<MapEntry>,
}

/// 校验后的映射表：on 已解析成封闭枚举，非法条目加载期剔除
pub struct BehaviorMap {
    pub entries: Vec<(EventKind, MapEntry)>,
}

impl BehaviorMap {
    /// 加载 <id>.map.toml；缺失/损坏回退内置 meili 副本
    pub fn load(creature_id: &str) -> Self {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources/creatures")
            .join(format!("{creature_id}.map.toml"));
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("映射表缺失 {}，回退内置副本: {e}", path.display());
                EMBEDDED_MEILI_MAP.to_string()
            }
        };
        Self::parse(&text)
    }

    /// 解析 + 校验。格式错误的条目逐条告警跳过（定案：不拖垮整表）。
    /// pub(crate)：测试与 gates 直接构造用
    pub(crate) fn parse(text: &str) -> Self {
        let file: BehaviorMapFile = match toml::from_str(text) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("映射表整体解析失败，回退内置副本: {e}");
                return Self::parse_unchecked_validated(EMBEDDED_MEILI_MAP);
            }
        };
        Self::validate(file.map)
    }

    /// 内置副本兜底路径：副本本身必须合法（开发期测试锁住），失败即 panic
    fn parse_unchecked_validated(text: &str) -> Self {
        let file: BehaviorMapFile = toml::from_str(text).expect("内置映射表必须可解析");
        Self::validate(file.map)
    }

    fn validate(entries: Vec<MapEntry>) -> Self {
        let mut valid = Vec::new();
        for mut e in entries {
            match EventKind::from_name(&e.on) {
                Some(kind) if !e.say.is_empty() => {
                    // anim 封闭集合校验：未知值告警剥离（写错字不应静默丢动作）
                    if let Some(a) = &e.anim {
                        if !KNOWN_ANIMS.contains(&a.as_str()) {
                            tracing::warn!(on = %e.on, anim = %a, "未知 anim，剥离");
                            e.anim = None;
                        }
                    }
                    valid.push((kind, e));
                }
                Some(_) => tracing::warn!(on = %e.on, "映射条目 say 为空，跳过"),
                None => tracing::warn!(on = %e.on, "映射条目 on 非封闭事件名，跳过"),
            }
        }
        tracing::info!(entries = valid.len(), "behavior map loaded");
        Self { entries: valid }
    }
}

impl BehaviorMap {
    /// 事件命中的第一条映射（kind + when 条件 + 条目级冷却已过）
    pub fn find(
        &self,
        event: &StdEvent,
        entry_cooled: impl Fn(usize) -> bool,
    ) -> Option<(usize, &MapEntry)> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(i, (kind, entry))| {
                *kind == event.kind && entry_cooled(*i) && when_matches(&entry.when, &event.payload)
            })
            .map(|(i, (_, entry))| (i, entry))
            .next()
    }
}

/// when 条件：None 恒真；object 的每对 key → payload[key] 命中 value 数组其一
/// （JSON 值相等：字符串/布尔/数字都按类型精确匹配，"true" 字符串不等于 true 布尔）
fn when_matches(when: &Option<serde_json::Value>, payload: &serde_json::Value) -> bool {
    let Some(cond) = when else { return true };
    let Some(obj) = cond.as_object() else {
        return false;
    };
    obj.iter().all(|(key, wanted)| {
        let Some(arr) = wanted.as_array() else {
            return false;
        };
        arr.iter().any(|w| w == &payload[key])
    })
}

/// 台词模板出口：{{field}} 槽位取 payload 字符串字段；缺字段/无非字符串
/// 变体可用时返回 None（该事件本轮沉默）
pub fn fill_template(template: &str, payload: &serde_json::Value) -> Option<String> {
    let mut out = template.to_string();
    // 逐槽位替换：扫描 {{name}}
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let Some(rel_end) = rest[start..].find("}}") else {
            break;
        };
        let name = &rest[start + 2..start + rel_end];
        let value = payload.get(name).and_then(|v| v.as_str())?;
        out = out.replacen(&format!("{{{{{name}}}}}"), value, 1);
        rest = &rest[start + rel_end + 2..];
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{EventSource, PrivacyTier};

    fn evt(kind: EventKind, payload: serde_json::Value) -> StdEvent {
        StdEvent {
            id: "t".into(),
            ts: 0,
            source: EventSource::Test,
            kind,
            payload,
            privacy_tier: PrivacyTier::Minimal,
        }
    }

    #[test]
    fn load_embedded_map_valid() {
        let m = BehaviorMap::parse(EMBEDDED_MEILI_MAP);
        assert!(m.entries.len() >= 10, "种子映射表不应少于 10 条");
    }

    #[test]
    fn invalid_on_entry_skipped_others_kept() {
        let text = r#"
[[map]]
on = "not_an_event"
say = ["不该出现"]

[[map]]
on = "booted"
say = ["合法"]
"#;
        let m = BehaviorMap::parse(text);
        assert_eq!(m.entries.len(), 1);
        assert_eq!(m.entries[0].0, EventKind::Booted);
    }

    #[test]
    fn when_filter_equality_and_array() {
        let m = BehaviorMap::parse(
            r#"[[map]]
on = "lock_state_changed"
when = { locked = [true] }
say = ["x"]
"#,
        );
        let hit = evt(
            EventKind::LockStateChanged,
            serde_json::json!({"locked": true}),
        );
        let miss = evt(
            EventKind::LockStateChanged,
            serde_json::json!({"locked": false}),
        );
        assert!(m.find(&hit, |_| true).is_some());
        assert!(m.find(&miss, |_| true).is_none());
    }

    #[test]
    fn entry_cooldown_excludes_entry() {
        let m = BehaviorMap::parse(
            r#"[[map]]
on = "booted"
say = ["x"]
"#,
        );
        let e = evt(EventKind::Booted, serde_json::json!({}));
        assert!(m.find(&e, |_| false).is_none(), "冷却中的条目不得命中");
        assert!(m.find(&e, |_| true).is_some());
    }

    #[test]
    fn fill_template_slots_and_missing_field() {
        let payload = serde_json::json!({"app": "Code", "n": 42});
        assert_eq!(
            fill_template("换到{{app}}了", &payload).as_deref(),
            Some("换到Code了")
        );
        // 非字符串字段不成槽
        assert!(fill_template("{{n}}个", &payload).is_none());
        // 缺字段不成槽
        assert!(fill_template("{{title}}", &payload).is_none());
        // 无槽原样过
        assert_eq!(
            fill_template(" plain ", &payload).as_deref(),
            Some(" plain ")
        );
    }

    /// 场景模拟器全覆盖回归（2026-11-22 用户实机投诉"暂停音乐没回复"：
    /// 模拟器按钮 payload 在 map.toml 无匹配条目被闸丢）。这里把 main.ts
    /// SCENARIOS 列表逐条镜像进测试——任何"按钮点了没反应"从此是测试失败。
    #[test]
    fn simulator_scenarios_all_covered_by_map() {
        // 与 src/main.ts SCENARIOS 保持一致（改前端按钮时同步改这里）
        let scenarios: Vec<(&str, serde_json::Value)> = vec![
            ("booted", serde_json::json!({"version": "0.0.1"})),
            ("lock_state_changed", serde_json::json!({"locked": true})),
            ("lock_state_changed", serde_json::json!({"locked": false})),
            (
                "foreground_switched",
                serde_json::json!({"app": "Code", "title": "meili.map.toml"}),
            ),
            (
                "foreground_switched",
                serde_json::json!({"app": "chrome", "title": "GitHub - 潮间带"}),
            ),
            (
                "foreground_switched",
                serde_json::json!({"app": "somegame", "title": "Some Game"}),
            ),
            (
                "idle_state_changed",
                serde_json::json!({"idle_seconds": 300, "is_idle": true}),
            ),
            (
                "idle_state_changed",
                serde_json::json!({"idle_seconds": 0, "is_idle": false}),
            ),
            (
                "media_playback_changed",
                serde_json::json!({"app": "cloudmusic", "title": "晴天", "artist": "周杰伦", "state": "playing"}),
            ),
            (
                "media_playback_changed",
                serde_json::json!({"state": "paused"}),
            ),
            (
                "power_state_changed",
                serde_json::json!({"ac_online": true, "charging": true, "battery_percent": 42}),
            ),
            (
                "power_state_changed",
                serde_json::json!({"ac_online": false, "charging": false, "battery_percent": 41}),
            ),
            ("network_changed", serde_json::json!({"connected": false})),
            ("network_changed", serde_json::json!({"connected": true})),
            (
                "mouse_clicked",
                serde_json::json!({"button": "left", "x": 960, "y": 540, "on_pet": false}),
            ),
            (
                "mouse_clicked",
                serde_json::json!({"button": "left", "x": 960, "y": 540, "on_pet": true}),
            ),
        ];
        let m = BehaviorMap::load("meili");
        for (name, payload) in scenarios {
            let kind = EventKind::from_name(name).expect("场景事件名必须封闭");
            let e = evt(kind, payload.clone());
            let found = m
                .entries
                .iter()
                .filter(|(k, entry)| *k == e.kind && when_matches(&entry.when, &e.payload))
                .collect::<Vec<_>>();
            assert!(
                !found.is_empty(),
                "场景 {name} {payload} 无匹配映射条目（点了没反应）"
            );
            let fillable = found.iter().any(|(_, entry)| {
                entry
                    .say
                    .iter()
                    .any(|v| fill_template(v, &payload).is_some())
            });
            assert!(fillable, "场景 {name} {payload} 的匹配条目无可填台词变体");
        }
    }

    #[test]
    fn anim_closed_set_unknown_stripped() {
        let m = BehaviorMap::parse(
            r#"[[map]]
on = "booted"
anim = "petted"
say = ["x"]

[[map]]
on = "booted"
anim = "bogus_anim"
say = ["y"]
"#,
        );
        assert_eq!(m.entries[0].1.anim.as_deref(), Some("petted"));
        assert_eq!(m.entries[1].1.anim, None, "未知 anim 应被剥离");
    }

    #[test]
    fn event_kind_name_roundtrip() {
        for (name, kind) in [
            ("foreground_switched", EventKind::ForegroundSwitched),
            ("media_playback_changed", EventKind::MediaPlaybackChanged),
            ("speak_ended", EventKind::SpeakEnded),
        ] {
            assert_eq!(kind.as_str(), name);
            assert_eq!(EventKind::from_name(name), Some(kind));
        }
        assert!(EventKind::from_name("bogus").is_none());
    }
}
