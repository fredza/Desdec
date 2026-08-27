//! Section table of the loaded binary.

use desdec_core::{Analysis, Section, entropy};
use eframe::egui;

use crate::{
    i18n::{Language, Text, text},
    ui::{ERROR, MUTED, format_size},
};

/// Draws the section table.
///
/// `focused` is the section the reader was sent here to look at — the overview
/// states the entry point and the section it lands in, and the name there
/// leads here. It is marked and scrolled to rather than merely present: a
/// table of forty rows that opens at the top has not answered the question
/// that was asked of it.
///
/// `bring_into_view` is taken down as soon as the scroll has been asked for.
/// The mark stays; the scroll happens once. Asking every frame would pin the
/// table to that row, and the reader could not scroll away from it.
pub fn show(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    language: Language,
    focused: Option<&str>,
    bring_into_view: &mut bool,
    accent: egui::Color32,
) {
    if analysis.sections.is_empty() {
        ui.label(text(language, Text::NoSections));
        return;
    }

    if analysis.truncated {
        ui.colored_label(ERROR, text(language, Text::TruncatedAnalysis));
        ui.add_space(8.0);
    }

    // Mapped size differs from stored size for zero-filled sections such as
    // `.bss`, so it is always shown.
    let columns: &[Text] = &[
        Text::Name,
        Text::Address,
        Text::Offset,
        Text::Size,
        Text::MappedSize,
        Text::Rights,
        Text::Entropy,
    ];

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
                        let marked = focused == Some(section.name.as_str());
                        let top = ui.cursor().top();
                        row(ui, section, language, marked, accent);
                        ui.end_row();
                        if marked && *bring_into_view {
                            *bring_into_view = false;
                            let row = egui::Rect::from_min_max(
                                egui::pos2(ui.max_rect().left(), top),
                                egui::pos2(ui.max_rect().right(), ui.cursor().top()),
                            );
                            ui.scroll_to_rect_animation(
                                row,
                                Some(egui::Align::Center),
                                egui::style::ScrollAnimation::none(),
                            );
                        }
                    }
                });
        });
}

fn row(
    ui: &mut egui::Ui,
    section: &Section,
    language: Language,
    marked: bool,
    accent: egui::Color32,
) {
    // The mark is on the name alone. Painting the whole row would fight the
    // striping the table already uses, and the name is what the reader was
    // sent here to find.
    if marked {
        ui.label(
            egui::RichText::new(&section.name)
                .monospace()
                .strong()
                .color(accent),
        );
    } else {
        ui.monospace(&section.name);
    }

    if section.is_mapped() {
        ui.monospace(format!("{:#018x}", section.virtual_address));
    } else {
        ui.label(egui::RichText::new(text(language, Text::NotMapped)).color(MUTED));
    }

    ui.monospace(format!("{:#x}", section.file_offset));
    ui.label(format_size(section.file_size));
    ui.label(format_size(section.virtual_size));
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
