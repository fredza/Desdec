use desdec_core::{Analysis, Confidence, NetworkUse, Protection, ProtectionKind, entropy};
use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog, WorkspaceView},
    i18n::{Language, Text, text},
    preferences::accent,
    ui::{
        ERROR, MUTED, assistant, card, classes, columns, decompile, disassembly, dump, expert,
        format_size, functions, graph, machine, monospace_value, patches_view, segments, strings,
        symbols, types, warning_sign, yara,
    },
};

/// Room between the workspace's edge and what is in it.
///
/// The panel used to start its content four points from the window's edge,
/// which is egui's default and is the measurement of a debug overlay: a
/// heading touching the frame, a table whose first column runs into the
/// navigation rail. A desktop window puts its content on a page, and a page
/// has a margin.
const WORKSPACE_MARGIN: egui::Margin = egui::Margin {
    left: 18,
    right: 18,
    top: 12,
    bottom: 8,
};

pub fn show_central_panel(app: &mut DesdecApp, ctx: &egui::Context) {
    let frame = egui::Frame::central_panel(&ctx.style()).inner_margin(WORKSPACE_MARGIN);
    egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
        view_header(app, ui);
        content(app, ui);
        if let Some(error) = &app.error {
            ui.add_space(16.0);
            ui.colored_label(ERROR, error);
        }
    });
}

/// The name of the view, the file it is reading, and a rule under both.
///
/// What was here before was the heading followed by `ui.separator()` inside a
/// horizontal row, which in egui draws a *vertical* bar: every view opened
/// with its name and a short stroke hanging off the end of it, saying nothing
/// and pointing at nothing. A rule belongs under a heading, across the width
/// of what it introduces.
fn view_header(app: &DesdecApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading(app.t(app.active_view.text()));
        // The open file at the other end of the row: every view here is a
        // reading of one binary, and which one is a question the reader should
        // never have to leave the view to answer.
        if let Some(analysis) = &app.analysis {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let path = analysis.summary.path.display().to_string();
                let name = analysis
                    .summary
                    .path
                    .file_name()
                    .map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned());
                ui.add(egui::Label::new(egui::RichText::new(name).color(MUTED)).truncate())
                    .on_hover_text(path);
            });
        }
    });
    ui.add_space(6.0);
    // A rule the width of the workspace, drawn a shade under the window's own
    // rim: loud enough to separate the heading from the content, quiet enough
    // not to be read as a border around something.
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        egui::Stroke::new(1.0_f32, ui.visuals().window_stroke.color),
    );
    ui.add_space(12.0);
}

/// The views that act on the whole application rather than read the analysis,
/// drawn here because each takes `&mut DesdecApp` and returns nothing to the
/// caller. Answers whether it drew one.
fn whole_application_view(app: &mut DesdecApp, ui: &mut egui::Ui) -> bool {
    let view = app.active_view;
    // These views hold the whole application, since they act on it: patches
    // are exported, and the decompiler is started from the view showing it.
    match view {
        WorkspaceView::Patches => {
            patches_view::show(app, ui);
            true
        }
        WorkspaceView::Decompile => {
            decompile::show(app, ui);
            true
        }
        WorkspaceView::Yara => {
            yara::show(app, ui);
            true
        }
        WorkspaceView::Dump => {
            dump::show(app, ui);
            true
        }
        WorkspaceView::Assistant => {
            assistant::show(app, ui);
            true
        }
        WorkspaceView::Machine => {
            machine::show(app, ui);
            true
        }
        // The structures view writes into the registry it draws from, and
        // moves the workspace when a pointer is followed to somewhere the
        // listing can show, so it takes the whole application too.
        WorkspaceView::Structures => {
            types::show(app, ui);
            true
        }
        // The graph reads the function index and the analysis together, and
        // moves the workspace when a block is clicked, so it takes the whole
        // application like the views above it.
        WorkspaceView::Graph => {
            graph::show_in(app, ui);
            true
        }
        WorkspaceView::Disassembly => {
            let action = disassembly::show(app, ui);
            if let Some(address) = action.inspect {
                app.inspecting_operand = Some(address);
                app.dialogs.open(Dialog::Operand);
            }
            if action.send_to_asm_studio {
                app.run_command(ui.ctx(), crate::commands::Command::SendToAsmStudio);
            }
            if let Some(address) = action.edit {
                // Editing happens where the patches live, so the pending list
                // and the export are in front of the user straight away.
                if patches_view::open_editor(app, address) {
                    app.active_view = WorkspaceView::Patches;
                }
            }
            true
        }
        _ => false,
    }
}

fn content(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    let view = app.active_view;

    if app.analysis.is_none() {
        if app.is_opening() {
            loading(app, ui);
        } else {
            // The welcome screen whatever the view, because it is the only one
            // here that offers a way out. A view selected before the binary
            // went away — or before an analysis was cancelled — used to draw a
            // bare "open a binary first" line, which is a dead end: no button,
            // and the way to open one is in a panel the reader may have
            // collapsed.
            welcome(app, ui);
        }
        return;
    }

    if whole_application_view(app, ui) {
        return;
    }

    // Read before the analysis is borrowed: these belong to the application,
    // and a view drawn from a borrow of the analysis cannot reach them.
    let app_focused_section = app.focused_section.clone();
    let mut bring_section_into_view = app.pending_section_scroll;
    let theme = app.preferences.theme;

    // Borrowing the analysis and the filter separately keeps both available.
    let Some(analysis) = &app.analysis else {
        return;
    };

    // An address a view asked the listing to show. Acted on after the borrow
    // of the analysis has ended, since moving the workspace takes the whole
    // application.
    let mut go_to = None;
    match view {
        WorkspaceView::Overview => {
            let explain = app.preferences.explain_libraries;
            let explain_mapping = app.preferences.explain_mapping;
            let asked = overview(ui, analysis, language, explain, explain_mapping);
            if let Some(question) = asked.library {
                app.explaining_library = Some(question.library);
                app.explaining_library_at = Some(question.asked_at);
                app.dialogs.open(Dialog::Library);
            }
            if let Some(wanted) = asked.explain_mapping {
                app.preferences.explain_mapping = wanted;
            }
            if let Some(name) = asked.section {
                app.focused_section = Some(name);
                bring_section_into_view = true;
                app.active_view = WorkspaceView::Segments;
            }
            go_to = asked.address;
        }
        WorkspaceView::Segments => segments::show(
            ui,
            analysis,
            language,
            app_focused_section.as_deref(),
            &mut bring_section_into_view,
            accent(theme),
        ),
        WorkspaceView::Strings => {
            let action = strings::show(
                ui,
                analysis,
                &app.string_references,
                &mut app.strings_filter,
                &mut app.strings_scopes,
                &mut app.strings_hide_prologues,
                &mut app.selected_string,
                language,
            );
            if let Some(value) = action.copy {
                app.copy_to_clipboard(ui.ctx(), &value, Text::AddressCopied);
            }
            go_to = action.go_to;
        }
        WorkspaceView::Functions => {
            go_to = functions::show(
                ui,
                analysis,
                &app.functions,
                &app.callgraph,
                &mut app.selected_function,
                &mut app.mangled_names,
                language,
            );
        }
        WorkspaceView::Symbols => {
            let action = symbols::show(
                ui,
                analysis,
                &mut app.symbols_filter,
                &mut app.symbols_hide_imports,
                &mut app.symbols_hide_defined,
                language,
            );
            go_to = action.go_to;
        }
        WorkspaceView::Classes => {
            let action = classes::show(ui, analysis, &mut app.classes_filter, language);
            go_to = action.go_to;
        }
        view => {
            if let Some(explanation) = view.planned_explanation() {
                planned_view(ui, view, explanation, language);
            }
        }
    }
    app.pending_section_scroll = bring_section_into_view;
    if let Some(address) = go_to {
        app.go_to_address(ui.ctx(), address);
    }
}

/// A deliberate, central loading state rather than a small status-bar hint:
/// opening a large binary is the moment the reader needs a clear way out.
fn loading(app: &mut DesdecApp, ui: &mut egui::Ui) {
    // The dialog waiting on a choice is part of the same opening as the
    // analysis of what was chosen. It used to show the welcome screen instead,
    // so a dialog that never came back left no way out at all.
    let analysing = app.is_analysing();
    ui.add_space(88.0);
    ui.vertical_centered(|ui| {
        ui.spinner();
        ui.add_space(12.0);
        ui.heading(app.t(if analysing {
            Text::StatusWorking
        } else {
            Text::StatusChoosing
        }));
        ui.add_space(14.0);
        let label = app.t(if analysing {
            Text::CancelAnalysis
        } else {
            Text::CancelChoosing
        });
        if ui
            .add(egui::Button::new(label).min_size(egui::vec2(180.0, 34.0)))
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
        // In red, and deliberately: it is the one line on this screen that is
        // about what the reader is allowed to do rather than about what the
        // program can do, and it reads as a footnote in the muted grey every
        // other small line uses.
        ui.small(egui::RichText::new(app.t(Text::LegalNotice)).color(ERROR));
    });
}

/// What the reader asked of the overview this frame.
///
/// Returned rather than acted on: every card here is drawn from a borrow of
/// the analysis, and moving the workspace to another view takes the whole
/// application.
#[derive(Default)]
pub struct Asked {
    /// A library whose explanation was asked for, and where the button sits.
    pub library: Option<expert::LibraryQuestion>,
    /// A section to show in the section table, by name.
    pub section: Option<String>,
    /// An address to show in the disassembly.
    pub address: Option<u64>,
    /// Whether the mapping card's explanations were switched on or off.
    pub explain_mapping: Option<bool>,
}

fn overview(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    language: Language,
    explain: bool,
    explain_mapping: bool,
) -> Asked {
    // `auto_shrink` off makes the panels span the window instead of hugging
    // their own content.
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            alerts(ui, analysis, language);
            expert_layout(ui, analysis, language, explain, explain_mapping)
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
    explain_mapping: bool,
) -> Asked {
    let mut asked = Asked::default();
    columns(
        ui,
        |ui| {
            file_card(ui, analysis, language);
            ui.add_space(12.0);
            expert::hardening_card(ui, analysis.details.hardening, language);
        },
        |ui| {
            let found = findings_card(ui, analysis, language);
            asked.section = found.section;
            asked.address = found.address;
            ui.add_space(12.0);
            asked.library = expert::libraries_card(ui, analysis, language, explain);
            ui.add_space(12.0);
            asked.explain_mapping = expert::mapping_card(ui, analysis, language, explain_mapping);
        },
    );
    asked
}

/// Warnings come first: they change how everything below should be read.
fn alerts(ui: &mut egui::Ui, analysis: &Analysis, language: Language) {
    let mut shown = false;
    // Ahead of everything, the network flag included: a packed file's listing
    // is its stub, so "this program can send" and "this program has forty
    // functions" are both statements about the wrapper until this is read.
    if protection_alert(ui, analysis, language) {
        shown = true;
    }
    // What a program can do to the outside is the next thing a reader wants
    // to know before reading anything else about it.
    if !analysis.network.is_silent() {
        if shown {
            ui.add_space(8.0);
        }
        network_alert(ui, &analysis.network, language);
        shown = true;
    }
    // Only when nothing above already said the file is packed: the dense-code
    // hint is the weakest form of the same statement, and two warnings about
    // one fact read as two faults.
    if analysis.suggests_packing() && analysis.protections.is_empty() {
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
    // A separate limit from the one above: the file may have been read whole
    // and the decoder still have stopped short of the end of the code.
    if analysis.code_truncated {
        if shown {
            ui.add_space(8.0);
        }
        ui.colored_label(ERROR, text(language, Text::TruncatedDisassembly));
        shown = true;
    }
    if shown {
        ui.add_space(12.0);
    }
}

/// The loudest thing the overview can say: what you are about to read is not
/// the program.
///
/// A packer replaces the program with a stub that unfolds it at run time, so
/// every view behind this one — the listing, the functions, the strings — is
/// describing the wrapper. That has to be said before any of them is read,
/// and it has to be said with its evidence, so the reader can check it in the
/// section table rather than take it on trust.
///
/// Returns whether anything was drawn.
fn protection_alert(ui: &mut egui::Ui, analysis: &Analysis, language: Language) -> bool {
    if analysis.protections.is_empty() {
        return false;
    }
    let named: Vec<&Protection> = analysis
        .protections
        .iter()
        .filter(|found| found.names_a_product())
        .collect();

    if !analysis.is_protected() {
        // Shapes alone. Drawn as a note rather than an alarm: a writable code
        // section is what a packer leaves behind and also what a just-in-time
        // compiler needs, and a red banner over the second one would be a
        // false accusation the reader cannot check.
        suspicion_card(ui, &analysis.protections, language);
        return true;
    }

    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.vertical(|ui| {
            // `horizontal`, never `horizontal_wrapped`: a wrapped row draws
            // every one of its parts at the same place in egui 0.31.
            ui.horizontal(|ui| {
                warning_sign(ui, ERROR);
                ui.label(
                    egui::RichText::new(text(language, Text::ProtectedBinary))
                        .color(ERROR)
                        .strong(),
                );
                for (position, found) in named.iter().enumerate() {
                    if position > 0 {
                        ui.label(egui::RichText::new("·").color(ERROR));
                    }
                    ui.label(egui::RichText::new(&found.name).color(ERROR).strong())
                        .on_hover_text(&found.evidence);
                    ui.label(
                        egui::RichText::new(format!("({})", kind_word(found.kind, language)))
                            .color(ERROR),
                    );
                }
            });
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(text(language, Text::ProtectionReadTheStub))
                        .small()
                        .color(MUTED),
                )
                .wrap(),
            );
            ui.add_space(6.0);
            evidence_row(
                ui,
                text(language, Text::ProtectionEvidence),
                &named
                    .iter()
                    .map(|found| found.evidence.as_str())
                    .collect::<Vec<_>>()
                    .join(" · "),
            );
        });
    });
    true
}

/// The shapes of a protected file, with nothing naming a product.
fn suspicion_card(ui: &mut egui::Ui, leads: &[Protection], language: Language) {
    card(ui, text(language, Text::ProtectionSuspected), |ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(text(language, Text::ProtectionSuspectedHint))
                    .small()
                    .color(MUTED),
            )
            .wrap(),
        );
        ui.add_space(4.0);
        for lead in leads {
            ui.horizontal(|ui| {
                // A lead that only *might* mean something is drawn quieter
                // than one that probably does. The distinction is the whole
                // reason these are not in the red banner above.
                let colour = if lead.confidence >= Confidence::Likely {
                    ERROR
                } else {
                    MUTED
                };
                ui.label(egui::RichText::new("—").color(colour));
                ui.add(
                    egui::Label::new(egui::RichText::new(&lead.evidence).small().color(colour))
                        .wrap(),
                );
            });
        }
    });
}

/// What a product does, in the reader's own language.
const fn kind_word(kind: ProtectionKind, language: Language) -> &'static str {
    text(
        language,
        match kind {
            ProtectionKind::Packer => Text::KindPacker,
            ProtectionKind::Protector => Text::KindProtector,
            ProtectionKind::Virtualiser => Text::KindVirtualiser,
            ProtectionKind::Obfuscator => Text::KindObfuscator,
            ProtectionKind::Bundler => Text::KindBundler,
            ProtectionKind::Unidentified => Text::KindUnidentified,
        },
    )
}

/// The red flag: this file can reach the network.
///
/// Loud on purpose, and honest about what it is: a statement read out of the
/// file, with the names it was read from listed underneath. A reader who can
/// see the evidence can judge it — and can tell the flag from a guess.
fn network_alert(ui: &mut egui::Ui, network: &NetworkUse, language: Language) {
    let says = match (network.sends(), network.receives()) {
        (true, true) => Text::NetworkSendsAndReceives,
        (true, false) => Text::NetworkSends,
        (false, true) => Text::NetworkReceives,
        // Sockets or a network library, with nothing naming a read or a write:
        // the road is there, and what goes down it is not stated.
        (false, false) => Text::NetworkOpens,
    };
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.vertical(|ui| {
            // `horizontal`, never `horizontal_wrapped`: a wrapped row draws
            // every one of its parts at the same place in egui 0.31.
            ui.horizontal(|ui| {
                warning_sign(ui, ERROR);
                ui.label(
                    egui::RichText::new(text(language, Text::NetworkAlert))
                        .color(ERROR)
                        .strong(),
                );
                ui.label(egui::RichText::new(text(language, says)).color(ERROR));
            });
            ui.add_space(6.0);
            ui.small(egui::RichText::new(text(language, Text::NetworkIsNotARun)).color(MUTED));
            if !network.names.is_empty() {
                ui.add_space(6.0);
                evidence_row(ui, text(language, Text::NetworkEvidence), &joined(network));
            }
            if !network.libraries.is_empty() {
                ui.add_space(4.0);
                evidence_row(
                    ui,
                    text(language, Text::NetworkLibraries),
                    &network.libraries.join(" · "),
                );
            }
        });
    });
}

/// The names an alert was read from, on one line that wraps rather than one
/// that grows: a file may name a hundred of them.
fn evidence_row(ui: &mut egui::Ui, title: &str, names: &str) {
    ui.horizontal(|ui| {
        ui.small(egui::RichText::new(title).color(MUTED));
    });
    ui.add(egui::Label::new(egui::RichText::new(names).monospace().small()).wrap());
}

/// The names an alert was read from, as many as a reader will actually read.
///
/// Shortest first, and each one cut: `send` and `getaddrinfo` say what they
/// are at a glance, while a mangled Rust symbol runs past eighty characters
/// and a row of those says nothing at all. The rest are counted rather than
/// listed — the whole of them is in the symbols view.
fn joined(network: &NetworkUse) -> String {
    const SHOWN: usize = 12;
    const LONGEST: usize = 44;

    let mut names: Vec<&str> = network
        .names
        .iter()
        .map(|found| found.name.as_str())
        .collect();
    names.sort_by_key(|name| (name.chars().count(), *name));
    let mut said: Vec<String> = names
        .iter()
        .take(SHOWN)
        .map(|name| match name.char_indices().nth(LONGEST) {
            Some((cut, _)) => format!("{}…", &name[..cut]),
            None => (*name).to_owned(),
        })
        .collect();
    let rest = names.len().saturating_sub(SHOWN);
    if rest > 0 {
        said.push(format!("+{rest}"));
    }
    said.join(" · ")
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
                // Truncated rather than laid out whole: a path is as long as
                // the reader's directories happen to be, and one laid out
                // plainly used to carry this card past its column.
                monospace_value(ui, &summary.path.display().to_string());
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
fn findings_card(ui: &mut egui::Ui, analysis: &Analysis, language: Language) -> Asked {
    let mut asked = Asked::default();
    card(ui, text(language, Text::Overview), |ui| {
        egui::Grid::new("analysis_summary")
            .num_columns(2)
            .spacing([24.0, 10.0])
            .show(ui, |ui| {
                ui.strong(text(language, Text::EntryPoint));
                let pressed = entry_point(ui, analysis, language);
                asked.section = pressed.section;
                asked.address = pressed.address;
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
    asked
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
    use desdec_core::{NetworkName, NetworkUse, Reach};
    use eframe::egui;

    use super::{ERROR, code_may_be_obfuscated};
    use crate::{
        i18n::{Language, Text, text},
        testing::{drawn, drawn_in_colour, window_input},
    };

    fn networked(names: &[(&str, Reach)], libraries: &[&str]) -> NetworkUse {
        NetworkUse {
            names: names
                .iter()
                .map(|(name, reach)| NetworkName {
                    name: String::from(*name),
                    reach: *reach,
                })
                .collect(),
            libraries: libraries.iter().map(|name| String::from(*name)).collect(),
        }
    }

    /// Draws the alert alone, and answers with every string it painted and the
    /// colour each was painted in.
    fn alert(network: &NetworkUse) -> Vec<(String, egui::Color32)> {
        let ctx = egui::Context::default();
        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| {
                super::network_alert(ui, network, Language::French);
            });
        };
        let _ = ctx.run(window_input(), &mut draw);
        let output = ctx.run(window_input(), &mut draw);
        drawn_in_colour(&output.shapes)
    }

    /// The flag has to be red to be a flag: painted like every other line it
    /// says the same words and none of the warning.
    #[test]
    fn a_file_that_reaches_the_network_is_flagged_in_red() {
        let painted = alert(&networked(
            &[("socket", Reach::Connect), ("send", Reach::Send)],
            &["libcurl.so.4"],
        ));
        let said: String = painted.iter().map(|(text, _)| text.as_str()).collect();
        assert!(
            said.contains(text(Language::French, Text::NetworkAlert)),
            "{said}"
        );
        assert!(
            said.contains("socket") && said.contains("libcurl.so.4"),
            "the evidence is shown, so the reader can judge the flag: {said}"
        );
        assert!(
            painted
                .iter()
                .filter(|(_, colour)| *colour == ERROR)
                .count()
                >= 2,
            "the mark and its sentence are both red: {painted:?}"
        );
    }

    /// What it says depends on what was found: a file that only opens a
    /// connection must not be reported as one that sends and receives.
    #[test]
    fn the_sentence_says_only_what_the_names_found_say() {
        let opens: String = alert(&networked(&[("socket", Reach::Connect)], &[]))
            .into_iter()
            .map(|(text, _)| text)
            .collect();
        assert!(
            opens.contains(text(Language::French, Text::NetworkOpens)),
            "{opens}"
        );

        let both: String = alert(&networked(&[("curl_easy_perform", Reach::Protocol)], &[]))
            .into_iter()
            .map(|(text, _)| text)
            .collect();
        assert!(
            both.contains(text(Language::French, Text::NetworkSendsAndReceives)),
            "{both}"
        );
    }

    /// The same fault the machine view carries a test for: a row that wraps
    /// paints all of its parts at one place in egui 0.31.
    #[test]
    fn nothing_in_the_alert_is_drawn_on_top_of_anything_else() {
        let ctx = egui::Context::default();
        let network = networked(&[("socket", Reach::Connect)], &["libcurl.so.4"]);
        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| {
                super::network_alert(ui, &network, Language::French);
            });
        };
        let _ = ctx.run(window_input(), &mut draw);
        let output = ctx.run(window_input(), &mut draw);
        let mut seen: Vec<egui::Pos2> = Vec::new();
        for (said, at) in drawn(&output.shapes) {
            assert!(
                !seen.contains(&at),
                "{said:?} is drawn on top of something else"
            );
            seen.push(at);
        }
    }

    /// A long name is cut rather than allowed to push the row past the card,
    /// and what does not fit is counted.
    #[test]
    fn the_evidence_is_short_names_first_and_the_rest_counted() {
        let long = "_RNvMNtNtCse_3std3net3tcp9TcpStream7connectCsabcdef_0123456789abcdef";
        let mut names = vec![("send", Reach::Send), (long, Reach::Connect)];
        for filler in [
            "recv",
            "socket",
            "connect",
            "bind",
            "listen",
            "accept",
            "sendto",
            "recvfrom",
            "sendmsg",
            "recvmsg",
            "getaddrinfo",
        ] {
            names.push((filler, Reach::Connect));
        }
        let said = super::joined(&networked(&names, &[]));
        assert!(
            said.starts_with("bind · recv · send"),
            "shortest first: {said}"
        );
        assert!(said.ends_with("+1"), "what is left over is counted: {said}");
        assert!(
            !said.contains(long),
            "the long one is not shown whole: {said}"
        );
    }

    /// A packed file's listing is its stub's, so the overview says so before
    /// it says anything else — and names what it read that from.
    #[test]
    fn a_packed_file_is_flagged_by_name_and_by_evidence() {
        use desdec_core::{Confidence, Protection, ProtectionKind};

        let mut analysis = crate::testing::reference_analysis().clone();
        analysis.protections = vec![Protection {
            name: "UPX".to_owned(),
            kind: ProtectionKind::Packer,
            confidence: Confidence::Certain,
            evidence: "section UPX1".to_owned(),
        }];

        let ctx = egui::Context::default();
        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| {
                super::protection_alert(ui, &analysis, Language::French);
            });
        };
        let _ = ctx.run(window_input(), &mut draw);
        let output = ctx.run(window_input(), &mut draw);
        let painted = drawn_in_colour(&output.shapes);
        let said: String = painted.iter().map(|(text, _)| text.as_str()).collect();

        assert!(
            said.contains(text(Language::French, Text::ProtectedBinary)),
            "{said}"
        );
        assert!(said.contains("UPX"), "the product is named: {said}");
        assert!(
            said.contains("section UPX1"),
            "the finding is shown so the reader can check it: {said}"
        );
        assert!(
            painted
                .iter()
                .filter(|(_, colour)| *colour == ERROR)
                .count()
                >= 2,
            "the mark and the product are both red: {painted:?}"
        );
    }

    /// A shape is not a product. Writable code is what a packer leaves behind
    /// and also what a just-in-time compiler needs, and a red banner over the
    /// second one is an accusation the reader cannot check.
    #[test]
    fn a_shape_without_a_product_is_a_note_rather_than_an_alarm() {
        use desdec_core::{Confidence, Protection, ProtectionKind};

        let mut analysis = crate::testing::reference_analysis().clone();
        analysis.protections = vec![Protection {
            name: String::new(),
            kind: ProtectionKind::Unidentified,
            confidence: Confidence::Possible,
            evidence: "section .text is both writable and executable".to_owned(),
        }];

        let ctx = egui::Context::default();
        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| {
                super::protection_alert(ui, &analysis, Language::French);
            });
        };
        let _ = ctx.run(window_input(), &mut draw);
        let output = ctx.run(window_input(), &mut draw);
        let said: String = drawn_in_colour(&output.shapes)
            .iter()
            .map(|(text, _)| text.as_str())
            .collect();

        assert!(
            said.contains(text(Language::French, Text::ProtectionSuspected)),
            "{said}"
        );
        assert!(
            !said.contains(text(Language::French, Text::ProtectedBinary)),
            "a shape must not be reported as a protected binary: {said}"
        );
    }

    /// A file nothing points at says nothing at all here — the alert is not
    /// drawn empty, and nothing is inferred from absence.
    #[test]
    fn a_file_with_no_sign_of_protection_draws_no_alert() {
        let mut analysis = crate::testing::reference_analysis().clone();
        analysis.protections.clear();

        let ctx = egui::Context::default();
        let mut drew = true;
        let _ = ctx.run(window_input(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                drew = super::protection_alert(ui, &analysis, Language::French);
            });
        });
        assert!(!drew);
    }

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
///
/// Both halves lead somewhere, because both are answers to "and where is
/// that?": the address opens the listing at the first instruction the program
/// runs, and the section name opens the section table with that row marked.
/// Stating a place and leaving the reader to go and find it by hand is the
/// one thing an overview should not do.
fn entry_point(ui: &mut egui::Ui, analysis: &Analysis, language: Language) -> Asked {
    let mut asked = Asked::default();
    let Some(address) = analysis.entry_point else {
        ui.label("—");
        return asked;
    };

    ui.horizontal(|ui| {
        let decoded = analysis.instruction_index(address).is_some();
        let spelled = format!("{address:#018x}");
        if decoded {
            // A link, not a button: it is a value that also happens to lead
            // somewhere, and a button here would weigh more than the row.
            if ui
                .link(egui::RichText::new(&spelled).monospace())
                .on_hover_text(text(language, Text::GoToEntryPoint))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                asked.address = Some(address);
            }
        } else {
            // Not decoded — a stripped stub, or an address past the analysis
            // limit — so the offer is withheld rather than made and refused.
            ui.monospace(&spelled);
        }
        if let Some(section) = analysis.section_at(address) {
            ui.label(egui::RichText::new(text(language, Text::EntryPointIn)).color(MUTED));
            if ui
                .link(egui::RichText::new(&section.name).monospace())
                .on_hover_text(text(language, Text::GoToSection))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                asked.section = Some(section.name.clone());
            }
        }
    });
    asked
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
