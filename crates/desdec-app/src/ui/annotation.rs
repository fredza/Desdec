//! The reader's own note on one address: a name, a sentence, a mark.
//!
//! Everything else in this application reports what the file says. This is the
//! one window where the reader writes, and what they write is theirs: it is
//! never mixed with a symbol the format actually carries, and the listing
//! draws it in a column of its own.

use eframe::egui;

use crate::{
    annotations::Annotation,
    app::{DesdecApp, Dialog},
    i18n::{Text, text},
    ui::{MUTED, syntax},
};

/// Size assumed the first time the window opens, before egui has measured it.
const ASSUMED_SIZE: egui::Vec2 = egui::vec2(460.0, 260.0);

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::Annotation) {
        return;
    }
    let Some(address) = app.annotating_address else {
        app.dialogs.close(Dialog::Annotation);
        return;
    };

    let id = egui::Id::new("desdec.annotation");
    let mut open = true;
    let mut window = egui::Window::new(format!("{} {address:#018x}", app.t(Text::NoteFor)))
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(ASSUMED_SIZE.x);
    window = crate::ui::centred(
        window,
        ctx,
        app.dialogs.opening_step(Dialog::Annotation).is_some(),
    );
    window.show(ctx, |ui| contents(app, ui, address));

    app.dialogs.set(Dialog::Annotation, open);
    if !open {
        app.annotating_address = None;
    }
}

fn contents(app: &mut DesdecApp, ui: &mut egui::Ui, address: u64) {
    let language = app.preferences.language;
    // The instruction being named, so the reader is writing about what they
    // can see rather than about an address in the abstract.
    if let Some(instruction) = app
        .analysis
        .as_ref()
        .and_then(|analysis| analysis.instruction_at(address))
    {
        ui.label(syntax::assembly(
            ui,
            &instruction.text,
            egui::Color32::TRANSPARENT,
        ));
        ui.add_space(8.0);
    }

    let mut annotation = app.annotations.at(address).cloned().unwrap_or_default();
    let before = annotation.clone();

    ui.horizontal(|ui| {
        ui.label(text(language, Text::NoteLabel));
        ui.add(
            egui::TextEdit::singleline(&mut annotation.label)
                .hint_text(text(language, Text::NoteLabelHint))
                .desired_width(ui.available_width()),
        );
    });
    ui.add_space(6.0);
    ui.label(text(language, Text::NoteComment));
    ui.add(
        egui::TextEdit::multiline(&mut annotation.comment)
            .hint_text(text(language, Text::NoteCommentHint))
            .desired_rows(3)
            .desired_width(ui.available_width()),
    );
    ui.add_space(6.0);
    ui.checkbox(&mut annotation.bookmarked, text(language, Text::Bookmark));

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        let has_note = !annotation.is_empty();
        if ui
            .add_enabled(has_note, egui::Button::new(text(language, Text::ClearNote)))
            .clicked()
        {
            annotation = Annotation::default();
        }
    });
    ui.add_space(8.0);
    ui.small(egui::RichText::new(text(language, Text::NoteHelp)).color(MUTED));

    // Written as it is typed: the note is saved once the typing settles, and
    // a window that had to be confirmed would lose what was in it whenever it
    // was closed by an Escape meant for something else.
    if annotation != before {
        app.annotations.set(address, annotation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::WorkspaceView, commands::Command, i18n::Language};
    use eframe::egui;

    fn opened() -> (egui::Context, DesdecApp, u64) {
        let analysis = crate::testing::reference_analysis();
        let address = analysis
            .instructions
            .first()
            .map_or(0x1000, |instruction| instruction.address);
        let mut app = crate::testing::opened_app(WorkspaceView::Disassembly);
        app.preferences.language = Language::English;
        app.selected_instruction = Some(address);
        (egui::Context::default(), app, address)
    }

    /// The command opens the window on the row the reader is standing on.
    #[test]
    fn the_note_is_opened_on_the_selected_instruction() {
        let (ctx, mut app, address) = opened();

        app.run_command(&ctx, Command::EditAnnotation);

        assert_eq!(app.annotating_address, Some(address));
        assert!(app.dialogs.is_open(Dialog::Annotation));
    }

    /// A note typed in the window is the reader's work; it must reach the
    /// binary's annotations without a save button to forget to press.
    #[test]
    fn what_is_typed_is_kept_as_it_is_typed() {
        let (ctx, mut app, address) = opened();
        app.annotating_address = Some(address);
        app.dialogs.open(Dialog::Annotation);
        app.annotations.set(
            address,
            Annotation {
                label: "parse_header".to_owned(),
                comment: "reads the magic".to_owned(),
                bookmarked: true,
            },
        );

        // Two frames: a window placed by its own measured size is laid out on
        // the first and painted on the second.
        let _ = ctx.run(crate::testing::window_input(), |ctx| show(&mut app, ctx));
        let output = ctx.run(crate::testing::window_input(), |ctx| show(&mut app, ctx));

        let drawn = crate::testing::drawn_text(&output.shapes);
        assert!(drawn.contains("parse_header"), "the name must be shown");
        assert!(drawn.contains("reads the magic"));
    }

    /// A bookmark is a mark, not a window: the command puts it on and takes
    /// it off without asking anything.
    #[test]
    fn the_bookmark_command_marks_the_row_it_stands_on() {
        let (ctx, mut app, address) = opened();

        app.run_command(&ctx, Command::ToggleBookmark);
        assert!(app.annotations.is_bookmarked(address));

        app.run_command(&ctx, Command::ToggleBookmark);
        assert!(!app.annotations.is_bookmarked(address));
    }
}
