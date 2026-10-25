//! SQLite cache. Every table is rebuildable from the lazer files.

use rusqlite::{Connection, params};
use std::collections::HashSet;
use std::path::Path;

pub const ANALYZER_VERSION: &str = "1";

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(path)?;
    init(&conn)?;
    Ok(conn)
}

pub fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT);
         CREATE TABLE IF NOT EXISTS files(
            hash TEXT PRIMARY KEY, kind TEXT NOT NULL, size INTEGER NOT NULL, seen_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS beatmaps(
            md5 TEXT PRIMARY KEY, file_hash TEXT, artist TEXT, title TEXT, version TEXT, creator TEXT,
            mode INTEGER, cs REAL, ar REAL, od REAL, hp REAL, object_count INTEGER);
         CREATE TABLE IF NOT EXISTS replays(
            score_id INTEGER PRIMARY KEY AUTOINCREMENT, file_hash TEXT UNIQUE, beatmap_md5 TEXT, player TEXT,
            mods INTEGER, rate REAL, played_at INTEGER, n300 INTEGER, n100 INTEGER, n50 INTEGER, nmiss INTEGER,
            is_mine INTEGER, excluded_reason TEXT);
         CREATE INDEX IF NOT EXISTS replays_md5 ON replays(beatmap_md5);",
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO meta(key,value) VALUES('analyzer_version', ?1)",
        params![ANALYZER_VERSION],
    )?;
    Ok(())
}

/// Stored analyzer_version differs from this build => caller must rebuild.
pub fn needs_rebuild(conn: &Connection) -> bool {
    conn.query_row("SELECT value FROM meta WHERE key='analyzer_version'", [], |r| {
        r.get::<_, String>(0)
    })
    .map(|v| v != ANALYZER_VERSION)
    .unwrap_or(false)
}

pub fn known_hashes(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut st = conn.prepare("SELECT hash FROM files")?;
    let rows = st.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_idempotent_and_versioned() {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        init(&c).unwrap();
        assert!(!needs_rebuild(&c));
        c.execute("UPDATE meta SET value='0' WHERE key='analyzer_version'", []).unwrap();
        assert!(needs_rebuild(&c));
    }
}
