//! Section table of the loaded binary.

use desdec_core::{Analysis, Section, entropy};
use eframe::egui;

use crate::{
    i18n::{Language, Text, text},
    ui::{ERROR, MUTED, format_size},
};

pub fn show(ui: &mut egui::Ui, analysis: &Analysis, expert_mode: bool, language: Language) {
    if analysis.sections.is_empty() {
        ui.label(text(language, Text::NoSections));
        return;
    }

    if !expert_mode {
        ui.small(text(language, Text::SegmentsHelp));
        ui.add_space(8.0);
    }
    if analysis.truncated {
        ui.colored_label(ERROR, text(language, Text::TruncatedAnalysis));
        ui.add_space(8.0);
    }

    // Expert mode adds the mapped size, which differs from the stored size for
    // zero-filled sections such as `.bss`.
    let columns: &[Text] = if expert_mode {
        &[
            Text::Name,
            Text::Address,
            Text::Offset,
            Text::Size,
            Text::MappedSize,
            Text::Rights,
            Text::Entropy,
        ]
    } else {
        &[
            Text::Name,
            Text::Address,
            Text::Offset,
            Text::Size,
            Text::Rights,
            Text::Entropy,
        ]
    };

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("sections")
                .num_columns(columns.len())
                .striped(true)
                .spacing([18.0, 6.0])
                .show(ui, |ui| {
                    for header in columns {
                        ui.strong(text(language, *header));
                    }
                    ui.end_row();

                    for section in &analysis.sections {
                        row(ui, section, expert_mode, language);
                        ui.end_row();
                    }
                });
        });
}

fn row(ui: &mut egui::Ui, section: &Section, expert_mode: bool, language: Language) {
    ui.monospace(&section.name);

    if section.is_mapped() {
        ui.monospace(format!("{:#018x}", section.virtual_address));
    } else {
        ui.label(egui::RichText::new(text(language, Text::NotMapped)).color(MUTED));
    }

    ui.monospace(format!("{:#x}", section.file_offset));
    ui.label(format_size(section.file_size));
    if expert_mode {
        ui.label(format_size(section.virtual_size));
    }
    ui.monospace(section.permissions.label());

    match section.entropy {
        // A dense executable section is the one combination worth pointing out.
        Some(value) if section.permissions.execute && entropy::suggests_packing(value) => {
            ui.colored_label(ERROR, format!("{value:.2}"))
                .on_hover_text(text(language, Text::DenseCodeHint));
        }
        Some(value) => {
            ui.label(format!("{value:.2}"));
        }
        None => {
            ui.label(egui::RichText::new("—").color(MUTED));
        }
    }
}
