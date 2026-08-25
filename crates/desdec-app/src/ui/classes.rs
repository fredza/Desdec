//! C++ classes recovered from the names the file declares.
//!
//! A reading of the symbol table, not of the code: each class here was named
//! by a virtual table, a `type_info` or a member function the compiler
//! emitted. The view groups those and nothing more — it does not walk the
//! virtual table in memory or reconstruct a base-class graph, because neither
//! is stated by a name alone, and a hierarchy drawn from a guess would be
//! worse than the members shown plainly.

use desdec_core::{Analysis, Class, ClassSource};
use eframe::egui;

use crate::{
    i18n::{Language, Text, text},
    ui::{MUTED, ROW_HEIGHT},
};

/// What the view asks of the workspace: an address to bring into the listing.
#[derive(Default)]
pub struct Action {
    pub go_to: Option<u64>,
}

pub fn show(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    filter: &mut String,
    language: Language,
) -> Action {
    let mut action = Action::default();

    if analysis.classes.is_empty() {
        ui.label(text(language, Text::NoClasses));
        return action;
    }

    ui.label(egui::RichText::new(text(language, Text::ClassesIntro)).color(MUTED));
    ui.add_space(8.0);

    let needle = filter.to_lowercase();
    let matches: Vec<&Class> = analysis
        .classes
        .iter()
        .filter(|class| needle.is_empty() || class.name.to_lowercase().contains(&needle))
        .collect();

    header(ui, filter, matches.len(), analysis.classes.len(), language);
    ui.add_space(8.0);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for class in matches {
                if let Some(address) = class_block(ui, class, language) {
                    action.go_to = Some(address);
                }
            }
        });
    action
}

fn header(ui: &mut egui::Ui, filter: &mut String, shown: usize, total: usize, language: Language) {
    ui.horizontal_wrapped(|ui| {
        ui.label(text(language, Text::FilterClasses));
        ui.add(
            egui::TextEdit::singleline(filter)
                .hint_text(text(language, Text::FilterHint))
                .desired_width(240.0),
        );
        let filtering = !filter.is_empty();
        if ui
            .add_enabled(filtering, egui::Button::new(text(language, Text::ClearFilter)))
            .clicked()
        {
            filter.clear();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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

/// One class, as a collapsing header over its members. Returns an address to
/// navigate to if a member was clicked.
fn class_block(ui: &mut egui::Ui, class: &Class, language: Language) -> Option<u64> {
    let mut go_to = None;

    let title = format!(
        "{}   {} {}",
        class.name,
        class.methods.len(),
        text(language, Text::ClassMethodsCount)
    );
    egui::CollapsingHeader::new(egui::RichText::new(title).strong())
        .id_salt(("class", &class.name, u8::from(class.source == ClassSource::Msvc)))
        .show(ui, |ui| {
            // What the file states about the class as a whole, above its
            // members: the addresses of the tables that named it.
            markers(ui, class, language);
            if class.methods.is_empty() {
                return;
            }
            ui.add_space(4.0);
            egui::Grid::new(("methods", &class.name))
                .num_columns(3)
                .striped(true)
                .spacing([18.0, ui.spacing().item_spacing.y])
                .min_row_height(ROW_HEIGHT)
                .show(ui, |ui| {
                    for method in &class.methods {
                        if method_row(ui, method) {
                            go_to = method.address;
                        }
                        ui.end_row();
                    }
                });
        });
    go_to
}

/// The vtable and `type_info` addresses, and the ABI the class came from.
fn markers(ui: &mut egui::Ui, class: &Class, language: Language) {
    ui.horizontal_wrapped(|ui| {
        let source = match class.source {
            ClassSource::Itanium => "Itanium",
            ClassSource::Msvc => "MSVC",
        };
        ui.label(egui::RichText::new(source).small().color(MUTED));
        if let Some(vtable) = class.vtable {
            ui.label(egui::RichText::new(text(language, Text::ClassVtable)).small().color(MUTED));
            ui.monospace(format!("{vtable:#x}"));
        }
        if let Some(typeinfo) = class.typeinfo {
            ui.label(
                egui::RichText::new(text(language, Text::ClassTypeInfo))
                    .small()
                    .color(MUTED),
            );
            ui.monospace(format!("{typeinfo:#x}"));
        }
    });
}

/// One member: its readable name (clickable where an address is known), then
/// its address, then the mangled symbol it came from.
fn method_row(ui: &mut egui::Ui, method: &desdec_core::ClassMethod) -> bool {
    let mut clicked = false;
    if method.address.is_some() {
        let response = ui
            .add(
                egui::Label::new(egui::RichText::new(&method.name).monospace())
                    .sense(egui::Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        clicked = response.clicked();
    } else {
        ui.monospace(&method.name);
    }

    match method.address {
        Some(address) => ui.monospace(format!("{address:#x}")),
        None => ui.label(egui::RichText::new("—").color(MUTED)),
    };

    // The name as the linker wrote it, kept beside the reading of it: a member
    // shown demangled is still checkable against its original.
    ui.label(egui::RichText::new(&method.mangled).small().color(MUTED));
    clicked
}
