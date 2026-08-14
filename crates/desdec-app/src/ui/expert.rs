//! Panels shown only in expert mode.
//!
//! Guided mode answers "what is this file?"; expert mode answers the questions
//! that come next — how it is mapped, what it depends on, and what the compiler
//! and linker did to make it harder to exploit.
//!
//! The pieces are exposed one by one rather than as a single block, so
//! [`crate::ui::views`] can place them across its two-column layout.

use desdec_core::{Analysis, Hardening, Relro, hash};
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

pub fn libraries_card(ui: &mut egui::Ui, analysis: &Analysis, language: Language) {
    card(ui, text(language, Text::LinkedLibraries), |ui| {
        if analysis.details.linked_libraries.is_empty() {
            ui.label(text(language, Text::NoLinkedLibraries));
            return;
        }
        for library in &analysis.details.linked_libraries {
            ui.monospace(library);
        }
    });
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
