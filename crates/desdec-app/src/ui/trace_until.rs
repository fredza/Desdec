//! Running until the state says so.
//!
//! x64dbg's conditional trace. Between "step once" and "run until something
//! stops it" there is the thing a reader actually wants most of the time:
//! carry on until `rax` holds what I am looking for, until this counter
//! reaches zero, until the pointer being written is the one I am watching.
//! Without it that is a step key pressed several thousand times.
//!
//! Two fields, because a condition that never holds must cost what the reader
//! agreed to and not the whole budget: what to run until, and how many
//! instructions at most. The condition is asked of the machine's state before
//! each instruction and never changes it — the language is the one breakpoint
//! conditions are written in, and it has no way to assign to anything.

use desdec_core::emulate::condition::Expression;
use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::{Text, text},
    journal::Level,
    ui::{ERROR, MUTED},
};

/// Size assumed the first time the window opens, before egui has measured it.
const ASSUMED_SIZE: egui::Vec2 = egui::vec2(480.0, 220.0);

/// How many instructions a trace is allowed by default.
const DEFAULT_LIMIT: u64 = 100_000;

/// What the window holds between openings.
pub struct State {
    /// The condition, as the reader wrote it.
    pub condition: String,
    /// How many instructions at most.
    pub limit: u64,
    /// Why the condition could not be read, when it could not.
    refused: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            condition: String::new(),
            limit: DEFAULT_LIMIT,
            refused: None,
        }
    }
}

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::TraceUntil) {
        return;
    }
    let mut open = true;
    let mut window = egui::Window::new(app.t(Text::TraceUntil))
        .id(egui::Id::new("desdec.trace-until"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_size(ASSUMED_SIZE);
    window = crate::ui::centred(
        window,
        ctx,
        app.dialogs.opening_step(Dialog::TraceUntil).is_some(),
    );

    let mut start = false;
    window.show(ctx, |ui| {
        start = contents(app, ui);
    });

    app.dialogs.set(Dialog::TraceUntil, open);
    if start {
        run(app);
        app.dialogs.close(Dialog::TraceUntil);
    }
}

/// Returns whether the reader asked for the trace to start.
fn contents(app: &mut DesdecApp, ui: &mut egui::Ui) -> bool {
    let language = app.preferences.language;
    ui.label(egui::RichText::new(text(language, Text::TraceUntilHelp)).color(MUTED));
    ui.add_space(10.0);

    ui.add(
        egui::TextEdit::singleline(&mut app.trace_until.condition)
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Monospace)
            .hint_text("rax == 0"),
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(text(language, Text::TraceLimit));
        ui.add(
            egui::DragValue::new(&mut app.trace_until.limit)
                .speed(1000.0)
                .range(1..=desdec_core::emulate::RUN_BUDGET),
        );
    });

    if let Some(error) = &app.trace_until.refused {
        ui.add_space(6.0);
        ui.colored_label(ERROR, error);
    }

    ui.add_space(12.0);
    ui.add_enabled(
        !app.trace_until.condition.trim().is_empty(),
        egui::Button::new(text(language, Text::TraceUntil)),
    )
    .clicked()
}

/// Reads the condition and runs, or says why it could not be read.
fn run(app: &mut DesdecApp) {
    let language = app.preferences.language;
    let source = app.trace_until.condition.clone();
    let limit = app.trace_until.limit;
    let parsed = {
        let names = &app.names;
        Expression::parse_naming(&source, &|name| names.address_of(name))
    };
    let parsed = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            let said = format!(
                "{} {error}",
                text(language, Text::BreakpointConditionRefused)
            );
            app.trace_until.refused = Some(said.clone());
            app.note(Level::Failure, said);
            return;
        }
    };
    app.trace_until.refused = None;
    app.note(
        Level::Note,
        format!("{} — {source}", text(language, Text::TraceStarted)),
    );

    let reached_the_limit = {
        let Some(machine) = app.machine() else {
            return;
        };
        machine.run_until(&parsed, limit);
        matches!(
            machine.stop(),
            Some(desdec_core::emulate::Stop::OutOfBudget)
        )
    };
    if reached_the_limit {
        // Said out loud: a run that stopped because the reader's limit ran out
        // looks exactly like one that stopped because the condition held, and
        // the two mean opposite things.
        app.note(Level::Note, text(language, Text::TraceReachedItsLimit));
    }
    app.follow_the_run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::WorkspaceView;

    /// The trace stops on the state the reader described, and the machine is
    /// left standing there.
    #[test]
    fn a_trace_runs_until_the_condition_it_was_given_holds() {
        let mut app = crate::testing::emulatable_sample().opened(WorkspaceView::Machine);
        // Something true a little way into any run, and not at its start.
        app.trace_until.condition = String::from("rsp != 0");
        app.trace_until.limit = 500;
        run(&mut app);
        let machine = app.machine.as_ref().expect("a machine was built");
        assert!(machine.executed() > 0, "the trace moved");
        assert!(app.trace_until.refused.is_none());
    }

    /// A condition that cannot be read is refused, by name and position, and
    /// nothing is run on it.
    #[test]
    fn a_condition_that_does_not_parse_is_refused_and_nothing_runs() {
        let mut app = crate::testing::emulatable_sample().opened(WorkspaceView::Machine);
        app.trace_until.condition = String::from("rax ==");
        run(&mut app);
        assert!(app.trace_until.refused.is_some(), "it was refused");
        assert!(
            app.machine.is_none(),
            "a condition that cannot be read never builds a machine"
        );
    }

    /// A condition that never holds costs the limit and says so, rather than
    /// looking like a run that stopped for a reason.
    #[test]
    fn a_condition_that_never_holds_stops_at_the_limit() {
        let mut app = crate::testing::emulatable_sample().opened(WorkspaceView::Machine);
        app.trace_until.condition = String::from("rip == 1");
        app.trace_until.limit = 40;
        run(&mut app);
        let machine = app.machine.as_ref().expect("a machine was built");
        assert!(
            machine.executed() <= 40,
            "it spent no more than it was given"
        );
    }
}
