//! M04 heatmap math. Hits are binned once per filter change on a 128x96 grid
//! (4 osu!px per bin) storing sums and counts per judgement. Smoothing is a
//! box window computed from a summed-area table at display time, so changing
//! the slider or the 300/100/50/miss toggles never rescans the hits.
//! Window sums are true sample counts, so every ratio is an exact mean.

use crate::data::Hit;

pub const W: usize = 128;
pub const H: usize = 96;
/// osu!px per bin (512 / 128 and 384 / 96).
pub const CELL: f64 = 4.0;
const N: usize = W * H;

// Channels stored per bin.
const C_OBJ: usize = 0; // objects (by object position)
const C_OFFN: usize = 1; // objects that have an offset (were clicked)
const C_OFF: usize = 2; // sum of offset_ms
const C_OFFSQ: usize = 3; // sum of offset_ms^2
const C_DX: usize = 4; // sum of cursor dx at object time (osu!px)
const C_DY: usize = 5; // sum of cursor dy at object time (osu!px, +y = down)
const C_CLICK: usize = 6; // clicks (by cursor position)
const C_ACC: usize = 7; // sum of accuracy weights (aggregate only)
const NSTORED: usize = 7;
const NCH: usize = 8;

/// Accuracy weight per judgement layer: 300, 100, 50, miss.
const ACC_W: [f64; 4] = [1.0, 1.0 / 3.0, 1.0 / 6.0, 0.0];

/// Directional bias colour is fully saturated at this many osu!px.
pub const BIAS_SCALE: f32 = 10.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Accuracy,
    ClickDensity,
    MeanOffset,
    Ur,
    Bias,
}

impl Mode {
    pub const ALL: [Mode; 5] = [Mode::Accuracy, Mode::ClickDensity, Mode::MeanOffset, Mode::Ur, Mode::Bias];
    pub fn label(self) -> &'static str {
        match self {
            Mode::Accuracy => "Accuracy %",
            Mode::ClickDensity => "Click density (clicks per object)",
            Mode::MeanOffset => "Mean offset (ms)",
            Mode::Ur => "Unstable rate",
            Mode::Bias => "Directional bias",
        }
    }
}

fn bin(x: f64, y: f64) -> Option<usize> {
    if !(0.0..512.0).contains(&x) || !(0.0..384.0).contains(&y) {
        return None;
    }
    Some((y / CELL) as usize * W + (x / CELL) as usize)
}

/// Raw per-judgement bins. layers[j][channel][bin], j: 0=300 1=100 2=50 3=miss.
pub struct Bins {
    layers: [Vec<Vec<f64>>; 4],
    pub hits_used: usize,
}

impl Bins {
    /// `selected[score_id]` says whether that play passes the global filter.
    pub fn build(hits: &[Hit], selected: &[bool]) -> Bins {
        let mut layers: [Vec<Vec<f64>>; 4] =
            std::array::from_fn(|_| (0..NSTORED).map(|_| vec![0.0; N]).collect());
        let mut used = 0usize;
        for h in hits {
            let sid = h.score_id as usize;
            if sid >= selected.len() || !selected[sid] {
                continue;
            }
            let Some(b) = bin(h.obj_x as f64, h.obj_y as f64) else { continue };
            used += 1;
            let j = match h.judgement {
                300 => 0,
                100 => 1,
                50 => 2,
                _ => 3,
            };
            let l = &mut layers[j];
            l[C_OBJ][b] += 1.0;
            let cursor_ok = h.dx.is_finite() && h.dy.is_finite();
            if cursor_ok {
                l[C_DX][b] += h.dx as f64;
                l[C_DY][b] += h.dy as f64;
            }
            if h.offset_ms.is_finite() {
                let o = h.offset_ms as f64;
                l[C_OFFN][b] += 1.0;
                l[C_OFF][b] += o;
                l[C_OFFSQ][b] += o * o;
            }
            if h.key != 0 && cursor_ok {
                if let Some(cb) = bin(h.obj_x as f64 + h.dx as f64, h.obj_y as f64 + h.dy as f64) {
                    l[C_CLICK][cb] += 1.0;
                }
            }
        }
        Bins { layers, hits_used: used }
    }

    /// Sum the layers ticked in `show` (300, 100, 50, miss).
    pub fn aggregate(&self, show: [bool; 4]) -> Vec<Vec<f64>> {
        let mut a: Vec<Vec<f64>> = (0..NCH).map(|_| vec![0.0; N]).collect();
        for (j, l) in self.layers.iter().enumerate() {
            if !show[j] {
                continue;
            }
            for c in 0..NSTORED {
                for i in 0..N {
                    a[c][i] += l[c][i];
                }
            }
            for i in 0..N {
                a[C_ACC][i] += l[C_OBJ][i] * ACC_W[j];
            }
        }
        a
    }
}

pub fn total_objects(agg: &[Vec<f64>]) -> u64 {
    agg[C_OBJ].iter().sum::<f64>() as u64
}

/// Sum of each (2r+1) x (2r+1) window, clipped at the edges.
fn box_sum(src: &[f64], r: usize) -> Vec<f64> {
    let s = W + 1;
    let mut sat = vec![0.0; s * (H + 1)];
    for y in 0..H {
        let mut row = 0.0;
        for x in 0..W {
            row += src[y * W + x];
            sat[(y + 1) * s + x + 1] = sat[y * s + x + 1] + row;
        }
    }
    let mut out = vec![0.0; N];
    for y in 0..H {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(H));
        for x in 0..W {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(W));
            out[y * W + x] = sat[y1 * s + x1] - sat[y0 * s + x1] - sat[y1 * s + x0] + sat[y0 * s + x0];
        }
    }
    out
}

fn smooth_sum(src: &[f64], r: usize) -> Vec<f64> {
    if r < 3 {
        return box_sum(src, r);
    }
    let r_sub = r / 3;
    let norm = ((2 * r_sub + 1) * (2 * r_sub + 1)) as f64;
    let target_w = (2 * r + 1) as f64;
    let scale = (target_w * target_w) / (norm * norm * norm);
    let p1 = box_sum(src, r_sub);
    let p2 = box_sum(&p1, r_sub);
    let p3 = box_sum(&p2, r_sub);
    p3.into_iter().map(|v| v * scale).collect()
}

pub struct Field {
    /// Mode value per bin; NaN = no data.
    pub value: Vec<f32>,
    /// Second value (bias: mean dy, osu!px, +y = down). NaN otherwise.
    pub value2: Vec<f32>,
    /// Samples inside the smoothing window.
    pub samples: Vec<f64>,
    /// Unsmoothed raw hits in this bin.
    pub raw_samples: Vec<f64>,
    /// Smoothing radius used to compute this field.
    pub radius: usize,
}

pub fn compute_field(agg: &[Vec<f64>], mode: Mode, radius: usize) -> Field {
    let b = |c: usize| smooth_sum(&agg[c], radius);
    let objs = b(C_OBJ);
    let objs = b(C_OBJ);
    let raw_samples = match mode {
        Mode::MeanOffset | Mode::Ur => agg[C_OFFN].clone(),
        _ => agg[C_OBJ].clone(),
    };
    let mut f = Field { value: vec![f32::NAN; N], value2: vec![f32::NAN; N], samples: vec![0.0; N], raw_samples, radius };
    match mode {
        Mode::Accuracy => {
            let acc = b(C_ACC);
            for i in 0..N {
                if objs[i] > 0.5 {
                    f.value[i] = (100.0 * acc[i] / objs[i]) as f32;
                    f.samples[i] = objs[i];
                }
            }
        }
        Mode::ClickDensity => {
            let clicks = b(C_CLICK);
            for i in 0..N {
                if objs[i] > 0.5 {
                    f.value[i] = (clicks[i] / objs[i]) as f32;
                    f.samples[i] = objs[i];
                }
            }
        }
        Mode::MeanOffset => {
            let (n, s) = (b(C_OFFN), b(C_OFF));
            for i in 0..N {
                if n[i] > 0.5 {
                    f.value[i] = (s[i] / n[i]) as f32;
                    f.samples[i] = n[i];
                }
            }
        }
        Mode::Ur => {
            let (n, s, sq) = (b(C_OFFN), b(C_OFF), b(C_OFFSQ));
            for i in 0..N {
                if n[i] > 1.5 {
                    let m = s[i] / n[i];
                    let var = (sq[i] / n[i] - m * m).max(0.0);
                    f.value[i] = (10.0 * var.sqrt()) as f32;
                    f.samples[i] = n[i];
                }
            }
        }
        Mode::Bias => {
            let (dx, dy) = (b(C_DX), b(C_DY));
            for i in 0..N {
                if objs[i] > 0.5 {
                    f.value[i] = (dx[i] / objs[i]) as f32;
                    f.value2[i] = (dy[i] / objs[i]) as f32;
                    f.samples[i] = objs[i];
                }
            }
        }
    }
    f
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn seq(t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    let (lo, mid, hi) = ([40.0, 40.0, 160.0], [40.0, 170.0, 140.0], [250.0, 230.0, 60.0]);
    if t < 0.5 { lerp(lo, mid, t * 2.0) } else { lerp(mid, hi, (t - 0.5) * 2.0) }
}

fn diverge(t: f32) -> [f32; 3] {
    let t = t.clamp(-1.0, 1.0);
    let grey = [235.0, 235.0, 235.0];
    if t < 0.0 { lerp(grey, [50.0, 90.0, 230.0], -t) } else { lerp(grey, [230.0, 60.0, 50.0], t) }
}

/// 2nd and 98th percentile of the values that have enough samples.
fn percentile_range(f: &Field, min: f64, abs: bool) -> (f32, f32) {
    let win_area = ((2 * f.radius + 1) * (2 * f.radius + 1)) as f64;
    let mut v: Vec<f32> = (0..N)
        .filter(|&i| (f.samples[i] / win_area) >= min && f.value[i].is_finite())
        .map(|i| if abs { f.value[i].abs() } else { f.value[i] })
        .collect();
    if v.is_empty() {
        return (0.0, 1.0);
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let lo = v[(v.len() as f32 * 0.02) as usize];
    let hi = v[((v.len() as f32 * 0.98) as usize).min(v.len() - 1)];
    (lo, if hi > lo { hi } else { lo + 1e-3 })
}

/// RGBA pixels (W*H*4) and a one-line legend. Bins below `min_samples` are faded.
pub fn colorize(f: &Field, mode: Mode, min_samples: f64) -> (Vec<u8>, String) {
    let (lo, hi) = match mode {
        Mode::MeanOffset => (0.0, percentile_range(f, min_samples, true).1),
        _ => percentile_range(f, min_samples, false),
    };
    let mut px = vec![0u8; N * 4];
    for i in 0..N {
        if f.samples[i] <= 0.0 || !f.value[i].is_finite() {
            continue;
        }
        let rgb = match mode {
            Mode::MeanOffset => diverge(f.value[i] / hi),
            Mode::Bias => {
                let r = 128.0 + 127.0 * (f.value[i] / BIAS_SCALE).clamp(-1.0, 1.0);
                let bl = 128.0 + 127.0 * (-f.value2[i] / BIAS_SCALE).clamp(-1.0, 1.0);
                [r, 128.0, bl]
            }
            _ => seq((f.value[i] - lo) / (hi - lo)),
        };
        let win_area = ((2 * f.radius + 1) * (2 * f.radius + 1)) as f64;
        let a = if (f.samples[i] / win_area) < min_samples { 45 } else { 255 };
        px[i * 4] = rgb[0] as u8;
        px[i * 4 + 1] = rgb[1] as u8;
        px[i * 4 + 2] = rgb[2] as u8;
        px[i * 4 + 3] = a;
    }
    let legend = match mode {
        Mode::Accuracy => format!("Colour scale: {lo:.1}% (dark blue) to {hi:.1}% (yellow). Mean accuracy of hits in the window (300=100%, 100=33%, 50=17%, miss=0%)."),
        Mode::ClickDensity => format!("Colour scale: {lo:.2} to {hi:.2} clicks per object (clicks divided by objects). Clicks are placed by cursor position at the object time."),
        Mode::MeanOffset => format!("Colour scale: 0 ms (grey) to ±{hi:.1} ms. Blue = early, red = late."),
        Mode::Ur => format!("Colour scale: UR {lo:.0} (dark blue) to {hi:.0} (yellow). UR = 10 x std of offset in ms."),
        Mode::Bias => format!("Full colour = ±{BIAS_SCALE:.0} osu!px from circle centre at offset 0 ms. Red = cursor right of centre, blue = above centre, grey = centred."),
    };
    (px, legend)
}

/// Hover text for bin `i`.
pub fn describe(f: &Field, mode: Mode, i: usize) -> String {
    let (bx, by) = (i % W, i / W);
    let head = format!(
        "x {:.0}-{:.0}, y {:.0}-{:.0} osu!px",
        bx as f64 * CELL, (bx + 1) as f64 * CELL, by as f64 * CELL, (by + 1) as f64 * CELL
    );
    let n = f.samples[i];
    let v = f.value[i];
    if n <= 0.0 || !v.is_finite() {
        return format!("{head}\nno data (samples: 0)");
    }
    let val = match mode {
        Mode::Accuracy => format!("accuracy {v:.1}%"),
        Mode::ClickDensity => format!("{v:.2} clicks per object"),
        Mode::MeanOffset => format!("mean offset {v:+.1} ms"),
        Mode::Ur => format!("UR {v:.0}"),
        Mode::Bias => format!("bias dx {v:+.1}, dy {:+.1} osu!px (+x right, +y down)", f.value2[i]),
    };
    let raw = f.raw_samples[i];
    format!("{head}\n{val}\nsamples: {raw:.0} (smoothed: {n:.0})")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(sid: u32, x: f32, y: f32, off: f32, dx: f32, dy: f32, key: u8, j: u16) -> Hit {
        Hit {
            score_id: sid, object_index: 0, time_ms: 0.0, offset_ms: off, dx, dy, obj_x: x, obj_y: y,
            density: 1.0, spacing: 1.0, key, judgement: j,
        }
    }
    const ALL: [bool; 4] = [true; 4];

    fn sel() -> Vec<bool> { vec![false, true] }

    #[test]
    fn bin_positions() {
        assert_eq!(bin(10.0, 10.0), Some(2 * W + 2));
        assert_eq!(bin(-1.0, 10.0), None);
        assert_eq!(bin(512.0, 10.0), None);
        assert_eq!(bin(f64::NAN, 1.0), None);
    }

    #[test]
    fn window_counts_are_true_counts() {
        let hits = [hit(1, 100.0, 100.0, 0.0, 0.0, 0.0, 3, 300)];
        let agg = Bins::build(&hits, &sel()).aggregate(ALL);
        let s = box_sum(&agg[C_OBJ], 2);
        let c = (100.0 / CELL) as usize;
        assert_eq!(s[c * W + c], 1.0);
        assert_eq!(s[(c + 2) * W + c - 2], 1.0);
        assert_eq!(s[(c + 3) * W + c], 0.0);
        assert_eq!(s.iter().filter(|v| **v > 0.0).count(), 25);
        let f = compute_field(&agg, Mode::Accuracy, 2);
        assert_eq!(f.samples[c * W + c], 1.0);
    }

    #[test]
    fn accuracy_is_mean_and_toggles_work() {
        let hits = [hit(1, 100.0, 100.0, 0.0, 0.0, 0.0, 3, 300), hit(1, 101.0, 101.0, 40.0, 0.0, 0.0, 3, 100)];
        let b = Bins::build(&hits, &sel());
        let i = (100.0 / CELL) as usize * W + (100.0 / CELL) as usize;
        let f = compute_field(&b.aggregate(ALL), Mode::Accuracy, 0);
        assert!((f.value[i] - 66.6667).abs() < 0.01);
        let f = compute_field(&b.aggregate([false, true, false, false]), Mode::Accuracy, 0);
        assert!((f.value[i] - 33.3333).abs() < 0.01);
        assert_eq!(total_objects(&b.aggregate([false, false, false, true])), 0);
    }

    #[test]
    fn ur_and_mean_offset() {
        let hits = [hit(1, 50.0, 50.0, -10.0, 0.0, 0.0, 3, 300), hit(1, 50.0, 50.0, 10.0, 0.0, 0.0, 3, 300)];
        let agg = Bins::build(&hits, &sel()).aggregate(ALL);
        let i = (50.0 / CELL) as usize * W + (50.0 / CELL) as usize;
        assert!((compute_field(&agg, Mode::Ur, 0).value[i] - 100.0).abs() < 1e-3);
        assert!(compute_field(&agg, Mode::MeanOffset, 0).value[i].abs() < 1e-6);
    }

    #[test]
    fn click_density_is_divided_by_objects() {
        // 2 objects in one bin; one clicked, one never clicked (key 0, no offset).
        let hits = [hit(1, 50.0, 50.0, 5.0, 0.0, 0.0, 3, 300), hit(1, 50.0, 50.0, f32::NAN, 0.0, 0.0, 0, 0)];
        let agg = Bins::build(&hits, &sel()).aggregate(ALL);
        let i = (50.0 / CELL) as usize * W + (50.0 / CELL) as usize;
        let f = compute_field(&agg, Mode::ClickDensity, 0);
        assert!((f.value[i] - 0.5).abs() < 1e-6);
        assert_eq!(f.samples[i], 2.0);
    }

    #[test]
    fn bias_mean_and_colour() {
        let hits = [hit(1, 50.0, 50.0, 0.0, 5.0, -5.0, 3, 300)];
        let agg = Bins::build(&hits, &sel()).aggregate(ALL);
        let i = (50.0 / CELL) as usize * W + (50.0 / CELL) as usize;
        let f = compute_field(&agg, Mode::Bias, 0);
        assert_eq!((f.value[i], f.value2[i]), (5.0, -5.0));
        let (px, _) = colorize(&f, Mode::Bias, 0.0);
        assert!(px[i * 4] > 128 && px[i * 4 + 2] > 128); // right and up
        // centred hits are neutral grey
        let agg = Bins::build(&[hit(1, 50.0, 50.0, 0.0, 0.0, 0.0, 3, 300)], &sel()).aggregate(ALL);
        let (px, _) = colorize(&compute_field(&agg, Mode::Bias, 0), Mode::Bias, 0.0);
        assert_eq!((px[i * 4], px[i * 4 + 2]), (128, 128));
    }

    #[test]
    fn low_sample_bins_fade() {
        let hits = [hit(1, 50.0, 50.0, 0.0, 0.0, 0.0, 3, 300)];
        let agg = Bins::build(&hits, &sel()).aggregate(ALL);
        let i = (50.0 / CELL) as usize * W + (50.0 / CELL) as usize;
        let f = compute_field(&agg, Mode::Accuracy, 0);
        assert_eq!(colorize(&f, Mode::Accuracy, 1.0).0[i * 4 + 3], 255);
        assert_eq!(colorize(&f, Mode::Accuracy, 2.0).0[i * 4 + 3], 45);
    }

    #[test]
    fn unselected_plays_are_ignored() {
        let hits = [hit(0, 50.0, 50.0, 0.0, 0.0, 0.0, 3, 300), hit(7, 50.0, 50.0, 0.0, 0.0, 0.0, 3, 300)];
        let b = Bins::build(&hits, &sel());
        assert_eq!(b.hits_used, 0);
    }
}
