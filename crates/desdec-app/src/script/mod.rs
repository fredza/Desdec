//! Turning one reading into a rule, and running it over the whole file.
//!
//! Everything the reader has been given so far answers about one address at a
//! time: this instruction, that function, the note written on this row. A file
//! holds a hundred thousand of them, and the questions worth asking are rarely
//! about one — every function longer than a page, every string nothing refers
//! to, every call to a library that decides something. Asking those by hand is
//! how an afternoon disappears.
//!
//! A script asks them once. It reads the analysis the application already
//! holds and answers in the only terms the application understands: a name for
//! an address, a sentence about it, a mark to come back to, a place to go, a
//! patch to propose. Nothing else. There is no file system here, no process,
//! no network — not because those are forbidden by a rule someone could
//! forget, but because the engine is never handed them.
//!
//! It runs against a *subject* — the analysis, the bytes, the notes — that the
//! application lends for the duration and takes back afterwards, and it
//! produces [`Effect`]s rather than performing them: a script cannot reach
//! into the application and change it, it can only say what it would like
//! changed, and the application decides whether the permissions cover it.
//!
//! Desdec still never executes the analysed binary. What runs here is the
//! reader's own script, written by them or installed by them; the binary
//! remains a file that is read.

mod api;

use std::{rc::Rc, time::Duration};

use serde::{Deserialize, Serialize};

use desdec_core::Analysis;

use crate::{
    annotations::Annotations,
    i18n::{Language, Text},
    xrefs,
};

/// What a script is allowed to do beyond reading what it was handed.
///
/// A permission is asked for in a plugin's manifest and shown to the reader
/// before it is granted, so installing one is a decision made with the list in
/// view rather than a discovery made afterwards. Reading is not on the list:
/// a script that could read nothing could do nothing at all, and the subject
/// it reads is the file already open on screen.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum Permission {
    /// Write labels, comments and bookmarks.
    WriteNotes,
    /// Move the listing, and change what is selected.
    Navigate,
    /// Propose byte patches, which still land in the pending list rather than
    /// in the file: a script cannot write to the analysed binary any more than
    /// the interface can.
    ProposePatches,
}

impl Permission {
    pub const ALL: &[Self] = &[Self::WriteNotes, Self::Navigate, Self::ProposePatches];

    /// What this permission lets a script do, said to the reader granting it.
    #[must_use]
    pub const fn label(self) -> Text {
        match self {
            Self::WriteNotes => Text::PermissionWriteNotes,
            Self::Navigate => Text::PermissionNavigate,
            Self::ProposePatches => Text::PermissionProposePatches,
        }
    }
}

/// One thing a script asked the application to do.
///
/// Collected while the script runs and applied afterwards, in order. A script
/// that fails halfway has still asked for what it asked for before failing,
/// and those effects are kept: a rule that names four hundred functions and
/// then divides by zero has still found four hundred names, and throwing them
/// away would punish the reader for the script's last line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    Label {
        address: u64,
        text: String,
    },
    Comment {
        address: u64,
        text: String,
    },
    Bookmark {
        address: u64,
        on: bool,
    },
    ClearNote {
        address: u64,
    },
    Goto {
        address: u64,
    },
    Patch {
        address: u64,
        bytes: Vec<u8>,
        source: String,
    },
}

impl Effect {
    /// The permission this effect needs, checked before it is ever produced.
    #[must_use]
    pub const fn permission(&self) -> Permission {
        match self {
            Self::Label { .. }
            | Self::Comment { .. }
            | Self::Bookmark { .. }
            | Self::ClearNote { .. } => Permission::WriteNotes,
            Self::Goto { .. } => Permission::Navigate,
            Self::Patch { .. } => Permission::ProposePatches,
        }
    }
}

/// The bounds a script runs inside, none of which it can raise.
///
/// A script is written by the reader or installed by them, so this is not a
/// defence against an attacker holding the keyboard. It is a defence against
/// the ordinary accident — the loop whose condition is never false, the
/// pattern that matches every byte in a two-hundred-megabyte file — which
/// would otherwise take the window with it and lose whatever else was open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// How much work the engine may do before it is stopped.
    pub operations: u64,
    /// How long it may take, whatever it is doing.
    pub time: Duration,
    /// How many effects it may ask for.
    pub effects: usize,
    /// How many hits one search may answer with.
    ///
    /// Its own bound rather than the effects one: a script that reads a
    /// thousand matches and acts on three is doing exactly what it should, and
    /// the two numbers answer different questions.
    pub hits: usize,
    /// How many lines it may print.
    pub printed: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            operations: 50_000_000,
            time: Duration::from_secs(10),
            effects: 200_000,
            hits: 200_000,
            printed: 1_000,
        }
    }
}

/// Why a script stopped short.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Failure {
    /// It reached one of the [`Limits`], and was stopped where it stood.
    Interrupted(Bound),
    /// It asked for something its permissions do not cover.
    Refused(Permission),
    /// It could not be read, or it failed while running. The message is the
    /// engine's own, in the engine's own words: a line and a column the reader
    /// can go to matter more here than a translated sentence that cannot name
    /// either.
    Faulted(String),
}

/// Which bound a script ran into.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Bound {
    Operations,
    Time,
    Effects,
    Printed,
}

impl Bound {
    #[must_use]
    pub const fn label(self) -> Text {
        match self {
            Self::Operations => Text::ScriptTooMuchWork,
            Self::Time => Text::ScriptTooLong,
            Self::Effects => Text::ScriptTooManyEffects,
            Self::Printed => Text::ScriptTooMuchPrinted,
        }
    }
}

/// What a script did, whether or not it finished.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Outcome {
    /// What it printed, in order, and never more than [`Limits::printed`].
    pub printed: Vec<String>,
    /// What it asked for, in order.
    pub effects: Vec<Effect>,
    /// Why it stopped, when it did not finish on its own.
    pub failure: Option<Failure>,
    /// How long it ran.
    pub elapsed: Duration,
}

/// Everything a script may read, lent by the application for the run.
///
/// Moved out of the application rather than copied: a listing of eighteen
/// million instructions and the bytes behind it are not something to duplicate
/// because a reader pressed a key. It comes back by the same door — [`run`]
/// returns it — so the application is never without it for longer than the
/// script takes.
#[derive(Clone, Debug, Default)]
pub struct Subject {
    pub analysis: Option<Analysis>,
    pub file: Vec<u8>,
    pub annotations: Annotations,
    pub xrefs: xrefs::Index,
    /// Named functions with their bounds, small enough to copy: the index the
    /// Functions view holds is keyed into the analysis, and a script has no
    /// use for the basic blocks.
    pub functions: Vec<FunctionBounds>,
}

/// One named function, as a script sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionBounds {
    pub name: String,
    pub start: u64,
    pub end: u64,
}

/// Everything about a run that is not the script or the binary: what it may
/// do, how far it may go, and which language its refusals are written in.
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub granted: Vec<Permission>,
    pub limits: Limits,
    pub language: Language,
}

impl Context {
    /// A run allowed everything, which is what the reader's own console gets:
    /// the person typing into it is the person the permissions protect.
    #[must_use]
    pub fn trusted(language: Language) -> Self {
        Self {
            granted: Permission::ALL.to_vec(),
            limits: Limits::default(),
            language,
        }
    }
}

/// Runs `source` against `subject`, and gives the subject back.
///
/// The subject is returned whatever happens — a script that fails, one that is
/// stopped at a bound, one that does not compile at all. Losing the open
/// binary because a script had a typo in it is not a failure mode this is
/// allowed to have.
#[must_use]
pub fn run(source: &str, subject: Subject, context: &Context) -> (Subject, Outcome) {
    let limits = context.limits;
    let state = Rc::new(api::State::new(context));
    let shared = Rc::new(subject);
    let outcome = {
        let mut engine = rhai::Engine::new();
        sandbox(&mut engine, &state, limits);
        api::register(&mut engine, &shared, &state);
        let started = std::time::Instant::now();
        let result = engine.run(source);
        let elapsed = started.elapsed();
        state.outcome(result, elapsed)
    };
    // The engine, and every closure holding the subject, went out of scope
    // above, so this is the only handle left. The clone is unreachable and
    // still written out: a panic here would take the window and the reader's
    // unsaved notes with it, to save a copy that never happens.
    let subject = Rc::try_unwrap(shared).unwrap_or_else(|shared| (*shared).clone());
    (subject, outcome)
}

/// Shuts every door the engine opens by default, and bounds what is left.
fn sandbox(engine: &mut rhai::Engine, state: &Rc<api::State>, limits: Limits) {
    // A script is one file, written by the reader. `import` would let it pull
    // in another, and `eval` would let it build one at run time out of text —
    // both put code in front of the engine that nobody reviewed, and neither
    // buys a reader of binaries anything.
    engine.disable_symbol("import");
    engine.disable_symbol("eval");
    engine.set_max_operations(limits.operations);
    engine.set_max_call_levels(64);
    engine.set_max_expr_depths(128, 64);
    engine.set_max_string_size(16 * 1024 * 1024);
    engine.set_max_array_size(4 * 1024 * 1024);
    engine.set_max_map_size(1024 * 1024);
    engine.set_max_modules(0);

    let progress = Rc::clone(state);
    engine.on_progress(move |_| progress.tick());

    let printer = Rc::clone(state);
    engine.on_print(move |line| printer.print(line));
    let debugger = Rc::clone(state);
    engine.on_debug(move |line, source, position| {
        debugger.print(&match source {
            Some(source) => format!("{source} ({position}): {line}"),
            None => format!("({position}): {line}"),
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bounds tight enough that a runaway script is caught in a test rather
    /// than in the ten seconds the real ones allow.
    fn tight() -> Limits {
        Limits {
            operations: 100_000,
            time: Duration::from_millis(500),
            effects: 8,
            hits: 32,
            printed: 4,
        }
    }

    fn everything() -> Context {
        Context {
            granted: Permission::ALL.to_vec(),
            limits: tight(),
            language: Language::English,
        }
    }

    fn nothing() -> Context {
        Context {
            granted: Vec::new(),
            limits: tight(),
            language: Language::English,
        }
    }

    /// The reference binary, as a script sees it.
    fn opened() -> Subject {
        let analysis = crate::testing::reference_analysis().clone();
        Subject {
            functions: crate::ui::functions::all(&analysis)
                .iter()
                .map(|function| FunctionBounds {
                    name: function.name.clone(),
                    start: function.start,
                    end: function.end,
                })
                .collect(),
            xrefs: crate::xrefs::Index::of(&analysis, &[]),
            file: crate::testing::reference_bytes().to_vec(),
            annotations: Annotations::default(),
            analysis: Some(analysis),
        }
    }

    #[test]
    fn what_a_script_asks_for_comes_back_as_effects() {
        let (_, outcome) = run(
            r#"label(0x401000, "start"); comment(0x401000, "here"); bookmark(0x401004);"#,
            Subject::default(),
            &everything(),
        );
        assert_eq!(outcome.failure, None, "{outcome:?}");
        assert_eq!(
            outcome.effects,
            vec![
                Effect::Label {
                    address: 0x0040_1000,
                    text: "start".to_owned()
                },
                Effect::Comment {
                    address: 0x0040_1000,
                    text: "here".to_owned()
                },
                Effect::Bookmark {
                    address: 0x0040_1004,
                    on: true
                },
            ]
        );
    }

    #[test]
    fn a_script_without_the_permission_is_refused_by_name() {
        let (_, outcome) = run(
            r#"label(0x401000, "start");"#,
            Subject::default(),
            &nothing(),
        );
        assert_eq!(
            outcome.failure,
            Some(Failure::Refused(Permission::WriteNotes)),
            "the refusal names what to grant"
        );
        assert!(
            outcome.effects.is_empty(),
            "a refused script changes nothing"
        );
    }

    #[test]
    fn each_permission_covers_only_its_own_effects() {
        let context = Context {
            granted: vec![Permission::WriteNotes],
            limits: tight(),
            language: Language::English,
        };
        let (_, outcome) = run(
            r#"label(0x401000, "start"); go_to(0x401000);"#,
            Subject::default(),
            &context,
        );
        assert_eq!(
            outcome.failure,
            Some(Failure::Refused(Permission::Navigate)),
            "writing a note is granted, moving the listing is not"
        );
        assert_eq!(
            outcome.effects.len(),
            1,
            "what was granted still happened: {:?}",
            outcome.effects
        );
    }

    #[test]
    fn a_script_that_fails_halfway_keeps_what_it_already_asked_for() {
        let (_, outcome) = run(
            r#"label(0x401000, "found"); throw "on purpose";"#,
            Subject::default(),
            &everything(),
        );
        assert!(
            matches!(outcome.failure, Some(Failure::Faulted(_))),
            "{:?}",
            outcome.failure
        );
        assert_eq!(
            outcome.effects.len(),
            1,
            "the first name is not thrown away"
        );
    }

    #[test]
    fn a_loop_that_never_ends_is_stopped_rather_than_taking_the_window() {
        let (_, outcome) = run("loop { }", Subject::default(), &everything());
        assert!(
            matches!(
                outcome.failure,
                Some(Failure::Interrupted(Bound::Operations | Bound::Time))
            ),
            "{:?}",
            outcome.failure
        );
    }

    #[test]
    fn a_script_asking_for_more_changes_than_it_may_is_stopped_at_the_bound() {
        let (_, outcome) = run(
            "for i in 0..1000 { bookmark(0x401000 + i); }",
            Subject::default(),
            &everything(),
        );
        assert_eq!(
            outcome.failure,
            Some(Failure::Interrupted(Bound::Effects)),
            "{outcome:?}"
        );
        assert_eq!(outcome.effects.len(), tight().effects);
    }

    #[test]
    fn printing_is_captured_and_bounded() {
        let (_, outcome) = run(
            r#"for i in 0..100 { print("line " + i); }"#,
            Subject::default(),
            &everything(),
        );
        assert_eq!(outcome.printed.len(), tight().printed);
        assert_eq!(
            outcome.printed.first().map(String::as_str),
            Some("line 0"),
            "the lines kept are the first ones, in order"
        );
    }

    #[test]
    fn the_engine_reaches_nothing_of_the_machine() {
        // Not a list of blocked names: these are the ways a script could ever
        // acquire code or capability the reader did not write, and each one
        // has to fail to compile or fail to resolve.
        for forbidden in [
            r#"import "std" as std;"#,
            r#"eval("1 + 1");"#,
            r#"open_file("/etc/passwd");"#,
            r#"system("id");"#,
        ] {
            let (_, outcome) = run(forbidden, Subject::default(), &everything());
            assert!(
                matches!(outcome.failure, Some(Failure::Faulted(_))),
                "{forbidden} was not refused: {:?}",
                outcome.failure
            );
        }
    }

    #[test]
    fn the_subject_comes_back_whatever_the_script_did() {
        let before = opened();
        let instructions = before
            .analysis
            .as_ref()
            .map(|analysis| analysis.instructions.len());
        let bytes = before.file.len();
        let (after, _) = run("loop { }", before, &everything());
        assert_eq!(
            after
                .analysis
                .as_ref()
                .map(|analysis| analysis.instructions.len()),
            instructions,
            "the open binary survives a script that had to be killed"
        );
        assert_eq!(after.file.len(), bytes);
    }

    #[test]
    fn a_script_reads_the_binary_it_was_handed() {
        let subject = opened();
        let expected = subject
            .analysis
            .as_ref()
            .and_then(|analysis| analysis.entry_point);
        let (_, outcome) = run(
            r#"
                let facts = binary();
                print(facts.name);
                print(facts.architecture);
                print("" + instruction_count());
                print("" + entry());
            "#,
            subject,
            &everything(),
        );
        assert_eq!(outcome.failure, None, "{outcome:?}");
        assert_eq!(outcome.printed.len(), 4, "{:?}", outcome.printed);
        if let Some(entry) = expected {
            assert_eq!(
                outcome.printed.get(3).map(String::as_str),
                Some(format!("{entry:#x}").as_str()),
                "an address prints as an address"
            );
        }
    }

    #[test]
    fn a_script_finds_bytes_and_names_what_it_found() {
        let subject = opened();
        // The first instruction's own bytes are certainly in the file.
        let pattern = subject
            .analysis
            .as_ref()
            .and_then(|analysis| analysis.instructions.first())
            .map(|instruction| {
                instruction
                    .bytes
                    .to_vec()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .expect("the reference binary decodes to something");
        let (_, outcome) = run(
            &format!(r#"print("" + find_bytes("{pattern}").len());"#),
            subject,
            &everything(),
        );
        assert_eq!(outcome.failure, None, "{outcome:?}");
        let found: usize = outcome
            .printed
            .first()
            .and_then(|line| line.parse().ok())
            .unwrap_or_default();
        assert!(found > 0, "the bytes of a decoded instruction were found");
    }

    #[test]
    fn an_address_past_the_middle_of_the_space_compares_as_itself() {
        // A kernel module is mapped high, and every address in it has the top
        // bit set. Read as a signed integer, each one is negative and every
        // comparison against a written address comes out backwards — which is
        // why an address is its own type here.
        let mut analysis = crate::testing::reference_analysis().clone();
        analysis.entry_point = Some(0xffff_ffff_8100_0000);
        let subject = Subject {
            analysis: Some(analysis),
            ..Subject::default()
        };
        let (_, outcome) = run(
            r#"
                if entry() > 0x400000 { print("above"); } else { print("below"); }
                print(entry());
            "#,
            subject,
            &everything(),
        );
        assert_eq!(outcome.failure, None, "{outcome:?}");
        assert_eq!(
            outcome.printed.first().map(String::as_str),
            Some("above"),
            "a high address is above a low one, not below it"
        );
        assert_eq!(
            outcome.printed.get(1).map(String::as_str),
            Some("0xffffffff81000000"),
            "and it prints in full"
        );
    }

    #[test]
    fn arithmetic_on_an_address_stays_an_address() {
        let (_, outcome) = run(
            r#"
                let here = address(0x401000);
                print(here + 4);
                print(here - 0x1000);
                print("" + ((here + 8) - here));
            "#,
            Subject::default(),
            &everything(),
        );
        assert_eq!(outcome.failure, None, "{outcome:?}");
        assert_eq!(
            outcome.printed,
            vec!["0x401004".to_owned(), "0x400000".to_owned(), "8".to_owned()]
        );
    }

    #[test]
    fn a_script_reads_the_notes_already_written_and_adds_to_them() {
        let mut annotations = Annotations::default();
        annotations.set(
            0x0040_1000,
            crate::annotations::Annotation {
                label: "checked".to_owned(),
                comment: String::new(),
                bookmarked: false,
            },
        );
        let subject = Subject {
            annotations,
            ..Subject::default()
        };
        let (_, outcome) = run(
            r#"
                print(label_of(0x401000));
                if label_of(0x401004) == () { comment(0x401004, "nothing here yet"); }
            "#,
            subject,
            &everything(),
        );
        assert_eq!(outcome.failure, None, "{outcome:?}");
        assert_eq!(outcome.printed.first().map(String::as_str), Some("checked"));
        assert_eq!(outcome.effects.len(), 1);
    }
}
