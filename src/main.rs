mod config;
mod db;
mod replay;
mod scan;
mod sim;

use eframe::egui;

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

struct App {
    current: Tab,
}

impl Default for App {
    fn default() -> Self {
        Self {
            current: Tab::Overview,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("tab_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                for tab in Tab::ALL {
                    ui.selectable_value(&mut self.current, tab, tab.name());
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(self.current.name());
        });
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
        _ => {}
    }
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "osu! Replay Analyzer",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}
