//! The names the file declares: what it imports from another file, and what
//! it defines itself.
//!
//! Two facts the file states plainly are joined here. Its symbol table names
//! each function and says whether it is defined in the image or expected from
//! a library, and its relocations name the slot each imported address is
//! filled into. An import therefore carries the address of that slot — the
//! place a loader Desdec does not have would write the real address — while a
//! defined name carries its own address in the image. Neither is invented: an
//! import whose slot the file does not state, and a defined name with no
//! address, are shown without one rather than with a guessed value.

use std::collections::{HashMap, HashSet};

use desdec_core::Analysis;
use eframe::egui;

use crate::{
    i18n::{Language, Text, text},
    ui::{MUTED, ROW_HEIGHT},
};

/// Whether a name is expected from elsewhere or provided by the image.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Kind {
    /// Undefined here: the loader fills its slot before the program runs.
    Import,
    /// Defined in the image: its export, or a local name it kept.
    Defined,
}

/// One row: a declared name and the little the file states about it.
struct Entry<'a> {
    name: &'a str,
    kind: Kind,
    /// For an import, the slot the target is read from; for a defined name,
    /// its own address. `None` where the file does not state one.
    address: Option<u64>,
    size: u64,
}

/// What the view asks of the workspace, routed like every other navigation
/// through the one gateway that keeps the listing and its pseudo-code on the
/// same instruction.
#[derive(Default)]
pub struct Action {
    pub go_to: Option<u64>,
}

pub fn show(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    filter: &mut String,
    hide_imports: &mut bool,
    hide_defined: &mut bool,
    language: Language,
) -> Action {
    let mut action = Action::default();

    let entries = collect(analysis);
    if entries.is_empty() {
        ui.label(text(language, Text::NoSymbols));
        return action;
    }

    ui.label(egui::RichText::new(text(language, Text::SymbolsIntro)).color(MUTED));
    ui.add_space(8.0);

    let needle = filter.to_lowercase();
    let matches: Vec<&Entry> = entries
        .iter()
        .filter(|entry| {
            let kind_shown = match entry.kind {
                Kind::Import => !*hide_imports,
                Kind::Defined => !*hide_defined,
            };
            kind_shown && (needle.is_empty() || entry.name.to_lowercase().contains(&needle))
        })
        .collect();

    header(
        ui,
        filter,
        hide_imports,
        hide_defined,
        matches.len(),
        entries.len(),
        language,
    );
    ui.add_space(8.0);

    let row_spacing = ui.spacing().item_spacing.y;
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, matches.len(), |ui, range| {
            egui::Grid::new("symbols")
                .num_columns(4)
                .striped(true)
                .spacing([18.0, row_spacing])
                .min_row_height(ROW_HEIGHT)
                .show(ui, |ui| {
                    for entry in &matches[range] {
                        if let Some(address) = row(ui, entry, language) {
                            action.go_to = Some(address);
                        }
                        ui.end_row();
                    }
                });
        });
    action
}

/// Builds the unified list from the symbol table and the import slots.
///
/// The two are matched by name so an imported function appears once, carrying
/// the slot address its relocation named. An import slot the symbol table did
/// not also list — a data import, typically — is still shown, because it is as
/// unfilled as any other and the reader looking for it would not find it under
/// the functions alone.
fn collect(analysis: &Analysis) -> Vec<Entry<'_>> {
    // Where each imported name is filled in, kept for the join below.
    let mut slot_of: HashMap<&str, u64> = HashMap::new();
    for slot in &analysis.import_slots {
        slot_of.entry(slot.name.as_str()).or_insert(slot.address);
    }

    let mut entries = Vec::new();
    let mut named: HashSet<&str> = HashSet::new();
    for symbol in &analysis.symbols {
        named.insert(symbol.name.as_str());
        let (kind, address) = if symbol.imported {
            (Kind::Import, slot_of.get(symbol.name.as_str()).copied())
        } else {
            (Kind::Defined, symbol.address)
        };
        entries.push(Entry {
            name: &symbol.name,
            kind,
            address,
            size: symbol.size,
        });
    }
    // Import slots the symbol table did not name in its own right.
    for slot in &analysis.import_slots {
        if !named.contains(slot.name.as_str()) {
            entries.push(Entry {
                name: &slot.name,
                kind: Kind::Import,
                address: Some(slot.address),
                size: 0,
            });
        }
    }

    // Imports first, then by name, so the same file always answers in the same
    // order whatever order the tables were read in.
    entries.sort_by(|a, b| {
        kind_rank(a.kind)
            .cmp(&kind_rank(b.kind))
            .then_with(|| a.name.cmp(b.name))
    });
    entries.dedup_by(|a, b| a.name == b.name && a.kind == b.kind);
    entries
}

const fn kind_rank(kind: Kind) -> u8 {
    match kind {
        Kind::Import => 0,
        Kind::Defined => 1,
    }
}

fn header(
    ui: &mut egui::Ui,
    filter: &mut String,
    hide_imports: &mut bool,
    hide_defined: &mut bool,
    shown: usize,
    total: usize,
    language: Language,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(text(language, Text::FilterSymbols));
        ui.add(
            egui::TextEdit::singleline(filter)
                .hint_text(text(language, Text::FilterHint))
                .desired_width(240.0),
        );
        // The toggles read as "show this kind"; they are stored as their
        // opposite so the default, an untouched `false`, shows both.
        kind_toggle(ui, hide_imports, text(language, Text::SymbolsShowImports));
        kind_toggle(ui, hide_defined, text(language, Text::SymbolsShowDefined));

        let filtering = !filter.is_empty();
        if ui
            .add_enabled(filtering, egui::Button::new(text(language, Text::ClearFilter)))
            .clicked()
        {
            filter.clear();
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // How many were left out is part of the answer: a narrowed list
            // that does not say so looks like a file with fewer names in it.
            let counted = egui::RichText::new(format!(
                "{shown} {} {total}",
                text(language, Text::ShownOfTotal)
            ));
            ui.label(if filtering {
                counted.strong()
            } else {
                counted.color(MUTED)
            });
        });
    });
}

/// A checkbox that shows a kind, stored as the `hide` flag it is the opposite
/// of, so an untouched view shows everything.
fn kind_toggle(ui: &mut egui::Ui, hide: &mut bool, label: &str) {
    let mut shown = !*hide;
    if ui.checkbox(&mut shown, label).changed() {
        *hide = !shown;
    }
}

/// Draws one row and returns the address to navigate to if it was clicked.
fn row(ui: &mut egui::Ui, entry: &Entry<'_>, language: Language) -> Option<u64> {
    let mut go_to = None;

    // The name, clickable only where there is an address to go to.
    if let Some(address) = entry.address {
        let response = ui
            .add(
                egui::Label::new(egui::RichText::new(entry.name).monospace())
                    .sense(egui::Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            go_to = Some(address);
        }
    } else {
        ui.monospace(entry.name);
    }

    let kind_label = match entry.kind {
        Kind::Import => text(language, Text::SymbolsImported),
        Kind::Defined => text(language, Text::SymbolsDefined),
    };
    ui.label(egui::RichText::new(kind_label).color(MUTED));

    match entry.address {
        Some(address) => ui.monospace(format!("{address:#x}")),
        None => ui.label(egui::RichText::new("—").color(MUTED)),
    };

    if entry.size == 0 {
        ui.label(egui::RichText::new("—").color(MUTED));
    } else {
        ui.monospace(entry.size.to_string());
    }

    go_to
}
