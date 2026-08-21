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
    system::{SystemCall, SystemPlatform},
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
    let rewindable = machine.rewindable();
    // A call that left for nowhere read its target from a slot, and the file
    // says whose address belongs there even though nothing has written it.
    let import = match &stop {
        Some(Stop::UnresolvedCall {
            through: Some(slot),
            ..
        }) => app
            .analysis
            .as_ref()
            .and_then(|analysis| analysis.import_at(*slot))
            .map(str::to_owned),
        _ => None,
    };

    state_line(
        ui,
        language,
        stop.as_ref(),
        import.as_deref(),
        &Counts {
            pointer,
            executed,
            depth,
            pages,
            rewindable,
        },
    );
    ui.add_space(10.0);

    let mut asked = BreakpointEdit::default();
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
                    if let Some(Stop::SystemCall { call, .. }) = machine.stop() {
                        card(ui, text(language, Text::SystemRequest), |ui| {
                            system_call(ui, call, language);
                        });
                        ui.add_space(8.0);
                    }
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
                        asked = breakpoints(ui, machine, language);
                    });
                },
                |ui| {
                    card(ui, text(language, Text::Watchpoints), |ui| {
                        watchpoints(ui, machine, language);
                    });
                },
            );
        });
    apply(app, ui.ctx(), asked);
}

/// What the reader asked of one breakpoint, acted on once the borrow the panes
/// were drawn under has ended.
#[derive(Default)]
struct BreakpointEdit {
    /// A breakpoint to take away.
    remove: Option<u64>,
    /// One to turn on or off.
    enable: Option<(u64, bool)>,
    /// One whose condition was typed into.
    condition: Option<(u64, String)>,
    /// One whose pass count was changed.
    skip: Option<(u64, u64)>,
    /// One to bring into view in the listing.
    show: Option<u64>,
}

/// Acts on what a row of the breakpoint pane asked for.
fn apply(app: &mut DesdecApp, ctx: &egui::Context, asked: BreakpointEdit) {
    if let Some(address) = asked.show {
        app.go_to_address(ctx, address);
    }
    let language = app.preferences.language;
    let Some(machine) = app.machine.as_mut() else {
        return;
    };
    if let Some(address) = asked.remove {
        machine.toggle_breakpoint(address);
    }
    if let Some((address, enabled)) = asked.enable
        && let Some(breakpoint) = machine.breakpoint_mut(address)
    {
        breakpoint.enabled = enabled;
    }
    if let Some((address, skip)) = asked.skip
        && let Some(breakpoint) = machine.breakpoint_mut(address)
    {
        breakpoint.skip = skip;
    }
    if let Some((address, source)) = asked.condition {
        // The file's own names are in reach here, so a condition can be
        // written about `main` rather than about the number it stands for.
        // Borrowed from a different field than the machine, which is what lets
        // both be held at once.
        let names = &app.names;
        let refused = machine.breakpoint_mut(address).and_then(|breakpoint| {
            breakpoint
                .set_condition_naming(&source, &|name| names.address_of(name))
                .err()
        });
        if let Some(error) = refused {
            // Said out loud rather than swallowed: the condition the reader
            // typed is not the one the run is using, and they have to be told
            // which one it is using.
            app.note(
                crate::journal::Level::Failure,
                format!(
                    "{} {error}",
                    text(language, Text::BreakpointConditionRefused)
                ),
            );
        }
    }
}

/// The transport of a debugger, in the order the buttons sit on one.
fn transport(app: &mut DesdecApp, ui: &mut egui::Ui) {
    use crate::commands::Command;

    let ctx = ui.ctx().clone();
    let tint = accent(app.preferences.theme);
    let buttons: &[(Icon, Command)] = &[
        (Icon::Run, Command::MachineRun),
        (Icon::WalkInto, Command::MachineStepInto),
        (Icon::WalkOver, Command::MachineStepOver),
        (Icon::WalkOut, Command::MachineStepOut),
        (Icon::WalkBack, Command::MachineStepBack),
        (Icon::Breakpoint, Command::MachineToggleBreakpoint),
        (Icon::Restart, Command::MachineRestart),
    ];
    let language = app.preferences.language;
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
        // Written out rather than drawn: it opens a window to be filled in,
        // which is a different act from the presses beside it, and a glyph for
        // "run until something I am about to describe" would be a riddle.
        ui.separator();
        if ui
            .button(text(language, Text::TraceUntil))
            .on_hover_text(text(language, Text::TraceUntilHelp))
            .clicked()
        {
            chosen = Some(Command::MachineTraceUntil);
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

/// The numbers the state line reports, gathered before the panes borrow the
/// machine to draw themselves.
struct Counts {
    pointer: u64,
    executed: u64,
    depth: i64,
    pages: usize,
    rewindable: usize,
}

/// One line saying where the run is and why it is not running.
fn state_line(
    ui: &mut egui::Ui,
    language: Language,
    stop: Option<&Stop>,
    import: Option<&str>,
    counts: &Counts,
) {
    let Counts {
        pointer,
        executed,
        depth,
        pages,
        rewindable,
    } = *counts;
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
        ui.separator();
        ui.label(egui::RichText::new(text(language, Text::RewindableSteps)).color(MUTED))
            .on_hover_text(text(language, Text::RewindExplained));
        ui.monospace(rewindable.to_string());
    });
    ui.add_space(6.0);
    match stop {
        None if executed == 0 => {
            ui.label(egui::RichText::new(text(language, Text::MachineNotStarted)).color(MUTED));
        }
        None => {}
        Some(stop) => {
            let (sentence, grave) = explain(stop, import, language);
            let colour = if grave { ERROR } else { MUTED };
            ui.label(egui::RichText::new(sentence).color(colour));
        }
    }
}

/// What a stop says, in the reader's language, and whether it ended the run
/// for good rather than merely paused it.
fn explain(stop: &Stop, import: Option<&str>, language: Language) -> (String, bool) {
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
        Stop::SystemCall {
            at,
            instruction,
            call,
        } => (
            format!(
                "{} — {} / {instruction} ({at:#018x})",
                say(Text::StoppedSystemCall),
                call.display_name(),
            ),
            true,
        ),
        Stop::LeftTheImage { at } => (
            format!("{} — {at:#018x}", say(Text::StoppedLeftImage)),
            true,
        ),
        Stop::UnresolvedCall { at, from, .. } => {
            // The address left for is zero every time, so what is worth
            // printing is the call site — where the reader has to go — and the
            // import whose address the slot it read is still missing.
            let site = match from {
                Some((from, instruction)) => format!("{instruction} ({from:#018x})"),
                None => format!("{at:#018x}"),
            };
            let named = import
                .map(|import| format!(" → {import}"))
                .unwrap_or_default();
            (
                format!("{} — {site}{named}", say(Text::StoppedUnresolvedCall)),
                true,
            )
        }
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
    ui.add_space(8.0);
    ui.label(egui::RichText::new(text(language, Text::VectorRegisters)).color(MUTED));
    egui::Grid::new("machine-vector-registers")
        .num_columns(2)
        .spacing([18.0, 4.0])
        .show(ui, |ui| {
            for (name, value) in machine.registers.vector() {
                ui.monospace(egui::RichText::new(name).color(MUTED));
                ui.monospace(format!("{value:032x}"));
                ui.end_row();
            }
        });
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

/// The operating-system boundary, decoded like a tiny `strace` line but
/// deliberately without a return value: nothing behind this emulator answers
/// the request.
fn system_call(ui: &mut egui::Ui, call: &SystemCall, language: Language) {
    ui.label(egui::RichText::new(text(language, Text::SystemRequestExplained)).color(MUTED));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.monospace(egui::RichText::new(call.display_name()).strong());
        ui.label(egui::RichText::new(call.platform.label()).color(MUTED));
        ui.label(format!(
            "{} {:#x}",
            text(language, Text::SystemRequestNumber),
            call.number
        ));
    });
    ui.add_space(4.0);
    ui.label(egui::RichText::new(text(language, Text::SystemRequestArguments)).color(MUTED));
    egui::Grid::new("machine-system-call")
        .num_columns(2)
        .spacing([14.0, 4.0])
        .show(ui, |ui| {
            for argument in call.arguments {
                ui.monospace(argument.register);
                ui.monospace(format!("{:#018x}", argument.value));
                ui.end_row();
            }
        });
    if call.platform == SystemPlatform::WindowsX86_64 {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(text(language, Text::SystemRequestWindowsNote)).color(MUTED));
    }
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

/// Every breakpoint, with what it takes for each to stop the run.
///
/// Editable in place rather than behind a window: a condition is written while
/// looking at the run it is about, and a reader who has to open a dialog to
/// change `rcx == 4` into `rcx == 3` will not bother.
fn breakpoints(ui: &mut egui::Ui, machine: &Machine, language: Language) -> BreakpointEdit {
    let mut asked = BreakpointEdit::default();
    let set: Vec<(u64, &desdec_core::emulate::Breakpoint)> = machine.breakpoints().collect();
    if set.is_empty() {
        ui.label(egui::RichText::new(text(language, Text::MachineNoBreakpoints)).color(MUTED));
        return asked;
    }
    for (address, breakpoint) in set {
        ui.horizontal(|ui| {
            let mut enabled = breakpoint.enabled;
            if ui.checkbox(&mut enabled, "").changed() {
                asked.enable = Some((address, enabled));
            }
            let colour = if breakpoint.enabled {
                BREAKPOINT
            } else {
                MUTED
            };
            if ui
                .add(
                    egui::Label::new(
                        egui::RichText::new(format!("{address:#018x}"))
                            .monospace()
                            .color(colour),
                    )
                    .sense(egui::Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                asked.show = Some(address);
            }
            if breakpoint.passes > 0 {
                ui.label(
                    egui::RichText::new(format!(
                        "{} {}",
                        text(language, Text::BreakpointPasses),
                        breakpoint.passes
                    ))
                    .small()
                    .color(MUTED),
                );
            }
            if ui
                .small_button(text(language, Text::RemoveBreakpoint))
                .clicked()
            {
                asked.remove = Some(address);
            }
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(text(language, Text::BreakpointCondition)).color(MUTED))
                .on_hover_text(text(language, Text::BreakpointConditionHelp));
            let mut source = breakpoint.condition.clone();
            let field = ui.add(
                egui::TextEdit::singleline(&mut source)
                    .hint_text(text(language, Text::BreakpointConditionHint))
                    .font(egui::TextStyle::Monospace)
                    .desired_width(260.0),
            );
            // Read when the reader has finished, not on every keystroke: a
            // half-typed `rcx ==` is not a condition that failed, it is one
            // that is not written yet.
            if field.lost_focus() && source != breakpoint.condition {
                asked.condition = Some((address, source));
            }
            ui.label(egui::RichText::new(text(language, Text::BreakpointSkip)).color(MUTED))
                .on_hover_text(text(language, Text::BreakpointSkipHelp));
            let mut skip = breakpoint.skip;
            if ui
                .add(
                    egui::DragValue::new(&mut skip)
                        .speed(1.0)
                        .range(0..=u64::MAX),
                )
                .changed()
            {
                asked.skip = Some((address, skip));
            }
        });
        ui.add_space(6.0);
    }
    asked
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
    use desdec_core::emulate::Stop;
    use desdec_core::emulate::system::{SystemArgument, SystemCall, SystemPlatform};
    use eframe::egui;

    use crate::{
        app::WorkspaceView,
        commands::Command,
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
            said.contains("rax")
                && said.contains("rsp")
                && said.contains("xmm0")
                && said.contains("xmm15"),
            "the general-purpose and XMM registers are on screen: {said}"
        );
    }

    #[test]
    fn a_system_request_shows_its_abi_without_a_made_up_result() {
        let call = SystemCall {
            platform: SystemPlatform::LinuxX86_64,
            number: 1,
            name: Some("write"),
            arguments: [
                SystemArgument {
                    register: "rdi",
                    value: 2,
                },
                SystemArgument {
                    register: "rsi",
                    value: 0x401000,
                },
                SystemArgument {
                    register: "rdx",
                    value: 7,
                },
                SystemArgument {
                    register: "r10",
                    value: 0,
                },
                SystemArgument {
                    register: "r8",
                    value: 0,
                },
                SystemArgument {
                    register: "r9",
                    value: 0,
                },
            ],
        };
        let ctx = egui::Context::default();
        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| {
                super::system_call(ui, &call, crate::i18n::Language::French);
            });
        };
        let _ = ctx.run(window_input(), &mut draw);
        let output = ctx.run(window_input(), &mut draw);
        let said: String = drawn(&output.shapes)
            .into_iter()
            .map(|(text, _)| text)
            .collect();
        assert!(said.contains("write") && said.contains("Linux x86-64"));
        assert!(said.contains("n’invente aucun résultat"), "{said}");
    }

    /// The address an unresolved call leaves for is zero every time, and the
    /// line above already says it. What the reader cannot see anywhere else is
    /// which instruction went there and what it was a call to, so that is what
    /// the sentence carries.
    #[test]
    fn an_unresolved_call_names_the_instruction_that_made_it_and_the_import() {
        let stop = Stop::UnresolvedCall {
            at: 0,
            from: Some((0x23e8, String::from("call qword ptr [4FF0h]"))),
            through: Some(0x4ff0),
        };
        let (said, grave) = super::explain(
            &stop,
            Some("__libc_start_main"),
            crate::i18n::Language::French,
        );
        assert!(
            said.contains("call qword ptr [4FF0h]")
                && said.contains("0x00000000000023e8")
                && said.contains("__libc_start_main"),
            "{said}"
        );
        assert!(grave, "the run cannot be carried on from there");

        // A file that names nothing for that slot is said as it was before,
        // rather than with an invented name or an empty arrow.
        let (said, _) = super::explain(&stop, None, crate::i18n::Language::French);
        assert!(said.ends_with("(0x00000000000023e8)"), "{said}");
    }

    /// The name the line ends with comes out of the file being read, not out
    /// of a table written here: a sample that imports something is analysed,
    /// and asked what belongs at one of its slots.
    #[test]
    fn the_import_a_stopped_call_was_to_is_read_from_the_file() {
        let sample = crate::testing::samples()
            .into_iter()
            .find(|sample| !sample.analysis.import_slots.is_empty())
            .expect("a fixture that imports something");
        let slot = sample.analysis.import_slots[0].clone();
        assert_eq!(sample.analysis.import_at(slot.address), Some(&*slot.name));
        assert_eq!(
            sample.analysis.import_at(slot.address.wrapping_add(1)),
            None,
            "an address that is no slot names nothing"
        );
    }

    /// The transport's back button undoes exactly one instruction, through the
    /// command a key is bound to.
    #[test]
    fn the_back_button_takes_the_run_back_one_instruction() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::emulatable_sample().opened(WorkspaceView::Machine);
        for _ in 0..4 {
            app.run_command(&ctx, Command::MachineStepInto);
        }
        let machine = app.machine().expect("a machine");
        let executed = machine.executed();
        let pointer = machine.instruction_pointer();
        assert!(executed >= 4, "the fixture's entry point runs");

        app.run_command(&ctx, Command::MachineStepBack);
        let machine = app.machine().expect("a machine");
        assert_eq!(machine.executed(), executed - 1, "exactly one");
        assert_ne!(machine.instruction_pointer(), pointer);

        app.run_command(&ctx, Command::MachineStepInto);
        let machine = app.machine().expect("a machine");
        assert_eq!(machine.executed(), executed, "and forward again");
        assert_eq!(machine.instruction_pointer(), pointer);
    }

    /// A condition typed into a breakpoint is what the run then uses; one that
    /// does not parse is refused and said out loud.
    #[test]
    fn a_condition_typed_into_a_breakpoint_is_read_or_refused_aloud() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::emulatable_sample().opened(WorkspaceView::Machine);
        let entry = app
            .analysis
            .as_ref()
            .and_then(|analysis| analysis.entry_point)
            .expect("an entry point");
        app.selected_instruction = Some(entry);
        app.run_command(&ctx, Command::MachineToggleBreakpoint);

        super::apply(
            &mut app,
            &ctx,
            super::BreakpointEdit {
                condition: Some((entry, String::from("rax == 1"))),
                ..super::BreakpointEdit::default()
            },
        );
        assert_eq!(
            app.machine()
                .and_then(|machine| machine.breakpoint(entry))
                .map(|breakpoint| breakpoint.condition.clone()),
            Some(String::from("rax == 1"))
        );

        let before = app.journal.entries().len();
        super::apply(
            &mut app,
            &ctx,
            super::BreakpointEdit {
                condition: Some((entry, String::from("rax == "))),
                ..super::BreakpointEdit::default()
            },
        );
        assert_eq!(
            app.machine()
                .and_then(|machine| machine.breakpoint(entry))
                .map(|breakpoint| breakpoint.condition.clone()),
            Some(String::from("rax == 1")),
            "what does not parse never replaces what does"
        );
        assert!(
            app.journal.entries().len() > before,
            "and the reader is told, rather than left with a field that did nothing"
        );
    }

    /// The breakpoint pane is one of several callers of the listing.  It
    /// must use the shared navigation route: otherwise it would select an
    /// instruction without scrolling the pseudo-code alongside it or giving
    /// the reader the transient marker that says where the jump landed.
    #[test]
    fn showing_a_breakpoint_uses_the_shared_disassembly_navigation() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::emulatable_sample().opened(WorkspaceView::Machine);
        let address = app
            .analysis
            .as_ref()
            .and_then(|analysis| analysis.entry_point)
            .expect("the fixture has an entry point");

        super::apply(
            &mut app,
            &ctx,
            super::BreakpointEdit {
                show: Some(address),
                ..super::BreakpointEdit::default()
            },
        );

        assert_eq!(app.active_view, WorkspaceView::Disassembly);
        assert_eq!(app.selected_instruction, Some(address));
        assert_eq!(app.pending_instruction_scroll, Some(address));
        assert!(
            app.instruction_attention
                .is_some_and(|(marked, _)| marked == address),
            "callers share the same visible landing marker"
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
