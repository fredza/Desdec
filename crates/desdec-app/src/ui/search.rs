//! Finding what the listing will not scroll to.
//!
//! Three questions in one window, because they are the same gesture: a run of
//! bytes, a word in the decoded instructions, or something in the reader's own
//! notes. Each answer is a list of addresses, and each row goes there.
//!
//! With nothing typed, the notes mode lists every note and every bookmark —
//! which is what makes this the bookmark list as well as a search.

use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::{Text, text},
    search::{self, Mode, Results},
    ui::{MUTED, syntax},
};

/// Size assumed the first time the window opens, before egui has measured it.
const ASSUMED_SIZE: egui::Vec2 = egui::vec2(620.0, 380.0);

/// What is being looked for, and what was found.
#[derive(Default)]
pub struct State {
    pub mode: Mode,
    pub query: String,
    /// The last answer, kept so the scan runs when the question changes rather
    /// than on every frame the window is open: a byte pattern over a large
    /// image is millions of comparisons.
    results: Results,
    asked: Option<(Mode, String)>,
}

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::Search) {
        return;
    }
    let id = egui::Id::new("desdec.search");
    let mut open = true;
    let mut window = egui::Window::new(app.t(Text::Search))
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(ASSUMED_SIZE);
    if let Some(step) = app.dialogs.opening_step(Dialog::Search) {
        window = window.current_pos(crate::ui::opening_position(ctx, id, step, ASSUMED_SIZE));
    }
    let mut go_to = None;
    window.show(ctx, |ui| {
        go_to = contents(app, ui);
    });

    app.dialogs.set(Dialog::Search, open);
    if let Some(address) = go_to {
        app.go_to_address(ctx, address);
    }
}

fn contents(app: &mut DesdecApp, ui: &mut egui::Ui) -> Option<u64> {
    let language = app.preferences.language;
    ui.horizontal(|ui| {
        for mode in Mode::ALL {
            ui.radio_value(
                &mut app.search.mode,
                *mode,
                text(
                    language,
                    match mode {
                        Mode::Bytes => Text::SearchBytes,
                        Mode::Instructions => Text::SearchInstructions,
                        Mode::Notes => Text::SearchNotes,
                    },
                ),
            );
        }
    });
    ui.add_space(6.0);
    let hint = match app.search.mode {
        Mode::Bytes => Text::SearchBytesHint,
        Mode::Instructions | Mode::Notes => Text::SearchTextHint,
    };
    ui.add(
        egui::TextEdit::singleline(&mut app.search.query)
            .hint_text(text(language, hint))
            .desired_width(ui.available_width()),
    );
    match app.search.mode {
        Mode::Bytes => {
            ui.small(egui::RichText::new(text(language, Text::SearchBytesHelp)).color(MUTED))
        }
        Mode::Notes => {
            ui.small(egui::RichText::new(text(language, Text::SearchNotesHelp)).color(MUTED))
        }
        Mode::Instructions => ui.small(""),
    };
    ui.add_space(6.0);
    ui.separator();

    run(app);
    results(app, ui)
}

/// Answers the question on screen, when it is not the one already answered.
fn run(app: &mut DesdecApp) {
    let asked = (app.search.mode, app.search.query.clone());
    if app.search.asked.as_ref() == Some(&asked) {
        return;
    }
    let Some(analysis) = &app.analysis else {
        return;
    };
    app.search.results = match app.search.mode {
        Mode::Bytes => match search::Pattern::parse(&app.search.query) {
            Some(pattern) => search::bytes(analysis, &app.file_bytes, &pattern),
            // Not a pattern yet: a reader halfway through typing one is not
            // an error, and answering "nothing found" would say it was.
            None => Results::default(),
        },
        Mode::Instructions => search::instructions(analysis, &app.search.query),
        Mode::Notes => search::notes(analysis, &app.annotations, &app.search.query),
    };
    app.search.asked = Some(asked);
}

/// Returns the address the reader asked to be taken to.
fn results(app: &DesdecApp, ui: &mut egui::Ui) -> Option<u64> {
    let language = app.preferences.language;
    let found = &app.search.results;
    if found.hits.is_empty() {
        ui.add_space(8.0);
        // A question that has not been asked yet is not a question that failed:
        // an empty field, or half a byte pattern, is a reader still typing.
        let asked = match app.search.mode {
            Mode::Bytes => search::Pattern::parse(&app.search.query).is_some(),
            Mode::Instructions => !app.search.query.trim().is_empty(),
            Mode::Notes => true,
        };
        if asked {
            ui.label(egui::RichText::new(text(language, Text::NothingFound)).color(MUTED));
        }
        return None;
    }

    let mut go_to = None;
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("search_results")
                .num_columns(3)
                .striped(true)
                .spacing([14.0, 4.0])
                .show(ui, |ui| {
                    for hit in &found.hits {
                        if row(app, ui, hit) {
                            go_to = hit.address;
                        }
                        ui.end_row();
                    }
                });
        });
    if found.truncated {
        ui.add_space(4.0);
        ui.small(egui::RichText::new(text(language, Text::SearchTruncated)).color(MUTED));
    }
    go_to
}

/// One hit: where it is, in which section, and what was found there.
fn row(app: &DesdecApp, ui: &mut egui::Ui, hit: &search::Hit) -> bool {
    let language = app.preferences.language;
    let shown = match (hit.address, hit.file_offset) {
        (Some(address), _) => format!("{address:#018x}"),
        // Bytes in a part of the file that is never mapped have no address at
        // all; the offset is the only honest thing to show for them.
        (None, Some(offset)) => format!("{offset:#010x}"),
        (None, None) => String::new(),
    };
    let address = ui.add(
        egui::Label::new(syntax::dim(ui, &shown, egui::Color32::TRANSPARENT))
            .sense(egui::Sense::click()),
    );
    ui.label(
        egui::RichText::new(hit.section.clone().unwrap_or_default())
            .small()
            .color(MUTED),
    );
    ui.label(syntax::assembly(ui, &hit.text, egui::Color32::TRANSPARENT));

    let reachable = hit.address.is_some_and(|address| app.is_decoded(address));
    if reachable {
        address
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    } else {
        address.on_hover_text(text(language, Text::NotInTheListing));
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::WorkspaceView, commands::Command, i18n::Language, testing::window_input};

    fn searching(mode: Mode, query: &str) -> (egui::Context, DesdecApp) {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Disassembly);
        app.preferences.language = Language::English;
        app.run_command(&ctx, Command::Search);
        app.search.mode = mode;
        app.search.query = query.to_owned();
        (ctx, app)
    }

    fn drawn(ctx: &egui::Context, app: &mut DesdecApp) -> String {
        // Two frames: a window placed by its own measured size is laid out on
        // the first and painted on the second.
        let _ = ctx.run(window_input(), |ctx| show(app, ctx));
        let output = ctx.run(window_input(), |ctx| show(app, ctx));
        crate::testing::drawn_text(&output.shapes)
    }

    /// A pattern the file really holds must be found where it holds it.
    #[test]
    fn a_byte_pattern_is_found_at_its_own_offset() {
        let bytes = crate::testing::reference_bytes();
        let Some(first) = bytes.first().copied() else {
            return;
        };
        let pattern = format!("{first:02x} ?? {:02x}", bytes[2]);
        let (ctx, mut app) = searching(Mode::Bytes, &pattern);

        let _ = drawn(&ctx, &mut app);

        assert!(
            !app.search.results.hits.is_empty(),
            "{pattern} is in the file by construction"
        );
        assert_eq!(app.search.results.hits[0].file_offset, Some(0));
    }

    /// Half a pattern is a reader still typing, not a failed search.
    #[test]
    fn an_unfinished_pattern_is_not_an_answer() {
        let (ctx, mut app) = searching(Mode::Bytes, "48 8");

        let drawn = drawn(&ctx, &mut app);

        assert!(app.search.results.hits.is_empty());
        assert!(
            !drawn.contains(text(Language::English, Text::NothingFound)),
            "a half-written pattern must not be answered with a failure"
        );
    }

    /// The notes mode with nothing typed is the list of everything the reader
    /// has written and marked.
    #[test]
    fn the_notes_mode_lists_the_bookmarks() {
        let analysis = crate::testing::reference_analysis();
        let Some(address) = analysis.instructions.first().map(|i| i.address) else {
            return;
        };
        let (ctx, mut app) = searching(Mode::Notes, "");
        app.annotations.set(
            address,
            crate::annotations::Annotation {
                label: "parse_header".to_owned(),
                comment: String::new(),
                bookmarked: true,
            },
        );
        app.search.asked = None;

        let drawn = drawn(&ctx, &mut app);

        assert!(drawn.contains("parse_header"), "the note must be listed");
        assert!(drawn.contains(&format!("{address:#018x}")));
    }

    /// An instruction search takes the reader to the row it found.
    #[test]
    fn a_result_takes_the_reader_to_the_address() {
        let analysis = crate::testing::reference_analysis();
        let Some(instruction) = analysis.instructions.first() else {
            return;
        };
        let address = instruction.address;
        let (ctx, mut app) = searching(Mode::Instructions, &instruction.text);
        app.selected_instruction = None;

        let _ = drawn(&ctx, &mut app);
        assert!(!app.search.results.hits.is_empty());

        // Following a hit is what the row does when it is clicked.
        assert!(app.go_to_address(&ctx, address));
        assert_eq!(app.selected_instruction, Some(address));
        assert_eq!(app.active_view, WorkspaceView::Disassembly);
    }
}
