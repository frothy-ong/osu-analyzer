//! M05 key statistics and stamina curve over the in-RAM hits.
//! "Hit" here = a judged object (300/100/50) that has a key and an offset.
//! Early-click misses (judgement 0 with a key) are counted separately and
//! excluded from every number below. Offsets are real ms.

use crate::data::Hit;

pub const KEY_NAMES: [&str; 4] = ["K1", "K2", "M1", "M2"];
pub const HIST_MAX: f64 = 200.0;
pub const HIST_BIN: f64 = 4.0;
pub const NBINS: usize = 100; // 2 * HIST_MAX / HIST_BIN
pub const TIME_BIN_MS: f64 = 30_000.0;
pub const MIN_CURVE_SAMPLES: u32 = 30;
/// Upper edges of the density bands, objects per real second (+-1 s window).
pub const DENSITY_EDGES: [f32; 5] = [2.0, 4.0, 6.0, 8.0, 10.0];
pub const NBANDS: usize = 6;
const MAX_TIME_BINS: usize = 480;

pub fn band(d: f32) -> usize {
    DENSITY_EDGES.iter().take_while(|e| d >= **e).count()
}

pub fn band_label(b: usize) -> String {
    match b {
        0 => format!("< {}", DENSITY_EDGES[0]),
        b if b >= NBANDS - 1 => format!(">= {}", DENSITY_EDGES[NBANDS - 2]),
        b => format!("{}-{}", DENSITY_EDGES[b - 1], DENSITY_EDGES[b]),
    }
}

#[derive(Clone, Debug, Default)]
pub struct KeyStat {
    pub hits: u64,
    pub mean: f64,
    pub std: f64,
    pub ur: f64,
    pub hist: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct CurvePoint {
    pub minute: f64,
    /// Mean |offset| in ms of all hits in the time bin.
    pub raw: f64,
    /// Mean of |offset| / (mean |offset| of that hit's density band). 1.0 = typical for the density.
    pub norm: f64,
    pub samples: u32,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub keys: [KeyStat; 4],
    pub total_hits: u64,
    pub early_miss_clicks: u64,
    pub curve: Vec<CurvePoint>,
    /// Per density band: (mean |offset| ms, hits).
    pub bands: Vec<(f64, u64)>,
}

pub fn compute(hits: &[Hit], selected: &[bool], min_samples: u32) -> Report {
    let mut n = [0u64; 4];
    let mut s = [0f64; 4];
    let mut sq = [0f64; 4];
    let mut hist: [Vec<u32>; 4] = std::array::from_fn(|_| vec![0u32; NBINS]);
    let mut early = 0u64;
    let mut tb: Vec<[(f64, u32); NBANDS]> = Vec::new();
    let mut band_tot = [(0f64, 0u64); NBANDS];
    for h in hits {
        let sid = h.score_id as usize;
        if sid >= selected.len() || !selected[sid] || h.key == 0 || !h.offset_ms.is_finite() {
            continue;
        }
        if h.judgement == 0 {
            early += 1;
            continue;
        }
        let k = (h.key - 1) as usize;
        if k >= 4 {
            continue;
        }
        let o = h.offset_ms as f64;
        n[k] += 1;
        s[k] += o;
        sq[k] += o * o;
        let b = (((o + HIST_MAX) / HIST_BIN).floor().max(0.0) as usize).min(NBINS - 1);
        hist[k][b] += 1;
        if h.density.is_finite() && h.time_ms.is_finite() && h.time_ms >= 0.0 {
            let t = (h.time_ms as f64 / TIME_BIN_MS) as usize;
            if t < MAX_TIME_BINS {
                if tb.len() <= t {
                    tb.resize(t + 1, [(0.0, 0); NBANDS]);
                }
                let bd = band(h.density);
                let a = o.abs();
                tb[t][bd].0 += a;
                tb[t][bd].1 += 1;
                band_tot[bd].0 += a;
                band_tot[bd].1 += 1;
            }
        }
    }
    let bm: Vec<f64> =
        band_tot.iter().map(|(sum, c)| if *c > 0 { sum / *c as f64 } else { 0.0 }).collect();
    let mut curve = Vec::new();
    for (t, bands) in tb.iter().enumerate() {
        let cnt: u32 = bands.iter().map(|b| b.1).sum();
        if cnt == 0 || cnt < min_samples {
            continue;
        }
        let raw: f64 = bands.iter().map(|b| b.0).sum::<f64>() / cnt as f64;
        let norm: f64 = bands
            .iter()
            .enumerate()
            .filter(|(i, b)| b.1 > 0 && bm[*i] > 0.0)
            .map(|(i, b)| b.0 / bm[i])
            .sum::<f64>()
            / cnt as f64;
        curve.push(CurvePoint { minute: (t as f64 + 0.5) * TIME_BIN_MS / 60_000.0, raw, norm, samples: cnt });
    }
    let keys: [KeyStat; 4] = std::array::from_fn(|k| {
        let c = n[k] as f64;
        let (mean, std) = if n[k] > 0 {
            let m = s[k] / c;
            (m, (sq[k] / c - m * m).max(0.0).sqrt())
        } else {
            (0.0, 0.0)
        };
        KeyStat { hits: n[k], mean, std, ur: 10.0 * std, hist: hist[k].clone() }
    });
    Report {
        total_hits: n.iter().sum(),
        keys,
        early_miss_clicks: early,
        curve,
        bands: band_tot.iter().zip(&bm).map(|((_, c), m)| (*m, *c)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(sid: u32, t: f32, off: f32, dens: f32, key: u8, j: u16) -> Hit {
        Hit {
            score_id: sid, object_index: 0, time_ms: t, offset_ms: off, dx: 0.0, dy: 0.0, obj_x: 0.0,
            obj_y: 0.0, density: dens, spacing: 1.0, key, judgement: j,
        }
    }

    #[test]
    fn key_counts_sum_to_total_and_stats() {
        let hits = [
            hit(1, 0.0, -10.0, 1.0, 1, 300),
            hit(1, 0.0, 10.0, 1.0, 1, 300),
            hit(1, 0.0, 20.0, 1.0, 3, 100),
            hit(1, 0.0, -200.0, 1.0, 4, 0), // early-click miss: excluded
            hit(1, 0.0, f32::NAN, 1.0, 0, 0), // never clicked
            hit(2, 0.0, 5.0, 1.0, 2, 300), // play not selected
        ];
        let r = compute(&hits, &[false, true, false], 1);
        assert_eq!(r.keys.iter().map(|k| k.hits).sum::<u64>(), r.total_hits);
        assert_eq!(r.total_hits, 3);
        assert_eq!(r.early_miss_clicks, 1);
        assert_eq!(r.keys[0].hits, 2);
        assert!(r.keys[0].mean.abs() < 1e-9);
        assert!((r.keys[0].ur - 100.0).abs() < 1e-9);
        assert_eq!(r.keys[2].hits, 1);
        assert_eq!(r.keys[0].hist.iter().sum::<u32>(), 2);
    }

    #[test]
    fn normalization_changes_curve() {
        let hits = [
            hit(0, 1000.0, 10.0, 1.0, 3, 300),
            hit(0, 2000.0, -10.0, 1.0, 3, 300),
            hit(0, 31_000.0, 20.0, 1.0, 3, 300),
            hit(0, 32_000.0, -20.0, 1.0, 3, 300),
        ];
        let r = compute(&hits, &[true], 1);
        assert_eq!(r.curve.len(), 2);
        assert!((r.curve[0].raw - 10.0).abs() < 1e-9 && (r.curve[1].raw - 20.0).abs() < 1e-9);
        assert!((r.curve[0].norm - 10.0 / 15.0).abs() < 1e-9);
        assert!((r.curve[1].norm - 20.0 / 15.0).abs() < 1e-9);
    }

    #[test]
    fn density_mix_is_removed_by_normalization() {
        // Both time bins have the same per-density error; bin 1 has more dense notes.
        let mut hits = vec![];
        for (t, n_dense) in [(1000.0, 1), (31_000.0, 3)] {
            hits.push(hit(0, t, 10.0, 1.0, 3, 300));
            for _ in 0..n_dense {
                hits.push(hit(0, t, 30.0, 9.0, 3, 300));
            }
        }
        let r = compute(&hits, &[true], 1);
        assert!((r.curve[0].raw - r.curve[1].raw).abs() > 1.0);
        let n0 = r.curve[0].norm;
        assert!((n0 - 1.0).abs() < 0.2 && (r.curve[1].norm - 1.0).abs() < 0.2);
    }

    #[test]
    fn min_samples_hides_thin_bins_and_bands() {
        let hits = [hit(0, 1000.0, 10.0, 1.0, 3, 300)];
        assert!(compute(&hits, &[true], 2).curve.is_empty());
        assert_eq!(band(1.0), 0);
        assert_eq!(band(2.0), 1);
        assert_eq!(band(50.0), NBANDS - 1);
    }
}
