//! Native entry point of the Desdec binary explorer.

use eframe::egui;

mod app;
mod commands;
mod i18n;
mod icons;
mod libraries;
mod patches;
mod preferences;
mod ui;

use app::DesdecApp;

const INITIAL_SIZE: [f32; 2] = [1120.0, 720.0];
const MINIMUM_SIZE: [f32; 2] = [860.0, 560.0];

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(INITIAL_SIZE)
            .with_min_inner_size(MINIMUM_SIZE)
            // Names the storage directory holding the saved preferences, so it
            // no longer depends on the translated window title.
            .with_app_id("Desdec"),
        ..Default::default()
    };
    eframe::run_native(
        "Desdec",
        options,
        Box::new(|creation_context| Ok(Box::new(DesdecApp::new(creation_context)))),
    )
}
