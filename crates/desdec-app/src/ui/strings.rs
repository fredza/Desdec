//! Printable strings extracted from the loaded binary.

use std::collections::BTreeSet;

use desdec_core::{Analysis, ExtractedString, Instruction};
use eframe::egui;

use crate::{
    app::WorkspaceView,
    i18n::{Language, Text, text},
    ui::{MUTED, ROW_HEIGHT, card},
};

pub fn show(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    filter: &mut String,
    hide_unmapped: &mut bool,
    hide_unreferenced: &mut bool,
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

    let matches = matching(analysis, filter, *hide_unmapped, *hide_unreferenced);
    header(
        ui,
        filter,
        hide_unmapped,
        hide_unreferenced,
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

    // The grid's vertical spacing has to be the one the virtualiser assumed
    // when placing this batch of rows, or the two disagree by a pixel a row.
    let row_spacing = ui.spacing().item_spacing.y;
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, matches.len(), |ui, range| {
            egui::Grid::new("strings")
                .num_columns(3)
                .striped(true)
                .spacing([18.0, row_spacing])
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
    hide_unmapped: &mut bool,
    hide_unreferenced: &mut bool,
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
        criteria(ui, hide_unmapped, hide_unreferenced, language);

        let filtering = !filter.is_empty() || *hide_unmapped || *hide_unreferenced;
        if ui
            .add_enabled(
                filtering,
                egui::Button::new(text(language, Text::ClearFilter)),
            )
            .clicked()
        {
            filter.clear();
            *hide_unmapped = false;
            *hide_unreferenced = false;
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // How many were left out is part of the answer: a narrowed list
            // that does not say so looks like a file with fewer strings in it.
            let counted = egui::RichText::new(format!(
                "{shown} {} {total}",
                text(language, Text::ShownOfTotal)
            ));
            // Emphasised while something is filtering, so a short list is
            // never mistaken for a file with little in it.
            ui.label(if filtering {
                counted.strong()
            } else {
                counted.color(MUTED)
            });
        });
    });

    // The extractor stops at a fixed number of strings; say so rather than let
    // the list look complete.
    if total >= desdec_core::strings::MAXIMUM_COUNT {
        ui.small(text(language, Text::StringLimitReached));
    }
}

/// The criteria, folded into one drop-down.
///
/// They were two toggle buttons sitting in the header, which is a row that
/// only ever grows: the next criterion would have pushed the count off a
/// narrow window. Folded away, the header stays one line whatever is added,
/// and the button says what is active without being opened.
fn criteria(
    ui: &mut egui::Ui,
    hide_unmapped: &mut bool,
    hide_unreferenced: &mut bool,
    language: Language,
) {
    let chosen = usize::from(*hide_unmapped) + usize::from(*hide_unreferenced);
    let summary = match (*hide_unmapped, *hide_unreferenced) {
        (false, false) => text(language, Text::AllStrings).to_owned(),
        (true, false) => text(language, Text::FilterUnmappedStrings).to_owned(),
        (false, true) => text(language, Text::FilterUnreferencedStrings).to_owned(),
        (true, true) => format!("{chosen} {}", text(language, Text::CriteriaChosen)),
    };

    egui::ComboBox::from_id_salt("strings_criteria")
        .selected_text(summary)
        .width(220.0)
        .show_ui(ui, |ui| {
            // Checkboxes rather than entries that pick one: these narrow the
            // list together, and a drop-down that closed on the first click
            // would make the second criterion a second visit.
            ui.checkbox(hide_unmapped, text(language, Text::FilterUnmappedStrings));
            ui.small(text(language, Text::FilterUnmappedHelp));
            ui.add_space(4.0);
            ui.checkbox(
                hide_unreferenced,
                text(language, Text::FilterUnreferencedStrings),
            );
            ui.small(text(language, Text::FilterUnreferencedHelp));
        })
        .response
        .on_hover_text(text(language, Text::FilterCriteriaHelp));
}

struct StringMatch<'a> {
    string: &'a ExtractedString,
    unmapped: bool,
    unreferenced: bool,
}

/// The strings a reader has asked to see.
///
/// The criteria hide the noise rather than isolate it. A binary's string table
/// is mostly padding, format fragments and dead constants; what a reader is
/// after is the handful the code actually reaches. So each criterion drops the
/// strings that fail it — an unmapped string is never loaded, an unreferenced
/// one is never pointed at — and what is left is what the program uses. What
/// is hidden is never hidden silently, since the header says how many of the
/// total are shown.
fn matching<'a>(
    analysis: &'a Analysis,
    filter: &str,
    hide_unmapped: bool,
    hide_unreferenced: bool,
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
            StringMatch {
                string,
                unmapped: address.is_none(),
                unreferenced: !address.is_some_and(|address| referenced.contains(&address)),
            }
        })
        // Several criteria narrow together: each one is a condition the string
        // has to meet, not another list added to the first.
        .filter(|item| !hide_unmapped || !item.unmapped)
        .filter(|item| !hide_unreferenced || !item.unreferenced)
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
    if response.hovered() {
        ui.painter().rect_stroke(
            response.rect.expand(1.0),
            2.0,
            egui::Stroke::new(0.5_f32, egui::Color32::from_rgb(241, 169, 75)),
            egui::StrokeKind::Outside,
        );
        // What is odd about a string is said on the string itself, whether or
        // not a criterion is filtering on it: a reader hovering a line should
        // not have to switch a filter on to be told why it stands out.
        let mut reasons = Vec::new();
        if item.unmapped {
            reasons.push(text(language, Text::StringAddressUnavailable));
        }
        if item.unreferenced {
            reasons.push(text(language, Text::NoStringReferences));
        }
        if !reasons.is_empty() {
            response.clone().on_hover_text(reasons.join("\n"));
        }
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
                bytes: desdec_core::InstructionBytes::new(&[0x48, 0x8d, 0x05]).expect("short"),
                text: "mov $4010h,%rax".to_owned(),
                section: std::sync::Arc::from(".text"),
            }],
            code_truncated: false,
            details: Default::default(),
            languages: Vec::new(),
            sha256: None,
            entropy: None,
            analysed_bytes: 0,
            truncated: false,
        }
    }

    /// Each criterion hides what fails it, and several hide together. This is
    /// the direction that makes the view useful: a string table is mostly
    /// noise, and the reader is after the few strings the code actually
    /// reaches — not after the noise on its own.
    #[test]
    fn each_criterion_hides_what_fails_it_and_they_hide_together() {
        let analysis = analysis_for_filters();
        let shown = |hide_unmapped, hide_unreferenced| {
            matching(&analysis, "", hide_unmapped, hide_unreferenced)
                .into_iter()
                .map(|item| item.string.value.clone())
                .collect::<Vec<_>>()
        };

        assert_eq!(
            shown(false, false),
            ["referenced", "unreferenced", "not mapped"],
            "with no criterion, every string is shown"
        );
        assert_eq!(
            shown(true, false),
            ["referenced", "unreferenced"],
            "hiding the unmapped drops the one outside every mapped section"
        );
        assert_eq!(
            shown(false, true),
            ["referenced"],
            "hiding the unreferenced drops everything no instruction points at"
        );
        assert_eq!(
            shown(true, true),
            ["referenced"],
            "both at once leave only what is both loaded and pointed at"
        );
    }

    /// The text filter and the criteria narrow together too.
    #[test]
    fn the_text_filter_applies_alongside_the_criteria() {
        let analysis = analysis_for_filters();
        let shown: Vec<String> = matching(&analysis, "referenc", false, true)
            .into_iter()
            .map(|item| item.string.value.clone())
            .collect();

        assert_eq!(
            shown, ["referenced"],
            "the text matches two strings; the criterion drops the unreferenced one"
        );
    }

    #[test]
    fn reads_direct_gas_hex_operands_for_string_references() {
        let instruction = Instruction {
            address: 0x401000,
            bytes: desdec_core::InstructionBytes::new(&[0x48, 0x8d, 0x05]).expect("short"),
            text: "mov $402000h,%rax".to_owned(),
            section: std::sync::Arc::from(".text"),
        };

        assert_eq!(instruction_addresses(&instruction), [0x402000]);
    }

    #[test]
    fn resolves_rip_relative_string_references() {
        let instruction = Instruction {
            address: 0x400ff0,
            bytes: desdec_core::InstructionBytes::new(&[0x48, 0x8d, 0x05, 0x09, 0x10, 0x00, 0x00])
                .expect("short"),
            text: "leaq 0x1009(%rip),%rax".to_owned(),
            section: std::sync::Arc::from(".text"),
        };

        assert_eq!(instruction_addresses(&instruction), [0x402000]);
    }
}
