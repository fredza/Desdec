//! Two binaries set beside each other, and what the comparison came to.
//!
//! [`desdec_core::diff`] answers the question; this holds the answer in a form
//! the view can draw. Two things make that its own module rather than a field
//! on the workspace.
//!
//! **The answer must not borrow either file.** A comparison over the analyses
//! themselves would tie the second file's lifetime to the first, and the
//! workspace already owns the first and would then own a borrow of itself.
//! Everything here is owned: the rows carry the names and the addresses they
//! show, and nothing in them points back into an analysis.
//!
//! **It is computed once.** Aligning two bodies is quadratic in their length,
//! and a pair of programs holds tens of thousands of them. Doing that on the
//! frame thread would stop the interface for as long as it took; doing it every
//! frame would stop it for good. So it is done on the thread that read the
//! second file, and what lands here is the finished report.

use std::path::PathBuf;

use desdec_core::{
    Analysis,
    diff::{self, Difference, Pairing},
};

use crate::ui::functions::Function;

/// What one row of the comparison says about one function.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Standing {
    /// Paired, and the same bytes on both sides.
    Identical,
    /// Paired, and the bytes differ.
    Changed,
    /// In the file the reader has open, and nothing in the other was paired
    /// with it.
    OnlyMine,
    /// In the other file only.
    OnlyTheirs,
}

impl Standing {
    /// Whether this is a difference, as against a function both files hold
    /// unchanged.
    #[must_use]
    pub const fn is_a_difference(self) -> bool {
        !matches!(self, Self::Identical)
    }
}

/// One function, as the two files have it.
#[derive(Clone, Debug)]
pub struct Row {
    pub standing: Standing,
    /// What the open file calls it, and where, when it holds it at all.
    pub mine: Option<(String, u64)>,
    /// What the other file calls it, and where.
    pub theirs: Option<(String, u64)>,
    /// How the two were paired; `None` for a row only one file has.
    pub pairing: Option<Pairing>,
    /// How far apart the bodies are. `None` when they were too large to align,
    /// which is not the same as being no distance apart — see
    /// [`desdec_core::diff::MAXIMUM_ALIGNMENT`].
    pub difference: Option<Difference>,
    /// Whether the function sits at a different address in the two files.
    pub moved: bool,
}

/// One section, as the two files have it.
#[derive(Clone, Debug)]
pub struct SectionRow {
    pub name: String,
    pub mine: Option<diff::SectionFacts>,
    pub theirs: Option<diff::SectionFacts>,
    pub changed: bool,
}

/// Everything a comparison came to, owned.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Whether the two are the same bytes; `None` when either file was too
    /// large to be read whole and so has no digest of its own.
    pub same_file: Option<bool>,
    /// The functions, those both files hold first and in the open file's own
    /// address order, then the ones only the other file holds.
    pub rows: Vec<Row>,
    pub sections: Vec<SectionRow>,
    pub libraries: diff::Changes,
    pub strings: diff::Changes,
}

impl Report {
    /// How many rows stand a given way.
    #[must_use]
    pub fn count(&self, standing: Standing) -> usize {
        self.rows
            .iter()
            .filter(|row| row.standing == standing)
            .count()
    }

    /// Whether the comparison found anything at all to report.
    #[must_use]
    pub fn any_difference(&self) -> bool {
        self.rows.iter().any(|row| row.standing.is_a_difference())
            || self.libraries.any()
            || self.strings.any()
            || self.sections.iter().any(|section| section.changed)
    }

    /// Compares two analysed files and their function lists.
    ///
    /// `mine` is the file the reader has open, and every row is written from
    /// its side: what it holds, what it has lost, what the other has gained.
    #[must_use]
    pub fn of(
        mine: &Analysis,
        my_functions: &[Function],
        theirs: &Analysis,
        their_functions: &[Function],
    ) -> Self {
        let my_bodies = bodies(mine, my_functions);
        let their_bodies = bodies(theirs, their_functions);
        let compared = diff::compare(mine, &my_bodies, theirs, &their_bodies);

        let mut rows: Vec<Row> = compared
            .functions
            .pairs
            .iter()
            .map(|pair| {
                let (mine, theirs) = (&my_functions[pair.left], &their_functions[pair.right]);
                Row {
                    standing: match pair.verdict {
                        diff::Verdict::Identical => Standing::Identical,
                        diff::Verdict::Changed => Standing::Changed,
                    },
                    mine: Some((mine.name.clone(), mine.start)),
                    theirs: Some((theirs.name.clone(), theirs.start)),
                    pairing: Some(pair.pairing),
                    difference: pair.difference,
                    moved: pair.moved,
                }
            })
            .collect();
        rows.extend(compared.functions.only_left.iter().map(|index| {
            let function = &my_functions[*index];
            Row {
                standing: Standing::OnlyMine,
                mine: Some((function.name.clone(), function.start)),
                theirs: None,
                pairing: None,
                difference: None,
                moved: false,
            }
        }));
        rows.extend(compared.functions.only_right.iter().map(|index| {
            let function = &their_functions[*index];
            Row {
                standing: Standing::OnlyTheirs,
                mine: None,
                theirs: Some((function.name.clone(), function.start)),
                pairing: None,
                difference: None,
                moved: false,
            }
        }));
        // The open file's own order, which is the order every other view puts
        // its functions in. What only the other file holds cannot be placed in
        // that order at all, so it goes after it rather than at an address the
        // open file does not have.
        rows.sort_by_key(|row| (row.mine.is_none(), row.mine.as_ref().map(|(_, at)| *at)));

        Self {
            same_file: compared.same_file,
            rows,
            sections: compared
                .sections
                .iter()
                .map(|section| SectionRow {
                    name: section.name.to_owned(),
                    mine: section.left,
                    theirs: section.right,
                    changed: section.changed(),
                })
                .collect(),
            libraries: compared.libraries,
            strings: compared.strings,
        }
    }
}

/// The bodies of a file's functions, for the comparison.
///
/// Only a name the *file* gives is passed on. A placeholder built out of an
/// address — `sub_401000` — would pair two unrelated functions that happen to
/// begin at the same place in two different programs, and the pairing would
/// then be reported as the strongest kind there is.
fn bodies<'a>(analysis: &'a Analysis, functions: &'a [Function]) -> Vec<diff::Body<'a>> {
    functions
        .iter()
        .map(|function| diff::Body {
            start: function.start,
            name: function
                .found_by
                .is_none()
                .then_some(function.name.as_str()),
            instructions: function.body(analysis),
        })
        .collect()
}

/// The other file, once it has been read and compared.
///
/// Its analysis is deliberately not kept. A second [`Analysis`] is the file's
/// bytes and every instruction decoded from them — hundreds of megabytes for a
/// large image — and nothing here reads it after the report is built: the rows
/// carry the names and the addresses they show. Holding it against a use that
/// does not exist yet would double what an open binary costs, in a tool whose
/// whole first rule is to be bounded.
#[derive(Clone, Debug)]
pub struct Other {
    pub path: PathBuf,
}

/// Where the comparison stands, and what the view is asking to see of it.
pub struct State {
    /// The other file and its analysis, once one has been chosen and read.
    pub other: Option<Other>,
    /// The finished comparison. Held apart from [`Self::other`] because the
    /// two arrive together but are cleared apart: reopening the same second
    /// file against a different first one keeps the file and throws the
    /// comparison away.
    pub report: Option<Report>,
    /// Why the last attempt did not produce one.
    pub error: Option<String>,
    /// Whether the functions both files hold unchanged are kept out of the
    /// table.
    ///
    /// On by default, and the one place in Desdec where something true is
    /// hidden without being asked: a comparison of two builds of the same
    /// program is ten thousand identical rows and forty that matter, and a
    /// table that opens on the ten thousand does not answer the question it
    /// was opened to answer. The switch says how many are being kept out, and
    /// puts them back.
    pub hide_identical: bool,
    /// Free-text filter over the names on either side.
    pub filter: String,
}

impl Default for State {
    /// Derived on everything but [`Self::hide_identical`], which starts on.
    ///
    /// Written out rather than derived because `bool::default()` is `false`,
    /// and a table that opens on ten thousand unchanged rows does not answer
    /// the question it was opened to answer. The switch is right there, and it
    /// says how many it is holding back.
    fn default() -> Self {
        Self {
            other: None,
            report: None,
            error: None,
            hide_identical: true,
            filter: String::new(),
        }
    }
}

impl State {
    /// Forgets the other file, its comparison and any failure.
    ///
    /// Called when the open binary changes: a report is about two files, and
    /// keeping it across a change of the first one would show the reader
    /// answers about a file they have closed.
    pub fn clear(&mut self) {
        self.other = None;
        self.report = None;
        self.error = None;
        self.filter.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{Report, Standing, State};
    use crate::testing::reference_analysis;
    use crate::ui::functions;

    /// A file compared against itself pairs every function with itself and
    /// finds nothing changed. The strongest check there is that the pairing
    /// does not invent differences, and it runs against a real binary rather
    /// than a fixture.
    #[test]
    fn a_file_compared_against_itself_reports_no_difference() {
        let analysis = reference_analysis();
        let all = functions::all(analysis);
        let report = Report::of(analysis, &all, analysis, &all);

        assert_eq!(report.same_file, Some(true));
        assert_eq!(report.count(Standing::Identical), all.len());
        assert_eq!(report.count(Standing::Changed), 0);
        assert_eq!(report.count(Standing::OnlyMine), 0);
        assert_eq!(report.count(Standing::OnlyTheirs), 0);
        assert!(!report.any_difference());
        assert!(report.rows.iter().all(|row| !row.moved));
    }

    /// And every row of that comparison names the same function on both sides.
    #[test]
    fn every_row_of_a_self_comparison_names_the_same_function_twice() {
        let analysis = reference_analysis();
        let all = functions::all(analysis);
        let report = Report::of(analysis, &all, analysis, &all);

        for row in &report.rows {
            assert_eq!(row.mine, row.theirs, "a function paired with another one");
        }
    }

    /// The rows are in the open file's own address order, which is the order
    /// every other view puts its functions in.
    #[test]
    fn the_rows_follow_the_open_file_s_addresses() {
        let analysis = reference_analysis();
        let all = functions::all(analysis);
        let report = Report::of(analysis, &all, analysis, &all);

        let addresses: Vec<u64> = report
            .rows
            .iter()
            .filter_map(|row| row.mine.as_ref().map(|(_, at)| *at))
            .collect();
        assert!(addresses.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    /// Half a file against the whole of it: everything held back is reported
    /// as the other file's alone, and nothing is invented on this side.
    #[test]
    fn a_function_the_other_file_alone_holds_is_reported_as_theirs() {
        let analysis = reference_analysis();
        let all = functions::all(analysis);
        assert!(all.len() > 4, "the reference binary has functions");
        let some = &all[..all.len() / 2];
        let report = Report::of(analysis, some, analysis, &all);

        assert_eq!(report.count(Standing::OnlyMine), 0);
        assert_eq!(report.count(Standing::OnlyTheirs), all.len() - some.len());
        assert!(report.any_difference());
        assert!(
            report
                .rows
                .iter()
                .filter(|row| row.standing == Standing::OnlyTheirs)
                .all(|row| row.mine.is_none()),
            "a row only the other file has must claim nothing on this side"
        );
    }

    /// The unchanged rows start hidden, which is what the field says it does.
    ///
    /// Derived, it started `false`, and the switch's whole reason — a
    /// comparison of two builds is ten thousand identical rows and forty that
    /// matter — was written down and then not applied.
    #[test]
    fn the_unchanged_rows_are_held_back_until_the_reader_asks_for_them() {
        assert!(State::default().hide_identical);
    }

    #[test]
    fn clearing_forgets_the_other_file_and_its_report() {
        let analysis = reference_analysis();
        let all = functions::all(analysis);
        let mut state = State {
            report: Some(Report::of(analysis, &all, analysis, &all)),
            filter: "parse".to_owned(),
            ..State::default()
        };
        assert!(state.report.is_some());

        state.clear();
        assert!(state.report.is_none());
        assert!(state.other.is_none());
        assert!(state.error.is_none());
        assert!(state.filter.is_empty());
    }
}
