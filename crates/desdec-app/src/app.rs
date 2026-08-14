use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::Duration,
};

use desdec_core::{Analysis, analyse_path};
use eframe::{Storage, egui};

use crate::{
    commands::{Command, Shortcut},
    i18n::{Language, Text, text},
    preferences::{Preferences, ThemePreference, apply_theme},
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

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub enum WorkspaceView {
    #[default]
    Overview,
    Segments,
    Functions,
    Strings,
    Disassembly,
    Patches,
}

impl WorkspaceView {
    pub const ALL: &[Self] = &[
        Self::Overview,
        Self::Segments,
        Self::Functions,
        Self::Strings,
        Self::Disassembly,
        Self::Patches,
    ];

    pub const fn text(self) -> Text {
        match self {
            Self::Overview => Text::Overview,
            Self::Segments => Text::Segments,
            Self::Functions => Text::Functions,
            Self::Strings => Text::Strings,
            Self::Disassembly => Text::Disassembly,
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
            Self::Patches => "PATCH",
        }
    }

    /// What a not-yet-implemented view announces. `None` for views that already
    /// show real data.
    pub const fn planned_explanation(self) -> Option<Text> {
        match self {
            Self::Overview | Self::Segments | Self::Strings => None,
            Self::Functions => Some(Text::FunctionsInfo),
            Self::Disassembly => Some(Text::DisassemblyInfo),
            Self::Patches => Some(Text::PatchesInfo),
        }
    }
}

/// Modal windows, ordered from the topmost to the oldest.
#[derive(Default)]
pub struct Dialogs {
    pub command_palette: bool,
    pub preferences: bool,
    pub about: bool,
}

impl Dialogs {
    /// Closes the topmost open dialog and reports whether one was closed.
    fn dismiss_topmost(&mut self) -> bool {
        for open in [
            &mut self.command_palette,
            &mut self.preferences,
            &mut self.about,
        ] {
            if *open {
                *open = false;
                return true;
            }
        }
        false
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
}

#[derive(Default)]
pub struct DesdecApp {
    /// Result of the deep analysis of the loaded binary.
    pub analysis: Option<Analysis>,
    pub error: Option<String>,
    pub active_view: WorkspaceView,
    pub navigation_open: bool,
    pub expert_mode: bool,
    pub dialogs: Dialogs,
    pub preferences: Preferences,
    pub preferences_tab: PreferencesTab,
    pub editing_shortcut: Option<Command>,
    pub palette: PaletteState,
    /// Free-text filter applied to the extracted strings.
    pub strings_filter: String,
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

    /// Short name of the current mode, for the status-bar toggle.
    pub fn mode_label(&self) -> &'static str {
        if self.expert_mode {
            self.t(Text::Expert)
        } else {
            self.t(Text::Guided)
        }
    }

    /// How the application describes what it is doing, in the title area and in
    /// the tooltip of the mode toggle. Follows the selected mode.
    pub fn analysis_mode_label(&self) -> &'static str {
        if self.expert_mode {
            self.t(Text::ExpertAnalysis)
        } else {
            self.t(Text::GuidedAnalysis)
        }
    }

    pub fn shortcut_label(&self, command: Command) -> String {
        self.preferences
            .shortcuts
            .shortcut_for(command)
            .map_or_else(|| self.t(Text::NoShortcut).to_owned(), Shortcut::label)
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
                self.palette = PaletteState::default();
                self.dialogs.command_palette = true;
            }
            Command::Preferences => self.dialogs.preferences = true,
            Command::About => self.dialogs.about = true,
            Command::Overview => self.select_view(WorkspaceView::Overview),
            Command::Disassembly => self.select_view(WorkspaceView::Disassembly),
            Command::Functions => self.select_view(WorkspaceView::Functions),
            Command::Strings => self.select_view(WorkspaceView::Strings),
            Command::Patches => self.select_view(WorkspaceView::Patches),
            Command::ToggleExpertMode => self.expert_mode = !self.expert_mode,
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
        if self.editing_shortcut.is_some() || ctx.wants_keyboard_input() {
            return;
        }
        let command = Command::ALL.iter().copied().find(|command| {
            self.preferences
                .shortcuts
                .shortcut_for(*command)
                .is_some_and(|shortcut| shortcut.pressed(ctx))
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
                self.active_view = WorkspaceView::Overview;
                self.strings_filter.clear();
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
    }

    pub fn close_binary(&mut self) {
        self.analysis = None;
        self.error = None;
        self.active_view = WorkspaceView::Overview;
        self.strings_filter.clear();
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
}

impl eframe::App for DesdecApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.preferences.theme == ThemePreference::System {
            apply_theme(ctx, ThemePreference::System);
        }
        self.process_shortcuts(ctx);
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

    fn save(&mut self, storage: &mut dyn Storage) {
        self.persist_preferences(storage);
    }

    fn auto_save_interval(&self) -> Duration {
        AUTO_SAVE_INTERVAL
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

    #[test]
    fn escape_dismisses_dialogs_from_the_topmost_to_the_oldest() {
        let mut app = DesdecApp {
            dialogs: Dialogs {
                command_palette: true,
                preferences: true,
                about: true,
            },
            ..Default::default()
        };

        assert!(app.dismiss_topmost_dialog());
        assert!(!app.dialogs.command_palette);
        assert!(app.dialogs.preferences);

        assert!(app.dismiss_topmost_dialog());
        assert!(!app.dialogs.preferences);
        assert!(app.dialogs.about);

        assert!(app.dismiss_topmost_dialog());
        assert!(!app.dialogs.about);

        assert!(!app.dismiss_topmost_dialog());
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

    /// The header and the status-bar tooltip must name the mode that is
    /// actually selected, in every language.
    #[test]
    fn the_analysis_label_follows_the_selected_mode() {
        let mut app = DesdecApp::default();
        for language in Language::ALL {
            app.preferences.language = *language;

            app.expert_mode = false;
            assert_eq!(app.analysis_mode_label(), app.t(Text::GuidedAnalysis));
            assert_eq!(app.mode_label(), app.t(Text::Guided));

            app.expert_mode = true;
            assert_eq!(app.analysis_mode_label(), app.t(Text::ExpertAnalysis));
            assert_eq!(app.mode_label(), app.t(Text::Expert));
        }
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
            WorkspaceView::Strings,
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
