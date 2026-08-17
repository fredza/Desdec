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
const ASSUMED_SIZE: egui::Vec2 = egui::vec2(520.0, 340.0);

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
        window = window.current_pos(crate::ui::opening_position(ctx, id, step, ASSUMED_SIZE));
    }
    window.show(ctx, |ui| contents(app, ui, address));

    app.dialogs.set(Dialog::Operand, open);
    if !open {
        app.inspecting_operand = None;
    }
}

fn contents(app: &DesdecApp, ui: &mut egui::Ui, address: u64) {
    let language = app.preferences.language;
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
    ui.add_space(10.0);

    match operand::resolve(analysis, instruction, &app.file_bytes) {
        Some(target) => target_rows(ui, app, &target),
        None => {
            ui.label(egui::RichText::new(app.t(Text::NoTargetResolved)).color(MUTED));
        }
    }

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);
    register_rows(ui, app, analysis, instruction, architecture);

    ui.add_space(12.0);
    ui.small(egui::RichText::new(app.t(Text::StaticOnlyWarning)).color(MUTED));
    let _ = language;
}

/// Where the operand points, and what is there.
fn target_rows(ui: &mut egui::Ui, app: &DesdecApp, target: &operand::Target) {
    egui::Grid::new("operand_target")
        .num_columns(2)
        .spacing([20.0, 6.0])
        .show(ui, |ui| {
            ui.strong(app.t(Text::Designates));
            ui.label(egui::RichText::new(format!("{:#018x}", target.address)).monospace());
            ui.end_row();

            if let Some(section) = &target.section {
                ui.strong(app.t(Text::TargetSection));
                ui.label(egui::RichText::new(section).monospace());
                ui.end_row();
            }
            if let Some(symbol) = &target.symbol {
                ui.strong(app.t(Text::TargetSymbol));
                ui.label(egui::RichText::new(symbol).monospace());
                ui.end_row();
            }
            // Text is what a reader is usually after, so it comes before the
            // raw bytes it was read from.
            if let Some(text) = &target.text {
                ui.strong(app.t(Text::TargetText));
                ui.label(egui::RichText::new(format!("{text:?}")).monospace());
                ui.end_row();
            }
            if !target.bytes.is_empty() {
                ui.strong(app.t(Text::TargetBytes));
                ui.label(syntax::dim(
                    ui,
                    &hex(&target.bytes),
                    egui::Color32::TRANSPARENT,
                ));
                ui.end_row();
            }
        });
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
                    ui.small(app.t(Text::WrittenValue));
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
