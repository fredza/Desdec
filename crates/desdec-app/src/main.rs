//! Native entry point of the Desdec binary explorer.

use eframe::egui;

mod annotations;
mod app;
mod commands;
mod i18n;
mod icons;
mod journal;
mod libraries;
mod patches;
mod preferences;
mod search;
#[cfg(test)]
mod testing;
mod ui;
mod walk;
mod xrefs;

use app::DesdecApp;

const INITIAL_SIZE: [f32; 2] = [1120.0, 720.0];
const MINIMUM_SIZE: [f32; 2] = [860.0, 560.0];

fn main() -> eframe::Result<()> {
    // First of all, and before anything starts a thread: the local time
    // offset can only be read while this process has one thread, and the
    // session's account is stamped with it. See
    // [`journal::capture_local_offset`].
    journal::capture_local_offset();

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
