use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Duration,
};

use desdec_core::{
    AnalysedFile, Analysis, Trace, analyse_path_with_bytes_cancellable, assistant, decompiler,
    emulate::Machine, update, yara,
};
use eframe::{Storage, egui};

use crate::{
    commands::{Command, Shortcut},
    i18n::{Language, Text, text},
    icons::Icon,
    patches::{Editor, Patches},
    preferences::{
        BinaryAnalyzerPreference, DecompilerPreference, DisassemblyStart, Preferences,
        ThemePreference, apply_theme,
    },
    ui::{self, preferences_window::PreferencesTab},
};

pub const PREFERENCES_KEY: &str = "desdec.preferences";

/// How often the host is allowed to save on its own.
///
/// This is the safety net, not the mechanism: preferences are written by
/// [`DesdecApp::persist_settled_preferences`] as soon as they settle. The
/// host's own saving happens on an automatic interval and at a clean
/// shutdown, and a shutdown that is not clean — a forced close, a driver
/// reset, a session ending with the window still open — loses everything
/// since its last write. Its default of 30 seconds was wide enough to lose a
/// theme chosen a moment earlier, which is what made Windows users see their
/// preferences vanish; it is shortened here in case the direct write ever
/// cannot happen.
pub const AUTO_SAVE_INTERVAL: Duration = Duration::from_secs(2);

/// How long the preferences must hold still before they are written.
///
/// Long enough that a gesture which changes one continuously — dragging the
/// navigation menu's edge — is one write rather than one per frame, short
/// enough that a choice made just before the window is closed is already on
/// disk.
pub const SETTLE_DELAY: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WorkspaceView {
    #[default]
    Overview,
    Segments,
    Functions,
    Strings,
    /// The names the file declares: imports and defined symbols; see
    /// [`crate::ui::symbols`].
    Symbols,
    /// C++ classes recovered from the symbol names; see [`crate::ui::classes`].
    Classes,
    Disassembly,
    Decompile,
    /// The file's bytes as they are, sixteen to a row.
    Dump,
    /// A model's reading of what has been decoded; see [`crate::ui::assistant`].
    Assistant,
    /// The emulated processor: registers, memory, breakpoints and a run; see
    /// [`desdec_core::emulate`].
    Machine,
    /// One function drawn as its control flow; see [`crate::ui::graph`].
    Graph,
    /// What the bytes at an address mean, through a type the reader wrote; see
    /// [`crate::ui::types`].
    Structures,
    Patches,
    Yara,
}

impl WorkspaceView {
    pub const ALL: &[Self] = &[
        Self::Overview,
        Self::Segments,
        Self::Functions,
        Self::Strings,
        Self::Symbols,
        Self::Classes,
        Self::Disassembly,
        Self::Decompile,
        Self::Dump,
        Self::Assistant,
        Self::Machine,
        Self::Graph,
        Self::Structures,
        Self::Patches,
        Self::Yara,
    ];

    pub const fn text(self) -> Text {
        match self {
            Self::Overview => Text::Overview,
            Self::Segments => Text::Segments,
            Self::Functions => Text::Functions,
            Self::Strings => Text::Strings,
            Self::Symbols => Text::Symbols,
            Self::Classes => Text::Classes,
            Self::Disassembly => Text::Disassembly,
            Self::Decompile => Text::Decompile,
            Self::Dump => Text::Dump,
            Self::Assistant => Text::AiAssistance,
            Self::Machine => Text::Machine,
            Self::Graph => Text::Graph,
            Self::Structures => Text::Structures,
            Self::Patches => Text::Patches,
            Self::Yara => Text::Yara,
        }
    }

    /// The drawn symbol standing for this view, wherever it is offered — the
    /// toolbar, the navigation menu, and the menu's icon-only rail.
    pub const fn glyph(self) -> Icon {
        match self {
            Self::Overview => Icon::Overview,
            Self::Segments => Icon::Segments,
            Self::Functions => Icon::Functions,
            Self::Strings => Icon::Strings,
            Self::Symbols => Icon::Symbols,
            Self::Classes => Icon::Classes,
            Self::Disassembly => Icon::Disassembly,
            Self::Decompile => Icon::Decompile,
            Self::Dump => Icon::Dump,
            Self::Assistant => Icon::Assistant,
            Self::Machine => Icon::Machine,
            Self::Graph => Icon::Graph,
            Self::Structures => Icon::Structures,
            Self::Patches => Icon::Patches,
            Self::Yara => Icon::Yara,
        }
    }

    /// The command that opens this view, so a menu entry and a shortcut are
    /// never two different ways of saying different things.
    pub const fn command(self) -> Command {
        match self {
            Self::Overview => Command::Overview,
            Self::Segments => Command::Segments,
            Self::Functions => Command::Functions,
            Self::Strings => Command::Strings,
            Self::Symbols => Command::Symbols,
            Self::Classes => Command::Classes,
            Self::Disassembly => Command::Disassembly,
            Self::Decompile => Command::Decompile,
            Self::Dump => Command::Dump,
            Self::Assistant => Command::AiAssistance,
            Self::Machine => Command::Machine,
            Self::Graph => Command::Graph,
            Self::Structures => Command::Structures,
            Self::Patches => Command::Patches,
            Self::Yara => Command::Yara,
        }
    }

    /// What a not-yet-implemented view announces. `None` for views that already
    /// show real data.
    pub const fn planned_explanation(self) -> Option<Text> {
        match self {
            Self::Overview | Self::Segments | Self::Functions | Self::Strings => None,
            Self::Symbols | Self::Classes => None,
            Self::Disassembly | Self::Decompile | Self::Dump | Self::Assistant => None,
            Self::Machine | Self::Graph | Self::Structures => None,
            Self::Patches | Self::Yara => None,
        }
    }
}

/// One modal window the interface can put on screen.
///
/// Naming the windows in an enum, rather than giving [`Dialogs`] one field per
/// window, is what keeps them consistent: adding one used to mean editing a
/// count, a flag list and two index tables in step, and forgetting any of them
/// still compiled — the new window simply never took part in `Escape`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dialog {
    CommandPalette,
    Preferences,
    About,
    /// Explanation of one linked library.
    Library,
    /// What an instruction's operand designates.
    Operand,
    /// The assembly behind a pseudo-code line.
    Assembly,
    /// The account of what the application has done this session.
    Output,
    /// The reader's own note on one address.
    Annotation,
    /// Who names an address.
    References,
    /// Finding a run of bytes, an instruction or a note.
    Search,
    /// The reader's own script, and what running it did.
    Console,
    /// What is installed, what it asks for, and what was granted.
    Plugins,
    /// Whether Desdec may ask GitHub about newer releases, asked once.
    UpdateConsent,
    /// A newer release, what it changes, and the download of it.
    Update,
    /// The reader's own arithmetic over the machine's state.
    Expression,
    /// One value in every base at once, and the bit operations over it.
    Calculator,
    /// A run carried on until a condition holds.
    TraceUntil,
    /// The reader's own descriptions of the libraries they meet.
    LibraryFile,
    /// The complete versioned JSON emitted by the selected external analyzer.
    ExternalAnalysis,
}

impl Dialog {
    pub const ALL: [Self; 19] = [
        Self::CommandPalette,
        Self::Preferences,
        Self::About,
        Self::Library,
        Self::Operand,
        Self::Assembly,
        Self::Output,
        Self::Annotation,
        Self::References,
        Self::Search,
        Self::Console,
        Self::Plugins,
        Self::UpdateConsent,
        Self::Update,
        Self::Expression,
        Self::Calculator,
        Self::TraceUntil,
        Self::LibraryFile,
        Self::ExternalAnalysis,
    ];

    const fn index(self) -> usize {
        self as usize
    }

    /// Whether a press on the workspace behind this window closes it.
    ///
    /// True of every window that answers one question and is then done with.
    /// The session's account is not one of those: a reader keeps it open
    /// beside the listing to watch what happens, and it would be gone on the
    /// first row they clicked. `Escape` and its own close button still shut
    /// it, which is what a window kept on purpose needs.
    const fn closed_by_a_press_outside(self) -> bool {
        // A script is written over several minutes, in a window the reader
        // keeps beside the listing they are writing it about; closing it on a
        // press would throw away what they had typed.
        // The update window is not one either: a download runs in it, and a
        // press on the listing behind would abandon it half-way.
        !matches!(
            self,
            Self::Output
                | Self::References
                | Self::Search
                | Self::Console
                | Self::Plugins
                | Self::Update
                // An expression is written over several tries, against a
                // machine the reader is stepping in the view behind: a press
                // there is how they change what it will answer.
                | Self::Expression
                // The calculator is kept open beside the listing for as long
                // as the reader is converting things in it, and a press on the
                // view behind is how they find the next thing to convert.
                | Self::Calculator
                // The descriptions are typed over several minutes, and a
                // press on the listing behind would throw away what is in
                // the box before it reached the file.
                | Self::LibraryFile
        )
    }
}

const DIALOG_COUNT: usize = Dialog::ALL.len();

/// Modal windows. Each one is opened from wherever in the interface needs it;
/// opening one is stamped here, which is what lets `Escape` close the window
/// the user is actually looking at and lets a new window step aside from the
/// ones already on screen.
#[derive(Default)]
pub struct Dialogs {
    open: [bool; DIALOG_COUNT],
    /// Opening rank of each dialog, the highest being the topmost.
    ranks: [u64; DIALOG_COUNT],
    /// How many windows were already on screen when each one opened, kept
    /// until the window that owns it has placed itself.
    steps: [Option<usize>; DIALOG_COUNT],
    /// The window a press outside closed this frame; see [`Self::toggle`].
    dismissed_by_press: Option<Dialog>,
    clock: u64,
}

impl Dialogs {
    #[must_use]
    pub const fn is_open(&self, dialog: Dialog) -> bool {
        self.open[dialog.index()]
    }

    /// Whether any window is on screen.
    ///
    /// A view that reads bare keys asks first: the windows are drawn after the
    /// central panel, so a key the panel takes never reaches the palette
    /// walking its own list with the same arrows.
    #[must_use]
    pub fn any_open(&self) -> bool {
        self.open.iter().any(|open| *open)
    }

    /// Every window calls this once a frame with the state of its close
    /// button, so only a change from shut to open counts as an opening.
    pub fn set(&mut self, dialog: Dialog, open: bool) {
        let index = dialog.index();
        if open && !self.open[index] {
            self.clock += 1;
            self.ranks[index] = self.clock;
            self.steps[index] = Some(self.open.iter().filter(|open| **open).count());
        }
        self.open[index] = open;
    }

    pub fn open(&mut self, dialog: Dialog) {
        self.set(dialog, true);
    }

    pub fn close(&mut self, dialog: Dialog) {
        self.set(dialog, false);
    }

    /// A window that the press now being handled has just closed stays closed.
    ///
    /// The toolbar's palette button sits behind the palette it opens: the same
    /// press closes the window and then reaches the button, and without this
    /// the button would reopen what the press just dismissed — leaving a
    /// palette that cannot be closed by the button that opened it.
    pub fn toggle(&mut self, dialog: Dialog) {
        if self.dismissed_by_press == Some(dialog) {
            return;
        }
        self.set(dialog, !self.is_open(dialog));
    }

    /// How many windows this one has to step aside from, taken once by the
    /// window as it opens so the reader can then move it where they like.
    pub fn opening_step(&mut self, dialog: Dialog) -> Option<usize> {
        self.steps[dialog.index()].take()
    }

    /// Closes the most recently opened dialog and says which it was.
    fn dismiss_topmost(&mut self) -> Option<Dialog> {
        self.dismiss_topmost_of(|_| true)
    }

    /// Closes the most recently opened window the filter accepts.
    fn dismiss_topmost_of(&mut self, wanted: impl Fn(Dialog) -> bool) -> Option<Dialog> {
        let topmost = Dialog::ALL
            .into_iter()
            .filter(|dialog| self.is_open(*dialog) && wanted(*dialog))
            .max_by_key(|dialog| self.ranks[dialog.index()])?;
        self.open[topmost.index()] = false;
        Some(topmost)
    }
}

#[derive(Default)]
pub struct PaletteState {
    pub query: String,
    pub selected: usize,
}

/// What a pseudo-code click can be mapped to without inventing source-line
/// addresses for an external decompiler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PseudocodeAssembly {
    Instruction(u64),
    Function(u64),
}

/// The history stays useful without letting a long-lived preferences file grow
/// indefinitely. New successful analyses are inserted at the front.
///
/// Twelve of them filled the menu's whole height and pushed the views out of
/// sight, which is the opposite of what a shortcut is for: nobody scrolls a
/// side panel looking for the file they opened nine binaries ago. Five is
/// what a glance takes in, and the file dialog opens on the last directory
/// used for anything older.
const RECENT_BINARY_LIMIT: usize = 5;

/// Work handed to background threads so the interface never blocks on the file
/// system or on a native dialog.
#[derive(Default)]
struct BackgroundJobs {
    file_picker: Option<Receiver<Option<PathBuf>>>,
    inspection: Option<InspectionJob>,
    /// Complete report from the optional isolated binary analyzer.
    external_analysis: Option<Receiver<Result<String, String>>>,
    /// An external decompiler, which can take a minute on a large binary.
    decompilation: Option<Receiver<std::io::Result<String>>>,
    /// Where the user chose to export the patched copy.
    export_picker: Option<Receiver<Option<PathBuf>>>,
    /// Optional external YARA scan, which can take time on large files.
    yara: Option<Receiver<Result<Vec<yara::Match>, String>>>,
    /// A model reading the listing, local or remote; both take seconds.
    assistance: Option<Receiver<Result<assistant::Answer, assistant::Error>>>,
    /// Asking GitHub whether there is a newer release.
    update_check: Option<Receiver<Result<update::Release, update::Error>>>,
    /// Fetching one, which is several megabytes.
    update_download: Option<Receiver<Result<PathBuf, update::Error>>>,
}

/// One in-flight analysis, and the flag the interface uses to abandon it.
struct InspectionJob {
    receiver: Receiver<(PathBuf, std::io::Result<PreparedInspection>)>,
    cancelled: Arc<AtomicBool>,
}

/// Everything the interface needs from one opened file, prepared away from
/// the frame loop. Installing this is just moving values into the app: a
/// large binary must never appear to have opened successfully, then freeze
/// while its indexes are built on the UI thread.
struct PreparedInspection {
    analysis: Analysis,
    file_bytes: Vec<u8>,
    functions: Vec<crate::ui::functions::Function>,
    stack: Trace,
    string_references: crate::ui::strings::CodeReferences,
    section_starts: Vec<usize>,
    listing_columns: crate::ui::disassembly::Columns,
    callgraph: crate::callgraph::Graph,
    xrefs: crate::xrefs::Index,
    names: crate::names::Table,
}

impl PreparedInspection {
    fn of(analysed: AnalysedFile) -> Self {
        let AnalysedFile { analysis, bytes } = analysed;
        let functions = crate::ui::functions::all(&analysis);
        let stack = Trace::of(&analysis);
        let string_references = crate::ui::strings::CodeReferences::of(&analysis);
        let section_starts = crate::ui::disassembly::section_starts(&analysis);
        let listing_columns = crate::ui::disassembly::Columns::of(&analysis, &stack);
        let callgraph = crate::callgraph::Graph::of(&analysis, &functions);
        let xrefs = crate::xrefs::Index::of(&analysis, &bytes);
        let names = crate::names::Table::of(&analysis, &functions);
        Self {
            analysis,
            file_bytes: bytes,
            functions,
            stack,
            string_references,
            section_starts,
            listing_columns,
            callgraph,
            xrefs,
            names,
        }
    }
}

/// The usual place for downloads, where a platform has one.
///
/// Worked out from the home directory rather than by taking a dependency for
/// it: the answer is one join on every platform Desdec runs on, and a reader
/// who wants it elsewhere says so in the preferences.
fn dirs_download() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    let downloads = home.join("Downloads");
    if downloads.is_dir() {
        return Some(downloads);
    }
    // What a French or Spanish desktop calls it, when the English name is not
    // there. Checked rather than guessed: a path that does not exist would
    // fail the write with a message about a directory the reader never chose.
    for name in ["Téléchargements", "Descargas"] {
        let candidate = home.join(name);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    home.is_dir().then_some(home)
}

/// Where the update stands: what was asked, what came back, what is on disk.
///
/// One value rather than a handful of flags, because the states exclude one
/// another — a check is not running while a download is — and a reader must
/// never be shown a window saying two things at once.
#[derive(Default)]
pub enum UpdateState {
    /// Nothing has been asked. Also where a refusal leaves it.
    #[default]
    Idle,
    /// GitHub has been asked and has not answered yet.
    Checking,
    /// It answered, and this build is the newest there is.
    UpToDate,
    /// There is a newer release.
    Offered(Box<update::Release>),
    /// Its archive is on its way, with what has arrived so far.
    Downloading {
        release: Box<update::Release>,
        received: update::Progress,
    },
    /// It arrived, and its hash is the one the release published.
    Downloaded {
        release: Box<update::Release>,
        file: PathBuf,
    },
    /// Something did not work, said in the reader's own language by the view.
    Failed(update::Error),
}

impl UpdateState {
    /// The release this state is about, when it is about one.
    #[must_use]
    pub const fn release(&self) -> Option<&update::Release> {
        match self {
            Self::Offered(release)
            | Self::Downloading { release, .. }
            | Self::Downloaded { release, .. } => Some(release),
            _ => None,
        }
    }
}

/// Result of the optional local YARA scan.
#[derive(Default)]
pub struct YaraScan {
    pub matches: Vec<yara::Match>,
    pub error: Option<String>,
    pub running: bool,
}

/// A short confirmation of an action that changed nothing on screen.
///
/// Copying to the clipboard is the case this exists for: the bytes go
/// somewhere the application cannot show, so without a word said the reader
/// cannot tell a successful copy from a click that missed. It fades on its own
/// rather than waiting to be dismissed — a confirmation that has to be closed
/// is worse than the silence it replaced.
pub struct Notice {
    pub text: String,
    /// Clock reading past which it stops being drawn.
    pub until: f64,
}

/// A model's reading of what was decoded, and where it came from.
///
/// The question and the exact text that was sent are kept beside the answer:
/// an answer whose question has scrolled away is unattributable, and a reader
/// who cannot see what left their machine has only been told it was harmless.
#[derive(Default)]
pub struct Assistance {
    pub question: Option<assistant::Question>,
    /// Word for word what was sent, shown on demand.
    pub prompt: Option<assistant::Prompt>,
    pub answer: Option<assistant::Answer>,
    pub error: Option<assistant::Error>,
    pub running: bool,
    /// Whether the sent text is unfolded in the view.
    pub show_prompt: bool,
    /// What the provider last answered to a knock on the door, from the
    /// preferences window. Kept across binaries: it is about the machine's
    /// configuration, not about the file.
    pub availability: Option<assistant::Availability>,
}

/// The C that Desdec's own decompiler last produced, and what it was for.
///
/// Kept rather than recomputed each frame — a function of a few hundred
/// instructions decompiles in about a millisecond, which is fine once and not
/// fine sixty times a second — and thrown away when the selection moves, so
/// nothing stale is ever shown under a different name.
#[derive(Default)]
pub struct NativeDecompilation {
    /// The function the text belongs to.
    pub source: Option<u64>,
    pub result: Option<desdec_core::decompiler::native::emit::Decompiled>,
    /// The reader's own names that produced this text, so renaming a variable
    /// is enough to have the pseudo-code redone. Without it the text would be
    /// kept as long as the function did not change, and a name given to a
    /// local would appear only after walking away and coming back.
    pub named: Vec<(String, String)>,
}

/// Result of the selected external decompiler.
#[derive(Default)]
pub struct ExternalDecompilation {
    /// Engine and function the shown text came from, so a stale result is not
    /// left under a different selection.
    pub source: Option<(DecompilerPreference, Option<u64>)>,
    pub text: Option<String>,
    pub error: Option<String>,
    pub running: bool,
    /// Whether this text was read back from the cache rather than produced
    /// now. Said plainly on screen: a reader should know whether they are
    /// looking at a fresh answer or a stored one.
    pub from_cache: bool,
}

/// Result of the optional process-isolated binary analyzer.
#[derive(Default)]
pub struct ExternalAnalysis {
    pub report: Option<String>,
    pub error: Option<String>,
    pub running: bool,
}

#[derive(Default)]
pub struct DesdecApp {
    /// Result of the deep analysis of the loaded binary.
    pub analysis: Option<Analysis>,
    pub error: Option<String>,
    pub active_view: WorkspaceView,
    pub navigation_open: bool,
    /// Whether the menu has already folded its recent-files section shut for
    /// this session.
    ///
    /// `eframe` persists egui's memory, and a collapsing header's open state
    /// lives in it: a reader who unfolded the list once reopened the program
    /// into an unfolded list for good, whatever `default_open` said. The
    /// menu therefore folds it the first frame it draws it and leaves it
    /// alone afterwards, so the section is shut at every opening and still
    /// answers to the reader for the rest of the session.
    pub recent_binaries_folded: bool,
    pub dialogs: Dialogs,
    pub preferences: Preferences,
    pub preferences_tab: PreferencesTab,
    pub editing_shortcut: Option<Command>,
    pub palette: PaletteState,
    /// Named functions of the open binary, with their bodies and basic blocks.
    ///
    /// Derived from the analysis and rebuilt only when one is installed: the
    /// Functions view reads it every frame, and finding basic blocks for a
    /// whole symbol table is not frame work.
    pub functions: Vec<crate::ui::functions::Function>,
    /// The stack at each decoded instruction, indexed once per binary.
    ///
    /// Derived from the analysis and computed only when one is installed: the
    /// disassembly reads it every frame, and following the stack pointer
    /// through a hundred thousand instructions is not frame work.
    pub stack: desdec_core::Trace,
    /// Where the decoded code points, indexed once per binary.
    ///
    /// Every row of the Strings view asks whether anything refers to it, and
    /// answering that from the listing itself meant re-reading a million
    /// instructions on every frame drawn.
    pub string_references: crate::ui::strings::CodeReferences,
    /// Address of the function currently inspected in the Functions view.
    pub selected_function: Option<u64>,
    /// Section the overview sent the reader to, by name.
    ///
    /// The overview states the entry point and the section it lands in; this
    /// is what that statement leads to when it is pressed. Kept as a name
    /// rather than an index because the two views hold their own borrows of
    /// the analysis and never each other's.
    ///
    /// It marks the row for as long as the binary is open. Bringing it into
    /// view is [`Self::pending_section_scroll`]'s business, and only once.
    pub focused_section: Option<String>,
    /// Whether the marked section still has to be brought into view.
    ///
    /// Its own flag because the two are not the same act. Asking for the
    /// scroll on every frame the section is marked would pin the table to that
    /// row and make it impossible to scroll away from — the mark is a state,
    /// the scroll is a moment.
    pub pending_section_scroll: bool,
    /// Whether the names the compiler wrote are left as the file spells them.
    ///
    /// Stored as the opposite of what the switch says, like every other
    /// `hide_*` here: an untouched `false` reads the names back, which is what
    /// a reader opening a Rust binary for the first time wants. Turning the
    /// switch off puts the file's own spelling back.
    ///
    /// Lives with the session rather than with the preferences: it is a way of
    /// looking at one file, changed while reading it, not a setting anyone
    /// goes to a dialog to choose.
    pub mangled_names: bool,
    /// Instruction selected in the disassembly or local pseudo-code view.
    pub selected_instruction: Option<u64>,
    /// Instruction that must be brought into view after a new selection.
    pub pending_instruction_scroll: Option<u64>,
    /// A scroll the reader made in the pseudo-code beside the listing, for the
    /// listing to follow on the next frame.
    ///
    /// The two columns of the disassembly view are one listing read two ways,
    /// so they hold the same rows: the listing decides where the pair stands,
    /// and the panel is drawn at the offset the listing reached. That leaves
    /// the wheel over the panel with nowhere to go, which is why what it did
    /// is kept here and handed back the other way round.
    pub pseudocode_scroll: Option<f32>,
    /// Temporarily draws attention to an instruction reached from another view.
    pub instruction_attention: Option<(u64, f64)>,
    /// The static walk through the code, and the trail it has left.
    pub walk: crate::walk::Walk,
    /// Where the search for a newer release stands.
    pub update: UpdateState,
    /// Whether this session has already looked, so it looks once and not on
    /// every frame that notices the preference is on.
    update_checked_this_session: bool,
    /// Whether the check now running was asked for, rather than started on its
    /// own when the window opened.
    ///
    /// The two deserve different answers when they fail. A reader who pressed
    /// `Check for updates` is owed the reason in the loudest terms the bar
    /// has; the check the window starts by itself is owed nothing louder than
    /// a line in the account, and used to greet every opening with a red
    /// failure naming a platform triple.
    update_check_was_asked_for: bool,
    /// How far the download has got, as the thread doing it reports.
    update_progress: Option<Receiver<update::Progress>>,
    /// Whether the question about updates has already been put this session,
    /// so putting it off puts it off rather than asking again next frame.
    update_consent_asked_this_session: bool,
    /// The emulated processor, once a run has been asked for.
    ///
    /// `None` until then, and built from the file's own section table on the
    /// first press: opening a binary must not start anything, and a reader who
    /// never opens the Machine view has never had one built.
    pub machine: Option<desdec_core::emulate::Machine>,
    /// The address the machine view's memory pane is looking at.
    pub machine_memory_at: Option<u64>,
    /// Which platform's rule says where a function's arguments are, for a run
    /// started at a function rather than at the entry point.
    pub machine_convention: desdec_core::emulate::Convention,
    /// Where the graph view has been panned and zoomed to.
    pub graph: crate::ui::graph::View,
    /// The calculator: the value it holds, and how wide it is read.
    pub calculator: crate::ui::calculator::State,
    /// The expression window: what is written in it, and what it answered.
    pub expression: crate::ui::expression::State,
    /// The reader's own expressions, read again at every pause.
    pub watches: Vec<crate::ui::expression::Watch>,
    /// The conditional trace: what to run until, and for how long at most.
    pub trace_until: crate::ui::trace_until::State,
    /// The types the reader has written about this binary's data, and what is
    /// applied where; see [`crate::ui::types`].
    pub structures: crate::ui::types::State,
    /// What each instruction of the listing touches, named through those
    /// types. Built from the reader's sayings and rebuilt when they change,
    /// because the listing reads it on every visible row of every frame.
    pub member_names: crate::ui::types::MemberNames,
    /// The file's own names for its addresses, indexed once per binary so an
    /// expression can be written about `main` rather than about `0x1a40`.
    pub names: crate::names::Table,
    /// Where each section begins in the listing, indexed once per binary.
    ///
    /// Derived from the analysis, like the stack and the function index, and
    /// for the same reason: finding the section boundaries means walking every
    /// decoded instruction, and a large shared library holds eighteen million.
    pub section_starts: Vec<usize>,
    /// Which function calls which, indexed once per binary.
    ///
    /// Derived from the analysis and the function list, like the indexes
    /// beside it: it walks every decoded call, and the view reads it on every
    /// frame.
    pub callgraph: crate::callgraph::Graph,
    /// How wide each column of the listing is held, indexed once per binary.
    ///
    /// Derived from the analysis and the stack index, like `section_starts`
    /// above: it walks every decoded instruction, and the listing reads it on
    /// every row of every frame. See [`crate::ui::disassembly::Columns`].
    pub listing_columns: crate::ui::disassembly::Columns,
    /// The assembly bubble opened from a pseudo-code line.
    pub pseudocode_assembly: Option<PseudocodeAssembly>,
    /// File offset of the string inspected in the Strings view.
    pub selected_string: Option<u64>,
    /// Free-text filter applied to the extracted strings.
    pub strings_filter: String,
    /// Hide strings that do not belong to a mapped memory region.
    /// Whether the opening pushes of a function — printable bytes the string
    /// extractor cannot tell from text — are kept out of the Strings view.
    pub strings_hide_prologues: bool,
    /// Which strings the reader is asking to see; see [`crate::ui::strings::Scope`].
    pub strings_scopes: crate::ui::strings::Scopes,
    /// Free-text filter applied to the declared symbols.
    pub symbols_filter: String,
    /// Hide the imported names in the Symbols view.
    pub symbols_hide_imports: bool,
    /// Hide the defined names in the Symbols view.
    pub symbols_hide_defined: bool,
    /// Free-text filter applied to the recovered C++ classes.
    pub classes_filter: String,
    /// A short confirmation of something that left no trace on screen.
    pub notice: Option<Notice>,
    /// What the reader has written about the open binary's addresses.
    pub annotations: crate::annotations::Annotations,
    /// The notes as they were last written to disk, and as they stood on the
    /// previous frame: the pair is what says whether there is anything to
    /// write, and whether the reader has stopped typing.
    annotations_saved: crate::annotations::Annotations,
    annotations_last_seen: crate::annotations::Annotations,
    annotations_changed_at: Option<f64>,
    /// Everything the application has done this session, in order.
    ///
    /// Held in this process alone: an account of a session is an account of
    /// which files someone opened, and it is never written anywhere.
    pub journal: crate::journal::Journal,
    /// Pending byte patches, and the instruction being edited.
    pub patches: Patches,
    pub patch_editor: Option<Editor>,
    /// Outcome of the last export, kept until the next one.
    pub export_report: Option<Result<PathBuf, String>>,
    /// How many cache entries the last clear removed.
    pub cache_report: Option<usize>,
    /// What each linked library is for, read once per session.
    pub library_notes: crate::libraries::Catalogue,
    /// The file of those descriptions, while it is being edited.
    pub library_file: crate::ui::library_file::Draft,
    /// The library whose explanation is on screen.
    pub explaining_library: Option<String>,
    /// Where the button that asked sits, so the explanation opens over it
    /// rather than wherever the window happened to be left.
    pub explaining_library_at: Option<egui::Rect>,
    /// The instruction whose operand is being inspected.
    pub inspecting_operand: Option<u64>,
    /// The address whose note is open for editing.
    pub annotating_address: Option<u64>,
    /// Who names which address, indexed once per binary.
    pub xrefs: crate::xrefs::Index,
    /// The address the references window is answering about.
    pub references_address: Option<u64>,
    /// What is being looked for, and what was found.
    pub search: crate::ui::search::State,
    /// What the reader is writing in the script console, and what running it
    /// last did.
    pub script: crate::ui::script::State,
    /// What the plugin directory held when it was last read.
    pub plugins: crate::plugins::Installed,
    /// Where the byte view is looking.
    pub dump: crate::ui::dump::State,
    /// The bytes of the open file, kept for reading what an operand points at.
    pub file_bytes: Vec<u8>,
    /// Text produced by the selected external decompiler.
    pub external: ExternalDecompilation,
    /// Report generated by the analyzer selected in Preferences.
    pub external_analysis: ExternalAnalysis,
    /// C produced by Desdec's own decompiler, for the selected function.
    pub native: NativeDecompilation,
    pub yara: YaraScan,
    /// The assistant's last reading, and the request behind it.
    pub assistance: Assistance,
    /// What was found for each engine, and for which configured path.
    ///
    /// Detecting an engine touches the file system, and `rz-ghidra` is probed
    /// by running `rizin`: doing that every frame while the preferences window
    /// is open would spawn a process sixty times a second.
    engine_availability: HashMap<&'static str, (String, decompiler::Availability)>,
    jobs: BackgroundJobs,
    /// Preferences as of the previous frame, and the time they last changed,
    /// so a burst of changes is written once rather than frame by frame.
    preferences_last_seen: Preferences,
    preferences_changed_at: Option<f64>,
    /// Last state handed to storage, used to detect unsaved preferences.
    persisted_preferences: Preferences,
}

impl DesdecApp {
    /// `path` is the file named on the command line, already sorted from the
    /// options by [`crate::cli`]. Reading the arguments again here would mean
    /// two places deciding what `--version` means.
    pub fn new(creation_context: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        let mut preferences: Preferences = creation_context
            .storage
            .and_then(|storage| eframe::get_value(storage, PREFERENCES_KEY))
            .unwrap_or_default();
        // A file written when the history was longer would otherwise keep its
        // extra entries until the next binary is opened.
        preferences.recent_binaries.truncate(RECENT_BINARY_LIMIT);
        crate::fonts::install(&creation_context.egui_ctx);
        apply_theme(&creation_context.egui_ctx, preferences.theme);
        let mut app = Self {
            persisted_preferences: preferences.clone(),
            preferences_last_seen: preferences.clone(),
            preferences,
            ..Self::default()
        };

        app.reload_plugins();

        // `desdec-app <binary>` starts the analysis straight away, like a file
        // manager handing the application a file to open.
        if let Some(path) = path {
            app.inspect_binary(&creation_context.egui_ctx, path);
        }
        app
    }

    pub fn t(&self, item: Text) -> &'static str {
        text(self.preferences.language, item)
    }

    pub fn set_theme(&mut self, ctx: &egui::Context, theme: ThemePreference) {
        self.preferences.theme = theme;
        apply_theme(ctx, theme);
    }

    pub fn command_tooltip(&self, command: Command) -> String {
        let label = command.label(self.preferences.language);
        match self.preferences.shortcuts.shortcut_for(command) {
            Some(shortcut) => format!("{label} ({})", shortcut.label()),
            None => label,
        }
    }

    /// Command tooltip, unless tooltips are disabled in the preferences.
    pub fn optional_command_tooltip(&self, command: Command) -> Option<String> {
        self.preferences
            .show_tooltips
            .then(|| self.command_tooltip(command))
    }

    pub fn tooltip(&self, response: egui::Response, text: &str) -> egui::Response {
        if self.preferences.show_tooltips {
            response.on_hover_text(text)
        } else {
            response
        }
    }

    pub fn shortcut_label(&self, command: Command) -> String {
        self.preferences
            .shortcuts
            .shortcut_for(command)
            .map_or_else(|| self.t(Text::NoShortcut).to_owned(), Shortcut::label)
    }

    /// Whether the command would do something if run right now.
    ///
    /// The palette lists every command, so the whole application is visible
    /// from one place, but an entry that cannot act must say so rather than
    /// swallow a keystroke: a command that answers nothing reads as broken.
    #[must_use]
    pub fn can_run(&self, command: Command) -> bool {
        if !command.implemented() {
            return false;
        }
        if command == Command::CancelAnalysis {
            return self.is_opening();
        }
        if command.needs_a_binary() && self.analysis.is_none() {
            return false;
        }
        if command.needs_patches() && self.patches.is_empty() {
            return false;
        }
        if command == Command::RunYara && !self.preferences.yara_enabled {
            return false;
        }
        // Asking needs somewhere to ask, and two of the questions need
        // something to ask about. The palette greys them out rather than
        // letting a reader choose an entry that would answer nothing.
        if matches!(
            command,
            Command::AskAboutBinary | Command::AskAboutFunction | Command::AskAboutInstruction
        ) && self.preferences.assistant.provider() == assistant::Provider::None
        {
            return false;
        }
        if command == Command::AskAboutFunction && self.selected_function.is_none() {
            return false;
        }
        if command == Command::AskAboutInstruction && self.selected_instruction.is_none() {
            return false;
        }
        // The walk's transport. A button that cannot move must be greyed out
        // rather than swallow the press: the reader is following a flow, and
        // an unresolved call is an answer they need to see, not a dead key.
        match command {
            // A note is written about an address, so there has to be one.
            Command::EditAnnotation | Command::ToggleBookmark | Command::References => {
                return self.selected_instruction.is_some();
            }
            Command::WalkBack => return self.walk.can_go_back(),
            Command::WalkClear => return !self.walk.is_empty(),
            Command::WalkToEntry => return self.entry_instruction().is_some(),
            Command::WalkStepInto => return self.walk_lands(crate::walk::Step::Into).is_some(),
            Command::WalkStepOver => return self.walk_lands(crate::walk::Step::Over).is_some(),
            Command::WalkStepOut => return self.walk_lands(crate::walk::Step::Out).is_some(),
            Command::RerunDecompilation => {
                return self.preferences.decompiler.engine().is_some() && !self.external.running;
            }
            Command::ShowDecompilationAssembly => return self.selected_function.is_some(),
            _ => {}
        }
        // Choosing an engine always does something — it records the choice —
        // even when that engine is not installed yet. Detecting one spawns a
        // process, and this is asked for every command on every frame the
        // palette is open, so nothing here probes; the pseudo-code view
        // reports a missing engine, with the command that installs it.
        true
    }

    fn set_decompiler(&mut self, choice: DecompilerPreference) {
        self.preferences.decompiler = choice;
        // The shown text belongs to the previous engine.
        self.external = ExternalDecompilation::default();
    }

    /// Starts the chosen external engine again, deliberately bypassing the
    /// cached answer. The old answer remains on disk; this only asks the
    /// selected engine for a fresh observation.
    fn rerun_decompilation(&mut self, ctx: &egui::Context) {
        if self.preferences.decompiler.engine().is_none() || self.external.running {
            return;
        }
        self.external = ExternalDecompilation::default();
        self.request_decompilation_without_cache(ctx, self.selected_function);
    }

    /// Decompiles a function with Desdec's own decompiler, keeping the result.
    ///
    /// Returns `None` for a selection that names no function of this binary —
    /// which is what a stripped file with nothing decoded gives — rather than
    /// an empty body that would read as a function with nothing in it.
    pub fn native_decompilation(
        &mut self,
        address: u64,
    ) -> Option<&desdec_core::decompiler::native::emit::Decompiled> {
        // Recomputed when the function changes, and when the reader has
        // renamed something in it: a name given to a variable has to reach the
        // pseudo-code without the reader having to walk away and come back.
        let named: Vec<(String, String)> = self
            .annotations
            .variables()
            .iter()
            .filter(|named| named.function == address)
            .map(|named| (named.slot.clone(), named.name.clone()))
            .collect();
        if self.native.source != Some(address) || self.native.named != named {
            let analysis = self.analysis.as_ref()?;
            let function = self
                .functions
                .iter()
                .find(|function| function.start == address)?;
            let body = function.body(analysis);
            self.native.result = Some(desdec_core::decompiler::native::decompile(
                &desdec_core::decompiler::native::Request {
                    analysis,
                    name: &function.name,
                    start: function.start,
                    body,
                    file: Some(&self.file_bytes),
                    named_variables: &named,
                },
            ));
            self.native.source = Some(address);
            self.native.named = named;
        }
        self.native.result.as_ref()
    }

    /// What the pseudo-code pane currently represents, ready for a deliberate
    /// copy action. The local translation shares the analysis instruction cap
    /// with the pane, rather than creating a second unbounded operation.
    fn displayed_pseudocode(&self) -> Option<String> {
        if let Some(text) = &self.external.text {
            return Some(text.clone());
        }
        // What the view is showing: the decompiler's own C, for the function
        // the reader is looking at. Copying the line-by-line translation of a
        // whole binary from under it would be a different thing entirely.
        if let Some(decompiled) = &self.native.result {
            return Some(decompiled.text());
        }
        let analysis = self.analysis.as_ref()?;
        if analysis.instructions.is_empty() {
            return None;
        }
        let mut result = String::from("void decompiled_entry(void) {\n");
        for instruction in &analysis.instructions {
            result.push_str("    ");
            result.push_str(&crate::ui::decompile::pseudo_c(&instruction.text));
            result.push('\n');
        }
        result.push('}');
        Some(result)
    }

    /// Where this binary's notes are kept, and the digest that names them.
    ///
    /// `None` when they must not be kept: the reader turned it off, or the
    /// file was only read in part and so has no digest — notes keyed on
    /// anything weaker could be shown for a different binary.
    fn notes_target(&self) -> Option<(PathBuf, [u8; 32])> {
        if !self.preferences.save_annotations {
            return None;
        }
        let digest = self.analysis.as_ref()?.sha256?;
        Some((crate::annotations::directory()?, digest))
    }

    fn stored_annotations(&self) -> Option<crate::annotations::Annotations> {
        let (directory, digest) = self.notes_target()?;
        crate::annotations::read(&directory, &digest)
    }

    /// Writes the notes out, if there is anywhere to write them and anything
    /// to say.
    fn write_annotations(&mut self) {
        if self.annotations == self.annotations_saved {
            return;
        }
        let Some((directory, digest)) = self.notes_target() else {
            return;
        };
        match crate::annotations::write(&directory, &digest, &self.annotations) {
            Ok(()) => self.annotations_saved = self.annotations.clone(),
            Err(error) => self.note(
                crate::journal::Level::Failure,
                format!("{} : {error}", self.t(Text::JournalNotesFailed)),
            ),
        }
        // Either way, not again until something else changes: a directory that
        // cannot be written to would otherwise be attempted every frame.
        self.annotations_changed_at = None;
    }

    /// Writes the notes once the reader has stopped typing.
    ///
    /// The same settling as the preferences, and for the same reason: a write
    /// per keystroke is a hundred writes for one sentence.
    fn persist_settled_annotations(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        // The type definitions are part of what the reader has written about
        // this binary, and are saved by the same settling as the notes rather
        // than by a mechanism of their own.
        if self.annotations.types() != self.structures.source {
            self.annotations.set_types(self.structures.source.clone());
        }
        if self.annotations != self.annotations_last_seen {
            self.annotations_last_seen = self.annotations.clone();
            self.annotations_changed_at = Some(now);
        }
        if self.annotations == self.annotations_saved {
            self.annotations_changed_at = None;
            return;
        }
        let Some(changed_at) = self.annotations_changed_at else {
            return;
        };
        if now - changed_at < SETTLE_DELAY.as_secs_f64() {
            // Come back when the delay is up, even if nothing else asks for a
            // frame: an idle application must still save what it was given.
            ctx.request_repaint_after(SETTLE_DELAY);
            return;
        }
        self.write_annotations();
    }

    /// Runs one round of the note-saving cycle, for the test that checks the
    /// type definitions take part in it.
    #[cfg(test)]
    pub fn persist_settled_annotations_for_a_test(&mut self, ctx: &egui::Context) {
        self.persist_settled_annotations(ctx);
    }

    /// Names every access the reader's sayings cover, again.
    ///
    /// Called when a saying is made or taken back, when the definitions change
    /// and when a binary is opened — never per frame: naming an access means
    /// decoding an operand and walking a type, for every instruction of every
    /// function that has a saying about it.
    pub fn rebuild_member_names(&mut self) {
        let built = crate::ui::types::MemberNames::build(self);
        self.member_names = built;
    }

    /// Records a line in the session's account.
    ///
    /// Every call site is somewhere the application did something the reader
    /// cannot otherwise look back on: a view shows the last answer, never the
    /// one before it.
    pub fn note(&mut self, level: crate::journal::Level, text: impl Into<String>) {
        self.journal.record(level, text);
    }

    /// The entry point, when the format declares one and it was decoded.
    ///
    /// An address with no instruction behind it is no place to start walking:
    /// a packed or stripped file can name an entry in a section that holds no
    /// decoded code, and the transport says so by staying greyed out.
    #[must_use]
    pub fn entry_instruction(&self) -> Option<u64> {
        let analysis = self.analysis.as_ref()?;
        let entry = analysis.entry_point?;
        analysis.instruction_index(entry).map(|_| entry)
    }

    fn disassembly_start(&self, choice: DisassemblyStart) -> Option<u64> {
        let analysis = self.analysis.as_ref()?;
        let decoded = |address| analysis.instruction_index(address).map(|_| address);
        match choice {
            DisassemblyStart::EntryPoint => analysis.entry_point.and_then(decoded),
            DisassemblyStart::Main => self
                .functions
                .iter()
                .find(|function| function.name == "main")
                .and_then(|function| decoded(function.start))
                .or_else(|| analysis.entry_point.and_then(decoded)),
            DisassemblyStart::ProbableFunction => self
                .functions
                .iter()
                .find(|function| function.found_by.is_some())
                .or_else(|| self.functions.first())
                .and_then(|function| decoded(function.start))
                .or_else(|| analysis.entry_point.and_then(decoded)),
        }
    }

    fn select_disassembly_start(&mut self, choice: DisassemblyStart) {
        self.preferences.disassembly_start = choice;
        if let Some(address) = self.disassembly_start(choice) {
            self.selected_instruction = Some(address);
            self.pending_instruction_scroll = Some(address);
            self.active_view = WorkspaceView::Disassembly;
        }
    }

    /// Where one step of the walk would land, from where the reader stands.
    #[must_use]
    pub fn walk_lands(&self, step: crate::walk::Step) -> Option<u64> {
        let analysis = self.analysis.as_ref()?;
        self.walk
            .preview_from(analysis, self.selected_instruction, step)
    }

    /// The reader's own reading of the file: what they name, what they mark,
    /// what they look for, who reaches an address, and the rules they write
    /// about all of it.
    fn run_reading_command(&mut self, ctx: &egui::Context, command: Command) {
        match command {
            Command::References => {
                self.references_address = self.selected_instruction;
                self.dialogs.open(Dialog::References);
            }
            // A window kept open beside the listing, so pressing its key again
            // puts it away rather than reopening it where it already is.
            Command::Search => self.dialogs.toggle(Dialog::Search),
            Command::Script => self.dialogs.toggle(Dialog::Console),
            // The key runs what is in the console, wherever the reader
            // pressed it: a script is written to be run over and over while
            // the listing it is about is on screen, and reaching for the
            // window every time would be the whole friction it removes.
            Command::RunScript => {
                self.dialogs.open(Dialog::Console);
                crate::ui::script::run_console(self, ctx);
            }
            Command::Plugins => self.dialogs.toggle(Dialog::Plugins),
            Command::ReloadPlugins => {
                self.reload_plugins();
                self.dialogs.open(Dialog::Plugins);
            }
            Command::EditAnnotation => {
                self.open_view(command);
                self.annotating_address = self.selected_instruction;
                self.dialogs.open(Dialog::Annotation);
            }
            Command::ToggleBookmark => {
                self.open_view(command);
                if let Some(address) = self.selected_instruction {
                    self.annotations.toggle_bookmark(address);
                }
            }
            // Every other command reaches this through its own arm.
            _ => {}
        }
    }

    /// The emulated processor, built on the first press and kept afterwards.
    ///
    /// Returns `None` when there is nothing to build one over. Building it is
    /// the moment the file's bytes are laid out as an address space, which is
    /// why it does not happen when a binary is merely opened: only asking for
    /// a run, or opening the view a run lives in, does it.
    pub fn machine(&mut self) -> Option<&mut desdec_core::emulate::Machine> {
        if self.machine.is_none() {
            let analysis = self.analysis.as_ref()?;
            // The one copy the emulator costs: the address space borrows these
            // bytes for as long as it lives, and every other view keeps
            // reading the application's own.
            let image: std::sync::Arc<[u8]> = self.file_bytes.as_slice().into();
            self.machine = Some(desdec_core::emulate::Machine::new(analysis, image));
        }
        self.machine.as_mut()
    }

    /// Puts the reader where the run now stands, when the listing has a row
    /// for it. A run that stopped in a library has no row, and the selection
    /// is left where it was rather than cleared.
    pub fn follow_the_run(&mut self) {
        let Some(address) = self.machine.as_ref().map(Machine::instruction_pointer) else {
            return;
        };
        if self.is_decoded(address) {
            self.go_to_instruction(address);
        }
    }

    /// The transport of the emulated run: the same six buttons a debugger has,
    /// each ending by putting the reader on the instruction about to run.
    fn run_machine_command(&mut self, command: Command) {
        use desdec_core::emulate::Step;

        // The view first, whatever happens next: a reader who pressed F9 with
        // no binary open must still be shown where a run would appear, rather
        // than have the key answer nothing.
        self.open_view(command);
        let cursor = self.selected_instruction;
        let Some(machine) = self.machine() else {
            return;
        };
        match command {
            Command::MachineRun => {
                machine.run();
            }
            Command::MachineStepInto => {
                machine.step(Step::Into);
            }
            Command::MachineStepOver => {
                machine.step(Step::Over);
            }
            Command::MachineStepOut => {
                machine.step(Step::Out);
            }
            Command::MachineStepBack => {
                machine.step(Step::Back);
            }
            Command::MachineRunToCursor => {
                let Some(address) = cursor else { return };
                machine.run_to(address);
            }
            Command::MachineRestart => machine.restart(),
            Command::MachineToggleBreakpoint => {
                // The listing draws the mark, so nothing else has to be said:
                // the reader who pressed the key is looking at the row.
                let Some(address) = cursor else { return };
                machine.toggle_breakpoint(address);
                return;
            }
            _ => return,
        }
        // Wherever the run stopped is where the reader should be looking, in
        // the listing as well as in the machine view.
        self.follow_the_run();
    }

    /// The transport of the static walk: six commands that all end by moving
    /// the selection through the listing they open.
    fn run_walk_command(&mut self, command: Command) {
        self.open_view(command);
        match command {
            Command::WalkStepInto => self.walk_step(crate::walk::Step::Into),
            Command::WalkStepOver => self.walk_step(crate::walk::Step::Over),
            Command::WalkStepOut => self.walk_step(crate::walk::Step::Out),
            Command::WalkBack => {
                self.walk.follow_selection(self.selected_instruction);
                if let Some(address) = self.walk.back() {
                    self.go_to_instruction(address);
                }
            }
            Command::WalkToEntry => {
                if let Some(entry) = self.entry_instruction() {
                    self.walk.start(entry);
                    self.go_to_instruction(entry);
                }
            }
            Command::WalkClear => self.walk.clear(),
            // Every other command reaches this through its own arm.
            _ => {}
        }
    }

    /// Throws away every pending patch, and says how many there were.
    fn discard_patches(&mut self) {
        self.note(
            crate::journal::Level::Note,
            format!(
                "{} {}",
                self.t(Text::JournalPatchesDiscarded),
                self.patches.len()
            ),
        );
        self.patches.clear();
        self.patch_editor = None;
        self.export_report = None;
    }

    /// Empties the decompilation cache on disk, and says how much it held.
    fn clear_decompilation_cache(&mut self) {
        self.cache_report = decompilation_cache_dir()
            .and_then(|directory| decompiler::cache::clear(&directory).ok());
        if let Some(removed) = self.cache_report {
            self.note(
                crate::journal::Level::Note,
                format!("{} {removed}", self.t(Text::JournalCacheCleared)),
            );
        }
    }

    /// Takes one step, and brings the listing with it.
    fn walk_step(&mut self, step: crate::walk::Step) {
        let Some(analysis) = &self.analysis else {
            return;
        };
        // A selection moved by hand was not walked to, so the trail restarts
        // there rather than claiming to have arrived by stepping.
        self.walk.follow_selection(self.selected_instruction);
        if let Some(address) = self.walk.step(analysis, step) {
            self.go_to_instruction(address);
        }
    }

    /// Sends the reader to an address in the disassembly, and marks the row
    /// so it can be found on a screen full of hexadecimal.
    ///
    /// Only an address that was decoded: a hit in a data section is somewhere
    /// the listing cannot show, and jumping to the nearest instruction instead
    /// would put the reader somewhere they did not ask for.
    pub fn go_to_address(&mut self, ctx: &egui::Context, address: u64) -> bool {
        let decoded = self
            .analysis
            .as_ref()
            .and_then(|analysis| analysis.instruction_index(address))
            .is_some();
        if !decoded {
            return false;
        }
        self.active_view = WorkspaceView::Disassembly;
        self.go_to_instruction(address);
        self.instruction_attention = Some((address, ctx.input(|input| input.time) + 3.0));
        true
    }

    /// Shows the byte view at a file offset.
    pub fn show_bytes_at_offset(&mut self, offset: u64) {
        self.active_view = WorkspaceView::Dump;
        self.dump.offset = Some(offset);
        self.dump.pending_scroll = Some(offset);
        self.dump.goto_failed = false;
    }

    /// Shows the byte view at whatever address names, when the file holds it.
    ///
    /// `.bss` is mapped and stores nothing, so an address in it has no byte to
    /// show; the view is left where it was rather than sent to the wrong one.
    pub fn follow_in_dump(&mut self, address: u64) -> bool {
        let Some(offset) = self
            .analysis
            .as_ref()
            .and_then(|analysis| analysis.file_offset_of(address))
        else {
            return false;
        };
        self.show_bytes_at_offset(offset);
        true
    }

    /// Whether the listing could show this address at all.
    #[must_use]
    pub fn is_decoded(&self, address: u64) -> bool {
        self.analysis
            .as_ref()
            .and_then(|analysis| analysis.instruction_index(address))
            .is_some()
    }

    /// What to call an address, when anything in the file or in the reader's
    /// own notes calls it something.
    #[must_use]
    pub fn name_at(&self, address: u64) -> Option<String> {
        let analysis = self.analysis.as_ref()?;
        crate::names::describe(address, analysis, &self.functions, &self.annotations)
    }

    /// Selects an instruction and brings it into view in both listings.
    fn go_to_instruction(&mut self, address: u64) {
        self.selected_instruction = Some(address);
        self.pending_instruction_scroll = Some(address);
    }

    pub fn run_command(&mut self, ctx: &egui::Context, command: Command) {
        match command {
            Command::OpenBinary => self.choose_binary(ctx),
            Command::CloseBinary => self.close_binary(),
            Command::CancelAnalysis => self.cancel_analysis(),
            Command::ToggleNavigation => self.navigation_open = !self.navigation_open,
            Command::ToggleToolbar => {
                self.preferences.show_toolbar = !self.preferences.show_toolbar;
            }
            Command::ToggleTooltips => {
                self.preferences.show_tooltips = !self.preferences.show_tooltips;
            }
            Command::CommandPalette => {
                self.dialogs.toggle(Dialog::CommandPalette);
                if self.dialogs.is_open(Dialog::CommandPalette) {
                    self.palette = PaletteState::default();
                }
            }
            Command::Preferences => self.dialogs.open(Dialog::Preferences),
            Command::Output => self.dialogs.toggle(Dialog::Output),
            Command::About => self.dialogs.open(Dialog::About),
            Command::Overview
            | Command::Segments
            | Command::Patches
            | Command::Yara
            | Command::Decompile
            | Command::AiAssistance
            | Command::Disassembly
            | Command::Functions
            | Command::Strings
            | Command::Symbols
            | Command::Classes
            | Command::Dump => self.open_view(command),
            Command::ExportPatched => {
                self.open_view(command);
                self.export_patched_copy(ctx);
            }
            Command::SaveSession => self.save_session(),
            Command::OpenSession => self.open_session(),
            Command::DiscardPatches => self.discard_patches(),
            Command::SendToAsmStudio => {
                self.open_view(command);
                self.send_to_asm_studio(ctx);
            }
            Command::DecompilerBuiltin => self.set_decompiler(DecompilerPreference::Builtin),
            Command::DecompilerRzGhidra => self.set_decompiler(DecompilerPreference::RzGhidra),
            Command::DecompilerRetDec => self.set_decompiler(DecompilerPreference::RetDec),
            Command::RerunDecompilation => {
                self.open_view(command);
                self.rerun_decompilation(ctx);
            }
            Command::ShowDecompilationAssembly => {
                self.open_view(command);
                if let Some(address) = self.selected_function {
                    self.pseudocode_assembly = Some(PseudocodeAssembly::Function(address));
                    self.dialogs.open(Dialog::Assembly);
                }
            }
            Command::CopyPseudoCode => {
                self.open_view(command);
                if let Some(pseudocode) = self.displayed_pseudocode() {
                    self.copy_to_clipboard(ctx, &pseudocode, Text::PseudoCodeCopied);
                }
            }
            Command::DecompilerPreferences => {
                self.preferences_tab = PreferencesTab::Decompiler;
                self.dialogs.open(Dialog::Preferences);
            }
            Command::ToggleDecompilationCache => {
                self.preferences.cache_decompilations = !self.preferences.cache_decompilations;
            }
            Command::ClearDecompilationCache => self.clear_decompilation_cache(),
            Command::References
            | Command::Search
            | Command::EditAnnotation
            | Command::ToggleBookmark
            | Command::Script
            | Command::RunScript
            | Command::Plugins
            | Command::ReloadPlugins => self.run_reading_command(ctx, command),
            Command::WalkStepInto
            | Command::WalkStepOver
            | Command::WalkStepOut
            | Command::WalkBack
            | Command::WalkToEntry
            | Command::WalkClear => self.run_walk_command(command),
            Command::DisassemblyStartEntry => self.select_disassembly_start(DisassemblyStart::EntryPoint),
            Command::DisassemblyStartMain => self.select_disassembly_start(DisassemblyStart::Main),
            Command::DisassemblyStartProbable => self.select_disassembly_start(DisassemblyStart::ProbableFunction),
            Command::MachineRun
            | Command::MachineStepInto
            | Command::MachineStepOver
            | Command::MachineStepOut
            | Command::MachineRunToCursor
            | Command::MachineRestart
            | Command::MachineStepBack
            | Command::MachineToggleBreakpoint
            // Opening the view is the same act as asking for a run: it is what
            // builds the machine, and it goes through the one path that does.
            | Command::Machine => self.run_machine_command(command),
            Command::MachineTraceUntil => {
                self.select_view(WorkspaceView::Machine);
                self.dialogs.open(Dialog::TraceUntil);
            }
            // The graph draws the function already selected, so opening it
            // moves nothing else.
            Command::Graph => self.select_view(WorkspaceView::Graph),
            Command::Structures => self.select_view(WorkspaceView::Structures),
            Command::Expression => self.dialogs.open(Dialog::Expression),
            // Toggled, like every other window a reader keeps open beside
            // the listing: the button in the bar is lit while it is on
            // screen, and a lit button that cannot be pressed off is a lie.
            Command::Calculator => self.dialogs.toggle(Dialog::Calculator),
            Command::AskAboutBinary => {
                self.open_view(command);
                self.request_assistance(ctx, assistant::Question::Binary);
            }
            Command::AskAboutFunction => {
                self.open_view(command);
                if let Some(address) = self.selected_function {
                    self.request_assistance(ctx, assistant::Question::Function { address });
                }
            }
            Command::AskAboutInstruction => {
                self.open_view(command);
                if let Some(address) = self.selected_instruction {
                    self.request_assistance(ctx, assistant::Question::Instruction { address });
                }
            }
            Command::StringsScopeAll => {
                self.open_view(command);
                self.strings_scopes = crate::ui::strings::Scopes::EVERYTHING;
            }
            // Each of these turns its own kind on or off, like the switch it
            // stands for in the view. They used to select one kind and drop
            // the rest, which is what the view itself no longer does.
            Command::StringsScopeUsed => {
                self.open_view(command);
                self.strings_scopes.toggle(crate::ui::strings::Scope::Used);
            }
            Command::StringsScopeMappedUnreferenced => {
                self.open_view(command);
                self.strings_scopes
                    .toggle(crate::ui::strings::Scope::MappedUnreferenced);
            }
            Command::StringsScopeUnmapped => {
                self.open_view(command);
                self.strings_scopes
                    .toggle(crate::ui::strings::Scope::Unmapped);
            }
            Command::StringsClearFilter => {
                self.open_view(command);
                self.strings_filter.clear();
                self.strings_scopes = crate::ui::strings::Scopes::EVERYTHING;
                self.strings_hide_prologues = false;
            }
            Command::ThemeSystem => self.set_theme(ctx, ThemePreference::System),
            Command::ThemeDark => self.set_theme(ctx, ThemePreference::Dark),
            Command::ThemeLight => self.set_theme(ctx, ThemePreference::Light),
            Command::ThemeCatppuccin => self.set_theme(ctx, ThemePreference::Catppuccin),
            Command::ThemeAbyss => self.set_theme(ctx, ThemePreference::Abyss),
            Command::LanguageFrench => self.preferences.language = Language::French,
            Command::LanguageEnglish => self.preferences.language = Language::English,
            Command::LanguageSpanish => self.preferences.language = Language::Spanish,
            Command::TogglePersistence => {
                self.preferences.persistence_enabled = !self.preferences.persistence_enabled;
            }
            Command::CheckForUpdates => {
                // Deliberate: the window opens and says what came back, even
                // when the answer is that there is nothing new.
                if self.preferences.check_for_updates == Some(true) {
                    self.start_update_check(ctx, true);
                } else {
                    self.update = UpdateState::Idle;
                    self.dialogs.open(Dialog::Update);
                }
            }
            Command::RunYara => self.request_yara_scan(ctx),
            Command::ToggleYaraModule => {
                self.preferences.yara_enabled = !self.preferences.yara_enabled;
                if !self.preferences.yara_enabled {
                    self.yara = YaraScan::default();
                }
            }
        }
    }

    fn process_shortcuts(&mut self, ctx: &egui::Context) {
        if self.editing_shortcut.is_some() {
            return;
        }
        // While a text field has the focus, a bare key belongs to what is being
        // typed. A combination held with Ctrl or Alt never does, so those keep
        // working — otherwise the palette, once open, could not be closed with
        // its own shortcut, and no command could be reached from a filter box.
        let typing = ctx.wants_keyboard_input();
        let command = Command::ALL.iter().copied().find(|command| {
            self.preferences
                .shortcuts
                .shortcut_for(*command)
                .is_some_and(|shortcut| {
                    (!typing || shortcut.ctrl || shortcut.alt) && shortcut.pressed(ctx)
                })
        });
        if let Some(command) = command {
            self.run_command(ctx, command);
        }
    }

    /// Escape closes the shortcut capture first, then dialogs from the topmost
    /// to the oldest.
    pub fn dismiss_topmost_dialog(&mut self) -> bool {
        if self.editing_shortcut.is_some() {
            self.editing_shortcut = None;
            return true;
        }
        self.dialogs.dismiss_topmost().is_some()
    }

    fn dismiss_dialog_with_escape(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            return;
        }
        // A dialog first, always: the key belongs to whatever is in front of
        // the reader, and a window open over the graph is in front of it.
        if self.dismiss_topmost_dialog() {
            return;
        }
        self.leave_view_with_escape();
    }

    /// Views a reader looks *into* and then leaves, rather than works in.
    ///
    /// The graph is one: it is opened to find the block worth reading, and
    /// what follows is reading it. Escape is the key every viewer uses for
    /// that, and without it the way back was the navigation rail — which is
    /// collapsed to icons on a narrow window.
    ///
    /// Only the graph, deliberately. The listing, the strings and the rest are
    /// places a reader stays; a key that emptied whichever of them they were
    /// in would be a key nobody could press safely.
    fn leave_view_with_escape(&mut self) {
        if self.active_view == WorkspaceView::Graph {
            self.active_view = WorkspaceView::Disassembly;
        }
    }

    /// A press on the workspace behind the windows closes the topmost one.
    ///
    /// Read from the layers of the previous frame, before the panels are drawn:
    /// a window sits in [`egui::Order::Middle`] and a menu or a drop-down in
    /// [`egui::Order::Foreground`], so only a press that reaches the panels
    /// underneath — the background layer — counts as a press outside. That
    /// distinction is what keeps a window's own drop-down from closing it.
    /// Pressing rather than clicking is what lets a window be dragged by its
    /// title bar and released anywhere.
    ///
    /// Doing it before the panels also means a press on the `?` of another
    /// library closes the explanation on screen and the panel then reopens it
    /// on the new one, rather than the two fighting over the same press.
    fn dismiss_dialog_clicked_outside(&mut self, ctx: &egui::Context) {
        // Only for as long as the press it belongs to is being handled.
        self.dialogs.dismissed_by_press = None;
        let pressed_at = ctx.input(|input| {
            input
                .pointer
                .any_pressed()
                .then(|| input.pointer.interact_pos())
                .flatten()
        });
        let outside = pressed_at.is_some_and(|position| {
            ctx.layer_id_at(position)
                .is_none_or(|layer| layer.order == egui::Order::Background)
        });
        if outside {
            // A press elsewhere gives up on capturing a shortcut as well as
            // closing the window that was asking for it.
            self.editing_shortcut = None;
            self.dialogs.dismissed_by_press = self
                .dialogs
                .dismiss_topmost_of(Dialog::closed_by_a_press_outside);
        }
    }

    /// Recently and successfully analysed files, newest first.
    #[must_use]
    pub fn recent_binaries(&self) -> &[PathBuf] {
        &self.preferences.recent_binaries
    }

    /// Forgets every remembered file without affecting the binary currently
    /// open in the workspace.
    pub fn clear_recent_binaries(&mut self) {
        self.preferences.recent_binaries.clear();
    }

    /// Starts analysing an item chosen from the local history.
    pub fn open_recent_binary(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.inspect_binary(ctx, path);
    }

    /// Opens the file dialog, unless one is already waiting on the user.
    ///
    /// A running analysis does not stand in the way: asking to open another
    /// binary is an answer to the one being analysed, so it is abandoned here
    /// rather than the request being dropped in silence.
    pub fn choose_binary(&mut self, ctx: &egui::Context) {
        if !self.can_choose_binary() {
            return;
        }
        self.cancel_analysis();

        let title = self.t(Text::OpenBinary).to_owned();
        let repaint = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.jobs.file_picker = Some(receiver);
        std::thread::spawn(move || {
            let path = pollster::block_on(rfd::AsyncFileDialog::new().set_title(title).pick_file())
                .map(|file| file.path().to_path_buf());
            let _ = sender.send(path);
            repaint.request_repaint();
        });
    }

    fn inspect_binary(&mut self, ctx: &egui::Context, path: PathBuf) {
        if self.jobs.inspection.is_some() {
            return;
        }

        // The previous failure belongs to the previous file; leaving it on
        // screen next to a running analysis reads as this one having failed.
        self.error = None;
        self.note(
            crate::journal::Level::Note,
            format!("{} {}", self.t(Text::JournalOpening), path.display()),
        );
        let repaint = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.jobs.inspection = Some(InspectionJob {
            receiver,
            cancelled: Arc::clone(&cancelled),
        });
        std::thread::spawn(move || {
            let result =
                analyse_path_with_bytes_cancellable(&path, &cancelled).and_then(|analysed| {
                    analysed.ok_or_else(|| {
                        std::io::Error::new(std::io::ErrorKind::Interrupted, "analysis cancelled")
                    })
                });
            let result = result.map(PreparedInspection::of);
            // Building the indexes is deliberately part of the job. It is all
            // CPU work over the bounded prefix the analysis just read, and
            // doing it on the frame thread made a large file look as if the
            // opening had failed just after the worker answered.
            let result = if cancelled.load(Ordering::Relaxed) {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "analysis cancelled",
                ))
            } else {
                result
            };
            // The receiver is deliberately dropped when the user cancels. A
            // result that completed just before that point is then ignored.
            let _ = sender.send((path, result));
            repaint.request_repaint();
        });
    }

    fn apply_inspection(&mut self, path: &Path, result: std::io::Result<PreparedInspection>) {
        match result {
            Ok(prepared) => {
                self.remember_recent_binary(path);
                // Cleared first: what follows describes the new file, and
                // resetting afterwards would throw the function index away.
                self.reset_file_state();
                self.functions = prepared.functions;
                self.stack = prepared.stack;
                self.string_references = prepared.string_references;
                self.section_starts = prepared.section_starts;
                self.listing_columns = prepared.listing_columns;
                self.callgraph = prepared.callgraph;
                self.file_bytes = prepared.file_bytes;
                self.xrefs = prepared.xrefs;
                self.names = prepared.names;
                self.analysis = Some(prepared.analysis);
                self.selected_instruction =
                    self.disassembly_start(self.preferences.disassembly_start);
                self.error = None;
                // Whatever was worked out about these bytes last time.
                self.annotations = self.stored_annotations().unwrap_or_default();
                self.annotations_saved = self.annotations.clone();
                self.annotations_last_seen = self.annotations.clone();
                // The definitions come back with the file, and are laid out
                // against this file's own shape: how wide its pointers are,
                // and — the one word that differs between an ELF and a PE of
                // the same architecture — how wide its `long` is.
                self.structures.source = self.annotations.types().to_owned();
                if let Some(analysis) = &self.analysis {
                    self.structures.set_model(desdec_core::types::Model::of(
                        analysis.summary.architecture,
                        analysis.summary.format,
                    ));
                }
                self.structures.reread();
                self.rebuild_member_names();
                self.note(crate::journal::Level::Note, self.opened_summary(path));
            }
            Err(error) => {
                let failure = format!(
                    "{} {}: {error}",
                    self.t(Text::CannotInspect),
                    path.display()
                );
                self.note(crate::journal::Level::Failure, failure.clone());
                self.error = Some(failure);
            }
        }
    }

    /// What was found in the file just opened, in one line.
    fn opened_summary(&self, path: &Path) -> String {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let Some(analysis) = &self.analysis else {
            return name;
        };
        format!(
            "{name} : {} {} · {} · {} {} · {} {}",
            analysis.summary.format.label(),
            analysis.summary.architecture.label(),
            crate::ui::format_size(analysis.summary.size),
            analysis.symbols.len(),
            self.t(Text::JournalSymbols),
            analysis.instructions.len(),
            self.t(Text::SectionInstructions),
        )
    }

    /// Places `path` first, removes an older duplicate and caps the history.
    fn remember_recent_binary(&mut self, path: &Path) {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.preferences
            .recent_binaries
            .retain(|known| known != &path);
        self.preferences.recent_binaries.insert(0, path);
        self.preferences
            .recent_binaries
            .truncate(RECENT_BINARY_LIMIT);
    }

    fn poll_background_jobs(&mut self, ctx: &egui::Context) {
        let picked_file = self.jobs.file_picker.as_ref().map(Receiver::try_recv);
        match picked_file {
            Some(Ok(Some(path))) => {
                self.jobs.file_picker = None;
                self.inspect_binary(ctx, path);
            }
            Some(Ok(None) | Err(TryRecvError::Disconnected)) => self.jobs.file_picker = None,
            Some(Err(TryRecvError::Empty)) | None => {}
        }

        let inspection = self
            .jobs
            .inspection
            .as_ref()
            .map(|job| job.receiver.try_recv());
        match inspection {
            Some(Ok((path, result))) => {
                self.jobs.inspection = None;
                self.apply_inspection(&path, result);
                self.start_external_analysis(ctx, path);
                self.run_plugins_on_open(ctx);
            }
            Some(Err(TryRecvError::Disconnected)) => self.jobs.inspection = None,
            Some(Err(TryRecvError::Empty)) | None => {}
        }

        self.poll_decompilation();
        self.poll_external_analysis();
        self.poll_yara();
        self.poll_assistance();
        self.poll_update();

        let export = self.jobs.export_picker.as_ref().map(Receiver::try_recv);
        match export {
            Some(Ok(Some(destination))) => {
                self.jobs.export_picker = None;
                self.write_export(&destination);
            }
            Some(Ok(None) | Err(TryRecvError::Disconnected)) => self.jobs.export_picker = None,
            Some(Err(TryRecvError::Empty)) | None => {}
        }
    }

    /// Starts the separately compiled analyzer after the local inspection has
    /// succeeded. Its process boundary deliberately keeps a C/C++ (or any
    /// other language) implementation out of Desdec's address space.
    fn start_external_analysis(&mut self, ctx: &egui::Context, path: PathBuf) {
        if self.analysis.is_none()
            || self.preferences.binary_analyzer != BinaryAnalyzerPreference::ExternalJson
            || self.jobs.external_analysis.is_some()
        {
            return;
        }
        self.external_analysis = ExternalAnalysis {
            running: true,
            ..ExternalAnalysis::default()
        };
        let program = if self.preferences.external_analyzer_path.trim().is_empty() {
            "desdec-analyzer".to_owned()
        } else {
            self.preferences.external_analyzer_path.trim().to_owned()
        };
        let repaint = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.jobs.external_analysis = Some(receiver);
        std::thread::spawn(move || {
            let result = ProcessCommand::new(&program)
                .args(["report", &path.to_string_lossy(), "--pretty"])
                .output()
                .map_err(|error| format!("{program}: {error}"))
                .and_then(|output| {
                    if output.status.success() {
                        String::from_utf8(output.stdout)
                            .map_err(|error| format!("{program}: invalid UTF-8 report: {error}"))
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                        Err(format!("{program}: {stderr}"))
                    }
                });
            let _ = sender.send(result);
            repaint.request_repaint();
        });
    }

    fn poll_external_analysis(&mut self) {
        let result = self.jobs.external_analysis.as_ref().map(Receiver::try_recv);
        match result {
            Some(Ok(Ok(report))) => {
                self.jobs.external_analysis = None;
                self.external_analysis.running = false;
                self.external_analysis.report = Some(report);
            }
            Some(Ok(Err(error))) => {
                self.jobs.external_analysis = None;
                self.external_analysis.running = false;
                self.external_analysis.error = Some(error);
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.jobs.external_analysis = None;
                self.external_analysis.running = false;
                self.external_analysis.error =
                    Some("external analyzer stopped without a report".to_owned());
            }
            Some(Err(TryRecvError::Empty)) | None => {}
        }
    }

    /// The text an external decompiler was asked for, when it comes back.
    fn poll_decompilation(&mut self) {
        let decompilation = self.jobs.decompilation.as_ref().map(Receiver::try_recv);
        match decompilation {
            Some(Ok(result)) => {
                self.jobs.decompilation = None;
                self.external.running = false;
                match result {
                    Ok(text) => {
                        self.note(crate::journal::Level::Note, self.t(Text::JournalDecompiled));
                        self.external.text = Some(text);
                    }
                    Err(error) => {
                        self.note(
                            crate::journal::Level::Failure,
                            format!("{} : {error}", self.t(Text::JournalDecompileFailed)),
                        );
                        self.external.error = Some(error.to_string());
                    }
                }
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.jobs.decompilation = None;
                self.external.running = false;
            }
            Some(Err(TryRecvError::Empty)) | None => {}
        }
    }

    /// What the local YARA scan matched, when it finishes.
    fn poll_yara(&mut self) {
        let yara = self.jobs.yara.as_ref().map(Receiver::try_recv);
        match yara {
            Some(Ok(result)) => {
                self.jobs.yara = None;
                self.yara.running = false;
                match result {
                    Ok(matches) => {
                        self.note(
                            crate::journal::Level::Note,
                            format!(
                                "{} : {} {}",
                                self.t(Text::Yara),
                                matches.len(),
                                self.t(Text::YaraMatches)
                            ),
                        );
                        self.yara.matches = matches;
                    }
                    Err(error) => {
                        self.note(
                            crate::journal::Level::Failure,
                            format!("{} : {error}", self.t(Text::Yara)),
                        );
                        self.yara.error = Some(error);
                    }
                }
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.jobs.yara = None;
                self.yara.running = false;
                self.yara.error = Some("YARA scan did not return a result".to_owned());
            }
            Some(Err(TryRecvError::Empty)) | None => {}
        }
    }

    /// The model's answer, when it arrives.
    fn poll_assistance(&mut self) {
        let assistance = self.jobs.assistance.as_ref().map(Receiver::try_recv);
        match assistance {
            Some(Ok(result)) => {
                self.jobs.assistance = None;
                self.assistance.running = false;
                match result {
                    Ok(answer) => {
                        self.note(crate::journal::Level::Note, self.t(Text::JournalAnswered));
                        self.assistance.answer = Some(answer);
                    }
                    Err(error) => {
                        self.note(
                            crate::journal::Level::Failure,
                            format!("{} : {error}", self.t(Text::JournalAskFailed)),
                        );
                        self.assistance.error = Some(error);
                    }
                }
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.jobs.assistance = None;
                self.assistance.running = false;
                self.assistance.error = Some(assistant::Error::Unreadable(
                    "the worker returned no answer".to_owned(),
                ));
            }
            Some(Err(TryRecvError::Empty)) | None => {}
        }
    }

    /// Whether a binary is being analysed right now.
    ///
    /// Deliberately not "is something running": while the file dialog is open
    /// nothing is being analysed, and saying otherwise announced an analysis
    /// of a file the user had not chosen yet — and often would never choose.
    pub const fn is_analysing(&self) -> bool {
        self.jobs.inspection.is_some()
    }

    /// Requests the current analysis stop and discards any result it may finish
    /// producing after this point.
    pub fn cancel_analysis(&mut self) {
        if let Some(job) = self.jobs.inspection.take() {
            job.cancelled.store(true, Ordering::Relaxed);
            self.note(
                crate::journal::Level::Warning,
                self.t(Text::JournalCancelled),
            );
        }
        // The dialog is part of the same opening. A desktop that never answers
        // one — a portal that does not come back, a window lost behind another
        // — otherwise left the application waiting on it for the rest of the
        // session, refusing every later request to open anything at all. The
        // receiver is dropped, so a file chosen after this point goes nowhere.
        self.jobs.file_picker = None;
        // With nothing loaded, the view that was selected describes a file that
        // never arrived. Overview is the one that offers a way to open another,
        // and leaving the selection where it was stranded the reader on a view
        // with nothing in it.
        if self.analysis.is_none() {
            self.active_view = WorkspaceView::Overview;
        }
    }

    /// Whether an opening is under way and can be abandoned: the dialog
    /// waiting on a choice, or the analysis of what was chosen.
    #[must_use]
    pub const fn is_opening(&self) -> bool {
        self.jobs.inspection.is_some() || self.jobs.file_picker.is_some()
    }

    /// Whether asking to open a binary would put the file dialog on screen.
    ///
    /// Only a dialog already waiting on the user stands in the way.
    #[must_use]
    pub const fn can_choose_binary(&self) -> bool {
        self.jobs.file_picker.is_none()
    }

    /// Whether the file dialog is open, waiting on the user.
    pub const fn is_choosing_file(&self) -> bool {
        self.jobs.file_picker.is_some()
    }

    /// How long a confirmation stays on screen.
    ///
    /// Long enough to be read after the eye has gone back to what it was
    /// doing, short enough that it is gone before it becomes furniture.
    const NOTICE_SECONDS: f64 = 2.5;

    /// Says something happened that the interface otherwise cannot show.
    pub fn notify(&mut self, ctx: &egui::Context, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            until: ctx.input(|input| input.time) + Self::NOTICE_SECONDS,
        });
        // Without this the notice would sit there until some other event
        // happened to redraw the window, outlasting its own deadline.
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(Self::NOTICE_SECONDS));
    }

    /// Puts `value` on the clipboard and says so, naming what was copied.
    ///
    /// The two belong together: every copy in this application has to be
    /// confirmed, and pairing them here means no call site can forget. The
    /// value goes in the message as well as on the clipboard — "copied" alone
    /// leaves the reader to take the application's word for what it took.
    pub fn copy_to_clipboard(&mut self, ctx: &egui::Context, value: &str, said: Text) {
        ctx.copy_text(value.to_owned());
        let message = format!("{} — {value}", self.t(said));
        self.note(crate::journal::Level::Note, message.clone());
        self.notify(ctx, message);
    }

    /// Whether a native dialog has been put on the user's screen.
    ///
    /// Tests assert this stays false: a test that opened one made seven file
    /// explorers appear while the suite ran.
    #[cfg(test)]
    #[must_use]
    pub const fn showing_a_native_dialog(&self) -> bool {
        self.jobs.file_picker.is_some() || self.jobs.export_picker.is_some()
    }

    /// What is installed for `engine`, detected once per configured path.
    ///
    /// The cache key is the configured path, so editing it re-detects while
    /// typing something else does not.
    pub fn engine_availability(&mut self, engine: decompiler::Engine) -> decompiler::Availability {
        let configured = self
            .preferences
            .engine_paths
            .for_engine(engine)
            .map_or_else(String::new, |path| path.display().to_string());
        if let Some((cached_for, availability)) = self.engine_availability.get(engine.program())
            && *cached_for == configured
        {
            return availability.clone();
        }
        let availability =
            decompiler::locate(engine, self.preferences.engine_paths.for_engine(engine));
        self.engine_availability
            .insert(engine.program(), (configured, availability.clone()));
        availability
    }

    /// Starts the selected external decompiler, unless its result is already
    /// on screen. Does nothing when the built-in engine is selected.
    pub fn request_decompilation(&mut self, ctx: &egui::Context, address: Option<u64>) {
        self.request_decompilation_with_cache(ctx, address, true);
    }

    fn request_decompilation_without_cache(&mut self, ctx: &egui::Context, address: Option<u64>) {
        self.request_decompilation_with_cache(ctx, address, false);
    }

    fn request_decompilation_with_cache(
        &mut self,
        ctx: &egui::Context,
        address: Option<u64>,
        use_cache: bool,
    ) {
        let choice = self.preferences.decompiler;
        let Some(engine) = choice.engine() else {
            self.external = ExternalDecompilation::default();
            return;
        };
        if self.jobs.decompilation.is_some() || self.external.source == Some((choice, address)) {
            return;
        }
        let Some(path) = self
            .analysis
            .as_ref()
            .map(|analysis| analysis.summary.path.clone())
        else {
            return;
        };

        // A cached answer is the same text the engine produced for these exact
        // bytes, so it is shown straight away rather than paying the engine's
        // start-up again.
        if use_cache && let Some(cached) = self.cached_decompilation(engine, address) {
            self.external = ExternalDecompilation {
                source: Some((choice, address)),
                text: Some(cached),
                from_cache: true,
                ..ExternalDecompilation::default()
            };
            return;
        }

        let decompiler::Availability::Found(program) = self.engine_availability(engine) else {
            self.external = ExternalDecompilation {
                source: Some((choice, address)),
                error: Some(format!(
                    "{} {}",
                    self.t(Text::EngineUnavailable),
                    engine.install_hint()
                )),
                ..ExternalDecompilation::default()
            };
            return;
        };

        self.external = ExternalDecompilation {
            source: Some((choice, address)),
            running: true,
            ..ExternalDecompilation::default()
        };
        // Carried into the thread so the answer can be stored the moment it
        // arrives, without the interface having to be involved.
        let store = self.cache_target();
        let repaint = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.jobs.decompilation = Some(receiver);
        std::thread::spawn(move || {
            let result = decompiler::decompile(
                engine,
                &program,
                &path,
                address,
                decompiler::DEFAULT_TIMEOUT,
            );
            if let (Ok(text), Some((directory, digest))) = (&result, &store) {
                // A cache that cannot be written is not a failed analysis: the
                // answer is on screen either way, so this stays silent.
                let _ = decompiler::cache::write(
                    directory,
                    decompiler::cache::Key {
                        digest,
                        engine,
                        address,
                    },
                    text,
                );
            }
            let _ = sender.send(result);
            repaint.request_repaint();
        });
    }

    /// Starts a bounded, static scan with the local YARA command.
    pub fn request_yara_scan(&mut self, ctx: &egui::Context) {
        if !self.preferences.yara_enabled || self.jobs.yara.is_some() || self.yara.running {
            return;
        }
        let Some(binary) = self
            .analysis
            .as_ref()
            .map(|analysis| analysis.summary.path.clone())
        else {
            return;
        };
        let configured = (!self.preferences.yara_path.trim().is_empty())
            .then(|| PathBuf::from(self.preferences.yara_path.trim()));
        let Some(program) = yara::locate(configured.as_deref()) else {
            let failure = "YARA executable was not found";
            self.note(
                crate::journal::Level::Failure,
                format!("{} : {failure}", self.t(Text::Yara)),
            );
            self.yara = YaraScan {
                error: Some(failure.to_owned()),
                ..YaraScan::default()
            };
            return;
        };
        let rules = PathBuf::from(self.preferences.yara_rules_path.trim());
        if self.preferences.yara_rules_path.trim().is_empty() || !rules.is_file() {
            let failure = "YARA rules file was not found";
            self.note(
                crate::journal::Level::Failure,
                format!("{} : {failure}", self.t(Text::Yara)),
            );
            self.yara = YaraScan {
                error: Some(failure.to_owned()),
                ..YaraScan::default()
            };
            return;
        }

        self.note(
            crate::journal::Level::Note,
            format!("{} : {}", self.t(Text::Yara), self.t(Text::JournalScanning)),
        );
        self.yara = YaraScan {
            running: true,
            ..YaraScan::default()
        };
        let repaint = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.jobs.yara = Some(receiver);
        std::thread::spawn(move || {
            let result = yara::scan(&program, &rules, &binary, yara::DEFAULT_TIMEOUT)
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
            repaint.request_repaint();
        });
    }

    // ----- updates ----------------------------------------------------------

    /// Looks once per session, if the reader has said Desdec may look.
    ///
    /// Called from the frame loop rather than from `main`, because the answer
    /// to "may we ask?" can arrive during the session — the reader saying yes
    /// in the window below is what starts the first check.
    pub fn check_for_updates_if_allowed(&mut self, ctx: &egui::Context) {
        if self.update_checked_this_session || self.preferences.check_for_updates != Some(true) {
            return;
        }
        // Not over an answer already on screen. The once-a-session look is a
        // background courtesy; a reader reading what the last one said, or
        // watching a download, must not have it replaced under them.
        if !matches!(self.update, UpdateState::Idle) {
            return;
        }
        self.update_checked_this_session = true;
        self.start_update_check(ctx, false);
    }

    /// Asks whether the reader has been asked at all, and asks them if not.
    ///
    /// Opened once and never again: a refusal is remembered, and a question a
    /// program asks twice is a question it does not accept the answer to.
    pub fn ask_about_updates_if_never_asked(&mut self) {
        // Never over something else. It is a question about the application
        // rather than about the file, so it waits until the reader is not in
        // the middle of anything — a window that steals the press meant for
        // the one underneath is worse than a question asked a minute later.
        if self.preferences.check_for_updates.is_none()
            && !self.update_consent_asked_this_session
            && !self.dialogs.any_open()
        {
            self.dialogs.open(Dialog::UpdateConsent);
        }
    }

    /// Records that the reader agreed, and looks straight away.
    pub fn allow_update_checks(&mut self, ctx: &egui::Context) {
        self.preferences.check_for_updates = Some(true);
        self.dialogs.close(Dialog::UpdateConsent);
        self.update_consent_asked_this_session = true;
        self.note(
            crate::journal::Level::Note,
            format!(
                "{} : {}",
                self.t(Text::Updates),
                self.t(Text::UpdateConsentYes)
            ),
        );
        self.update_checked_this_session = true;
        self.start_update_check(ctx, true);
    }

    /// Puts the question off. It settles nothing, so nothing is written down:
    /// the preference stays unanswered and the question comes back next time
    /// the application starts — but not again in this session, which would be
    /// asking twice in one sitting.
    ///
    /// Turning the checks off for good is a thing the preferences do, where a
    /// decision belongs. A pop-up is a bad place to be asked to decide for
    /// ever, and "never" offered next to "yes" is a question that punishes
    /// hesitation.
    pub fn postpone_update_consent(&mut self) {
        self.dialogs.close(Dialog::UpdateConsent);
        self.update_consent_asked_this_session = true;
    }

    /// Asks GitHub, on a thread, whether there is a newer release.
    ///
    /// `deliberate` is the difference between the reader pressing "check" and
    /// the once-a-session look: the first shows its answer whatever it is, and
    /// the second stays quiet unless there is something to say.
    pub fn start_update_check(&mut self, ctx: &egui::Context, deliberate: bool) {
        if self.preferences.check_for_updates != Some(true) || self.jobs.update_check.is_some() {
            return;
        }
        self.update_check_was_asked_for = deliberate;
        if deliberate {
            self.dialogs.open(Dialog::Update);
        }
        self.update = UpdateState::Checking;
        let repaint = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.jobs.update_check = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(update::latest());
            repaint.request_repaint();
        });
    }

    /// Fetches the offered release's archive and checks what arrives.
    pub fn start_update_download(&mut self, ctx: &egui::Context) {
        let Some(release) = self.update.release().cloned() else {
            return;
        };
        if self.jobs.update_download.is_some() {
            return;
        }
        let directory = self.download_directory();
        self.update = UpdateState::Downloading {
            release: Box::new(release.clone()),
            received: update::Progress {
                received: 0,
                total: release.archive.size,
            },
        };
        let repaint = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        let (reports, seen) = mpsc::channel();
        self.jobs.update_download = Some(receiver);
        self.update_progress = Some(seen);
        std::thread::spawn(move || {
            let result = update::download(&release, &directory, |progress| {
                // The channel is read by the frame loop; a receiver that has
                // gone away means the window closed, and the download carries
                // on to its end rather than leaving a half file behind.
                let _ = reports.send(progress);
                repaint.request_repaint();
            });
            let _ = sender.send(result);
            repaint.request_repaint();
        });
    }

    /// Where a downloaded archive is written: what the reader chose, or the
    /// usual place for downloads on this machine.
    fn download_directory(&self) -> PathBuf {
        let chosen = self.preferences.download_directory.trim();
        if !chosen.is_empty() {
            return PathBuf::from(chosen);
        }
        std::env::var_os("XDG_DOWNLOAD_DIR")
            .map(PathBuf::from)
            .or_else(dirs_download)
            .unwrap_or_else(std::env::temp_dir)
    }

    /// The reader says they do not want this one. Offered again only when
    /// something newer than it appears.
    pub fn skip_offered_update(&mut self) {
        if let Some(release) = self.update.release() {
            self.preferences.skipped_release = Some(release.version.to_string());
        }
        self.update = UpdateState::Idle;
        self.dialogs.close(Dialog::Update);
    }

    /// Whether a release is one to put in front of the reader.
    #[must_use]
    pub fn would_offer(&self, release: &update::Release) -> bool {
        let Some(running) = update::Version::running() else {
            return false;
        };
        if !release.is_newer_than(running) {
            return false;
        }
        // A version the reader turned down stays turned down until something
        // newer than it is published.
        match self
            .preferences
            .skipped_release
            .as_deref()
            .and_then(update::Version::parse)
        {
            Some(skipped) => release.version > skipped,
            None => true,
        }
    }

    /// Takes in whatever the update threads have said since the last frame.
    fn poll_update(&mut self) {
        // The progress reports first, so a download that finished this frame
        // is not drawn as still running.
        if let Some(reports) = self.update_progress.as_ref() {
            let mut latest = None;
            while let Ok(progress) = reports.try_recv() {
                latest = Some(progress);
            }
            if let (Some(progress), UpdateState::Downloading { received, .. }) =
                (latest, &mut self.update)
            {
                *received = progress;
            }
        }

        let checked = self.jobs.update_check.as_ref().map(Receiver::try_recv);
        match checked {
            Some(Ok(answer)) => {
                self.jobs.update_check = None;
                self.apply_update_check(answer);
            }
            Some(Err(TryRecvError::Disconnected)) => self.jobs.update_check = None,
            Some(Err(TryRecvError::Empty)) | None => {}
        }

        let downloaded = self.jobs.update_download.as_ref().map(Receiver::try_recv);
        match downloaded {
            Some(Ok(answer)) => {
                self.jobs.update_download = None;
                self.update_progress = None;
                self.apply_update_download(answer);
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.jobs.update_download = None;
                self.update_progress = None;
            }
            Some(Err(TryRecvError::Empty)) | None => {}
        }
    }

    /// What the check came back with.
    fn apply_update_check(&mut self, answer: Result<update::Release, update::Error>) {
        match answer {
            Ok(release) if self.would_offer(&release) => {
                self.note(
                    crate::journal::Level::Note,
                    format!("{} : {}", self.t(Text::Updates), release.version),
                );
                self.update = UpdateState::Offered(Box::new(release));
                self.dialogs.open(Dialog::Update);
            }
            Ok(_) => self.update = UpdateState::UpToDate,
            Err(error) => {
                // Red is for what the reader asked for and did not get. A
                // check nobody asked for — the one the window starts on its
                // own — has nothing to announce when it comes back empty: the
                // program still works, and the release it could not read is
                // not the reader's business. It goes into the account all the
                // same, where the Output window has it if anyone looks.
                let level = if self.update_check_was_asked_for {
                    crate::journal::Level::Failure
                } else {
                    crate::journal::Level::Note
                };
                let said = crate::ui::update_window::explain(&error, self.preferences.language);
                self.note(level, format!("{} : {said}", self.t(Text::Updates)));
                self.update = UpdateState::Failed(error);
            }
        }
    }

    /// What the download came back with.
    fn apply_update_download(&mut self, answer: Result<PathBuf, update::Error>) {
        let release = self.update.release().cloned();
        match (answer, release) {
            (Ok(file), Some(release)) => {
                self.note(
                    crate::journal::Level::Note,
                    format!("{} : {}", self.t(Text::UpdateVerified), file.display()),
                );
                self.update = UpdateState::Downloaded {
                    release: Box::new(release),
                    file,
                };
            }
            (Ok(_), None) => self.update = UpdateState::Idle,
            (Err(error), _) => {
                let said = crate::ui::update_window::explain(&error, self.preferences.language);
                self.note(
                    crate::journal::Level::Failure,
                    format!("{} : {said}", self.t(Text::Updates)),
                );
                self.update = UpdateState::Failed(error);
            }
        }
    }

    /// The provider and its settings, as the preferences describe them.
    #[must_use]
    pub fn assistant_settings(&self) -> assistant::Settings {
        assistant::Settings {
            provider: self.preferences.assistant.provider(),
            model: self.preferences.assistant_model.clone(),
            ollama_url: self.preferences.ollama_url.clone(),
            api_key_path: PathBuf::from(self.preferences.anthropic_key_path.trim()),
            timeout: assistant::DEFAULT_TIMEOUT,
        }
    }

    /// Asks the configured model a question about what is on screen.
    ///
    /// The request is built here, from the analysis, and kept: the view shows
    /// it in full on demand, so what left the machine is never something the
    /// reader has to take on trust.
    pub fn request_assistance(&mut self, ctx: &egui::Context, question: assistant::Question) {
        if self.assistance.running || self.jobs.assistance.is_some() {
            return;
        }
        let settings = self.assistant_settings();
        let Some(analysis) = self.analysis.as_ref() else {
            return;
        };
        let prompt = assistant::prompt::build(analysis, question, self.preferences.language.name());

        self.assistance = Assistance {
            question: Some(question),
            prompt: Some(prompt.clone()),
            running: true,
            // Kept: the reader's choice about the panel, not about the answer.
            show_prompt: self.assistance.show_prompt,
            availability: self.assistance.availability.clone(),
            ..Assistance::default()
        };

        // What left the machine, and where it went: the one event in this
        // application a reader must always be able to look back on.
        self.note(
            crate::journal::Level::Note,
            format!(
                "{} {}",
                self.t(Text::JournalAsked),
                self.preferences.assistant.provider().label()
            ),
        );
        let repaint = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.jobs.assistance = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(assistant::ask(&settings, &prompt));
            repaint.request_repaint();
        });
    }

    /// Where this binary's answers are kept, and the digest that identifies it.
    /// them.
    ///
    /// `None` when caching must not happen: no digest means the file was only
    /// read in part, and keying on anything weaker would risk showing one
    /// binary's decompilation for another.
    fn cache_target(&self) -> Option<(PathBuf, [u8; 32])> {
        if !self.preferences.cache_decompilations {
            return None;
        }
        let digest = self.analysis.as_ref()?.sha256?;
        Some((decompilation_cache_dir()?, digest))
    }

    fn cached_decompilation(
        &self,
        engine: decompiler::Engine,
        address: Option<u64>,
    ) -> Option<String> {
        let (directory, digest) = self.cache_target()?;
        decompiler::cache::read(
            &directory,
            decompiler::cache::Key {
                digest: &digest,
                engine,
                address,
            },
        )
    }

    /// Asks where to write the patched copy. The analysed file is never the
    /// destination: [`desdec_core::patch::write_patched_copy`] refuses it.
    /// Writes the reader's work to `binary.dcl`, beside the binary.
    ///
    /// No dialog and no chosen path: the file is named after the binary and
    /// sits next to it, which is the whole point — a reader who wants it
    /// somewhere else can copy it, and one who wants it saved wants it saved
    /// rather than wants to be asked where.
    ///
    /// Desdec keeps these notes on its own already, under its data directory
    /// and keyed by the binary's digest. This file is the other half: one a
    /// reader can commit next to the binary, or hand to whoever asked them
    /// what the thing does.
    pub fn save_session(&mut self) {
        let Some(analysis) = self.analysis.as_ref() else {
            self.note(
                crate::journal::Level::Warning,
                self.t(Text::SessionNoBinary),
            );
            return;
        };
        let binary = analysis.summary.path.clone();
        let session = crate::session::Session::of(&binary, analysis.sha256, &self.annotations);
        let path = crate::session::beside(&binary);
        match crate::session::write(&path, &session) {
            Ok(()) => self.note(
                crate::journal::Level::Note,
                format!("{} : {}", self.t(Text::SessionSaved), path.display()),
            ),
            Err(error) => self.note(
                crate::journal::Level::Failure,
                format!("{} : {error}", self.t(Text::SessionFailed)),
            ),
        }
    }

    /// Reads the work back from `binary.dcl`.
    ///
    /// A file written about a *different* binary is read and then said so:
    /// the notes are the reader's and are not thrown away, but an address
    /// means nothing without the bytes it points into, and a name landing on
    /// the wrong function is worse than no name at all. What to do about it is
    /// the reader's decision, so they are told rather than protected.
    pub fn open_session(&mut self) {
        let Some(analysis) = self.analysis.as_ref() else {
            self.note(
                crate::journal::Level::Warning,
                self.t(Text::SessionNoBinary),
            );
            return;
        };
        let digest = analysis.sha256;
        let path = crate::session::beside(&analysis.summary.path);
        let session = match crate::session::read(&path) {
            Ok(session) => session,
            Err(crate::session::Error::FromTheFuture { version }) => {
                self.note(
                    crate::journal::Level::Failure,
                    format!(
                        "{} (v{version}) : {}",
                        self.t(Text::SessionFromTheFuture),
                        path.display()
                    ),
                );
                return;
            }
            Err(crate::session::Error::Unreadable(why)) => {
                self.note(
                    crate::journal::Level::Failure,
                    format!("{} : {why}", self.t(Text::SessionUnreadable)),
                );
                return;
            }
            Err(crate::session::Error::Io(why)) => {
                self.note(crate::journal::Level::Failure, why);
                return;
            }
        };

        if session.belongs_to(digest) == crate::session::Belongs::ToAnother {
            self.note(
                crate::journal::Level::Warning,
                format!("{} ({})", self.t(Text::SessionOtherBinary), session.binary),
            );
        }
        self.annotations = session.notes;
        // Written back to Desdec's own store too, so the work read from the
        // file is what a later session finds without the file being opened
        // again.
        self.annotations_changed_at = Some(0.0);
        self.note(
            crate::journal::Level::Note,
            format!("{} : {}", self.t(Text::SessionOpened), path.display()),
        );
    }

    pub fn export_patched_copy(&mut self, ctx: &egui::Context) {
        if self.jobs.export_picker.is_some() || self.patches.is_empty() {
            return;
        }
        let Some(source) = self
            .analysis
            .as_ref()
            .map(|analysis| &analysis.summary.path)
        else {
            return;
        };
        let suggested = suggested_export_name(source);
        let title = self.t(Text::ExportPatched).to_owned();
        let repaint = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.jobs.export_picker = Some(receiver);
        std::thread::spawn(move || {
            let path = pollster::block_on(
                rfd::AsyncFileDialog::new()
                    .set_title(title)
                    .set_file_name(suggested)
                    .save_file(),
            )
            .map(|file| file.path().to_path_buf());
            let _ = sender.send(path);
            repaint.request_repaint();
        });
    }

    fn write_export(&mut self, destination: &Path) {
        let Some(source) = self
            .analysis
            .as_ref()
            .map(|analysis| &analysis.summary.path)
        else {
            return;
        };
        let written =
            desdec_core::patch::write_patched_copy(source, destination, self.patches.entries());
        self.export_report = match written {
            Ok(_) => {
                self.note(
                    crate::journal::Level::Note,
                    format!(
                        "{} {}",
                        self.t(Text::JournalExported),
                        destination.display()
                    ),
                );
                Some(Ok(destination.to_path_buf()))
            }
            Err(error) => {
                let failure = format!(
                    "{} {} : {error}",
                    self.t(Text::JournalExportFailed),
                    destination.display()
                );
                self.note(crate::journal::Level::Failure, failure);
                Some(Err(error.to_string()))
            }
        };
    }

    /// Writes the selected function out as NASM source, and hands it to ASM
    /// Studio when that is installed.
    ///
    /// **The file is written whether or not ASM Studio is there**, and its
    /// path is put on the clipboard either way. The export is worth having on
    /// its own — Desdec had no way at all to get assembly out of a listing —
    /// and a reader without that IDE installed still gets the file.
    ///
    /// The hand-off itself is the path on the command line, which is where
    /// this began: ASM Studio ignored its argument and opened on its start
    /// screen, so the clipboard was all there was. It reads it since
    /// 2026-08-27 and the file opens; the clipboard stays as a fallback for a
    /// build that predates that, which costs nothing and answers the case
    /// where nothing appears.
    pub fn send_to_asm_studio(&mut self, ctx: &egui::Context) {
        let Some(analysis) = self.analysis.as_ref() else {
            return;
        };
        // The function the reader is in, not the one they last clicked in
        // another view: this is asked for from the listing.
        let address = self
            .selected_instruction
            .or(self.selected_function)
            .unwrap_or_default();
        let Some(function) = self
            .functions
            .iter()
            .find(|function| (function.start..function.end).contains(&address))
            .or_else(|| {
                self.functions
                    .iter()
                    .find(|function| function.start == address)
            })
        else {
            self.note(
                crate::journal::Level::Failure,
                self.t(Text::AsmStudioNoFunction).to_owned(),
            );
            return;
        };

        let binary = analysis
            .summary
            .path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let name = sanitised_label(&function.name);
        let source = match desdec_core::export::nasm(
            function.body(analysis),
            &desdec_core::export::Source {
                binary: &binary,
                name: &name,
                architecture: analysis.summary.architecture,
            },
        ) {
            Ok(source) => source,
            Err(error) => {
                self.note(
                    crate::journal::Level::Failure,
                    format!("{} : {error}", self.t(Text::AsmStudioFailed)),
                );
                return;
            }
        };

        let path = std::env::temp_dir().join(format!("{name}.asm"));
        if let Err(error) = std::fs::write(&path, source) {
            self.note(
                crate::journal::Level::Failure,
                format!(
                    "{} {} : {error}",
                    self.t(Text::AsmStudioFailed),
                    path.display()
                ),
            );
            return;
        }
        self.note(
            crate::journal::Level::Note,
            format!("{} {}", self.t(Text::AsmStudioWrote), path.display()),
        );
        self.copy_to_clipboard(ctx, &path.to_string_lossy(), Text::AsmStudioPathCopied);

        match asm_studio_program() {
            Some(program) => {
                // Detached on purpose: an IDE is opened, not run for its
                // output, and waiting on it would freeze the listing behind it
                // for as long as the reader keeps it open.
                match ProcessCommand::new(&program).arg(&path).spawn() {
                    Ok(_) => self.note(
                        crate::journal::Level::Note,
                        format!("{} {}", self.t(Text::AsmStudioOpened), program.display()),
                    ),
                    Err(error) => self.note(
                        crate::journal::Level::Failure,
                        format!("{}: {error}", program.display()),
                    ),
                }
            }
            None => self.note(
                crate::journal::Level::Note,
                self.t(Text::AsmStudioMissing).to_owned(),
            ),
        }
    }

    /// Lends the open binary to a script, and carries out what it asked for.
    ///
    /// The analysis, the file's bytes and the reference index are moved out
    /// for the duration rather than copied — eighteen million decoded
    /// instructions are not something to duplicate because a key was pressed —
    /// and moved back before this returns, whatever the script did with them.
    pub fn run_script(
        &mut self,
        ctx: &egui::Context,
        source: &str,
        context: &crate::script::Context,
    ) -> crate::script::Outcome {
        let subject = crate::script::Subject {
            analysis: self.analysis.take(),
            file: std::mem::take(&mut self.file_bytes),
            annotations: self.annotations.clone(),
            xrefs: std::mem::take(&mut self.xrefs),
            functions: self
                .functions
                .iter()
                .map(|function| crate::script::FunctionBounds {
                    name: function.name.clone(),
                    start: function.start,
                    end: function.end,
                })
                .collect(),
        };
        let (subject, outcome) = crate::script::run(source, subject, context);
        self.analysis = subject.analysis;
        self.file_bytes = subject.file;
        self.xrefs = subject.xrefs;
        self.apply_script_effects(ctx, &outcome.effects);
        outcome
    }

    /// Carries out what a script asked for, in the order it asked.
    ///
    /// Every effect was already checked against the permissions when the
    /// script asked for it; this is where the checked list becomes the notes
    /// on the listing. A patch still lands in the pending list and not in the
    /// file — a script has no more access to the analysed binary than the
    /// interface does, which is none.
    fn apply_script_effects(&mut self, ctx: &egui::Context, effects: &[crate::script::Effect]) {
        use crate::script::Effect;

        for effect in effects {
            match effect {
                Effect::Label { address, text } => {
                    let mut annotation = self.annotations.at(*address).cloned().unwrap_or_default();
                    annotation.label.clone_from(text);
                    self.annotations.set(*address, annotation);
                }
                Effect::Comment { address, text } => {
                    let mut annotation = self.annotations.at(*address).cloned().unwrap_or_default();
                    annotation.comment.clone_from(text);
                    self.annotations.set(*address, annotation);
                }
                Effect::Bookmark { address, on } => {
                    let mut annotation = self.annotations.at(*address).cloned().unwrap_or_default();
                    annotation.bookmarked = *on;
                    self.annotations.set(*address, annotation);
                }
                Effect::ClearNote { address } => {
                    self.annotations
                        .set(*address, crate::annotations::Annotation::default());
                }
                Effect::Goto { address } => {
                    self.go_to_address(ctx, *address);
                }
                Effect::Patch { address, bytes, .. } => {
                    let made = self.analysis.as_ref().and_then(|analysis| {
                        let instruction = analysis.instruction_at(*address)?;
                        let offset = analysis.file_offset_of(*address)?;
                        desdec_core::Patch::new(
                            offset,
                            *address,
                            instruction.bytes.to_vec(),
                            bytes.clone(),
                        )
                        .ok()
                    });
                    if let Some(patch) = made {
                        self.patches.set(patch);
                    }
                }
            }
        }
    }

    /// Reads the plugin directory again, from scratch.
    ///
    /// From scratch because the reader edits plugins in a file manager and a
    /// text editor, not here: what is on disk is the truth, and a list that
    /// remembered a plugin deleted an hour ago would be lying about what is
    /// installed.
    pub fn reload_plugins(&mut self) {
        self.plugins = crate::plugins::directory()
            .map(|directory| crate::plugins::read(&directory))
            .unwrap_or_default();
    }

    /// The plugins that may run at this hook: enabled, and granted everything
    /// they currently ask for.
    fn plugins_ready_for(
        &self,
        hook: crate::plugins::Hook,
    ) -> Vec<(String, String, crate::script::Context)> {
        self.plugins
            .plugins
            .iter()
            .filter(|plugin| plugin.runs_on(hook))
            .filter_map(|plugin| {
                let consent = self.preferences.plugins.get(&plugin.id)?;
                if !consent.enabled || !consent.covers(&plugin.wanted()) {
                    return None;
                }
                Some((
                    plugin.title().to_owned(),
                    plugin.source.clone(),
                    crate::script::Context {
                        granted: consent.granted.clone(),
                        limits: crate::script::Limits::default(),
                        language: self.preferences.language,
                    },
                ))
            })
            .collect()
    }

    /// Runs the plugins that asked to see a binary as soon as it is opened.
    ///
    /// Each one is written to the session's account, whether it worked or not:
    /// a plugin that quietly renamed forty addresses, and one that quietly
    /// failed, must not look the same to the reader afterwards.
    pub fn run_plugins_on_open(&mut self, ctx: &egui::Context) {
        if self.analysis.is_none() {
            return;
        }
        for (title, source, context) in self.plugins_ready_for(crate::plugins::Hook::OnOpen) {
            let outcome = self.run_script(ctx, &source, &context);
            self.record_script_run(&title, &outcome);
        }
    }

    /// Writes one script run into the session's account.
    pub fn record_script_run(&mut self, title: &str, outcome: &crate::script::Outcome) {
        let language = self.preferences.language;
        let (level, what) = match &outcome.failure {
            Some(failure) => (
                crate::journal::Level::Warning,
                crate::ui::script::failure_text(language, failure),
            ),
            None => (
                crate::journal::Level::Note,
                format!(
                    "{} {}",
                    outcome.effects.len(),
                    text(language, Text::ScriptChangesApplied)
                ),
            ),
        };
        self.note(level, format!("{title} : {what}"));
    }

    pub fn close_binary(&mut self) {
        self.cancel_analysis();
        if self.analysis.is_some() {
            self.note(crate::journal::Level::Note, self.t(Text::JournalClosed));
        }
        self.analysis = None;
        self.error = None;
        // Dropping the receiver makes a late report for the closed file
        // harmless; it must never appear under the next binary.
        self.jobs.external_analysis = None;
        self.external_analysis = ExternalAnalysis::default();
        self.reset_file_state();
    }

    /// Clears everything that describes one particular file.
    ///
    /// Patches belong to the file they were written against: carrying them
    /// over to the next binary would offer to write bytes at offsets that mean
    /// something else entirely.
    fn reset_file_state(&mut self) {
        // The notes belong to the binary being put away, so they are written
        // while its digest is still the one this application knows.
        self.write_annotations();
        self.annotations.clear();
        self.annotations_saved.clear();
        self.annotations_last_seen.clear();
        self.annotations_changed_at = None;
        self.active_view = WorkspaceView::Overview;
        self.functions.clear();
        self.stack = desdec_core::Trace::default();
        self.string_references = crate::ui::strings::CodeReferences::default();
        self.section_starts.clear();
        self.listing_columns = crate::ui::disassembly::Columns::default();
        self.callgraph = crate::callgraph::Graph::default();
        self.xrefs = crate::xrefs::Index::default();
        self.names = crate::names::Table::default();
        // The graph is placed for one function's blocks and the watches are
        // written about one file's addresses; neither means anything for the
        // next binary opened.
        self.graph = crate::ui::graph::View::default();
        self.watches.clear();
        // The definitions are the reader's, but they were written about one
        // file's data and are kept with that file's notes: the next binary
        // opened brings back its own.
        self.structures = crate::ui::types::State::default();
        self.member_names = crate::ui::types::MemberNames::default();
        self.references_address = None;
        self.search = crate::ui::search::State::default();
        self.dump = crate::ui::dump::State::default();
        self.strings_filter.clear();
        self.symbols_filter.clear();
        self.symbols_hide_imports = false;
        self.symbols_hide_defined = false;
        self.classes_filter.clear();
        self.strings_scopes = crate::ui::strings::Scopes::EVERYTHING;
        self.strings_hide_prologues = false;
        self.selected_function = None;
        self.focused_section = None;
        self.pending_section_scroll = false;
        self.selected_instruction = None;
        self.pending_instruction_scroll = None;
        self.pseudocode_scroll = None;
        self.instruction_attention = None;
        self.walk.clear();
        // The machine is built over one file's address space. Carrying it over
        // to the next binary would offer a run through bytes that are no
        // longer there, breakpoints included.
        self.machine = None;
        self.machine_memory_at = None;
        self.pseudocode_assembly = None;
        self.dialogs.close(Dialog::Assembly);
        self.selected_string = None;
        self.patches.clear();
        self.patch_editor = None;
        self.inspecting_operand = None;
        self.explaining_library_at = None;
        self.file_bytes.clear();
        self.export_report = None;
        self.external = ExternalDecompilation::default();
        self.native = NativeDecompilation::default();
        self.yara = YaraScan::default();
        // An answer about a file that is no longer open reads as an answer
        // about the next one.
        self.assistance = Assistance {
            availability: self.assistance.availability.clone(),
            show_prompt: self.assistance.show_prompt,
            ..Assistance::default()
        };
    }

    /// Opens whatever view a command declares, doing nothing for one that
    /// opens none.
    fn open_view(&mut self, command: Command) {
        if let Some(view) = command.opens_view() {
            self.select_view(view);
        }
    }

    pub fn select_view(&mut self, view: WorkspaceView) {
        self.active_view = view;
        self.navigation_open = false;
    }

    /// Whether preferences changed since the last write to storage.
    pub fn has_unsaved_preferences(&self) -> bool {
        self.preferences != self.persisted_preferences
    }

    /// Requests the frame that will carry the automatic save.
    ///
    /// `eframe` only saves at the end of a rendered frame, and an idle window
    /// renders none. Without this, an edit made just before the application is
    /// closed — or killed — would never reach the disk.
    fn schedule_pending_save(&self, ctx: &egui::Context) {
        if self.has_unsaved_preferences() {
            ctx.request_repaint_after(AUTO_SAVE_INTERVAL);
        }
    }

    /// Writes the preferences, or clears them when persistence is disabled.
    pub fn persist_preferences(&mut self, storage: &mut dyn Storage) {
        if self.preferences.persistence_enabled {
            eframe::set_value(storage, PREFERENCES_KEY, &self.preferences);
        } else {
            storage.set_string(PREFERENCES_KEY, String::new());
        }
        self.persisted_preferences = self.preferences.clone();
    }

    /// Writes the preferences to disk once they stop changing.
    ///
    /// The host writes them on a timer of its own and at a clean shutdown, and
    /// a shutdown that is not clean loses everything since its last write —
    /// which on Windows routinely meant losing a theme chosen a moment before
    /// closing the window. This does not wait for that timer: it writes, and
    /// flushes, as soon as the preferences have held still for
    /// [`SETTLE_DELAY`].
    ///
    /// Settling matters as much as writing. Dragging the menu's edge changes a
    /// preference on every frame of the drag, and a write per frame would be
    /// hundreds of writes for one gesture; waiting for the change to stop
    /// turns the whole drag into a single write, when the reader lets go.
    pub fn persist_settled_preferences(
        &mut self,
        ctx: &egui::Context,
        storage: Option<&mut (dyn Storage + 'static)>,
    ) {
        let now = ctx.input(|input| input.time);
        if self.preferences != self.preferences_last_seen {
            self.preferences_last_seen = self.preferences.clone();
            self.preferences_changed_at = Some(now);
        }
        if !self.has_unsaved_preferences() {
            self.preferences_changed_at = None;
            return;
        }
        let Some(changed_at) = self.preferences_changed_at else {
            // Unsaved but unchanged since the last frame: nothing has moved
            // for a while, so this is a leftover from a write that could not
            // happen — storage missing at the time, most likely.
            self.preferences_changed_at = Some(now);
            return;
        };
        if now - changed_at < SETTLE_DELAY.as_secs_f64() {
            // Come back when the delay is up, even if nothing else asks for a
            // frame: an idle application must still save what it was given.
            ctx.request_repaint_after(SETTLE_DELAY);
            return;
        }
        let Some(storage) = storage else {
            return; // No storage at all: nothing to write to, and nothing to retry.
        };
        self.persist_preferences(storage);
        storage.flush();
        self.preferences_changed_at = None;
    }
}

/// Where decompiled functions are kept between runs.
///
/// The platform's cache location, which is the right home for data that can be
/// recomputed: a system may clear it, and losing it costs only the time to run
/// the engine again. Resolved from the environment rather than through a
/// dependency, since it is three rules.
#[must_use]
pub fn decompilation_cache_dir() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
    }?;
    Some(base.join("desdec").join("decompiled"))
}

/// Where ASM Studio is, when it is installed.
///
/// The `PATH` first, because that is where its own installer puts it, then the
/// place a user-local install lands. Nothing is downloaded and nothing is
/// installed: a program that is not there is reported as not there.
fn asm_studio_program() -> Option<std::path::PathBuf> {
    asm_studio_in(
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// The same, over an environment handed in.
///
/// Split out so a test can ask it about a directory it built rather than about
/// the machine the test happens to run on: this workspace forbids `unsafe`,
/// and setting an environment variable is `unsafe` since the 2024 edition.
fn asm_studio_in(
    paths: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<std::path::PathBuf> {
    const NAMES: &[&str] = &["asm-studio", "asmstudio"];
    if let Some(paths) = paths {
        for directory in std::env::split_paths(paths) {
            for name in NAMES {
                let candidate = directory.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    let home = home?;
    NAMES
        .iter()
        .map(|name| Path::new(home).join(".local/bin").join(name))
        .find(|candidate| candidate.is_file())
}

/// A function's name, cut down to something an assembler will take as a label
/// and a filesystem as a name.
///
/// A C++ method arrives here as `Parser::read(char const*)`, which is neither.
/// Everything outside the label alphabet becomes an underscore, and a name
/// that starts with a digit is given one in front: NASM reads a leading digit
/// as the start of a number.
fn sanitised_label(name: &str) -> String {
    let mut label: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    while label.ends_with('_') {
        label.pop();
    }
    if label.is_empty() {
        return "routine".to_owned();
    }
    if label.starts_with(|character: char| character.is_ascii_digit()) {
        label.insert(0, '_');
    }
    label
}

/// Default name of the exported copy: the original with `.patched` before its
/// extension, so the two never collide in a file listing.
fn suggested_export_name(source: &Path) -> String {
    let stem = source.file_stem().map_or_else(
        || "binary".to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    match source.extension() {
        Some(extension) => format!("{stem}.patched.{}", extension.to_string_lossy()),
        None => format!("{stem}.patched"),
    }
}

#[cfg(test)]
impl DesdecApp {
    /// Builds an application in a chosen state, for tests living in other
    /// modules that cannot reach the private fields.
    pub fn for_test(analysis: Option<Analysis>, active_view: WorkspaceView) -> Self {
        Self {
            // Derived exactly as opening a binary derives it, or the views
            // under test would read an index the real application would have
            // filled.
            functions: analysis
                .as_ref()
                .map(crate::ui::functions::all)
                .unwrap_or_default(),
            stack: analysis
                .as_ref()
                .map(desdec_core::Trace::of)
                .unwrap_or_default(),
            string_references: analysis
                .as_ref()
                .map(crate::ui::strings::CodeReferences::of)
                .unwrap_or_default(),
            section_starts: analysis
                .as_ref()
                .map(crate::ui::disassembly::section_starts)
                .unwrap_or_default(),
            listing_columns: analysis
                .as_ref()
                .map(|analysis| {
                    crate::ui::disassembly::Columns::of(analysis, &desdec_core::Trace::of(analysis))
                })
                .unwrap_or_default(),
            callgraph: analysis
                .as_ref()
                .map(|analysis| {
                    crate::callgraph::Graph::of(analysis, &crate::ui::functions::all(analysis))
                })
                .unwrap_or_default(),
            // The file's bytes are installed by the test fixture afterwards,
            // so the pointer half of the index is filled in there.
            xrefs: analysis
                .as_ref()
                .map(|analysis| crate::xrefs::Index::of(analysis, &[]))
                .unwrap_or_default(),
            analysis,
            active_view,
            // The question about updates is answered, so no test meets the
            // window asking it unless it opens the window itself. A test that
            // met it by accident would be a test of that window, drawn over
            // whatever it was really about.
            preferences: Preferences {
                check_for_updates: Some(false),
                ..Preferences::default()
            },
            ..Self::default()
        }
    }

    /// An application waiting on the file dialog, without opening a real one:
    /// a test must never make a native window appear.
    pub fn for_test_choosing_file() -> Self {
        let (_sender, receiver) = mpsc::channel();
        Self {
            jobs: BackgroundJobs {
                file_picker: Some(receiver),
                ..BackgroundJobs::default()
            },
            ..Self::default()
        }
    }
}

impl eframe::App for DesdecApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.run_frame(ctx);
        // Written here rather than left to `save`: see
        // [`DesdecApp::persist_settled_preferences`].
        self.persist_settled_preferences(ctx, frame.storage_mut());
    }

    fn save(&mut self, storage: &mut dyn Storage) {
        self.persist_preferences(storage);
        // The window is closing on notes that may not have settled yet.
        self.write_annotations();
    }

    fn auto_save_interval(&self) -> Duration {
        AUTO_SAVE_INTERVAL
    }
}

impl DesdecApp {
    /// One whole interface frame.
    ///
    /// Separate from [`eframe::App::update`] so tests can run the real
    /// sequence — shortcuts, dialogs, every panel in order — without an
    /// `eframe::Frame`, which only a live window can provide.
    pub fn run_frame(&mut self, ctx: &egui::Context) {
        if self.preferences.theme == ThemePreference::System {
            apply_theme(ctx, ThemePreference::System);
        }
        self.process_shortcuts(ctx);
        self.dismiss_dialog_with_escape(ctx);
        self.dismiss_dialog_clicked_outside(ctx);
        self.poll_background_jobs(ctx);
        // Asked once, ever, and only when nothing else is in front of the
        // reader; then looked for once a session, if they said yes.
        self.ask_about_updates_if_never_asked();
        self.check_for_updates_if_allowed(ctx);
        let dropped_files = ctx.input(|input| input.raw.dropped_files.clone());
        if let Some(path) = dropped_files.into_iter().find_map(|file| file.path) {
            self.inspect_binary(ctx, path);
        }

        ui::action_bar::show(self, ctx);
        ui::navigation::show(self, ctx);
        ui::status_bar::show(self, ctx);
        ui::views::show_central_panel(self, ctx);
        ui::palette::show(self, ctx);
        ui::preferences_window::show(self, ctx);
        ui::about::show(self, ctx);
        ui::library_note::show(self, ctx);
        ui::library_file::show(self, ctx);
        ui::operand_note::show(self, ctx);
        ui::annotation::show(self, ctx);
        ui::references::show(self, ctx);
        ui::search::show(self, ctx);
        ui::expression::show(self, ctx);
        ui::calculator::show(self, ctx);
        ui::external_analysis::show(self, ctx);
        ui::trace_until::show(self, ctx);
        ui::script::show(self, ctx);
        ui::plugins::show(self, ctx);
        ui::update_window::consent(self, ctx);
        ui::update_window::show(self, ctx);
        ui::output::show(self, ctx);
        self.persist_settled_annotations(ctx);
        ui::notice::show(self, ctx);

        self.schedule_pending_save(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A symbol's name is not a label. `Parser::read(char const*)` is what the
    /// symbol table holds, and NASM would refuse every character of it after
    /// the first six — as would a filesystem, for the file it names.
    #[test]
    fn a_symbol_name_is_cut_down_to_something_an_assembler_will_take() {
        assert_eq!(sanitised_label("check_password"), "check_password");
        assert_eq!(
            sanitised_label("Parser::read(char const*)"),
            "Parser__read_char_const"
        );
        // NASM reads a leading digit as the start of a number.
        assert_eq!(sanitised_label("3way"), "_3way");
        // A name that is entirely punctuation still has to name the file.
        assert_eq!(sanitised_label("***"), "routine");
        assert_eq!(sanitised_label(""), "routine");
    }

    /// Whether ASM Studio is installed is a question about the filesystem, and
    /// it must answer *no* rather than guess when the program is absent — the
    /// export is written either way, and a launch that silently fails would
    /// leave the reader waiting for a window that is never coming.
    #[test]
    fn asm_studio_is_looked_for_and_not_assumed() {
        let root = std::env::temp_dir().join(format!(
            "desdec-asm-studio-{}-{}",
            std::process::id(),
            line!()
        ));
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).expect("a directory to look in");

        assert_eq!(
            asm_studio_in(Some(bin.as_os_str()), Some(root.as_os_str())),
            None,
            "the directory is empty, so the answer is no"
        );

        let program = bin.join("asm-studio");
        std::fs::write(&program, b"#!/bin/sh\n").expect("something to find");
        assert_eq!(
            asm_studio_in(Some(bin.as_os_str()), None),
            Some(program.clone()),
            "and it is found on the PATH where its installer puts it"
        );

        // The other place a user-local install lands, when the PATH says
        // nothing.
        let local = root.join(".local/bin");
        std::fs::create_dir_all(&local).expect("a home to look in");
        std::fs::write(local.join("asm-studio"), b"#!/bin/sh\n").expect("something to find");
        assert_eq!(
            asm_studio_in(None, Some(root.as_os_str())),
            Some(local.join("asm-studio"))
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Minimal in-memory stand-in for the native RON file storage.
    #[derive(Default)]
    struct MemoryStorage {
        values: HashMap<String, String>,
        /// How many times the contents were pushed out — on the real thing,
        /// how many times the file was written.
        flushes: usize,
    }

    impl Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.values.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.values.insert(key.to_owned(), value);
        }

        fn flush(&mut self) {
            self.flushes += 1;
        }
    }

    /// Escape closes what is on top, which is whatever was opened last — not a
    /// fixed favourite among the three windows.
    #[test]
    fn escape_dismisses_dialogs_from_the_newest_to_the_oldest() {
        let mut app = DesdecApp::default();
        app.dialogs.open(Dialog::Preferences);
        app.dialogs.open(Dialog::About);
        app.dialogs.open(Dialog::CommandPalette);

        assert!(app.dismiss_topmost_dialog());
        assert!(!app.dialogs.is_open(Dialog::CommandPalette));
        assert!(app.dialogs.is_open(Dialog::About));

        assert!(app.dismiss_topmost_dialog());
        assert!(!app.dialogs.is_open(Dialog::About));
        assert!(app.dialogs.is_open(Dialog::Preferences));

        assert!(app.dismiss_topmost_dialog());
        assert!(!app.dialogs.is_open(Dialog::Preferences));

        assert!(!app.dismiss_topmost_dialog());
    }

    #[test]
    fn disassembly_start_commands_select_and_remember_the_requested_landmark() {
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        let expected = app.disassembly_start(DisassemblyStart::ProbableFunction);

        app.select_disassembly_start(DisassemblyStart::ProbableFunction);

        assert_eq!(
            app.preferences.disassembly_start,
            DisassemblyStart::ProbableFunction
        );
        assert_eq!(app.selected_instruction, expected);
        assert_eq!(app.pending_instruction_scroll, expected);
        if expected.is_some() {
            assert_eq!(app.active_view, WorkspaceView::Disassembly);
        }
    }

    /// A press on the workspace behind the windows closes the topmost one, and
    /// a press inside a window leaves it alone — otherwise reading the thing
    /// the window is there to show would shut it.
    #[test]
    fn a_press_outside_a_dialog_closes_it_and_a_press_inside_does_not() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.dialogs.open(Dialog::About);
        // One frame so the window is laid out and its layer known.
        let _ = ctx.run(crate::testing::window_input(), |ctx| app.run_frame(ctx));
        let window = ctx
            .memory(|memory| memory.area_rect(egui::Id::new("desdec.about")))
            .expect("the About window was laid out");

        let _ = ctx.run(crate::testing::press_at(window.center()), |ctx| {
            app.run_frame(ctx);
        });
        assert!(
            app.dialogs.is_open(Dialog::About),
            "a press inside the window must not close it"
        );

        let outside = egui::pos2(window.right() + 40.0, window.center().y);
        let _ = ctx.run(crate::testing::press_at(outside), |ctx| app.run_frame(ctx));
        assert!(
            !app.dialogs.is_open(Dialog::About),
            "a press at {outside:?}, outside {window:?}, must close it"
        );
    }

    /// The toolbar's palette button sits behind the palette it opens, so one
    /// press both closes the window and reaches the button. The button must
    /// not undo the closing, or the palette could never be shut from it.
    #[test]
    fn the_press_that_closes_a_window_is_not_the_one_that_reopens_it() {
        let mut app = DesdecApp::default();
        app.dialogs.open(Dialog::CommandPalette);

        // The press lands on the toolbar behind the palette …
        app.dialogs.dismissed_by_press = app.dialogs.dismiss_topmost();
        // … and then reaches the button under it.
        app.dialogs.toggle(Dialog::CommandPalette);
        assert!(!app.dialogs.is_open(Dialog::CommandPalette));

        // The next press is free to open it again.
        app.dialogs.dismissed_by_press = None;
        app.dialogs.toggle(Dialog::CommandPalette);
        assert!(app.dialogs.is_open(Dialog::CommandPalette));
    }

    /// A second window opens beside the first rather than exactly on top of
    /// it: two windows in the same corner leave a stack the reader cannot see
    /// into, and no way to reach the one underneath.
    #[test]
    fn a_second_window_opens_clear_of_the_first() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);

        app.dialogs.open(Dialog::Preferences);
        let _ = ctx.run(crate::testing::window_input(), |ctx| app.run_frame(ctx));
        app.dialogs.open(Dialog::About);
        // Twice: the first opening has no measured size to be placed by.
        let _ = ctx.run(crate::testing::window_input(), |ctx| app.run_frame(ctx));
        let _ = ctx.run(crate::testing::window_input(), |ctx| app.run_frame(ctx));

        let rect = |id| {
            ctx.memory(|memory| memory.area_rect(egui::Id::new(id)))
                .expect("the window was laid out")
        };
        let preferences = rect("desdec.preferences");
        let about = rect("desdec.about");
        assert!(
            about.left_top().y - preferences.left_top().y >= 30.0
                || about.left_top().x - preferences.left_top().x >= 30.0,
            "About ({about:?}) opened on top of the preferences ({preferences:?})"
        );
    }

    /// Reopening a dialog puts it back on top of the ones already open.
    #[test]
    fn a_reopened_dialog_becomes_the_topmost_one() {
        let mut app = DesdecApp::default();
        app.dialogs.open(Dialog::About);
        app.dialogs.open(Dialog::Preferences);

        app.dialogs.close(Dialog::About);
        app.dialogs.open(Dialog::About);

        assert!(app.dismiss_topmost_dialog());
        assert!(!app.dialogs.is_open(Dialog::About));
        assert!(app.dialogs.is_open(Dialog::Preferences));
    }

    /// A cached answer must be found again for the same binary and function,
    /// and never for another. This drives the real application state rather
    /// than the cache module alone, so the key it builds is exercised too.
    #[test]
    fn a_cached_function_is_reused_and_never_confused_with_another() {
        let digest = crate::testing::reference_analysis()
            .sha256
            .expect("a whole file has a digest");

        let mut app = crate::testing::opened_app(WorkspaceView::Decompile);
        app.preferences.decompiler = DecompilerPreference::RzGhidra;
        let (directory, keyed) = app.cache_target().expect("caching is on by default");
        assert_eq!(keyed, digest, "the binary's digest is what keys the cache");

        let engine = decompiler::Engine::RzGhidra;
        let key = |address| decompiler::cache::Key {
            digest: &digest,
            engine,
            address,
        };
        // A directory of this test's own, so a real cache is never touched.
        let directory = directory.join("test-scratch");
        let _ = std::fs::remove_dir_all(&directory);
        decompiler::cache::write(&directory, key(Some(0x1234)), "void f(void) {}")
            .expect("the cache is writable");

        assert_eq!(
            decompiler::cache::read(&directory, key(Some(0x1234))),
            Some("void f(void) {}".to_owned())
        );
        assert_eq!(
            decompiler::cache::read(&directory, key(Some(0x5678))),
            None,
            "another function must not read this one's answer"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// Nothing is cached when the digest is unknown, which is the case for a
    /// file too large to be read whole: keying on anything weaker could show
    /// one binary's decompilation for another.
    #[test]
    fn a_binary_without_a_digest_is_never_cached() {
        let mut analysis = crate::testing::reference_analysis().clone();
        analysis.sha256 = None;
        analysis.truncated = true;

        let app = DesdecApp::for_test(Some(analysis), WorkspaceView::Decompile);

        assert!(app.cache_target().is_none());
    }

    #[test]
    fn turning_the_cache_off_stops_it_being_used() {
        let mut app = crate::testing::opened_app(WorkspaceView::Decompile);

        assert!(app.cache_target().is_some());
        app.preferences.cache_decompilations = false;
        assert!(app.cache_target().is_none());
    }

    /// Opening the file dialog is not an analysis. The status bar used to
    /// announce one the moment the dialog appeared, for a file the user had
    /// not chosen yet — and might never choose.
    #[test]
    fn cancelling_an_analysis_drops_its_result_and_signals_its_worker() {
        let (_sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut app = DesdecApp {
            jobs: BackgroundJobs {
                inspection: Some(InspectionJob {
                    receiver,
                    cancelled: Arc::clone(&cancelled),
                }),
                ..BackgroundJobs::default()
            },
            ..DesdecApp::default()
        };

        app.cancel_analysis();

        assert!(cancelled.load(Ordering::Relaxed));
        assert!(!app.is_analysing());
    }

    /// Cancelling must leave somewhere to go. A view chosen while the first
    /// binary was loading describes a file that never arrived, and stranded
    /// the reader on a screen with no way to open another one.
    #[test]
    fn cancelling_the_first_analysis_returns_to_a_view_that_can_open_a_binary() {
        let (_sender, receiver) = mpsc::channel();
        let mut app = DesdecApp {
            jobs: BackgroundJobs {
                inspection: Some(InspectionJob {
                    receiver,
                    cancelled: Arc::new(AtomicBool::new(false)),
                }),
                ..BackgroundJobs::default()
            },
            active_view: WorkspaceView::Disassembly,
            ..DesdecApp::default()
        };

        app.cancel_analysis();

        assert!(app.analysis.is_none());
        assert_eq!(app.active_view, WorkspaceView::Overview);
    }

    /// Cancelling one analysis while another binary is open must not throw the
    /// reader out of the view they were in: that file is still loaded.
    #[test]
    fn cancelling_keeps_the_view_when_a_binary_is_still_open() {
        let (_sender, receiver) = mpsc::channel();
        let mut app = crate::testing::opened_app(WorkspaceView::Disassembly);
        app.jobs.inspection = Some(InspectionJob {
            receiver,
            cancelled: Arc::new(AtomicBool::new(false)),
        });

        app.cancel_analysis();

        assert_eq!(app.active_view, WorkspaceView::Disassembly);
    }

    /// Copying puts the value on the clipboard and says so: the bytes go
    /// somewhere the application cannot show, so silence would leave a
    /// successful copy and a missed click looking identical. The message names
    /// the value, so the reader can see it is the one they meant.
    #[test]
    fn copying_confirms_itself() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::default();
        assert!(app.notice.is_none());

        app.copy_to_clipboard(&ctx, "/tmp/example.bin", Text::PathCopied);

        let notice = app.notice.as_ref().expect("copying says so");
        assert!(
            notice.text.contains(app.t(Text::PathCopied)),
            "{}",
            notice.text
        );
        assert!(notice.text.contains("/tmp/example.bin"), "{}", notice.text);
    }

    /// A file dialog the desktop never answers used to leave the application
    /// waiting on it for the rest of the session: nothing on screen abandoned
    /// it, and every later request to open a binary was refused in silence.
    #[test]
    fn the_choice_of_a_file_can_be_abandoned() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test_choosing_file();
        assert!(app.is_opening(), "an opening is under way");
        assert!(
            app.can_run(Command::CancelAnalysis),
            "abandoning it must be offered"
        );
        let output = ctx.run(crate::testing::window_input(), |ctx| {
            crate::ui::views::show_central_panel(&mut app, ctx);
            crate::ui::status_bar::show(&mut app, ctx);
        });
        assert!(
            crate::testing::drawn_text(&output.shapes).contains(app.t(Text::CancelChoosing)),
            "the workspace must offer a way out of the dialog"
        );

        app.run_command(&ctx, Command::CancelAnalysis);

        assert!(!app.is_opening());
        assert!(
            app.can_choose_binary(),
            "abandoning must free the application to open something else"
        );
    }

    #[test]
    fn choosing_a_file_is_not_reported_as_an_analysis() {
        let app = DesdecApp::for_test_choosing_file();

        assert!(app.is_choosing_file(), "the dialog is open");
        assert!(
            !app.is_analysing(),
            "nothing is analysed while the dialog waits on the user"
        );
    }

    /// Closing must be reachable without hunting through the collapsed menu,
    /// so the action bar carries it whenever a binary is open.
    #[test]
    fn a_loaded_binary_can_be_closed_from_the_keyboard() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Disassembly);

        app.run_command(&ctx, Command::CloseBinary);

        assert!(app.analysis.is_none());
        assert!(app.patches.is_empty(), "patches belong to the closed file");
        assert_eq!(app.active_view, WorkspaceView::Overview);
    }

    /// The function index is derived from the analysis, so opening a binary
    /// must fill it and closing one must drop it: a stale index would describe
    /// a file that is no longer open.
    ///
    /// Which binary names functions is a question about the *format*, not about
    /// this code, so the filling is asserted on the fixtures — each declares
    /// the functions it carries — and the host's own binary is held only to
    /// what is true of any of them. A Windows executable exports nothing: its
    /// symbol table is an import table, every entry of it belonging to another
    /// file, and an empty index is the correct answer there rather than a
    /// failure to build one.
    #[test]
    fn opening_a_binary_indexes_its_functions_and_closing_forgets_them() {
        for sample in crate::testing::samples() {
            let mut app = DesdecApp::default();
            app.apply_inspection(
                Path::new("fixture.bin"),
                Ok(PreparedInspection::of(AnalysedFile {
                    analysis: sample.analysis.clone(),
                    bytes: sample.fixture.bytes.clone(),
                })),
            );
            assert!(
                !app.functions.is_empty(),
                "{} declares {} functions and none was indexed",
                sample.fixture.label,
                sample.fixture.functions.len()
            );
            app.close_binary();
            assert!(app.functions.is_empty(), "{}", sample.fixture.label);
        }

        // The host's binary, for the one thing that holds whatever it is: an
        // index built from a listing must point inside the listing it indexes.
        let mut app = DesdecApp::default();
        app.apply_inspection(
            crate::testing::reference_path(),
            Ok(PreparedInspection::of(AnalysedFile {
                analysis: crate::testing::reference_analysis().clone(),
                bytes: crate::testing::reference_bytes().to_vec(),
            })),
        );
        let listing = app
            .analysis
            .as_ref()
            .expect("the analysis was installed")
            .instructions
            .len();
        assert!(
            app.functions
                .iter()
                .all(|function| function.instructions.end <= listing),
            "every indexed body must point inside the listing it indexes"
        );
        app.close_binary();
        assert!(app.functions.is_empty());
    }

    #[test]
    fn yara_module_is_toggled_and_its_view_is_opened_by_commands() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::default();

        app.run_command(&ctx, Command::ToggleYaraModule);
        assert!(app.preferences.yara_enabled);
        app.run_command(&ctx, Command::Yara);
        assert_eq!(app.active_view, WorkspaceView::Yara);
        app.run_command(&ctx, Command::ToggleYaraModule);
        assert!(!app.preferences.yara_enabled);
    }

    #[test]
    fn command_palette_command_toggles_the_palette() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::default();

        app.run_command(&ctx, Command::CommandPalette);
        assert!(app.dialogs.is_open(Dialog::CommandPalette));

        app.run_command(&ctx, Command::CommandPalette);
        assert!(!app.dialogs.is_open(Dialog::CommandPalette));
    }

    #[test]
    fn escape_leaves_a_shortcut_capture_before_closing_its_dialog() {
        let mut app = DesdecApp {
            editing_shortcut: Some(Command::OpenBinary),
            ..Default::default()
        };
        app.dialogs.open(Dialog::Preferences);

        app.dismiss_topmost_dialog();
        assert!(app.editing_shortcut.is_none());
        assert!(app.dialogs.is_open(Dialog::Preferences));
    }

    #[test]
    fn recent_binaries_are_deduplicated_bounded_and_clearable() {
        let mut app = DesdecApp::default();
        for index in 0..=RECENT_BINARY_LIMIT {
            app.remember_recent_binary(&PathBuf::from(format!("binary-{index}")));
        }
        // Reopening an existing item puts it first instead of adding a copy.
        app.remember_recent_binary(&PathBuf::from("binary-4"));

        assert_eq!(app.recent_binaries().len(), RECENT_BINARY_LIMIT);
        assert_eq!(app.recent_binaries()[0], PathBuf::from("binary-4"));
        assert_eq!(
            app.recent_binaries()
                .iter()
                .filter(|path| *path == &PathBuf::from("binary-4"))
                .count(),
            1
        );

        app.clear_recent_binaries();
        assert!(app.recent_binaries().is_empty());
    }

    #[test]
    fn a_fresh_app_has_nothing_to_save() {
        assert!(!DesdecApp::default().has_unsaved_preferences());
    }

    #[test]
    fn editing_a_preference_schedules_a_save_until_it_is_written() {
        let mut app = DesdecApp::default();
        app.preferences.theme = ThemePreference::Catppuccin;
        assert!(app.has_unsaved_preferences());

        let mut storage = MemoryStorage::default();
        app.persist_preferences(&mut storage);
        assert!(!app.has_unsaved_preferences());

        let restored: Preferences = eframe::get_value(&storage, PREFERENCES_KEY)
            .expect("preferences should be readable back");
        assert_eq!(restored.theme, ThemePreference::Catppuccin);
    }

    #[test]
    fn disabling_persistence_clears_the_stored_preferences() {
        let mut storage = MemoryStorage::default();
        let mut app = DesdecApp::default();
        app.preferences.theme = ThemePreference::Dark;
        app.persist_preferences(&mut storage);

        app.preferences.persistence_enabled = false;
        app.persist_preferences(&mut storage);

        assert_eq!(storage.get_string(PREFERENCES_KEY), Some(String::new()));
        assert!(eframe::get_value::<Preferences>(&storage, PREFERENCES_KEY).is_none());
        assert!(!app.has_unsaved_preferences());
    }

    /// Reports how long egui waits before the next frame once the interface has
    /// settled. `Duration::MAX` means "only when something happens", which is
    /// the idle state where an automatic save would never run.
    fn settled_repaint_delay(ctx: &egui::Context, app: &DesdecApp) -> Duration {
        // The first frames of a context always ask to be repainted; run a few
        // so the measurement reflects a settled window.
        let mut delay = Duration::ZERO;
        for _ in 0..4 {
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                app.schedule_pending_save(ctx);
            });
            delay = output
                .viewport_output
                .values()
                .map(|viewport| viewport.repaint_delay)
                .min()
                .expect("a frame always produces one viewport");
        }
        delay
    }

    #[test]
    fn an_edited_preference_schedules_the_frame_that_saves_it() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::default();
        assert_eq!(
            settled_repaint_delay(&ctx, &app),
            Duration::MAX,
            "an untouched application should stay idle"
        );

        app.preferences.theme = ThemePreference::Dark;
        assert!(
            settled_repaint_delay(&ctx, &app) <= AUTO_SAVE_INTERVAL,
            "an edited preference should schedule the frame that saves it"
        );

        let mut storage = MemoryStorage::default();
        app.persist_preferences(&mut storage);
        assert_eq!(
            settled_repaint_delay(&ctx, &app),
            Duration::MAX,
            "once written, the application should go idle again"
        );
    }

    /// Runs the frames of one stretch of time, as the interface really would.
    ///
    /// Time is the application's own clock, so it has to run forward across
    /// the whole test: a fresh context starting at zero would place the last
    /// change in the future and nothing would ever settle.
    fn frames(
        app: &mut DesdecApp,
        storage: &mut MemoryStorage,
        ctx: &egui::Context,
        from: f64,
        to: f64,
    ) -> f64 {
        let mut time = from;
        while time <= to {
            let input = egui::RawInput {
                time: Some(time),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                app.persist_settled_preferences(ctx, Some(storage));
            });
            time += 0.1;
        }
        time
    }

    /// A preference must reach the disk on its own, shortly after it is set,
    /// without waiting for the host's auto-save or a clean shutdown: on
    /// Windows that wait routinely lost a theme chosen a moment before the
    /// window was closed.
    #[test]
    fn a_changed_preference_is_written_shortly_after_it_settles() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::default();
        let mut storage = MemoryStorage::default();
        app.preferences.theme = ThemePreference::Catppuccin;

        frames(
            &mut app,
            &mut storage,
            &ctx,
            0.0,
            SETTLE_DELAY.as_secs_f64() * 2.0,
        );

        assert!(!app.has_unsaved_preferences(), "the change was not written");
        assert_eq!(storage.flushes, 1, "the write must reach the disk, once");
        let written: Preferences =
            eframe::get_value(&storage, PREFERENCES_KEY).expect("preferences were stored");
        assert_eq!(written.theme, ThemePreference::Catppuccin);
    }

    /// A gesture that changes a preference on every frame — dragging the
    /// menu's edge — must cost one write, not one per frame.
    #[test]
    fn a_continuous_change_is_written_once_when_it_stops() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::default();
        let mut storage = MemoryStorage::default();

        let mut time = 0.0;
        for width in 120..160 {
            app.preferences.navigation_width = width;
            let input = egui::RawInput {
                time: Some(time),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                app.persist_settled_preferences(ctx, Some(&mut storage));
            });
            time += 0.05;
        }
        assert_eq!(storage.flushes, 0, "nothing is written mid-gesture");

        frames(
            &mut app,
            &mut storage,
            &ctx,
            time,
            time + SETTLE_DELAY.as_secs_f64() * 2.0,
        );
        assert_eq!(storage.flushes, 1, "the finished gesture is one write");
        assert!(!app.has_unsaved_preferences());
    }

    /// Views backed by real data must not announce themselves as planned, and
    /// planned ones must always carry an explanation.
    #[test]
    fn every_view_either_shows_data_or_explains_itself() {
        const IMPLEMENTED: &[WorkspaceView] = &[
            WorkspaceView::Overview,
            WorkspaceView::Segments,
            WorkspaceView::Functions,
            WorkspaceView::Disassembly,
            WorkspaceView::Decompile,
            WorkspaceView::Dump,
            WorkspaceView::Strings,
            WorkspaceView::Symbols,
            WorkspaceView::Classes,
            WorkspaceView::Assistant,
            WorkspaceView::Machine,
            WorkspaceView::Graph,
            WorkspaceView::Structures,
            WorkspaceView::Patches,
            WorkspaceView::Yara,
        ];

        for view in WorkspaceView::ALL {
            assert_eq!(
                view.planned_explanation().is_none(),
                IMPLEMENTED.contains(view),
                "{view:?} is inconsistent"
            );
        }
    }
}
