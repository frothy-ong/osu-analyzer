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

fn main() -> eframe::Result {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "osu! Replay Analyzer",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}
