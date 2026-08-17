//! What a linked library is for, shown on demand.
//!
//! A separate window rather than a tooltip: an explanation is a paragraph, it
//! deserves to stay on screen while the reader looks back at the dependency
//! list, and a tooltip vanishes the moment the pointer moves.

use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::Text,
    libraries::NoteSource,
    ui::{MUTED, section_title},
};

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::Library) {
        return;
    }
    let Some(library) = app.explaining_library.clone() else {
        app.dialogs.close(Dialog::Library);
        return;
    };

    let language = app.preferences.language;
    let note = app.library_notes.note(&library, language);
    let mut open = true;
    egui::Window::new(app.t(Text::LibraryExplanation))
        // A stable id keeps the window in place as the subject changes.
        .id(egui::Id::new("desdec.library_note"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(430.0)
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(&library).monospace().strong());
            ui.add_space(10.0);

            let Some(note) = note else {
                // Naming the library is all that can honestly be done:
                // guessing from the name would send the reader looking in the
                // wrong place.
                ui.label(
                    egui::RichText::new(app.t(Text::LibraryUndescribed))
                        .color(MUTED)
                        .italics(),
                );
                ui.add_space(10.0);
                ui.small(app.t(Text::DescribeItYourself));
                return;
            };

            ui.label(&note.summary);
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(4.0);
            ui.label(section_title(app.t(match note.source {
                NoteSource::Builtin => Text::NoteFromCatalogue,
                NoteSource::UserFile => Text::NoteFromYourFile,
            })));
        });

    app.dialogs.set(Dialog::Library, open);
    if !open {
        app.explaining_library = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::WorkspaceView;

    /// The dialog must lay out for a described library, an undescribed one,
    /// and in every interface language.
    #[test]
    fn the_explanation_lays_out_whatever_the_library() {
        let ctx = egui::Context::default();
        for library in ["libc.so.6", "libcompletelyunknown.so.1"] {
            let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
            app.explaining_library = Some(library.to_owned());
            app.dialogs.open(Dialog::Library);

            for language in crate::i18n::Language::ALL {
                app.preferences.language = *language;
                let output = ctx.run(egui::RawInput::default(), |ctx| show(&mut app, ctx));
                assert!(!output.shapes.is_empty(), "{library} {language:?}");
            }
        }
    }

    /// A real binary's dependencies must actually reach the catalogue —
    /// matching fixtures would prove nothing about real library names.
    #[test]
    fn a_real_binarys_libraries_are_described() {
        let analysis = crate::testing::reference_analysis();
        let mut catalogue = crate::libraries::Catalogue::default();

        let described = analysis
            .details
            .linked_libraries
            .iter()
            .filter(|library| {
                catalogue
                    .note(library, crate::i18n::Language::French)
                    .is_some()
            })
            .count();
        // This binary links a C library on every platform the tests run on.
        assert!(
            described > 0 || analysis.details.linked_libraries.is_empty(),
            "none of {:?} reached the catalogue",
            analysis.details.linked_libraries
        );
    }

    /// Closing the window must forget its subject, or reopening it would show
    /// the previous library.
    #[test]
    fn closing_forgets_which_library_was_explained() {
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.explaining_library = Some("libc.so.6".to_owned());
        app.dialogs.close(Dialog::Library);

        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| show(&mut app, ctx));

        // The window is shut, so nothing was drawn and the subject is stale
        // only until it is opened again with a new one.
        assert!(!app.dialogs.is_open(Dialog::Library));
    }
}
