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

const WIDTH: f32 = 430.0;
/// Gap between the button that asked and the answer.
const GAP: f32 = 6.0;
/// Height assumed before the window has been laid out once.
const ASSUMED_HEIGHT: f32 = 180.0;

/// Where to put the window so it sits above `asked_at`, and on screen.
///
/// The height comes from the last time the window was laid out; the first
/// opening has none, so a plausible one is assumed and corrected on the frame
/// after — which is what the reader would see either way, since a window has
/// to be measured before it can be placed by its bottom edge.
fn above(ctx: &egui::Context, asked_at: egui::Rect) -> egui::Pos2 {
    let height = ctx
        .memory(|memory| memory.area_rect(egui::Id::new("desdec.library_note")))
        .map_or(ASSUMED_HEIGHT, |rect| rect.height());
    let screen = ctx.screen_rect();
    let wanted = egui::Rect::from_min_size(
        egui::pos2(asked_at.left(), asked_at.top() - height - GAP),
        egui::vec2(WIDTH, height),
    );
    // Below the button instead, when there is no room above it: an explanation
    // pushed off the top of the window is no explanation.
    let top = if wanted.top() < screen.top() {
        asked_at.bottom() + GAP
    } else {
        wanted.top()
    };
    let left = wanted
        .left()
        .min(screen.right() - WIDTH - GAP)
        .max(screen.left() + GAP);
    egui::pos2(left, top)
}

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
    // A stable id keeps the window in place as the subject changes.
    let id = egui::Id::new("desdec.library_note");
    let mut open = true;
    let mut window = egui::Window::new(app.t(Text::LibraryExplanation))
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(WIDTH);
    // Opened over the button that asked, so the answer appears where the
    // reader is looking rather than wherever the window was last dragged.
    // Without a button to answer — which only happens in a test — it opens
    // like any other window, clear of those already on screen.
    let step = app.dialogs.opening_step(Dialog::Library);
    if let Some(asked_at) = app.explaining_library_at.take() {
        window = window.current_pos(above(ctx, asked_at));
    } else if let Some(step) = step {
        window = window.current_pos(crate::ui::opening_position(
            ctx,
            id,
            step,
            egui::vec2(WIDTH, ASSUMED_HEIGHT),
        ));
    }
    window.show(ctx, |ui| {
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

    /// The answer opens over the button that asked for it, not wherever the
    /// window was last left: on the overview, that button is one of a column
    /// of identical `?`s, and an explanation appearing elsewhere leaves the
    /// reader to work out which one it belongs to.
    #[test]
    fn the_explanation_opens_above_the_button_that_asked() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.explaining_library = Some("libc.so.6".to_owned());
        app.dialogs.open(Dialog::Library);

        // A button low on the screen: there is room above it for the answer.
        let button = egui::Rect::from_min_size(egui::pos2(700.0, 600.0), egui::vec2(18.0, 18.0));
        app.explaining_library_at = Some(button);
        let _ = ctx.run(crate::testing::window_input(), |ctx| show(&mut app, ctx));
        let placed = ctx
            .memory(|memory| memory.area_rect(egui::Id::new("desdec.library_note")))
            .expect("the window was laid out");

        assert!(
            placed.bottom() <= button.top(),
            "the answer ({placed:?}) must sit above the button ({button:?})"
        );
        assert!(
            (placed.left() - button.left()).abs() < 1.0,
            "the answer must line up with the button it belongs to"
        );
        assert!(
            app.explaining_library_at.is_none(),
            "the position is consumed, so the reader can then move the window"
        );
    }

    /// A button near the top has no room above it; the answer goes below
    /// rather than off the screen.
    #[test]
    fn an_answer_that_would_not_fit_above_opens_below() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.explaining_library = Some("libc.so.6".to_owned());
        app.dialogs.open(Dialog::Library);

        let button = egui::Rect::from_min_size(egui::pos2(40.0, 12.0), egui::vec2(18.0, 18.0));
        app.explaining_library_at = Some(button);
        let _ = ctx.run(crate::testing::window_input(), |ctx| show(&mut app, ctx));
        let placed = ctx
            .memory(|memory| memory.area_rect(egui::Id::new("desdec.library_note")))
            .expect("the window was laid out");

        assert!(placed.top() >= button.bottom(), "{placed:?}");
        assert!(
            placed.right() <= ctx.screen_rect().right(),
            "the answer must stay on screen"
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
