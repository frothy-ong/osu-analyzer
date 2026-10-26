//! M03 data layer: PP recompute (one pinned rosu-pp), load plays and hits into
//! RAM, global filter, small aggregations. Read-only on the lazer folder.

use crate::{db, scan, sim};
use rayon::prelude::*;
use rusqlite::{params, Connection};
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The ONE PP library used for every number shown. Keep equal to Cargo.toml.
pub const PP_LIB: &str = "rosu-pp 3.0.0";

#[allow(dead_code)] // fields used by M04+
#[derive(Clone, Debug)]
pub struct Play {
    pub score_id: i64,
    pub played_at: i64,
    pub mods: i32,
    pub n300: u32,
    pub n100: u32,
    pub n50: u32,
    pub nmiss: u32,
    pub acc: f64,
    pub pp: Option<f64>,
    pub stars: Option<f64>,
    pub md5: String,
    pub title: String,
    pub passed: bool,
    pub hours: f64,
}

/// One simulated hit, compact. offset_ms / spacing are NaN when unknown.
#[allow(dead_code)] // fields used by M04+
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub score_id: u32,
    pub object_index: u32,
    pub time_ms: f32,
    pub offset_ms: f32,
    pub dx: f32,
    pub dy: f32,
    pub obj_x: f32,
    pub obj_y: f32,
    pub density: f32,
    pub spacing: f32,
    /// 0 none, 1 K1, 2 K2, 3 M1, 4 M2
    pub key: u8,
    /// 300, 100, 50 or 0 (miss)
    pub judgement: u16,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct PpStats {
    pub new: usize,
    pub failed: usize,
    pub seconds: f64,
}

pub struct Dataset {
    pub plays: Vec<Play>,
    pub hits: Vec<Hit>,
    pub load_seconds: f64,
    pub pp: PpStats,
}

pub fn accuracy(n300: u32, n100: u32, n50: u32, nmiss: u32) -> f64 {
    let tot = n300 + n100 + n50 + nmiss;
    if tot == 0 {
        return 0.0;
    }
    100.0 * (300 * n300 + 100 * n100 + 50 * n50) as f64 / (300 * tot) as f64
}

// ---------------------------------------------------------------------------
// PP
// ---------------------------------------------------------------------------

fn skip_str(b: &[u8], p: usize) -> Option<usize> {
    match *b.get(p)? {
        0 => Some(p + 1),
        0x0b => {
            let (mut len, mut shift, mut q) = (0usize, 0, p + 1);
            loop {
                let by = *b.get(q)?;
                q += 1;
                len |= ((by & 0x7f) as usize) << shift;
                if by & 0x80 == 0 {
                    break;
                }
                shift += 7;
                if shift > 28 {
                    return None;
                }
            }
            Some(q + len)
        }
        _ => None,
    }
}

/// Max combo from the replay header (mode, version, 3 strings, 6 u16, score i32, combo u16).
pub fn header_max_combo(b: &[u8]) -> Option<u32> {
    let mut p = 5usize;
    for _ in 0..3 {
        p = skip_str(b, p)?;
    }
    p += 12 + 4;
    Some(u16::from_le_bytes([*b.get(p)?, *b.get(p + 1)?]) as u32)
}

fn read_combo(path: &Path) -> Option<u32> {
    let mut f = std::fs::File::open(path).ok()?; // read-only
    let mut buf = [0u8; 512];
    let n = f.read(&mut buf).ok()?;
    header_max_combo(&buf[..n])
}

struct PpJob {
    hash: String,
    map_hash: String,
    mods: i32,
    rate: f64,
    n: [u32; 4],
}

fn calc_pp(paths: &HashMap<String, PathBuf>, j: &PpJob) -> Result<(f64, f64), String> {
    let mp = paths.get(&j.map_hash).ok_or("beatmap file missing")?;
    let bytes = std::fs::read(mp).map_err(|e| e.to_string())?; // read-only
    let map = rosu_pp::Beatmap::from_bytes(&bytes).map_err(|e| e.to_string())?;
    let mut p = rosu_pp::Performance::new(&map)
        .mods(j.mods as u32)
        .lazer(false)
        .clock_rate(j.rate)
        .n300(j.n[0])
        .n100(j.n[1])
        .n50(j.n[2])
        .misses(j.n[3]);
    if let Some(c) = paths.get(&j.hash).and_then(|p| read_combo(p)) {
        p = p.combo(c);
    }
    let a = p.calculate();
    Ok((a.pp(), a.stars()))
}

/// Compute PP and stars for every kept replay not yet done with this PP_LIB
/// and analyzer_version. Results are cached in play_pp (keyed by replay file hash).
pub fn ensure_pp(conn: &mut Connection, lazer: &Path) -> Result<PpStats, String> {
    let t0 = Instant::now();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS play_pp(file_hash TEXT PRIMARY KEY, pp REAL, stars REAL,
            pp_lib TEXT NOT NULL, analyzer_version TEXT NOT NULL);",
    )
    .map_err(|e| e.to_string())?;
    let jobs: Vec<PpJob> = {
        let mut st = conn
            .prepare(
                "SELECT r.file_hash, b.file_hash, r.mods, COALESCE(r.rate,1.0), r.n300, r.n100, r.n50, r.nmiss
                 FROM replays r JOIN beatmaps b ON b.md5 = r.beatmap_md5
                 WHERE r.excluded_reason IS NULL
                   AND r.file_hash NOT IN (SELECT file_hash FROM play_pp WHERE pp_lib=?1 AND analyzer_version=?2)",
            )
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map(params![PP_LIB, db::ANALYZER_VERSION], |r| {
                Ok(PpJob {
                    hash: r.get(0)?,
                    map_hash: r.get(1)?,
                    mods: r.get(2)?,
                    rate: r.get(3)?,
                    n: [r.get::<_, i64>(4)? as u32, r.get::<_, i64>(5)? as u32, r.get::<_, i64>(6)? as u32, r.get::<_, i64>(7)? as u32],
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
    };
    if jobs.is_empty() {
        return Ok(PpStats { new: 0, failed: 0, seconds: t0.elapsed().as_secs_f64() });
    }
    let paths: HashMap<String, PathBuf> = scan::list_files(&lazer.join("files"))
        .into_iter()
        .filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), p)))
        .collect();
    let results: Vec<(&PpJob, Result<(f64, f64), String>)> =
        jobs.par_iter().map(|j| (j, calc_pp(&paths, j))).collect();
    let (mut ok, mut bad) = (0usize, 0usize);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    {
        let mut ins = tx
            .prepare_cached("INSERT OR REPLACE INTO play_pp(file_hash,pp,stars,pp_lib,analyzer_version) VALUES(?1,?2,?3,?4,?5)")
            .map_err(|e| e.to_string())?;
        for (j, res) in results {
            let (pp, stars) = match res {
                Ok((pp, st)) => {
                    ok += 1;
                    (Some(pp), Some(st))
                }
                Err(_) => {
                    bad += 1;
                    (None, None)
                }
            };
            ins.execute(params![j.hash, pp, stars, PP_LIB, db::ANALYZER_VERSION]).map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(PpStats { new: ok, failed: bad, seconds: t0.elapsed().as_secs_f64() })
}

// ---------------------------------------------------------------------------
// Load
// ---------------------------------------------------------------------------

pub fn load(conn: &Connection, pp: PpStats) -> Result<Dataset, String> {
    let t0 = Instant::now();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS play_pp(file_hash TEXT PRIMARY KEY, pp REAL, stars REAL,
            pp_lib TEXT NOT NULL, analyzer_version TEXT NOT NULL);",
    )
    .map_err(|e| e.to_string())?;

    let mut plays: Vec<Play> = {
        let mut st = conn
            .prepare(
                "SELECT r.score_id, r.played_at, r.mods, r.n300, r.n100, r.n50, r.nmiss, r.beatmap_md5,
                        b.artist, b.title, b.version, p.pp, p.stars,
                        s.sim_n300+s.sim_n100+s.sim_n50+s.sim_nmiss, s.expected_objects
                 FROM replays r
                 LEFT JOIN beatmaps b ON b.md5 = r.beatmap_md5
                 LEFT JOIN play_pp p ON p.file_hash = r.file_hash AND p.analyzer_version = ?1 AND p.pp_lib = ?2
                 LEFT JOIN sim_check s ON s.score_id = r.score_id AND s.analyzer_version = ?1 AND s.status = 'ok'
                 WHERE r.excluded_reason IS NULL
                 ORDER BY r.played_at, r.score_id",
            )
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map(params![db::ANALYZER_VERSION, PP_LIB], |r| {
                let (n300, n100, n50, nmiss) = (r.get::<_, i64>(3)? as u32, r.get::<_, i64>(4)? as u32, r.get::<_, i64>(5)? as u32, r.get::<_, i64>(6)? as u32);
                let artist: Option<String> = r.get(8)?;
                let title: Option<String> = r.get(9)?;
                let version: Option<String> = r.get(10)?;
                let judged: Option<i64> = r.get(13)?;
                let expected: Option<i64> = r.get(14)?;
                let passed = match (judged, expected) {
                    (Some(j), Some(e)) => j >= e,
                    _ => true,
                };
                let pp: Option<f64> = r.get(11)?;
                Ok(Play {
                    score_id: r.get(0)?,
                    played_at: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    mods: r.get(2)?,
                    n300,
                    n100,
                    n50,
                    nmiss,
                    acc: accuracy(n300, n100, n50, nmiss),
                    pp: if passed { pp } else { None },
                    stars: r.get(12)?,
                    md5: r.get(7)?,
                    title: match (artist, title) {
                        (Some(a), Some(t)) => format!("{a} - {t} [{}]", version.unwrap_or_default()),
                        _ => "(beatmap file missing)".to_string(),
                    },
                    passed,
                    hours: 0.0,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
    };

    let mut hits: Vec<Hit> = Vec::new();
    let mut last_time: HashMap<i64, f64> = HashMap::new();
    {
        let mut st = conn
            .prepare("SELECT score_id,object_index,time_ms,offset_ms,cursor_dx,cursor_dy,key,judgement,local_density,spacing,obj_x,obj_y FROM hits")
            .map_err(|e| e.to_string())?;
        let mut rows = st.query([]).map_err(|e| e.to_string())?;
        while let Some(r) = rows.next().map_err(|e| e.to_string())? {
            let f = |i: usize| -> Result<f32, rusqlite::Error> { Ok(r.get::<_, Option<f64>>(i)?.map(|v| v as f32).unwrap_or(f32::NAN)) };
            let score_id: i64 = r.get(0).map_err(|e| e.to_string())?;
            let time = r.get::<_, Option<f64>>(2).map_err(|e| e.to_string())?.unwrap_or(0.0);
            let e = last_time.entry(score_id).or_insert(0.0);
            if time > *e {
                *e = time;
            }
            let key = match r.get::<_, Option<String>>(6).map_err(|e| e.to_string())?.as_deref() {
                Some("K1") => 1,
                Some("K2") => 2,
                Some("M1") => 3,
                Some("M2") => 4,
                _ => 0,
            };
            hits.push(Hit {
                score_id: score_id as u32,
                object_index: r.get::<_, i64>(1).map_err(|e| e.to_string())? as u32,
                time_ms: time as f32,
                offset_ms: f(3).map_err(|e| e.to_string())?,
                dx: f(4).map_err(|e| e.to_string())?,
                dy: f(5).map_err(|e| e.to_string())?,
                obj_x: f(10).map_err(|e| e.to_string())?,
                obj_y: f(11).map_err(|e| e.to_string())?,
                density: f(8).map_err(|e| e.to_string())?,
                spacing: f(9).map_err(|e| e.to_string())?,
                key,
                judgement: r.get::<_, i64>(7).map_err(|e| e.to_string())? as u16,
            });
        }
    }
    for p in plays.iter_mut() {
        p.hours = last_time.get(&p.score_id).copied().unwrap_or(0.0) / 3_600_000.0;
    }
    Ok(Dataset { plays, hits, load_seconds: t0.elapsed().as_secs_f64(), pp })
}

/// Startup path: config, cache check, PP for new plays, load into RAM.
pub fn load_all() -> Result<Dataset, String> {
    use crate::config;
    let cfg = config::load_or_create(&config::config_path())?;
    let mut conn = db::open(&config::cache_db_path()).map_err(|e| e.to_string())?;
    if db::needs_rebuild(&conn) {
        return Err("The cache is from an older analyzer version. Run `cargo run --release -- sim` once, then start again.".into());
    }
    let pp = match config::resolve_lazer_path(&cfg, &config::default_candidates()) {
        Some(l) => ensure_pp(&mut conn, &l)?,
        None => PpStats::default(),
    };
    load(&conn, pp)
}

// ---------------------------------------------------------------------------
// Filter
// ---------------------------------------------------------------------------

/// Mod groups offered in the filter bar (legacy bits). DT/NC share a group.
pub const MOD_GROUPS: [(&str, i32); 7] = [
    ("NF", 1),
    ("EZ", 2),
    ("HD", 8),
    ("HR", 16),
    ("DT/NC", 64 | 512),
    ("HT", 256),
    ("FL", 1024),
];

pub const STAR_CAP: f64 = 20.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Range {
    All,
    Days7,
    Days30,
    Days90,
    Days365,
}

impl Range {
    pub const ALL: [Range; 5] = [Range::All, Range::Days7, Range::Days30, Range::Days90, Range::Days365];
    pub fn label(self) -> &'static str {
        match self {
            Range::All => "All time",
            Range::Days7 => "Last 7 days",
            Range::Days30 => "Last 30 days",
            Range::Days90 => "Last 90 days",
            Range::Days365 => "Last 365 days",
        }
    }
    pub fn days(self) -> Option<i64> {
        match self {
            Range::All => None,
            Range::Days7 => Some(7),
            Range::Days30 => Some(30),
            Range::Days90 => Some(90),
            Range::Days365 => Some(365),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Filter {
    pub mods_sel: [bool; 7],
    /// With "exact": plays may not carry any listed mod that is not ticked
    /// (so nothing ticked + exact = no-mod plays).
    pub mods_exact: bool,
    pub range: Range,
    pub star_min: f64,
    pub star_max: f64,
    pub best_only: bool,
    pub include_fails: bool,
}

impl Default for Filter {
    fn default() -> Self {
        Filter {
            mods_sel: [false; 7],
            mods_exact: false,
            range: Range::All,
            star_min: 0.0,
            star_max: STAR_CAP,
            best_only: false,
            include_fails: true,
        }
    }
}

impl Filter {
    /// Indices into `plays` that pass, in the original (time) order.
    pub fn apply(&self, plays: &[Play], now: i64) -> Vec<usize> {
        let cutoff = self.range.days().map(|d| now - d * 86_400);
        let star_on = self.star_min > 0.0 || self.star_max < STAR_CAP;
        let mut out: Vec<usize> = Vec::new();
        'p: for (i, p) in plays.iter().enumerate() {
            if cutoff.is_some_and(|c| p.played_at < c) || (!self.include_fails && !p.passed) {
                continue;
            }
            if star_on {
                match p.stars {
                    Some(s) if s >= self.star_min && s <= self.star_max => {}
                    _ => continue,
                }
            }
            for (k, g) in MOD_GROUPS.iter().enumerate() {
                let has = p.mods & g.1 != 0;
                if (self.mods_sel[k] && !has) || (self.mods_exact && !self.mods_sel[k] && has) {
                    continue 'p;
                }
            }
            out.push(i);
        }
        if self.best_only {
            let mut best: HashMap<&str, (usize, f64)> = HashMap::new();
            for &i in &out {
                let pp = plays[i].pp.unwrap_or(-1.0);
                let e = best.entry(plays[i].md5.as_str()).or_insert((i, pp));
                if pp > e.1 {
                    *e = (i, pp);
                }
            }
            out = best.values().map(|b| b.0).collect();
            out.sort_unstable();
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Aggregations and dates
// ---------------------------------------------------------------------------

/// Days since 1970-01-01 -> (year, month 1-12, day).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn date_label(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn month_key(unix: i64) -> i64 {
    let (y, m, _) = civil_from_days(unix.div_euclid(86_400));
    y * 12 + (m as i64 - 1)
}

pub fn month_label(key: i64) -> String {
    format!("{:04}-{:02}", key.div_euclid(12), key.rem_euclid(12) + 1)
}

pub fn plays_per_month(plays: &[Play], idx: &[usize]) -> BTreeMap<i64, u32> {
    let mut m = BTreeMap::new();
    for &i in idx {
        *m.entry(month_key(plays[i].played_at)).or_insert(0) += 1;
    }
    m
}

/// Mod combination -> the plays using it, biggest first.
pub fn mod_usage(plays: &[Play], idx: &[usize]) -> Vec<(String, Vec<usize>)> {
    let mut m: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for &i in idx {
        m.entry(sim::mods_to_string(plays[i].mods)).or_default().push(i);
    }
    let mut v: Vec<_> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(id: i64, md5: &str, mods: i32, pp: Option<f64>, stars: Option<f64>, passed: bool, at: i64) -> Play {
        Play {
            score_id: id, played_at: at, mods, n300: 1, n100: 0, n50: 0, nmiss: 0, acc: 100.0, pp, stars,
            md5: md5.into(), title: "t".into(), passed, hours: 0.0,
        }
    }

    #[test]
    fn dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(date_label(19_723), "2024-01-01");
        assert_eq!(date_label(19_782), "2024-02-29");
        assert_eq!(month_label(month_key(1_700_000_000)), "2023-11");
    }

    #[test]
    fn accuracy_formula() {
        assert_eq!(accuracy(10, 0, 0, 0), 100.0);
        assert!((accuracy(1, 1, 0, 0) - 100.0 * 400.0 / 600.0).abs() < 1e-9);
        assert_eq!(accuracy(0, 0, 0, 0), 0.0);
    }

    #[test]
    fn combo_from_header() {
        let b = crate::replay::tests::build(0, "3f8beee408d05f83de870f9f86fc375c", "fr01hy", 0, [1, 2, 3, 4], 0);
        assert_eq!(header_max_combo(&b), Some(245));
    }

    #[test]
    fn filters_change_results() {
        let now = 1_000_000_000;
        let plays = vec![
            play(1, "a", 0, Some(100.0), Some(5.0), true, now - 86_400),
            play(2, "a", 64, Some(150.0), Some(6.0), true, now - 86_400 * 10),
            play(3, "b", 8 | 64, Some(50.0), Some(4.0), false, now - 86_400 * 200),
            play(4, "c", 0, None, None, true, now - 86_400 * 2),
        ];
        let mut f = Filter::default();
        assert_eq!(f.apply(&plays, now).len(), 4);
        f.include_fails = false;
        assert_eq!(f.apply(&plays, now).len(), 3);
        f.include_fails = true;
        f.range = Range::Days30;
        assert_eq!(f.apply(&plays, now), vec![0, 1, 3]);
        f.range = Range::All;
        f.mods_sel[4] = true; // DT
        assert_eq!(f.apply(&plays, now), vec![1, 2]);
        f.mods_exact = true; // DT and nothing else tracked
        assert_eq!(f.apply(&plays, now), vec![1]);
        f.mods_sel[4] = false; // exact no-mod
        assert_eq!(f.apply(&plays, now), vec![0, 3]);
        f.mods_exact = false;
        f.star_min = 5.0;
        assert_eq!(f.apply(&plays, now), vec![0, 1]);
        f.star_min = 0.0;
        f.best_only = true;
        assert_eq!(f.apply(&plays, now), vec![1, 2, 3]);
    }

    #[test]
    fn mod_usage_counts_add_up() {
        let plays = vec![play(1, "a", 0, None, None, true, 0), play(2, "b", 0, None, None, true, 0), play(3, "c", 64, None, None, true, 0)];
        let u = mod_usage(&plays, &[0, 1, 2]);
        assert_eq!(u[0].0, "NM");
        assert_eq!(u[0].1.len(), 2);
        assert_eq!(u.iter().map(|x| x.1.len()).sum::<usize>(), 3);
    }

    #[test]
    fn pp_is_computed_once_and_loaded() {
        use crate::config::Config;
        use md5::{Digest, Md5};
        const OSU: &str = "osu file format v14\n\n[General]\nMode: 0\n\n[Metadata]\nTitle:T\nArtist:A\nCreator:C\nVersion:V\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:4\nOverallDifficulty:8\nApproachRate:9\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0\n100,100,1500,1,0\n";
        let dir = std::env::temp_dir().join(format!("osu-pp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sub = dir.join("files/a/ab");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("map1"), OSU).unwrap();
        let mut h = Md5::new();
        h.update(OSU.as_bytes());
        let md5: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        std::fs::write(sub.join("rep1"), crate::replay::tests::build(0, &md5, "fr01hy", 0, [2, 0, 0, 0], 0)).unwrap();
        let cfg = Config { lazer_path: String::new(), player_name: "fr01hy".into() };
        let mut conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        scan::scan(&mut conn, &dir, &cfg).unwrap();
        let s = ensure_pp(&mut conn, &dir).unwrap();
        assert_eq!((s.new, s.failed), (1, 0));
        let s2 = ensure_pp(&mut conn, &dir).unwrap();
        assert_eq!(s2.new + s2.failed, 0);
        let d = load(&conn, s).unwrap();
        assert_eq!(d.plays.len(), 1);
        assert_eq!(d.plays[0].title, "A - T [V]");
        assert!(d.plays[0].pp.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
