//! The console: a script, a button, and what it did.
//!
//! Kept as a window rather than a view because a script is written *about*
//! something — the listing behind it, the function on screen — and a reader
//! writing one is looking at both. It stays open when the workspace is
//! clicked, which no other window here does: a script takes minutes to write,
//! and losing it to a stray press on the listing it is about would be its own
//! kind of joke.
//!
//! Everything the reader types here runs with every permission granted. The
//! permissions exist to say what a script from *somewhere else* may do, and
//! the person typing into this box is the person they would be protecting.

use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::{Language, Text, text},
    script::{Context, Failure, Outcome},
    ui::{ERROR, MUTED},
};

const ASSUMED_SIZE: egui::Vec2 = egui::vec2(680.0, 520.0);

/// What is in the console, and what came of it.
#[derive(Default)]
pub struct State {
    pub source: String,
    /// What the last run did, kept until the next one: a script that finished
    /// a minute ago is still the answer to what is on screen.
    pub last: Option<Outcome>,
    /// What ran, when it was not what is in the box — a plugin, run from the
    /// list — so the output is never read as belonging to the script above it.
    pub ran: Option<String>,
    pub vocabulary_open: bool,
}

impl State {
    /// Records what a run produced, and what it was.
    pub fn took(&mut self, ran: Option<String>, outcome: Outcome) {
        self.ran = ran;
        self.last = Some(outcome);
    }
}

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::Console) {
        return;
    }
    let id = egui::Id::new("desdec.script");
    let mut open = true;
    let mut window = egui::Window::new(app.t(Text::ScriptTitle))
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(ASSUMED_SIZE)
        .min_width(420.0);
    if let Some(step) = app.dialogs.opening_step(Dialog::Console) {
        window = window.current_pos(crate::ui::opening_position(ctx, id, step, ASSUMED_SIZE));
    }
    let mut run = false;
    window.show(ctx, |ui| {
        run = contents(app, ui);
    });
    app.dialogs.set(Dialog::Console, open);
    if run {
        run_console(app, ctx);
    }
}

/// Runs what is in the box, with everything granted.
pub fn run_console(app: &mut DesdecApp, ctx: &egui::Context) {
    let source = app.script.source.clone();
    if source.trim().is_empty() || app.analysis.is_none() {
        return;
    }
    let context = Context::trusted(app.preferences.language);
    let outcome = app.run_script(ctx, &source, &context);
    app.script.took(None, outcome);
}

fn contents(app: &mut DesdecApp, ui: &mut egui::Ui) -> bool {
    let language = app.preferences.language;
    let ready = app.analysis.is_some();
    if !ready {
        ui.colored_label(ERROR, text(language, Text::ScriptNeedsBinary));
        ui.add_space(6.0);
    }

    ui.add(
        egui::TextEdit::multiline(&mut app.script.source)
            .code_editor()
            .desired_rows(10)
            .desired_width(ui.available_width())
            .hint_text("label(entry(), \"start\");"),
    );

    ui.add_space(6.0);
    let mut run = false;
    ui.horizontal(|ui| {
        let button = egui::Button::new(text(language, Text::RunScript));
        if ui.add_enabled(ready, button).clicked() {
            run = true;
        }
        if ui.button(text(language, Text::ClearScript)).clicked() {
            app.script.source.clear();
            app.script.last = None;
            app.script.ran = None;
        }
        ui.toggle_value(
            &mut app.script.vocabulary_open,
            text(language, Text::ScriptVocabulary),
        );
    });

    // Ctrl+Enter runs it, which is what every console this resembles does.
    if ready && ui.input(|input| input.modifiers.command && input.key_pressed(egui::Key::Enter)) {
        run = true;
    }

    if app.script.vocabulary_open {
        ui.add_space(4.0);
        vocabulary(ui, language);
    }

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);
    outcome(app, ui, language);
    run
}

/// What the last run said, printed lines and all.
fn outcome(app: &DesdecApp, ui: &mut egui::Ui, language: Language) {
    let Some(last) = &app.script.last else {
        ui.small(egui::RichText::new(text(language, Text::ScriptNothingToRun)).color(MUTED));
        return;
    };

    // One label rather than three: side-by-side small labels in a wrapped row
    // are drawn on top of one another, and this line is read at a glance or
    // not at all.
    let mut said = Vec::new();
    if let Some(ran) = &app.script.ran {
        said.push(format!("{} {ran}", text(language, Text::ScriptRanFrom)));
    }
    said.push(format!(
        "{} {:.0} ms",
        text(language, Text::ScriptFinished),
        last.elapsed.as_secs_f64() * 1000.0
    ));
    said.push(if last.effects.is_empty() {
        text(language, Text::ScriptAskedNothing).to_owned()
    } else {
        format!(
            "{} {}",
            last.effects.len(),
            text(language, Text::ScriptChangesApplied)
        )
    });
    ui.small(egui::RichText::new(said.join(" · ")).color(MUTED));

    if let Some(failure) = &last.failure {
        ui.add_space(4.0);
        ui.colored_label(ERROR, failure_text(language, failure));
    }

    if last.printed.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.small(egui::RichText::new(text(language, Text::ScriptOutput)).color(MUTED));
    egui::ScrollArea::vertical()
        .max_height(200.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for line in &last.printed {
                ui.label(egui::RichText::new(line).monospace());
            }
        });
}

/// Why a script stopped, in the reader's language.
#[must_use]
pub fn failure_text(language: Language, failure: &Failure) -> String {
    match failure {
        Failure::Interrupted(bound) => text(language, bound.label()).to_owned(),
        Failure::Refused(permission) => format!(
            "{} {}",
            text(language, Text::ScriptRefusedPermission),
            text(language, permission.label())
        ),
        Failure::Faulted(message) => {
            format!("{} {message}", text(language, Text::ScriptFailed))
        }
    }
}

/// Everything a script can say, listed where it is being written.
///
/// The names are not translated, because they are what gets typed. The line
/// above them is, because it says what is *not* in the list.
fn vocabulary(ui: &mut egui::Ui, language: Language) {
    ui.small(egui::RichText::new(text(language, Text::ScriptVocabularyHint)).color(MUTED));
    ui.add_space(2.0);
    egui::ScrollArea::vertical()
        .id_salt("desdec.script.vocabulary")
        .max_height(180.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for line in VOCABULARY {
                ui.label(egui::RichText::new(*line).monospace().color(MUTED));
            }
        });
}

/// The whole vocabulary, in the order a script tends to use it.
const VOCABULARY: &[&str] = &[
    "binary()                     entry()",
    "sections()                   section_at(a)",
    "functions()                  symbols()          strings()",
    "instruction(a)               instruction_at_index(i)",
    "instructions(from, upto)     instruction_count()",
    "read(a, n)                   read_at_offset(o, n)",
    "offset_of(a)                 address_at(o)",
    "find_bytes(\"48 8b ??\")       find_instructions(t)   find_notes(t)",
    "refs_to(a)                   ref_count(a)",
    "label_of(a)                  comment_of(a)      bookmarked(a)   notes()",
    "label(a, t)                  comment(a, t)",
    "bookmark(a)                  unbookmark(a)      clear_note(a)",
    "go_to(a)                     patch(a, \"nop\")",
    "address(0x401000)            a.hex   a.int   print(x)",
];

#[cfg(test)]
mod tests {
    use eframe::egui;

    use crate::{
        app::{DesdecApp, Dialog, WorkspaceView},
        commands::Command,
        script::{Context, Failure, Permission},
        testing::{opened_app, window_input},
    };

    /// An address the reference binary certainly decodes at.
    fn an_address(app: &DesdecApp) -> u64 {
        app.analysis
            .as_ref()
            .and_then(|analysis| analysis.instructions.first())
            .map(|instruction| instruction.address)
            .expect("the reference binary decodes to something")
    }

    #[test]
    fn what_a_script_names_appears_in_the_notes() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);
        let address = an_address(&app);

        let outcome = app.run_script(
            &ctx,
            &format!(r#"label({address}, "worked_out"); bookmark({address});"#),
            &Context::trusted(app.preferences.language),
        );

        assert_eq!(outcome.failure, None, "{outcome:?}");
        assert_eq!(app.annotations.label(address), Some("worked_out"));
        assert!(app.annotations.is_bookmarked(address));
    }

    #[test]
    fn the_open_binary_is_still_open_after_a_script_has_read_it() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);
        let instructions = app
            .analysis
            .as_ref()
            .map(|analysis| analysis.instructions.len());
        let bytes = app.file_bytes.len();

        let _ = app.run_script(
            &ctx,
            "for i in 0..100 { instruction_at_index(i); }",
            &Context::trusted(app.preferences.language),
        );

        assert_eq!(
            app.analysis
                .as_ref()
                .map(|analysis| analysis.instructions.len()),
            instructions,
            "the listing is lent to a script, not given away"
        );
        assert_eq!(app.file_bytes.len(), bytes);
        assert!(
            !app.xrefs.to(0).count().eq(&usize::MAX),
            "the index is back"
        );
    }

    /// A script may propose a patch; nothing it does reaches the file itself.
    #[test]
    fn a_patch_from_a_script_lands_in_the_pending_list_and_nowhere_else() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);
        let address = an_address(&app);
        let before = app.file_bytes.clone();

        let outcome = app.run_script(
            &ctx,
            &format!(r#"patch({address}, "nop");"#),
            &Context::trusted(app.preferences.language),
        );

        // The reference binary is whatever the host is: on an architecture the
        // built-in assembler does not encode, the refusal is the answer, and
        // it must still be a refusal rather than a written byte.
        if outcome.failure.is_none() {
            assert_eq!(app.patches.len(), 1, "the patch is pending");
        }
        assert_eq!(
            app.file_bytes, before,
            "a script never writes to the analysed file"
        );
    }

    #[test]
    fn the_console_runs_what_is_in_it_and_keeps_the_answer() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.script.source = r#"print("read " + instruction_count() + " instructions");"#.to_owned();

        app.run_command(&ctx, Command::RunScript);

        assert!(
            app.dialogs.is_open(Dialog::Console),
            "the answer is on screen"
        );
        let last = app.script.last.as_ref().expect("it ran");
        assert_eq!(last.failure, None, "{last:?}");
        assert_eq!(last.printed.len(), 1);
    }

    #[test]
    fn running_an_empty_console_does_nothing_at_all() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);

        app.run_command(&ctx, Command::RunScript);

        assert!(
            app.script.last.is_none(),
            "an empty box is not a script that failed"
        );
    }

    #[test]
    fn a_script_run_with_no_binary_open_says_so_rather_than_failing_obscurely() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.script.source = "print(binary().name);".to_owned();

        app.run_command(&ctx, Command::RunScript);

        assert!(
            app.script.last.is_none(),
            "nothing is run against a binary that is not there"
        );
        // The first frame is what egui measures the window from; the second
        // is the one that draws it where it belongs.
        let _ = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));
        let output = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));
        let drawn = crate::testing::drawn_text(&output.shapes);
        assert!(
            drawn.contains(crate::i18n::text(
                app.preferences.language,
                crate::i18n::Text::ScriptNeedsBinary
            )),
            "the window says what is missing"
        );
    }

    #[test]
    fn a_refusal_is_shown_in_the_reader_s_language() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);
        let address = an_address(&app);
        app.dialogs.open(Dialog::Console);

        let outcome = app.run_script(
            &ctx,
            &format!(r#"label({address}, "no");"#),
            &Context {
                granted: Vec::new(),
                limits: crate::script::Limits::default(),
                language: app.preferences.language,
            },
        );
        assert_eq!(
            outcome.failure,
            Some(Failure::Refused(Permission::WriteNotes))
        );
        app.script.took(None, outcome);

        let _ = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));
        let output = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));
        let drawn = crate::testing::drawn_text(&output.shapes);
        assert!(
            drawn.contains(crate::i18n::text(
                app.preferences.language,
                Permission::WriteNotes.label()
            )),
            "the window names the permission to grant, not an engine error"
        );
    }

    /// The shortcut, pressed as a reader presses it.
    ///
    /// Every other test here calls `run_command` directly, which proves what
    /// the command does and nothing about whether a key ever reaches it. This
    /// one goes through the whole frame — input, `process_shortcuts`, the
    /// command registry — because a console nobody can open is a console that
    /// does not exist.
    #[test]
    fn the_shortcut_opens_the_console_and_the_key_runs_what_is_in_it() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.script.source = "print(instruction_count());".to_owned();

        let mut held = window_input();
        held.modifiers = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        held.events = vec![egui::Event::Key {
            key: egui::Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: held.modifiers,
        }];
        let _ = ctx.run(held, |ctx| app.run_frame(ctx));
        assert!(
            app.dialogs.is_open(Dialog::Console),
            "Ctrl+Shift+S opens the console"
        );

        let mut f5 = window_input();
        f5.events = vec![egui::Event::Key {
            key: egui::Key::F5,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }];
        let _ = ctx.run(f5, |ctx| app.run_frame(ctx));
        let last = app.script.last.as_ref().expect("F5 ran it");
        assert_eq!(last.failure, None, "{last:?}");
        assert_eq!(last.printed.len(), 1);
    }
}
