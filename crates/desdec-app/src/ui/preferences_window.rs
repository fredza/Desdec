use eframe::egui;

use desdec_core::decompiler::{self, Availability};

use crate::{
    app::{DesdecApp, decompilation_cache_dir},
    commands::{Command, Shortcut},
    i18n::{Language, Text, text},
    preferences::{DecompilerPreference, ThemePreference, accent, success},
    ui::{ERROR, MUTED, format_size},
};

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub enum PreferencesTab {
    #[default]
    Appearance,
    Shortcuts,
    Behaviour,
    Decompiler,
}

impl PreferencesTab {
    const ALL: &[Self] = &[
        Self::Appearance,
        Self::Shortcuts,
        Self::Behaviour,
        Self::Decompiler,
    ];

    const fn text(self) -> Text {
        match self {
            Self::Appearance => Text::Appearance,
            Self::Shortcuts => Text::Shortcuts,
            Self::Behaviour => Text::Behaviour,
            Self::Decompiler => Text::Decompiler,
        }
    }
}

const THEME_CHOICES: &[(ThemePreference, Text)] = &[
    (ThemePreference::System, Text::SystemTheme),
    (ThemePreference::Dark, Text::DarkTheme),
    (ThemePreference::Light, Text::LightTheme),
    (ThemePreference::Catppuccin, Text::CatppuccinTheme),
];

const LANGUAGE_CHOICES: &[(Language, Text)] = &[
    (Language::French, Text::French),
    (Language::English, Text::English),
    (Language::Spanish, Text::Spanish),
];

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.preferences {
        return;
    }

    let mut open = true;
    egui::Window::new(app.t(Text::Preferences))
        // A stable id keeps the window in place when the title is translated.
        .id(egui::Id::new("desdec.preferences"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(410.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for tab in PreferencesTab::ALL {
                    let label = app.t(tab.text());
                    ui.selectable_value(&mut app.preferences_tab, *tab, label);
                }
            });
            ui.separator();
            match app.preferences_tab {
                PreferencesTab::Appearance => appearance(app, ctx, ui),
                PreferencesTab::Shortcuts => shortcuts(app, ctx, ui),
                PreferencesTab::Behaviour => behaviour(app, ui),
                PreferencesTab::Decompiler => decompiler(app, ui),
            }
        });
    app.dialogs.preferences = open;
}

fn appearance(app: &mut DesdecApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.heading(app.t(Text::Appearance));
    ui.label(app.t(Text::Theme));
    let mut theme = app.preferences.theme;
    for (candidate, label) in THEME_CHOICES {
        ui.radio_value(&mut theme, *candidate, app.t(*label));
    }
    if theme != app.preferences.theme {
        app.set_theme(ctx, theme);
    }

    ui.add_space(10.0);
    ui.separator();
    ui.add_space(6.0);
    ui.label(app.t(Text::Language));
    for (candidate, label) in LANGUAGE_CHOICES {
        let label = app.t(*label);
        ui.radio_value(&mut app.preferences.language, *candidate, label);
    }

    ui.add_space(12.0);
    ui.group(|ui| {
        ui.strong(app.t(Text::FreeExtensions));
        ui.label(app.t(Text::FreeExtensionsInfo));
    });
    ui.add_space(8.0);
    ui.small(app.t(Text::PreferencesInfo));
}

fn shortcuts(app: &mut DesdecApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.heading(app.t(Text::Shortcuts));
    if let Some(command) = app.editing_shortcut {
        capture(app, ctx, ui, command);
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
                    ui.label(command.label(app.preferences.language));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(app.t(Text::Modify)).clicked() {
                            app.editing_shortcut = Some(command);
                        }
                        ui.monospace(app.shortcut_label(command));
                    });
                });
            }
        });

    ui.add_space(8.0);
    if ui.button(app.t(Text::ResetDefaults)).clicked() {
        app.preferences.shortcuts.reset();
        app.editing_shortcut = None;
    }
}

/// Waits for the next key combination and assigns it to `command`.
fn capture(app: &mut DesdecApp, ctx: &egui::Context, ui: &mut egui::Ui, command: Command) {
    ui.colored_label(accent(app.preferences.theme), app.t(Text::PressShortcut));
    let cancelled = ui.button(app.t(Text::Cancel)).clicked()
        || ctx.input(|input| input.key_pressed(egui::Key::Escape));
    if cancelled {
        app.editing_shortcut = None;
    } else if let Some(shortcut) = Shortcut::capture(ctx) {
        app.preferences.shortcuts.set(command, shortcut);
        app.editing_shortcut = None;
    }
    ui.add_space(6.0);
}

/// Chooses which decompiler produces the pseudo-code, and reports honestly
/// what is actually installed: an engine offered but absent would be a promise
/// the application cannot keep.
fn decompiler(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    ui.heading(text(language, Text::Decompiler));
    ui.small(text(language, Text::DecompilerInfo));
    ui.add_space(10.0);

    ui.radio_value(
        &mut app.preferences.decompiler,
        DecompilerPreference::Builtin,
        text(language, Text::BuiltinDecompiler),
    );

    for choice in DecompilerPreference::ALL
        .iter()
        .copied()
        .filter(|choice| choice.engine().is_some())
    {
        let Some(engine) = choice.engine() else {
            continue;
        };
        ui.add_space(10.0);
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            let availability = app.engine_availability(engine);
            ui.horizontal(|ui| {
                // Selectable even when absent: the choice is recorded, the
                // status below says what is missing, and the pseudo-code view
                // repeats it with the command that installs it. Disabling the
                // option here would also have contradicted the command
                // palette, which lists every command.
                ui.radio_value(&mut app.preferences.decompiler, choice, engine.label());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    availability_badge(app, ui, &availability);
                });
            });

            match &availability {
                Availability::Found(path) => {
                    ui.small(egui::RichText::new(path.display().to_string()).color(MUTED));
                }
                Availability::Incomplete { .. } => {
                    ui.small(
                        egui::RichText::new(text(language, Text::EngineMissingPlugin)).color(ERROR),
                    );
                    ui.horizontal(|ui| {
                        ui.small(text(language, Text::EngineInstallWith));
                        ui.small(egui::RichText::new(engine.install_hint()).monospace());
                    });
                }
                Availability::Missing => {
                    ui.horizontal(|ui| {
                        ui.small(text(language, Text::EngineInstallWith));
                        ui.small(egui::RichText::new(engine.install_hint()).monospace());
                    });
                }
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.small(text(language, Text::EnginePath));
                let hint = text(language, Text::EnginePathHint);
                ui.add(
                    egui::TextEdit::singleline(app.preferences.engine_paths.field_mut(engine))
                        .hint_text(hint)
                        .desired_width(240.0),
                );
            });
        });
    }

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(6.0);
    cache_controls(app, ui);
}

/// Turning the library explanations on, and pointing at the file that holds
/// the user's own.
fn library_explanations(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    let label = text(language, Text::ExplainLibraries);
    ui.checkbox(&mut app.preferences.explain_libraries, label);
    ui.small(text(language, Text::ExplainLibrariesInfo));

    let Some(path) = crate::libraries::user_catalogue_path() else {
        return;
    };
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if path.exists() {
            // An edit should take effect without restarting the application.
            if ui.button(text(language, Text::ReloadLibraryFile)).clicked() {
                app.library_notes.reload();
            }
            ui.label(
                egui::RichText::new(format!(
                    "{} {}",
                    app.library_notes.user_entries(),
                    text(language, Text::LibraryFileEntries)
                ))
                .color(MUTED),
            );
        } else if ui.button(text(language, Text::CreateLibraryFile)).clicked() {
            // Written with its format shown as comments, so there is nothing
            // to look up before the first entry.
            if crate::libraries::write_example_file(language).is_ok() {
                app.library_notes.reload();
            }
        }
    });
    ui.small(egui::RichText::new(path.display().to_string()).color(MUTED));
}

/// The decompilation cache: what it is for, and how to empty it.
fn cache_controls(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    let label = text(language, Text::CacheDecompilations);
    ui.checkbox(&mut app.preferences.cache_decompilations, label);
    ui.small(text(language, Text::CacheInfo));

    let Some(directory) = decompilation_cache_dir() else {
        return;
    };
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button(text(language, Text::ClearCache)).clicked() {
            app.cache_report = decompiler::cache::clear(&directory)
                .map_err(|error| error.to_string())
                .ok();
        }
        let held = decompiler::cache::size(&directory);
        if held > 0 {
            ui.label(egui::RichText::new(format_size(held)).color(MUTED));
        }
    });
    if let Some(removed) = app.cache_report {
        ui.small(format!(
            "{} {removed} {}",
            text(language, Text::CacheCleared),
            text(language, Text::CacheEntries)
        ));
    }
    ui.small(egui::RichText::new(directory.display().to_string()).color(MUTED));
}

fn availability_badge(app: &DesdecApp, ui: &mut egui::Ui, availability: &Availability) {
    let language = app.preferences.language;
    let (label, color) = match availability {
        Availability::Found(_) => (Text::EngineAvailable, success(app.preferences.theme)),
        Availability::Incomplete { .. } => (Text::EngineIncomplete, ERROR),
        Availability::Missing => (Text::EngineMissing, MUTED),
    };
    ui.label(
        egui::RichText::new(text(language, label))
            .color(color)
            .small(),
    );
}

fn behaviour(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    ui.heading(text(language, Text::Behaviour));
    for (index, (value, label)) in [
        (&mut app.preferences.show_toolbar, Text::ShowToolbar),
        (&mut app.preferences.show_tooltips, Text::ShowTooltips),
        (&mut app.preferences.persistence_enabled, Text::Persistence),
    ]
    .into_iter()
    .enumerate()
    {
        if index > 0 {
            ui.add_space(8.0);
        }
        ui.checkbox(value, text(language, label));
    }
    ui.small(text(language, Text::PersistenceInfo));

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(6.0);
    library_explanations(app, ui);

    ui.add_space(12.0);
    ui.checkbox(
        &mut app.preferences.ai_assistance,
        text(language, Text::AiAssistance),
    );
    ui.small(text(language, Text::AiAssistanceUnavailable));
}
