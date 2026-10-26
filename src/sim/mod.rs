//! Hit simulator (M02). `simulate_replay` is pure; `run` talks to SQLite.

pub mod engine;
pub mod objects;
pub mod run;

use crate::replay::{self, LazerMod, ReplayData};
use engine::{classify, cursor_at, simulate, RULES};
use objects::{density_and_spacing, load_map, prepare, ModAdjust};

/// Allowed |sim misses - header misses| on maps with sliders or spinners.
pub const MISS_TOLERANCE: i64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapType {
    Circles,
    CirclesSpinners,
    Sliders,
}

impl MapType {
    pub fn as_str(self) -> &'static str {
        match self {
            MapType::Circles => "circles",
            MapType::CirclesSpinners => "circles+spinners",
            MapType::Sliders => "sliders",
        }
    }
}

#[derive(Debug, Clone)]
pub struct HitRow {
    pub object_index: i64,
    pub time_ms: f64,
    pub offset_ms: Option<f64>,
    pub cursor_dx: f64,
    pub cursor_dy: f64,
    pub key: Option<&'static str>,
    pub judgement: i64,
    pub local_density: f64,
    pub spacing: Option<f64>,
    pub obj_x: f64,
    pub obj_y: f64,
}

#[derive(Debug, Clone)]
pub struct SimOut {
    pub hits: Vec<HitRow>,
    /// [300, 100, 50, miss] under the primary rule.
    pub counts: [u32; 4],
    /// Same counts under each engine::RULES entry.
    pub counts_by_rule: [[u32; 4]; 3],
    pub map_type: MapType,
    pub expected_objects: u32,
    pub rate: f64,
}

fn idx(j: i64) -> usize {
    match j {
        300 => 0,
        100 => 1,
        50 => 2,
        _ => 3,
    }
}

pub fn mod_adjust(header_mods: i32, lazer: &Option<Vec<LazerMod>>) -> Result<ModAdjust, String> {
    let has = |a: &str| lazer.as_ref().is_some_and(|m| m.iter().any(|x| x.acronym == a));
    let get = |a: &str| lazer.as_ref().and_then(|m| m.iter().find(|x| x.acronym == a));
    if has("WU") || has("WD") || has("AS") {
        return Err("variable rate mod".into());
    }
    let rate = if let Some(m) = get("DT").or_else(|| get("NC")) {
        m.speed_change.unwrap_or(1.5)
    } else if let Some(m) = get("HT").or_else(|| get("DC")) {
        m.speed_change.unwrap_or(0.75)
    } else {
        replay::rate_from_mods(header_mods)
    };
    let da = get("DA");
    Ok(ModAdjust {
        hr: header_mods & replay::MOD_HARDROCK != 0 || has("HR"),
        ez: header_mods & replay::MOD_EASY != 0 || has("EZ"),
        rate,
        cs: da.and_then(|m| m.circle_size),
        ar: da.and_then(|m| m.approach_rate),
        od: da.and_then(|m| m.overall_difficulty),
    })
}

pub fn simulate_replay(map_bytes: &[u8], rep: &ReplayData) -> Result<SimOut, String> {
    if rep.frames.is_empty() {
        return Err("no frames".into());
    }
    let raw = load_map(map_bytes)?;
    if !raw.standard {
        return Err("beatmap is not osu!standard".into());
    }
    let adj = mod_adjust(rep.header.mods, &rep.lazer_mods)?;
    let p = prepare(&raw, &adj);
    let recs = simulate(&p, &rep.frames);
    let ds = density_and_spacing(&p);

    let mut hits = Vec::with_capacity(p.objects.len());
    let mut counts = [0u32; 4];
    let mut by_rule = [[0u32; 4]; 3];
    for (k, o) in p.objects.iter().enumerate() {
        let Some(r) = recs[k] else { continue };
        counts[idx(r.judgement)] += 1;
        for (ri, rule) in RULES.iter().enumerate() {
            let j = match r.offset {
                Some(off) => classify(&p.windows, *rule, off.abs()),
                None => 0,
            };
            by_rule[ri][idx(j)] += 1;
        }
        let (cx, cy) = cursor_at(&rep.frames, o.time);
        let (dx, mut dy) = (cx - o.pos.0, cy - o.pos.1);
        let mut oy = o.pos.1;
        if p.hr {
            dy = -dy;
            oy = objects::PLAYFIELD_H - oy;
        }
        hits.push(HitRow {
            object_index: o.index as i64,
            time_ms: o.time / p.rate,
            offset_ms: r.offset.map(|v| v / p.rate),
            cursor_dx: dx,
            cursor_dy: dy,
            key: r.key.map(|k| k.as_str()),
            judgement: r.judgement,
            local_density: ds[k].0,
            spacing: ds[k].1,
            obj_x: o.pos.0,
            obj_y: oy,
        });
    }
    let map_type = if p.sliders > 0 {
        MapType::Sliders
    } else if p.spinners > 0 {
        MapType::CirclesSpinners
    } else {
        MapType::Circles
    };
    Ok(SimOut {
        hits,
        counts,
        counts_by_rule: by_rule,
        map_type,
        expected_objects: p.circles + p.sliders,
        rate: p.rate,
    })
}

/// Circles-only maps must match the header exactly; others within tolerance on misses.
pub fn header_matches(t: MapType, sim: [u32; 4], h: [u16; 4]) -> bool {
    match t {
        MapType::Circles => (0..4).all(|i| sim[i] == h[i] as u32),
        _ => (sim[3] as i64 - h[3] as i64).abs() <= MISS_TOLERANCE,
    }
}

pub fn mods_to_string(m: i32) -> String {
    let table = [
        (1, "NF"), (2, "EZ"), (4, "TD"), (8, "HD"), (16, "HR"), (32, "SD"), (64, "DT"), (128, "RX"),
        (256, "HT"), (512, "NC"), (1024, "FL"), (4096, "SO"), (8192, "AP"), (16384, "PF"),
    ];
    let mut out = Vec::new();
    for (bit, name) in table {
        if m & bit != 0 && !(bit == 64 && m & 512 != 0) {
            out.push(name);
        }
    }
    if out.is_empty() { "NM".into() } else { out.join(",") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::tests::build_full;

    const MD5: &str = "3f8beee408d05f83de870f9f86fc375c";
    const MAP: &str = "osu file format v14\n\n[General]\nMode: 0\n\n[Metadata]\nTitle:T\nArtist:A\nCreator:C\nVersion:V\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:4\nOverallDifficulty:8\nApproachRate:9\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0\n100,100,1500,1,0\n300,300,2000,1,0\n";

    #[test]
    fn end_to_end_from_file_bytes() {
        // 1000: +10 -> 300 ; 1500: -40 -> 100 ; 2000: no click -> miss
        let text = "0|256|-500|0,-1|256|-500|0,991|256|192|0,20|256|192|1,20|256|192|0,410|100|100|0,20|100|100|1,20|100|100|0,700|100|100|0,-12345|0|0|1,";
        let b = build_full(MD5, "p", 0, [1, 1, 0, 1], text, true, None);
        let rep = replay::parse_full(&b).unwrap();
        let out = simulate_replay(MAP.as_bytes(), &rep).unwrap();
        assert_eq!(out.counts, [1, 1, 0, 1]);
        assert_eq!(out.map_type, MapType::Circles);
        assert!(header_matches(out.map_type, out.counts, [1, 1, 0, 1]));
        assert!(!header_matches(out.map_type, out.counts, [2, 0, 0, 1]));
        assert_eq!(out.hits.len(), 3);
        assert_eq!(out.hits[0].offset_ms, Some(10.0));
        assert_eq!(out.hits[0].key, Some("M1"));
        assert_eq!(out.hits[2].judgement, 0);
        assert_eq!(out.hits[2].offset_ms, None);
        assert!((out.hits[1].cursor_dx).abs() < 1e-9);
    }

    #[test]
    fn dt_offsets_are_real_ms() {
        let text = "0|256|-500|0,-1|256|-500|0,1001|256|192|0,15|256|192|1,20|256|192|0,";
        let json = br#"{"mods":[{"acronym":"DT"}]}"#;
        let b = build_full(MD5, "p", 64, [1, 0, 0, 0], text, true, Some(json));
        let rep = replay::parse_full(&b).unwrap();
        let out = simulate_replay(MAP.as_bytes(), &rep).unwrap();
        assert_eq!(out.rate, 1.5);
        assert_eq!(out.hits[0].offset_ms, Some(15.0 / 1.5));
        assert!((out.hits[0].time_ms - 1000.0 / 1.5).abs() < 1e-9);
    }

    #[test]
    fn custom_rate_and_variable_rate() {
        let m = vec![LazerMod { acronym: "DT".into(), speed_change: Some(1.3), ..Default::default() }];
        assert_eq!(mod_adjust(64, &Some(m)).unwrap().rate, 1.3);
        let w = vec![LazerMod { acronym: "WU".into(), ..Default::default() }];
        assert!(mod_adjust(0, &Some(w)).is_err());
        assert_eq!(mod_adjust(replay::MOD_HT, &None).unwrap().rate, 0.75);
    }

    #[test]
    fn mods_names() {
        assert_eq!(mods_to_string(0), "NM");
        assert_eq!(mods_to_string(8 | 16 | 64), "HD,HR,DT");
        assert_eq!(mods_to_string(64 | 512), "NC");
    }

    #[test]
    fn slider_map_tolerance() {
        assert!(header_matches(MapType::Sliders, [10, 5, 1, 3], [100, 0, 0, 1]));
        assert!(!header_matches(MapType::Sliders, [10, 5, 1, 4], [100, 0, 0, 1]));
    }
}
