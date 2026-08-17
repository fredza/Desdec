use eframe::egui;

use crate::{
    app::{DesdecApp, WorkspaceView},
    commands::Command,
    i18n::Text,
    preferences::accent,
    ui::{ERROR, MUTED, section_title},
};

/// Fixed width: the menu is fully collapsible, so a drag handle would only add
/// visual noise.
const WIDTH: f32 = 276.0;
const CLOSE_BUTTON_SIZE: egui::Vec2 = egui::vec2(28.0, 28.0);
const PRIMARY_BUTTON_HEIGHT: f32 = 34.0;
const SECONDARY_BUTTON_HEIGHT: f32 = 30.0;

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.navigation_open {
        return;
    }

    egui::SidePanel::left("navigation")
        .resizable(false)
        .exact_width(WIDTH)
        .show(ctx, |ui| {
            header(app, ui);
            binary_actions(app, ctx, ui);

            ui.add_space(16.0);
            exploration_section(app, ui);

            ui.add_space(10.0);
            tools_section(app, ctx, ui);

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.separator();
                ui.small(app.t(Text::MenuHint));
            });
        });
}

fn header(app: &mut DesdecApp, ui: &mut egui::Ui) {
    ui.add_space(6.0);
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("D")
                    .color(accent(app.preferences.theme))
                    .strong()
                    .size(24.0),
            );
            ui.vertical(|ui| {
                ui.strong("Desdec");
                ui.small(egui::RichText::new(app.t(Text::Menu)).color(MUTED));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = app.tooltip(
                    ui.add(
                        egui::Button::new("×")
                            .frame(false)
                            .min_size(CLOSE_BUTTON_SIZE),
                    ),
                    app.t(Text::CollapseMenu),
                );
                if close.clicked() {
                    app.navigation_open = false;
                }
            });
        });
    });
    ui.add_space(10.0);
}

fn binary_actions(app: &mut DesdecApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let accent = accent(app.preferences.theme);
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.label(section_title(app.t(Text::OpenBinary)));
        ui.add_space(5.0);
        let open = ui.add_sized(
            [ui.available_width(), PRIMARY_BUTTON_HEIGHT],
            egui::Button::new(
                egui::RichText::new(app.t(Text::OpenBinary)).color(egui::Color32::WHITE),
            )
            .fill(accent.gamma_multiply(0.72)),
        );
        if open.clicked() {
            app.choose_binary(ctx);
        }

        if app.is_analysing() {
            ui.add_space(7.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.small(egui::RichText::new(app.t(Text::StatusWorking)).color(MUTED));
            });
            let cancel = ui.add_sized(
                [ui.available_width(), SECONDARY_BUTTON_HEIGHT],
                egui::Button::new(
                    egui::RichText::new(app.t(Text::CancelAnalysis)).color(egui::Color32::WHITE),
                )
                .fill(ERROR.gamma_multiply(0.78)),
            );
            if cancel.clicked() {
                app.cancel_analysis();
            }
        }

        if app.analysis.is_some() {
            ui.add_space(7.0);
            let close = ui.add_sized(
                [ui.available_width(), SECONDARY_BUTTON_HEIGHT],
                egui::Button::new(app.t(Text::CloseBinary)),
            );
            if close.clicked() {
                app.close_binary();
                app.navigation_open = false;
            }
        }
    });
    recent_binaries(app, ctx, ui);
}

fn recent_binaries(app: &mut DesdecApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let recent = app.recent_binaries().to_vec();
    if recent.is_empty() {
        return;
    }

    ui.add_space(6.0);
    egui::CollapsingHeader::new(section_title(app.t(Text::RecentBinaries)))
        .id_salt("navigation.recent_binaries")
        .default_open(true)
        .show(ui, |ui| {
            let mut selected = None;
            for path in recent {
                let label = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                let response = ui
                    .add_sized(
                        [ui.available_width(), SECONDARY_BUTTON_HEIGHT],
                        egui::Button::new(label).truncate(),
                    )
                    .on_hover_text(path.display().to_string());
                if response.clicked() {
                    selected = Some(path);
                }
            }
            if ui.button(app.t(Text::ClearRecentBinaries)).clicked() {
                app.clear_recent_binaries();
            }
            if let Some(path) = selected {
                app.open_recent_binary(ctx, path);
                app.navigation_open = false;
            }
        });
}

fn exploration_section(app: &mut DesdecApp, ui: &mut egui::Ui) {
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        egui::CollapsingHeader::new(section_title(app.t(Text::Exploration)))
            .id_salt("navigation.exploration")
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(4.0);
                for view in WorkspaceView::ALL {
                    let selected = app.active_view == *view;
                    let label = format!("{}  {}", view.icon(), app.t(view.text()));
                    if ui
                        .add_sized(
                            [ui.available_width(), SECONDARY_BUTTON_HEIGHT],
                            egui::SelectableLabel::new(selected, label),
                        )
                        .clicked()
                    {
                        app.select_view(*view);
                    }
                }
            });
    });
}

fn tools_section(app: &mut DesdecApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        egui::CollapsingHeader::new(section_title(app.t(Text::Tools)))
            .id_salt("navigation.tools")
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(4.0);
                for (label, command) in [
                    (Text::CommandPalette, Command::CommandPalette),
                    (Text::Preferences, Command::Preferences),
                    (Text::About, Command::About),
                ] {
                    if ui
                        .add_sized(
                            [ui.available_width(), SECONDARY_BUTTON_HEIGHT],
                            egui::Button::new(app.t(label)),
                        )
                        .clicked()
                    {
                        app.run_command(ctx, command);
                        app.navigation_open = false;
                    }
                }
            });
    });
}
