//! 数据层实现。schema 唯一来源是 `schema.sql`；SQL 只许出现在本文件与 `repo.rs`。
//!
//! 迁移机制：PRAGMA user_version。当前 v1 = 应用全部 schema.sql。
//! 向后兼容一个版本（工程规范 §9）：未来加列/加表用增量迁移，不许改旧语句。

mod repo;

use std::path::Path;

use rusqlite::Connection;

/// 当前 schema 版本。
const SCHEMA_VERSION: i32 = 1;

/// 打开（必要时创建）存档库并跑到最新版本。
///
/// - `path`：存档文件路径（父目录须已存在，调用方负责建目录）。
/// - WAL 模式：事件流水与读路径并发（UI 读档案 + 采集写日志）。
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate(&conn)?;
    Ok(conn)
}

/// 内存库（测试用）。
#[cfg(test)]
pub fn open_in_memory() -> rusqlite::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version < 1 {
        conn.execute_batch(include_str!("schema.sql"))?;
    }
    // 未来 v2+：else if version < 2 { ... }，逐版本递增
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_tables_created() {
        let conn = open_in_memory().expect("open in-memory db");
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .expect("prepare");
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect();
        assert_eq!(
            tables,
            vec![
                "collection",
                "context_memories",
                "creatures",
                "event_log",
                "goals",
                "letters",
                "settings",
                "stats",
            ],
            "八张表必须与架构 §3.8 一一对应",
        );
        // 迁移幂等：再跑一次不炸
        migrate(&conn).expect("re-migrate");
    }

    #[test]
    fn stats_append_only_latest_wins() {
        let conn = open_in_memory().expect("open in-memory db");
        conn.execute(
            "INSERT INTO creatures (id, petpack_id, name, adopted_at, stage) VALUES ('c1','meili','美丽',0,'sheltered')",
            [],
        )
        .expect("insert creature");
        for (delta, after) in [(1.0, 1.0), (2.0, 3.0)] {
            conn.execute(
                "INSERT INTO stats (creature_id, stat_type, delta, value_after, reason, ts) VALUES ('c1','trust',?1,?2,'test',0)",
                rusqlite::params![delta, after],
            )
            .expect("insert stat");
        }
        let latest: f64 = conn
            .query_row(
                "SELECT value_after FROM stats WHERE creature_id='c1' AND stat_type='trust' ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("query latest");
        assert!((latest - 3.0).abs() < f64::EPSILON);
    }
}
