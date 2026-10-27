mod config;
mod data;
mod db;
mod heatmap_ui;
mod keys_ui;
mod replay;
mod scan;
mod sim;
mod stats;
mod ui;

use eframe::egui;
use std::sync::mpsc::{self, TryRecvError};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    Playstyle,
    Training,
    Analysis,
    Tablet,
    Data,
    Settings,
}

impl Tab {
    const ALL: [Tab; 7] = [
        Tab::Overview,
        Tab::Playstyle,
        Tab::Training,
        Tab::Analysis,
        Tab::Tablet,
        Tab::Data,
        Tab::Settings,
    ];

    fn name(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Playstyle => "Playstyle",
            Tab::Training => "Training",
            Tab::Analysis => "Analysis",
            Tab::Tablet => "Tablet",
            Tab::Data => "Data",
            Tab::Settings => "Settings",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AnalysisPage {
    Heatmaps,
    Keys,
}

enum State {
    Loading(mpsc::Receiver<Result<data::Dataset, String>>),
    Ready(Box<data::Dataset>),
    Failed(String),
}

struct App {
    current: Tab,
    analysis: AnalysisPage,
    state: State,
    filter: data::Filter,
    detail: Option<ui::Detail>,
    heat: heatmap_ui::HeatState,
    keys: keys_ui::KeysState,
}

impl App {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(data::load_all());
        });
        Self {
            current: Tab::Overview,
            analysis: AnalysisPage::Heatmaps,
            state: State::Loading(rx),
            filter: data::Filter::default(),
            detail: None,
            heat: heatmap_ui::HeatState::default(),
            keys: keys_ui::KeysState::default(),
        }
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let msg = if let State::Loading(rx) = &self.state { Some(rx.try_recv()) } else { None };
        match msg {
            Some(Ok(Ok(d))) => self.state = State::Ready(Box::new(d)),
            Some(Ok(Err(e))) => self.state = State::Failed(e),
            Some(Err(TryRecvError::Empty)) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            Some(Err(TryRecvError::Disconnected)) => self.state = State::Failed("loader thread stopped".into()),
            None => {}
        }

        egui::TopBottomPanel::top("tab_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                for tab in Tab::ALL {
                    ui.selectable_value(&mut self.current, tab, tab.name());
                }
            });
        });
        egui::TopBottomPanel::top("filter_bar").show(ctx, |ui| {
            ui::filter_bar(ui, &mut self.filter);
        });
        if let State::Ready(d) = &self.state {
            egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
                ui.small(format!(
                    "Loaded {} plays and {} hits in {:.2} s. PP computed for {} new plays in {:.1} s ({} failed). PP library: {}.",
                    d.plays.len(), d.hits.len(), d.load_seconds, d.pp.new, d.pp.seconds, d.pp.failed, data::PP_LIB
                ));
            });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(self.current.name());
            if self.current != Tab::Overview && self.current != Tab::Analysis {
                return;
            }
            match &self.state {
                State::Loading(_) => {
                    ui.label("Loading plays (computing PP for new plays can take a few seconds)...");
                }
                State::Failed(e) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, e);
                }
                State::Ready(d) => {
                    let idx = self.filter.apply(&d.plays, now_unix());
                    if self.current == Tab::Overview {
                        ui::overview(ui, d, &idx, &mut self.detail);
                    } else {
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.analysis, AnalysisPage::Heatmaps, "Heatmaps");
                            ui.selectable_value(&mut self.analysis, AnalysisPage::Keys, "Keys");
                        });
                        match self.analysis {
                            AnalysisPage::Heatmaps => heatmap_ui::show(ui, d, &idx, &self.filter, &mut self.heat),
                            AnalysisPage::Keys => keys_ui::show(ui, d, &idx, &self.filter, &mut self.keys),
                        }
                    }
                }
            }
        });
        if let State::Ready(d) = &self.state {
            ui::detail_window(ctx, d, &mut self.detail);
        }
    }
}

/// `cargo run -- scan`: index the lazer folder and print a summary.
fn run_scan() -> Result<(), String> {
    let cfg = config::load_or_create(&config::config_path())?;
    let lazer = config::resolve_lazer_path(&cfg, &config::default_candidates())
        .ok_or("lazer folder not found; set lazer_path in ~/.config/osu-analyzer/config.toml")?;
    let mut conn = db::open(&config::cache_db_path()).map_err(|e| e.to_string())?;
    let result = scan::scan(&mut conn, &lazer, &cfg)?;
    scan::print_summary(&conn, &result);
    Ok(())
}

/// `cargo run --release -- sim`: scan (incremental), simulate hits, print the report.
fn run_sim() -> Result<(), String> {
    let cfg = config::load_or_create(&config::config_path())?;
    let lazer = config::resolve_lazer_path(&cfg, &config::default_candidates())
        .ok_or("lazer folder not found; set lazer_path in ~/.config/osu-analyzer/config.toml")?;
    let mut conn = db::open(&config::cache_db_path()).map_err(|e| e.to_string())?;
    scan::scan(&mut conn, &lazer, &cfg)?;
    let stats = sim::run::run(&mut conn, &lazer)?;
    sim::run::print_report(&conn, &stats);
    Ok(())
}

/// `cargo run --release -- load`: the M03 startup path without a window; prints totals and timings.
fn run_load() -> Result<(), String> {
    let t0 = std::time::Instant::now();
    let d = data::load_all()?;
    let hours: f64 = d.plays.iter().map(|p| p.hours).sum();
    let passes = d.plays.iter().filter(|p| p.passed).count();
    let with_pp = d.plays.iter().filter(|p| p.pp.is_some()).count();
    println!("PP library:              {}", data::PP_LIB);
    println!("PP computed this run:    {} new, {} failed, {:.1} s", d.pp.new, d.pp.failed, d.pp.seconds);
    println!("Plays (kept replays):    {}", d.plays.len());
    println!("  passes / fails:        {} / {}", passes, d.plays.len() - passes);
    println!("  with PP:               {}", with_pp);
    println!("Hours played (approx.):  {:.1}", hours);
    println!("Hits in RAM:             {}", d.hits.len());
    println!("Load into RAM:           {:.2} s", d.load_seconds);
    println!("Startup total:           {:.2} s", t0.elapsed().as_secs_f64());
    Ok(())
}

fn main() -> eframe::Result {
    match std::env::args().nth(1).as_deref() {
        Some("scan") => {
            if let Err(e) = run_scan() {
                eprintln!("scan failed: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        Some("sim") => {
            if let Err(e) = run_sim() {
                eprintln!("sim failed: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        Some("load") => {
            if let Err(e) = run_load() {
                eprintln!("load failed: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        _ => {}
    }
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "osu! Replay Analyzer",
        options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}
