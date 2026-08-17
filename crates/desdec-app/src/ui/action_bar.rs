use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog, WorkspaceView},
    commands::Command,
    i18n::Text,
    icons::{self, Icon},
    preferences::accent,
};

const HEIGHT: f32 = 48.0;
const HAMBURGER_SIZE: egui::Vec2 = egui::vec2(34.0, 30.0);
const CLOSE_BUTTON_SIZE: egui::Vec2 = egui::vec2(26.0, 26.0);

/// Views the toolbar offers directly, in reading order.
///
/// Each draws the icon and runs the command the view itself declares, so the
/// toolbar can never come to name a view differently from anywhere else. The
/// menu reads this list too, and leaves out whatever is already here: a view
/// one click away in the toolbar does not need a second entry three rows down
/// in the menu.
pub const VIEW_ACTIONS: &[WorkspaceView] = &[
    WorkspaceView::Overview,
    WorkspaceView::Disassembly,
    WorkspaceView::Decompile,
    WorkspaceView::Functions,
    WorkspaceView::Strings,
    WorkspaceView::Patches,
];

/// Right-aligned actions, drawn right to left in this order.
const ACTIONS: &[(Icon, Command)] = &[
    (Icon::Palette, Command::CommandPalette),
    (Icon::Open, Command::OpenBinary),
];

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("action_bar")
        .exact_height(HEIGHT)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                // Drawn rather than typed: the three-bar character is missing
                // from some system fonts and rendered as a blank box there.
                let hamburger = app.tooltip(
                    icons::sized_button(
                        ui,
                        Icon::Menu,
                        None,
                        app.navigation_open,
                        accent(app.preferences.theme),
                        HAMBURGER_SIZE,
                    ),
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
        icons::sized_button(
            ui,
            Icon::Close,
            None,
            false,
            accent(app.preferences.theme),
            CLOSE_BUTTON_SIZE,
        ),
        &app.command_tooltip(Command::CloseBinary),
    );
    if close.clicked() {
        app.close_binary();
    }
}

fn toolbar(app: &mut DesdecApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let accent = accent(app.preferences.theme);

    for view in VIEW_ACTIONS {
        let command = view.command();
        let selected = app.active_view == *view;
        if icons::button(
            ui,
            view.glyph(),
            app.optional_command_tooltip(command),
            selected,
            accent,
        )
        .clicked()
        {
            app.run_command(ctx, command);
        }
    }

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        for (icon, command) in ACTIONS {
            if icons::button(
                ui,
                *icon,
                app.optional_command_tooltip(*command),
                matches!(command, Command::CommandPalette)
                    && app.dialogs.is_open(Dialog::CommandPalette),
                accent,
            )
            .clicked()
            {
                app.run_command(ctx, *command);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::WorkspaceView,
        testing::{opened_app, window_input},
    };

    /// The right-hand actions are actions, not views: a view there would be
    /// offered twice over, since the row to their left already carries every
    /// view the toolbar shows.
    #[test]
    fn the_trailing_actions_are_not_views() {
        for (_, command) in ACTIONS {
            assert!(
                command.opens_view().is_none(),
                "{command:?} opens a view, and the toolbar already lists those"
            );
        }
    }

    /// With a binary open and the toolbar on, the bar still draws: the name of
    /// the file, the way to close it, and the actions.
    #[test]
    fn the_bar_draws_with_a_binary_open() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Overview);
        app.preferences.show_toolbar = true;

        let output = ctx.run(window_input(), |ctx| show(&mut app, ctx));

        let drawn = crate::testing::drawn_text(&output.shapes);
        assert!(drawn.contains("Desdec"), "the bar names the application");
        assert!(
            !output.shapes.is_empty(),
            "the bar must draw with a binary open"
        );
    }
}
