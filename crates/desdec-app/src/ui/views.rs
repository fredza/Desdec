use desdec_core::Analysis;
use eframe::egui;

use crate::{
    app::{DesdecApp, WorkspaceView},
    i18n::{Language, Text, text},
    preferences::accent,
    ui::{ERROR, MUTED, card, columns, expert, format_size, segments, strings},
};

pub fn show_central_panel(app: &mut DesdecApp, ctx: &egui::Context) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading(app.t(app.active_view.text()));
            ui.separator();
            ui.label(egui::RichText::new(app.mode_label()).color(MUTED));
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
    let expert_mode = app.expert_mode;
    let view = app.active_view;

    // Borrowing the analysis and the filter separately keeps both available.
    let Some(analysis) = &app.analysis else {
        if view == WorkspaceView::Overview {
            welcome(app, ui);
        } else {
            ui.label(text(language, Text::OpenFirst));
        }
        return;
    };

    match view {
        WorkspaceView::Overview => overview(ui, analysis, expert_mode, language),
        WorkspaceView::Segments => segments::show(ui, analysis, expert_mode, language),
        WorkspaceView::Strings => {
            strings::show(ui, analysis, &mut app.strings_filter, expert_mode, language);
        }
        view => {
            if let Some(explanation) = view.planned_explanation() {
                planned_view(ui, view, explanation, expert_mode, language);
            }
        }
    }
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

fn overview(ui: &mut egui::Ui, analysis: &Analysis, expert_mode: bool, language: Language) {
    // `auto_shrink` off makes the panels span the window instead of hugging
    // their own content.
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            alerts(ui, analysis, language);
            if expert_mode {
                expert_layout(ui, analysis, language);
            } else {
                guided_layout(ui, analysis, language);
            }
        });
}

/// Guided mode keeps one column and explains the next step.
fn guided_layout(ui: &mut egui::Ui, analysis: &Analysis, language: Language) {
    file_card(ui, analysis, false, language);
    ui.add_space(12.0);
    findings_card(ui, analysis, language);
    ui.add_space(12.0);
    ui.small(text(language, Text::ExpertHint));

    ui.add_space(12.0);
    card(ui, text(language, Text::NextStep), |ui| {
        ui.label(text(language, Text::NextStepDetail));
    });
}

/// Expert mode uses the whole width: what the file *is* on the left, what it
/// *contains and depends on* on the right.
fn expert_layout(ui: &mut egui::Ui, analysis: &Analysis, language: Language) {
    columns(
        ui,
        |ui| {
            file_card(ui, analysis, true, language);
            ui.add_space(12.0);
            expert::hardening_card(ui, analysis.details.hardening, language);
        },
        |ui| {
            findings_card(ui, analysis, language);
            ui.add_space(12.0);
            expert::libraries_card(ui, analysis, language);
            ui.add_space(12.0);
            expert::mapping_card(ui, analysis, language);
        },
    );
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

/// What the file is. Expert mode adds the loader-level identity, so the two
/// modes never show two frames saying almost the same thing.
fn file_card(ui: &mut egui::Ui, analysis: &Analysis, expert_mode: bool, language: Language) {
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

                if expert_mode {
                    expert::identity_rows(ui, analysis, language);
                }
            });
        if expert_mode {
            expert::digest_row(ui, analysis, language);
        }
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
                    Some(value) => ui.label(format!("{value:.2} / 8.00")),
                    None => ui.label("—"),
                };
                ui.end_row();

                ui.strong(text(language, Text::AnalysedBytes));
                ui.label(format_size(analysis.analysed_bytes));
                ui.end_row();
            });
    });
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
fn planned_view(
    ui: &mut egui::Ui,
    view: WorkspaceView,
    explanation: Text,
    expert_mode: bool,
    language: Language,
) {
    let title = format!(
        "{} {}",
        text(language, view.text()),
        text(language, Text::ComingSoon)
    );
    card(ui, &title, |ui| {
        ui.label(text(language, explanation));
        if !expert_mode {
            ui.add_space(8.0);
            ui.small(text(language, Text::GuidedHelp));
        }
    });
}
