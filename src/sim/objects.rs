//! Beatmap -> simulation objects: mod-adjusted difficulty, Hard Rock flip,
//! stacking, hit windows, local density and spacing. No database access.

use rosu_map::section::general::GameMode;
use rosu_map::section::hit_objects::HitObjectKind;

pub const PLAYFIELD_H: f64 = 384.0;
pub const MISS_WINDOW: f64 = 400.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Circle,
    Slider,
    Spinner,
}

#[derive(Debug, Clone)]
pub struct RawObj {
    pub index: usize,
    pub kind: Kind,
    pub time: f64,
    pub end_time: f64,
    pub pos: (f64, f64),
    pub end_pos: (f64, f64),
}

#[derive(Debug, Clone)]
pub struct RawMap {
    pub standard: bool,
    pub cs: f64,
    pub ar: f64,
    pub od: f64,
    pub stack_leniency: f64,
    pub objects: Vec<RawObj>,
}

pub fn load_map(bytes: &[u8]) -> Result<RawMap, String> {
    let mut bm = rosu_map::Beatmap::from_bytes(bytes).map_err(|e| format!("beatmap parse: {e}"))?;
    let mut objects = Vec::with_capacity(bm.hit_objects.len());
    for (index, ho) in bm.hit_objects.iter_mut().enumerate() {
        let time = ho.start_time;
        match &mut ho.kind {
            HitObjectKind::Circle(c) => {
                let p = (c.pos.x as f64, c.pos.y as f64);
                objects.push(RawObj { index, kind: Kind::Circle, time, end_time: time, pos: p, end_pos: p });
            }
            HitObjectKind::Slider(s) => {
                let dur = s.duration();
                let prog = if s.span_count() % 2 == 1 { 1.0 } else { 0.0 };
                let rel = s.path.curve().position_at(prog);
                let p = (s.pos.x as f64, s.pos.y as f64);
                objects.push(RawObj {
                    index,
                    kind: Kind::Slider,
                    time,
                    end_time: time + dur,
                    pos: p,
                    end_pos: (p.0 + rel.x as f64, p.1 + rel.y as f64),
                });
            }
            HitObjectKind::Spinner(sp) => {
                let c = (256.0, 192.0);
                objects.push(RawObj { index, kind: Kind::Spinner, time, end_time: time + sp.duration, pos: c, end_pos: c });
            }
            HitObjectKind::Hold(_) => {}
        }
    }
    Ok(RawMap {
        standard: bm.mode == GameMode::Osu,
        cs: bm.circle_size as f64,
        ar: bm.approach_rate as f64,
        od: bm.overall_difficulty as f64,
        stack_leniency: bm.stack_leniency as f64,
        objects,
    })
}

#[derive(Debug, Clone, Default)]
pub struct ModAdjust {
    pub hr: bool,
    pub ez: bool,
    pub rate: f64,
    pub cs: Option<f64>,
    pub ar: Option<f64>,
    pub od: Option<f64>,
}

/// Hit windows in beatmap ms (not rate-adjusted), half-width.
#[derive(Debug, Clone, Copy)]
pub struct Windows {
    pub great: f64,
    pub ok: f64,
    pub meh: f64,
}

pub fn difficulty_range(v: f64, min: f64, mid: f64, max: f64) -> f64 {
    if v > 5.0 {
        mid + (max - mid) * (v - 5.0) / 5.0
    } else if v < 5.0 {
        mid - (mid - min) * (5.0 - v) / 5.0
    } else {
        mid
    }
}

pub fn windows_for_od(od: f64) -> Windows {
    Windows {
        great: difficulty_range(od, 80.0, 50.0, 20.0),
        ok: difficulty_range(od, 140.0, 100.0, 60.0),
        meh: difficulty_range(od, 200.0, 150.0, 100.0),
    }
}

#[derive(Debug, Clone)]
pub struct Obj {
    pub index: usize,
    pub kind: Kind,
    pub time: f64,
    pub end_time: f64,
    /// Final position (game space: HR flipped, stack offset applied).
    pub pos: (f64, f64),
    pub end_pos: (f64, f64),
    pub stack: i32,
}

#[derive(Debug, Clone)]
pub struct Prepared {
    pub objects: Vec<Obj>,
    pub radius: f64,
    pub preempt: f64,
    pub windows: Windows,
    pub hr: bool,
    pub rate: f64,
    pub circles: u32,
    pub sliders: u32,
    pub spinners: u32,
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// lazer OsuBeatmapProcessor stacking (v6+ algorithm) over the whole map.
fn apply_stacking(o: &mut [Obj], preempt: f64, leniency: f64) {
    const D: f64 = 3.0;
    let thr = preempt * leniency;
    for i in (1..o.len()).rev() {
        if o[i].stack != 0 || o[i].kind == Kind::Spinner {
            continue;
        }
        let mut n = i;
        let mut cur = i;
        if o[i].kind == Kind::Circle {
            while n > 0 {
                n -= 1;
                if o[n].kind == Kind::Spinner {
                    continue;
                }
                if o[cur].time - o[n].end_time > thr {
                    break;
                }
                if o[n].kind == Kind::Slider && dist(o[n].end_pos, o[cur].pos) < D {
                    let offset = o[cur].stack - o[n].stack + 1;
                    for j in (n + 1)..=i {
                        if dist(o[n].end_pos, o[j].pos) < D {
                            o[j].stack -= offset;
                        }
                    }
                    break;
                }
                if dist(o[n].pos, o[cur].pos) < D {
                    o[n].stack = o[cur].stack + 1;
                    cur = n;
                }
            }
        } else {
            while n > 0 {
                n -= 1;
                if o[n].kind == Kind::Spinner {
                    continue;
                }
                if o[cur].time - o[n].end_time > thr {
                    break;
                }
                if dist(o[n].end_pos, o[cur].pos) < D {
                    o[n].stack = o[cur].stack + 1;
                    cur = n;
                }
            }
        }
    }
}

pub fn prepare(raw: &RawMap, adj: &ModAdjust) -> Prepared {
    let (mut cs, mut ar, mut od) = (raw.cs, raw.ar, raw.od);
    if adj.ez {
        cs *= 0.5;
        ar *= 0.5;
        od *= 0.5;
    }
    if adj.hr {
        cs = (cs * 1.3).min(10.0);
        ar = (ar * 1.4).min(10.0);
        od = (od * 1.4).min(10.0);
    }
    if let Some(v) = adj.cs {
        cs = v;
    }
    if let Some(v) = adj.ar {
        ar = v;
    }
    if let Some(v) = adj.od {
        od = v;
    }
    let preempt = difficulty_range(ar, 1800.0, 1200.0, 450.0);
    let scale = (1.0 - 0.7 * (cs - 5.0) / 5.0) / 2.0;
    let flip = |p: (f64, f64)| if adj.hr { (p.0, PLAYFIELD_H - p.1) } else { p };

    let mut objects: Vec<Obj> = raw
        .objects
        .iter()
        .map(|r| Obj {
            index: r.index,
            kind: r.kind,
            time: r.time,
            end_time: r.end_time,
            pos: flip(r.pos),
            end_pos: flip(r.end_pos),
            stack: 0,
        })
        .collect();
    objects.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
    apply_stacking(&mut objects, preempt, raw.stack_leniency);
    let (mut circles, mut sliders, mut spinners) = (0, 0, 0);
    for o in objects.iter_mut() {
        let off = o.stack as f64 * scale * -6.4;
        o.pos = (o.pos.0 + off, o.pos.1 + off);
        o.end_pos = (o.end_pos.0 + off, o.end_pos.1 + off);
        match o.kind {
            Kind::Circle => circles += 1,
            Kind::Slider => sliders += 1,
            Kind::Spinner => spinners += 1,
        }
    }
    Prepared {
        objects,
        radius: 64.0 * scale,
        preempt,
        windows: windows_for_od(od),
        hr: adj.hr,
        rate: if adj.rate > 0.0 { adj.rate } else { 1.0 },
        circles,
        sliders,
        spinners,
    }
}

/// Per object (same order as `Prepared::objects`): local density in objects per
/// real second within +-1 real second, and spacing from the previous
/// non-spinner object's end to this object's start (osu!px).
pub fn density_and_spacing(p: &Prepared) -> Vec<(f64, Option<f64>)> {
    let times: Vec<f64> = p.objects.iter().filter(|o| o.kind != Kind::Spinner).map(|o| o.time).collect();
    let half = 1000.0 * p.rate;
    let (mut lo, mut hi) = (0usize, 0usize);
    let mut out = Vec::with_capacity(p.objects.len());
    let mut prev_end: Option<(f64, f64)> = None;
    for o in &p.objects {
        if o.kind == Kind::Spinner {
            out.push((0.0, None));
            continue;
        }
        while times[lo] < o.time - half {
            lo += 1;
        }
        while hi < times.len() && times[hi] <= o.time + half {
            hi += 1;
        }
        let density = (hi - lo) as f64 / 2.0;
        out.push((density, prev_end.map(|e| dist(e, o.pos))));
        prev_end = Some(o.end_pos);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn osu(objs: &str, extra_diff: &str) -> String {
        format!(
            "osu file format v14\n\n[General]\nMode: 0\nStackLeniency: 0.7\n\n[Metadata]\nTitle:T\nArtist:A\nCreator:C\nVersion:V\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:4\nOverallDifficulty:8\nApproachRate:9\nSliderMultiplier:1.4\nSliderTickRate:1\n{extra_diff}\n[TimingPoints]\n0,500,4,2,0,100,1,0\n\n[HitObjects]\n{objs}"
        )
    }

    #[test]
    fn slider_end_and_duration() {
        let m = load_map(osu("100,100,1000,2,0,L|200:100,1,100\n100,100,3000,2,0,L|200:100,2,100\n", "").as_bytes()).unwrap();
        assert_eq!(m.objects[0].kind, Kind::Slider);
        assert!((m.objects[0].end_pos.0 - 200.0).abs() < 1e-3 && (m.objects[0].end_pos.1 - 100.0).abs() < 1e-3);
        assert!((m.objects[0].end_time - 1000.0 - 100.0 / 0.28).abs() < 1.0, "{}", m.objects[0].end_time);
        assert!((m.objects[1].end_pos.0 - 100.0).abs() < 1e-3, "{:?}", m.objects[1].end_pos);
    }

    #[test]
    fn stacking_two_circles() {
        let m = load_map(osu("100,100,1000,1,0\n100,100,1200,1,0\n", "").as_bytes()).unwrap();
        let p = prepare(&m, &ModAdjust { rate: 1.0, ..Default::default() });
        // CS4 scale = 0.57, offset per stack = 3.648
        assert_eq!(p.objects[0].stack, 1);
        assert_eq!(p.objects[1].stack, 0);
        assert!((p.objects[0].pos.0 - (100.0 - 3.648)).abs() < 1e-6);
        assert!((p.radius - 36.48).abs() < 1e-6);
    }

    #[test]
    fn hr_flips_and_scales_difficulty() {
        let m = load_map(osu("100,100,1000,1,0\n", "").as_bytes()).unwrap();
        let p = prepare(&m, &ModAdjust { rate: 1.0, hr: true, ..Default::default() });
        assert_eq!(p.objects[0].pos, (100.0, 284.0));
        assert!((p.windows.great - (80.0 - 6.0 * 10.0)).abs() < 1e-9); // OD 8*1.4 -> 10 (clamped)
        let e = prepare(&m, &ModAdjust { rate: 1.0, ez: true, ..Default::default() });
        assert!((e.windows.great - (80.0 - 6.0 * 4.0)).abs() < 1e-9);
    }

    #[test]
    fn windows_od8_and_preempt() {
        let w = windows_for_od(8.0);
        assert_eq!((w.great, w.ok, w.meh), (32.0, 76.0, 120.0));
        assert!((difficulty_range(9.0, 1800.0, 1200.0, 450.0) - 600.0).abs() < 1e-9);
    }

    #[test]
    fn density_and_spacing_basic() {
        let m = load_map(osu("0,0,1000,1,0\n30,40,1500,1,0\n", "").as_bytes()).unwrap();
        let p = prepare(&m, &ModAdjust { rate: 1.0, ..Default::default() });
        let d = density_and_spacing(&p);
        assert_eq!(d[0], (1.0, None));
        assert_eq!(d[1].0, 1.0);
        assert!((d[1].1.unwrap() - 50.0).abs() < 1e-9);
    }
}
