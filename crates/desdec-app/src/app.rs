use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::Duration,
};

use desdec_core::{Analysis, analyse_path, decompiler};
use eframe::{Storage, egui};

use crate::{
    commands::{Command, Shortcut},
    i18n::{Language, Text, text},
    patches::{Editor, Patches},
    preferences::{DecompilerPreference, Preferences, ThemePreference, apply_theme},
    ui::{self, preferences_window::PreferencesTab},
};

pub const PREFERENCES_KEY: &str = "desdec.preferences";

/// How long an edited preference may stay in memory before `eframe` writes it
/// to disk.
///
/// `eframe` only persists during an automatic save or a clean shutdown, and a
/// shutdown that is not clean — a forced close, a driver reset, a session that
/// ends with the window still open — loses everything since the last one. The
/// default interval of 30 seconds made that window wide enough to routinely
/// lose a theme change on Windows, so we shorten it and, in [`DesdecApp::update`],
/// schedule a repaint whenever preferences differ from the saved snapshot: the
/// frame that follows triggers the save even if the application is otherwise
/// idle. Storage is only written when a value actually changed, so a short
/// interval costs nothing while nothing is edited.
pub const AUTO_SAVE_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WorkspaceView {
    #[default]
    Overview,
    Segments,
    Functions,
    Strings,
    Disassembly,
    Decompile,
    Patches,
}

impl WorkspaceView {
    pub const ALL: &[Self] = &[
        Self::Overview,
        Self::Segments,
        Self::Functions,
        Self::Strings,
        Self::Disassembly,
        Self::Decompile,
        Self::Patches,
    ];

    pub const fn text(self) -> Text {
        match self {
            Self::Overview => Text::Overview,
            Self::Segments => Text::Segments,
            Self::Functions => Text::Functions,
            Self::Strings => Text::Strings,
            Self::Disassembly => Text::Disassembly,
            Self::Decompile => Text::Decompile,
            Self::Patches => Text::Patches,
        }
    }

    /// Short label used by the navigation menu, chosen because the native font
    /// does not cover every pictogram uniformly.
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Overview => "ACC",
            Self::Segments => "SEG",
            Self::Functions => "Fn",
            Self::Strings => "STR",
            Self::Disassembly => "ASM",
            Self::Decompile => "DEC",
            Self::Patches => "PATCH",
        }
    }

    /// What a not-yet-implemented view announces. `None` for views that already
    /// show real data.
    pub const fn planned_explanation(self) -> Option<Text> {
        match self {
            Self::Overview | Self::Segments | Self::Functions | Self::Strings => None,
            Self::Disassembly | Self::Decompile | Self::Patches => None,
        }
    }
}

const DIALOG_COUNT: usize = 3;

/// Modal windows. Each one is opened by simply setting its flag, from wherever
/// in the interface; [`Dialogs::track_openings`] turns those flags into an
/// order so `Escape` closes the window the user is actually looking at.
#[derive(Default)]
pub struct Dialogs {
    pub command_palette: bool,
    pub preferences: bool,
    pub about: bool,
    /// Opening rank of each dialog, the highest being the topmost.
    ranks: [u64; DIALOG_COUNT],
    /// Flags as of the previous frame, to spot the ones that just opened.
    was_open: [bool; DIALOG_COUNT],
    clock: u64,
}

impl Dialogs {
    fn flags(&self) -> [bool; DIALOG_COUNT] {
        [self.command_palette, self.preferences, self.about]
    }

    fn flag_mut(&mut self, index: usize) -> &mut bool {
        match index {
            0 => &mut self.command_palette,
            1 => &mut self.preferences,
            _ => &mut self.about,
        }
    }

    /// Stamps every dialog opened since the last frame.
    fn track_openings(&mut self) {
        let open = self.flags();
        for (index, opened) in open.iter().enumerate() {
            if *opened && !self.was_open[index] {
                self.clock += 1;
                self.ranks[index] = self.clock;
            }
        }
        self.was_open = open;
    }

    /// Closes the most recently opened dialog and reports whether one was
    /// closed.
    fn dismiss_topmost(&mut self) -> bool {
        let open = self.flags();
        let topmost = (0..DIALOG_COUNT)
            .filter(|index| open[*index])
            .max_by_key(|index| self.ranks[*index]);
        let Some(index) = topmost else {
            return false;
        };
        *self.flag_mut(index) = false;
        self.was_open[index] = false;
        true
    }
}

#[derive(Default)]
pub struct PaletteState {
    pub query: String,
    pub selected: usize,
}

/// Work handed to background threads so the interface never blocks on the file
/// system or on a native dialog.
#[derive(Default)]
struct BackgroundJobs {
    file_picker: Option<Receiver<Option<PathBuf>>>,
    inspection: Option<Receiver<(PathBuf, std::io::Result<Analysis>)>>,
    /// An external decompiler, which can take a minute on a large binary.
    decompilation: Option<Receiver<std::io::Result<String>>>,
    /// Where the user chose to export the patched copy.
    export_picker: Option<Receiver<Option<PathBuf>>>,
}

/// Result of the external decompiler, when one is selected.
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

#[derive(Default)]
pub struct DesdecApp {
    /// Result of the deep analysis of the loaded binary.
    pub analysis: Option<Analysis>,
    pub error: Option<String>,
    pub active_view: WorkspaceView,
    pub navigation_open: bool,
    pub dialogs: Dialogs,
    pub preferences: Preferences,
    pub preferences_tab: PreferencesTab,
    pub editing_shortcut: Option<Command>,
    pub palette: PaletteState,
    /// Address of the function currently inspected in the Functions view.
    pub selected_function: Option<u64>,
    /// Instruction selected in the disassembly or local pseudo-code view.
    pub selected_instruction: Option<u64>,
    /// Instruction that must be brought into view after a new selection.
    pub pending_instruction_scroll: Option<u64>,
    /// Temporarily draws attention to an instruction reached from another view.
    pub instruction_attention: Option<(u64, f64)>,
    /// File offset of the string inspected in the Strings view.
    pub selected_string: Option<u64>,
    /// Free-text filter applied to the extracted strings.
    pub strings_filter: String,
    /// Pending byte patches, and the instruction being edited.
    pub patches: Patches,
    pub patch_editor: Option<Editor>,
    /// Outcome of the last export, kept until the next one.
    pub export_report: Option<Result<PathBuf, String>>,
    /// How many cache entries the last clear removed.
    pub cache_report: Option<usize>,
    /// Text produced by the selected external decompiler.
    pub external: ExternalDecompilation,
    /// What was found for each engine, and for which configured path.
    ///
    /// Detecting an engine touches the file system, and `rz-ghidra` is probed
    /// by running `rizin`: doing that every frame while the preferences window
    /// is open would spawn a process sixty times a second.
    engine_availability: HashMap<&'static str, (String, decompiler::Availability)>,
    jobs: BackgroundJobs,
    /// Last state handed to storage, used to detect unsaved preferences.
    persisted_preferences: Preferences,
}

impl DesdecApp {
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        let preferences: Preferences = creation_context
            .storage
            .and_then(|storage| eframe::get_value(storage, PREFERENCES_KEY))
            .unwrap_or_default();
        apply_theme(&creation_context.egui_ctx, preferences.theme);
        let mut app = Self {
            persisted_preferences: preferences.clone(),
            preferences,
            ..Self::default()
        };

        // `desdec-app <binary>` starts the analysis straight away, like a file
        // manager handing the application a file to open.
        if let Some(path) = std::env::args_os().nth(1) {
            app.inspect_binary(&creation_context.egui_ctx, PathBuf::from(path));
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
        if command.needs_a_binary() && self.analysis.is_none() {
            return false;
        }
        if command.needs_patches() && self.patches.is_empty() {
            return false;
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

    pub fn run_command(&mut self, ctx: &egui::Context, command: Command) {
        match command {
            Command::OpenBinary => self.choose_binary(ctx),
            Command::CloseBinary => self.close_binary(),
            Command::ToggleNavigation => self.navigation_open = !self.navigation_open,
            Command::ToggleToolbar => {
                self.preferences.show_toolbar = !self.preferences.show_toolbar;
            }
            Command::ToggleTooltips => {
                self.preferences.show_tooltips = !self.preferences.show_tooltips;
            }
            Command::CommandPalette => {
                self.dialogs.command_palette = !self.dialogs.command_palette;
                if self.dialogs.command_palette {
                    self.palette = PaletteState::default();
                }
            }
            Command::Preferences => self.dialogs.preferences = true,
            Command::About => self.dialogs.about = true,
            Command::Overview => self.select_view(WorkspaceView::Overview),
            Command::Segments => self.select_view(WorkspaceView::Segments),
            Command::ExportPatched => {
                self.select_view(WorkspaceView::Patches);
                self.export_patched_copy(ctx);
            }
            Command::DiscardPatches => {
                self.patches.clear();
                self.patch_editor = None;
                self.export_report = None;
            }
            Command::DecompilerBuiltin => self.set_decompiler(DecompilerPreference::Builtin),
            Command::DecompilerRzGhidra => self.set_decompiler(DecompilerPreference::RzGhidra),
            Command::DecompilerRetDec => self.set_decompiler(DecompilerPreference::RetDec),
            Command::ToggleDecompilationCache => {
                self.preferences.cache_decompilations = !self.preferences.cache_decompilations;
            }
            Command::ClearDecompilationCache => {
                self.cache_report = decompilation_cache_dir()
                    .and_then(|directory| decompiler::cache::clear(&directory).ok());
            }
            Command::Disassembly => self.select_view(WorkspaceView::Disassembly),
            Command::Decompile => self.select_view(WorkspaceView::Decompile),
            Command::AiAssistance => {}
            Command::Functions => self.select_view(WorkspaceView::Functions),
            Command::Strings => self.select_view(WorkspaceView::Strings),
            Command::Patches => self.select_view(WorkspaceView::Patches),
            Command::ThemeSystem => self.set_theme(ctx, ThemePreference::System),
            Command::ThemeDark => self.set_theme(ctx, ThemePreference::Dark),
            Command::ThemeLight => self.set_theme(ctx, ThemePreference::Light),
            Command::ThemeCatppuccin => self.set_theme(ctx, ThemePreference::Catppuccin),
            Command::LanguageFrench => self.preferences.language = Language::French,
            Command::LanguageEnglish => self.preferences.language = Language::English,
            Command::LanguageSpanish => self.preferences.language = Language::Spanish,
            Command::TogglePersistence => {
                self.preferences.persistence_enabled = !self.preferences.persistence_enabled;
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
        self.dialogs.dismiss_topmost()
    }

    fn dismiss_dialog_with_escape(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.dismiss_topmost_dialog();
        }
    }

    pub fn choose_binary(&mut self, ctx: &egui::Context) {
        if self.jobs.file_picker.is_some() || self.jobs.inspection.is_some() {
            return;
        }

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
        let repaint = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.jobs.inspection = Some(receiver);
        std::thread::spawn(move || {
            let result = analyse_path(&path);
            let _ = sender.send((path, result));
            repaint.request_repaint();
        });
    }

    fn apply_inspection(&mut self, path: &Path, result: std::io::Result<Analysis>) {
        match result {
            Ok(analysis) => {
                self.analysis = Some(analysis);
                self.error = None;
                self.reset_file_state();
            }
            Err(error) => {
                self.error = Some(format!(
                    "{} {}: {error}",
                    self.t(Text::CannotInspect),
                    path.display()
                ));
            }
        }
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

        let inspection = self.jobs.inspection.as_ref().map(Receiver::try_recv);
        match inspection {
            Some(Ok((path, result))) => {
                self.jobs.inspection = None;
                self.apply_inspection(&path, result);
            }
            Some(Err(TryRecvError::Disconnected)) => self.jobs.inspection = None,
            Some(Err(TryRecvError::Empty)) | None => {}
        }

        let decompilation = self.jobs.decompilation.as_ref().map(Receiver::try_recv);
        match decompilation {
            Some(Ok(result)) => {
                self.jobs.decompilation = None;
                self.external.running = false;
                match result {
                    Ok(text) => self.external.text = Some(text),
                    Err(error) => self.external.error = Some(error.to_string()),
                }
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.jobs.decompilation = None;
                self.external.running = false;
            }
            Some(Err(TryRecvError::Empty)) | None => {}
        }

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

    /// Whether a binary is being analysed right now.
    ///
    /// Deliberately not "is something running": while the file dialog is open
    /// nothing is being analysed, and saying otherwise announced an analysis
    /// of a file the user had not chosen yet — and often would never choose.
    pub const fn is_analysing(&self) -> bool {
        self.jobs.inspection.is_some()
    }

    /// Whether the file dialog is open, waiting on the user.
    pub const fn is_choosing_file(&self) -> bool {
        self.jobs.file_picker.is_some()
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
        if let Some(cached) = self.cached_decompilation(engine, address) {
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

    /// Where this binary's answers are kept, and the digest that identifies
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
        self.export_report = match desdec_core::patch::write_patched_copy(
            source,
            destination,
            self.patches.entries(),
        ) {
            Ok(_) => Some(Ok(destination.to_path_buf())),
            Err(error) => Some(Err(error.to_string())),
        };
    }

    pub fn close_binary(&mut self) {
        self.analysis = None;
        self.error = None;
        self.reset_file_state();
    }

    /// Clears everything that describes one particular file.
    ///
    /// Patches belong to the file they were written against: carrying them
    /// over to the next binary would offer to write bytes at offsets that mean
    /// something else entirely.
    fn reset_file_state(&mut self) {
        self.active_view = WorkspaceView::Overview;
        self.strings_filter.clear();
        self.selected_function = None;
        self.selected_instruction = None;
        self.pending_instruction_scroll = None;
        self.instruction_attention = None;
        self.selected_string = None;
        self.patches.clear();
        self.patch_editor = None;
        self.export_report = None;
        self.external = ExternalDecompilation::default();
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
            analysis,
            active_view,
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.run_frame(ctx);
    }

    fn save(&mut self, storage: &mut dyn Storage) {
        self.persist_preferences(storage);
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
        self.dialogs.track_openings();
        self.dismiss_dialog_with_escape(ctx);
        self.poll_background_jobs(ctx);
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

        self.schedule_pending_save(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Minimal in-memory stand-in for the native RON file storage.
    #[derive(Default)]
    struct MemoryStorage {
        values: HashMap<String, String>,
    }

    impl Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.values.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.values.insert(key.to_owned(), value);
        }

        fn flush(&mut self) {}
    }

    /// Escape closes what is on top, which is whatever was opened last — not a
    /// fixed favourite among the three windows.
    #[test]
    fn escape_dismisses_dialogs_from_the_newest_to_the_oldest() {
        let mut app = DesdecApp::default();
        app.dialogs.preferences = true;
        app.dialogs.track_openings();
        app.dialogs.about = true;
        app.dialogs.track_openings();
        app.dialogs.command_palette = true;
        app.dialogs.track_openings();

        assert!(app.dismiss_topmost_dialog());
        assert!(!app.dialogs.command_palette);
        assert!(app.dialogs.about);

        assert!(app.dismiss_topmost_dialog());
        assert!(!app.dialogs.about);
        assert!(app.dialogs.preferences);

        assert!(app.dismiss_topmost_dialog());
        assert!(!app.dialogs.preferences);

        assert!(!app.dismiss_topmost_dialog());
    }

    /// Reopening a dialog puts it back on top of the ones already open.
    #[test]
    fn a_reopened_dialog_becomes_the_topmost_one() {
        let mut app = DesdecApp::default();
        app.dialogs.about = true;
        app.dialogs.track_openings();
        app.dialogs.preferences = true;
        app.dialogs.track_openings();

        app.dialogs.about = false;
        app.dialogs.track_openings();
        app.dialogs.about = true;
        app.dialogs.track_openings();

        assert!(app.dismiss_topmost_dialog());
        assert!(!app.dialogs.about);
        assert!(app.dialogs.preferences);
    }

    /// A cached answer must be found again for the same binary and function,
    /// and never for another. This drives the real application state rather
    /// than the cache module alone, so the key it builds is exercised too.
    #[test]
    fn a_cached_function_is_reused_and_never_confused_with_another() {
        let path = std::env::current_exe().expect("the test binary has a path");
        let analysis = desdec_core::analyse_path(&path).expect("the test binary is analysable");
        let digest = analysis.sha256.expect("a whole file has a digest");

        let mut app = DesdecApp::for_test(Some(analysis), WorkspaceView::Decompile);
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
        let path = std::env::current_exe().expect("the test binary has a path");
        let mut analysis = desdec_core::analyse_path(&path).expect("analysable");
        analysis.sha256 = None;
        analysis.truncated = true;

        let app = DesdecApp::for_test(Some(analysis), WorkspaceView::Decompile);

        assert!(app.cache_target().is_none());
    }

    #[test]
    fn turning_the_cache_off_stops_it_being_used() {
        let path = std::env::current_exe().expect("the test binary has a path");
        let analysis = desdec_core::analyse_path(&path).expect("analysable");
        let mut app = DesdecApp::for_test(Some(analysis), WorkspaceView::Decompile);

        assert!(app.cache_target().is_some());
        app.preferences.cache_decompilations = false;
        assert!(app.cache_target().is_none());
    }

    /// Opening the file dialog is not an analysis. The status bar used to
    /// announce one the moment the dialog appeared, for a file the user had
    /// not chosen yet — and might never choose.
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
        let path = std::env::current_exe().expect("the test binary has a path");
        let analysis = desdec_core::analyse_path(&path).expect("the test binary is analysable");
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(Some(analysis), WorkspaceView::Disassembly);

        app.run_command(&ctx, Command::CloseBinary);

        assert!(app.analysis.is_none());
        assert!(app.patches.is_empty(), "patches belong to the closed file");
        assert_eq!(app.active_view, WorkspaceView::Overview);
    }

    #[test]
    fn command_palette_command_toggles_the_palette() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::default();

        app.run_command(&ctx, Command::CommandPalette);
        assert!(app.dialogs.command_palette);

        app.run_command(&ctx, Command::CommandPalette);
        assert!(!app.dialogs.command_palette);
    }

    #[test]
    fn escape_leaves_a_shortcut_capture_before_closing_its_dialog() {
        let mut app = DesdecApp {
            dialogs: Dialogs {
                preferences: true,
                ..Dialogs::default()
            },
            editing_shortcut: Some(Command::OpenBinary),
            ..Default::default()
        };

        app.dismiss_topmost_dialog();
        assert!(app.editing_shortcut.is_none());
        assert!(app.dialogs.preferences);
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
            WorkspaceView::Strings,
            WorkspaceView::Patches,
        ];

        for view in WorkspaceView::ALL {
            assert_eq!(
                view.planned_explanation().is_none(),
                IMPLEMENTED.contains(view),
                "{} is inconsistent",
                view.icon()
            );
        }
    }
}
