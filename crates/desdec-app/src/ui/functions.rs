//! Function symbols exposed by the loaded binary.

use desdec_core::{Analysis, Symbol};
use eframe::egui;

use crate::{
    i18n::{Language, Text, text},
    ui::MUTED,
};

pub fn show(ui: &mut egui::Ui, analysis: &Analysis, expert_mode: bool, language: Language) {
    if !expert_mode {
        ui.small(text(language, Text::FunctionSymbolsHelp));
        ui.add_space(8.0);
    }
    if analysis.symbols.is_empty() {
        ui.label(egui::RichText::new(text(language, Text::NoFunctionSymbols)).color(MUTED));
        return;
    }
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("function_symbols")
                .num_columns(4)
                .striped(true)
                .spacing([18.0, 6.0])
                .show(ui, |ui| {
                    for title in [Text::Name, Text::Address, Text::Size, Text::SymbolOrigin] {
                        ui.strong(text(language, title));
                    }
                    ui.end_row();
                    for symbol in &analysis.symbols {
                        row(ui, symbol, language);
                        ui.end_row();
                    }
                });
        });
}

fn row(ui: &mut egui::Ui, symbol: &Symbol, language: Language) {
    ui.monospace(&symbol.name);
    match symbol.address {
        Some(address) => ui.monospace(format!("{address:#018x}")),
        None => ui.label(egui::RichText::new("—").color(MUTED)),
    };
    ui.label(symbol.size.to_string());
    ui.label(text(
        language,
        if symbol.imported {
            Text::Imported
        } else {
            Text::Defined
        },
    ));
}
