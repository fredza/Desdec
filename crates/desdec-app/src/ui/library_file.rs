//! Editing the descriptions the reader wrote themselves.
//!
//! The preferences told the reader where the file lives and how many
//! descriptions it holds, and then left them to find a text editor. That is a
//! long way round for what is usually one line — a name, an equals sign, a
//! sentence about an in-house library the built-in catalogue will never know.
//!
//! So the file is edited here, in the application that reads it: the box holds
//! the file as it stands, saving writes it back and reads it again, and the
//! "?" buttons in the Overview say the new sentence on the next press. A
//! window rather than a panel in the preferences, because the lines are long
//! and the preferences window is narrow and fixed.

use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::{Text, text},
    ui::{ERROR, MUTED},
};

/// Size assumed the first time the window opens, before egui has measured it.
const ASSUMED_SIZE: egui::Vec2 = egui::vec2(560.0, 420.0);

/// The file as it is being edited, and what went wrong writing it.
#[derive(Default)]
pub struct Draft {
    /// The text in the box. Loaded when the window opens, so a file changed
    /// outside Desdec is never silently overwritten with a stale copy.
    pub text: String,
    /// Why the file could not be written, when it could not. Kept on screen
    /// rather than flashed as a notice: the reader still has unsaved work in
    /// the box, and needs to know it did not reach the disk.
    refused: Option<String>,
}

/// Reads the file into the box and puts the window on screen.
pub fn open(app: &mut DesdecApp) {
    open_for(app, None);
}

/// The same, started on `library`: the reader who just read that nothing
/// describes it should not have to type its name back in.
pub fn open_for(app: &mut DesdecApp, library: Option<&str>) {
    let mut text = crate::libraries::read_user_file(app.preferences.language);
    if let Some(library) = library {
        let name = crate::libraries::catalogue_key(library);
        if !crate::libraries::describes_any(&text, library) {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&name);
            text.push_str(" = ");
        }
    }
    app.library_file.text = text;
    app.library_file.refused = None;
    app.dialogs.open(Dialog::LibraryFile);
}

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::LibraryFile) {
        return;
    }
    let id = egui::Id::new("desdec.library_file");
    let mut open = true;
    let mut window = egui::Window::new(app.t(Text::LibraryFileTitle))
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(ASSUMED_SIZE)
        .min_width(360.0);
    window = crate::ui::centred(
        window,
        ctx,
        app.dialogs.opening_step(Dialog::LibraryFile).is_some(),
    );

    let mut save = false;
    let mut cancel = false;
    window.show(ctx, |ui| {
        (save, cancel) = contents(app, ui);
    });

    app.dialogs.set(Dialog::LibraryFile, open);
    if cancel {
        app.dialogs.close(Dialog::LibraryFile);
    }
    if save {
        save_it(app, ctx);
    }
}

/// Returns whether the reader asked to save, and whether they gave up.
fn contents(app: &mut DesdecApp, ui: &mut egui::Ui) -> (bool, bool) {
    let language = app.preferences.language;
    ui.small(text(language, Text::LibraryFileFormat));
    ui.add_space(6.0);

    egui::ScrollArea::vertical()
        .max_height(ui.available_height() - 56.0)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut app.library_file.text)
                    .code_editor()
                    .desired_rows(12)
                    .desired_width(ui.available_width())
                    .hint_text("libmaison = ..."),
            );
        });

    if let Some(path) = crate::libraries::user_catalogue_path() {
        ui.add_space(4.0);
        ui.small(egui::RichText::new(path.display().to_string()).color(MUTED));
    }
    if let Some(refused) = &app.library_file.refused {
        ui.add_space(4.0);
        ui.colored_label(
            ERROR,
            format!(
                "{} : {refused}",
                text(language, Text::LibraryFileNotWritten)
            ),
        );
    }

    ui.add_space(8.0);
    let mut save = false;
    let mut cancel = false;
    ui.horizontal(|ui| {
        save = ui.button(text(language, Text::SaveFile)).clicked();
        cancel = ui.button(text(language, Text::Cancel)).clicked();
    });
    (save, cancel)
}

/// Writes the box back to the file and reads it again, so the Overview says
/// the new sentences without a restart.
fn save_it(app: &mut DesdecApp, ctx: &egui::Context) {
    let text_to_write = app.library_file.text.clone();
    match crate::libraries::save_user_file(&text_to_write) {
        Ok(_) => {
            app.library_file.refused = None;
            app.library_notes.reload();
            let language = app.preferences.language;
            let entries = app.library_notes.user_entries();
            app.notify(
                ctx,
                format!(
                    "{} : {entries} {}",
                    text(language, Text::LibraryFileSaved),
                    text(language, Text::LibraryFileEntries)
                ),
            );
            app.dialogs.close(Dialog::LibraryFile);
        }
        // The window stays open: what is in the box is the only copy.
        Err(error) => app.library_file.refused = Some(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::WorkspaceView;

    /// The window must lay out in every interface language.
    #[test]
    fn the_editor_lays_out_in_every_language() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.dialogs.open(Dialog::LibraryFile);
        app.library_file.text = "libmaison = Format maison.\n".to_owned();

        for language in crate::i18n::Language::ALL {
            app.preferences.language = *language;
            let output = ctx.run(crate::testing::window_input(), |ctx| show(&mut app, ctx));
            assert!(!output.shapes.is_empty(), "{language:?}");
        }
    }

    /// A refusal belongs on screen, next to the text that did not reach the
    /// disk — not in a notice that fades while the reader reads it.
    #[test]
    fn a_refusal_stays_on_screen() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.dialogs.open(Dialog::LibraryFile);
        app.library_file.refused = Some("permission denied".to_owned());

        let output = ctx.run(crate::testing::window_input(), |ctx| show(&mut app, ctx));
        assert!(!output.shapes.is_empty());
        assert!(
            app.library_file.refused.is_some(),
            "drawing the window must not clear the refusal"
        );
    }

    /// Opening on an undescribed library must leave the reader with only the
    /// sentence to write.
    #[test]
    fn opening_on_a_library_starts_its_line() {
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        open_for(&mut app, Some("libcompletelyunknown.so.1"));

        assert!(app.dialogs.is_open(Dialog::LibraryFile));
        assert!(
            app.library_file.text.contains("libcompletelyunknown = "),
            "{:?}",
            app.library_file.text
        );
        assert!(app.library_file.refused.is_none());
    }

    /// Giving up shuts the window and keeps the file as it was.
    #[test]
    fn cancelling_closes_the_window() {
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.dialogs.open(Dialog::LibraryFile);
        app.dialogs.close(Dialog::LibraryFile);

        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| show(&mut app, ctx));
        assert!(!app.dialogs.is_open(Dialog::LibraryFile));
    }
}
