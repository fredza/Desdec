//! Two binaries set beside each other.
//!
//! The question a reader arrives with when a fix ships is not "what is in this
//! file" but "what is different about it", and no other view answers it: the
//! image has moved by a few bytes, every address is new, and a hex comparison
//! reports that nothing survived. What the reader wanted was the three
//! functions somebody edited.
//!
//! The reading is [`crate::compare`]'s; this draws it. Two things it takes care
//! to say rather than smooth over, because both are the difference between a
//! finding and a guess:
//!
//! - **How each pair was arrived at.** A name both files carry and a body that
//!   matches byte for byte are facts. A shape that looks alike and a function
//!   reached from a pair already made are readings, and the column says which.
//! - **What was not measured.** A pair of bodies too long to align has no
//!   distance rather than a distance of nothing, and the two are drawn
//!   differently.

use eframe::egui;

use desdec_core::diff::Pairing;

use crate::{
    compare::{Report, Row, SectionRow, Standing, State},
    i18n::{Language, Text, text},
    ui::{ERROR, MUTED, ROW_HEIGHT, card, format_size},
};

/// What the reader asked of this view.
#[derive(Default)]
pub struct Action {
    /// Open the file dialog for the other binary.
    pub choose_other: bool,
    /// Forget the other file and its comparison.
    pub forget: bool,
    /// An address in the *open* file the reader asked to see in the listing.
    ///
    /// Only ever one of the open file's own addresses. A row that only the
    /// other file has names an address in a file that is not open, and taking
    /// the listing there would show the reader somebody else's bytes under
    /// that name — so those rows offer no such button at all.
    pub go_to: Option<u64>,
}

/// Draws the comparison.
pub fn show(ui: &mut egui::Ui, state: &mut State, reading: bool, language: Language) -> Action {
    let mut action = Action::default();

    ui.label(egui::RichText::new(text(language, Text::CompareIntro)).color(MUTED));
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        let choose = if state.other.is_some() {
            Text::CompareReplaceOther
        } else {
            Text::CompareChooseOther
        };
        if ui
            .add_enabled(!reading, egui::Button::new(text(language, choose)))
            .clicked()
        {
            action.choose_other = true;
        }
        if state.other.is_some() && ui.button(text(language, Text::CompareForget)).clicked() {
            action.forget = true;
        }
        if reading {
            ui.label(egui::RichText::new(text(language, Text::CompareReading)).color(MUTED));
        }
        if let Some(other) = &state.other {
            ui.label(
                egui::RichText::new(other.path.display().to_string())
                    .color(MUTED)
                    .monospace(),
            );
        }
    });

    if let Some(failure) = &state.error {
        ui.add_space(8.0);
        ui.colored_label(ERROR, failure);
    }

    let Some(report) = &state.report else {
        if state.error.is_none() && !reading {
            ui.add_space(12.0);
            ui.label(text(language, Text::CompareNoOtherFile));
        }
        return action;
    };

    ui.add_space(12.0);
    match report.same_file {
        Some(true) => {
            ui.label(text(language, Text::CompareSameFile));
        }
        Some(false) => {}
        None => {
            ui.colored_label(ERROR, text(language, Text::CompareDigestUnknown));
        }
    }
    if !report.any_difference() {
        ui.add_space(8.0);
        ui.label(text(language, Text::CompareNoDifference));
    }

    ui.add_space(8.0);
    tallies(ui, report, language);
    ui.add_space(12.0);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if let Some(address) = functions(ui, state, language) {
                action.go_to = Some(address);
            }
            if let Some(report) = state.report.as_ref() {
                ui.add_space(12.0);
                names(ui, report, language);
                ui.add_space(12.0);
                sections(ui, report, language);
            }
        });

    action
}

/// The four counts, which is what a reader reads first and often all they
/// need.
fn tallies(ui: &mut egui::Ui, report: &Report, language: Language) {
    // Not `horizontal_wrapped`: in this egui it draws every item at the same
    // position, so the four tallies came out one on top of another and read as
    // a smear. Four short words and four counts fit on a line anyway.
    ui.horizontal(|ui| {
        for (item, count, colour) in [
            (
                Text::CompareChanged,
                report.count(Standing::Changed),
                Some(ERROR),
            ),
            (
                Text::CompareOnlyTheirs,
                report.count(Standing::OnlyTheirs),
                None,
            ),
            (
                Text::CompareOnlyMine,
                report.count(Standing::OnlyMine),
                None,
            ),
            (
                Text::CompareIdentical,
                report.count(Standing::Identical),
                None,
            ),
        ] {
            let label = format!("{}\u{a0}: {count}", text(language, item));
            match colour {
                Some(colour) if count > 0 => ui.colored_label(colour, label),
                _ => ui.label(egui::RichText::new(label).color(MUTED)),
            };
            ui.add_space(14.0);
        }
    });
}

/// The function table, and the address a row asked the listing to go to.
fn functions(ui: &mut egui::Ui, state: &mut State, language: Language) -> Option<u64> {
    let mut go_to = None;
    let report = state.report.as_ref()?;

    let needle = state.filter.to_lowercase();
    let shown: Vec<&Row> = report
        .rows
        .iter()
        .filter(|row| !(state.hide_identical && row.standing == Standing::Identical))
        .filter(|row| {
            needle.is_empty()
                || [row.mine.as_ref(), row.theirs.as_ref()]
                    .into_iter()
                    .flatten()
                    .any(|(name, _)| name.to_lowercase().contains(&needle))
        })
        .collect();
    let identical = report.count(Standing::Identical);

    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.filter)
                .desired_width(220.0)
                .hint_text(text(language, Text::CompareFilterHint)),
        );
        ui.add_space(12.0);
        ui.checkbox(
            &mut state.hide_identical,
            text(language, Text::CompareHideIdentical),
        );
        if state.hide_identical && identical > 0 {
            ui.label(
                egui::RichText::new(format!(
                    "{identical} {}",
                    text(language, Text::CompareHiddenCount)
                ))
                .color(MUTED),
            );
        }
    });
    ui.add_space(8.0);

    if shown.is_empty() {
        ui.label(egui::RichText::new(text(language, Text::CompareNothingToShow)).color(MUTED));
        return go_to;
    }

    let row_spacing = ui.spacing().item_spacing.y;
    egui::Grid::new("compare-functions")
        .num_columns(5)
        .striped(true)
        .spacing([18.0, row_spacing])
        .min_row_height(ROW_HEIGHT)
        .show(ui, |ui| {
            for header in [
                Text::CompareMine,
                Text::CompareTheirs,
                Text::ComparePairing,
                Text::CompareDistance,
                Text::Address,
            ] {
                let label = ui.strong(text(language, header));
                match header {
                    Text::ComparePairing => {
                        label.on_hover_text(text(language, Text::ComparePairingHint))
                    }
                    Text::CompareDistance => {
                        label.on_hover_text(text(language, Text::CompareDistanceHint))
                    }
                    _ => label,
                };
            }
            ui.end_row();

            for row in shown {
                if let Some(address) = function_row(ui, row, language) {
                    go_to = Some(address);
                }
                ui.end_row();
            }
        });
    go_to
}

/// One function's row, and the address it asked the listing to go to.
fn function_row(ui: &mut egui::Ui, row: &Row, language: Language) -> Option<u64> {
    let mut go_to = None;

    side(ui, row.mine.as_ref(), row.standing, false, language);
    side(ui, row.theirs.as_ref(), row.standing, row.moved, language);

    match row.pairing {
        Some(pairing) => {
            let label = egui::RichText::new(text(language, pairing_text(pairing)));
            // A reading is drawn quieter than a fact, the way the Functions
            // view already draws a discovered address quieter than a named
            // one. The words say it too; the colour says it first.
            ui.label(if pairing.is_certain() {
                label
            } else {
                label.color(MUTED)
            })
            .on_hover_text(text(language, Text::ComparePairingHint));
        }
        None => {
            ui.label(egui::RichText::new("—").color(MUTED));
        }
    }

    match (row.standing, row.difference) {
        (Standing::Changed, Some(difference)) => {
            ui.monospace(format!("+{} −{}", difference.added, difference.removed))
                .on_hover_text(text(language, Text::CompareDistanceHint));
        }
        (Standing::Changed, None) => {
            ui.label(
                egui::RichText::new(text(language, Text::CompareDistanceUnmeasured)).color(MUTED),
            )
            .on_hover_text(text(language, Text::CompareDistanceUnmeasuredHint));
        }
        _ => {
            ui.label(egui::RichText::new("—").color(MUTED));
        }
    }

    // The listing only ever opens on the open file, so only a row that side
    // holds offers to go there.
    match &row.mine {
        Some((_, address)) => {
            if ui
                .button(format!("{address:#x}"))
                .on_hover_text(text(language, Text::CompareGoTo))
                .clicked()
            {
                go_to = Some(*address);
            }
        }
        None => {
            ui.label(egui::RichText::new("—").color(MUTED));
        }
    }

    go_to
}

/// One side of a row: what that file calls the function, or that it has none.
///
/// `moved` marks the function as sitting at another address in that file,
/// which is the ordinary state of every function in a recompiled image and so
/// is said quietly — beside the name rather than in a column of its own.
fn side(
    ui: &mut egui::Ui,
    held: Option<&(String, u64)>,
    standing: Standing,
    moved: bool,
    language: Language,
) {
    match held {
        Some((name, address)) => {
            // One cell, whatever goes in it: a second `ui.label` here would
            // take the next column of the grid and shift every row that has a
            // moved function one place to the right.
            ui.horizontal(|ui| {
                let label = egui::RichText::new(name).monospace();
                match standing {
                    Standing::Changed => ui.label(label.color(ERROR)),
                    _ => ui.label(label),
                }
                .on_hover_text(format!("{address:#x}"));
                if moved {
                    ui.label(
                        egui::RichText::new(text(language, Text::CompareMoved))
                            .color(MUTED)
                            .small(),
                    )
                    .on_hover_text(text(language, Text::CompareMovedHint));
                }
            });
        }
        None => {
            ui.label(
                egui::RichText::new(text(language, Text::CompareAbsent))
                    .color(MUTED)
                    .italics(),
            );
        }
    }
}

const fn pairing_text(pairing: Pairing) -> Text {
    match pairing {
        Pairing::Name => Text::ComparePairingName,
        Pairing::Address => Text::ComparePairingAddress,
        Pairing::Bytes => Text::ComparePairingBytes,
        Pairing::Shape => Text::ComparePairingShape,
        Pairing::Neighbour => Text::ComparePairingNeighbour,
    }
}

/// The libraries and the strings one file holds and the other does not.
fn names(ui: &mut egui::Ui, report: &Report, language: Language) {
    for (title, changes) in [
        (Text::CompareOnlyLibraries, &report.libraries),
        (Text::CompareOnlyStrings, &report.strings),
    ] {
        if !changes.any() {
            continue;
        }
        card(ui, text(language, title), |ui| {
            if changes.truncated {
                ui.colored_label(ERROR, text(language, Text::CompareTruncatedList));
                ui.add_space(6.0);
            }
            for (item, names, sign) in [
                (Text::CompareOnlyTheirs, &changes.added, '+'),
                (Text::CompareOnlyMine, &changes.removed, '−'),
            ] {
                if names.is_empty() {
                    continue;
                }
                ui.label(
                    egui::RichText::new(format!("{} ({})", text(language, item), names.len()))
                        .color(MUTED),
                );
                for name in names.iter().take(LISTED_PER_CARD) {
                    ui.monospace(format!("{sign} {name}"));
                }
                if names.len() > LISTED_PER_CARD {
                    ui.label(
                        egui::RichText::new(format!("… {}", names.len() - LISTED_PER_CARD))
                            .color(MUTED),
                    );
                }
                ui.add_space(6.0);
            }
        });
        ui.add_space(8.0);
    }
}

/// How many names a card shows before it stops.
///
/// The card sits under a table the reader scrolls; a list of five thousand
/// inside it would push everything below it out of reach and answer nothing
/// the first twenty do not. What is held back is counted rather than dropped
/// in silence.
const LISTED_PER_CARD: usize = 20;

/// What each file's section table says, side by side.
fn sections(ui: &mut egui::Ui, report: &Report, language: Language) {
    if report.sections.is_empty() {
        return;
    }
    card(ui, text(language, Text::CompareSectionsTitle), |ui| {
        let row_spacing = ui.spacing().item_spacing.y;
        egui::Grid::new("compare-sections")
            .num_columns(3)
            .striped(true)
            .spacing([18.0, row_spacing])
            .min_row_height(ROW_HEIGHT)
            .show(ui, |ui| {
                ui.strong(text(language, Text::Name));
                ui.strong(text(language, Text::CompareMine));
                ui.strong(text(language, Text::CompareTheirs));
                ui.end_row();

                for section in &report.sections {
                    section_row(ui, section, language);
                    ui.end_row();
                }
            });
    });
}

fn section_row(ui: &mut egui::Ui, section: &SectionRow, language: Language) {
    let name = egui::RichText::new(&section.name).monospace();
    ui.label(if section.changed {
        name.color(ERROR)
    } else {
        name
    });
    for facts in [section.mine, section.theirs] {
        match facts {
            Some(facts) => {
                ui.label(format_size(facts.file_size));
            }
            None => {
                ui.label(
                    egui::RichText::new(text(language, Text::CompareAbsent))
                        .color(MUTED)
                        .italics(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::show;
    use crate::{
        app::WorkspaceView,
        compare::{Report, Row, SectionRow, Standing, State},
        i18n::{Language, Text, text},
        testing::{drawn, drawn_text, opened_app, reference_analysis, window_input},
        ui::functions,
    };
    use desdec_core::diff::{Changes, Difference, Pairing, SectionFacts};
    use eframe::egui;

    /// Draws the view over `state` and answers what came out.
    fn frame(state: &mut State) -> String {
        let ctx = egui::Context::default();
        let output = ctx.run(window_input(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, state, false, Language::English);
            });
        });
        drawn_text(&output.shapes)
    }

    /// With nothing set against the open file the view says so, and offers the
    /// one thing there is to do.
    #[test]
    fn the_empty_view_says_nothing_is_set_against_this_file() {
        let drawn = frame(&mut State::default());
        assert!(drawn.contains(text(Language::English, Text::CompareNoOtherFile)));
        assert!(drawn.contains(text(Language::English, Text::CompareChooseOther)));
    }

    /// A file against itself: the count of unchanged functions is drawn, and
    /// nothing is called changed.
    #[test]
    fn a_self_comparison_draws_its_tallies() {
        let analysis = reference_analysis();
        let all = functions::all(analysis);
        let mut state = State {
            report: Some(Report::of(analysis, &all, analysis, &all)),
            ..State::default()
        };

        let drawn = frame(&mut state);
        assert!(drawn.contains(text(Language::English, Text::CompareSameFile)));
        assert!(drawn.contains(text(Language::English, Text::CompareNoDifference)));
        assert!(
            drawn.contains(&format!(
                "{}\u{a0}: 0",
                text(Language::English, Text::CompareChanged)
            )),
            "nothing changed between a file and itself"
        );
    }

    /// And with the unchanged rows hidden — which is how the view opens —
    /// that same comparison draws a table of nothing and says how many it is
    /// holding back.
    #[test]
    fn hiding_the_unchanged_rows_says_how_many_are_held_back() {
        let analysis = reference_analysis();
        let all = functions::all(analysis);
        let mut state = State {
            report: Some(Report::of(analysis, &all, analysis, &all)),
            hide_identical: true,
            ..State::default()
        };

        let drawn = frame(&mut state);
        assert!(drawn.contains(text(Language::English, Text::CompareHiddenCount)));
        assert!(drawn.contains(text(Language::English, Text::CompareNothingToShow)));
    }

    /// A filter that matches nothing answers, rather than drawing an empty
    /// grid the reader has to work out the meaning of.
    #[test]
    fn a_filter_that_matches_nothing_says_so() {
        let analysis = reference_analysis();
        let all = functions::all(analysis);
        let mut state = State {
            report: Some(Report::of(analysis, &all, analysis, &all)),
            filter: "no function is called this".to_owned(),
            ..State::default()
        };

        assert!(frame(&mut state).contains(text(Language::English, Text::CompareNothingToShow)));
    }

    /// A comparison holding one row of every kind there is.
    ///
    /// Built by hand rather than read out of two files: the point is the
    /// layout, and every column has a path only some rows take — a pairing
    /// that is a reading, a distance that was not measured, a side one file
    /// does not hold at all. Two real binaries would exercise whichever of
    /// those they happened to contain.
    fn every_kind_of_row() -> Report {
        let facts = |file_size| SectionFacts {
            file_size,
            virtual_size: file_size,
            entropy: None,
        };
        Report {
            same_file: Some(false),
            rows: vec![
                Row {
                    standing: Standing::Identical,
                    mine: Some(("read_header".to_owned(), 0x1000)),
                    theirs: Some(("read_header".to_owned(), 0x1000)),
                    pairing: Some(Pairing::Name),
                    difference: None,
                    moved: false,
                },
                Row {
                    standing: Standing::Changed,
                    mine: Some(("parse".to_owned(), 0x1100)),
                    theirs: Some(("parse".to_owned(), 0x1180)),
                    pairing: Some(Pairing::Bytes),
                    difference: Some(Difference {
                        removed: 1,
                        added: 4,
                    }),
                    moved: true,
                },
                Row {
                    standing: Standing::Changed,
                    mine: Some(("main".to_owned(), 0x1200)),
                    theirs: Some(("main".to_owned(), 0x1200)),
                    pairing: Some(Pairing::Shape),
                    difference: None,
                    moved: false,
                },
                Row {
                    standing: Standing::OnlyMine,
                    mine: Some(("dropped".to_owned(), 0x1300)),
                    theirs: None,
                    pairing: None,
                    difference: None,
                    moved: false,
                },
                Row {
                    standing: Standing::OnlyTheirs,
                    mine: None,
                    theirs: Some(("added".to_owned(), 0x1400)),
                    pairing: None,
                    difference: None,
                    moved: false,
                },
            ],
            sections: vec![
                SectionRow {
                    name: ".text".to_owned(),
                    mine: Some(facts(0x800)),
                    theirs: Some(facts(0x900)),
                    changed: true,
                },
                SectionRow {
                    name: ".init_array".to_owned(),
                    mine: Some(facts(0x10)),
                    theirs: None,
                    changed: true,
                },
            ],
            libraries: Changes {
                added: vec!["libz.so.1".to_owned()],
                removed: vec!["libbz2.so.1".to_owned()],
                truncated: false,
            },
            strings: Changes {
                added: vec!["a message this build carries".to_owned()],
                removed: Vec::new(),
                truncated: true,
            },
        }
    }

    /// Draws this view alone, and answers every string it put on screen with
    /// where it landed.
    ///
    /// The view alone rather than the whole frame: the panels around it draw
    /// their own text, and a test about this view must not answer for theirs.
    fn rendered(state: &mut State) -> Vec<(String, egui::Pos2)> {
        let ctx = egui::Context::default();
        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, state, false, Language::English);
            });
        };
        // Two frames: a panel is measured on the first and painted after.
        let _ = ctx.run(window_input(), &mut draw);
        let output = ctx.run(window_input(), &mut draw);
        drawn(&output.shapes)
    }

    /// The four tallies name four things and give four counts. Drawn one over
    /// another they read as a smear, and no assertion about the strings
    /// themselves would ever notice: they are all on screen either way. Only
    /// where they landed says it.
    #[test]
    fn nothing_in_the_view_is_drawn_on_top_of_anything_else() {
        let mut state = State {
            report: Some(every_kind_of_row()),
            hide_identical: false,
            ..State::default()
        };

        let mut seen: Vec<egui::Pos2> = Vec::new();
        for (said, at) in rendered(&mut state) {
            // An empty galley paints nothing and hides nothing: the filter
            // field draws one while there is nothing typed in it, and where it
            // lands says nothing about the layout.
            if said.is_empty() {
                continue;
            }
            assert!(
                !seen.contains(&at),
                "{said:?} is drawn on top of something else, at {at:?}"
            );
            seen.push(at);
        }
    }

    /// The view is reachable, and drawing it leaves the workspace on it.
    #[test]
    fn the_workspace_draws_the_comparison_when_it_is_the_open_view() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Compare);
        let output = ctx.run(window_input(), |ctx| {
            crate::ui::views::show_central_panel(&mut app, ctx);
        });

        let drawn = drawn_text(&output.shapes);
        assert!(drawn.contains(app.t(Text::CompareIntro)));
        assert_eq!(app.active_view, WorkspaceView::Compare);
    }
}
