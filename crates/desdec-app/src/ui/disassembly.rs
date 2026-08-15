//! Detailed x86/x86-64 decoding, synchronised with the local pseudo-code.
use crate::{ui::MUTED, ui::decompile, ui::syntax};
use desdec_core::Analysis;
use eframe::egui;

pub fn show(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    selected_instruction: &mut Option<u64>,
    pending_scroll: &mut Option<u64>,
    instruction_attention: &mut Option<(u64, f64)>,
) {
    if analysis.instructions.is_empty() {
        ui.label(egui::RichText::new("Le désassemblage est disponible pour les binaires x86, x86-64 et ARM64 ; ce fichier utilise une autre architecture ou ne contient aucune section exécutable lisible.").color(MUTED));
        return;
    }
    ui.horizontal(|ui| {
        ui.add_enabled(false, egui::Button::new("Décodage approfondi"))
            .on_hover_text("Les décodeurs locaux iced-x86 et Capstone sont actifs. Les moteurs alternatifs, comme rz-ghidra, seront proposés ici.");
        ui.small("Décodeurs locaux : iced-x86 (x86/x86-64) et Capstone (ARM64, dont Apple Silicon).");
    });
    ui.add_space(8.0);
    let scroll_target = *pending_scroll;
    let attention = decompile::active_attention(ui.ctx(), instruction_attention);
    ui.columns(2, |columns| {
        columns[1].strong("Pseudo-code local");
        columns[1].small("Traduction déterministe du flot, sans code source inventé.");
        decompile::panel(
            &mut columns[1],
            analysis,
            selected_instruction,
            scroll_target,
            pending_scroll,
            attention,
        );
        instructions(
            &mut columns[0],
            analysis,
            selected_instruction,
            scroll_target,
            pending_scroll,
            attention,
        );
    });
    if *pending_scroll == scroll_target {
        *pending_scroll = None;
    }
}

fn instructions(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    selected_instruction: &mut Option<u64>,
    scroll_target: Option<u64>,
    pending_scroll: &mut Option<u64>,
    attention: Option<u64>,
) {
    decompile::ensure_selected_instruction(analysis, selected_instruction);
    egui::ScrollArea::both()
        .id_salt("instructions")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("disassembly")
                .num_columns(4)
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Adresse");
                    ui.strong("Octets");
                    ui.strong("Section");
                    ui.strong("Instruction");
                    ui.end_row();
                    for instruction in &analysis.instructions {
                        let selected_fill = decompile::instruction_fill(
                            ui,
                            instruction.address,
                            *selected_instruction,
                            attention,
                        );
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
                        let bytes = instruction
                            .bytes
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<Vec<_>>()
                            .join(" ");
                        ui.label(syntax::dim(ui, &bytes, egui::Color32::TRANSPARENT));
                        ui.label(syntax::dim(
                            ui,
                            &instruction.section,
                            egui::Color32::TRANSPARENT,
                        ));
                        let assembly = ui
                            .add(
                                egui::Label::new(syntax::assembly(
                                    ui,
                                    &instruction.text,
                                    selected_fill,
                                ))
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        if address.clicked() || assembly.clicked() {
                            *selected_instruction = Some(instruction.address);
                            *pending_scroll = Some(instruction.address);
                            ui.ctx().request_repaint();
                        }
                        if scroll_target == Some(instruction.address) {
                            ui.scroll_to_rect(assembly.rect, Some(egui::Align::Center));
                        }
                        ui.end_row();
                    }
                });
        });
}
