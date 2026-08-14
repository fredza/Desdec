//! Printable strings extracted from the loaded binary.

use desdec_core::{Analysis, ExtractedString};
use eframe::egui;

use crate::{
    i18n::{Language, Text, text},
    ui::MUTED,
};

/// Height of one row, needed up front so the list can be virtualised: only the
/// visible rows are laid out, which keeps twenty thousand strings smooth.
const ROW_HEIGHT: f32 = 18.0;

pub fn show(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    filter: &mut String,
    expert_mode: bool,
    language: Language,
) {
    if analysis.strings.is_empty() {
        ui.label(text(language, Text::NoStrings));
        return;
    }

    if !expert_mode {
        ui.small(text(language, Text::StringsHelp));
        ui.add_space(8.0);
    }

    let matches = matching(analysis, filter);
    header(ui, filter, matches.len(), analysis.strings.len(), language);
    ui.add_space(8.0);

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, matches.len(), |ui, range| {
            egui::Grid::new("strings")
                .num_columns(3)
                .striped(true)
                .spacing([18.0, 4.0])
                .min_row_height(ROW_HEIGHT)
                .show(ui, |ui| {
                    for string in &matches[range] {
                        row(ui, string);
                        ui.end_row();
                    }
                });
        });
}

fn header(ui: &mut egui::Ui, filter: &mut String, shown: usize, total: usize, language: Language) {
    ui.horizontal(|ui| {
        ui.label(text(language, Text::FilterStrings));
        ui.add(
            egui::TextEdit::singleline(filter)
                .hint_text(text(language, Text::FilterHint))
                .desired_width(240.0),
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

fn matching<'a>(analysis: &'a Analysis, filter: &str) -> Vec<&'a ExtractedString> {
    if filter.is_empty() {
        return analysis.strings.iter().collect();
    }
    let needle = filter.to_lowercase();
    analysis
        .strings
        .iter()
        .filter(|string| string.value.to_lowercase().contains(&needle))
        .collect()
}

fn row(ui: &mut egui::Ui, string: &ExtractedString) {
    ui.monospace(format!("{:#010x}", string.file_offset));
    ui.label(egui::RichText::new(string.encoding.label()).color(MUTED));

    let value = if string.truncated {
        format!("{}…", string.value)
    } else {
        string.value.clone()
    };
    ui.monospace(value);
}
