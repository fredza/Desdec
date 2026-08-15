//! Named functions, their basic blocks, and a local control-flow view.

use std::{
    collections::{BTreeSet, HashMap},
    ops::Range,
};

use desdec_core::{Analysis, Instruction, Symbol};
use eframe::egui;

use crate::{
    i18n::{Language, Text, text},
    ui::{MUTED, card, format_size, syntax},
};

const GRAPH_NODE_SIZE: egui::Vec2 = egui::vec2(230.0, 42.0);
const GRAPH_ROW_HEIGHT: f32 = 68.0;

/// A function selected from the symbol table, with its decoded body and basic
/// blocks. Function boundaries are exact when the symbol gives a size; for
/// zero-sized symbols the next function symbol bounds the decoded body.
struct Function<'a> {
    symbol: &'a Symbol,
    start: u64,
    end: u64,
    instructions: Vec<&'a Instruction>,
    blocks: Vec<BasicBlock>,
}

impl Function<'_> {
    fn size(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }
}

#[derive(Debug, Eq, PartialEq)]
struct BasicBlock {
    start: u64,
    instructions: Range<usize>,
    successors: Vec<u64>,
}

impl BasicBlock {
    fn instruction_count(&self) -> usize {
        self.instructions.len()
    }
}

pub fn show(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    selected_function: &mut Option<u64>,
    language: Language,
) {
    let functions = functions(analysis);
    if functions.is_empty() {
        ui.label(egui::RichText::new(text(language, Text::NoFunctionSymbols)).color(MUTED));
        return;
    }
    if !selected_function.is_some_and(|address| functions.iter().any(|item| item.start == address))
    {
        *selected_function = Some(functions[0].start);
    }

    ui.columns(2, |columns| {
        function_list(&mut columns[0], &functions, selected_function, language);
        let selected = selected_function
            .and_then(|address| functions.iter().find(|function| function.start == address));
        function_details(&mut columns[1], selected, language);
    });
}

fn function_list(
    ui: &mut egui::Ui,
    functions: &[Function<'_>],
    selected_function: &mut Option<u64>,
    language: Language,
) {
    card(ui, text(language, Text::Functions), |ui| {
        egui::ScrollArea::both()
            .id_salt("function_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("function_symbols")
                    .num_columns(3)
                    .striped(true)
                    .spacing([14.0, 6.0])
                    .show(ui, |ui| {
                        for title in [Text::Name, Text::Size, Text::Blocks] {
                            ui.strong(text(language, title));
                        }
                        ui.end_row();

                        for function in functions {
                            let selected = *selected_function == Some(function.start);
                            if ui
                                .selectable_label(selected, &function.symbol.name)
                                .on_hover_text(format!("{:#018x}", function.start))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                *selected_function = Some(function.start);
                            }
                            ui.label(format_size(function.size()));
                            ui.label(function.blocks.len().to_string());
                            ui.end_row();
                        }
                    });
            });
    });
}

fn function_details(ui: &mut egui::Ui, selected: Option<&Function<'_>>, language: Language) {
    let Some(function) = selected else {
        ui.label(egui::RichText::new(text(language, Text::SelectFunction)).color(MUTED));
        return;
    };

    ui.heading(&function.symbol.name);
    ui.monospace(format!("{:#018x}", function.start));
    ui.add_space(8.0);
    let hovered_block = control_flow_graph(ui, function, language);
    ui.add_space(12.0);
    pseudocode(ui, function, hovered_block, language);
}

/// Returns the block currently under the pointer so the pseudo-code can follow
/// the graph without making a click change the selected function.
fn control_flow_graph(
    ui: &mut egui::Ui,
    function: &Function<'_>,
    language: Language,
) -> Option<u64> {
    let mut hovered_block = None;
    card(ui, text(language, Text::ControlFlow), |ui| {
        if function.blocks.is_empty() {
            ui.label(egui::RichText::new(text(language, Text::NoFunctionBody)).color(MUTED));
            return;
        }

        egui::ScrollArea::both()
            .id_salt(("control_flow", function.start))
            .max_height(220.0)
            .show(ui, |ui| {
                let graph_width = ui.available_width().max(520.0);
                let graph_height = function.blocks.len() as f32 * GRAPH_ROW_HEIGHT + 20.0;
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(graph_width, graph_height),
                    egui::Sense::hover(),
                );
                let painter = ui.painter();
                let visuals = ui.style().visuals.clone();
                let positions: HashMap<u64, egui::Rect> = function
                    .blocks
                    .iter()
                    .enumerate()
                    .map(|(index, block)| {
                        let alternate_column = (index % 2) as f32;
                        let x = rect.left() + 24.0 + alternate_column * (GRAPH_NODE_SIZE.x + 70.0);
                        let y = rect.top() + 10.0 + index as f32 * GRAPH_ROW_HEIGHT;
                        (
                            block.start,
                            egui::Rect::from_min_size(egui::pos2(x, y), GRAPH_NODE_SIZE),
                        )
                    })
                    .collect();

                for block in &function.blocks {
                    let Some(node) = positions.get(&block.start) else {
                        continue;
                    };
                    let response = ui.interact(
                        *node,
                        ui.id()
                            .with(("control_flow_block", function.start, block.start)),
                        egui::Sense::hover(),
                    );
                    if response.hovered() {
                        hovered_block = Some(block.start);
                        response.on_hover_ui(|ui| block_details(ui, function, block));
                    }
                }

                for block in &function.blocks {
                    let Some(source) = positions.get(&block.start) else {
                        continue;
                    };
                    for successor in &block.successors {
                        let Some(target) = positions.get(successor) else {
                            continue;
                        };
                        let from = source.center_bottom();
                        let to = target.center_top();
                        let color = visuals.weak_text_color();
                        painter.line_segment([from, to], egui::Stroke::new(1.4_f32, color));
                        let direction = (to - from).normalized();
                        let tip = to - direction * 5.0;
                        let wing = egui::vec2(-direction.y, direction.x) * 4.0;
                        painter.add(egui::Shape::convex_polygon(
                            vec![to, tip + wing, tip - wing],
                            color,
                            egui::Stroke::NONE,
                        ));
                    }
                }

                for (index, block) in function.blocks.iter().enumerate() {
                    let Some(node) = positions.get(&block.start) else {
                        continue;
                    };
                    let fill = if hovered_block == Some(block.start) {
                        visuals.selection.bg_fill
                    } else {
                        visuals.faint_bg_color
                    };
                    painter.rect_filled(*node, 5.0, fill);
                    painter.rect_stroke(
                        *node,
                        5.0,
                        visuals.window_stroke,
                        egui::StrokeKind::Inside,
                    );
                    painter.text(
                        node.left_top() + egui::vec2(8.0, 8.0),
                        egui::Align2::LEFT_TOP,
                        format!("B{}  {:#x}", index + 1, block.start),
                        egui::FontId::monospace(12.0),
                        visuals.text_color(),
                    );
                    painter.text(
                        node.left_bottom() - egui::vec2(-8.0, 7.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{} instr.", block.instruction_count()),
                        egui::FontId::proportional(11.0),
                        visuals.weak_text_color(),
                    );
                }
            });
    });
    hovered_block
}

fn block_details(ui: &mut egui::Ui, function: &Function<'_>, block: &BasicBlock) {
    ui.strong(format!("Bloc {:#x}", block.start));
    ui.separator();
    let transparent = egui::Color32::TRANSPARENT;
    for instruction in &function.instructions[block.instructions.clone()] {
        ui.horizontal(|ui| {
            ui.label(syntax::dim(
                ui,
                &format!("{:#018x}", instruction.address),
                transparent,
            ));
            let bytes = instruction
                .bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            ui.label(syntax::dim(ui, &bytes, transparent));
            ui.label(syntax::assembly(ui, &instruction.text, transparent));
        });
    }
}

fn pseudocode(
    ui: &mut egui::Ui,
    function: &Function<'_>,
    hovered_block: Option<u64>,
    language: Language,
) {
    let highlighted_instructions = hovered_block.and_then(|address| {
        function
            .blocks
            .iter()
            .find(|block| block.start == address)
            .map(|block| block.instructions.clone())
    });

    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.strong(text(language, Text::PseudoCode));
        ui.add_space(8.0);
        if function.instructions.is_empty() {
            ui.label(egui::RichText::new(text(language, Text::NoFunctionBody)).color(MUTED));
            return;
        }
        // The graph is intentionally capped above; the pseudo-code owns all
        // remaining vertical space and scrolls in both directions when needed.
        let available_height = ui.available_height().max(180.0);
        egui::ScrollArea::both()
            .id_salt(("function_pseudocode", function.start))
            .max_height(available_height)
            .show(ui, |ui| {
                let signature = format!("void {}(void) {{", function.symbol.name);
                ui.label(syntax::pseudo_code(
                    ui,
                    &signature,
                    egui::Color32::TRANSPARENT,
                ));
                for (index, instruction) in function.instructions.iter().enumerate() {
                    let highlighted = highlighted_instructions
                        .as_ref()
                        .is_some_and(|range| range.contains(&index));
                    ui.horizontal(|ui| {
                        let fill = if highlighted {
                            ui.style().visuals.selection.bg_fill
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        ui.label(syntax::dim(
                            ui,
                            &format!("{:#018x}", instruction.address),
                            fill,
                        ));
                        let code = format!("    {}", super::decompile::pseudo_c(&instruction.text));
                        let response = ui.label(syntax::pseudo_code(ui, &code, fill));
                        if highlighted
                            && highlighted_instructions
                                .as_ref()
                                .is_some_and(|range| index == range.start)
                        {
                            ui.scroll_to_rect(response.rect, Some(egui::Align::Center));
                        }
                    });
                }
                ui.label(syntax::pseudo_code(ui, "}", egui::Color32::TRANSPARENT));
            });
    });
}

fn functions(analysis: &Analysis) -> Vec<Function<'_>> {
    let mut symbols: Vec<&Symbol> = analysis
        .symbols
        .iter()
        .filter(|symbol| !symbol.imported)
        .filter(|symbol| symbol.address.is_some())
        .collect();
    symbols.sort_by_key(|symbol| symbol.address);
    symbols.dedup_by_key(|symbol| symbol.address);

    let mut instructions: Vec<&Instruction> = analysis.instructions.iter().collect();
    instructions.sort_by_key(|instruction| instruction.address);
    let decoded_end = instructions
        .iter()
        .map(|instruction| {
            instruction
                .address
                .saturating_add(instruction.bytes.len() as u64)
        })
        .max()
        .unwrap_or_default();

    symbols
        .iter()
        .enumerate()
        .map(|(index, symbol)| {
            let start = symbol
                .address
                .expect("function symbols were filtered above");
            let end = if symbol.size > 0 {
                start.saturating_add(symbol.size)
            } else {
                symbols
                    .get(index + 1)
                    .and_then(|next| next.address)
                    .unwrap_or(decoded_end)
            };
            let body: Vec<&Instruction> = instructions
                .iter()
                .copied()
                .filter(|instruction| (start..end).contains(&instruction.address))
                .collect();
            Function {
                symbol,
                start,
                end,
                blocks: basic_blocks(&body),
                instructions: body,
            }
        })
        .collect()
}

fn basic_blocks(instructions: &[&Instruction]) -> Vec<BasicBlock> {
    if instructions.is_empty() {
        return Vec::new();
    }
    let indices: HashMap<u64, usize> = instructions
        .iter()
        .enumerate()
        .map(|(index, instruction)| (instruction.address, index))
        .collect();
    let mut leaders = BTreeSet::from([0]);
    for (index, instruction) in instructions.iter().enumerate() {
        if let Some(target) = branch_target(instruction).and_then(|address| indices.get(&address)) {
            leaders.insert(*target);
        }
        if ends_block(instruction) && index + 1 < instructions.len() {
            leaders.insert(index + 1);
        }
    }
    let leaders: Vec<usize> = leaders.into_iter().collect();
    let block_by_address: HashMap<u64, usize> = leaders
        .iter()
        .enumerate()
        .map(|(block, index)| (instructions[*index].address, block))
        .collect();

    leaders
        .iter()
        .enumerate()
        .map(|(block_index, start_index)| {
            let end_index = leaders
                .get(block_index + 1)
                .copied()
                .unwrap_or(instructions.len());
            let last = instructions[end_index - 1];
            let mut successors = Vec::new();
            if is_conditional_branch(last) {
                push_branch_target(&mut successors, last, &block_by_address);
                push_fallthrough(&mut successors, block_index, &leaders, instructions);
            } else if is_unconditional_jump(last) {
                push_branch_target(&mut successors, last, &block_by_address);
            } else if !is_return(last) {
                push_fallthrough(&mut successors, block_index, &leaders, instructions);
            }
            BasicBlock {
                start: instructions[*start_index].address,
                instructions: *start_index..end_index,
                successors,
            }
        })
        .collect()
}

fn push_branch_target(
    successors: &mut Vec<u64>,
    instruction: &Instruction,
    blocks: &HashMap<u64, usize>,
) {
    if let Some(target) = branch_target(instruction).filter(|address| blocks.contains_key(address))
    {
        successors.push(target);
    }
}

fn push_fallthrough(
    successors: &mut Vec<u64>,
    block_index: usize,
    leaders: &[usize],
    instructions: &[&Instruction],
) {
    if let Some(next) = leaders.get(block_index + 1) {
        successors.push(instructions[*next].address);
    }
}

fn ends_block(instruction: &Instruction) -> bool {
    is_conditional_branch(instruction)
        || is_unconditional_jump(instruction)
        || is_return(instruction)
}

fn is_conditional_branch(instruction: &Instruction) -> bool {
    let opcode = opcode(instruction);
    (opcode.starts_with('j') && !opcode.starts_with("jmp")) || opcode.starts_with("loop")
}

fn is_unconditional_jump(instruction: &Instruction) -> bool {
    opcode(instruction).starts_with("jmp")
}

fn is_return(instruction: &Instruction) -> bool {
    opcode(instruction).starts_with("ret")
}

fn opcode(instruction: &Instruction) -> &str {
    instruction
        .text
        .split_whitespace()
        .next()
        .unwrap_or_default()
}

fn branch_target(instruction: &Instruction) -> Option<u64> {
    instruction
        .text
        .split_whitespace()
        .skip(1)
        .find_map(|part| {
            let candidate = part.trim_matches(|character: char| {
                !character.is_ascii_hexdigit() && !matches!(character, 'x' | 'X' | 'h' | 'H')
            });
            let hexadecimal = candidate
                .strip_prefix("0x")
                .or_else(|| candidate.strip_prefix("0X"))
                .or_else(|| candidate.strip_suffix(['h', 'H']))?;
            u64::from_str_radix(hexadecimal, 16).ok()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instruction(address: u64, text: &str) -> Instruction {
        Instruction {
            address,
            bytes: vec![0x90],
            text: text.to_owned(),
            section: ".text".to_owned(),
        }
    }

    #[test]
    fn conditional_branch_creates_two_successors() {
        let instructions = [
            instruction(0x1000, "cmp %eax,%eax"),
            instruction(0x1002, "je 1006h"),
            instruction(0x1004, "ret"),
            instruction(0x1006, "ret"),
        ];
        let references: Vec<&Instruction> = instructions.iter().collect();

        let blocks = basic_blocks(&references);

        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].instructions, 0..2);
        assert_eq!(blocks[1].instructions, 2..3);
        assert_eq!(blocks[2].instructions, 3..4);
        assert_eq!(blocks[0].successors, [0x1006, 0x1004]);
        assert!(blocks[1].successors.is_empty());
        assert!(blocks[2].successors.is_empty());
    }
}
