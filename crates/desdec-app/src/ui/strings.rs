//! Printable strings extracted from the loaded binary.

use desdec_core::{Analysis, ExtractedString, Instruction};
use eframe::egui;

use crate::{
    i18n::{Language, Text, text},
    ui::{MUTED, ROW_HEIGHT, card},
};

/// What a strings card asked of the workspace.
///
/// The card itself deliberately does not change the shared instruction
/// selection.  Its caller routes this request through `go_to_address`, the
/// one navigation gateway that keeps every caller, the listing and its
/// pseudo-code counterpart on the same instruction.
#[derive(Default)]
pub struct Action {
    pub copy: Option<String>,
    pub go_to: Option<u64>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "the view needs its filters, its selection and the cross-references"
)]
pub fn show(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    references: &CodeReferences,
    filter: &mut String,
    scope: &mut Scope,
    selected_string: &mut Option<u64>,
    language: Language,
) -> Action {
    let mut action = Action::default();
    if analysis.strings.is_empty() {
        ui.label(text(language, Text::NoStrings));
        return action;
    }

    let matches = matching(
        analysis,
        references,
        filter,
        *scope,
    );
    header(
        ui,
        filter,
        scope,
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
        action = reference_card(ui, analysis, references, string, language);
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
    action
}

fn header(
    ui: &mut egui::Ui,
    filter: &mut String,
    scope: &mut Scope,
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
        criteria(ui, scope, language);

        let filtering = !filter.is_empty() || *scope != Scope::All;
        if ui
            .add_enabled(
                filtering,
                egui::Button::new(text(language, Text::ClearFilter)),
            )
            .clicked()
        {
            filter.clear();
            *scope = Scope::All;
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
/// They were two toggle buttons — "hide the unmapped", "hide the unreferenced"
/// — folded into a drop-down. Two things were wrong with that. Both were
/// **double negatives**, and a reader wanting the strings the program uses had
/// to work out that it means switching both of them on. And the combination a
/// reader actually asks for most often could not be expressed at all: *mapped
/// and unreferenced* — loaded into memory, and yet nothing in the decoded code
/// points at them — which is where a string reached by arithmetic, through a
/// table, or built at run time shows up, and is the most interesting question
/// the string table answers.
///
/// So the criteria are four named scopes, said in the positive, on the row
/// itself. One click, no combination to work out, and the one that was
/// impossible is now the third of them.
fn criteria(ui: &mut egui::Ui, scope: &mut Scope, language: Language) {
    ui.small(text(language, Text::StringsScope));
    for choice in Scope::ALL {
        let response = ui.selectable_label(*scope == *choice, choice.label(language));
        let response = match choice.help(language) {
            Some(help) => response.on_hover_text(help),
            None => response,
        };
        if response.clicked() {
            *scope = *choice;
        }
    }
}

/// Which strings a reader is asking to see.
///
/// Said in the positive and named for what the strings *are*, not for what is
/// being hidden.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Scope {
    #[default]
    All,
    /// Mapped and pointed at by the decoded code: what the program uses.
    Used,
    /// Mapped, and named by nothing in the decoded code. A string reached by
    /// arithmetic, through a table, or assembled at run time lands here — and
    /// so does one only an undecoded part of the file reaches.
    MappedUnreferenced,
    /// In the file and never loaded. A run cannot read these at all.
    Unmapped,
}

impl Scope {
    pub const ALL: &'static [Self] = &[
        Self::All,
        Self::Used,
        Self::MappedUnreferenced,
        Self::Unmapped,
    ];

    pub fn label(self, language: Language) -> &'static str {
        text(
            language,
            match self {
                Self::All => Text::StringsScopeAll,
                Self::Used => Text::StringsScopeUsed,
                Self::MappedUnreferenced => Text::StringsScopeMappedUnreferenced,
                Self::Unmapped => Text::StringsScopeUnmapped,
            },
        )
    }

    /// What the scope means, for a reader who has not met the distinction
    /// before. `All` needs none: it is the absence of a question.
    pub fn help(self, language: Language) -> Option<&'static str> {
        Some(text(
            language,
            match self {
                Self::All => return None,
                Self::Used => Text::StringsScopeUsedHelp,
                Self::MappedUnreferenced => Text::StringsScopeMappedUnreferencedHelp,
                Self::Unmapped => Text::StringsScopeUnmappedHelp,
            },
        ))
    }

    /// Whether a string belongs in this scope.
    const fn keeps(self, unmapped: bool, unreferenced: bool) -> bool {
        match self {
            Self::All => true,
            Self::Used => !unmapped && !unreferenced,
            Self::MappedUnreferenced => !unmapped && unreferenced,
            Self::Unmapped => unmapped,
        }
    }
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
    references: &CodeReferences,
    filter: &str,
    scope: Scope,
) -> Vec<StringMatch<'a>> {
    let needle = filter.to_lowercase();

    analysis
        .strings
        .iter()
        .filter(|string| filter.is_empty() || string.value.to_lowercase().contains(&needle))
        .map(|string| {
            let address = string_address(analysis, string);
            StringMatch {
                string,
                unmapped: address.is_none(),
                unreferenced: !address.is_some_and(|address| references.any(address)),
            }
        })
        // Several criteria narrow together: each one is a condition the string
        // has to meet, not another list added to the first.
        .filter(|item| scope.keeps(item.unmapped, item.unreferenced))
        .collect()
}

/// Where the decoded code points, resolved once when a binary is opened.
///
/// Every row of the listing asks whether anything refers to its string, and
/// the answer used to be recomputed from scratch on every frame drawn: a pass
/// over a million instructions, parsing each one's text and allocating a
/// vector per instruction, sixty times a second. It is one sorted table now,
/// built once, and both questions the view asks — *is this referenced* and *by
/// what* — are a binary search into it.
///
/// Measured on a 1.2-million-instruction binary: a tenth of a second to build,
/// once, against a millisecond a frame to filter twenty thousand strings
/// through it.
#[derive(Debug, Default)]
pub struct CodeReferences {
    /// `(target, the instruction naming it)`, sorted by target.
    entries: Vec<(u64, u64)>,
}

impl CodeReferences {
    #[must_use]
    pub fn of(analysis: &Analysis) -> Self {
        let mut entries: Vec<(u64, u64)> = analysis
            .instructions
            .iter()
            .flat_map(|instruction| {
                instruction_addresses(instruction)
                    .into_iter()
                    .map(|target| (target, instruction.address))
            })
            .collect();
        entries.sort_unstable();
        entries.dedup();
        entries.shrink_to_fit();
        Self { entries }
    }

    /// The entries naming `target`, as one contiguous run of the table.
    fn span(&self, target: u64) -> &[(u64, u64)] {
        let start = self.entries.partition_point(|(at, _)| *at < target);
        let end = self.entries.partition_point(|(at, _)| *at <= target);
        &self.entries[start..end]
    }

    /// Whether any decoded instruction names this address.
    #[must_use]
    pub fn any(&self, target: u64) -> bool {
        !self.span(target).is_empty()
    }

    /// The instructions that name it, by address and in listing order.
    pub fn instructions(&self, target: u64) -> impl Iterator<Item = u64> + '_ {
        self.span(target).iter().map(|(_, address)| *address)
    }
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

/// Tallest the references card grows before it scrolls inside itself.
const REFERENCES_HEIGHT: f32 = 180.0;

/// The instructions that name the selected string, and the way to each.
///
/// Returns an address the reader asked to have on the clipboard.
fn reference_card(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    references: &CodeReferences,
    string: &ExtractedString,
    language: Language,
) -> Action {
    let mut action = Action::default();
    let mut jump = None;

    let count = string_address(analysis, string)
        .map(|address| direct_references(analysis, references, address).len())
        .unwrap_or_default();
    let title = if count > 0 {
        format!("{} ({count})", text(language, Text::StringReferences))
    } else {
        text(language, Text::StringReferences).to_owned()
    };

    card(ui, &title, |ui| {
        ui.monospace(&string.value);
        let Some(address) = string_address(analysis, string) else {
            ui.small(text(language, Text::StringAddressUnavailable));
            return;
        };
        ui.small(format!("{address:#018x}"));

        let found = direct_references(analysis, references, address);
        if found.is_empty() {
            ui.small(text(language, Text::NoStringReferences));
            return;
        }
        // What the rows answer to is said once, above them: a listing whose
        // every line reacts to a click and a right-click, and says so nowhere,
        // is a listing nobody clicks.
        ui.small(egui::RichText::new(text(language, Text::ReferenceHelp)).color(MUTED));
        ui.add_space(4.0);

        // Bounded: a string reached from forty places filled the window with
        // this card and pushed the listing it belongs to off the bottom.
        egui::ScrollArea::vertical()
            .id_salt("string_references")
            .max_height(REFERENCES_HEIGHT)
            .show(ui, |ui| {
                for instruction in found {
                    let (row, button) = reference_row(ui, instruction, language);
                    if row.clicked() || button {
                        jump = Some(instruction.address);
                    }
                    // The right button carries the same jump and what a
                    // listing of addresses otherwise makes the reader retype
                    // by hand.
                    row.context_menu(|ui| {
                        if ui.button(text(language, Text::GoToDisassembly)).clicked() {
                            jump = Some(instruction.address);
                            ui.close_menu();
                        }
                        if ui.button(text(language, Text::CopyAddress)).clicked() {
                            action.copy = Some(format!("{:#018x}", instruction.address));
                            ui.close_menu();
                        }
                    });
                }
            });
    });

    if let Some(address) = jump {
        action.go_to = Some(address);
    }
    action
}

/// One reference: where it is, in which section, what it does, and the button
/// that opens it.
///
/// Three ways to the same place, on purpose. The button is the one a reader
/// sees without being told, the whole row answers a click because a line of a
/// listing that leads somewhere should, and the right button carries what a
/// menu can hold and a row cannot — the address, for the tool they will paste
/// it into next. Returns the row and whether the button was pressed.
fn reference_row(
    ui: &mut egui::Ui,
    instruction: &Instruction,
    language: Language,
) -> (egui::Response, bool) {
    let mut pressed = false;
    let row = ui
        .horizontal(|ui| {
            let address = ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("{:#018x}", instruction.address)).monospace(),
                )
                .selectable(false),
            );
            let section = ui.add(
                egui::Label::new(
                    egui::RichText::new(instruction.section.as_ref())
                        .small()
                        .color(MUTED),
                )
                .selectable(false),
            );
            let body = ui.add(
                egui::Label::new(egui::RichText::new(&instruction.text).monospace())
                    .selectable(false),
            );
            pressed = ui
                .small_button(text(language, Text::GoToDisassembly))
                .clicked();
            address.union(section).union(body)
        })
        .inner
        // Sensed on the union of the row's text rather than on one label, so a
        // click anywhere along the line answers — but not on the button, which
        // would then count twice.
        .interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);

    if row.hovered() {
        ui.painter().rect_stroke(
            row.rect.expand(2.0),
            2.0,
            egui::Stroke::new(0.5_f32, egui::Color32::from_rgb(241, 169, 75)),
            egui::StrokeKind::Outside,
        );
    }
    (
        row.on_hover_text(text(language, Text::GoToDisassembly)),
        pressed,
    )
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

/// The instructions naming an address, read out of the index rather than by
/// scanning the listing again.
///
/// Only direct and `%rip`-relative operands are in the index: an indirect
/// reference needs full instruction-semantic analysis, and is left out rather
/// than reported as a false positive.
fn direct_references<'a>(
    analysis: &'a Analysis,
    references: &CodeReferences,
    address: u64,
) -> Vec<&'a Instruction> {
    references
        .instructions(address)
        .filter_map(|at| analysis.instruction_at(at))
        .collect()
}

/// The addresses an instruction names outright.
///
/// Only an operand that *is* an address counts. A displacement written against
/// a base register — `0x474(%rsp)`, `[x1, #0x10]` — is an offset into whatever
/// that register happens to hold when the program runs, and reading it as an
/// address made a four-letter string in `.rodata` look as though thirty
/// instructions referred to it, none of which did. A bracket anywhere in an
/// operand is what marks one — the comma this splits on cuts `[x1, #0x474]` in
/// two, so both halves have to be recognised. The same goes for a jump through
/// a table, `*0x129f388`: the number is where the pointer lives, not where it
/// points.
fn instruction_addresses(instruction: &Instruction) -> Vec<u64> {
    if let Some(target) = rip_relative_target(instruction) {
        return vec![target];
    }

    let operands = instruction
        .text
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest);
    operands
        .split(',')
        .map(str::trim)
        .filter(|part| !part.contains(['(', ')', '[', ']', '*']))
        .flat_map(hexadecimals)
        .collect()
}

/// Every hexadecimal number in one operand, in either notation the formatters
/// use: `0x1000` and `1000h`.
fn hexadecimals(operand: &str) -> Vec<u64> {
    operand
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
            classes: Vec::new(),
            import_slots: Vec::new(),
            network: desdec_core::NetworkUse::default(),
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

    /// Each scope keeps what belongs to it, and the four together account for
    /// every string exactly once.
    ///
    /// The interesting one is `MappedUnreferenced`: loaded into memory, and yet
    /// nothing in the decoded code points at it. That is where a string reached
    /// by arithmetic, through a table, or built at run time shows up — and the
    /// old pair of "hide this, hide that" toggles could not express it at all.
    #[test]
    fn each_scope_keeps_what_belongs_to_it_and_the_four_account_for_everything() {
        let analysis = analysis_for_filters();
        let references = CodeReferences::of(&analysis);
        let shown = |scope| {
            matching(&analysis, &references, "", scope)
                .into_iter()
                .map(|item| item.string.value.clone())
                .collect::<Vec<_>>()
        };

        assert_eq!(
            shown(Scope::All),
            ["referenced", "unreferenced", "not mapped"],
            "every string, which is the absence of a question"
        );
        assert_eq!(
            shown(Scope::Used),
            ["referenced"],
            "loaded and pointed at: what the program uses"
        );
        assert_eq!(
            shown(Scope::MappedUnreferenced),
            ["unreferenced"],
            "loaded, and named by nothing the decoder read"
        );
        assert_eq!(
            shown(Scope::Unmapped),
            ["not mapped"],
            "in the file and never loaded"
        );

        // The three narrow scopes partition the whole, so nothing can fall
        // between them and nothing is counted twice.
        let mut parts: Vec<String> = [Scope::Used, Scope::MappedUnreferenced, Scope::Unmapped]
            .into_iter()
            .flat_map(shown)
            .collect();
        parts.sort();
        let mut whole = shown(Scope::All);
        whole.sort();
        assert_eq!(parts, whole);
    }

    /// The text filter and the criteria narrow together too.
    #[test]
    fn the_text_filter_applies_alongside_the_criteria() {
        let analysis = analysis_for_filters();
        let references = CodeReferences::of(&analysis);
        let shown: Vec<String> = matching(&analysis, &references, "referenc", Scope::Used)
            .into_iter()
            .map(|item| item.string.value.clone())
            .collect();

        assert_eq!(
            shown,
            ["referenced"],
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

    /// A number written against a base register is an offset into whatever
    /// that register holds at run time. Reading it as an address made a
    /// four-letter string look as though thirty instructions named it.
    #[test]
    fn a_displacement_against_a_register_is_not_a_reference() {
        let displacement = Instruction {
            address: 0x0040_1000,
            bytes: desdec_core::InstructionBytes::new(&[0x89, 0x84, 0x24]).expect("short"),
            text: "mov %eax,0x474(%rsp)".to_owned(),
            section: std::sync::Arc::from(".text"),
        };
        assert_eq!(instruction_addresses(&displacement), [] as [u64; 0]);

        let through_a_table = Instruction {
            address: 0x0040_1000,
            bytes: desdec_core::InstructionBytes::new(&[0xff, 0x25]).expect("short"),
            text: "jmpq *0x129f388".to_owned(),
            section: std::sync::Arc::from(".text"),
        };
        assert_eq!(instruction_addresses(&through_a_table), [] as [u64; 0]);

        let arm64 = Instruction {
            address: 0x0040_1000,
            bytes: desdec_core::InstructionBytes::new(&[0x00, 0x00, 0x40, 0xf9]).expect("short"),
            text: "ldr x0, [x1, #0x474]".to_owned(),
            section: std::sync::Arc::from(".text"),
        };
        assert_eq!(instruction_addresses(&arm64), [] as [u64; 0]);
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
