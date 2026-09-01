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
///
/// The calculator alone. Opening a file and the command palette used to sit
/// here, and neither belonged: the file is already named in this very bar, one
/// separator to the left, with the button that closes it — and the palette is
/// what a reader reaches for by key, not by pointer. What is worth a button in
/// the corner of every frame is the thing a reader wants *while* reading a
/// listing and cannot get from the listing: a number turned into another base.
const ACTIONS: &[(Icon, Command, Dialog)] =
    &[(Icon::Calculator, Command::Calculator, Dialog::Calculator)];

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

                // No separator here. There was one, and it existed to detach
                // the application's own name from the hamburger — a name this
                // bar no longer carries: it was the way to About, and it cost
                // a permanent inch of a row that is otherwise entirely about
                // the file being read. About is on F1, in the menu, and in the
                // palette under `version`.
                //
                // Left in place it drew a second line hard against the one
                // below, since each of the two things that follow opens with a
                // separator of its own. Two rules with nothing between them
                // separate nothing.
                //
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
    // right-aligned actions, where they would have come out back to front.
    //
    // Only in the disassembly view: they answer a question about the selected
    // *instruction*, and there is no selected instruction to speak of while
    // the reader is looking at strings, at sections or at a call graph — six
    // greyed letters there say nothing and take the room the calculator and
    // the other actions need. And only while there is room for them: the bar
    // is one fixed row.
    if app.active_view == WorkspaceView::Disassembly && ui.available_width() > flags::NEEDED_WIDTH {
        ui.separator();
        flags::show(app, ui);
    }

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        for (icon, command, dialog) in ACTIONS {
            // Lit while the window it opens is on screen: the button is the
            // way back to a window the reader may have pushed behind the
            // listing, and an unlit one reads as *not open*.
            if icons::button(
                ui,
                *icon,
                app.optional_command_tooltip(*command),
                app.dialogs.is_open(*dialog),
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
        for (_, command, _) in ACTIONS {
            assert!(
                command.opens_view().is_none(),
                "{command:?} opens a view, and the toolbar already lists those"
            );
        }
    }

    /// No two rules stand side by side with nothing between them.
    ///
    /// The bar used to carry the application's own name, with a separator in
    /// front of it. Taking the name out left that separator hard against the
    /// one belonging to whatever came next — two lines a few pixels apart,
    /// separating nothing. No assertion about text catches it: a separator
    /// says nothing.
    ///
    /// Checked in both states, because they draw different things: with a
    /// binary open the file's name and the toolbar each open with a rule of
    /// their own, and with none the toolbar's is the only one that should be
    /// there.
    #[test]
    fn no_two_separators_are_drawn_against_each_other() {
        for open in [false, true] {
            let ctx = egui::Context::default();
            let mut app = if open {
                opened_app(WorkspaceView::Overview)
            } else {
                crate::app::DesdecApp::for_test(None, WorkspaceView::Overview)
            };
            app.preferences.show_toolbar = true;

            let _ = ctx.run(window_input(), |ctx| show(&mut app, ctx));
            let output = ctx.run(window_input(), |ctx| show(&mut app, ctx));

            // A separator is a rule: far taller than it is wide, and the only
            // thing in this bar shaped that way.
            let mut rules: Vec<f32> = Vec::new();
            for clipped in &output.shapes {
                collect_rules(&clipped.shape, &mut rules);
            }
            rules.sort_by(f32::total_cmp);

            // Measured rather than guessed at. With the defect the two rules
            // stood at 53 and 67 — fourteen pixels, which is the bar's own
            // item spacing and nothing else. Without it they are at 53 and
            // 298, with the file's name between them. The smallest thing this
            // bar can legitimately put between two rules is one icon button,
            // which is twenty-eight pixels wide.
            const ROOM_FOR_SOMETHING: f32 = 24.0;
            for pair in rules.windows(2) {
                assert!(
                    pair[1] - pair[0] > ROOM_FOR_SOMETHING,
                    "rules at {} and {} with nothing between them (binary open: {open})",
                    pair[0],
                    pair[1]
                );
            }
        }
    }

    /// Where each vertical rule was drawn.
    fn collect_rules(shape: &egui::Shape, out: &mut Vec<f32>) {
        match shape {
            egui::Shape::Rect(rect) => {
                let at = rect.rect;
                if at.width() <= 3.0 && at.height() >= 12.0 {
                    out.push(at.center().x);
                }
            }
            egui::Shape::LineSegment { points, .. } => {
                let [from, to] = points;
                if (from.x - to.x).abs() <= 1.0 && (from.y - to.y).abs() >= 12.0 {
                    out.push(from.x);
                }
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_rules(shape, out);
                }
            }
            _ => {}
        }
    }

    /// The bar does not name the application.
    ///
    /// It did, as a button opening About, and that cost a permanent inch of a
    /// row otherwise entirely about the file being read. About is on F1, in
    /// the menu, and in the palette under `version`; a program does not have
    /// to tell its reader which program it is on every frame.
    #[test]
    fn the_bar_is_about_the_file_and_not_about_the_program() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Overview);
        app.preferences.show_toolbar = true;

        let output = ctx.run(window_input(), |ctx| show(&mut app, ctx));

        let drawn = crate::testing::drawn_text(&output.shapes);
        assert!(
            !drawn.contains("Desdec"),
            "the bar still names the application: {drawn:?}"
        );
    }

    /// With a binary open and the toolbar on, the bar draws what it is for:
    /// the name of the file, the way to close it, and the actions.
    #[test]
    fn the_bar_draws_with_a_binary_open() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Overview);
        app.preferences.show_toolbar = true;

        let output = ctx.run(window_input(), |ctx| show(&mut app, ctx));

        // Taken from the file that was actually opened rather than written
        // here: the suite analyses the runner's own executable, and
        // `DESDEC_REFERENCE` points it at another one on the platforms that
        // need it.
        let opened = app
            .analysis
            .as_ref()
            .expect("a binary is open")
            .summary
            .path
            .file_name()
            .expect("the open file has a name")
            .to_string_lossy()
            .chars()
            .take(10)
            .collect::<String>();
        let drawn = crate::testing::drawn_text(&output.shapes);
        assert!(
            drawn.contains(&opened),
            "the bar does not name the open file {opened:?}: {drawn:?}"
        );
        assert!(
            !output.shapes.is_empty(),
            "the bar must draw with a binary open"
        );
    }
}
