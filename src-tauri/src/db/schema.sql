-- 《潮间带》存档 schema（唯一来源，改动需人审——工程规范 §5.4）
-- 八张表对应架构 §3.8；迁移机制用 PRAGMA user_version（见 mod.rs）。
-- 约定：时间戳统一 INTEGER 秒级 Unix epoch；id 用 TEXT UUID（Rust 侧生成）。

-- 1. 生物档案 + 当前状态（每只生物一行）
CREATE TABLE IF NOT EXISTS creatures (
    id            TEXT PRIMARY KEY,
    petpack_id    TEXT NOT NULL,              -- pets/<id>，如 "meili"
    name          TEXT NOT NULL,
    adopted_at    INTEGER NOT NULL,           -- 入所时间
    stage         TEXT NOT NULL               -- 封闭集：sheltered(在所) / awake(活过来) / released(已告别)
                  CHECK (stage IN ('sheltered', 'awake', 'released')),
    released_at   INTEGER,                    -- 告别时间，NULL = 未告别
    revisit_style TEXT                        -- 性格参数：恋家/标准/不羁（内容包 pet.toml 冗余存一份，防包丢失）
);

-- 2. 信任度/生命力 + 变更流水（append-only，当前值 = 该生物该指标最后一行 value_after）
CREATE TABLE IF NOT EXISTS stats (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    creature_id TEXT NOT NULL REFERENCES creatures(id),
    stat_type   TEXT NOT NULL CHECK (stat_type IN ('trust', 'vitality')),
    delta       REAL NOT NULL,                -- 本次变更量（可负）
    value_after REAL NOT NULL,                -- 变更后绝对值
    reason      TEXT NOT NULL,                -- 变更原因（事件 id 或照顾动作，便于审计）
    ts          INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_stats_latest ON stats(creature_id, stat_type, id);

-- 3. 目标账本（用户向生物提过的目标/想法）
CREATE TABLE IF NOT EXISTS goals (
    id          TEXT PRIMARY KEY,
    creature_id TEXT REFERENCES creatures(id),
    title       TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'open'
                CHECK (status IN ('open', 'done', 'dropped')),
    source      TEXT NOT NULL,                -- 哪次对话/事件里提到的（审计用）
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- 4. 情境记忆（截屏感知/对话沉淀的脱敏摘要）
CREATE TABLE IF NOT EXISTS context_memories (
    id           TEXT PRIMARY KEY,
    creature_id  TEXT REFERENCES creatures(id),
    kind         TEXT NOT NULL,               -- 封闭集：screen_observation / chat_digest
    summary      TEXT NOT NULL,               -- 已脱敏摘要，禁止存原始屏幕内容
    privacy_tier TEXT NOT NULL,               -- 三档隐私：minimal / balanced / companion
    created_at   INTEGER NOT NULL
);

-- 5. 信件/明信片（含告别后与回访信件）
CREATE TABLE IF NOT EXISTS letters (
    id           TEXT PRIMARY KEY,
    creature_id  TEXT NOT NULL REFERENCES creatures(id),
    kind         TEXT NOT NULL CHECK (kind IN ('farewell', 'postcard', 'reply')),
    content      TEXT NOT NULL,
    scheduled_at INTEGER NOT NULL,            -- 计划送达（潮水速度延迟是有意设计）
    delivered_at INTEGER,                     -- NULL = 未送达
    read_at      INTEGER
);
CREATE INDEX IF NOT EXISTS idx_letters_queue ON letters(delivered_at, scheduled_at);

-- 6. 图鉴/校友录（每只生物解锁一行，告别时补全）
CREATE TABLE IF NOT EXISTS collection (
    creature_id TEXT PRIMARY KEY REFERENCES creatures(id),
    petpack_id  TEXT NOT NULL,
    unlocked_at INTEGER NOT NULL,
    released_at INTEGER,                      -- 告别后成为"校友"
    best_snapshot TEXT                        -- 告别时数值/天数快照（JSON）
);

-- 7. 隐私日志（可查可删：用户能看到出过哪些事件、删得掉）
CREATE TABLE IF NOT EXISTS event_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ts           INTEGER NOT NULL,
    kind         TEXT NOT NULL,               -- 事件类型（脱敏后）
    summary      TEXT NOT NULL,               -- 人类可读描述，禁止敏感内容
    privacy_tier TEXT NOT NULL,
    sent_offdevice INTEGER NOT NULL DEFAULT 0 -- 是否出过机（BYOK 对话等）
);
CREATE INDEX IF NOT EXISTS idx_event_log_ts ON event_log(ts);

-- 8. 设置（隐私档、API key 引用等；key 的密文存 Credential Manager，这里只存引用名）
CREATE TABLE IF NOT EXISTS settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
