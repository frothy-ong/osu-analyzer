//! Walk the lazer files/ folder (read-only), identify files by CONTENT,
//! and record replays and beatmaps in SQLite. Known file hashes are skipped.

use crate::{config::Config, db, replay};
use md5::{Digest, Md5};
use rayon::prelude::*;
use rusqlite::{Connection, params};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

enum Parsed {
    Other,
    BadReplay,
    BadBeatmap,
    Replay(replay::ReplayHeader),
    Beatmap(BeatmapRow),
}

struct BeatmapRow {
    md5: String,
    artist: String,
    title: String,
    version: String,
    creator: String,
    mode: i64,
    cs: f64,
    ar: f64,
    od: f64,
    hp: f64,
    object_count: i64,
}

struct Item {
    hash: String,
    size: u64,
    parsed: Parsed,
}

/// Collect every file under files/ (files are stored as files/a/ab/<sha256>).
fn list_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() {
                out.push(e.path());
            }
        }
    }
    out
}

fn is_beatmap_prefix(p: &[u8]) -> bool {
    let p = p.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(p);
    p.starts_with(b"osu file format v")
}

/// Read-only open; classify by first bytes; read the rest only if needed.
fn process(path: &Path) -> Option<Item> {
    let hash = path.file_name()?.to_string_lossy().into_owned();
    let mut f = std::fs::File::open(path).ok()?; // read-only
    let size = f.metadata().ok()?.len();
    let mut prefix = [0u8; 32];
    let n = f.read(&mut prefix).ok()?;
    let prefix = &prefix[..n];

    let parsed = if replay::looks_like_replay(prefix) {
        let mut buf = prefix.to_vec();
        if f.read_to_end(&mut buf).is_err() {
            Parsed::BadReplay
        } else {
            match replay::parse_header(&buf) {
                Some(h) => Parsed::Replay(h),
                None => Parsed::BadReplay,
            }
        }
    } else if is_beatmap_prefix(prefix) {
        let mut buf = prefix.to_vec();
        if f.read_to_end(&mut buf).is_err() {
            Parsed::BadBeatmap
        } else {
            match parse_beatmap(&buf) {
                Some(b) => Parsed::Beatmap(b),
                None => Parsed::BadBeatmap,
            }
        }
    } else {
        Parsed::Other
    };
    Some(Item { hash, size, parsed })
}

fn parse_beatmap(bytes: &[u8]) -> Option<BeatmapRow> {
    let bm = rosu_map::Beatmap::from_bytes(bytes).ok()?;
    let mut h = Md5::new();
    h.update(bytes);
    let md5: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Some(BeatmapRow {
        md5,
        artist: bm.artist,
        title: bm.title,
        version: bm.version,
        creator: bm.creator,
        mode: bm.mode as i64,
        cs: bm.circle_size as f64,
        ar: bm.approach_rate as f64,
        od: bm.overall_difficulty as f64,
        hp: bm.hp_drain_rate as f64,
        object_count: bm.hit_objects.len() as i64,
    })
}

pub struct ScanResult {
    pub lazer_dir: PathBuf,
    pub files_seen: usize,
    pub files_parsed_now: usize,
    pub seconds: f64,
}

pub fn scan(conn: &mut Connection, lazer: &Path, cfg: &Config) -> Result<ScanResult, String> {
    let t0 = Instant::now();
    let files_dir = lazer.join("files");
    if !files_dir.is_dir() {
        return Err(format!("no files/ folder in {lazer:?}"));
    }
    if db::needs_rebuild(conn) {
        conn.execute_batch("DELETE FROM files; DELETE FROM beatmaps; DELETE FROM replays;")
            .map_err(|e| e.to_string())?;
        db::init(conn).map_err(|e| e.to_string())?;
    }
    let known = db::known_hashes(conn).map_err(|e| e.to_string())?;
    let all = list_files(&files_dir);
    let files_seen = all.len();
    let todo: Vec<&PathBuf> = all
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| !known.contains(n.to_string_lossy().as_ref())))
        .collect();
    let items: Vec<Item> = todo.par_iter().filter_map(|p| process(p)).collect();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for it in &items {
        let kind = match &it.parsed {
            Parsed::Other => "other",
            Parsed::BadReplay => "bad_replay",
            Parsed::BadBeatmap => "bad_beatmap",
            Parsed::Replay(_) => "replay",
            Parsed::Beatmap(_) => "beatmap",
        };
        tx.execute(
            "INSERT OR IGNORE INTO files(hash,kind,size,seen_at) VALUES(?1,?2,?3,?4)",
            params![it.hash, kind, it.size as i64, now],
        )
        .map_err(|e| e.to_string())?;
        match &it.parsed {
            Parsed::Beatmap(b) => {
                tx.execute(
                    "INSERT OR IGNORE INTO beatmaps VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                    params![b.md5, it.hash, b.artist, b.title, b.version, b.creator, b.mode, b.cs, b.ar, b.od, b.hp, b.object_count],
                )
                .map_err(|e| e.to_string())?;
            }
            Parsed::Replay(h) => {
                let reason = replay::exclusion_reason(h, &cfg.player_name);
                let is_mine = !cfg.player_name.is_empty() && h.player.eq_ignore_ascii_case(&cfg.player_name);
                tx.execute(
                    "INSERT OR IGNORE INTO replays(file_hash,beatmap_md5,player,mods,rate,played_at,n300,n100,n50,nmiss,is_mine,excluded_reason)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                    params![it.hash, h.beatmap_md5, h.player, h.mods, replay::rate_from_mods(h.mods), h.played_at,
                            h.n300, h.n100, h.n50, h.nmiss, is_mine as i64, reason],
                )
                .map_err(|e| e.to_string())?;
            }
            _ => {}
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(ScanResult {
        lazer_dir: lazer.to_path_buf(),
        files_seen,
        files_parsed_now: items.len(),
        seconds: t0.elapsed().as_secs_f64(),
    })
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap_or(-1)
}

pub fn print_summary(conn: &Connection, r: &ScanResult) {
    println!("Lazer folder:            {}", r.lazer_dir.display());
    println!("Files in files/:         {}", r.files_seen);
    println!("Files parsed this run:   {}", r.files_parsed_now);
    println!("Replays found:           {}", count(conn, "SELECT COUNT(*) FROM replays"));
    println!("Beatmaps found:          {}", count(conn, "SELECT COUNT(*) FROM beatmaps"));
    println!(
        "Replays, no beatmap:     {} (all) / {} (kept, i.e. not excluded)",
        count(conn, "SELECT COUNT(*) FROM replays r WHERE NOT EXISTS (SELECT 1 FROM beatmaps b WHERE b.md5=r.beatmap_md5)"),
        count(conn, "SELECT COUNT(*) FROM replays r WHERE excluded_reason IS NULL AND NOT EXISTS (SELECT 1 FROM beatmaps b WHERE b.md5=r.beatmap_md5)"),
    );
    println!("Replays kept:            {}", count(conn, "SELECT COUNT(*) FROM replays WHERE excluded_reason IS NULL"));
    println!("Replays excluded:        {}", count(conn, "SELECT COUNT(*) FROM replays WHERE excluded_reason IS NOT NULL"));
    if let Ok(mut st) = conn.prepare("SELECT excluded_reason, COUNT(*) FROM replays WHERE excluded_reason IS NOT NULL GROUP BY 1 ORDER BY 2 DESC") {
        if let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
            for row in rows.flatten() {
                println!("  excluded - {}: {}", row.0, row.1);
            }
        }
    }
    if let Ok(mut st) = conn.prepare("SELECT player, COUNT(*) FROM replays GROUP BY 1 ORDER BY 2 DESC LIMIT 5") {
        if let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
            for row in rows.flatten() {
                println!("  player '{}': {} replays", row.0, row.1);
            }
        }
    }
    println!("Unreadable replay files: {}", count(conn, "SELECT COUNT(*) FROM files WHERE kind='bad_replay'"));
    println!("Unreadable beatmaps:     {}", count(conn, "SELECT COUNT(*) FROM files WHERE kind='bad_beatmap'"));
    println!("Scan time:               {:.2} s", r.seconds);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::tests::build;

    const OSU: &str = "osu file format v14\n\n[General]\nMode: 0\n\n[Metadata]\nTitle:T\nArtist:A\nCreator:C\nVersion:V\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:4\nOverallDifficulty:8\nApproachRate:9\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0\n100,100,1500,1,0\n";

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("osu-an-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn scan_matches_and_is_incremental() {
        let lazer = tmp("scan");
        let sub = lazer.join("files/a/ab");
        std::fs::create_dir_all(&sub).unwrap();

        std::fs::write(sub.join("beatmap_hash_1"), OSU).unwrap();
        let mut h = Md5::new();
        h.update(OSU.as_bytes());
        let md5: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();

        std::fs::write(sub.join("replay_mine"), build(0, &md5, "fr01hy", 0, [2, 0, 0, 0], 0)).unwrap();
        std::fs::write(sub.join("replay_other"), build(0, &md5, "someone", 0, [2, 0, 0, 0], 0)).unwrap();
        std::fs::write(sub.join("replay_nomap"), build(0, &"0".repeat(32), "fr01hy", 0, [2, 0, 0, 0], 0)).unwrap();
        std::fs::write(sub.join("audio"), b"ID3\x03\x00\x00junk").unwrap();

        let cfg = Config { lazer_path: String::new(), player_name: "fr01hy".into() };
        let mut conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();

        let r1 = scan(&mut conn, &lazer, &cfg).unwrap();
        assert_eq!(r1.files_seen, 5);
        assert_eq!(r1.files_parsed_now, 5);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM replays"), 3);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM beatmaps"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM replays WHERE excluded_reason='not own replay'"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM replays r WHERE NOT EXISTS (SELECT 1 FROM beatmaps b WHERE b.md5=r.beatmap_md5)"), 1);
        assert_eq!(count(&conn, "SELECT object_count FROM beatmaps"), 2);

        let r2 = scan(&mut conn, &lazer, &cfg).unwrap();
        assert_eq!(r2.files_parsed_now, 0);
        let _ = std::fs::remove_dir_all(&lazer);
    }

    #[test]
    fn missing_files_dir_is_error() {
        let d = tmp("nofiles");
        let mut conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        assert!(scan(&mut conn, &d, &Config::default()).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn beatmap_prefix_detection() {
        assert!(is_beatmap_prefix(b"osu file format v9\n"));
        assert!(is_beatmap_prefix(b"\xEF\xBB\xBFosu file format v14"));
        assert!(!is_beatmap_prefix(b"RIFF"));
    }
}
