//! SQLite cache. Every table is rebuildable from the lazer files.

use rusqlite::{Connection, params};
use std::collections::HashSet;
use std::path::Path;

/// Bump whenever any algorithm (scan or simulator) changes. A mismatch wipes
/// the cache; the next scan and sim recompute everything.
pub const ANALYZER_VERSION: &str = "4";

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(path)?;
    init(&conn)?;
    Ok(conn)
}

/// Create tables. Records the version only if none is stored yet, so an old
/// cache is still detected by `needs_rebuild`.
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
         CREATE INDEX IF NOT EXISTS replays_md5 ON replays(beatmap_md5);
         CREATE TABLE IF NOT EXISTS hits(
            score_id INTEGER NOT NULL, object_index INTEGER NOT NULL, time_ms REAL, offset_ms REAL,
            cursor_dx REAL, cursor_dy REAL, key TEXT, judgement INTEGER NOT NULL,
            local_density REAL, spacing REAL, obj_x REAL, obj_y REAL,
            PRIMARY KEY(score_id, object_index)) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS sim_check(
            score_id INTEGER PRIMARY KEY, analyzer_version TEXT NOT NULL,
            sim_n300 INTEGER, sim_n100 INTEGER, sim_n50 INTEGER, sim_nmiss INTEGER,
            matches_header INTEGER NOT NULL, map_type TEXT, expected_objects INTEGER, rate REAL,
            status TEXT NOT NULL, match_incl INTEGER, match_incl_floor INTEGER, match_excl_floor INTEGER,
            miss_diff INTEGER);",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO meta(key,value) VALUES('analyzer_version', ?1)",
        params![ANALYZER_VERSION],
    )?;
    Ok(())
}

/// Stored analyzer_version differs from this build => caller must wipe.
pub fn needs_rebuild(conn: &Connection) -> bool {
    conn.query_row("SELECT value FROM meta WHERE key='analyzer_version'", [], |r| {
        r.get::<_, String>(0)
    })
    .map(|v| v != ANALYZER_VERSION)
    .unwrap_or(false)
}

/// Drop and recreate all cached tables (an old cache may hold tables with an
/// older column layout), then record the current version.
pub fn wipe(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS hits; DROP TABLE IF EXISTS sim_check; DROP TABLE IF EXISTS replays;
         DROP TABLE IF EXISTS beatmaps; DROP TABLE IF EXISTS files;",
    )?;
    init(conn)?;
    conn.execute(
        "INSERT OR REPLACE INTO meta(key,value) VALUES('analyzer_version', ?1)",
        params![ANALYZER_VERSION],
    )?;
    Ok(())
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
        init(&c).unwrap(); // must not hide the stale version
        assert!(needs_rebuild(&c));
        c.execute_batch("DROP TABLE hits; CREATE TABLE hits(a,b);").unwrap(); // old layout
        wipe(&c).unwrap();
        assert!(!needs_rebuild(&c));
        c.execute("INSERT INTO hits VALUES(1,2,3,4,5,6,'M1',300,1,1,1,1)", []).unwrap();
    }
}
