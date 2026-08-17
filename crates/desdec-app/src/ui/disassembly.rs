//! Detailed x86/x86-64 decoding, synchronised with the local pseudo-code.
use crate::{
    i18n::{Language, Text, text},
    patches::Patches,
    ui::ERROR,
    ui::MUTED,
    ui::ROW_HEIGHT,
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
    // A listing that stopped at the decoder's limit looks exactly like a
    // program that ends there, so it says which one this is.
    if analysis.code_truncated {
        ui.colored_label(ERROR, text(language, Text::TruncatedDisassembly));
    }
    ui.add_space(8.0);
    let scroll_target = *pending_scroll;
    let attention = decompile::active_attention(ui.ctx(), instruction_attention);
    ui.columns(2, |columns| {
        columns[1].strong(text(language, Text::PseudoCode));
        columns[1].small(text(language, Text::PseudoCodeHelp));
        // Clicking a pseudo-code line here only moves the selection: the
        // assembly it stands for is already in the left column, so the address
        // the panel reports has no window to open.
        let _selected_by_click = decompile::panel(
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
    // Only the visible rows are laid out: a decoded binary reaches a hundred
    // thousand instructions, and a widget for each took seconds per frame.
    // The virtualiser draws no row the reader is scrolled away from, so
    // bringing one into view is done by offset rather than by asking an
    // unlaid-out row to scroll itself.
    let area = decompile::listing_area(
        egui::ScrollArea::both().id_salt("instructions"),
        ui,
        analysis,
        scroll_target,
        1,
    );
    area.auto_shrink([false, false]).show_rows(
        ui,
        ROW_HEIGHT,
        analysis.instructions.len() + 1,
        |ui, rows| {
            egui::Grid::new("disassembly")
                .num_columns(4)
                .striped(true)
                .min_row_height(ROW_HEIGHT)
                .show(ui, |ui| {
                    if rows.start == 0 {
                        for title in [Text::Address, Text::Bytes, Text::Section, Text::Instruction]
                        {
                            ui.strong(text(language, title));
                        }
                        ui.end_row();
                    }
                    let body = decompile::rows_of(&analysis.instructions, &rows, 1);
                    for instruction in body {
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
                        ui.end_row();
                    }
                });
        },
    );
    inspect
}

#[cfg(test)]
mod tests {
    use crate::{
        app::WorkspaceView,
        testing::{drawn, drawn_text, opened_app, reference_analysis, window_input},
        ui::views,
    };
    use eframe::egui;

    /// The listing is virtualised, so the row an instruction sits on is not
    /// laid out until it is scrolled to. Reaching one far down the listing has
    /// to actually put it on screen, or every cross-reference in the interface
    /// leads nowhere.
    #[test]
    fn scrolling_to_a_distant_instruction_brings_it_into_view() {
        let analysis = reference_analysis();
        let Some(target) = analysis.instructions.last() else {
            return; // Nothing decoded on this host: nothing to scroll to.
        };
        let address = target.address;
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.selected_instruction = Some(address);
        app.pending_instruction_scroll = Some(address);

        let ctx = egui::Context::default();
        // The first frame is what the scroll area learns its content size
        // from; the offset it was given lands on the second.
        let _ = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });
        app.pending_instruction_scroll = Some(address);
        let output = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });

        // Twice: the disassembly and the pseudo-code beside it are two
        // virtualised listings, and both have to follow the same address.
        let drawn = drawn_text(&output.shapes);
        assert_eq!(
            drawn.matches(&format!("{address:#018x}")).count(),
            2,
            "both listings must scroll to the instruction"
        );
    }

    /// The virtualiser is told how tall a row is before drawing one, so a row
    /// that grew taller than [`crate::ui::ROW_HEIGHT`] would drift away from
    /// the position the offset was computed for — the further down the
    /// listing, the further off.
    #[test]
    fn rows_are_as_tall_as_the_virtualiser_was_told() {
        if reference_analysis().instructions.is_empty() {
            return;
        }
        let mut app = opened_app(WorkspaceView::Disassembly);
        let ctx = egui::Context::default();
        let output = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });

        // Addresses are drawn once per row, so their vertical positions are
        // the row positions.
        let mut rows: Vec<f32> = drawn(&output.shapes)
            .into_iter()
            .filter(|(text, _)| text.starts_with("0x00"))
            .map(|(_, position)| position.y)
            .collect();
        rows.sort_by(f32::total_cmp);
        rows.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        assert!(rows.len() > 5, "the listing must have drawn several rows");

        let spacing = ctx.style().spacing.item_spacing.y;
        let tallest = rows
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .fold(0.0_f32, f32::max);
        assert!(
            tallest <= crate::ui::ROW_HEIGHT + spacing + 0.5,
            "a row was {tallest} tall, more than the {} the virtualiser assumes",
            crate::ui::ROW_HEIGHT + spacing
        );
    }
}
