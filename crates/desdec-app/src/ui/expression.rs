//! The reader's own arithmetic over the machine's state.
//!
//! x64dbg's calculator, over exactly the language its breakpoint conditions
//! are written in — which here is [`desdec_core::emulate::condition`]. One
//! language for the whole tool: what a reader learns writing a condition they
//! can use here, and what they work out here they can paste into a condition.
//!
//! Two things are on screen at once, because they are the same question asked
//! twice. The top is one expression and what it is worth right now, in every
//! base a reader might want it in. Below it are the watches: expressions read
//! again at every pause, which is how a value is followed through a run rather
//! than asked about once.
//!
//! Nothing here runs anything. An expression is asked of the state the
//! emulated machine is in, and asking never changes it: see the module the
//! language lives in. An expression that reads memory nothing maps has **no
//! value**, and is said to have none rather than answered with zero.

use desdec_core::emulate::{Machine, condition::Expression};
use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::{Language, Text, text},
    ui::{ERROR, MUTED},
};

/// Size assumed the first time the window opens, before egui has measured it.
const ASSUMED_SIZE: egui::Vec2 = egui::vec2(520.0, 420.0);

/// What is written in the window, and what it last answered.
#[derive(Default)]
pub struct State {
    /// What the reader has typed.
    pub source: String,
    /// Why the last thing typed could not be read, when it could not.
    refused: Option<String>,
}

/// One expression the reader is following through a run.
pub struct Watch {
    /// What was written, kept so it can be shown and edited again.
    pub source: String,
    /// The same, read. A watch that does not parse is never added, so this is
    /// never a silent failure.
    parsed: Expression,
}

impl Watch {
    /// What this watch is worth right now, and `None` when the machine has not
    /// been started or part of the expression could not be read.
    #[must_use]
    pub fn value(&self, machine: Option<&Machine>) -> Option<u64> {
        let machine = machine?;
        self.parsed.value(&machine.registers, &machine.memory)
    }
}

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::Expression) {
        return;
    }
    let mut open = true;
    let mut window = egui::Window::new(app.t(Text::Expression))
        .id(egui::Id::new("desdec.expression"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(ASSUMED_SIZE);
    window = crate::ui::centred(
        window,
        ctx,
        app.dialogs.opening_step(Dialog::Expression).is_some(),
    );

    let mut go_to = None;
    window.show(ctx, |ui| {
        go_to = contents(app, ui);
    });

    app.dialogs.set(Dialog::Expression, open);
    if let Some(address) = go_to {
        app.go_to_address(ctx, address);
    }
}

/// Returns an address the reader asked the listing to show.
fn contents(app: &mut DesdecApp, ui: &mut egui::Ui) -> Option<u64> {
    let language = app.preferences.language;
    ui.label(egui::RichText::new(text(language, Text::ExpressionHelp)).color(MUTED));
    ui.add_space(8.0);

    let typed = ui.add(
        egui::TextEdit::singleline(&mut app.expression.source)
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Monospace)
            .hint_text("rax + [rsp]:8"),
    );
    let submitted =
        typed.lost_focus() && ui.ctx().input(|input| input.key_pressed(egui::Key::Enter));

    // Read on every change rather than only on Enter: the answer to an
    // expression is what the window is for, and having to press a key to see
    // it makes the window feel like a form.
    let read = read_now(app);
    ui.add_space(8.0);

    let mut go_to = None;
    match &read {
        Reading::Empty => {}
        Reading::Refused(error) => {
            ui.colored_label(ERROR, error);
        }
        Reading::NoValue => {
            ui.colored_label(ERROR, text(language, Text::ExpressionHasNoValue));
            if app.machine.is_none() {
                ui.label(
                    egui::RichText::new(text(language, Text::ExpressionNeedsAMachine)).color(MUTED),
                );
            }
        }
        Reading::Value(value) => {
            go_to = value_rows(ui, *value, language);
        }
    }

    ui.add_space(10.0);
    if ui
        .add_enabled(
            matches!(read, Reading::Value(_)) || submitted,
            egui::Button::new(text(language, Text::AddWatch)),
        )
        .clicked()
    {
        add_watch(app);
    }

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);
    ui.strong(text(language, Text::Watches));
    ui.add_space(6.0);
    watches(app, ui, language);
    go_to
}

/// What the window has to say about what is typed in it.
enum Reading {
    Empty,
    /// It could not be read, and this says where it stopped making sense.
    Refused(String),
    /// It was read, and part of it could not be answered.
    NoValue,
    Value(u64),
}

fn read_now(app: &mut DesdecApp) -> Reading {
    if app.expression.source.trim().is_empty() {
        app.expression.refused = None;
        return Reading::Empty;
    }
    let names = &app.names;
    let parsed =
        match Expression::parse_naming(&app.expression.source, &|name| names.address_of(name)) {
            Ok(parsed) => parsed,
            Err(error) => {
                let said = error.to_string();
                app.expression.refused = Some(said.clone());
                return Reading::Refused(said);
            }
        };
    app.expression.refused = None;
    // The machine as it stands, without building one: opening this window must
    // not start anything, and an expression of numbers alone answers without a
    // machine at all.
    if let Some(machine) = app.machine.as_ref() {
        return parsed
            .value(&machine.registers, &machine.memory)
            .map_or(Reading::NoValue, Reading::Value);
    }
    // No machine: an expression of numbers alone still answers, and a register
    // in one has no value rather than a value of zero.
    let registers = desdec_core::emulate::registers::Registers::new();
    let memory = desdec_core::emulate::memory::Memory::new(std::sync::Arc::from(Vec::new()));
    parsed
        .value(&registers, &memory)
        .map_or(Reading::NoValue, Reading::Value)
}

/// One value in every base a reader might want it in, and a way to go there.
fn value_rows(ui: &mut egui::Ui, value: u64, language: Language) -> Option<u64> {
    let mut go_to = None;
    egui::Grid::new("expression_value")
        .num_columns(2)
        .spacing([18.0, 6.0])
        .show(ui, |ui| {
            for (label, written) in [
                (Text::Address, format!("{value:#018x}")),
                (Text::Unsigned, value.to_string()),
                #[expect(
                    clippy::cast_possible_wrap,
                    reason = "reading the same bits as signed is the point of the row"
                )]
                (Text::Signed, (value as i64).to_string()),
                (Text::Binary, format!("{value:#b}")),
            ] {
                ui.strong(text(language, label));
                crate::ui::monospace_value(ui, &written);
                ui.end_row();
            }
        });
    ui.add_space(8.0);
    if ui.button(text(language, Text::GoToThisAddress)).clicked() {
        go_to = Some(value);
    }
    go_to
}

/// Adds what is typed as a watch, if it can be read.
fn add_watch(app: &mut DesdecApp) {
    let source = app.expression.source.trim().to_owned();
    if source.is_empty() {
        return;
    }
    let names = &app.names;
    let Ok(parsed) = Expression::parse_naming(&source, &|name| names.address_of(name)) else {
        return;
    };
    // The same expression twice is one watch: a reader pressing the button
    // again means "keep this", not "keep two of these".
    if app.watches.iter().any(|watch| watch.source == source) {
        return;
    }
    app.watches.push(Watch { source, parsed });
}

/// Adds what is typed as a watch, for the sheet that draws this window.
#[cfg(test)]
pub fn watch_for_a_sheet(app: &mut DesdecApp) {
    add_watch(app);
}

/// The watches, each with what it is worth right now.
fn watches(app: &mut DesdecApp, ui: &mut egui::Ui, language: Language) {
    if app.watches.is_empty() {
        ui.label(egui::RichText::new(text(language, Text::NoWatches)).color(MUTED));
        return;
    }
    let mut remove = None;
    // Read against the machine as it stands; building one is a run, and
    // looking at a list of watches is not.
    let machine = app.machine.as_ref();
    egui::Grid::new("expression_watches")
        .num_columns(3)
        .striped(true)
        // Room for an expression before it is shortened: `rax * 2` cut to
        // `rax …` is a list that says nothing about what is in it.
        .min_col_width(180.0)
        .spacing([14.0, 6.0])
        .show(ui, |ui| {
            for (index, watch) in app.watches.iter().enumerate() {
                crate::ui::monospace_value(ui, &watch.source);
                match watch.value(machine) {
                    Some(value) => {
                        crate::ui::monospace_value(ui, &format!("{value:#x}"));
                    }
                    None => {
                        ui.label(egui::RichText::new("—").color(MUTED));
                    }
                }
                if ui.small_button("×").clicked() {
                    remove = Some(index);
                }
                ui.end_row();
            }
        });
    if let Some(index) = remove {
        app.watches.remove(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::WorkspaceView, testing::window_input};

    /// The window answers arithmetic before anything has been run: a reader
    /// converting a number should not have to start a machine first.
    #[test]
    fn plain_arithmetic_is_answered_without_a_machine() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.dialogs.open(Dialog::Expression);
        app.expression.source = String::from("0x10 * 4 + 2");
        assert!(app.machine.is_none(), "nothing has been run");

        let mut draw = |ctx: &egui::Context| show(&mut app, ctx);
        let _ = ctx.run(window_input(), &mut draw);
        let output = ctx.run(window_input(), &mut draw);
        let said = crate::testing::drawn_text(&output.shapes);
        assert!(
            said.contains("66"),
            "the decimal answer is on screen: {said}"
        );
        assert!(said.contains("0x42") || said.contains("42"), "{said}");
    }

    /// What does not parse is said, with where it stopped making sense, rather
    /// than answered with a number that was never asked for.
    #[test]
    fn what_cannot_be_read_is_refused_rather_than_answered() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.dialogs.open(Dialog::Expression);
        app.expression.source = String::from("rax +");

        let mut draw = |ctx: &egui::Context| show(&mut app, ctx);
        let _ = ctx.run(window_input(), &mut draw);
        let output = ctx.run(window_input(), &mut draw);
        let said = crate::testing::drawn_text(&output.shapes);
        assert!(
            !said.contains("0x0000000000000000"),
            "nothing was answered: {said}"
        );
    }

    /// A watch is kept once however many times the button is pressed.
    #[test]
    fn the_same_expression_is_watched_once() {
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.expression.source = String::from("rsp");
        add_watch(&mut app);
        add_watch(&mut app);
        assert_eq!(app.watches.len(), 1);
    }

    /// A watch that cannot be read is never added: the list holds expressions,
    /// not attempts at them.
    #[test]
    fn a_watch_that_does_not_parse_is_not_kept() {
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.expression.source = String::from("rax +");
        add_watch(&mut app);
        assert!(app.watches.is_empty());
    }

    /// Without a machine a register has no value — not zero, which would read
    /// as a measurement of something that has not run.
    #[test]
    fn a_register_has_no_value_before_anything_has_run() {
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.expression.source = String::from("rsp");
        add_watch(&mut app);
        assert_eq!(app.watches[0].value(None), None);
    }
}
