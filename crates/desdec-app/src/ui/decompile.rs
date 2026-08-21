use crate::{
    app::{DesdecApp, Dialog, PseudocodeAssembly, WorkspaceView},
    commands::Command,
    i18n::{Language, Text, text},
    ui::{ERROR, MUTED, ROW_HEIGHT, card, syntax},
};
use desdec_core::Analysis;
use eframe::egui;
use std::time::Duration;
/// The pseudo-code view: the built-in translation, or the external engine
/// chosen in the preferences.
pub fn show(app: &mut DesdecApp, ui: &mut egui::Ui) {
    if app.preferences.decompiler.engine().is_some() {
        // An engine that is not installed, or that failed, leaves the view
        // with nothing in it. The built-in translation depends on nothing and
        // always answers, so it stands in rather than the reader being shown
        // an error where the pseudo-code should be.
        let answered = external(app, ui);
        if !answered {
            ui.add_space(12.0);
            builtin(app, ui);
        }
        show_assembly_preview(app, ui.ctx());
        return;
    }
    builtin(app, ui);
    show_assembly_preview(app, ui.ctx());
}

/// The translation this tool makes itself, from the flow it decoded.
fn builtin(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    let actions = PseudocodeActions::from_app(app);
    let Some(analysis) = &app.analysis else {
        return;
    };
    let mut chosen = None;
    card(ui, text(language, Text::PseudoCode), |ui| {
        if app.preferences.decompiler.engine().is_some() {
            ui.small(egui::RichText::new(text(language, Text::BuiltinFallbackNote)).color(MUTED));
            ui.add_space(4.0);
        }
        let help = ui.small(text(language, Text::PseudoCodeHelp));
        help.context_menu(|ui| {
            chosen = pseudocode_menu(ui, language, actions);
        });
        if analysis.instructions.is_empty() {
            ui.label(egui::RichText::new(text(language, Text::NoDisassembly)).color(MUTED));
            return;
        }
        let scroll_target = app.pending_instruction_scroll;
        let attention = active_attention(ui.ctx(), &mut app.instruction_attention);
        if let Some(address) = panel(
            ui,
            analysis,
            &mut app.selected_instruction,
            scroll_target,
            &mut app.pending_instruction_scroll,
            attention,
        ) {
            app.pseudocode_assembly = Some(PseudocodeAssembly::Instruction(address));
            app.dialogs.open(Dialog::Assembly);
        }
        if app.pending_instruction_scroll == scroll_target {
            app.pending_instruction_scroll = None;
        }
    });
    if let Some(command) = chosen {
        app.run_command(ui.ctx(), command);
    }
}

/// Output of an external decompiler, started on demand.
///
/// The engine is named next to its text: two decompilers disagree often
/// enough that reading one without knowing which would be misleading. The
/// card is named for the engine too — it used to be titled "local
/// pseudo-code" while saying "produced by `RetDec`" underneath, which is a
/// contradiction on the face of it.
///
/// Returns whether it actually put pseudo-code on screen.
fn external(app: &mut DesdecApp, ui: &mut egui::Ui) -> bool {
    let language = app.preferences.language;
    let engine = app.preferences.decompiler.engine();
    let title = engine.map_or_else(|| String::from("—"), |engine| engine.label().to_owned());

    // These engines decompile one function at a time. Without a choice the
    // view always showed `entry0`, the C runtime stub, which says nothing
    // about the program — so the function is picked here, defaulting to the
    // one a reader actually starts from.
    let functions = decompilable_functions(app);
    if app.selected_function.is_none() {
        app.selected_function = default_function(&functions);
    }
    app.request_decompilation(ui.ctx(), app.selected_function);

    let mut answered = false;
    let mut fall_back = false;
    let mut show_path = false;
    let mut chosen = None;
    let actions_for_menu = PseudocodeActions::from_app(app);
    card(ui, text(language, Text::ExternalPseudoCode), |ui| {
        ui.horizontal(|ui| {
            ui.small(text(language, Text::DecompiledBy));
            ui.small(egui::RichText::new(&title).strong());
            if app.external.from_cache {
                // Said plainly: this text was produced earlier, for these same
                // bytes, and is not the engine answering again now.
                ui.small(egui::RichText::new(text(language, Text::FromCache)).color(MUTED));
            }
            ui.separator();
            function_picker(app, ui, &functions, language);
            // These engines decompile a function as a whole and publish no
            // line-to-address map. One button naming the function is what can
            // honestly be offered; clickable lines implied a per-line mapping
            // that does not exist, and every one of them opened the same body.
            if let Some(address) = app.selected_function
                && ui
                    .button(text(language, Text::AssemblyPreview))
                    .on_hover_text(text(language, Text::WholeFunctionAssembly))
                    .clicked()
            {
                app.pseudocode_assembly = Some(PseudocodeAssembly::Function(address));
                app.dialogs.open(Dialog::Assembly);
            }
        });
        let actions =
            ui.small(egui::RichText::new(text(language, Text::PseudoCodeContextHelp)).color(MUTED));
        actions.context_menu(|ui| {
            chosen = pseudocode_menu(ui, language, actions_for_menu);
        });
        ui.add_space(8.0);

        if app.external.running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(text(language, Text::Decompiling));
            });
            // Nothing is missing while it is still working: an answer is on
            // its way, and standing in for it would put two texts on screen.
            answered = true;
            return;
        }
        if let Some(error) = &app.external.error {
            ui.colored_label(
                ERROR,
                format!("{} {error}", text(language, Text::DecompilerFailed)),
            );
            ui.add_space(6.0);
            // Both ways out, next to what went wrong: an engine that cannot be
            // found is a preference to change, not a state to be stuck in —
            // and it is as often installed somewhere the PATH does not reach
            // as it is not installed at all.
            ui.horizontal(|ui| {
                fall_back = ui
                    .button(text(language, Text::UseBuiltinDecompiler))
                    .clicked();
                show_path = ui.button(text(language, Text::ShowEnginePath)).clicked();
            });
            return;
        }
        let Some(decompiled) = &app.external.text else {
            return;
        };
        answered = true;
        // Only the visible rows are laid out: a large function decompiles to
        // thousands of lines, and building a widget for every one of them cost
        // more each frame than the decompilation itself.
        let lines: Vec<&str> = decompiled.lines().collect();
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::both()
            .id_salt("external_decompilation")
            .auto_shrink([false, false])
            .show_rows(ui, row_height, lines.len(), |ui, rows| {
                for line in &lines[rows] {
                    let response =
                        ui.label(syntax::pseudo_code(ui, line, egui::Color32::TRANSPARENT));
                    response.context_menu(|ui| {
                        chosen = pseudocode_menu(ui, language, actions_for_menu);
                    });
                }
            });
    });

    if fall_back {
        app.run_command(ui.ctx(), crate::commands::Command::DecompilerBuiltin);
        return true;
    }
    if show_path {
        // Straight to the field that answers it, rather than "it is in the
        // preferences somewhere".
        app.preferences_tab = crate::ui::preferences_window::PreferencesTab::Decompiler;
        app.dialogs.open(crate::app::Dialog::Preferences);
    }
    if let Some(command) = chosen {
        app.run_command(ui.ctx(), command);
    }
    answered
}

/// The context menu is a front-end to the command registry, never a private
/// second set of actions. That keeps every useful pseudo-code action available
/// from the palette and an assignable shortcut as well.
#[derive(Clone, Copy)]
struct PseudocodeActions {
    rerun: bool,
    assembly: bool,
    copy: bool,
}

impl PseudocodeActions {
    fn from_app(app: &DesdecApp) -> Self {
        Self {
            rerun: app.can_run(Command::RerunDecompilation),
            assembly: app.can_run(Command::ShowDecompilationAssembly),
            copy: app.can_run(Command::CopyPseudoCode),
        }
    }
}

fn pseudocode_menu(
    ui: &mut egui::Ui,
    language: Language,
    actions: PseudocodeActions,
) -> Option<Command> {
    let mut chosen = None;
    for (command, enabled) in [
        (Command::RerunDecompilation, actions.rerun),
        (Command::ShowDecompilationAssembly, actions.assembly),
        (Command::CopyPseudoCode, actions.copy),
    ] {
        if ui
            .add_enabled(enabled, egui::Button::new(command.label(language)))
            .clicked()
        {
            chosen = Some(command);
            ui.close_menu();
            return chosen;
        }
    }
    ui.separator();
    for command in [
        Command::DecompilerBuiltin,
        Command::DecompilerRzGhidra,
        Command::DecompilerRetDec,
    ] {
        if ui.button(command.label(language)).clicked() {
            chosen = Some(command);
            ui.close_menu();
            return chosen;
        }
    }
    ui.separator();
    if ui
        .button(Command::DecompilerPreferences.label(language))
        .clicked()
    {
        chosen = Some(Command::DecompilerPreferences);
        ui.close_menu();
    }
    chosen
}

/// Functions the engine can be pointed at: named, defined here, with an
/// address. Sorted by address so the list reads like the image itself.
fn decompilable_functions(app: &DesdecApp) -> Vec<(u64, String)> {
    let Some(analysis) = app.analysis.as_ref() else {
        return Vec::new();
    };
    let mut functions: Vec<(u64, String)> = analysis
        .symbols
        .iter()
        .filter(|symbol| !symbol.imported)
        .filter_map(|symbol| Some((symbol.address?, symbol.name.clone())))
        .collect();
    functions.sort_by_key(|(address, _)| *address);
    functions.dedup_by_key(|(address, _)| *address);
    functions
}

/// Where a reader starts: `main` when the binary has one, otherwise the entry
/// point, and failing both the first named function.
fn default_function(functions: &[(u64, String)]) -> Option<u64> {
    functions
        .iter()
        .find(|(_, name)| name == "main")
        .or_else(|| functions.first())
        .map(|(address, _)| *address)
}

fn function_picker(
    app: &mut DesdecApp,
    ui: &mut egui::Ui,
    functions: &[(u64, String)],
    language: Language,
) {
    if functions.is_empty() {
        // A stripped binary names nothing. Saying which address is being
        // decompiled anyway beats an empty picker and an unexplained listing.
        ui.small(egui::RichText::new(text(language, Text::StrippedEntryPoint)).color(MUTED));
        return;
    }
    let selected = app
        .selected_function
        .and_then(|address| {
            functions
                .iter()
                .find(|(candidate, _)| *candidate == address)
        })
        .map_or_else(|| "—".to_owned(), |(_, name)| name.clone());

    ui.small(text(language, Text::Function));
    egui::ComboBox::from_id_salt("external_function")
        .selected_text(selected)
        .width(260.0)
        .show_ui(ui, |ui| {
            for (address, name) in functions {
                let label = format!("{name}  {address:#x}");
                if ui
                    .selectable_label(app.selected_function == Some(*address), label)
                    .clicked()
                {
                    app.selected_function = Some(*address);
                }
            }
        });
}

/// Returns the address to flash until its deadline, clearing it afterwards.
/// A short repaint cadence makes the mark blink without disturbing scrolling.
pub fn active_attention(
    ctx: &egui::Context,
    instruction_attention: &mut Option<(u64, f64)>,
) -> Option<u64> {
    let now = ctx.input(|input| input.time);
    match *instruction_attention {
        Some((address, until)) if now < until => {
            ctx.request_repaint_after(Duration::from_millis(180));
            Some(address)
        }
        Some(_) => {
            *instruction_attention = None;
            None
        }
        None => None,
    }
}

pub fn instruction_fill(
    ui: &egui::Ui,
    address: u64,
    selected_instruction: Option<u64>,
    attention: Option<u64>,
) -> egui::Color32 {
    let flashes_now = attention == Some(address)
        && (ui.ctx().input(|input| input.time * 5.0).floor() as u64) % 2 == 0;
    if flashes_now {
        egui::Color32::from_rgb(241, 169, 75)
    } else if selected_instruction == Some(address) {
        ui.style().visuals.selection.bg_fill
    } else {
        egui::Color32::TRANSPARENT
    }
}

pub fn ensure_selected_instruction(analysis: &Analysis, selected_instruction: &mut Option<u64>) {
    if !selected_instruction.is_some_and(|address| {
        analysis
            .instructions
            .iter()
            .any(|instruction| instruction.address == address)
    }) {
        *selected_instruction = analysis
            .instructions
            .first()
            .map(|instruction| instruction.address);
    }
}

pub fn panel(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    selected_instruction: &mut Option<u64>,
    scroll_target: Option<u64>,
    pending_scroll: &mut Option<u64>,
    attention: Option<u64>,
) -> Option<u64> {
    let mut clicked_instruction = None;
    ensure_selected_instruction(analysis, selected_instruction);
    // Virtualised like the disassembly beside it, and for the same reason: one
    // widget per decoded instruction is seconds of layout per frame.
    let area = listing_area(
        egui::ScrollArea::both().id_salt("pseudo_code"),
        ui,
        analysis,
        scroll_target,
        1,
    );
    let total_rows = analysis.instructions.len() + 2;
    area
        // Fill the space the panel was given instead of hugging the longest
        // line, so both listings of the disassembly view stay side by side.
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, total_rows, |ui, rows| {
            let transparent = egui::Color32::TRANSPARENT;
            if rows.start == 0 {
                ui.label(syntax::pseudo_code(
                    ui,
                    "void decompiled_entry(void) {",
                    transparent,
                ));
            }
            for instruction in rows_of(&analysis.instructions, &rows, 1) {
                ui.horizontal(|ui| {
                    ui.set_min_height(ROW_HEIGHT);
                    let selected_fill =
                        instruction_fill(ui, instruction.address, *selected_instruction, attention);
                    let address = ui
                        .add(
                            egui::Label::new(syntax::dim(
                                ui,
                                &format!("{:#018x}", instruction.address),
                                selected_fill,
                            ))
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    let code = ui
                        .add(
                            egui::Label::new(syntax::pseudo_code(
                                ui,
                                &format!("    {}", pseudo_c(&instruction.text)),
                                selected_fill,
                            ))
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if address.clicked() || code.clicked() {
                        *selected_instruction = Some(instruction.address);
                        *pending_scroll = Some(instruction.address);
                        clicked_instruction = Some(instruction.address);
                        ui.ctx().request_repaint();
                    }
                });
            }
            if rows.end == total_rows {
                ui.label(syntax::pseudo_code(ui, "}", transparent));
            }
        });
    clicked_instruction
}

/// Prepares a virtualised listing of the decoded instructions.
///
/// `leading` counts the rows drawn before the first instruction — a header, a
/// signature line — so an instruction's row number matches its position in the
/// listing. When an instruction is to be brought into view, the offset is set
/// here: the row itself may well not be laid out this frame, so it cannot be
/// the one to ask for the scroll.
pub fn listing_area(
    area: egui::ScrollArea,
    ui: &egui::Ui,
    analysis: &Analysis,
    scroll_target: Option<u64>,
    leading: usize,
) -> egui::ScrollArea {
    let row = scroll_target
        .and_then(|address| analysis.instruction_index(address))
        .map(|index| index + leading);
    listing_area_at_row(area, ui, row)
}

/// The same, for a listing whose rows are not one per instruction.
///
/// The disassembly draws a heading where each section begins, so the row an
/// instruction sits on is its index plus the headings above it; only the
/// caller knows how many those are.
pub fn listing_area_at_row(
    area: egui::ScrollArea,
    ui: &egui::Ui,
    row: Option<usize>,
) -> egui::ScrollArea {
    let Some(row) = row else {
        return area;
    };
    // The virtualiser spaces its rows by the row height *plus* the item
    // spacing, and reads back the offset the same way: computing it from the
    // row height alone lands short by that spacing on every row, which over a
    // hundred thousand of them is a seventh of the listing.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a listing long enough to lose f32 precision is far beyond what a file can hold"
    )]
    let offset = row as f32 * (ROW_HEIGHT + ui.spacing().item_spacing.y);
    // Centred rather than pinned to the top, so the instructions around the
    // one being shown are part of the answer.
    let offset = (offset - ui.available_height() / 2.0).max(0.0);
    area.vertical_scroll_offset(offset)
}

/// The instructions covered by a virtualiser's row range, given `leading` rows
/// drawn before the first of them.
pub fn rows_of<'a>(
    instructions: &'a [desdec_core::Instruction],
    rows: &std::ops::Range<usize>,
    leading: usize,
) -> &'a [desdec_core::Instruction] {
    let start = rows.start.saturating_sub(leading).min(instructions.len());
    let end = rows.end.saturating_sub(leading).min(instructions.len());
    &instructions[start..end.max(start)]
}

/// A click opens a small, persistent assembly window rather than claiming that
/// every external C line has an exact machine address. Local pseudo-code maps
/// to one instruction; Rizin and `RetDec` map to their selected function.
///
/// A window and not a bubble following the pointer: the reader has to reach the
/// button inside it, which a surface anchored to the mouse never lets them do.
fn show_assembly_preview(app: &mut DesdecApp, ctx: &egui::Context) {
    /// Size assumed the first time the window opens, before egui has measured
    /// it.
    const ASSEMBLY_SIZE: egui::Vec2 = egui::vec2(620.0, 400.0);

    if !app.dialogs.is_open(Dialog::Assembly) {
        return;
    }
    let Some(preview) = app.pseudocode_assembly else {
        app.dialogs.close(Dialog::Assembly);
        return;
    };
    let Some(analysis) = app.analysis.as_ref() else {
        app.dialogs.close(Dialog::Assembly);
        return;
    };
    let (rows, hidden) = preview_instructions(analysis, &app.functions, preview);
    let mut jump = None;
    let language = app.preferences.language;
    // The same width the listing holds its address column to, so a line read
    // here and the same line read there are read at the same place.
    let address_width = app.listing_columns.address;

    // A stable id keeps the window where the reader put it as the inspected
    // line changes.
    let id = egui::Id::new("desdec.pseudocode_assembly");
    let mut open = true;
    let mut window = egui::Window::new(text(language, Text::AssemblyPreview))
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(ASSEMBLY_SIZE.x);
    if let Some(step) = app.dialogs.opening_step(Dialog::Assembly) {
        window = window.current_pos(crate::ui::opening_position(ctx, id, step, ASSEMBLY_SIZE));
    }
    window.show(ctx, |ui| {
        if rows.is_empty() {
            ui.label(egui::RichText::new(text(language, Text::NoFunctionBody)).color(MUTED));
            return;
        }
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                for instruction in rows {
                    ui.horizontal(|ui| {
                        let width = crate::ui::disassembly::monospace_width(ui, address_width);
                        crate::ui::disassembly::sized_cell(ui, width, |ui| {
                            ui.label(syntax::dim(
                                ui,
                                &format!("{:#018x}", instruction.address),
                                egui::Color32::TRANSPARENT,
                            ))
                        });
                        ui.label(syntax::assembly(
                            ui,
                            &instruction.text,
                            egui::Color32::TRANSPARENT,
                        ));
                    });
                }
                // Silently cutting the body would read as a short function.
                if hidden > 0 {
                    ui.small(
                        egui::RichText::new(format!(
                            "… {hidden} {}",
                            text(language, Text::MoreInstructions)
                        ))
                        .color(MUTED),
                    );
                }
            });
        ui.add_space(8.0);
        if ui.button(text(language, Text::JumpToAssembly)).clicked() {
            jump = rows.first().map(|instruction| instruction.address);
        }
    });

    app.dialogs.set(Dialog::Assembly, open);
    if !open {
        app.pseudocode_assembly = None;
    }
    if let Some(address) = jump {
        app.selected_instruction = Some(address);
        app.pending_instruction_scroll = Some(address);
        app.instruction_attention = Some((address, ctx.input(|input| input.time) + 3.0));
        app.active_view = WorkspaceView::Disassembly;
        app.dialogs.close(Dialog::Assembly);
        app.pseudocode_assembly = None;
        ctx.request_repaint();
    }
}

/// The instructions to show, and how many the limit left out.
///
/// Both cases read a slice of the listing — bisected by address, or taken from
/// the function index built when the binary was opened — so an open window
/// costs nothing per frame.
fn preview_instructions<'a>(
    analysis: &'a Analysis,
    functions: &[super::functions::Function],
    preview: PseudocodeAssembly,
) -> (&'a [desdec_core::Instruction], usize) {
    const LIMIT: usize = 48;
    match preview {
        PseudocodeAssembly::Instruction(address) => match analysis.instruction_index(address) {
            Some(index) => (&analysis.instructions[index..=index], 0),
            None => (&[], 0),
        },
        PseudocodeAssembly::Function(start) => {
            let Some(function) = functions.iter().find(|function| function.start == start) else {
                return (&[], 0);
            };
            let body = analysis
                .instructions
                .get(function.instructions.clone())
                .unwrap_or_default();
            let shown = body.len().min(LIMIT);
            (&body[..shown], body.len() - shown)
        }
    }
}

/// Conservative AT&T-to-C presentation. The decoder remains authoritative;
/// this layer deliberately keeps unknown semantics as comments instead of
/// inventing types, variable names, or source-level control structures.
pub(crate) fn pseudo_c(asm: &str) -> String {
    let mut fields = asm.splitn(2, char::is_whitespace);
    let opcode = fields.next().unwrap_or_default();
    let operands = fields.next().unwrap_or_default().trim();
    let pair = || {
        operands
            .split_once(',')
            .map(|(left, right)| (left.trim(), right.trim()))
    };
    if opcode.starts_with("ret") {
        "return;".into()
    } else if opcode == "call" || opcode == "callq" {
        format!("{}();", c_target(operands))
    } else if opcode == "jmp" || opcode == "jmpq" {
        format!("goto {} ;", label(operands))
    } else if opcode.starts_with('j') {
        format!(
            "if (/* {} condition from flags */) goto {};",
            opcode,
            label(operands)
        )
    } else if opcode.starts_with("mov") {
        pair().map_or_else(
            || unknown(asm),
            |(source, destination)| format!("{} = {};", c_value(destination), c_value(source)),
        )
    } else if opcode.starts_with("lea") {
        pair().map_or_else(
            || unknown(asm),
            |(source, destination)| format!("{} = &({});", c_value(destination), c_value(source)),
        )
    } else if let Some(operator) = match opcode {
        "add" | "addq" | "addl" => Some("+="),
        "sub" | "subq" | "subl" => Some("-="),
        "and" | "andq" | "andl" => Some("&="),
        "or" | "orq" | "orl" => Some("|="),
        "xor" | "xorq" | "xorl" => Some("^="),
        // The one-operand multiply form writes the implicit RDX:RAX pair and
        // is intentionally left unsupported. This spelling has an explicit
        // source and destination, so the C-like assignment is honest.
        "imul" | "imulq" | "imull" => Some("*="),
        "shl" | "sal" | "shlq" | "shll" => Some("<<="),
        "shr" | "sar" | "shrq" | "sarl" => Some(">>="),
        _ => None,
    } {
        pair().map_or_else(
            || unknown(asm),
            |(source, destination)| {
                format!("{} {} {};", c_value(destination), operator, c_value(source))
            },
        )
    } else if opcode.starts_with("cmp") || opcode.starts_with("test") {
        format!(
            "/* {}: condition flags set for the next branch */",
            operands
        )
    } else if opcode.starts_with("push") {
        format!("stack_push({});", c_value(operands))
    } else if opcode.starts_with("pop") {
        format!("{} = stack_pop();", c_value(operands))
    } else if opcode.starts_with("inc") {
        format!("{}++;", c_value(operands))
    } else if opcode.starts_with("dec") {
        format!("{}--;", c_value(operands))
    } else if opcode.starts_with("neg") {
        format!("{} = -{};", c_value(operands), c_value(operands))
    } else if opcode.starts_with("not") {
        format!("{} = ~{};", c_value(operands), c_value(operands))
    } else if opcode == "nop" || opcode == "nopw" || opcode == "nopl" {
        "/* no operation */".into()
    } else if opcode == "syscall" || opcode == "sysenter" || opcode == "int" {
        "/* system request: inspect the Machine trace */".into()
    } else {
        unknown(asm)
    }
}

fn c_value(value: &str) -> String {
    value.trim_start_matches('%').replace('$', "")
}
fn c_target(value: &str) -> String {
    c_value(value).trim_start_matches('*').to_owned()
}
fn label(value: &str) -> String {
    format!(
        "label_{}",
        c_target(value).replace(|c: char| !c.is_ascii_alphanumeric(), "_")
    )
}
fn unknown(asm: &str) -> String {
    format!("/* unsupported: {asm} */")
}

#[cfg(test)]
mod tests {
    use crate::{
        app::WorkspaceView,
        i18n::{Language, Text, text},
        preferences::DecompilerPreference,
        testing::{drawn_text, window_input},
        ui::views,
    };
    use eframe::egui;

    /// A card titled "local pseudocode" that says "produced by `RetDec`"
    /// underneath contradicts itself on the face of it.
    #[test]
    fn the_external_card_is_not_named_after_the_built_in_one() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Decompile);
        app.preferences.language = Language::French;
        app.preferences.decompiler = DecompilerPreference::RetDec;
        app.external.error = Some("not installed".to_owned());

        let output = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });

        let drawn = drawn_text(&output.shapes);
        assert!(
            drawn.contains(text(Language::French, Text::ExternalPseudoCode)),
            "the external output must be named for what produced it"
        );
    }

    /// An engine that is not installed used to leave the view with nothing but
    /// an error where the pseudo-code should be. The built-in translation
    /// depends on nothing, so it stands in.
    #[test]
    fn a_failed_engine_falls_back_to_the_translation_that_always_answers() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Decompile);
        app.preferences.language = Language::French;
        app.preferences.decompiler = DecompilerPreference::RetDec;
        app.external.error = Some("Ce décompilateur n’est pas installé.".to_owned());

        let output = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });

        let drawn = drawn_text(&output.shapes);
        assert!(
            drawn.contains(text(Language::French, Text::BuiltinFallbackNote)),
            "the reader must be told why the built-in translation is there"
        );
        assert!(
            drawn.contains(text(Language::French, Text::UseBuiltinDecompiler)),
            "and offered the way out of a decompiler that cannot be found"
        );
        assert!(
            drawn.contains(text(Language::French, Text::ShowEnginePath)),
            "an engine installed off the PATH needs its path given, not a \
             suggestion to install what is already there"
        );
    }

    use super::*;
    use crate::testing::reference_analysis;

    #[test]
    fn the_local_dictionary_keeps_only_explicit_instruction_meaning() {
        assert_eq!(pseudo_c("imulq $8, %rax"), "rax *= 8;");
        assert_eq!(pseudo_c("not %eax"), "eax = ~eax;");
        assert_eq!(pseudo_c("nop"), "/* no operation */");
        assert_eq!(
            pseudo_c("syscall"),
            "/* system request: inspect the Machine trace */"
        );
        // A one-operand multiply has implicit registers, so claiming a C
        // expression for it would invent information the decoded text lacks.
        assert!(pseudo_c("imul %rcx").starts_with("/* unsupported:"));
    }

    /// First decoded instruction of the reference binary.
    fn first_address() -> u64 {
        reference_analysis()
            .instructions
            .first()
            .map(|instruction| instruction.address)
            .expect("the test binary has decoded instructions")
    }

    #[test]
    fn an_instruction_preview_contains_only_the_clicked_instruction() {
        let analysis = reference_analysis();
        let address = first_address();
        let functions = super::super::functions::all(analysis);

        let (rows, hidden) = preview_instructions(
            analysis,
            &functions,
            PseudocodeAssembly::Instruction(address),
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].address, address);
        assert_eq!(hidden, 0);
    }

    /// A function preview shows that function's body and nothing beyond it.
    #[test]
    fn a_function_preview_stops_at_the_end_of_the_function() {
        let analysis = reference_analysis();
        let functions = super::super::functions::all(analysis);
        let Some(function) = functions
            .iter()
            .find(|function| !function.instructions.is_empty())
        else {
            return; // A stripped host binary names no function to preview.
        };

        let (rows, hidden) = preview_instructions(
            analysis,
            &functions,
            PseudocodeAssembly::Function(function.start),
        );

        assert!(!rows.is_empty());
        assert_eq!(rows.len() + hidden, function.instructions.len());
        assert!(
            rows.iter()
                .all(|instruction| (function.start..function.end).contains(&instruction.address)),
            "the preview must not run past the function it stands for"
        );
    }

    /// The window has to stay put: a surface pinned to the pointer moves out
    /// from under the reader before they can press the button inside it.
    #[test]
    fn the_assembly_window_does_not_follow_the_pointer() {
        let address = first_address();

        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Decompile);
        app.pseudocode_assembly = Some(PseudocodeAssembly::Instruction(address));
        app.dialogs.open(Dialog::Assembly);

        let mut rect = |ctx: &egui::Context, pointer: egui::Pos2| {
            let input = egui::RawInput {
                events: vec![egui::Event::PointerMoved(pointer)],
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| show_assembly_preview(&mut app, ctx));
            ctx.memory(|memory| memory.area_rect(egui::Id::new("desdec.pseudocode_assembly")))
                .expect("the window is laid out")
        };

        let first = rect(&ctx, egui::pos2(100.0, 100.0));
        let second = rect(&ctx, egui::pos2(600.0, 400.0));
        assert_eq!(first.min, second.min);
    }
}
