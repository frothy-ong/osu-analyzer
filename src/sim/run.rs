//! Runs the simulator over all kept replays, stores hits and sim_check rows,
//! and prints the verification report.

use super::{header_matches, mods_to_string, simulate_replay, MapType, SimOut, MISS_TOLERANCE};
use crate::{db, replay, scan};
use rayon::prelude::*;
use rusqlite::{params, Connection};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

struct Job {
    score_id: i64,
    replay_hash: String,
    map_hash: String,
    header: [u16; 4],
}

pub struct RunStats {
    pub todo: usize,
    pub seconds: f64,
}

fn read(paths: &HashMap<String, PathBuf>, hash: &str) -> Result<Vec<u8>, String> {
    let p = paths.get(hash).ok_or_else(|| format!("file missing: {hash}"))?;
    std::fs::read(p).map_err(|e| format!("read {hash}: {e}")) // read-only
}

fn do_job(paths: &HashMap<String, PathBuf>, j: &Job) -> Result<SimOut, String> {
    let rb = read(paths, &j.replay_hash)?;
    let mb = read(paths, &j.map_hash)?;
    let rep = replay::parse_full(&rb).map_err(|e| format!("replay: {e}"))?;
    simulate_replay(&mb, &rep)
}

pub fn run(conn: &mut Connection, lazer: &Path) -> Result<RunStats, String> {
    let t0 = Instant::now();
    let jobs: Vec<Job> = {
        let mut st = conn
            .prepare(
                "SELECT r.score_id, r.file_hash, b.file_hash, r.n300, r.n100, r.n50, r.nmiss
                 FROM replays r JOIN beatmaps b ON b.md5 = r.beatmap_md5
                 WHERE r.excluded_reason IS NULL
                   AND r.score_id NOT IN (SELECT score_id FROM sim_check WHERE analyzer_version = ?1)
                 ORDER BY r.score_id",
            )
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map(params![db::ANALYZER_VERSION], |r| {
                Ok(Job {
                    score_id: r.get(0)?,
                    replay_hash: r.get(1)?,
                    map_hash: r.get(2)?,
                    header: [r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?],
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
    };
    if jobs.is_empty() {
        return Ok(RunStats { todo: 0, seconds: t0.elapsed().as_secs_f64() });
    }
    let paths: HashMap<String, PathBuf> = scan::list_files(&lazer.join("files"))
        .into_iter()
        .filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), p)))
        .collect();

    let total = jobs.len();
    let mut done = 0usize;
    for chunk in jobs.chunks(256) {
        let results: Vec<(&Job, Result<SimOut, String>)> =
            chunk.par_iter().map(|j| (j, do_job(&paths, j))).collect();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        {
            let mut del = tx.prepare_cached("DELETE FROM hits WHERE score_id=?1").map_err(|e| e.to_string())?;
            let mut ins = tx
                .prepare_cached("INSERT INTO hits(score_id,object_index,time_ms,offset_ms,cursor_dx,cursor_dy,key,judgement,local_density,spacing,obj_x,obj_y) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)")
                .map_err(|e| e.to_string())?;
            let mut chk = tx
                .prepare_cached(
                    "INSERT OR REPLACE INTO sim_check(score_id,analyzer_version,sim_n300,sim_n100,sim_n50,sim_nmiss,matches_header,map_type,expected_objects,rate,status,match_incl,match_incl_floor,match_excl_floor,miss_diff) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
                )
                .map_err(|e| e.to_string())?;
            for (j, res) in results {
                del.execute(params![j.score_id]).map_err(|e| e.to_string())?;
                match res {
                    Ok(o) => {
                        for h in &o.hits {
                            ins.execute(params![
                                j.score_id, h.object_index, h.time_ms, h.offset_ms, h.cursor_dx, h.cursor_dy,
                                h.key, h.judgement, h.local_density, h.spacing, h.obj_x, h.obj_y
                            ])
                            .map_err(|e| e.to_string())?;
                        }
                        let m = header_matches(o.map_type, o.counts, j.header);
                        let exact = |c: [u32; 4]| (0..4).all(|i| c[i] == j.header[i] as u32) as i64;
                        let miss_diff = o.counts[3] as i64 - j.header[3] as i64;
                        chk.execute(params![
                            j.score_id, db::ANALYZER_VERSION, o.counts[0], o.counts[1], o.counts[2], o.counts[3],
                            m as i64, o.map_type.as_str(), o.expected_objects, o.rate, "ok",
                            exact(o.counts_by_rule[0]), exact(o.counts_by_rule[1]), exact(o.counts_by_rule[2]),
                            miss_diff
                        ])
                        .map_err(|e| e.to_string())?;
                        tx.execute("UPDATE replays SET rate=?1 WHERE score_id=?2", params![o.rate, j.score_id])
                            .map_err(|e| e.to_string())?;
                    }
                    Err(e) => {
                        chk.execute(params![
                            j.score_id, db::ANALYZER_VERSION, None::<i64>, None::<i64>, None::<i64>, None::<i64>,
                            0i64, None::<String>, None::<i64>, None::<f64>, e,
                            None::<i64>, None::<i64>, None::<i64>, None::<i64>
                        ])
                        .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        done += chunk.len();
        println!("  simulated {done}/{total}");
    }
    Ok(RunStats { todo: total, seconds: t0.elapsed().as_secs_f64() })
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, params![db::ANALYZER_VERSION], |r| r.get(0)).unwrap_or(-1)
}

fn pct(a: i64, b: i64) -> String {
    if b <= 0 { "n/a".into() } else { format!("{:.1}%", 100.0 * a as f64 / b as f64) }
}

pub fn print_report(conn: &Connection, s: &RunStats) {
    println!("\n=== M02 simulator report (analyzer_version {}) ===", db::ANALYZER_VERSION);
    println!("Units: offsets and times are real milliseconds (rate-adjusted); cursor in osu!px.");
    if s.todo > 0 {
        println!(
            "Speed: {} replays in {:.1} s = {:.1} replays/s",
            s.todo, s.seconds, s.todo as f64 / s.seconds.max(1e-9)
        );
    } else {
        println!("Nothing new to simulate (all kept replays already done at this version).");
    }
    println!("Kept replays:               {}", count(conn, "SELECT COUNT(*) FROM replays WHERE excluded_reason IS NULL AND ?1=?1"));
    println!("  no beatmap file (skipped): {}", count(conn, "SELECT COUNT(*) FROM replays r WHERE excluded_reason IS NULL AND ?1=?1 AND NOT EXISTS (SELECT 1 FROM beatmaps b WHERE b.md5=r.beatmap_md5)"));
    println!("  simulated ok:              {}", count(conn, "SELECT COUNT(*) FROM sim_check WHERE analyzer_version=?1 AND status='ok'"));
    if let Ok(mut st) = conn.prepare("SELECT status, COUNT(*) FROM sim_check WHERE analyzer_version=?1 AND status!='ok' GROUP BY 1 ORDER BY 2 DESC") {
        if let Ok(rows) = st.query_map(params![db::ANALYZER_VERSION], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
            for (status, n) in rows.flatten() {
                println!("  failed - {status}: {n}");
            }
        }
    }
    println!("  complete plays (every object judged): {}", count(conn, "SELECT COUNT(*) FROM sim_check WHERE analyzer_version=?1 AND status='ok' AND sim_n300+sim_n100+sim_n50+sim_nmiss = expected_objects"));
    println!("  incomplete plays (failed/quit, fewer judged): {}", count(conn, "SELECT COUNT(*) FROM sim_check WHERE analyzer_version=?1 AND status='ok' AND sim_n300+sim_n100+sim_n50+sim_nmiss < expected_objects"));
    println!("  hit rows stored:           {}", count(conn, "SELECT COUNT(*) FROM hits WHERE ?1=?1"));

    println!("\nCount check vs replay header, by map type:");
    let q = "SELECT map_type, COUNT(*), SUM(matches_header), SUM(match_incl), SUM(match_incl_floor), SUM(match_excl_floor), SUM(miss_diff>=0)
             FROM sim_check WHERE analyzer_version=?1 AND status='ok' GROUP BY map_type ORDER BY map_type";
    if let Ok(mut st) = conn.prepare(q) {
        let rows = st.query_map(params![db::ANALYZER_VERSION], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?, r.get::<_, i64>(5)?, r.get::<_, i64>(6)?))
        });
        if let Ok(rows) = rows {
            let mut circles_seen = false;
            for (t, n, m, a, b, c, ge) in rows.flatten() {
                if t == MapType::Circles.as_str() {
                    circles_seen = true;
                    println!("  circles only: {n} replays; exact match {m} ({})", pct(m, n));
                    println!("    exact match by window-edge rule: inclusive {a}, inclusive-floor {b}, exclusive-floor {c}");
                    println!("    TEST 1 (>=95% exact): {}", if n > 0 && m * 100 >= n * 95 { "PASS" } else { "FAIL" });
                } else {
                    println!("  {t}: {n} replays; misses within +-{MISS_TOLERANCE} of header: {m} ({}); sim misses >= header misses: {ge}", pct(m, n));
                }
            }
            if !circles_seen {
                println!("  circles only: 0 replays (TEST 1 cannot be measured on this data)");
            }
        }
    }

    println!("\nMismatches by mods and map type (top 12):");
    let mut groups: BTreeMap<(String, String), (i64, i64)> = BTreeMap::new();
    if let Ok(mut st) = conn.prepare(
        "SELECT s.map_type, r.mods, COUNT(*), SUM(s.matches_header=0) FROM sim_check s JOIN replays r USING(score_id)
         WHERE s.analyzer_version=?1 AND s.status='ok' GROUP BY 1,2",
    ) {
        if let Ok(rows) = st.query_map(params![db::ANALYZER_VERSION], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))
        }) {
            for (t, m, n, bad) in rows.flatten() {
                let e = groups.entry((t, mods_to_string(m as i32))).or_insert((0, 0));
                e.0 += n;
                e.1 += bad;
            }
        }
    }
    let mut g: Vec<_> = groups.into_iter().filter(|(_, v)| v.1 > 0).collect();
    g.sort_by(|a, b| b.1.1.cmp(&a.1.1));
    if g.is_empty() {
        println!("  none");
    }
    for ((t, m), (n, bad)) in g.into_iter().take(12) {
        println!("  {t:<17} {m:<10} {bad} of {n} mismatched");
    }

    println!("\n3 worst mismatches, hit by hit (non-300 rows only, max 40 each):");
    let worst = "SELECT s.score_id, r.file_hash, r.mods, r.n300, r.n100, r.n50, r.nmiss,
                        s.sim_n300, s.sim_n100, s.sim_n50, s.sim_nmiss, s.map_type, b.artist, b.title, b.version
                 FROM sim_check s JOIN replays r USING(score_id) LEFT JOIN beatmaps b ON b.md5=r.beatmap_md5
                 WHERE s.analyzer_version=?1 AND s.status='ok' AND s.matches_header=0
                 ORDER BY (s.map_type='circles') DESC,
                          CASE WHEN s.map_type='circles'
                               THEN abs(r.n300-s.sim_n300)+abs(r.n100-s.sim_n100)+abs(r.n50-s.sim_n50)+abs(r.nmiss-s.sim_nmiss)
                               ELSE abs(s.miss_diff) END DESC
                 LIMIT 3";
    let mut found = false;
    if let Ok(mut st) = conn.prepare(worst) {
        type W = (i64, String, i64, [i64; 4], [i64; 4], String, String);
        let rows: Vec<W> = st
            .query_map(params![db::ANALYZER_VERSION], |r| {
                Ok((
                    r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?,
                    [r.get::<_, i64>(3)?, r.get(4)?, r.get(5)?, r.get(6)?],
                    [r.get::<_, i64>(7)?, r.get(8)?, r.get(9)?, r.get(10)?],
                    r.get::<_, String>(11)?,
                    format!("{} - {} [{}]", r.get::<_, String>(12).unwrap_or_default(), r.get::<_, String>(13).unwrap_or_default(), r.get::<_, String>(14).unwrap_or_default()),
                ))
            })
            .map(|it| it.flatten().collect())
            .unwrap_or_default();
        for (id, hash, mods, h, sim, mt, title) in rows {
            found = true;
            println!("- replay #{id} ({}) {title}", &hash[..hash.len().min(8)]);
            println!("  mods {} | {mt} | header 300/100/50/miss = {:?} | sim = {:?}", mods_to_string(mods as i32), h, sim);
            if let Ok(mut hs) = conn.prepare(
                "SELECT object_index, time_ms, offset_ms, key, judgement FROM hits WHERE score_id=?1 AND judgement!=300 ORDER BY object_index LIMIT 40",
            ) {
                if let Ok(it) = hs.query_map(params![id], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, Option<f64>>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, i64>(4)?))
                }) {
                    for (oi, t, off, key, j) in it.flatten() {
                        match off {
                            Some(o) => println!("    obj {oi} at {t:.0} ms: offset {o:+.1} ms, key {} -> {}", key.unwrap_or_default(), if j == 0 { "MISS (early/late click)".to_string() } else { j.to_string() }),
                            None => println!("    obj {oi} at {t:.0} ms: no click -> MISS"),
                        }
                    }
                }
            }
        }
    }
    if !found {
        println!("  none (no mismatches)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::replay::tests::build_full;
    use md5::{Digest, Md5};

    const OSU: &str = "osu file format v14\n\n[General]\nMode: 0\n\n[Metadata]\nTitle:T\nArtist:A\nCreator:C\nVersion:V\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:4\nOverallDifficulty:8\nApproachRate:9\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0\n100,100,1500,1,0\n";

    #[test]
    fn run_stores_rows_and_is_incremental() {
        let dir = std::env::temp_dir().join(format!("osu-sim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sub = dir.join("files/a/ab");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("map1"), OSU).unwrap();
        let mut h = Md5::new();
        h.update(OSU.as_bytes());
        let md5: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let text = "0|256|-500|0,-1|256|-500|0,991|256|192|0,20|256|192|1,20|256|192|0,410|100|100|0,20|100|100|1,20|100|100|0,700|100|100|0,";
        std::fs::write(sub.join("rep1"), build_full(&md5, "fr01hy", 0, [1, 1, 0, 0], text, true, None)).unwrap();
        std::fs::write(sub.join("rep2"), build_full(&md5, "fr01hy", 0, [2, 0, 0, 0], text, false, None)).unwrap();

        let cfg = Config { lazer_path: String::new(), player_name: "fr01hy".into() };
        let mut conn = Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        scan::scan(&mut conn, &dir, &cfg).unwrap();
        let s = run(&mut conn, &dir).unwrap();
        assert_eq!(s.todo, 2);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM hits WHERE ?1=?1"), 4);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM sim_check WHERE matches_header=1 AND ?1=?1"), 1);
        let s2 = run(&mut conn, &dir).unwrap();
        assert_eq!(s2.todo, 0);
        print_report(&conn, &s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
