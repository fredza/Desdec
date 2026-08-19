//! The condition flags of the selected instruction, in the toolbar.
//!
//! A conditional jump is the one place a listing stops reading top to bottom,
//! and what decides it is a flag settled by some instruction above. That
//! pairing is invisible in a column of hexadecimal, so it is drawn where the
//! reader's eye already goes for the state of things — the bar at the top —
//! and it follows the selection in the listing, row by row.
//!
//! Nothing here is a value. Desdec never runs the binary, so a row of flags
//! showing `ZF = 1` would be a fabrication; what is shown is what the bytes
//! really do state — which flags this instruction settles, which of those it
//! settles to a value known whatever ran before, which it leaves meaningless,
//! and which it consults. [`desdec_core::flags`] draws that line; this module
//! only colours it.

use desdec_core::flags::{self, Flag, Outcome};
use eframe::egui;

use crate::{
    app::DesdecApp,
    i18n::{Language, Text, text},
    preferences::accent,
    ui::MUTED,
};

/// One flag's box. Wide enough for the widest label and its mark — `ZF?` in
/// monospace — and no wider: seven of these share a bar with everything else.
const CHIP: egui::Vec2 = egui::vec2(30.0, 22.0);

/// Room the whole row needs before it is worth drawing at all: its own width,
/// and the right-aligned actions that come after it. Below that the bar is
/// already full, and the flags would push the palette off the end.
pub const NEEDED_WIDTH: f32 = 380.0;

/// A flag left at a value the bytes state whatever ran before: the one thing
/// here a static reading can be certain of, so it is the one thing coloured
/// like an answer.
const KNOWN: egui::Color32 = egui::Color32::from_rgb(91, 201, 139);

/// Touched and left meaningless. Not an error, but not something to read
/// either, and the reader must not take it for a value.
const MEANINGLESS: egui::Color32 = egui::Color32::from_rgb(232, 119, 91);

/// Draws the flags of the selected instruction, and follows a click back to
/// whatever last settled one.
pub fn show(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    let theme = accent(app.preferences.theme);
    let Some(analysis) = app.analysis.as_ref() else {
        return;
    };
    let architecture = analysis.summary.architecture;
    let Some(address) = app.selected_instruction else {
        return;
    };
    // The selection can name an address the listing no longer holds — a binary
    // closed and another opened — and a row of flags for an instruction that
    // is not there would describe nothing.
    let Some(instruction) = analysis.instruction_at(address) else {
        return;
    };
    let shown = Flag::of(architecture);
    if shown.is_empty() {
        ui.label(
            egui::RichText::new(text(language, Text::NoFlagsForArchitecture))
                .small()
                .color(MUTED),
        );
        return;
    }
    let effect = flags::effect(instruction, architecture);
    let mut go_to = None;

    ui.horizontal(|ui| {
        let title = ui.label(
            egui::RichText::new(text(language, Text::Flags))
                .small()
                .color(MUTED),
        );
        app.tooltip(title, text(language, Text::FlagsHelp));

        for flag in shown {
            let response = chip(ui, *flag, architecture, effect, theme);
            // What last settled the flag is a walk back through the listing,
            // so it is asked for only when the pointer is actually on the box.
            let writer = response
                .hovered()
                .then(|| flags::last_write(analysis, address, *flag, architecture))
                .flatten();
            if response.clicked() {
                go_to = writer.as_ref().map(|write| write.address);
            }
            let response = app.tooltip(
                response,
                &explanation(*flag, architecture, effect, writer.as_ref(), language),
            );
            if writer.is_some() {
                response.on_hover_cursor(egui::CursorIcon::PointingHand);
            }
        }
    });

    if let Some(target) = go_to {
        app.go_to_address(ui.ctx(), target);
    }
}

/// One flag's box: its name, a mark for what the instruction leaves in it, and
/// a ring when the instruction consults it.
fn chip(
    ui: &mut egui::Ui,
    flag: Flag,
    architecture: desdec_core::Architecture,
    effect: flags::Effect,
    accent: egui::Color32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(CHIP, egui::Sense::click());
    let outcome = effect.outcome(flag);
    let painter = ui.painter();
    let box_ = rect.shrink(1.0);

    painter.rect_filled(box_, 4.0, ui.visuals().faint_bg_color);
    // The ring says "this instruction asks this flag a question", which is the
    // one thing that makes a reader look upwards for the answer.
    if effect.reads(flag) {
        painter.rect_stroke(
            box_,
            4.0,
            egui::Stroke::new(1.4_f32, accent),
            egui::StrokeKind::Inside,
        );
    }
    painter.text(
        box_.center(),
        egui::Align2::CENTER_CENTER,
        format!("{}{}", flag.short_name(architecture), mark(outcome)),
        egui::FontId::monospace(11.0),
        colour(outcome, ui),
    );
    response
}

/// What this instruction leaves in the flag, in one character.
const fn mark(outcome: Outcome) -> char {
    match outcome {
        // A dot rather than a blank: an empty box reads as a box still being
        // filled in.
        Outcome::Untouched => '·',
        // The question mark is the honest one: something is written there, and
        // only a run would know what.
        Outcome::Written => '?',
        Outcome::Cleared => '0',
        Outcome::Set => '1',
        Outcome::Undefined => '×',
    }
}

fn colour(outcome: Outcome, ui: &egui::Ui) -> egui::Color32 {
    match outcome {
        Outcome::Untouched => MUTED,
        Outcome::Written => ui.visuals().text_color(),
        Outcome::Cleared | Outcome::Set => KNOWN,
        Outcome::Undefined => MEANINGLESS,
    }
}

/// The sentence behind a box: which flag it is, what this instruction does to
/// it, and — when one was found — the instruction that last settled it.
fn explanation(
    flag: Flag,
    architecture: desdec_core::Architecture,
    effect: flags::Effect,
    writer: Option<&flags::FlagWrite>,
    language: Language,
) -> String {
    let mut lines = vec![format!(
        "{} — {}",
        flag.short_name(architecture),
        text(language, name(flag))
    )];
    lines.push(text(language, outcome_text(effect.outcome(flag))).to_owned());
    if effect.reads(flag) {
        lines.push(text(language, Text::FlagRead).to_owned());
    }
    // Where the answer came from, for a flag this instruction did not settle
    // itself: that is the comparison the reader is looking for.
    if !effect.outcome(flag).touches() {
        match writer {
            Some(write) => {
                lines.push(format!(
                    "{} {:#x} · {}",
                    text(language, Text::FlagLastSetBy),
                    write.address,
                    write.text
                ));
                lines.push(text(language, Text::FlagGoToWriter).to_owned());
            }
            None => lines.push(text(language, Text::FlagNoRecentWrite).to_owned()),
        }
    }
    lines.join("\n")
}

const fn name(flag: Flag) -> Text {
    match flag {
        Flag::Carry => Text::FlagCarry,
        Flag::Parity => Text::FlagParity,
        Flag::Adjust => Text::FlagAdjust,
        Flag::Zero => Text::FlagZero,
        Flag::Sign => Text::FlagSign,
        Flag::Overflow => Text::FlagOverflow,
        Flag::Direction => Text::FlagDirection,
    }
}

const fn outcome_text(outcome: Outcome) -> Text {
    match outcome {
        Outcome::Untouched => Text::FlagUntouched,
        Outcome::Written => Text::FlagWritten,
        Outcome::Cleared => Text::FlagCleared,
        Outcome::Set => Text::FlagSet,
        Outcome::Undefined => Text::FlagUndefined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::WorkspaceView;

    /// With an instruction selected, the bar draws one box per flag the
    /// architecture has — and it draws them in every language.
    #[test]
    fn the_selected_instruction_puts_its_flags_in_the_bar() {
        let analysis = crate::testing::reference_analysis();
        let architecture = analysis.summary.architecture;
        let Some(instruction) = analysis.instructions.first() else {
            return;
        };
        let mut app = crate::testing::opened_app(WorkspaceView::Disassembly);
        app.selected_instruction = Some(instruction.address);

        let ctx = egui::Context::default();
        for language in crate::i18n::Language::ALL {
            app.preferences.language = *language;
            let output = ctx.run(crate::testing::window_input(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| show(&mut app, ui));
            });
            let drawn = crate::testing::drawn_text(&output.shapes);
            for flag in Flag::of(architecture) {
                assert!(
                    drawn.contains(flag.short_name(architecture)),
                    "{flag:?} missing in {language:?}"
                );
            }
        }
    }

    /// Nothing selected, nothing claimed: a row of flags with no instruction
    /// under it would describe an instruction the reader never chose.
    #[test]
    fn no_selection_draws_no_flags() {
        let mut app = crate::testing::opened_app(WorkspaceView::Disassembly);
        app.selected_instruction = None;

        let ctx = egui::Context::default();
        let output = ctx.run(crate::testing::window_input(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show(&mut app, ui));
        });

        assert!(
            crate::testing::drawn_text(&output.shapes).trim().is_empty(),
            "the flags must say nothing without a selected instruction"
        );
    }

    /// The whole point of the row: standing on a conditional jump, the flag it
    /// consults leads back to the comparison that settled it. Read on the
    /// host's own binary, where the pairing really occurs.
    #[test]
    fn a_jump_leads_back_to_the_comparison_above_it() {
        let analysis = crate::testing::reference_analysis();
        let architecture = analysis.summary.architecture;
        let Some(flag) = Flag::of(architecture).first().copied() else {
            return;
        };

        // A jump that consults a flag, with the instruction that settled it
        // directly above: the shape a compiler emits by the thousand.
        let found = analysis.instructions.windows(2).find(|pair| {
            let settles = flags::effect(&pair[0], architecture)
                .outcome(flag)
                .touches();
            settles && flags::effect(&pair[1], architecture).reads(flag)
        });
        let Some(pair) = found else {
            return;
        };

        let write = flags::last_write(analysis, pair[1].address, flag, architecture)
            .expect("the instruction just above settles it");
        assert_eq!(
            write.address, pair[0].address,
            "the nearest instruction that settles a flag is the one reported"
        );
    }

    /// The mark is what tells a value known from a value only a run would
    /// know, so every outcome must have one of its own.
    #[test]
    fn every_outcome_is_marked_differently() {
        let marks: Vec<char> = [
            Outcome::Untouched,
            Outcome::Written,
            Outcome::Cleared,
            Outcome::Set,
            Outcome::Undefined,
        ]
        .into_iter()
        .map(mark)
        .collect();
        let mut unique = marks.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), marks.len(), "two outcomes share a mark");
    }
}
