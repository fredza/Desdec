//! Who names an address.
//!
//! The listing answers "what is here?" by itself; it never answers "who gets
//! here?" — the call that reaches a function may be a hundred thousand rows
//! away, and the pointer holding its address is not code at all. This window
//! answers that, for whichever address the reader is standing on.
//!
//! Two strengths of answer, kept apart. An instruction that computes an
//! address is arithmetic on decoded bytes and is exact. A data word holding a
//! value that lands in the image is a likely pointer — a function table, a
//! relocation — but it may be a number that merely looks like one, and it is
//! labelled as such rather than counted among the calls.

use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::{Text, text},
    ui::{MUTED, syntax},
    xrefs::Kind,
};

/// Size assumed the first time the window opens, before egui has measured it.
const ASSUMED_SIZE: egui::Vec2 = egui::vec2(560.0, 320.0);

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::References) {
        return;
    }
    let Some(address) = app.references_address else {
        app.dialogs.close(Dialog::References);
        return;
    };

    let id = egui::Id::new("desdec.references");
    let mut open = true;
    let mut window = egui::Window::new(format!("{} {address:#018x}", app.t(Text::ReferencesTo)))
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(ASSUMED_SIZE);
    window = crate::ui::centred(
        window,
        ctx,
        app.dialogs.opening_step(Dialog::References).is_some(),
    );
    let mut go_to = None;
    window.show(ctx, |ui| {
        go_to = contents(app, ui, address);
    });

    app.dialogs.set(Dialog::References, open);
    if let Some(address) = go_to {
        app.go_to_address(ctx, address);
    }
}

/// Returns the address the reader asked to be taken to.
fn contents(app: &DesdecApp, ui: &mut egui::Ui, address: u64) -> Option<u64> {
    let language = app.preferences.language;
    let count = app.xrefs.count(address);
    if count == 0 {
        ui.label(egui::RichText::new(text(language, Text::NoReferences)).color(MUTED));
        return None;
    }
    ui.horizontal(|ui| {
        ui.strong(format!("{count}"));
        ui.label(
            egui::RichText::new(text(language, Text::References))
                .small()
                .color(MUTED),
        );
    });
    ui.add_space(6.0);

    let mut go_to = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("references")
                .num_columns(3)
                .striped(true)
                .spacing([14.0, 4.0])
                .show(ui, |ui| {
                    for reference in app.xrefs.to(address) {
                        if row(app, ui, reference.from, reference.kind) {
                            go_to = Some(reference.from);
                        }
                        ui.end_row();
                    }
                });
        });
    ui.add_space(6.0);
    ui.small(egui::RichText::new(text(language, Text::ReferencesHelp)).color(MUTED));
    go_to
}

/// One reference: where it is, what it does, and the line it sits on.
fn row(app: &DesdecApp, ui: &mut egui::Ui, from: u64, kind: Kind) -> bool {
    let language = app.preferences.language;
    let decoded = app.is_decoded(from);
    let address = ui.add(
        egui::Label::new(syntax::dim(
            ui,
            &format!("{from:#018x}"),
            egui::Color32::TRANSPARENT,
        ))
        .sense(egui::Sense::click()),
    );
    ui.label(
        egui::RichText::new(text(
            language,
            match kind {
                Kind::Call => Text::ReferenceCall,
                Kind::Jump => Text::ReferenceJump,
                Kind::Reads => Text::ReferenceReads,
                Kind::Pointer => Text::ReferencePointer,
            },
        ))
        .small()
        .color(MUTED),
    );

    // The instruction the reference sits on, or — for a pointer, which is not
    // code — the section the word lives in.
    if let Some(instruction) = app
        .analysis
        .as_ref()
        .and_then(|analysis| analysis.instruction_at(from))
    {
        ui.label(syntax::assembly(
            ui,
            &instruction.text,
            egui::Color32::TRANSPARENT,
        ));
    } else {
        let section = app
            .analysis
            .as_ref()
            .and_then(|analysis| analysis.section_at(from))
            .map_or_else(String::new, |section| section.name.clone());
        ui.label(egui::RichText::new(section).monospace().color(MUTED));
    }

    if decoded {
        address
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    } else {
        address.on_hover_text(text(language, Text::NotInTheListing));
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::WorkspaceView, commands::Command, i18n::Language, testing::window_input};

    /// The question this window exists for: who calls this?
    #[test]
    fn the_window_lists_what_calls_the_selected_address() {
        let analysis = crate::testing::reference_analysis();
        let Some((target, from)) = analysis.instructions.iter().find_map(|instruction| {
            let mnemonic = instruction.text.split_whitespace().next()?;
            if !mnemonic.starts_with("call") && mnemonic != "bl" {
                return None;
            }
            let target = desdec_core::operand::target_address(instruction)?;
            analysis.section_at(target)?;
            Some((target, instruction.address))
        }) else {
            return; // Nothing on this host calls a fixed address.
        };

        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Disassembly);
        app.preferences.language = Language::English;
        app.selected_instruction = Some(target);
        app.run_command(&ctx, Command::References);

        // Two frames: a window placed by its own measured size is laid out on
        // the first and painted on the second.
        let _ = ctx.run(window_input(), |ctx| show(&mut app, ctx));
        let output = ctx.run(window_input(), |ctx| show(&mut app, ctx));

        assert!(
            crate::testing::drawn_text(&output.shapes).contains(&format!("{from:#018x}")),
            "the call must be listed among what names {target:#x}"
        );
    }

    /// An address nothing names says so, rather than showing an empty box.
    #[test]
    fn an_address_nobody_names_says_so() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Disassembly);
        app.preferences.language = Language::English;
        app.references_address = Some(0xffff_ffff_0000);
        app.dialogs.open(Dialog::References);

        let _ = ctx.run(window_input(), |ctx| show(&mut app, ctx));
        let output = ctx.run(window_input(), |ctx| show(&mut app, ctx));

        assert!(
            crate::testing::drawn_text(&output.shapes)
                .contains(text(Language::English, Text::NoReferences))
        );
    }
}
