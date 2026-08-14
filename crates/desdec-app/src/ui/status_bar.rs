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

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mode = app.tooltip(ui.button(app.mode_label()), app.analysis_mode_label());
                    if mode.clicked() {
                        app.expert_mode = !app.expert_mode;
                    }
                    // Patch state belongs here once patches exist, and only
                    // when there are any: a permanent "no patches applied"
                    // reports on a feature that does not exist yet, and would
                    // keep saying nothing once it does.
                });
            });
        });
}
