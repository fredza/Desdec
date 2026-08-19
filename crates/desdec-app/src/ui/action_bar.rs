use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog, WorkspaceView},
    commands::Command,
    i18n::Text,
    icons::{self, Icon},
    preferences::accent,
    ui::flags,
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
                // The name is the way to About: an application's own identity
                // is where a reader looks for its version, its source and its
                // licence, and it was previously only reachable through the
                // menu or a function key.
                if name(app, ui).clicked() {
                    app.dialogs.open(Dialog::About);
                }
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

/// The mark and the name, as one button: two labels side by side would be two
/// things to click, one of which would do nothing.
fn name(app: &DesdecApp, ui: &mut egui::Ui) -> egui::Response {
    let mut job = egui::text::LayoutJob::default();
    let colour = ui.visuals().text_color();
    job.append(
        "D",
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(20.0),
            color: accent(app.preferences.theme),
            ..egui::TextFormat::default()
        },
    );
    job.append(
        " Desdec",
        0.0,
        egui::TextFormat {
            font_id: egui::TextStyle::Body.resolve(ui.style()),
            color: colour,
            ..egui::TextFormat::default()
        },
    );
    let response = ui
        .add(egui::Button::new(job).frame(false))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    app.tooltip(response, app.t(Text::About))
}

/// Longest file name shown in full.
///
/// The bar is a fixed row shared with the views and the actions; a name is the
/// one thing in it whose length the application does not choose, and a deeply
/// named file used to push the toolbar off the end of the window.
const NAME_LIMIT: usize = 28;

/// Name of the loaded binary, with the button that closes it.
fn open_binary(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let Some((name, full_path)) = app.analysis.as_ref().map(|analysis| {
        let path = &analysis.summary.path;
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        (name, path.display().to_string())
    }) else {
        return;
    };

    ui.separator();
    // Truncated on characters rather than bytes: a name is whatever the file
    // system holds, and cutting a multi-byte one mid-character would panic.
    let shown = if name.chars().count() > NAME_LIMIT {
        let kept: String = name.chars().take(NAME_LIMIT - 1).collect();
        format!("{kept}…")
    } else {
        name
    };

    // Clickable, and drawn as such: the full path is the thing a reader
    // actually needs to paste elsewhere, and it is too long to live in the
    // bar. Hovering says where it goes, so the click is never a guess.
    //
    // The highlight is reserved before the text and filled in after: its size
    // is the laid-out label's, and painted in call order it would cover the
    // name it is meant to pick out.
    let highlight = ui.painter().add(egui::Shape::Noop);
    let label = ui
        .add(egui::Label::new(egui::RichText::new(shown).monospace()).sense(egui::Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if label.hovered() {
        ui.painter().set(
            highlight,
            egui::epaint::RectShape::filled(
                label.rect.expand2(egui::vec2(4.0, 2.0)),
                3.0,
                ui.visuals().widgets.hovered.weak_bg_fill,
            ),
        );
    }
    let hint = format!(
        "{}\n{}\n{}",
        app.t(Text::ActiveFile),
        full_path,
        app.t(Text::CopyFullPath)
    );
    if label.on_hover_text(hint).clicked() {
        app.copy_to_clipboard(ui.ctx(), &full_path, Text::PathCopied);
    }
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

    // The condition flags of the selected instruction, following the listing
    // row by row. Drawn in the bar's own direction rather than among the
    // right-aligned actions, where they would have come out back to front, and
    // only while there is room for them and for the actions after them: the
    // bar is one fixed row, and a narrow window must not lose the palette.
    if ui.available_width() > flags::NEEDED_WIDTH {
        ui.separator();
        flags::show(app, ui);
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

    /// Clicking the application's own name opens About — where its version,
    /// its source and its licence are. That is where a reader looks for them,
    /// and they were otherwise a menu entry or a function key away.
    #[test]
    fn clicking_the_name_opens_about() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Overview);
        assert!(!app.dialogs.is_open(Dialog::About));

        // A first frame to find out where the name landed, then a click on it.
        let output = ctx.run(window_input(), |ctx| show(&mut app, ctx));
        let (_, at) = crate::testing::drawn(&output.shapes)
            .into_iter()
            .find(|(text, _)| text.contains("Desdec"))
            .expect("the bar draws the application's name");
        let at = at + egui::vec2(20.0, 8.0);
        let mut input = window_input();
        input.events = vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ];
        let _ = ctx.run(input, |ctx| show(&mut app, ctx));

        assert!(
            app.dialogs.is_open(Dialog::About),
            "the name must be the way to About"
        );
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
