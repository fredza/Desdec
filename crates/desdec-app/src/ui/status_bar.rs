use eframe::egui;

use crate::{
    app::DesdecApp,
    i18n::Text,
    preferences::{accent, success},
    ui::{ERROR, MUTED, format_size},
};

const HEIGHT: f32 = 28.0;

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(HEIGHT)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                // An analysis actually running is the whole status: claiming a
                // readiness the application does not have would contradict it.
                // Waiting on the file dialog is not that — nothing is being
                // analysed until a file has been chosen.
                if app.is_analysing() {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new(app.t(Text::StatusWorking))
                            .color(accent(app.preferences.theme)),
                    );
                    return;
                }
                if app.is_choosing_file() && app.analysis.is_none() {
                    ui.label(egui::RichText::new(app.t(Text::StatusChoosing)).color(MUTED));
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
