//! Detailed x86/x86-64 decoding, synchronised with the local pseudo-code.
use crate::{
    i18n::{Language, Text, text},
    patches::Patches,
    ui::MUTED,
    ui::decompile,
    ui::syntax,
};
use desdec_core::Analysis;
use eframe::egui;

/// Bytes a pending patch would write, marked so an edited row is never taken
/// for what the file currently holds.
const PATCHED: egui::Color32 = egui::Color32::from_rgb(224, 164, 104);

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Draws the disassembly, returning the instruction the user asked to edit.
pub fn show(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    selected_instruction: &mut Option<u64>,
    pending_scroll: &mut Option<u64>,
    instruction_attention: &mut Option<(u64, f64)>,
    patches: &Patches,
    language: Language,
) -> Action {
    if analysis.instructions.is_empty() {
        ui.label(egui::RichText::new(text(language, Text::NoDisassembly)).color(MUTED));
        return Action::default();
    }
    let mut action = Action::default();
    ui.horizontal(|ui| {
        let selected = *selected_instruction;
        let patchable = selected
            .is_some_and(|address| crate::patches::file_offset_of(analysis, address).is_some());
        let button = ui.add_enabled(
            selected.is_some() && patchable,
            egui::Button::new(text(language, Text::EditInstruction)),
        );
        if button.clicked() {
            action.edit = selected;
        }
        if selected.is_some() && !patchable {
            ui.label(egui::RichText::new(text(language, Text::NotPatchable)).color(MUTED));
        } else {
            ui.small(text(language, Text::LocalDecoders));
        }
    });
    ui.add_space(8.0);
    let scroll_target = *pending_scroll;
    let attention = decompile::active_attention(ui.ctx(), instruction_attention);
    ui.columns(2, |columns| {
        columns[1].strong(text(language, Text::PseudoCode));
        columns[1].small(text(language, Text::PseudoCodeHelp));
        decompile::panel(
            &mut columns[1],
            analysis,
            selected_instruction,
            scroll_target,
            pending_scroll,
            attention,
        );
        action.inspect = instructions(
            &mut columns[0],
            analysis,
            selected_instruction,
            scroll_target,
            pending_scroll,
            attention,
            patches,
            language,
        );
    });
    if *pending_scroll == scroll_target {
        *pending_scroll = None;
    }
    action
}

/// What the reader asked of the disassembly this frame.
#[derive(Default)]
pub struct Action {
    /// An instruction whose bytes are to be edited.
    pub edit: Option<u64>,
    /// An instruction whose operand is to be explained.
    pub inspect: Option<u64>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "one listing needs its selection, its scrolling and its patches"
)]
fn instructions(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    selected_instruction: &mut Option<u64>,
    scroll_target: Option<u64>,
    pending_scroll: &mut Option<u64>,
    attention: Option<u64>,
    patches: &Patches,
    language: Language,
) -> Option<u64> {
    let mut inspect = None;
    decompile::ensure_selected_instruction(analysis, selected_instruction);
    egui::ScrollArea::both()
        .id_salt("instructions")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("disassembly")
                .num_columns(4)
                .striped(true)
                .show(ui, |ui| {
                    for title in [Text::Address, Text::Bytes, Text::Section, Text::Instruction] {
                        ui.strong(text(language, title));
                    }
                    ui.end_row();
                    for instruction in &analysis.instructions {
                        let selected_fill = decompile::instruction_fill(
                            ui,
                            instruction.address,
                            *selected_instruction,
                            attention,
                        );
                        let patch = patches.patch_at(instruction.address);
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
                        // A patched row shows the bytes that would be written,
                        // marked, rather than the ones still in the file: the
                        // listing must describe the binary being built.
                        let bytes =
                            hex(patch.map_or(&instruction.bytes, |patch| &patch.replacement));
                        match patch {
                            Some(patch) => {
                                ui.label(
                                    egui::RichText::new(format!("{bytes} *"))
                                        .monospace()
                                        .color(PATCHED),
                                )
                                .on_hover_text(format!(
                                    "{} {}",
                                    text(language, Text::OriginalBytes),
                                    hex(&patch.original)
                                ));
                            }
                            None => {
                                ui.label(syntax::dim(ui, &bytes, egui::Color32::TRANSPARENT));
                            }
                        }
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
                        // The right button asks what the operand designates,
                        // which is the question an address in a listing
                        // provokes and which no amount of hovering answers.
                        assembly.context_menu(|ui| {
                            if ui.button(text(language, Text::InspectOperand)).clicked() {
                                inspect = Some(instruction.address);
                                ui.close_menu();
                            }
                        });
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
    inspect
}
