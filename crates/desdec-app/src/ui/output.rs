//! The floating account of the session: what the application has done, in the
//! order it did it.
//!
//! Every other surface here answers about the *file*. This one answers about
//! the *program*: which binary was opened and what was found in it, what was
//! written where, what was sent to a model, and everything that failed. Those
//! answers each appear once, in the view that produced them, and are gone by
//! the time the reader has looked elsewhere — so they are kept here as well.
//!
//! A window rather than a panel: a reader watches it while working in the
//! listing behind it, moves it out of the way, and closes it when the question
//! it answered is settled.

use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::{Text, text},
    journal::{Entry, Level},
    ui::{ERROR, MUTED},
};

/// Size assumed the first time the window opens, before egui has measured it.
const ASSUMED_SIZE: egui::Vec2 = egui::vec2(620.0, 360.0);

/// A line that could not be delivered, in the one colour reserved for those.
const WARNING: egui::Color32 = egui::Color32::from_rgb(224, 164, 104);

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::Output) {
        return;
    }
    let id = egui::Id::new("desdec.output");
    let mut open = true;
    let mut window = egui::Window::new(app.t(Text::Output))
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(ASSUMED_SIZE);
    window = crate::ui::centred(
        window,
        ctx,
        app.dialogs.opening_step(Dialog::Output).is_some(),
    );
    window.show(ctx, |ui| contents(app, ui));
    app.dialogs.set(Dialog::Output, open);
}

fn contents(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    let mut copy = false;
    let mut clear = false;

    ui.horizontal(|ui| {
        let has_lines = !app.journal.is_empty();
        copy = ui
            .add_enabled(
                has_lines,
                egui::Button::new(text(language, Text::CopyEverything)),
            )
            .clicked();
        clear = ui
            .add_enabled(
                has_lines,
                egui::Button::new(text(language, Text::ClearOutput)),
            )
            .clicked();
        ui.separator();
        let count = ui.label(
            egui::RichText::new(format!(
                "{} {}",
                app.journal.len(),
                text(language, Text::OutputLines)
            ))
            .small()
            .color(MUTED),
        );
        // The stamps are elapsed time, not the time of day, and a reader
        // comparing them with a clock on the wall has to be told so.
        count.on_hover_text(text(language, Text::OutputClock));
    });
    ui.small(egui::RichText::new(text(language, Text::OutputIntro)).color(MUTED));
    ui.add_space(6.0);
    ui.separator();

    if app.journal.is_empty() {
        ui.add_space(10.0);
        ui.label(egui::RichText::new(text(language, Text::OutputEmpty)).color(MUTED));
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        // The newest line is the one being waited for, so the view follows it
        // rather than leaving the reader to scroll after every event.
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for entry in app.journal.entries() {
                line(ui, entry);
            }
        });

    if copy {
        // The whole account goes to the clipboard, but not into the notice or
        // back into the account itself: a confirmation quoting a hundred lines
        // is not a confirmation, and a line recording the copy would be one
        // more thing to copy next time.
        ui.ctx().copy_text(app.journal.as_text());
        app.notify(ui.ctx(), app.t(Text::OutputCopied).to_owned());
    }
    if clear {
        app.journal.clear();
    }
}

/// One line: when it happened, then what happened.
fn line(ui: &mut egui::Ui, entry: &Entry) {
    ui.horizontal_top(|ui| {
        ui.label(
            egui::RichText::new(crate::journal::stamp(entry.at))
                .monospace()
                .color(MUTED),
        );
        ui.label(egui::RichText::new(&entry.text).color(match entry.level {
            Level::Note => ui.visuals().text_color(),
            Level::Warning => WARNING,
            Level::Failure => ERROR,
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::WorkspaceView, i18n::Language, testing::window_input};

    /// The window is the only place a failure survives the moment it
    /// happened, so what it says has to reach the screen.
    #[test]
    fn the_window_shows_what_the_session_recorded() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.note(Level::Failure, "could not read /tmp/nothing");
        app.dialogs.open(Dialog::Output);

        // Two frames: a window placed by its own measured size is laid out on
        // the first and painted on the second.
        let _ = ctx.run(window_input(), |ctx| show(&mut app, ctx));
        let output = ctx.run(window_input(), |ctx| show(&mut app, ctx));

        assert!(
            crate::testing::drawn_text(&output.shapes).contains("could not read /tmp/nothing"),
            "the recorded line must be drawn"
        );
    }

    /// An empty account says so rather than drawing an empty box the reader
    /// has to interpret.
    #[test]
    fn an_empty_account_says_it_is_empty() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.preferences.language = Language::English;
        app.dialogs.open(Dialog::Output);

        // Two frames: a window placed by its own measured size is laid out on
        // the first and painted on the second.
        let _ = ctx.run(window_input(), |ctx| show(&mut app, ctx));
        let output = ctx.run(window_input(), |ctx| show(&mut app, ctx));

        assert!(
            crate::testing::drawn_text(&output.shapes)
                .contains(text(Language::English, Text::OutputEmpty))
        );
    }

    /// The account is about the session, not about one file: closing a binary
    /// must not take with it the record of what happened to it.
    #[test]
    fn closing_a_binary_keeps_the_account_of_the_session() {
        let mut app = DesdecApp::for_test(
            Some(crate::testing::reference_analysis().clone()),
            WorkspaceView::Overview,
        );
        app.note(Level::Note, "opened something");

        app.close_binary();

        assert_eq!(app.journal.len(), 2, "closing is recorded, not forgotten");
        assert!(app.journal.as_text().contains("opened something"));
    }

    /// A reader keeps this window open beside the listing to watch what
    /// happens; a press on a row must not take it away.
    #[test]
    fn a_press_on_the_workspace_leaves_the_window_open() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.dialogs.open(Dialog::Output);
        app.dialogs.open(Dialog::About);

        // The topmost window that a press outside closes is About; the output
        // stays where it was put.
        let _ = ctx.run(crate::testing::press_at(egui::pos2(600.0, 700.0)), |ctx| {
            app.run_frame(ctx);
        });

        assert!(!app.dialogs.is_open(Dialog::About), "About is dismissed");
        assert!(app.dialogs.is_open(Dialog::Output), "the output stays");
    }

    /// Escape and the close button still shut it: a window kept on purpose is
    /// not a window that cannot be got rid of.
    #[test]
    fn escape_still_closes_the_window() {
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.dialogs.open(Dialog::Output);

        assert!(app.dismiss_topmost_dialog());
        assert!(!app.dialogs.is_open(Dialog::Output));
    }

    /// It lays out in every language, on a real session's worth of lines.
    #[test]
    fn the_window_lays_out_in_every_language() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        for level in [Level::Note, Level::Warning, Level::Failure] {
            app.note(level, format!("{level:?} line"));
        }
        app.dialogs.open(Dialog::Output);

        for language in crate::i18n::Language::ALL {
            app.preferences.language = *language;
            let output = ctx.run(window_input(), |ctx| show(&mut app, ctx));
            assert!(!output.shapes.is_empty(), "{language:?}");
        }
    }
}
