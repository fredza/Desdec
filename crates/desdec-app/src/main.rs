use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

use desdec_core::{BinarySummary, inspect_path};
use eframe::{Storage, egui};

const MUTED: egui::Color32 = egui::Color32::from_rgb(145, 155, 178);
const PREFERENCES_KEY: &str = "desdec.preferences";

mod commands;
mod i18n;
mod icons;
mod preferences;

use commands::{Command, Shortcut};
use i18n::{Language, Text, text};
use icons::Icon;
use preferences::{Preferences, ThemePreference, accent, apply_theme, success};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([860.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Desdec",
        options,
        Box::new(|creation_context| Ok(Box::new(DesdecApp::new(creation_context)))),
    )
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum WorkspaceView {
    #[default]
    Overview,
    Segments,
    Functions,
    Strings,
    Disassembly,
    Patches,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum PreferencesTab {
    #[default]
    Appearance,
    Shortcuts,
    Behaviour,
}

impl WorkspaceView {
    const fn text(self) -> Text {
        match self {
            Self::Overview => Text::Overview,
            Self::Segments => Text::Segments,
            Self::Functions => Text::Functions,
            Self::Strings => Text::Strings,
            Self::Disassembly => Text::Disassembly,
            Self::Patches => Text::Patches,
        }
    }

    const fn icon(self) -> &'static str {
        match self {
            Self::Overview => "ACC",
            Self::Segments => "SEG",
            Self::Functions => "Fn",
            Self::Strings => "STR",
            Self::Disassembly => "ASM",
            Self::Patches => "PATCH",
        }
    }
}

#[derive(Default)]
struct DesdecApp {
    binary: Option<BinarySummary>,
    error: Option<String>,
    active_view: WorkspaceView,
    navigation_open: bool,
    command_palette_open: bool,
    preferences_open: bool,
    about_open: bool,
    expert_mode: bool,
    preferences: Preferences,
    preferences_tab: PreferencesTab,
    editing_shortcut: Option<Command>,
    command_query: String,
    palette_selected: usize,
    file_picker_result: Option<Receiver<Option<PathBuf>>>,
    inspection_result: Option<Receiver<(PathBuf, std::io::Result<BinarySummary>)>>,
}

impl DesdecApp {
    fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        let preferences: Preferences = creation_context
            .storage
            .and_then(|storage| eframe::get_value(storage, PREFERENCES_KEY))
            .unwrap_or_default();
        apply_theme(&creation_context.egui_ctx, preferences.theme);
        Self {
            preferences,
            ..Self::default()
        }
    }

    fn t(&self, item: Text) -> &'static str {
        text(self.preferences.language, item)
    }

    fn set_theme(&mut self, ctx: &egui::Context, theme: ThemePreference) {
        self.preferences.theme = theme;
        apply_theme(ctx, theme);
    }

    fn command_tooltip(&self, command: Command) -> String {
        let label = command.label(self.preferences.language);
        match self.preferences.shortcuts.shortcut_for(command) {
            Some(shortcut) => format!("{label} ({})", shortcut.label()),
            None => label,
        }
    }

    fn tooltip(&self, response: egui::Response, text: &str) -> egui::Response {
        if self.preferences.show_tooltips {
            response.on_hover_text(text)
        } else {
            response
        }
    }

    fn run_command(&mut self, ctx: &egui::Context, command: Command) {
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
                self.command_query.clear();
                self.palette_selected = 0;
                self.command_palette_open = true;
            }
            Command::Preferences => self.preferences_open = true,
            Command::About => self.about_open = true,
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

    fn dismiss_topmost_dialog(&mut self) {
        if self.editing_shortcut.is_some() {
            self.editing_shortcut = None;
        } else if self.command_palette_open {
            self.command_palette_open = false;
        } else if self.preferences_open {
            self.preferences_open = false;
        } else if self.about_open {
            self.about_open = false;
        }
    }

    fn dismiss_dialog_with_escape(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.dismiss_topmost_dialog();
        }
    }

    fn choose_binary(&mut self, ctx: &egui::Context) {
        if self.file_picker_result.is_some() || self.inspection_result.is_some() {
            return;
        }

        let title = self.t(Text::OpenBinary).to_owned();
        let repaint = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.file_picker_result = Some(receiver);
        std::thread::spawn(move || {
            let path = pollster::block_on(rfd::AsyncFileDialog::new().set_title(title).pick_file())
                .map(|file| file.path().to_path_buf());
            let _ = sender.send(path);
            repaint.request_repaint();
        });
    }

    fn inspect_binary(&mut self, ctx: &egui::Context, path: PathBuf) {
        if self.inspection_result.is_some() {
            return;
        }

        let repaint = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.inspection_result = Some(receiver);
        std::thread::spawn(move || {
            let result = inspect_path(&path);
            let _ = sender.send((path, result));
            repaint.request_repaint();
        });
    }

    fn apply_inspection(&mut self, path: &Path, result: std::io::Result<BinarySummary>) {
        match result {
            Ok(summary) => {
                self.binary = Some(summary);
                self.error = None;
                self.active_view = WorkspaceView::Overview;
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
        let picked_file = self.file_picker_result.as_ref().map(Receiver::try_recv);
        match picked_file {
            Some(Ok(Some(path))) => {
                self.file_picker_result = None;
                self.inspect_binary(ctx, path);
            }
            Some(Ok(None) | Err(TryRecvError::Disconnected)) => self.file_picker_result = None,
            Some(Err(TryRecvError::Empty)) | None => {}
        }

        let inspection = self.inspection_result.as_ref().map(Receiver::try_recv);
        match inspection {
            Some(Ok((path, result))) => {
                self.inspection_result = None;
                self.apply_inspection(&path, result);
            }
            Some(Err(TryRecvError::Disconnected)) => self.inspection_result = None,
            Some(Err(TryRecvError::Empty)) | None => {}
        }
    }

    fn close_binary(&mut self) {
        self.binary = None;
        self.error = None;
        self.active_view = WorkspaceView::Overview;
    }

    fn select_view(&mut self, view: WorkspaceView) {
        self.active_view = view;
        self.navigation_open = false;
    }

    fn action_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("action_bar")
            .exact_height(48.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let hamburger = self.tooltip(
                        ui.add(
                            egui::Button::new("☰")
                                .min_size(egui::vec2(34.0, 30.0))
                                .frame(false),
                        ),
                        self.t(Text::Menu),
                    );
                    if hamburger.clicked() {
                        self.navigation_open = !self.navigation_open;
                    }

                    ui.separator();
                    ui.label(
                        egui::RichText::new("D")
                            .color(accent(self.preferences.theme))
                            .strong()
                            .size(20.0),
                    );
                    ui.strong("Desdec");
                    ui.label(
                        egui::RichText::new(format!("/ {}", self.t(Text::GuidedAnalysis)))
                            .color(MUTED),
                    );
                    if self.preferences.show_toolbar {
                        ui.separator();
                        let accent = accent(self.preferences.theme);
                        if icons::button(
                            ui,
                            Icon::Overview,
                            self.preferences
                                .show_tooltips
                                .then(|| self.command_tooltip(Command::Overview)),
                            self.active_view == WorkspaceView::Overview,
                            accent,
                        )
                        .clicked()
                        {
                            self.run_command(ctx, Command::Overview);
                        }
                        if icons::button(
                            ui,
                            Icon::Disassembly,
                            self.preferences
                                .show_tooltips
                                .then(|| self.command_tooltip(Command::Disassembly)),
                            self.active_view == WorkspaceView::Disassembly,
                            accent,
                        )
                        .clicked()
                        {
                            self.run_command(ctx, Command::Disassembly);
                        }
                        if icons::button(
                            ui,
                            Icon::Functions,
                            self.preferences
                                .show_tooltips
                                .then(|| self.command_tooltip(Command::Functions)),
                            self.active_view == WorkspaceView::Functions,
                            accent,
                        )
                        .clicked()
                        {
                            self.run_command(ctx, Command::Functions);
                        }
                        if icons::button(
                            ui,
                            Icon::Strings,
                            self.preferences
                                .show_tooltips
                                .then(|| self.command_tooltip(Command::Strings)),
                            self.active_view == WorkspaceView::Strings,
                            accent,
                        )
                        .clicked()
                        {
                            self.run_command(ctx, Command::Strings);
                        }
                        if icons::button(
                            ui,
                            Icon::Patches,
                            self.preferences
                                .show_tooltips
                                .then(|| self.command_tooltip(Command::Patches)),
                            self.active_view == WorkspaceView::Patches,
                            accent,
                        )
                        .clicked()
                        {
                            self.run_command(ctx, Command::Patches);
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icons::button(
                                ui,
                                Icon::Palette,
                                self.preferences
                                    .show_tooltips
                                    .then(|| self.command_tooltip(Command::CommandPalette)),
                                false,
                                accent,
                            )
                            .clicked()
                            {
                                self.run_command(ctx, Command::CommandPalette);
                            }
                            if icons::button(
                                ui,
                                Icon::Open,
                                self.preferences
                                    .show_tooltips
                                    .then(|| self.command_tooltip(Command::OpenBinary)),
                                false,
                                accent,
                            )
                            .clicked()
                            {
                                self.run_command(ctx, Command::OpenBinary);
                            }
                        });
                    }
                });
            });
    }

    fn navigation(&mut self, ctx: &egui::Context) {
        if !self.navigation_open {
            return;
        }

        egui::SidePanel::left("navigation")
            .resizable(false)
            .exact_width(276.0)
            .show(ctx, |ui| {
                let accent = accent(self.preferences.theme);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(self.t(Text::Menu))
                            .color(MUTED)
                            .small()
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self
                            .tooltip(
                                ui.add(
                                    egui::Button::new("×")
                                        .frame(false)
                                        .min_size(egui::vec2(28.0, 28.0)),
                                ),
                                self.t(Text::CollapseMenu),
                            )
                            .clicked()
                        {
                            self.navigation_open = false;
                        }
                    });
                });
                ui.add_space(10.0);

                if ui
                    .add_sized(
                        [ui.available_width(), 34.0],
                        egui::Button::new(
                            egui::RichText::new(self.t(Text::OpenBinary))
                                .color(egui::Color32::WHITE),
                        )
                        .fill(accent.gamma_multiply(0.72)),
                    )
                    .clicked()
                {
                    self.choose_binary(ctx);
                }
                if self.binary.is_some()
                    && ui
                        .add_sized(
                            [ui.available_width(), 30.0],
                            egui::Button::new(self.t(Text::CloseBinary)).frame(false),
                        )
                        .clicked()
                {
                    self.close_binary();
                }

                ui.add_space(16.0);
                egui::CollapsingHeader::new(
                    egui::RichText::new(self.t(Text::Exploration))
                        .color(MUTED)
                        .small()
                        .strong(),
                )
                .id_salt("navigation.exploration")
                .default_open(true)
                .show(ui, |ui| {
                    ui.add_space(3.0);
                    for view in [
                        WorkspaceView::Overview,
                        WorkspaceView::Segments,
                        WorkspaceView::Functions,
                        WorkspaceView::Strings,
                        WorkspaceView::Disassembly,
                        WorkspaceView::Patches,
                    ] {
                        let selected = self.active_view == view;
                        if ui
                            .selectable_label(
                                selected,
                                format!("{}  {}", view.icon(), self.t(view.text())),
                            )
                            .clicked()
                        {
                            self.select_view(view);
                        }
                    }
                    ui.add_space(3.0);
                });

                ui.add_space(10.0);
                egui::CollapsingHeader::new(
                    egui::RichText::new(self.t(Text::Tools))
                        .color(MUTED)
                        .small()
                        .strong(),
                )
                .id_salt("navigation.tools")
                .default_open(true)
                .show(ui, |ui| {
                    ui.add_space(3.0);
                    if ui.button(self.t(Text::CommandPalette)).clicked() {
                        self.run_command(ctx, Command::CommandPalette);
                        self.navigation_open = false;
                    }
                    if ui.button(self.t(Text::Preferences)).clicked() {
                        self.preferences_open = true;
                        self.navigation_open = false;
                    }
                    if ui.button(self.t(Text::About)).clicked() {
                        self.about_open = true;
                        self.navigation_open = false;
                    }
                    ui.add_space(3.0);
                });

                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.separator();
                    ui.small(self.t(Text::MenuHint));
                });
            });
    }

    fn appearance_preferences(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading(self.t(Text::Appearance));
        ui.label(self.t(Text::Theme));
        let mut theme = self.preferences.theme;
        for (candidate, label) in [
            (ThemePreference::System, Text::SystemTheme),
            (ThemePreference::Dark, Text::DarkTheme),
            (ThemePreference::Light, Text::LightTheme),
            (ThemePreference::Catppuccin, Text::CatppuccinTheme),
        ] {
            ui.radio_value(&mut theme, candidate, self.t(label));
        }
        if theme != self.preferences.theme {
            self.set_theme(ctx, theme);
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(self.t(Text::Language));
        for (candidate, label) in [
            (Language::French, Text::French),
            (Language::English, Text::English),
            (Language::Spanish, Text::Spanish),
        ] {
            let label = self.t(label);
            ui.radio_value(&mut self.preferences.language, candidate, label);
        }

        ui.add_space(12.0);
        ui.group(|ui| {
            ui.strong(self.t(Text::FreeExtensions));
            ui.label(self.t(Text::FreeExtensionsInfo));
        });
        ui.add_space(8.0);
        ui.small(self.t(Text::PreferencesInfo));
    }

    fn shortcut_preferences(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading(self.t(Text::Shortcuts));
        if let Some(command) = self.editing_shortcut {
            ui.colored_label(accent(self.preferences.theme), self.t(Text::PressShortcut));
            let cancel = ui.button(self.t(Text::Cancel)).clicked()
                || ctx.input(|input| input.key_pressed(egui::Key::Escape));
            if cancel {
                self.editing_shortcut = None;
            } else if let Some(shortcut) = Shortcut::capture(ctx) {
                self.preferences.shortcuts.set(command, shortcut);
                self.editing_shortcut = None;
            }
            ui.add_space(6.0);
        }

        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                for command in Command::ALL
                    .iter()
                    .copied()
                    .filter(|command| command.configurable_shortcut())
                {
                    ui.horizontal(|ui| {
                        ui.label(command.label(self.preferences.language));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(self.t(Text::Modify)).clicked() {
                                self.editing_shortcut = Some(command);
                            }
                            let shortcut = self
                                .preferences
                                .shortcuts
                                .shortcut_for(command)
                                .map_or_else(
                                    || self.t(Text::NoShortcut).to_owned(),
                                    Shortcut::label,
                                );
                            ui.monospace(shortcut);
                        });
                    });
                }
            });
        ui.add_space(8.0);
        if ui.button(self.t(Text::ResetDefaults)).clicked() {
            self.preferences.shortcuts.reset();
            self.editing_shortcut = None;
        }
    }

    fn behaviour_preferences(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.t(Text::Behaviour));
        let show_toolbar = self.t(Text::ShowToolbar);
        ui.checkbox(&mut self.preferences.show_toolbar, show_toolbar);
        ui.add_space(8.0);
        let show_tooltips = self.t(Text::ShowTooltips);
        ui.checkbox(&mut self.preferences.show_tooltips, show_tooltips);
        ui.add_space(8.0);
        let persistence = self.t(Text::Persistence);
        ui.checkbox(&mut self.preferences.persistence_enabled, persistence);
        ui.small(self.t(Text::PersistenceInfo));
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(28.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("OK").color(success(self.preferences.theme)));
                    if let Some(summary) = &self.binary {
                        ui.label(summary.format.label());
                        ui.separator();
                        ui.label(summary.architecture.label());
                        ui.separator();
                        ui.label(format_size(summary.size));
                    } else {
                        ui.label(self.t(Text::ReadyToOpen));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mode = if self.expert_mode {
                            self.t(Text::Expert)
                        } else {
                            self.t(Text::Guided)
                        };
                        if self
                            .tooltip(ui.button(mode), self.t(Text::GuidedAnalysis))
                            .clicked()
                        {
                            self.expert_mode = !self.expert_mode;
                        }
                        ui.separator();
                        ui.label(self.t(Text::NoPatches));
                    });
                });
            });
    }

    fn auxiliary_windows(&mut self, ctx: &egui::Context) {
        if self.command_palette_open {
            let mut open = self.command_palette_open;
            let mut close_palette = false;
            egui::Window::new(self.t(Text::PaletteTitle))
                .id(egui::Id::new("desdec.command_palette"))
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .default_size(egui::vec2(520.0, 420.0))
                .min_width(360.0)
                .min_height(260.0)
                .show(ctx, |ui| {
                    ui.label(self.t(Text::SearchAction));
                    let search_hint = self.t(Text::SearchHint);
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.command_query).hint_text(search_hint),
                    );
                    response.request_focus();
                    if response.changed() {
                        self.palette_selected = 0;
                    }
                    ui.add_space(8.0);
                    let query = self.command_query.to_lowercase();
                    let commands: Vec<_> = Command::ALL
                        .iter()
                        .copied()
                        .filter(|command| {
                            command
                                .label(self.preferences.language)
                                .to_lowercase()
                                .contains(&query)
                        })
                        .collect();
                    let mut selected = None;
                    if commands.is_empty() {
                        self.palette_selected = 0;
                    } else {
                        self.palette_selected = self.palette_selected.min(commands.len() - 1);
                        if ctx.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
                            self.palette_selected = (self.palette_selected + 1) % commands.len();
                        }
                        if ctx.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
                            self.palette_selected = if self.palette_selected == 0 {
                                commands.len() - 1
                            } else {
                                self.palette_selected - 1
                            };
                        }
                        if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
                            selected = Some(commands[self.palette_selected]);
                        }
                    }
                    egui::ScrollArea::vertical()
                        .max_height(ui.available_height().max(160.0))
                        .show(ui, |ui| {
                            for (index, command) in commands.iter().copied().enumerate() {
                                ui.horizontal(|ui| {
                                    if ui
                                        .selectable_label(
                                            index == self.palette_selected,
                                            command.label(self.preferences.language),
                                        )
                                        .clicked()
                                    {
                                        selected = Some(command);
                                        self.palette_selected = index;
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let shortcut = self
                                                .preferences
                                                .shortcuts
                                                .shortcut_for(command)
                                                .map_or_else(
                                                    || self.t(Text::NoShortcut).to_owned(),
                                                    Shortcut::label,
                                                );
                                            ui.label(egui::RichText::new(shortcut).color(MUTED));
                                        },
                                    );
                                });
                            }
                        });
                    if let Some(command) = selected {
                        self.run_command(ctx, command);
                        close_palette = true;
                    }
                });
            if close_palette {
                open = false;
            }
            self.command_palette_open = open;
        }

        if self.preferences_open {
            let mut open = self.preferences_open;
            egui::Window::new(self.t(Text::Preferences))
                .id(egui::Id::new("desdec.preferences"))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(410.0)
                .show(ctx, |ui| {
                    let appearance = self.t(Text::Appearance);
                    let shortcuts = self.t(Text::Shortcuts);
                    let behaviour = self.t(Text::Behaviour);
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.preferences_tab,
                            PreferencesTab::Appearance,
                            appearance,
                        );
                        ui.selectable_value(
                            &mut self.preferences_tab,
                            PreferencesTab::Shortcuts,
                            shortcuts,
                        );
                        ui.selectable_value(
                            &mut self.preferences_tab,
                            PreferencesTab::Behaviour,
                            behaviour,
                        );
                    });
                    ui.separator();
                    match self.preferences_tab {
                        PreferencesTab::Appearance => self.appearance_preferences(ui, ctx),
                        PreferencesTab::Shortcuts => self.shortcut_preferences(ui, ctx),
                        PreferencesTab::Behaviour => self.behaviour_preferences(ui),
                    }
                });
            self.preferences_open = open;
        }

        if self.about_open {
            let mut open = self.about_open;
            egui::Window::new(self.t(Text::AboutTitle))
                .id(egui::Id::new("desdec.about"))
                .open(&mut open)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.heading("Desdec");
                    ui.label(self.t(Text::AboutDescription));
                    ui.small(self.t(Text::LegalNotice));
                });
            self.about_open = open;
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

        self.action_bar(ctx);
        self.navigation(ctx);
        self.status_bar(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(self.t(self.active_view.text()));
                ui.separator();
                let mode = if self.expert_mode {
                    self.t(Text::Expert)
                } else {
                    self.t(Text::Guided)
                };
                ui.label(egui::RichText::new(mode).color(MUTED));
            });
            ui.add_space(12.0);
            content(ui, self);
            if let Some(error) = &self.error {
                ui.add_space(16.0);
                ui.colored_label(egui::Color32::from_rgb(232, 119, 91), error);
            }
        });

        self.auxiliary_windows(ctx);
    }

    fn save(&mut self, storage: &mut dyn Storage) {
        if self.preferences.persistence_enabled {
            eframe::set_value(storage, PREFERENCES_KEY, &self.preferences);
        } else {
            storage.set_string(PREFERENCES_KEY, String::new());
        }
    }
}

fn content(ui: &mut egui::Ui, app: &mut DesdecApp) {
    match app.active_view {
        WorkspaceView::Overview => match &app.binary {
            Some(summary) => overview(ui, summary, app.preferences.language),
            None => welcome(ui, app),
        },
        view => planned_view(
            ui,
            view,
            app.binary.as_ref(),
            app.expert_mode,
            app.preferences.language,
        ),
    }
}

fn welcome(ui: &mut egui::Ui, app: &mut DesdecApp) {
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

fn overview(ui: &mut egui::Ui, summary: &BinarySummary, language: Language) {
    ui.group(|ui| {
        ui.strong(text(language, Text::ActiveFile));
        ui.add_space(8.0);
        egui::Grid::new("binary_summary")
            .num_columns(2)
            .spacing([24.0, 12.0])
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
            });
    });
    ui.add_space(16.0);
    ui.group(|ui| {
        ui.strong(text(language, Text::NextStep));
        ui.label(text(language, Text::NextStepDetail));
    });
}

fn planned_view(
    ui: &mut egui::Ui,
    view: WorkspaceView,
    binary: Option<&BinarySummary>,
    expert_mode: bool,
    language: Language,
) {
    if binary.is_none() {
        ui.label(text(language, Text::OpenFirst));
        return;
    }
    ui.group(|ui| {
        ui.strong(format!(
            "{} {}",
            text(language, view.text()),
            text(language, Text::ComingSoon)
        ));
        ui.add_space(8.0);
        let explanation = match view {
            WorkspaceView::Segments => Text::SegmentsInfo,
            WorkspaceView::Functions => Text::FunctionsInfo,
            WorkspaceView::Strings => Text::StringsInfo,
            WorkspaceView::Disassembly => Text::DisassemblyInfo,
            WorkspaceView::Patches => Text::PatchesInfo,
            WorkspaceView::Overview => unreachable!(),
        };
        ui.label(text(language, explanation));
        if !expert_mode {
            ui.add_space(8.0);
            ui.small(text(language, Text::GuidedHelp));
        }
    });
}

fn format_size(size: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    if size >= MIB {
        format!("{:.2} MiB", size as f64 / MIB as f64)
    } else if size >= KIB {
        format!("{:.2} KiB", size as f64 / KIB as f64)
    } else {
        format!("{size} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_dismisses_dialogs_from_the_topmost_to_the_oldest() {
        let mut app = DesdecApp {
            command_palette_open: true,
            preferences_open: true,
            about_open: true,
            ..Default::default()
        };

        app.dismiss_topmost_dialog();
        assert!(!app.command_palette_open);
        assert!(app.preferences_open);

        app.dismiss_topmost_dialog();
        assert!(!app.preferences_open);
        assert!(app.about_open);

        app.dismiss_topmost_dialog();
        assert!(!app.about_open);
    }
}
