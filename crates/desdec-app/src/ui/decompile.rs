use crate::ui::{MUTED, card};
use desdec_core::Analysis;
use eframe::egui;
pub fn show(ui: &mut egui::Ui, analysis: &Analysis, expert: bool) {
    card(ui, "Pseudo-code local", |ui| {
        ui.small("Le flot observé est expliqué, sans prétendre reconstituer le code source.");
        if analysis.instructions.is_empty() {
            ui.label(egui::RichText::new("Aucune instruction x86/x86-64 disponible.").color(MUTED));
            return;
        }
        panel(ui, analysis, expert);
    });
}
pub fn panel(ui: &mut egui::Ui, analysis: &Analysis, expert: bool) {
    egui::ScrollArea::both()
        .id_salt("pseudo_code")
        .show(ui, |ui| {
            ui.monospace("void decompiled_entry(void) {");
            for instruction in &analysis.instructions {
                ui.horizontal(|ui| {
                    if expert {
                        ui.monospace(format!("{:#018x}", instruction.address));
                    }
                    ui.monospace(format!("    {}", pseudo_c(&instruction.text)));
                });
            }
            ui.monospace("}");
        });
}
/// Conservative AT&T-to-C presentation. The decoder remains authoritative;
/// this layer deliberately keeps unknown semantics as comments instead of
/// inventing types, variable names, or source-level control structures.
fn pseudo_c(asm: &str) -> String {
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
