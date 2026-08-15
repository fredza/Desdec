use eframe::egui;

use crate::{
    app::DesdecApp,
    commands::{Command, Shortcut},
    i18n::{Language, Text, text},
    preferences::{ThemePreference, accent},
};

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub enum PreferencesTab {
    #[default]
    Appearance,
    Shortcuts,
    Behaviour,
}

impl PreferencesTab {
    const ALL: &[Self] = &[Self::Appearance, Self::Shortcuts, Self::Behaviour];

    const fn text(self) -> Text {
        match self {
            Self::Appearance => Text::Appearance,
            Self::Shortcuts => Text::Shortcuts,
            Self::Behaviour => Text::Behaviour,
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
    ui.checkbox(
        &mut app.preferences.ai_assistance,
        text(language, Text::AiAssistance),
    );
    ui.small(text(language, Text::AiAssistanceUnavailable));
}
