use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::Text,
    journal::Level,
    preferences::{accent, success},
    ui::{ERROR, MUTED, format_size},
};

const HEIGHT: f32 = 28.0;

/// Longest last message shown in the bar. Beyond this it is cut, and the
/// window it opens has the whole of it.
const MESSAGE_LIMIT: usize = 64;

/// A line that could not be delivered, in the colour the output window uses.
const WARNING: egui::Color32 = egui::Color32::from_rgb(224, 164, 104);

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(HEIGHT)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                state(app, ui);
                // The last thing that happened, at the far end, whatever the
                // left-hand side is saying: an export that failed while the
                // reader was in another view is otherwise nowhere on screen.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    last_message(app, ui);
                });
            });
        });
}

/// What the application is doing, or what it has open.
fn state(app: &mut DesdecApp, ui: &mut egui::Ui) {
    // An analysis actually running is the whole status: claiming a readiness
    // the application does not have would contradict it. Waiting on the file
    // dialog is not that — nothing is being analysed until a file has been
    // chosen.
    if app.is_analysing() {
        ui.spinner();
        ui.label(
            egui::RichText::new(app.t(Text::StatusWorking)).color(accent(app.preferences.theme)),
        );
        if ui.button(app.t(Text::CancelAnalysis)).clicked() {
            app.cancel_analysis();
        }
        return;
    }
    if app.is_choosing_file() {
        ui.label(egui::RichText::new(app.t(Text::StatusChoosing)).color(MUTED));
        // A native dialog the desktop never answers used to leave the
        // application waiting on it for good, refusing every later request to
        // open anything.
        if ui.button(app.t(Text::CancelChoosing)).clicked() {
            app.cancel_analysis();
        }
        return;
    }

    if app.error.is_some() {
        ui.label(egui::RichText::new(app.t(Text::StatusFailed)).color(ERROR));
    } else {
        ui.label(egui::RichText::new("OK").color(success(app.preferences.theme)));
    }

    if let Some(summary) = app.analysis.as_ref().map(|analysis| &analysis.summary) {
        ui.label(summary.format.label());
        ui.separator();
        ui.label(summary.architecture.label());
        ui.separator();
        ui.label(format_size(summary.size));
    } else {
        ui.label(app.t(Text::ReadyToOpen));
    }
}

/// The most recent line of the session's account, and the way into all of it.
fn last_message(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let Some(entry) = app.journal.last() else {
        return;
    };
    let colour = match entry.level {
        Level::Note => MUTED,
        Level::Warning => WARNING,
        Level::Failure => ERROR,
    };
    let full = entry.text.clone();
    let shown = if full.chars().count() > MESSAGE_LIMIT {
        let cut: String = full.chars().take(MESSAGE_LIMIT).collect();
        format!("{cut}…")
    } else {
        full.clone()
    };
    let message = ui
        .add(
            egui::Label::new(egui::RichText::new(shown).small().color(colour))
                .sense(egui::Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("{full}\n\n{}", app.t(Text::Output)));
    if message.clicked() {
        app.dialogs.open(Dialog::Output);
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app::{DesdecApp, Dialog, WorkspaceView},
        journal::Level,
        testing::{drawn_text, window_input},
    };
    use eframe::egui;

    /// An export that failed while the reader was in another view is nowhere
    /// on screen unless the bar says so.
    #[test]
    fn the_bar_shows_the_last_thing_that_happened() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.note(Level::Failure, "could not write /tmp/copy");

        let output = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));

        assert!(drawn_text(&output.shapes).contains("could not write /tmp/copy"));
    }

    /// A message too long for the bar is cut rather than pushing the rest of
    /// the status off the end of the window.
    #[test]
    fn a_long_message_is_cut_and_the_window_holds_the_whole_of_it() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        let long = "x".repeat(super::MESSAGE_LIMIT * 2);
        app.note(Level::Note, long.clone());

        let output = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));

        let drawn = drawn_text(&output.shapes);
        assert!(drawn.contains('\u{2026}'), "the message must be cut");
        assert!(!drawn.contains(&long), "and not drawn in full");
        assert!(
            app.journal.as_text().contains(&long),
            "the account keeps all of it"
        );
    }

    /// The bar is the way in: the whole account is one click away.
    #[test]
    fn the_last_message_opens_the_output() {
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.note(Level::Note, "something happened");
        assert!(!app.dialogs.is_open(Dialog::Output));

        let ctx = egui::Context::default();
        // The bar is laid out first; the press then lands on the label in it.
        let output = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));
        let position = crate::testing::drawn(&output.shapes)
            .into_iter()
            .find(|(text, _)| text == "something happened")
            .map(|(_, position)| position + egui::vec2(4.0, 4.0))
            .expect("the message must be drawn");

        let _ = ctx.run(crate::testing::click_at(position), |ctx| {
            super::show(&mut app, ctx);
        });

        assert!(app.dialogs.is_open(Dialog::Output));
    }
}
