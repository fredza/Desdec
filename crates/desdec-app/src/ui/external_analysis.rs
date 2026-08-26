//! The raw, complete report of the optional external analyzer.

use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::Text,
    ui::MUTED,
};

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::ExternalAnalysis) {
        return;
    }
    let mut open = true;
    let mut window = egui::Window::new(app.t(Text::ExternalAnalysis))
        .id(egui::Id::new("desdec.external-analysis"))
        .open(&mut open)
        .resizable(true)
        .default_width(780.0)
        .default_height(560.0);
    window = crate::ui::centred(
        window,
        ctx,
        app.dialogs.opening_step(Dialog::ExternalAnalysis).is_some(),
    );
    window.show(ctx, |ui| {
        if app.external_analysis.running {
            ui.label(app.t(Text::ExternalAnalyzerRunning));
        } else if let Some(error) = &app.external_analysis.error {
            ui.colored_label(crate::ui::ERROR, error);
        } else if let Some(report) = &app.external_analysis.report {
            ui.small(egui::RichText::new("JSON protocol v1").color(MUTED));
            ui.add_space(6.0);
            egui::ScrollArea::both().show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(report).monospace()).selectable(true));
            });
        } else {
            ui.label(app.t(Text::ExternalAnalyzerNoReport));
        }
    });
    app.dialogs.set(Dialog::ExternalAnalysis, open);
}
