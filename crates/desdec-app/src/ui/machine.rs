//! The emulated processor: what it holds, and what it is doing.
//!
//! Everything on screen here is a measurement rather than a reading. The
//! register values are the values, the stack is the stack, and a loop's trip
//! count is a fact — none of it is the careful "this is what the bytes say,
//! and a branch could make it wrong" that the rest of the tool has to say,
//! because something actually ran.
//!
//! What it ran on is not your processor, and the view says so where it
//! matters: the line under the title, and every stop that names why the run
//! could go no further. A reader must never be left thinking Desdec attached
//! to a process.

use desdec_core::emulate::{
    Machine, Stop, Watchpoint,
    memory::{Access, Fault},
    registers::Flag,
};
use eframe::egui;

use crate::{
    app::DesdecApp,
    i18n::{Language, Text, text},
    icons::{self, Icon},
    preferences::accent,
    ui::{ERROR, MUTED, ROW_HEIGHT, card, columns},
};

/// The colour a breakpoint is drawn in, here and in the listing's margin.
pub const BREAKPOINT: egui::Color32 = egui::Color32::from_rgb(226, 92, 92);
/// The colour the instruction about to run is drawn in.
pub const CURRENT: egui::Color32 = egui::Color32::from_rgb(120, 196, 132);
/// How many rows of the trace and of the stack are shown at once.
const ROWS: usize = 14;

pub fn show(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    if app.analysis.is_none() {
        return;
    }
    ui.label(egui::RichText::new(text(language, Text::MachineExplained)).color(MUTED));
    ui.add_space(10.0);

    transport(app, ui);
    ui.add_space(6.0);
    start_at_function(app, ui);
    ui.add_space(10.0);

    let Some(machine) = app.machine() else {
        return;
    };
    // Read out of the machine before the panes borrow it, so each pane can
    // take it immutably without the state line having to be redrawn from it.
    let pointer = machine.instruction_pointer();
    let executed = machine.executed();
    let depth = machine.depth();
    let pages = machine.memory.written_pages();
    let stop = machine.stop().cloned();

    state_line(ui, language, stop.as_ref(), pointer, executed, depth, pages);
    ui.add_space(10.0);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let Some(machine) = &app.machine else {
                return;
            };
            columns(
                ui,
                |ui| {
                    card(ui, text(language, Text::Registers), |ui| {
                        registers(ui, machine, language);
                    });
                    ui.add_space(8.0);
                    card(ui, text(language, Text::CallStack), |ui| {
                        call_stack(ui, machine, language);
                    });
                    ui.add_space(8.0);
                    card(ui, text(language, Text::Stack), |ui| {
                        stack(ui, machine, language);
                    });
                },
                |ui| {
                    card(ui, text(language, Text::ExecutionTrace), |ui| {
                        trace(ui, machine, language);
                    });
                    ui.add_space(8.0);
                    card(ui, text(language, Text::MappedRegions), |ui| {
                        regions(ui, machine);
                    });
                },
            );
            ui.add_space(8.0);
            columns(
                ui,
                |ui| {
                    card(ui, text(language, Text::Breakpoints), |ui| {
                        breakpoints(ui, machine, language);
                    });
                },
                |ui| {
                    card(ui, text(language, Text::Watchpoints), |ui| {
                        watchpoints(ui, machine, language);
                    });
                },
            );
        });
}

/// The six buttons of a debugger's transport, in the order they sit on one.
fn transport(app: &mut DesdecApp, ui: &mut egui::Ui) {
    use crate::commands::Command;

    let ctx = ui.ctx().clone();
    let tint = accent(app.preferences.theme);
    let buttons: &[(Icon, Command)] = &[
        (Icon::Run, Command::MachineRun),
        (Icon::WalkInto, Command::MachineStepInto),
        (Icon::WalkOver, Command::MachineStepOver),
        (Icon::WalkOut, Command::MachineStepOut),
        (Icon::Breakpoint, Command::MachineToggleBreakpoint),
        (Icon::Restart, Command::MachineRestart),
    ];
    let mut chosen = None;
    ui.horizontal(|ui| {
        for (icon, command) in buttons {
            let enabled = app.can_run(*command);
            let tooltip = app.optional_command_tooltip(*command);
            ui.add_enabled_ui(enabled, |ui| {
                if icons::button(ui, *icon, tooltip, false, tint).clicked() {
                    chosen = Some(*command);
                }
            });
        }
    });
    if let Some(command) = chosen {
        app.run_command(&ctx, command);
    }
}

/// Starting the run at a function rather than at the entry point.
///
/// What makes the emulation useful on a file that will not run from its entry
/// point at all: a dynamically linked program calls into a library on its
/// third instruction, and a single function of it usually does not.
fn start_at_function(app: &mut DesdecApp, ui: &mut egui::Ui) {
    use desdec_core::emulate::Convention;

    let language = app.preferences.language;
    let selected = app.selected_function;
    let mut chosen = app.machine_convention;
    let mut start = false;
    ui.horizontal(|ui| {
        ui.add_enabled_ui(selected.is_some(), |ui| {
            if ui.button(text(language, Text::StartAtFunction)).clicked() {
                start = true;
            }
        });
        if let Some(address) = selected {
            ui.monospace(egui::RichText::new(format!("{address:#018x}")).color(MUTED));
        }
        ui.separator();
        egui::ComboBox::from_id_salt("machine-convention")
            .selected_text(chosen.label())
            .show_ui(ui, |ui| {
                for convention in [Convention::SystemV, Convention::Microsoft] {
                    ui.selectable_value(&mut chosen, convention, convention.label());
                }
            });
        ui.label(egui::RichText::new(text(language, Text::CallingConvention)).color(MUTED))
            .on_hover_text(text(language, Text::CallingConventionHelp));
    });
    app.machine_convention = chosen;
    if start
        && let Some(address) = selected
        && let Some(machine) = app.machine()
    {
        // No arguments: the reader can put values in the registers the
        // convention names before pressing anything, and a made-up argument
        // would be exactly the kind of invention this tool does not do.
        machine.call_function(address, &[], chosen);
        app.follow_the_run();
    }
}

/// One line saying where the run is and why it is not running.
fn state_line(
    ui: &mut egui::Ui,
    language: Language,
    stop: Option<&Stop>,
    pointer: u64,
    executed: u64,
    depth: i64,
    pages: usize,
) {
    // `horizontal`, never `horizontal_wrapped`: in a wrapped row egui 0.31
    // draws every one of these at the same position, so four facts and their
    // four values come out as one smear. It is the same fault that put a
    // plugin's version on top of its author, and it is not confined to small
    // text — a `label` and a `monospace` do it too. The row is short enough to
    // fit, and a test holds it to that.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(text(language, Text::NextInstruction)).color(MUTED));
        ui.monospace(egui::RichText::new(format!("{pointer:#018x}")).color(CURRENT));
        ui.separator();
        ui.label(egui::RichText::new(text(language, Text::InstructionsExecuted)).color(MUTED));
        ui.monospace(executed.to_string());
        ui.separator();
        ui.label(egui::RichText::new(text(language, Text::CallDepth)).color(MUTED));
        ui.monospace(depth.to_string());
        ui.separator();
        ui.label(egui::RichText::new(text(language, Text::PagesWritten)).color(MUTED));
        ui.monospace(pages.to_string());
    });
    ui.add_space(6.0);
    match stop {
        None if executed == 0 => {
            ui.label(egui::RichText::new(text(language, Text::MachineNotStarted)).color(MUTED));
        }
        None => {}
        Some(stop) => {
            let (sentence, grave) = explain(stop, language);
            let colour = if grave { ERROR } else { MUTED };
            ui.label(egui::RichText::new(sentence).color(colour));
        }
    }
}

/// What a stop says, in the reader's language, and whether it ended the run
/// for good rather than merely paused it.
fn explain(stop: &Stop, language: Language) -> (String, bool) {
    let say = |item: Text| text(language, item).to_owned();
    match stop {
        Stop::Breakpoint { address } => (
            format!("{} — {address:#018x}", say(Text::StoppedAtBreakpoint)),
            false,
        ),
        Stop::Watchpoint {
            address,
            at,
            access,
        } => {
            let how = say(match access {
                Access::Read => Text::ReadAccess,
                Access::Write => Text::WriteAccess,
                Access::Execute => Text::ExecuteAccess,
            });
            (
                format!(
                    "{} — {address:#018x} ({how}), {at:#018x}",
                    say(Text::StoppedAtWatchpoint)
                ),
                false,
            )
        }
        Stop::Finished => (say(Text::StoppedFinished), false),
        Stop::Paused => (say(Text::StoppedPaused), false),
        Stop::OutOfBudget => (say(Text::StoppedBudget), false),
        Stop::Unsupported { at, instruction } => (
            format!(
                "{} — {instruction} ({at:#018x})",
                say(Text::StoppedUnsupported)
            ),
            true,
        ),
        Stop::SystemCall { at, instruction } => (
            format!(
                "{} — {instruction} ({at:#018x})",
                say(Text::StoppedSystemCall)
            ),
            true,
        ),
        Stop::LeftTheImage { at } => (
            format!("{} — {at:#018x}", say(Text::StoppedLeftImage)),
            true,
        ),
        Stop::UnresolvedCall { at } => (
            format!("{} — {at:#018x}", say(Text::StoppedUnresolvedCall)),
            true,
        ),
        Stop::Fault { at, fault } => {
            let (what, address) = match fault {
                Fault::Unmapped { address } => (Text::StoppedFaultUnmapped, address),
                Fault::Protection { address, .. } => (Text::StoppedFaultProtection, address),
            };
            (format!("{} — {address:#018x}, {at:#018x}", say(what)), true)
        }
        Stop::DivideError { at } => (format!("{} — {at:#018x}", say(Text::StoppedDivide)), true),
        Stop::Halted { at, instruction } => (
            format!("{} — {instruction} ({at:#018x})", say(Text::StoppedHalted)),
            true,
        ),
        Stop::Undecodable { at } => (
            format!("{} — {at:#018x}", say(Text::StoppedUndecodable)),
            true,
        ),
        Stop::UnsupportedArchitecture { architecture } => (
            format!(
                "{} — {}",
                say(Text::StoppedArchitecture),
                architecture.label()
            ),
            true,
        ),
    }
}

/// The sixteen general-purpose registers, the pointer, and the flags.
fn registers(ui: &mut egui::Ui, machine: &Machine, language: Language) {
    egui::Grid::new("machine-registers")
        .num_columns(4)
        .spacing([18.0, 4.0])
        .show(ui, |ui| {
            let values: Vec<(&str, u64)> = machine.registers.general().collect();
            for pair in values.chunks(2) {
                for (name, value) in pair {
                    ui.monospace(egui::RichText::new(*name).color(MUTED));
                    ui.monospace(format!("{value:016x}"));
                }
                ui.end_row();
            }
            ui.monospace(egui::RichText::new("rip").color(MUTED));
            ui.monospace(
                egui::RichText::new(format!("{:016x}", machine.instruction_pointer()))
                    .color(CURRENT),
            );
            ui.monospace(egui::RichText::new("rflags").color(MUTED));
            ui.monospace(format!("{:016x}", machine.registers.rflags()));
            ui.end_row();
        });
    ui.add_space(6.0);
    // The flags on one line, each a letter that is lit or not: the shape a
    // reader recognises before they have read a single value. Not wrapped, for
    // the reason given in `state_line`.
    ui.horizontal(|ui| {
        for flag in Flag::ALL {
            let set = machine.registers.flag(flag);
            let colour = if set { CURRENT } else { MUTED };
            ui.monospace(
                egui::RichText::new(format!("{} {}", flag.short_name(), u8::from(set)))
                    .color(colour),
            );
        }
    });
    let _ = language;
}

/// The calls the run is inside, innermost first — which is the order a reader
/// asking "how did it get here?" reads them in.
fn call_stack(ui: &mut egui::Ui, machine: &Machine, language: Language) {
    let frames = machine.frames();
    if frames.is_empty() {
        ui.label(egui::RichText::new(text(language, Text::MachineNothingYet)).color(MUTED));
        return;
    }
    egui::Grid::new("machine-frames")
        .num_columns(3)
        .spacing([14.0, 4.0])
        .show(ui, |ui| {
            for frame in frames.iter().rev() {
                ui.monospace(format!("{:016x}", frame.entered));
                ui.monospace(
                    egui::RichText::new(format!("← {:016x}", frame.called_from)).color(MUTED),
                );
                ui.monospace(
                    egui::RichText::new(format!("rsp {:016x}", frame.stack_pointer)).color(MUTED),
                );
                ui.end_row();
            }
        });
}

/// What is on the stack, newest first, with the pointer's own row marked.
fn stack(ui: &mut egui::Ui, machine: &Machine, language: Language) {
    let top = machine.registers.stack_pointer();
    egui::Grid::new("machine-stack")
        .num_columns(2)
        .spacing([18.0, 4.0])
        .show(ui, |ui| {
            for slot in 0..ROWS as u64 {
                let address = top.wrapping_add(slot * 8);
                let mut bytes = [0_u8; 8];
                let mut readable = true;
                for (step, byte) in bytes.iter_mut().enumerate() {
                    match machine.memory.peek(address.wrapping_add(step as u64)) {
                        Some(value) => *byte = value,
                        None => readable = false,
                    }
                }
                let marker = if slot == 0 { "rsp" } else { "" };
                ui.monospace(
                    egui::RichText::new(format!("{marker:>3} {address:016x}"))
                        .color(if slot == 0 { CURRENT } else { MUTED }),
                );
                if readable {
                    let value = u64::from_le_bytes(bytes);
                    let cell = ui.monospace(format!("{value:016x}"));
                    // The one value on the stack that is the emulator's own,
                    // and that a reader would otherwise take for the program's.
                    if value == desdec_core::emulate::RETURN_SENTINEL {
                        cell.on_hover_text(text(language, Text::StackSentinel));
                    }
                } else {
                    ui.label(egui::RichText::new("—").color(MUTED));
                }
                ui.end_row();
            }
        });
}

/// The instructions most recently carried out, newest last.
fn trace(ui: &mut egui::Ui, machine: &Machine, language: Language) {
    let entries: Vec<_> = machine.trace().rev().take(ROWS).collect();
    if entries.is_empty() {
        ui.label(egui::RichText::new(text(language, Text::MachineNothingYet)).color(MUTED));
        return;
    }
    egui::Grid::new("machine-trace")
        .num_columns(3)
        .spacing([14.0, 4.0])
        .min_row_height(ROW_HEIGHT)
        .show(ui, |ui| {
            for entry in entries.into_iter().rev() {
                ui.monospace(egui::RichText::new(entry.ordinal.to_string()).color(MUTED));
                ui.monospace(format!("{:016x}", entry.address));
                ui.monospace(&entry.text);
                ui.end_row();
            }
        });
}

/// The address space the run sees, region by region.
fn regions(ui: &mut egui::Ui, machine: &Machine) {
    egui::Grid::new("machine-regions")
        .num_columns(3)
        .spacing([14.0, 4.0])
        .show(ui, |ui| {
            for region in machine.regions() {
                ui.monospace(&region.name);
                ui.monospace(format!("{:016x}–{:016x}", region.start, region.end()));
                ui.monospace(region.permissions.label());
                ui.end_row();
            }
        });
}

/// Every breakpoint the reader has set.
fn breakpoints(ui: &mut egui::Ui, machine: &Machine, language: Language) {
    let mut any = false;
    for address in machine.breakpoints() {
        any = true;
        ui.monospace(egui::RichText::new(format!("{address:#018x}")).color(BREAKPOINT));
    }
    if !any {
        ui.label(egui::RichText::new(text(language, Text::MachineNoBreakpoints)).color(MUTED));
    }
}

/// Every address the reader is watching, and what for.
fn watchpoints(ui: &mut egui::Ui, machine: &Machine, language: Language) {
    let watches: &[Watchpoint] = machine.watchpoints();
    if watches.is_empty() {
        ui.label(egui::RichText::new(text(language, Text::MachineNoWatchpoints)).color(MUTED));
        return;
    }
    for watch in watches {
        let mut how = Vec::new();
        if watch.on_read {
            how.push(text(language, Text::ReadAccess));
        }
        if watch.on_write {
            how.push(text(language, Text::WriteAccess));
        }
        ui.monospace(format!(
            "{:#018x} +{} — {}",
            watch.address,
            watch.size,
            how.join(", ")
        ));
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui;

    use crate::{
        app::WorkspaceView,
        testing::{drawn, window_input},
    };

    /// Draws this view alone, and returns every string it put on screen with
    /// where it landed.
    ///
    /// The view alone rather than the whole frame: the panels around it draw
    /// their own text, and a test about this view must not answer for theirs.
    fn rendered() -> Vec<(String, egui::Pos2)> {
        let ctx = egui::Context::default();
        // The x86-64 fixture rather than the host's own binary, so the view is
        // drawn holding the same thing on every runner: an Apple Silicon one
        // would draw the line saying its architecture has no interpreter, and
        // a test about the layout would be looking at a different view.
        let mut app = crate::testing::emulatable_sample().opened(WorkspaceView::Machine);
        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| super::show(&mut app, ui));
        };
        // Two frames: a panel is measured on the first and painted after.
        let _ = ctx.run(window_input(), &mut draw);
        let output = ctx.run(window_input(), &mut draw);
        drawn(&output.shapes)
    }

    /// The state line names four things and gives four values. Drawn one over
    /// another they read as a smear rather than as four facts, and no
    /// assertion about the strings themselves would ever notice: they are all
    /// on screen either way. Only where they landed says it.
    #[test]
    fn nothing_in_the_view_is_drawn_on_top_of_anything_else() {
        let mut seen: Vec<egui::Pos2> = Vec::new();
        for (said, at) in rendered() {
            assert!(
                !seen.contains(&at),
                "{said:?} is drawn on top of something else, at {at:?}"
            );
            seen.push(at);
        }
    }

    /// Opening the view builds the machine, and the machine reports the file's
    /// own entry point as the instruction about to run.
    #[test]
    fn the_view_opens_on_the_entry_point() {
        let said: String = rendered().into_iter().map(|(text, _)| text).collect();
        assert!(
            said.contains("rax") && said.contains("rsp"),
            "the registers are on screen: {said}"
        );
    }

    /// Once something has run, the view shows what ran rather than the line
    /// that says nothing has.
    #[test]
    fn a_run_puts_what_it_carried_out_on_screen() {
        let ctx = egui::Context::default();
        // The x86-64 fixture rather than the host's own binary: an Apple
        // Silicon runner's binary is AArch64, which has no interpreter here,
        // so nothing would run and the test would fail on the architecture
        // rather than on anything this view does.
        let mut app = crate::testing::emulatable_sample().opened(WorkspaceView::Machine);
        let entry = app
            .analysis
            .as_ref()
            .and_then(|analysis| analysis.entry_point)
            .expect("the reference binary has an entry point");
        let executed = {
            let machine = app.machine().expect("a machine is built");
            for _ in 0..4 {
                machine.step_one();
            }
            machine.executed()
        };
        assert!(executed > 0, "the fixture's entry point decodes and runs");
        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| super::show(&mut app, ui));
        };
        let _ = ctx.run(window_input(), &mut draw);
        let output = ctx.run(window_input(), &mut draw);
        let said: String = drawn(&output.shapes)
            .into_iter()
            .map(|(text, _)| text)
            .collect();
        assert!(
            said.contains(&format!("{entry:016x}")),
            "the entry point is in the trace: {said}"
        );
    }
}
