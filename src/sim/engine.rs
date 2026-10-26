//! Hit detection (lazer rules): press events, hover test, note lock,
//! early-click miss, auto-miss. Circles and slider heads only.

use super::objects::{Kind, Prepared, Windows, MISS_WINDOW};
use crate::replay::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    K1,
    K2,
    M1,
    M2,
}

impl Key {
    pub fn as_str(self) -> &'static str {
        match self {
            Key::K1 => "K1",
            Key::K2 => "K2",
            Key::M1 => "M1",
            Key::M2 => "M2",
        }
    }
}

/// How a time offset maps to 300/100/50/miss at a window edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// |offset| <= window (fractional window kept).
    Inclusive,
    /// |offset| <= floor(window).
    InclusiveFloor,
    /// |offset| < floor(window) (stable-like).
    ExclusiveFloor,
}

/// Verified 2026-10-26: reproduces the header counts of two real lazer replays exactly.
pub const PRIMARY_RULE: Rule = Rule::ExclusiveFloor;

pub const RULES: [Rule; 3] = [Rule::Inclusive, Rule::InclusiveFloor, Rule::ExclusiveFloor];

pub fn classify(w: &Windows, rule: Rule, abs_off: f64) -> i64 {
    let (g, o, m) = match rule {
        Rule::Inclusive => (w.great, w.ok, w.meh),
        _ => (w.great.floor(), w.ok.floor(), w.meh.floor()),
    };
    let le = |a: f64, b: f64| if rule == Rule::ExclusiveFloor { a < b } else { a <= b };
    if le(abs_off, g) {
        300
    } else if le(abs_off, o) {
        100
    } else if le(abs_off, m) {
        50
    } else {
        0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HitRec {
    /// Beatmap-time offset (click minus object time); None if never clicked.
    pub offset: Option<f64>,
    pub key: Option<Key>,
    pub judgement: i64,
}

/// Cursor position at beatmap time `t`, linearly interpolated.
pub fn cursor_at(frames: &[Frame], t: f64) -> (f64, f64) {
    let i = frames.partition_point(|f| f.t <= t);
    if i == 0 {
        return (frames[0].x as f64, frames[0].y as f64);
    }
    if i >= frames.len() {
        let l = &frames[frames.len() - 1];
        return (l.x as f64, l.y as f64);
    }
    let (a, b) = (&frames[i - 1], &frames[i]);
    let k = if b.t > a.t { (t - a.t) / (b.t - a.t) } else { 0.0 };
    (
        a.x as f64 + (b.x as f64 - a.x as f64) * k,
        a.y as f64 + (b.y as f64 - a.y as f64) * k,
    )
}

/// One entry per object in `p.objects` order; None for spinners.
pub fn simulate(p: &Prepared, frames: &[Frame]) -> Vec<Option<HitRec>> {
    let objs = &p.objects;
    let n = objs.len();
    let mut rec: Vec<Option<HitRec>> = vec![None; n];
    let skip = |k: usize| objs[k].kind == Kind::Spinner;
    let mut lo = 0usize; // first object that may still be unjudged
    let r2 = p.radius * p.radius;
    let miss_rec = HitRec { offset: None, key: None, judgement: 0 };

    let (mut prev_left, mut prev_right) = (false, false);
    for f in frames {
        let left = f.keys & 5 != 0;
        let right = f.keys & 10 != 0;
        let mut presses: [Option<Key>; 2] = [None, None];
        if left && !prev_left {
            presses[0] = Some(if f.keys & 4 != 0 { Key::K1 } else { Key::M1 });
        }
        if right && !prev_right {
            presses[1] = Some(if f.keys & 8 != 0 { Key::K2 } else { Key::M2 });
        }
        prev_left = left;
        prev_right = right;

        for key in presses.into_iter().flatten() {
            let (px, py) = (f.x as f64, f.y as f64);
            let mut k = lo;
            while k < n {
                let o = &objs[k];
                if o.time - p.preempt > f.t {
                    break;
                }
                let idx = k;
                k += 1;
                if skip(idx) || rec[idx].is_some() {
                    continue;
                }
                let (dx, dy) = (px - o.pos.0, py - o.pos.1);
                if dx * dx + dy * dy > r2 {
                    continue;
                }
                // note lock: earliest unjudged earlier object must have started
                let blocker = (lo..idx).find(|&m| !skip(m) && rec[m].is_none() && objs[m].time < o.time);
                if let Some(b) = blocker {
                    if f.t < objs[b].time {
                        continue;
                    }
                }
                let off = f.t - o.time;
                if off.abs() <= MISS_WINDOW {
                    let j = classify(&p.windows, PRIMARY_RULE, off.abs());
                    rec[idx] = Some(HitRec { offset: Some(off), key: Some(key), judgement: j });
                } else if off > 0.0 {
                    continue;
                }
                break; // press consumed (too-early presses do nothing)
            }
        }

        // update step: objects past the meh window become misses
        let mut m = lo;
        while m < n && objs[m].time + p.windows.meh < f.t {
            if !skip(m) && rec[m].is_none() {
                rec[m] = Some(miss_rec);
            }
            m += 1;
        }
        while lo < n && (skip(lo) || rec[lo].is_some()) {
            lo += 1;
        }
    }
    // Objects the replay reached but never clicked are misses. Objects after the
    // last frame were never played (failed or quit replay): leave them unjudged.
    let end = frames.last().map_or(f64::NEG_INFINITY, |f| f.t);
    for k in 0..n {
        if !skip(k) && rec[k].is_none() && objs[k].time <= end {
            rec[k] = Some(miss_rec);
        }
    }
    rec
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::objects::{load_map, prepare, ModAdjust};

    fn map(objs: &str) -> String {
        format!(
            "osu file format v14\n\n[General]\nMode: 0\nStackLeniency: 0.7\n\n[Metadata]\nTitle:T\nArtist:A\nCreator:C\nVersion:V\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:4\nOverallDifficulty:8\nApproachRate:9\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,0,100,1,0\n\n[HitObjects]\n{objs}"
        )
    }

    fn prep(objs: &str, adj: ModAdjust) -> Prepared {
        prepare(&load_map(map(objs).as_bytes()).unwrap(), &ModAdjust { rate: 1.0, ..adj })
    }

    fn fr(t: f64, x: f32, y: f32, keys: i32) -> Frame {
        Frame { t, x, y, keys }
    }

    /// press at (t,x,y) then release 20 ms later
    fn click(v: &mut Vec<Frame>, t: f64, x: f32, y: f32) {
        v.push(fr(t - 1.0, x, y, 0));
        v.push(fr(t, x, y, 1));
        v.push(fr(t + 20.0, x, y, 0));
    }

    #[test]
    fn judgements_by_offset() {
        let p = prep("100,100,1000,1,0\n200,200,2000,1,0\n300,100,3000,1,0\n400,100,4000,1,0\n", ModAdjust::default());
        let mut f = vec![];
        click(&mut f, 1031.0, 100.0, 100.0); // +31 -> 300 (32 is not, exclusive)
        click(&mut f, 1960.0, 200.0, 200.0); // -40 -> 100
        click(&mut f, 3110.0, 300.0, 100.0); // +110 -> 50
        f.push(fr(5000.0, 0.0, 0.0, 0)); // replay runs on; 4th never clicked
        let r = simulate(&p, &f);
        let j: Vec<i64> = r.iter().map(|x| x.unwrap().judgement).collect();
        assert_eq!(j, vec![300, 100, 50, 0]);
        assert_eq!(r[1].unwrap().offset, Some(-40.0));
        assert_eq!(r[0].unwrap().key, Some(Key::M1));
    }

    #[test]
    fn click_outside_radius_misses() {
        let p = prep("100,100,1000,1,0\n", ModAdjust::default());
        let mut f = vec![];
        click(&mut f, 1000.0, 140.0, 100.0); // 40 > 36.48
        assert_eq!(simulate(&p, &f)[0].unwrap().judgement, 0);
        let mut f = vec![];
        click(&mut f, 1000.0, 135.0, 100.0); // 35 < 36.48
        assert_eq!(simulate(&p, &f)[0].unwrap().judgement, 300);
    }

    #[test]
    fn early_click_is_miss_very_early_ignored() {
        let p = prep("100,100,1000,1,0\n", ModAdjust::default());
        let mut f = vec![];
        click(&mut f, 800.0, 100.0, 100.0); // -200: inside 400 window, outside meh -> miss
        let r = simulate(&p, &f)[0].unwrap();
        assert_eq!((r.judgement, r.offset), (0, Some(-200.0)));
        // -500 is ignored (object alive from 400), later click hits
        let mut f = vec![];
        click(&mut f, 500.0, 100.0, 100.0);
        click(&mut f, 1005.0, 100.0, 100.0);
        let r = simulate(&p, &f)[0].unwrap();
        assert_eq!((r.judgement, r.offset), (300, Some(5.0)));
    }

    #[test]
    fn note_lock_blocks_later_note() {
        // two overlapping circles at same place; click at t=1100 is on the
        // first (earliest under cursor); second is not hit by it.
        let p = prep("100,100,1000,1,0\n102,100,1100,1,0\n", ModAdjust::default());
        let mut f = vec![];
        click(&mut f, 1050.0, 100.0, 100.0);
        f.push(fr(3000.0, 0.0, 0.0, 0));
        let r = simulate(&p, &f);
        assert!(r[0].unwrap().offset.is_some());
        assert_eq!(r[1].unwrap().offset, None);
    }

    #[test]
    fn note_lock_ignores_click_on_later_note_before_earlier_starts() {
        // A at (100,100) t=1000, B far away at t=1100. Clicking B at t=950
        // (A not yet started) is ignored.
        let p = prep("100,100,1000,1,0\n400,300,1100,1,0\n", ModAdjust::default());
        let mut f = vec![];
        click(&mut f, 950.0, 400.0, 300.0);
        f.push(fr(3000.0, 0.0, 0.0, 0));
        let r = simulate(&p, &f);
        assert_eq!(r[1].unwrap().offset, None);
    }

    #[test]
    fn hr_flip_hit_position() {
        let p = prep("100,100,1000,1,0\n", ModAdjust { hr: true, ..Default::default() });
        let mut f = vec![];
        click(&mut f, 1000.0, 100.0, 284.0);
        assert_eq!(simulate(&p, &f)[0].unwrap().judgement, 300);
        let mut f = vec![];
        click(&mut f, 1000.0, 100.0, 100.0);
        assert_eq!(simulate(&p, &f)[0].unwrap().judgement, 0);
    }

    #[test]
    fn k1_k2_labels_and_held_keys() {
        let p = prep("100,100,1000,1,0\n100,100,1300,1,0\n", ModAdjust::default());
        let f = vec![
            fr(990.0, 100.0, 100.0, 0),
            fr(1000.0, 100.0, 100.0, 5),  // M1|K1 -> K1
            fr(1100.0, 100.0, 100.0, 5),  // held, no new press
            fr(1300.0, 100.0, 100.0, 15), // K2|M2 added -> K2
        ];
        let r = simulate(&p, &f);
        assert_eq!(r[0].unwrap().key, Some(Key::K1));
        assert_eq!(r[1].unwrap().key, Some(Key::K2));
    }

    #[test]
    fn slider_head_counts_and_spinner_skipped() {
        let p = prep("100,100,1000,2,0,L|200:100,1,100\n256,192,2000,12,0,3000\n", ModAdjust::default());
        let mut f = vec![];
        click(&mut f, 1010.0, 100.0, 100.0);
        let r = simulate(&p, &f);
        assert_eq!(r[0].unwrap().judgement, 300);
        assert!(r[1].is_none());
    }

    #[test]
    fn objects_after_replay_end_are_not_judged() {
        let p = prep("100,100,1000,1,0\n100,100,5000,1,0\n", ModAdjust::default());
        let mut f = vec![];
        click(&mut f, 1000.0, 100.0, 100.0); // replay ends at 1020 (quit)
        let r = simulate(&p, &f);
        assert!(r[0].is_some());
        assert!(r[1].is_none());
    }

    #[test]
    fn cursor_interpolation() {
        let f = vec![fr(0.0, 0.0, 0.0, 0), fr(100.0, 100.0, 50.0, 0)];
        assert_eq!(cursor_at(&f, 50.0), (50.0, 25.0));
        assert_eq!(cursor_at(&f, -5.0), (0.0, 0.0));
        assert_eq!(cursor_at(&f, 500.0), (100.0, 50.0));
    }

    #[test]
    fn rule_boundaries() {
        let w = crate::sim::objects::windows_for_od(8.0);
        assert_eq!(classify(&w, Rule::Inclusive, 32.0), 300);
        assert_eq!(classify(&w, Rule::ExclusiveFloor, 32.0), 100);
        assert_eq!(classify(&w, Rule::Inclusive, 120.0), 50);
        assert_eq!(classify(&w, Rule::Inclusive, 121.0), 0);
    }
}
