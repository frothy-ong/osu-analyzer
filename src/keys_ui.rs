//! Analysis > Keys page (M05).

use crate::data::{Dataset, Filter};
use crate::stats::keys::{self, Report, HIST_BIN, HIST_MAX, KEY_NAMES, MIN_CURVE_SAMPLES, NBINS, TIME_BIN_MS};
use eframe::egui;
use egui_plot::{HLine, Legend, Line, Plot, PlotPoints, Points};

const COLORS: [egui::Color32; 4] = [
    egui::Color32::from_rgb(230, 90, 70),
    egui::Color32::from_rgb(70, 140, 240),
    egui::Color32::from_rgb(80, 200, 120),
    egui::Color32::from_rgb(240, 200, 60),
];

pub struct KeysState {
    normalize: bool,
    show: [bool; 4],
    cache: Option<(String, Report)>,
}

impl Default for KeysState {
    fn default() -> Self {
        Self { normalize: true, show: [true; 4], cache: None }
    }
}

pub fn show(ui: &mut egui::Ui, d: &Dataset, idx: &[usize], filter: &Filter, st: &mut KeysState) {
    let key = format!("{filter:?}|{}", d.hits.len());
    if st.cache.as_ref().is_none_or(|(k, _)| *k != key) {
        let max_id = d.plays.iter().map(|p| p.score_id).max().unwrap_or(0).max(0) as usize;
        let mut selected = vec![false; max_id + 1];
        for &i in idx {
            let id = d.plays[i].score_id;
            if id >= 0 {
                selected[id as usize] = true;
            }
        }
        st.cache = Some((key, keys::compute(&d.hits, &selected, MIN_CURVE_SAMPLES)));
    }
    let Some((_, rep)) = st.cache.as_ref() else { return };

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.strong("Hit timing per key");
        ui.small(format!(
            "{} hits (300/100/50 with a key) from {} plays. Offsets in real ms: negative = early, positive = late. \
             UR = 10 x std. Early-click misses ({}) are not included.",
            rep.total_hits, idx.len(), rep.early_miss_clicks
        ));
        ui.horizontal_wrapped(|ui| {
            for (k, name) in KEY_NAMES.iter().enumerate() {
                ui.checkbox(&mut st.show[k], *name);
            }
        });
        Plot::new("key_hist")
            .height(240.0)
            .legend(Legend::default())
            .x_axis_label("offset (ms)")
            .y_axis_label("% of that key's hits per 4 ms bin")
            .show(ui, |p| {
                for k in 0..4 {
                    let ks = &rep.keys[k];
                    if !st.show[k] || ks.hits == 0 {
                        continue;
                    }
                    let pts: Vec<[f64; 2]> = (0..NBINS)
                        .map(|i| [-HIST_MAX + (i as f64 + 0.5) * HIST_BIN, 100.0 * ks.hist[i] as f64 / ks.hits as f64])
                        .collect();
                    p.line(Line::new(PlotPoints::from(pts)).name(KEY_NAMES[k]).color(COLORS[k]).width(2.0_f32));
                }
            });

        egui::Grid::new("key_table").striped(true).show(ui, |ui| {
            for h in ["Key", "Hits", "Share %", "Mean offset (ms)", "Std (ms)", "UR"] {
                ui.strong(h);
            }
            ui.end_row();
            for k in 0..4 {
                let ks = &rep.keys[k];
                ui.label(KEY_NAMES[k]);
                ui.label(format!("{}", ks.hits));
                if rep.total_hits > 0 {
                    ui.label(format!("{:.1}", 100.0 * ks.hits as f64 / rep.total_hits as f64));
                } else {
                    ui.label("-");
                }
                if ks.hits > 0 {
                    ui.label(format!("{:+.2}", ks.mean));
                    ui.label(format!("{:.2}", ks.std));
                    ui.label(format!("{:.1}", ks.ur));
                } else {
                    ui.label("-");
                    ui.label("-");
                    ui.label("-");
                }
                ui.end_row();
            }
            ui.strong("Total");
            ui.strong(format!("{}", rep.total_hits));
            ui.end_row();
        });
        let sum: u64 = rep.keys.iter().map(|k| k.hits).sum();
        ui.small(format!("Check: per-key hits add up to {sum}; total hits {}. Replays recorded by lazer only carry M1/M2.", rep.total_hits));

        ui.add_space(10.0);
        ui.strong("Stamina curve: timing error vs elapsed time");
        ui.checkbox(&mut st.normalize, "Normalize by local note density");
        ui.small(format!(
            "Elapsed time = time into the map (real time), {} s bins; bins with fewer than {} hits are hidden. \
             Raw = mean |offset| in ms. Normalized = each hit's |offset| divided by the mean |offset| of all hits \
             in the same density band (objects per second within +-1 s: {}), then averaged per bin; 1.0 = typical for that density, \
             above 1.0 = worse than usual.",
            (TIME_BIN_MS / 1000.0) as u32,
            MIN_CURVE_SAMPLES,
            (0..keys::NBANDS).map(keys::band_label).collect::<Vec<_>>().join(", ")
        ));
        if rep.curve.is_empty() {
            ui.label("Not enough hits for the current filter.");
        } else {
            let norm = st.normalize;
            let pts: Vec<[f64; 2]> =
                rep.curve.iter().map(|c| [c.minute, if norm { c.norm } else { c.raw }]).collect();
            Plot::new("stamina")
                .height(240.0)
                .legend(Legend::default())
                .x_axis_label("elapsed time (minutes)")
                .y_axis_label(if norm { "error relative to density (1.0 = typical)" } else { "mean |offset| (ms)" })
                .show(ui, |p| {
                    p.line(Line::new(PlotPoints::from(pts.clone())).name(if norm { "normalized" } else { "raw" }).width(2.0_f32));
                    p.points(Points::new(PlotPoints::from(pts)).radius(3.0_f32));
                    if norm {
                        p.hline(HLine::new(1.0).name("typical (1.0)"));
                    }
                });
            egui::CollapsingHeader::new("Samples per time bin and density band").show(ui, |ui| {
                egui::Grid::new("curve_samples").striped(true).show(ui, |ui| {
                    for h in ["Minute", "Raw (ms)", "Normalized", "Samples"] {
                        ui.strong(h);
                    }
                    ui.end_row();
                    for c in &rep.curve {
                        ui.label(format!("{:.2}", c.minute));
                        ui.label(format!("{:.2}", c.raw));
                        ui.label(format!("{:.3}", c.norm));
                        ui.label(format!("{}", c.samples));
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
                egui::Grid::new("band_samples").striped(true).show(ui, |ui| {
                    for h in ["Density band (obj/s)", "Mean |offset| (ms)", "Hits"] {
                        ui.strong(h);
                    }
                    ui.end_row();
                    for (b, (m, n)) in rep.bands.iter().enumerate() {
                        ui.label(keys::band_label(b));
                        ui.label(format!("{m:.2}"));
                        ui.label(format!("{n}"));
                        ui.end_row();
                    }
                });
            });
        }
    });
}
