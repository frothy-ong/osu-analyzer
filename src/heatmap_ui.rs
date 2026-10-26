//! Analysis > Heatmaps page (M04).

use crate::data::{Dataset, Filter};
use crate::stats::heatmap::{self, Bins, Field, Mode, CELL, H, W};
use eframe::egui;

const PX: f32 = 5.0; // screen pixels per bin

pub struct HeatState {
    mode: Mode,
    show: [bool; 4],
    smooth: u32,
    min_samples: u32,
    bins: Option<(String, Bins)>,
    cache: Option<(String, Field, egui::TextureHandle, String, u64)>,
}

impl Default for HeatState {
    fn default() -> Self {
        Self { mode: Mode::Accuracy, show: [true; 4], smooth: 10, min_samples: 1, bins: None, cache: None }
    }
}

pub fn show(ui: &mut egui::Ui, d: &Dataset, idx: &[usize], filter: &Filter, st: &mut HeatState) {
    // Re-bin only when the global filter (or the data) changes.
    let key = format!("{filter:?}|{}", d.hits.len());
    if st.bins.as_ref().is_none_or(|(k, _)| *k != key) {
        let max_id = d.plays.iter().map(|p| p.score_id).max().unwrap_or(0).max(0) as usize;
        let mut selected = vec![false; max_id + 1];
        for &i in idx {
            let id = d.plays[i].score_id;
            if id >= 0 {
                selected[id as usize] = true;
            }
        }
        st.bins = Some((key.clone(), Bins::build(&d.hits, &selected)));
        st.cache = None;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.strong("Heatmaps");
        ui.horizontal_wrapped(|ui| {
            for m in Mode::ALL {
                ui.selectable_value(&mut st.mode, m, m.label());
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Show judgements:");
            for (i, l) in ["300", "100", "50", "miss"].iter().enumerate() {
                ui.checkbox(&mut st.show[i], *l);
            }
        });
        ui.add(egui::Slider::new(&mut st.smooth, 0..=25).text("Smoothing (bins)"));
        let w = (2 * st.smooth + 1) as f64 * CELL;
        ui.small(format!("Smoothing window = {w:.0} x {w:.0} osu!px (one bin = {CELL:.0} osu!px)."));
        ui.add(egui::Slider::new(&mut st.min_samples, 0..=15).text("Minimum samples"));
        ui.small("Bins with fewer raw hits than this are faded out and excluded from the color scale to prevent isolated outlier notes from skewing the gradient.");

        let key_all = format!("{key}|{:?}|{:?}|{}|{}", st.mode, st.show, st.smooth, st.min_samples);
        if st.cache.as_ref().is_none_or(|c| c.0 != key_all) {
            let bins = &st.bins.as_ref().expect("bins built above").1;
            let agg = bins.aggregate(st.show);
            let total = heatmap::total_objects(&agg);
            let field = heatmap::compute_field(&agg, st.mode, st.smooth as usize);
            let (rgba, legend) = heatmap::colorize(&field, st.mode, st.min_samples as f64);
            let img = egui::ColorImage::from_rgba_unmultiplied([W, H], &rgba);
            let tex = ui.ctx().load_texture("heatmap", img, egui::TextureOptions::LINEAR);
            st.cache = Some((key_all, field, tex, legend, total));
        }
        let Some((_, field, tex, legend, total)) = st.cache.as_ref() else { return };

        ui.small(format!(
            "{total} hits shown from {} plays. Offsets in ms (real time), cursor in osu!px, Hard Rock un-flipped.",
            idx.len()
        ));
        ui.small(legend.as_str());

        let size = egui::vec2(W as f32 * PX, H as f32 * PX);
        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, egui::Color32::from_gray(24));
        ui.painter().image(
            tex.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        let hp = resp.hover_pos();
        let mode = st.mode;
        resp.on_hover_ui_at_pointer(|ui| {
            ui.set_min_width(220.0);
            if let Some(p) = hp {
                let bx = (((p.x - rect.min.x) / PX) as usize).min(W - 1);
                let by = (((p.y - rect.min.y) / PX) as usize).min(H - 1);
                ui.label(heatmap::describe(field, mode, by * W + bx));
            }
        });
    });
}
