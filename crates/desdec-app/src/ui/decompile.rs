use crate::{
    app::DesdecApp,
    i18n::{Language, Text, text},
    ui::{ERROR, MUTED, card, syntax},
};
use desdec_core::Analysis;
use eframe::egui;
use std::time::Duration;
/// The pseudo-code view: the built-in translation, or the external engine
/// chosen in the preferences.
pub fn show(app: &mut DesdecApp, ui: &mut egui::Ui) {
    if app.preferences.decompiler.engine().is_some() {
        external(app, ui);
        return;
    }
    let language = app.preferences.language;
    let Some(analysis) = &app.analysis else {
        return;
    };
    card(ui, text(language, Text::PseudoCode), |ui| {
        ui.small(text(language, Text::PseudoCodeHelp));
        if analysis.instructions.is_empty() {
            ui.label(egui::RichText::new(text(language, Text::NoDisassembly)).color(MUTED));
            return;
        }
        let scroll_target = app.pending_instruction_scroll;
        let attention = active_attention(ui.ctx(), &mut app.instruction_attention);
        panel(
            ui,
            analysis,
            &mut app.selected_instruction,
            scroll_target,
            &mut app.pending_instruction_scroll,
            attention,
        );
        if app.pending_instruction_scroll == scroll_target {
            app.pending_instruction_scroll = None;
        }
    });
}

/// Output of an external decompiler, started on demand.
///
/// The engine is named next to its text: two decompilers disagree often
/// enough that reading one without knowing which would be misleading.
fn external(app: &mut DesdecApp, ui: &mut egui::Ui) {
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

    card(ui, text(language, Text::PseudoCode), |ui| {
        ui.horizontal(|ui| {
            ui.small(text(language, Text::DecompiledBy));
            ui.small(egui::RichText::new(&title).strong());
            ui.separator();
            function_picker(app, ui, &functions, language);
        });
        ui.add_space(8.0);

        if app.external.running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(text(language, Text::Decompiling));
            });
            return;
        }
        if let Some(error) = &app.external.error {
            ui.colored_label(
                ERROR,
                format!("{} {error}", text(language, Text::DecompilerFailed)),
            );
            return;
        }
        let Some(decompiled) = &app.external.text else {
            return;
        };
        egui::ScrollArea::both()
            .id_salt("external_decompilation")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for line in decompiled.lines() {
                    ui.label(syntax::pseudo_code(ui, line, egui::Color32::TRANSPARENT));
                }
            });
    });
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
) {
    ensure_selected_instruction(analysis, selected_instruction);
    egui::ScrollArea::both()
        .id_salt("pseudo_code")
        // Fill the space the panel was given instead of hugging the longest
        // line, so both listings of the disassembly view stay side by side.
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let transparent = egui::Color32::TRANSPARENT;
            ui.label(syntax::pseudo_code(
                ui,
                "void decompiled_entry(void) {",
                transparent,
            ));
            for instruction in &analysis.instructions {
                ui.horizontal(|ui| {
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
                        ui.ctx().request_repaint();
                    }
                    if scroll_target == Some(instruction.address) {
                        ui.scroll_to_rect(code.rect, Some(egui::Align::Center));
                    }
                });
            }
            ui.label(syntax::pseudo_code(ui, "}", transparent));
        });
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
