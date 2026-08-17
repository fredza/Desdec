//! Printable strings extracted from the loaded binary.

use std::collections::BTreeSet;

use desdec_core::{Analysis, ExtractedString, Instruction};
use eframe::egui;

use crate::{
    app::WorkspaceView,
    i18n::{Language, Text, text},
    ui::{ERROR, MUTED, card},
};

/// Height of one row, needed up front so the list can be virtualised: only the
/// visible rows are laid out, which keeps twenty thousand strings smooth.
const ROW_HEIGHT: f32 = 18.0;

pub fn show(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    filter: &mut String,
    highlight_unmapped: &mut bool,
    highlight_unreferenced: &mut bool,
    selected_string: &mut Option<u64>,
    selected_instruction: &mut Option<u64>,
    pending_instruction_scroll: &mut Option<u64>,
    instruction_attention: &mut Option<(u64, f64)>,
    active_view: &mut WorkspaceView,
    language: Language,
) {
    if analysis.strings.is_empty() {
        ui.label(text(language, Text::NoStrings));
        return;
    }

    let matches = matching(
        analysis,
        filter,
        *highlight_unmapped,
        *highlight_unreferenced,
    );
    header(
        ui,
        filter,
        highlight_unmapped,
        highlight_unreferenced,
        matches.len(),
        analysis.strings.len(),
        language,
    );
    ui.add_space(8.0);

    if let Some(string) = selected_string.and_then(|offset| {
        analysis
            .strings
            .iter()
            .find(|string| string.file_offset == offset)
    }) {
        references(
            ui,
            analysis,
            string,
            selected_instruction,
            pending_instruction_scroll,
            instruction_attention,
            active_view,
            language,
        );
        ui.add_space(8.0);
    }

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, matches.len(), |ui, range| {
            egui::Grid::new("strings")
                .num_columns(3)
                .striped(true)
                .spacing([18.0, 4.0])
                .min_row_height(ROW_HEIGHT)
                .show(ui, |ui| {
                    for item in &matches[range] {
                        if row(
                            ui,
                            item,
                            *selected_string == Some(item.string.file_offset),
                            language,
                        ) {
                            *selected_string = Some(item.string.file_offset);
                        }
                        ui.end_row();
                    }
                });
        });
}

fn header(
    ui: &mut egui::Ui,
    filter: &mut String,
    highlight_unmapped: &mut bool,
    highlight_unreferenced: &mut bool,
    shown: usize,
    total: usize,
    language: Language,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(text(language, Text::FilterStrings));
        ui.add(
            egui::TextEdit::singleline(filter)
                .hint_text(text(language, Text::FilterHint))
                .desired_width(240.0),
        );
        ui.separator();
        ui.toggle_value(
            highlight_unmapped,
            text(language, Text::FilterUnmappedStrings),
        );
        ui.toggle_value(
            highlight_unreferenced,
            text(language, Text::FilterUnreferencedStrings),
        );
        ui.label(
            egui::RichText::new(format!(
                "{shown} {} {total}",
                text(language, Text::ShownOfTotal)
            ))
            .color(MUTED),
        );
    });

    // The extractor stops at a fixed number of strings; say so rather than let
    // the list look complete.
    if total >= desdec_core::strings::MAXIMUM_COUNT {
        ui.small(text(language, Text::StringLimitReached));
    }
}

struct StringMatch<'a> {
    string: &'a ExtractedString,
    unmapped: bool,
    unreferenced: bool,
    highlight: bool,
}

fn matching<'a>(
    analysis: &'a Analysis,
    filter: &str,
    highlight_unmapped: bool,
    highlight_unreferenced: bool,
) -> Vec<StringMatch<'a>> {
    let needle = filter.to_lowercase();
    // Resolve decoded operands once, not once per string: a stripped binary can
    // contain twenty thousand strings, while its instructions often outnumber
    // them by an order of magnitude.
    let referenced = direct_reference_addresses(analysis);

    analysis
        .strings
        .iter()
        .filter(|string| filter.is_empty() || string.value.to_lowercase().contains(&needle))
        .map(|string| {
            let address = string_address(analysis, string);
            let unmapped = address.is_none();
            let unreferenced = !address.is_some_and(|address| referenced.contains(&address));
            StringMatch {
                string,
                unmapped,
                unreferenced,
                highlight: (highlight_unmapped && unmapped)
                    || (highlight_unreferenced && unreferenced),
            }
        })
        .collect()
}

fn direct_reference_addresses(analysis: &Analysis) -> BTreeSet<u64> {
    analysis
        .instructions
        .iter()
        .flat_map(instruction_addresses)
        .collect()
}

fn row(ui: &mut egui::Ui, item: &StringMatch<'_>, selected: bool, language: Language) -> bool {
    let string = item.string;
    ui.monospace(format!("{:#010x}", string.file_offset));
    ui.label(egui::RichText::new(string.encoding.label()).color(MUTED));

    let value = if string.truncated {
        format!("{}…", string.value)
    } else {
        string.value.clone()
    };
    let response = ui
        .add(
            egui::Label::new(
                egui::RichText::new(value).monospace().background_color(
                    selected
                        .then_some(ui.style().visuals.selection.bg_fill)
                        .unwrap_or(egui::Color32::TRANSPARENT),
                ),
            )
            .sense(egui::Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if item.highlight || response.hovered() {
        let color = item
            .highlight
            .then_some(ERROR)
            .unwrap_or(egui::Color32::from_rgb(241, 169, 75));
        ui.painter().rect_stroke(
            response.rect.expand(1.0),
            2.0,
            egui::Stroke::new(0.5_f32, color),
            egui::StrokeKind::Outside,
        );
    }
    if response.hovered() && item.highlight {
        let mut reasons = Vec::new();
        if item.unmapped {
            reasons.push(text(language, Text::StringAddressUnavailable));
        }
        if item.unreferenced {
            reasons.push(text(language, Text::NoStringReferences));
        }
        response.clone().on_hover_text(reasons.join("\n"));
    }
    response.clicked()
}

fn references(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    string: &ExtractedString,
    selected_instruction: &mut Option<u64>,
    pending_instruction_scroll: &mut Option<u64>,
    instruction_attention: &mut Option<(u64, f64)>,
    active_view: &mut WorkspaceView,
    language: Language,
) {
    card(ui, text(language, Text::StringReferences), |ui| {
        ui.monospace(&string.value);
        let Some(address) = string_address(analysis, string) else {
            ui.small(text(language, Text::StringAddressUnavailable));
            return;
        };
        ui.small(format!("{address:#018x}"));

        let references = direct_references(analysis, address);
        if references.is_empty() {
            ui.small(text(language, Text::NoStringReferences));
            return;
        }
        for instruction in references {
            ui.horizontal(|ui| {
                ui.monospace(format!(
                    "{:#018x}  {}",
                    instruction.address, instruction.text
                ));
                if ui.button(text(language, Text::GoToDisassembly)).clicked() {
                    *selected_instruction = Some(instruction.address);
                    *pending_instruction_scroll = Some(instruction.address);
                    *instruction_attention = Some((
                        instruction.address,
                        ui.ctx().input(|input| input.time) + 3.0,
                    ));
                    *active_view = WorkspaceView::Disassembly;
                    ui.ctx().request_repaint();
                }
            });
        }
    });
}

/// Translates a file offset from the string extractor to its memory address.
fn string_address(analysis: &Analysis, string: &ExtractedString) -> Option<u64> {
    analysis.sections.iter().find_map(|section| {
        let end = section.file_offset.saturating_add(section.file_size);
        (section.is_mapped() && (section.file_offset..end).contains(&string.file_offset)).then(
            || {
                section
                    .virtual_address
                    .saturating_add(string.file_offset.saturating_sub(section.file_offset))
            },
        )
    })
}

/// Finds direct and RIP-relative operands in the decoded x86 text. Indirect
/// references need full instruction-semantic analysis and are left out rather
/// than reported as false positives.
fn direct_references<'a>(analysis: &'a Analysis, address: u64) -> Vec<&'a Instruction> {
    analysis
        .instructions
        .iter()
        .filter(|instruction| instruction_addresses(instruction).contains(&address))
        .collect()
}

fn instruction_addresses(instruction: &Instruction) -> Vec<u64> {
    if let Some(target) = rip_relative_target(instruction) {
        return vec![target];
    }

    instruction
        .text
        .split(|character: char| {
            !character.is_ascii_hexdigit() && !matches!(character, 'x' | 'X' | 'h' | 'H')
        })
        .filter_map(|candidate| {
            let hexadecimal = candidate
                .strip_prefix("0x")
                .or_else(|| candidate.strip_prefix("0X"))
                .or_else(|| candidate.strip_suffix(['h', 'H']))?;
            u64::from_str_radix(hexadecimal, 16).ok()
        })
        .collect()
}

/// x86-64 string literals are commonly addressed relative to `%rip`. The
/// formatter preserves that displacement, so recover the target from the next
/// instruction address instead of treating the displacement as an absolute.
fn rip_relative_target(instruction: &Instruction) -> Option<u64> {
    let operand = instruction
        .text
        .split_whitespace()
        .skip(1)
        .find(|part| part.contains("%rip"))?;
    let displacement = operand.split('(').next()?.trim_start_matches('$');
    let (negative, hexadecimal) = match displacement.strip_prefix('-') {
        Some(value) => (true, value),
        None => (false, displacement),
    };
    let hexadecimal = hexadecimal
        .strip_prefix("0x")
        .or_else(|| hexadecimal.strip_prefix("0X"))
        .or_else(|| hexadecimal.strip_suffix(['h', 'H']))?;
    let magnitude = i64::try_from(u64::from_str_radix(hexadecimal, 16).ok()?).ok()?;
    let signed_displacement = if negative { -magnitude } else { magnitude };
    instruction
        .address
        .saturating_add(instruction.bytes.len() as u64)
        .checked_add_signed(signed_displacement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use desdec_core::{
        Architecture, BinaryFormat, BinarySummary, Endianness, Permissions, Section, StringEncoding,
    };
    use std::path::PathBuf;

    fn analysis_for_filters() -> Analysis {
        Analysis {
            summary: BinarySummary {
                path: PathBuf::from("test.bin"),
                size: 0,
                format: BinaryFormat::Elf {
                    bits: 64,
                    endianness: Endianness::Little,
                },
                architecture: Architecture::X86_64,
            },
            entry_point: None,
            sections: vec![Section {
                name: ".rodata".to_owned(),
                virtual_address: 0x4000,
                file_offset: 0x100,
                virtual_size: 0x100,
                file_size: 0x100,
                permissions: Permissions {
                    read: true,
                    ..Permissions::default()
                },
                entropy: None,
            }],
            strings: vec![
                ExtractedString {
                    file_offset: 0x110,
                    encoding: StringEncoding::Ascii,
                    value: "referenced".to_owned(),
                    truncated: false,
                },
                ExtractedString {
                    file_offset: 0x120,
                    encoding: StringEncoding::Ascii,
                    value: "unreferenced".to_owned(),
                    truncated: false,
                },
                ExtractedString {
                    file_offset: 0x20,
                    encoding: StringEncoding::Ascii,
                    value: "not mapped".to_owned(),
                    truncated: false,
                },
            ],
            symbols: Vec::new(),
            instructions: vec![Instruction {
                address: 0x5000,
                bytes: vec![0x48, 0x8d, 0x05],
                text: "mov $4010h,%rax".to_owned(),
                section: ".text".to_owned(),
            }],
            details: Default::default(),
            languages: Vec::new(),
            sha256: None,
            entropy: None,
            analysed_bytes: 0,
            truncated: false,
        }
    }

    #[test]
    fn filters_highlight_their_matches_without_hiding_any_string() {
        let analysis = analysis_for_filters();
        let status = |unmapped, unreferenced| {
            matching(&analysis, "", unmapped, unreferenced)
                .into_iter()
                .map(|item| (item.string.value.as_str(), item.highlight))
                .collect::<Vec<_>>()
        };

        assert_eq!(
            status(false, false),
            [
                ("referenced", false),
                ("unreferenced", false),
                ("not mapped", false)
            ]
        );
        assert_eq!(
            status(true, false),
            [
                ("referenced", false),
                ("unreferenced", false),
                ("not mapped", true)
            ]
        );
        assert_eq!(
            status(false, true),
            [
                ("referenced", false),
                ("unreferenced", true),
                ("not mapped", true)
            ]
        );
    }

    #[test]
    fn reads_direct_gas_hex_operands_for_string_references() {
        let instruction = Instruction {
            address: 0x401000,
            bytes: vec![0x48, 0x8d, 0x05],
            text: "mov $402000h,%rax".to_owned(),
            section: ".text".to_owned(),
        };

        assert_eq!(instruction_addresses(&instruction), [0x402000]);
    }

    #[test]
    fn resolves_rip_relative_string_references() {
        let instruction = Instruction {
            address: 0x400ff0,
            bytes: vec![0x48, 0x8d, 0x05, 0x09, 0x10, 0x00, 0x00],
            text: "leaq 0x1009(%rip),%rax".to_owned(),
            section: ".text".to_owned(),
        };

        assert_eq!(instruction_addresses(&instruction), [0x402000]);
    }
}
