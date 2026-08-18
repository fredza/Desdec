//! Detailed binary-analysis panels.
//!
//! They answer how the image is mapped, what it depends on, and what the
//! compiler and linker did to make it harder to exploit.
//!
//! The pieces are exposed one by one rather than as a single block, so
//! [`crate::ui::views`] can place them across its two-column layout.

use desdec_core::{Analysis, Confidence, Hardening, Relro, hash};
use eframe::egui;

use crate::{
    i18n::{Language, Text, text},
    ui::{ERROR, MUTED, card, format_size},
};

/// Extra identity rows, added to the file card already open in a `Grid`.
pub fn identity_rows(ui: &mut egui::Ui, analysis: &Analysis, language: Language) {
    let details = &analysis.details;

    ui.strong(text(language, Text::Type));
    ui.label(details.file_kind.label());
    ui.end_row();

    source_language_row(ui, analysis, language);

    if details.bits > 0 {
        ui.strong(text(language, Text::WordSize));
        ui.label(format!("{} bits", details.bits));
        ui.end_row();
    }

    ui.strong(text(language, Text::ByteOrder));
    ui.label(details.endianness.label());
    ui.end_row();

    if let Some(interpreter) = &details.interpreter {
        ui.strong(text(language, Text::Interpreter));
        ui.monospace(interpreter);
        ui.end_row();
    }

    if let Some(subsystem) = details.subsystem {
        ui.strong(text(language, Text::Subsystem));
        ui.label(subsystem);
        ui.end_row();
    }

    if let Some(timestamp) = details.timestamp {
        ui.strong(text(language, Text::BuildTimestamp));
        ui.label(format!("{timestamp}"))
            .on_hover_text(text(language, Text::TimestampHint));
        ui.end_row();
    }
}

/// The language the binary was built from, with what says so.
///
/// A compiled file states no such field, so this reports evidence rather than
/// a verdict: the finding names what was found, and how firmly it points. A
/// file that says nothing is reported as saying nothing — the alternative
/// would be to guess, and a guess dressed as a finding is worse than a blank.
fn source_language_row(ui: &mut egui::Ui, analysis: &Analysis, language: Language) {
    ui.strong(text(language, Text::SourceLanguage));
    let Some(headline) = analysis.languages.first() else {
        ui.label(
            egui::RichText::new(text(language, Text::LanguageUnknown))
                .color(MUTED)
                .italics(),
        )
        .on_hover_text(text(language, Text::LanguageUnknownHint));
        ui.end_row();
        return;
    };

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(headline.language.label()).strong());
        // How firmly the file points at that language, drawn rather than only
        // named: "possible" and "certain" are one word apart and three steps
        // apart, and a reader scanning the panel should see which it is
        // without stopping to read.
        confidence_bar(ui, headline.confidence, language, &headline.evidence);
        also_carried(ui, analysis, language);
    });
    ui.end_row();

    // The compiler's own version, when the file records it: that is the file's
    // word, and it dates the build more reliably than a timestamp.
    if let Some(toolchain) = analysis
        .languages
        .iter()
        .find_map(|found| found.toolchain.as_ref())
    {
        ui.strong(text(language, Text::Toolchain));
        ui.label(egui::RichText::new(toolchain).monospace());
        ui.end_row();
    }
}

/// A three-step gauge of how firmly the evidence points, with the word for it
/// and the evidence itself on hover.
///
/// The steps are the three the analysis actually distinguishes; drawing a
/// smooth percentage would suggest a measurement that was never made.
fn confidence_bar(ui: &mut egui::Ui, confidence: Confidence, language: Language, evidence: &str) {
    const WIDTH: f32 = 66.0;
    const HEIGHT: f32 = 10.0;
    const STEPS: usize = 3;

    let filled = match confidence {
        Confidence::Possible => 1,
        Confidence::Likely => 2,
        Confidence::Certain => 3,
    };
    let colour = match confidence {
        Confidence::Possible => egui::Color32::from_rgb(145, 155, 178),
        Confidence::Likely => egui::Color32::from_rgb(241, 169, 75),
        Confidence::Certain => egui::Color32::from_rgb(77, 180, 110),
    };

    let (rect, response) = ui.allocate_exact_size(egui::vec2(WIDTH, HEIGHT), egui::Sense::hover());
    let painter = ui.painter();
    let visuals = ui.style().visuals.clone();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    #[expect(
        clippy::cast_precision_loss,
        reason = "three steps and a width in pixels"
    )]
    let step = rect.width() / STEPS as f32;
    for index in 0..filled {
        #[expect(
            clippy::cast_precision_loss,
            reason = "three steps and a width in pixels"
        )]
        let left = rect.left() + step * index as f32;
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(left + 1.0, rect.top() + 1.0),
                egui::pos2(left + step - 1.0, rect.bottom() - 1.0),
            ),
            1.0,
            colour,
        );
    }
    painter.rect_stroke(rect, 2.0, visuals.window_stroke, egui::StrokeKind::Inside);

    let word = text(language, confidence_label(confidence));
    response.on_hover_text(format!(
        "{}: {word}\n{evidence}",
        text(language, Text::Certainty)
    ));
    ui.label(egui::RichText::new(word).small().color(MUTED))
        .on_hover_text(evidence);
}

/// The other languages the file shows traces of, said as what they are.
///
/// They used to be listed as `· C` after the verdict, which read as a second
/// answer competing with the first: "certain — Rust — C" is a contradiction to
/// anyone who has not been told that a Rust binary links the C runtime. The
/// lead-in word is what makes it one statement instead of two.
fn also_carried(ui: &mut egui::Ui, analysis: &Analysis, language: Language) {
    let others: Vec<&desdec_core::LanguageEvidence> = analysis.languages.iter().skip(1).collect();
    if others.is_empty() {
        return;
    }

    let names = others
        .iter()
        .map(|other| other.language.label())
        .collect::<Vec<_>>()
        .join(", ");
    let evidence = others
        .iter()
        .map(|other| format!("{}: {}", other.language.label(), other.evidence))
        .collect::<Vec<_>>()
        .join("\n");
    ui.label(
        egui::RichText::new(format!("— {} {names}", text(language, Text::AlsoTraces)))
            .small()
            .color(MUTED),
    )
    .on_hover_text(format!(
        "{}\n\n{evidence}",
        text(language, Text::AlsoTracesHint)
    ));
}

const fn confidence_label(confidence: Confidence) -> Text {
    match confidence {
        Confidence::Certain => Text::EvidenceCertain,
        Confidence::Likely => Text::EvidenceLikely,
        Confidence::Possible => Text::EvidencePossible,
    }
}

/// The digest sits below the grid: sixty-four characters would stretch a
/// two-column grid far past everything else in it.
pub fn digest_row(ui: &mut egui::Ui, analysis: &Analysis, language: Language) {
    ui.add_space(8.0);
    ui.strong(text(language, Text::Digest));
    match analysis.sha256 {
        Some(digest) => {
            ui.monospace(hash::to_hex(&digest));
        }
        None => {
            ui.label(egui::RichText::new(text(language, Text::DigestWithheld)).color(MUTED));
        }
    }
}

/// A hardening answer, and whether it needs a caveat next to it.
struct Mitigation {
    label: Text,
    state: State,
    hint: Option<Text>,
}

enum State {
    Enabled(String),
    Disabled(String),
    /// The format does not express this notion, or the answer was unreadable.
    Unknown,
}

pub fn hardening_card(ui: &mut egui::Ui, hardening: Hardening, language: Language) {
    let yes = text(language, Text::Present).to_owned();
    let no = text(language, Text::Absent).to_owned();
    let state = |value: Option<bool>| match value {
        Some(true) => State::Enabled(yes.clone()),
        Some(false) => State::Disabled(no.clone()),
        None => State::Unknown,
    };

    let mitigations = [
        Mitigation {
            label: Text::PositionIndependent,
            state: state(hardening.position_independent),
            hint: None,
        },
        Mitigation {
            label: Text::NonExecutableStack,
            state: state(hardening.non_executable_stack),
            hint: None,
        },
        Mitigation {
            label: Text::RelroLabel,
            // RELRO is not a yes/no: partial protection still leaves the PLT
            // writable, so the degree is what matters.
            state: match hardening.relro {
                Some(Relro::Full) => State::Enabled(Relro::Full.label().to_owned()),
                Some(degree @ (Relro::Partial | Relro::None)) => {
                    State::Disabled(degree.label().to_owned())
                }
                None => State::Unknown,
            },
            hint: None,
        },
        Mitigation {
            label: Text::StackCanary,
            state: state(hardening.stack_canary),
            hint: Some(Text::StackCanaryHint),
        },
        Mitigation {
            label: Text::AddressRandomisation,
            state: state(hardening.address_space_randomisation),
            hint: None,
        },
        Mitigation {
            label: Text::DataExecutionPrevention,
            state: state(hardening.data_execution_prevention),
            hint: None,
        },
        Mitigation {
            label: Text::ControlFlowGuard,
            state: state(hardening.control_flow_guard),
            hint: None,
        },
        Mitigation {
            label: Text::SignedImage,
            state: state(hardening.signed),
            hint: Some(Text::SignatureHint),
        },
    ];

    card(ui, text(language, Text::Hardening), |ui| {
        egui::Grid::new("expert_hardening")
            .num_columns(2)
            .spacing([24.0, 8.0])
            .show(ui, |ui| {
                for mitigation in &mitigations {
                    mitigation_row(ui, mitigation, language);
                    ui.end_row();
                }
            });
    });
}

fn mitigation_row(ui: &mut egui::Ui, mitigation: &Mitigation, language: Language) {
    let label = ui.strong(text(language, mitigation.label));
    if let Some(hint) = mitigation.hint {
        label.on_hover_text(text(language, hint));
    }

    match &mitigation.state {
        State::Enabled(value) => {
            ui.label(value);
        }
        State::Disabled(value) => {
            ui.colored_label(ERROR, value);
        }
        // An unknown is not a missing mitigation, and must not read like one.
        State::Unknown => {
            ui.label(
                egui::RichText::new(text(language, Text::NotApplicable))
                    .color(MUTED)
                    .italics(),
            );
        }
    }
}

/// The libraries a binary needs, each with a way to ask what it is for.
///
/// Returns the library whose explanation was asked for, so the caller opens
/// the window rather than this drawing code reaching into the application.
/// A library the reader asked about, and where the button they pressed sits,
/// so the explanation can be put where they were looking.
pub struct LibraryQuestion {
    pub library: String,
    pub asked_at: egui::Rect,
}

pub fn libraries_card(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    language: Language,
    explain: bool,
) -> Option<LibraryQuestion> {
    let mut asked = None;
    card(ui, text(language, Text::LinkedLibraries), |ui| {
        if analysis.details.linked_libraries.is_empty() {
            ui.label(text(language, Text::NoLinkedLibraries));
            return;
        }
        for (position, library) in analysis.details.linked_libraries.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.monospace(library);
                if !explain {
                    return;
                }
                let button = ui
                    .small_button("?")
                    .on_hover_text(text(language, Text::WhatIsThisFor));
                if button.clicked() {
                    asked = Some(LibraryQuestion {
                        library: library.clone(),
                        asked_at: button.rect,
                    });
                }
            });
            imported_functions(ui, analysis, library, position, language);
        }
    });
    asked
}

/// What the file asks one library for, when the format says.
///
/// Only PE records it: its import table names both the library and every
/// function taken from it. It answers the question linking against a library
/// leaves open — `ntdll.dll` most of all, where knowing the program reaches
/// past the usual layers says far less than knowing which kernel routines it
/// reaches for.
///
/// A file may name the same library in several import descriptors — installers
/// routinely do — so the entry is taken by its place in the list rather than by
/// name. Looking it up by name would show the first descriptor's functions
/// under every repeat, and give every repeat the same widget identifier, which
/// egui paints over the panel as a clash.
fn imported_functions(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    library: &str,
    position: usize,
    language: Language,
) {
    let imports = &analysis.details.imports;
    // PE builds both lists from the same descriptors, so the positions line
    // up; a format that names its libraries without recording what it takes
    // from them has only the name to go on.
    let Some(entry) = imports
        .get(position)
        .filter(|entry| entry.library == library)
        .or_else(|| imports.iter().find(|entry| entry.library == library))
    else {
        return;
    };

    let native = is_the_native_api(library);
    let title = format!(
        "{} ({})",
        text(language, Text::ImportedFunctions),
        entry.functions.len()
    );
    egui::CollapsingHeader::new(egui::RichText::new(title).small().color(MUTED))
        .id_salt(("imports", position))
        // The one library whose imports are the finding rather than a detail.
        .default_open(native)
        .show(ui, |ui| {
            if native {
                ui.small(text(language, Text::NativeApiNote));
                ui.add_space(4.0);
            }
            if entry.functions.is_empty() {
                ui.small(
                    egui::RichText::new(text(language, Text::NoImportedFunctions))
                        .color(MUTED)
                        .italics(),
                );
                return;
            }
            for function in &entry.functions {
                ui.monospace(function);
            }
            if entry.truncated {
                ui.small(egui::RichText::new(text(language, Text::ImportsTruncated)).color(ERROR));
            }
        });
}

/// Whether this is Windows' own lowest-level library, whatever case the file
/// spells it in.
fn is_the_native_api(library: &str) -> bool {
    library.eq_ignore_ascii_case("ntdll.dll") || library.eq_ignore_ascii_case("ntdll")
}

pub fn mapping_card(ui: &mut egui::Ui, analysis: &Analysis, language: Language) {
    card(ui, text(language, Text::LoadMapping), |ui| {
        ui.small(text(language, Text::LoadMappingHelp));
        ui.add_space(8.0);

        if analysis.details.segments.is_empty() {
            ui.label(text(language, Text::NoLoadMapping));
            return;
        }

        // Five columns in half a window: let the table scroll sideways rather
        // than push the card past the column it lives in.
        egui::ScrollArea::horizontal()
            .id_salt("expert_mapping_scroll")
            .show(ui, |ui| {
                egui::Grid::new("expert_mapping")
                    .num_columns(5)
                    .striped(true)
                    .spacing([18.0, 6.0])
                    .show(ui, |ui| {
                        for header in [
                            Text::Type,
                            Text::Address,
                            Text::Offset,
                            Text::Size,
                            Text::Rights,
                        ] {
                            ui.strong(text(language, header));
                        }
                        ui.end_row();

                        for segment in &analysis.details.segments {
                            ui.monospace(&segment.kind);
                            ui.monospace(format!("{:#018x}", segment.virtual_address));
                            ui.monospace(format!("{:#x}", segment.file_offset));
                            ui.label(format_size(segment.file_size));
                            ui.monospace(segment.permissions.label());
                            ui.end_row();
                        }
                    });
            });
    });
}

#[cfg(test)]
mod tests {
    use crate::{
        i18n::{Language, Text, text},
        testing::{drawn_text, samples, window_input},
    };
    use eframe::egui;

    /// A file may name one library in several import descriptors — installers
    /// routinely do. Salting each imports header with the library's name gave
    /// every repeat one identifier between them, and egui painted the clash
    /// across the card the reader was trying to read.
    #[test]
    fn a_library_named_twice_keeps_its_headers_apart() {
        for sample in samples() {
            let mut analysis = sample.analysis.clone();
            let Some(repeat) = analysis.details.imports.first().cloned() else {
                continue; // A format that records no imports draws no headers.
            };
            analysis
                .details
                .linked_libraries
                .push(repeat.library.clone());
            analysis.details.imports.push(repeat);

            let ctx = egui::Context::default();
            // The warning is on in debug builds only; the test asks for it so
            // it holds however the suite is compiled.
            ctx.options_mut(|options| options.warn_on_id_clash = true);
            let output = ctx.run(window_input(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    super::libraries_card(ui, &analysis, Language::French, true);
                });
            });

            assert!(
                !drawn_text(&output.shapes).contains("use of widget ID"),
                "{}: the repeated library reused the first one's widget id",
                sample.fixture.label
            );
        }
    }

    /// Each descriptor names what it alone takes from the library. Reaching
    /// for the entry by name gives every repeat the first descriptor's list,
    /// so a library asked for fifty functions in one descriptor and four in
    /// another is reported as asking for the same fifty twice.
    #[test]
    fn a_library_named_twice_shows_each_descriptor_s_own_functions() {
        for sample in samples() {
            let mut analysis = sample.analysis.clone();
            let Some(mut repeat) = analysis.details.imports.first().cloned() else {
                continue; // A format that records no imports draws no headers.
            };
            let first = repeat.functions.len();
            if first == 0 {
                continue; // Nothing to tell the two lists apart by.
            }
            repeat.functions.clear();
            analysis
                .details
                .linked_libraries
                .push(repeat.library.clone());
            analysis.details.imports.push(repeat);

            let ctx = egui::Context::default();
            let output = ctx.run(window_input(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    super::libraries_card(ui, &analysis, Language::French, true);
                });
            });

            let drawn = drawn_text(&output.shapes);
            let heading = text(Language::French, Text::ImportedFunctions);
            for count in [first, 0] {
                assert!(
                    drawn.contains(&format!("{heading} ({count})")),
                    "{}: the descriptor taking {count} functions was not reported as its own",
                    sample.fixture.label
                );
            }
        }
    }
}
