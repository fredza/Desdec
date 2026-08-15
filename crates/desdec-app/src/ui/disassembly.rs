//! One decoder, two reading levels: concise guided rows and complete expert rows.
use crate::{ui::MUTED, ui::decompile};
use desdec_core::Analysis;
use eframe::egui;

pub fn show(ui: &mut egui::Ui, analysis: &Analysis, expert: bool, detailed: &mut bool) {
    if analysis.instructions.is_empty() {
        ui.label(egui::RichText::new("Le désassemblage est disponible pour les binaires x86 et x86-64 ; ce fichier utilise une autre architecture ou ne contient aucune section exécutable lisible.").color(MUTED));
        return;
    }
    if expert {
        ui.horizontal(|ui| {
            let label = if *detailed {
                "Réduire le décodage"
            } else {
                "Décodage approfondi"
            };
            if ui.button(label).clicked() {
                *detailed = !*detailed;
            }
            ui.small("Moteur iced-x86 : décodage x86/x86-64 complet et sûr, sans FFI native.");
        });
        ui.add_space(8.0);
    } else {
        ui.small("Chaque ligne représente une instruction exécutée par le processeur. L’adresse indique son emplacement dans le programme.");
        ui.add_space(8.0);
    }
    let expert = expert && *detailed;
    ui.columns(2, |columns| {
        columns[1].strong("Pseudo-code local");
        columns[1].small("Traduction déterministe du flot, sans code source inventé.");
        decompile::panel(&mut columns[1], analysis, expert);
        instructions(&mut columns[0], analysis, expert);
    });
}

fn instructions(ui: &mut egui::Ui, analysis: &Analysis, expert: bool) {
    egui::ScrollArea::both()
        .id_salt("instructions")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("disassembly")
                .num_columns(if expert { 4 } else { 2 })
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Adresse");
                    if expert {
                        ui.strong("Octets");
                        ui.strong("Section");
                    }
                    ui.strong("Instruction");
                    ui.end_row();
                    for instruction in &analysis.instructions {
                        ui.monospace(format!("{:#018x}", instruction.address));
                        if expert {
                            ui.monospace(
                                instruction
                                    .bytes
                                    .iter()
                                    .map(|b| format!("{b:02x}"))
                                    .collect::<Vec<_>>()
                                    .join(" "),
                            );
                            ui.monospace(&instruction.section);
                        }
                        ui.monospace(&instruction.text);
                        ui.end_row();
                    }
                });
        });
}
