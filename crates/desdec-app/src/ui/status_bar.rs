use eframe::egui;

use crate::{
    app::DesdecApp,
    i18n::Text,
    preferences::{accent, success},
    ui::{ERROR, format_size},
};

const HEIGHT: f32 = 28.0;

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(HEIGHT)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                // While a file is being chosen or analysed, the activity is the
                // whole status: announcing a file that is not loaded yet, or a
                // readiness the application does not have, would contradict it.
                if app.is_busy() {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new(app.t(Text::StatusWorking))
                            .color(accent(app.preferences.theme)),
                    );
                    return;
                }

                if app.error.is_some() {
                    ui.label(egui::RichText::new(app.t(Text::StatusFailed)).color(ERROR));
                } else {
                    ui.label(egui::RichText::new("OK").color(success(app.preferences.theme)));
                }

                if let Some(summary) = app.analysis.as_ref().map(|analysis| &analysis.summary) {
                    ui.label(summary.format.label());
                    ui.separator();
                    ui.label(summary.architecture.label());
                    ui.separator();
                    ui.label(format_size(summary.size));
                } else {
                    ui.label(app.t(Text::ReadyToOpen));
                }
            });
        });
}
