use eframe::egui;

use crate::{app::DesdecApp, i18n::Text, preferences::success, ui::format_size};

const HEIGHT: f32 = 28.0;

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(HEIGHT)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("OK").color(success(app.preferences.theme)));
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
