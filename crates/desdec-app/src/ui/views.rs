use desdec_core::{Analysis, entropy};
use eframe::egui;

use crate::{
    app::{DesdecApp, WorkspaceView},
    i18n::{Language, Text, text},
    preferences::accent,
    ui::{
        ERROR, MUTED, card, columns, decompile, disassembly, expert, format_size, functions,
        patches_view, segments, strings, yara,
    },
};

pub fn show_central_panel(app: &mut DesdecApp, ctx: &egui::Context) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading(app.t(app.active_view.text()));
            ui.separator();
        });
        ui.add_space(12.0);
        content(app, ui);
        if let Some(error) = &app.error {
            ui.add_space(16.0);
            ui.colored_label(ERROR, error);
        }
    });
}

fn content(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    let view = app.active_view;

    if app.analysis.is_none() {
        if app.is_analysing() {
            loading(app, ui);
        } else if view == WorkspaceView::Overview {
            welcome(app, ui);
        } else {
            ui.label(text(language, Text::OpenFirst));
        }
        return;
    }

    // These views hold the whole application, since they act on it: patches
    // are exported, and the decompiler is started from the view showing it.
    match view {
        WorkspaceView::Patches => {
            patches_view::show(app, ui);
            return;
        }
        WorkspaceView::Decompile => {
            decompile::show(app, ui);
            return;
        }
        WorkspaceView::Yara => {
            yara::show(app, ui);
            return;
        }
        _ => {}
    }

    // Borrowing the analysis and the filter separately keeps both available.
    let Some(analysis) = &app.analysis else {
        return;
    };

    match view {
        WorkspaceView::Overview => {
            let explain = app.preferences.explain_libraries;
            if let Some(library) = overview(ui, analysis, language, explain) {
                app.explaining_library = Some(library);
                app.dialogs.library = true;
            }
        }
        WorkspaceView::Segments => segments::show(ui, analysis, language),
        WorkspaceView::Strings => {
            strings::show(
                ui,
                analysis,
                &mut app.strings_filter,
                &mut app.strings_unmapped_only,
                &mut app.strings_unreferenced_only,
                &mut app.selected_string,
                &mut app.selected_instruction,
                &mut app.pending_instruction_scroll,
                &mut app.instruction_attention,
                &mut app.active_view,
                language,
            );
        }
        WorkspaceView::Functions => {
            functions::show(ui, analysis, &mut app.selected_function, language)
        }
        WorkspaceView::Disassembly => {
            let action = disassembly::show(
                ui,
                analysis,
                &mut app.selected_instruction,
                &mut app.pending_instruction_scroll,
                &mut app.instruction_attention,
                &app.patches,
                language,
            );
            if let Some(address) = action.inspect {
                app.inspecting_operand = Some(address);
                app.dialogs.operand = true;
            }
            if let Some(address) = action.edit {
                // Editing happens where the patches live, so the pending list
                // and the export are in front of the user straight away.
                if patches_view::open_editor(app, address) {
                    app.active_view = WorkspaceView::Patches;
                }
            }
        }
        view => {
            if let Some(explanation) = view.planned_explanation() {
                planned_view(ui, view, explanation, language);
            }
        }
    }
}

/// A deliberate, central loading state rather than a small status-bar hint:
/// opening a large binary is the moment the reader needs a clear way out.
fn loading(app: &mut DesdecApp, ui: &mut egui::Ui) {
    ui.add_space(88.0);
    ui.vertical_centered(|ui| {
        ui.spinner();
        ui.add_space(12.0);
        ui.heading(app.t(Text::StatusWorking));
        ui.add_space(14.0);
        if ui
            .add(egui::Button::new(app.t(Text::CancelAnalysis)).min_size(egui::vec2(180.0, 34.0)))
            .clicked()
        {
            app.cancel_analysis();
        }
    });
}

fn welcome(app: &mut DesdecApp, ui: &mut egui::Ui) {
    ui.add_space(88.0);
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new("D")
                .color(accent(app.preferences.theme))
                .strong()
                .size(42.0),
        );
        ui.add_space(8.0);
        ui.heading(app.t(Text::StartAnalysis));
        ui.add_space(8.0);
        ui.label(app.t(Text::DropFile));
        ui.add_space(18.0);
        if ui
            .add(egui::Button::new(app.t(Text::OpenBinary)).min_size(egui::vec2(150.0, 32.0)))
            .clicked()
        {
            app.choose_binary(ui.ctx());
        }
        ui.add_space(28.0);
        ui.small(app.t(Text::MenuAvailable));
        ui.small(app.t(Text::LegalNotice));
    });
}

fn overview(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    language: Language,
    explain: bool,
) -> Option<String> {
    // `auto_shrink` off makes the panels span the window instead of hugging
    // their own content.
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            alerts(ui, analysis, language);
            expert_layout(ui, analysis, language, explain)
        })
        .inner
}

/// The detailed overview uses the whole width: what the file *is* on the left,
/// what it *contains and depends on* on the right.
fn expert_layout(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    language: Language,
    explain: bool,
) -> Option<String> {
    let mut asked = None;
    columns(
        ui,
        |ui| {
            file_card(ui, analysis, language);
            ui.add_space(12.0);
            expert::hardening_card(ui, analysis.details.hardening, language);
        },
        |ui| {
            findings_card(ui, analysis, language);
            ui.add_space(12.0);
            asked = expert::libraries_card(ui, analysis, language, explain);
            ui.add_space(12.0);
            expert::mapping_card(ui, analysis, language);
        },
    );
    asked
}

/// Warnings come first: they change how everything below should be read.
fn alerts(ui: &mut egui::Ui, analysis: &Analysis, language: Language) {
    let mut shown = false;
    if analysis.suggests_packing() {
        card(ui, text(language, Text::DenseCodeWarning), |ui| {
            ui.small(text(language, Text::DenseCodeHint));
        });
        shown = true;
    }
    if analysis.truncated {
        if shown {
            ui.add_space(8.0);
        }
        ui.colored_label(ERROR, text(language, Text::TruncatedAnalysis));
        shown = true;
    }
    if shown {
        ui.add_space(12.0);
    }
}

/// What the file is, including its loader-level identity.
fn file_card(ui: &mut egui::Ui, analysis: &Analysis, language: Language) {
    let summary = &analysis.summary;

    card(ui, text(language, Text::ActiveFile), |ui| {
        egui::Grid::new("binary_summary")
            .num_columns(2)
            .spacing([24.0, 10.0])
            .show(ui, |ui| {
                ui.strong(text(language, Text::Path));
                ui.monospace(summary.path.display().to_string());
                ui.end_row();

                ui.strong(text(language, Text::Format));
                ui.label(summary.format.label());
                ui.end_row();

                ui.strong(text(language, Text::Architecture));
                ui.label(summary.architecture.label());
                ui.end_row();

                ui.strong(text(language, Text::Size));
                ui.label(format_size(summary.size));
                ui.end_row();

                expert::identity_rows(ui, analysis, language);
            });
        expert::digest_row(ui, analysis, language);
    });
}

/// What the analysis found inside it.
fn findings_card(ui: &mut egui::Ui, analysis: &Analysis, language: Language) {
    card(ui, text(language, Text::Overview), |ui| {
        egui::Grid::new("analysis_summary")
            .num_columns(2)
            .spacing([24.0, 10.0])
            .show(ui, |ui| {
                ui.strong(text(language, Text::EntryPoint));
                entry_point(ui, analysis, language);
                ui.end_row();

                ui.strong(text(language, Text::SectionCount));
                ui.label(analysis.sections.len().to_string());
                ui.end_row();

                ui.strong(text(language, Text::StringCount));
                ui.label(analysis.strings.len().to_string());
                ui.end_row();

                ui.strong(text(language, Text::Entropy));
                match analysis.entropy {
                    Some(value) => entropy_bar(ui, value),
                    None => {
                        ui.label("—");
                    }
                };
                ui.end_row();

                ui.strong(text(language, Text::AnalysedBytes));
                ui.label(format_size(analysis.analysed_bytes));
                ui.end_row();
            });

        obfuscation_hint(ui, analysis, language);
    });
}

/// Entropy is an indicator, not proof: a dense executable section can be
/// packed, encrypted or obfuscated. When it also contains almost no readable
/// strings, encrypted or obfuscated strings become a second lead to inspect.
fn obfuscation_hint(ui: &mut egui::Ui, analysis: &Analysis, language: Language) {
    let code_may_be_obfuscated =
        code_may_be_obfuscated(analysis.entropy, analysis.suggests_packing());
    let strings_may_be_obfuscated = code_may_be_obfuscated && analysis.strings.len() <= 2;
    if !code_may_be_obfuscated {
        return;
    }

    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.strong(text(language, Text::Obfuscation));
        ui.colored_label(ERROR, text(language, Text::CodeMayBeObfuscated));
        if strings_may_be_obfuscated {
            ui.separator();
            ui.colored_label(ERROR, text(language, Text::StringsMayBeObfuscated));
        }
    });
}

/// The overview gauges the whole analysed binary, so its warning follows that
/// same scale. A dense executable section remains an independent signal even
/// when other low-entropy data brings the global value back below 7.
fn code_may_be_obfuscated(entropy: Option<f32>, dense_executable_section: bool) -> bool {
    const ENTROPY_THRESHOLD: f32 = 7.0;

    dense_executable_section || entropy.is_some_and(|value| value > ENTROPY_THRESHOLD)
}

/// Compact entropy gauge. The filled portion progresses from green through
/// orange to red, while the divisions retain the 0–8 bits-per-byte scale.
fn entropy_bar(ui: &mut egui::Ui, value: f32) {
    const WIDTH: f32 = 156.0;
    const HEIGHT: f32 = 14.0;
    /// Eight one-bit bands: each division represents one bit per byte.
    const STEPS: usize = 8;

    let ratio = (value / entropy::MAXIMUM).clamp(0.0, 1.0);
    ui.horizontal(|ui| {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(WIDTH, HEIGHT), egui::Sense::hover());
        let painter = ui.painter();
        let visuals = ui.style().visuals.clone();
        painter.rect_filled(rect, 3.0, visuals.extreme_bg_color);

        for step in 0..STEPS {
            let start = step as f32 / STEPS as f32;
            let end = (step + 1) as f32 / STEPS as f32;
            let filled_end = end.min(ratio);
            if filled_end <= start {
                continue;
            }
            let segment = egui::Rect::from_min_max(
                egui::pos2(rect.left() + rect.width() * start, rect.top()),
                egui::pos2(rect.left() + rect.width() * filled_end, rect.bottom()),
            );
            painter.rect_filled(segment, 0.0, entropy_color((start + end) / 2.0));
        }

        for graduation in 1..STEPS {
            let x = rect.left() + rect.width() * graduation as f32 / STEPS as f32;
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(1.0_f32, visuals.window_stroke.color),
            );
        }
        painter.rect_stroke(rect, 3.0, visuals.window_stroke, egui::StrokeKind::Inside);
        response.on_hover_text(format!("{value:.2} / {:.2}", entropy::MAXIMUM));
        ui.monospace(format!("{value:.2} / {:.2}", entropy::MAXIMUM));
    });
}

/// Maps the normalised entropy range to green → orange → red.
fn entropy_color(ratio: f32) -> egui::Color32 {
    let ratio = ratio.clamp(0.0, 1.0);
    let (from, to, local_ratio) = if ratio < 0.5 {
        (
            egui::Color32::from_rgb(77, 180, 110),
            egui::Color32::from_rgb(241, 169, 75),
            ratio * 2.0,
        )
    } else {
        (
            egui::Color32::from_rgb(241, 169, 75),
            egui::Color32::from_rgb(222, 86, 76),
            (ratio - 0.5) * 2.0,
        )
    };
    let mix = |start: u8, end: u8| start as f32 + (end as f32 - start as f32) * local_ratio;
    egui::Color32::from_rgb(
        mix(from.r(), to.r()) as u8,
        mix(from.g(), to.g()) as u8,
        mix(from.b(), to.b()) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::code_may_be_obfuscated;

    #[test]
    fn global_entropy_above_seven_marks_code_as_possibly_obfuscated() {
        assert!(code_may_be_obfuscated(Some(7.01), false));
        assert!(!code_may_be_obfuscated(Some(7.0), false));
    }

    #[test]
    fn dense_executable_section_remains_a_signal_below_global_threshold() {
        assert!(code_may_be_obfuscated(Some(6.8), true));
    }
}

/// The entry point, and the section it lands in when one can be found.
fn entry_point(ui: &mut egui::Ui, analysis: &Analysis, language: Language) {
    let Some(address) = analysis.entry_point else {
        ui.label("—");
        return;
    };

    ui.horizontal(|ui| {
        ui.monospace(format!("{address:#018x}"));
        if let Some(section) = analysis.section_at(address) {
            ui.label(
                egui::RichText::new(format!(
                    "{} {}",
                    text(language, Text::EntryPointIn),
                    section.name
                ))
                .color(MUTED),
            );
        }
    });
}

/// A view that is announced but not implemented yet.
fn planned_view(ui: &mut egui::Ui, view: WorkspaceView, explanation: Text, language: Language) {
    let title = format!(
        "{} {}",
        text(language, view.text()),
        text(language, Text::ComingSoon)
    );
    card(ui, &title, |ui| {
        ui.label(text(language, explanation));
    });
}
