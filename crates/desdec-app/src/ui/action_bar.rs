use eframe::egui;

use crate::{
    app::{DesdecApp, WorkspaceView},
    commands::Command,
    i18n::Text,
    icons::{self, Icon},
    preferences::accent,
};

const HEIGHT: f32 = 48.0;
const HAMBURGER_SIZE: egui::Vec2 = egui::vec2(34.0, 30.0);
const CLOSE_BUTTON_SIZE: egui::Vec2 = egui::vec2(26.0, 26.0);

/// Toolbar actions that switch the workspace view.
const VIEW_ACTIONS: &[(Icon, Command, WorkspaceView)] = &[
    (Icon::Overview, Command::Overview, WorkspaceView::Overview),
    (
        Icon::Disassembly,
        Command::Disassembly,
        WorkspaceView::Disassembly,
    ),
    (
        Icon::Decompile,
        Command::Decompile,
        WorkspaceView::Decompile,
    ),
    (
        Icon::Functions,
        Command::Functions,
        WorkspaceView::Functions,
    ),
    (Icon::Strings, Command::Strings, WorkspaceView::Strings),
    (Icon::Patches, Command::Patches, WorkspaceView::Patches),
];

/// Right-aligned actions, drawn right to left in this order.
const TRAILING_ACTIONS: &[(Icon, Command)] = &[
    (Icon::Palette, Command::CommandPalette),
    (Icon::Open, Command::OpenBinary),
];

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("action_bar")
        .exact_height(HEIGHT)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let hamburger = app.tooltip(
                    ui.add(egui::Button::new("☰").min_size(HAMBURGER_SIZE).frame(false)),
                    app.t(Text::Menu),
                );
                if hamburger.clicked() {
                    app.navigation_open = !app.navigation_open;
                }

                ui.separator();
                ui.label(
                    egui::RichText::new("D")
                        .color(accent(app.preferences.theme))
                        .strong()
                        .size(20.0),
                );
                ui.strong("Desdec");
                // The open file, and the way to close it. Closing used to live
                // only in the collapsed side menu, which made it look as if a
                // binary could not be closed at all.
                open_binary(app, ui);

                if app.preferences.show_toolbar {
                    ui.separator();
                    toolbar(app, ctx, ui);
                }
            });
        });
}

/// Name of the loaded binary, with the button that closes it.
fn open_binary(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let Some(name) = app.analysis.as_ref().map(|analysis| {
        analysis.summary.path.file_name().map_or_else(
            || analysis.summary.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }) else {
        return;
    };

    ui.separator();
    let active_file = app.t(Text::ActiveFile);
    app.tooltip(ui.label(egui::RichText::new(name).monospace()), active_file);
    let close = app.tooltip(
        ui.add(egui::Button::new("×").min_size(CLOSE_BUTTON_SIZE)),
        &app.command_tooltip(Command::CloseBinary),
    );
    if close.clicked() {
        app.close_binary();
    }
}

fn toolbar(app: &mut DesdecApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let accent = accent(app.preferences.theme);

    for (icon, command, view) in VIEW_ACTIONS {
        let selected = app.active_view == *view;
        if icons::button(
            ui,
            *icon,
            app.optional_command_tooltip(*command),
            selected,
            accent,
        )
        .clicked()
        {
            app.run_command(ctx, *command);
        }
    }

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        for (icon, command) in TRAILING_ACTIONS {
            if icons::button(
                ui,
                *icon,
                app.optional_command_tooltip(*command),
                matches!(command, Command::CommandPalette) && app.dialogs.command_palette,
                accent,
            )
            .clicked()
            {
                app.run_command(ctx, *command);
            }
        }
    });
}
