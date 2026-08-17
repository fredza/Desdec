//! Optional static scans using the locally installed YARA command.

use eframe::egui;

use crate::{
    app::DesdecApp,
    i18n::Text,
    ui::{ERROR, MUTED, card},
};

pub fn show(app: &mut DesdecApp, ui: &mut egui::Ui) {
    if !app.preferences.yara_enabled {
        ui.label(egui::RichText::new(app.t(Text::YaraDisabled)).color(MUTED));
        return;
    }

    card(ui, app.t(Text::Yara), |ui| {
        ui.small(app.t(Text::YaraInfo));
        ui.add_space(8.0);
        let configured = !app.preferences.yara_rules_path.trim().is_empty();
        let scan = ui.add_enabled(
            configured && !app.yara.running,
            egui::Button::new(app.t(Text::RunYara)),
        );
        if scan.clicked() {
            app.request_yara_scan(ui.ctx());
        }
        if !configured {
            ui.small(egui::RichText::new(app.t(Text::YaraNotConfigured)).color(MUTED));
            return;
        }
        if app.yara.running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(app.t(Text::YaraScanning));
            });
            return;
        }
        if let Some(error) = &app.yara.error {
            ui.colored_label(ERROR, format!("{} {error}", app.t(Text::YaraFailed)));
            return;
        }
        if app.yara.matches.is_empty() {
            ui.small(app.t(Text::YaraNoMatches));
            return;
        }

        ui.strong(app.t(Text::YaraMatches));
        for matched in &app.yara.matches {
            let label = matched.namespace.as_ref().map_or_else(
                || matched.rule.clone(),
                |namespace| format!("{namespace}:{}", matched.rule),
            );
            ui.monospace(label);
        }
    });
}
