//! egui pages. M03: filter bar, Overview, "plays behind this number" window.

use crate::data::{self, Dataset, Filter, Range, MOD_GROUPS, STAR_CAP};
use eframe::egui;
use egui_plot::{Bar, BarChart, GridMark, Plot, PlotPoints, Points};
use std::ops::RangeInclusive;

pub struct Detail {
    pub title: String,
    pub rows: Vec<usize>,
}

pub fn filter_bar(ui: &mut egui::Ui, f: &mut Filter) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Mods:");
        for (i, g) in MOD_GROUPS.iter().enumerate() {
            ui.checkbox(&mut f.mods_sel[i], g.0);
        }
        ui.checkbox(&mut f.mods_exact, "exact");
        ui.separator();
        egui::ComboBox::from_id_salt("range").selected_text(f.range.label()).show_ui(ui, |ui| {
            for r in Range::ALL {
                ui.selectable_value(&mut f.range, r, r.label());
            }
        });
        ui.separator();
        ui.label("Stars:");
        ui.add(egui::DragValue::new(&mut f.star_min).range(0.0..=STAR_CAP).speed(0.1));
        ui.label("to");
        ui.add(egui::DragValue::new(&mut f.star_max).range(0.0..=STAR_CAP).speed(0.1));
        ui.separator();
        ui.checkbox(&mut f.best_only, "Best per map only");
        ui.checkbox(&mut f.include_fails, "Include fails");
        if ui.button("Reset").clicked() {
            *f = Filter::default();
        }
    });
}

fn date_axis(m: GridMark, _r: &RangeInclusive<f64>) -> String {
    data::date_label(m.value.floor() as i64)
}

fn month_axis(m: GridMark, _r: &RangeInclusive<f64>) -> String {
    data::month_label(m.value.round() as i64)
}

fn open(detail: &mut Option<Detail>, title: String, rows: Vec<usize>) {
    *detail = Some(Detail { title, rows });
}

pub fn overview(ui: &mut egui::Ui, d: &Dataset, idx: &[usize], detail: &mut Option<Detail>) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        let n = idx.len();
        let hours: f64 = idx.iter().map(|&i| d.plays[i].hours).sum();
        let passes: Vec<usize> = idx.iter().copied().filter(|&i| d.plays[i].passed).collect();
        let fails: Vec<usize> = idx.iter().copied().filter(|&i| !d.plays[i].passed).collect();
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("Plays: {n}")).clicked() {
                open(detail, format!("All plays ({n})"), idx.to_vec());
            }
            if ui.button(format!("Hours played: {hours:.1}")).clicked() {
                open(detail, format!("Plays behind {hours:.1} hours ({n} plays)"), idx.to_vec());
            }
            if ui.button(format!("Passes: {}", passes.len())).clicked() {
                open(detail, format!("Passes ({})", passes.len()), passes.clone());
            }
            if ui.button(format!("Fails/quits: {}", fails.len())).clicked() {
                open(detail, format!("Fails/quits ({})", fails.len()), fails.clone());
            }
        });
        ui.small("Hours = time up to each play's last judged object, in real time (an estimate). PP = recomputed with one pinned rosu-pp; fails have no PP.");
        ui.add_space(6.0);

        ui.strong("PP per play over time (osu!pp, each dot is one play)");
        let pts: Vec<[f64; 2]> = idx
            .iter()
            .filter_map(|&i| d.plays[i].pp.map(|pp| [d.plays[i].played_at as f64 / 86_400.0, pp]))
            .collect();
        if pts.is_empty() {
            ui.label("No PP data for the current filter.");
        } else {
            Plot::new("pp_time").height(220.0).x_axis_formatter(date_axis).show(ui, |p| {
                p.points(Points::new(PlotPoints::from(pts)).radius(2.0_f32).name("pp"));
            });
        }

        ui.add_space(6.0);
        ui.strong("Plays per month (count)");
        let months = data::plays_per_month(&d.plays, idx);
        if months.is_empty() {
            ui.label("No plays for the current filter.");
        } else {
            let bars: Vec<Bar> = months.iter().map(|(k, c)| Bar::new(*k as f64, *c as f64).width(0.8)).collect();
            Plot::new("per_month").height(200.0).x_axis_formatter(month_axis).show(ui, |p| {
                p.bar_chart(BarChart::new(bars).name("plays"));
            });
        }

        ui.add_space(6.0);
        ui.strong("Accuracy over time (%, each dot is one play)");
        let acc: Vec<[f64; 2]> = idx.iter().map(|&i| [d.plays[i].played_at as f64 / 86_400.0, d.plays[i].acc]).collect();
        if acc.is_empty() {
            ui.label("No plays for the current filter.");
        } else {
            Plot::new("acc_time").height(220.0).x_axis_formatter(date_axis).show(ui, |p| {
                p.points(Points::new(PlotPoints::from(acc)).radius(2.0_f32).name("accuracy %"));
            });
        }

        ui.add_space(6.0);
        ui.strong("Mod usage (click a count to see the plays)");
        for (name, rows) in data::mod_usage(&d.plays, idx) {
            ui.horizontal(|ui| {
                ui.label(format!("{name:<12}"));
                let c = rows.len();
                if ui.button(format!("{c}")).clicked() {
                    open(detail, format!("{name} plays ({c})"), rows);
                }
            });
        }
    });
}

pub fn detail_window(ctx: &egui::Context, d: &Dataset, detail: &mut Option<Detail>) {
    let Some(det) = detail.as_ref() else { return };
    let mut is_open = true;
    egui::Window::new(det.title.clone()).open(&mut is_open).default_size([760.0, 420.0]).show(ctx, |ui| {
        ui.label(format!("{} plays", det.rows.len()));
        egui::ScrollArea::vertical().show_rows(ui, 18.0, det.rows.len(), |ui, range| {
            for r in range {
                let p = &d.plays[det.rows[r]];
                let pp = p.pp.map(|v| format!("{v:.0}pp")).unwrap_or_else(|| "-".into());
                ui.label(format!(
                    "{}  {:<8} {:6.2}%  {:>6}  {}{}",
                    data::date_label(p.played_at.div_euclid(86_400)),
                    crate::sim::mods_to_string(p.mods),
                    p.acc,
                    pp,
                    p.title,
                    if p.passed { "" } else { "  (fail)" }
                ));
            }
        });
    });
    if !is_open {
        *detail = None;
    }
}
