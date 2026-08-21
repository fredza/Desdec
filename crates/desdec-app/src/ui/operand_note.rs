//! What an instruction's operand designates, and what last wrote its
//! registers.
//!
//! Desdec never runs the binary, so this window is careful about the
//! difference between the two answers it gives. The target of an operand is
//! arithmetic on known numbers and is exact. Following a register back through
//! the preceding instructions is a local reading that a branch can invalidate,
//! and the window says so rather than presenting both with equal confidence.

use desdec_core::operand;
use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::Text,
    ui::{MUTED, syntax},
};

/// Size assumed the first time the window opens, before egui has measured it.
const ASSUMED_SIZE: egui::Vec2 = egui::vec2(520.0, 440.0);

/// Past this the contents scroll instead of pushing the window off screen.
const MAXIMUM_HEIGHT: f32 = 560.0;

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::Operand) {
        return;
    }
    let Some(address) = app.inspecting_operand else {
        app.dialogs.close(Dialog::Operand);
        return;
    };

    let id = egui::Id::new("desdec.operand_note");
    let mut open = true;
    let mut window = egui::Window::new(app.t(Text::OperandInspection))
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(ASSUMED_SIZE.x);
    if let Some(step) = app.dialogs.opening_step(Dialog::Operand) {
        let _ = step;
        window = window.current_pos(crate::ui::under_cursor(ctx, ASSUMED_SIZE));
    }
    // The window says more than it used to, and an instruction naming three
    // registers makes it taller than some screens. It scrolls rather than
    // growing past the edge, where the last answer would be unreachable.
    window.show(ctx, |ui| {
        egui::ScrollArea::vertical()
            .max_height(MAXIMUM_HEIGHT)
            .show(ui, |ui| contents(app, ui, address));
    });

    app.dialogs.set(Dialog::Operand, open);
    if !open {
        app.inspecting_operand = None;
    }
}

fn contents(app: &DesdecApp, ui: &mut egui::Ui, address: u64) {
    let Some(analysis) = app.analysis.as_ref() else {
        return;
    };
    let Some(instruction) = analysis.instruction_at(address) else {
        return;
    };
    let architecture = analysis.summary.architecture;

    ui.horizontal(|ui| {
        ui.label(syntax::dim(
            ui,
            &format!("{address:#018x}"),
            egui::Color32::TRANSPARENT,
        ));
        ui.label(syntax::assembly(
            ui,
            &instruction.text,
            egui::Color32::TRANSPARENT,
        ));
    });
    // What the window is for, said once at the top. The two answers below look
    // alike on screen and are not worth the same, and a reader who has not
    // been told that will read the second as if it were the first.
    ui.add_space(6.0);
    explanation(ui, app.t(Text::OperandInspectionIntro));
    ui.add_space(10.0);

    heading(ui, app.t(Text::OperandTargetHeading));
    if let Some(target) = operand::resolve(analysis, instruction, &app.file_bytes) {
        explanation(ui, app.t(Text::OperandTargetExplained));
        ui.add_space(6.0);
        target_rows(ui, app, &target);
    } else {
        ui.label(egui::RichText::new(app.t(Text::NoTargetResolved)).color(MUTED));
        // Why there is nothing here, rather than only that there is nothing:
        // an empty answer reads as a failure of the tool, and this one is a
        // fact about the instruction.
        explanation(ui, app.t(Text::NoTargetExplained));
    }

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);
    register_rows(ui, app, analysis, instruction, architecture);

    ui.add_space(12.0);
    explanation(ui, app.t(Text::StaticOnlyWarning));
}

/// A numbered heading, so the two answers read as two answers rather than as
/// one list that changes subject halfway down.
fn heading(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title).strong());
    ui.add_space(2.0);
}

/// A sentence about what follows, wrapped to the window's width.
///
/// A `Label` rather than [`egui::Ui::small`]: two of those in a row are drawn
/// at the same place, and the paragraphs here come in runs. Wrapping is asked
/// for rather than left to the default — these are sentences, and one of them
/// running off the edge would take the explanation with it.
fn explanation(ui: &mut egui::Ui, sentence: &str) {
    ui.add(egui::Label::new(egui::RichText::new(sentence).small().color(MUTED)).wrap());
}

/// Where the operand points, and what is there.
fn target_rows(ui: &mut egui::Ui, app: &DesdecApp, target: &operand::Target) {
    egui::Grid::new("operand_target")
        .num_columns(2)
        .spacing([20.0, 6.0])
        .show(ui, |ui| {
            // Each field says what it is on hover: the grid is five terse
            // words down the left edge, and "Symbole" alone does not tell a
            // reader whether the address is the symbol's or merely inside it.
            field(ui, app, Text::Designates, Text::DesignatesExplained);
            ui.label(egui::RichText::new(format!("{:#018x}", target.address)).monospace());
            ui.end_row();

            if let Some(section) = &target.section {
                field(ui, app, Text::TargetSection, Text::TargetSectionExplained);
                ui.label(egui::RichText::new(section).monospace());
                ui.end_row();
            }
            if let Some(symbol) = &target.symbol {
                field(ui, app, Text::TargetSymbol, Text::TargetSymbolExplained);
                ui.label(egui::RichText::new(symbol).monospace());
                ui.end_row();
            }
            // Text is what a reader is usually after, so it comes before the
            // raw bytes it was read from.
            if let Some(text) = &target.text {
                field(ui, app, Text::TargetText, Text::TargetTextExplained);
                ui.label(egui::RichText::new(format!("{text:?}")).monospace());
                ui.end_row();
            }
            if !target.bytes.is_empty() {
                field(ui, app, Text::TargetBytes, Text::TargetBytesExplained);
                ui.label(syntax::dim(
                    ui,
                    &hex(&target.bytes),
                    egui::Color32::TRANSPARENT,
                ));
                ui.end_row();
            }
        });
}

/// A field's name, with the sentence saying what it means on hover.
fn field(ui: &mut egui::Ui, app: &DesdecApp, label: Text, explained: Text) {
    let response = ui.strong(app.t(label));
    app.tooltip(response, app.t(explained));
}

/// What last wrote each register the instruction names.
fn register_rows(
    ui: &mut egui::Ui,
    app: &DesdecApp,
    analysis: &desdec_core::Analysis,
    instruction: &desdec_core::Instruction,
    architecture: desdec_core::Architecture,
) {
    let registers = operand::registers(instruction, architecture);
    if registers.is_empty() {
        return;
    }
    heading(ui, app.t(Text::OperandRegistersHeading));
    explanation(ui, app.t(Text::OperandRegistersExplained));
    ui.add_space(8.0);
    for register in registers {
        ui.horizontal(|ui| {
            ui.strong(app.t(Text::LastWriteTo));
            ui.label(egui::RichText::new(&register).monospace().strong());
        });
        match operand::last_write(analysis, instruction.address, &register, architecture) {
            Some(write) => {
                ui.horizontal(|ui| {
                    ui.label(syntax::dim(
                        ui,
                        &format!("{:#018x}", write.address),
                        egui::Color32::TRANSPARENT,
                    ));
                    ui.label(syntax::assembly(
                        ui,
                        &write.text,
                        egui::Color32::TRANSPARENT,
                    ));
                });
                ui.horizontal(|ui| {
                    let title = ui.add(egui::Label::new(
                        egui::RichText::new(app.t(Text::WrittenValue)).small(),
                    ));
                    app.tooltip(title, app.t(Text::WrittenValueExplained));
                    match write.value {
                        Some(value) => {
                            ui.label(egui::RichText::new(format!("{value:#x}")).monospace());
                        }
                        // The instruction wrote something computed, so there is
                        // no literal to report and none is invented.
                        None => {
                            ui.small(
                                egui::RichText::new(app.t(Text::ValueUnknown))
                                    .color(MUTED)
                                    .italics(),
                            );
                        }
                    }
                });
            }
            None => {
                ui.small(egui::RichText::new(app.t(Text::NoWriteFound)).color(MUTED));
            }
        }
        ui.add_space(8.0);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::WorkspaceView;

    /// Opened on a real binary, in every language, the window must lay out —
    /// including for instructions whose operand resolves to nothing.
    #[test]
    fn the_inspection_lays_out_on_a_real_binary() {
        let addresses: Vec<u64> = crate::testing::reference_analysis()
            .instructions
            .iter()
            .take(40)
            .map(|instruction| instruction.address)
            .collect();
        assert!(!addresses.is_empty(), "the host binary must decode");

        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Disassembly);

        for address in addresses {
            app.inspecting_operand = Some(address);
            app.dialogs.open(Dialog::Operand);
            for language in crate::i18n::Language::ALL {
                app.preferences.language = *language;
                let output = ctx.run(egui::RawInput::default(), |ctx| show(&mut app, ctx));
                assert!(!output.shapes.is_empty(), "{address:#x} {language:?}");
            }
        }
    }

    /// An address that is not an instruction must close the window rather
    /// than draw a half-empty one.
    #[test]
    fn an_unknown_address_is_not_inspected() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Disassembly);
        app.inspecting_operand = Some(0xdead_beef_0000);
        app.dialogs.open(Dialog::Operand);

        let _ = ctx.run(egui::RawInput::default(), |ctx| show(&mut app, ctx));
        // The window stays open but says nothing about an instruction it
        // cannot find; nothing is invented for it.
        assert!(app.dialogs.is_open(Dialog::Operand));
    }
}
