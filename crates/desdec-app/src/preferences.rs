use std::path::Path;

use desdec_core::{assistant, decompiler::Engine};
use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::{commands::ShortcutBindings, i18n::Language};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum ThemePreference {
    #[default]
    System,
    Dark,
    Light,
    Catppuccin,
}

/// Which decompiler produces the pseudo-code.
///
/// The built-in one always works and needs nothing installed; the others are
/// external programs, run only when explicitly chosen.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum DecompilerPreference {
    #[default]
    Builtin,
    RzGhidra,
    RetDec,
}

impl DecompilerPreference {
    pub const ALL: &[Self] = &[Self::Builtin, Self::RzGhidra, Self::RetDec];

    /// The external engine behind this choice, or `None` for the built-in one.
    #[must_use]
    pub const fn engine(self) -> Option<Engine> {
        match self {
            Self::Builtin => None,
            Self::RzGhidra => Some(Engine::RzGhidra),
            Self::RetDec => Some(Engine::RetDec),
        }
    }
}

/// Where the optional AI assistance comes from.
///
/// A mirror of [`assistant::Provider`], which the core keeps free of serde:
/// this is the value that gets written to the preferences file, and the core
/// should not have to care what that file looks like.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum AssistantPreference {
    /// Nothing is configured, and nothing is ever sent. The default, because
    /// no reader should discover after the fact that their binary was
    /// described to a service.
    #[default]
    None,
    Ollama,
    Claude,
}

impl AssistantPreference {
    pub const ALL: &[Self] = &[Self::None, Self::Ollama, Self::Claude];

    #[must_use]
    pub const fn provider(self) -> assistant::Provider {
        match self {
            Self::None => assistant::Provider::None,
            Self::Ollama => assistant::Provider::Ollama,
            Self::Claude => assistant::Provider::Claude,
        }
    }
}

/// Where an external engine lives, when it is not simply on `PATH`.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnginePaths {
    pub rz_ghidra: String,
    pub retdec: String,
}

impl EnginePaths {
    #[must_use]
    pub fn for_engine(&self, engine: Engine) -> Option<&Path> {
        let configured = match engine {
            Engine::RzGhidra => &self.rz_ghidra,
            Engine::RetDec => &self.retdec,
        };
        (!configured.trim().is_empty()).then(|| Path::new(configured.trim()))
    }

    pub fn field_mut(&mut self, engine: Engine) -> &mut String {
        match engine {
            Engine::RzGhidra => &mut self.rz_ghidra,
            Engine::RetDec => &mut self.retdec,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: ThemePreference,
    pub language: Language,
    pub show_toolbar: bool,
    pub show_tooltips: bool,
    /// Whether hovering a disassembly row says what its operand designates.
    ///
    /// Its own switch rather than a corner of `show_tooltips`: this one reads
    /// the file at the address an instruction computes, and a reader who wants
    /// the listing and nothing else should be able to say so.
    pub show_operand_hints: bool,
    pub persistence_enabled: bool,
    /// Which model, if any, the assistant asks. `None` by default.
    pub assistant: AssistantPreference,
    /// Model name given to that provider, or empty for its own default.
    pub assistant_model: String,
    /// Where a local Ollama server listens, or empty for the usual address.
    pub ollama_url: String,
    /// File the Anthropic key is read from when `ANTHROPIC_API_KEY` is unset.
    ///
    /// The path, never the key: this file is written in plain text, and a
    /// secret in it would be copied into every backup of the preferences.
    pub anthropic_key_path: String,
    pub decompiler: DecompilerPreference,
    pub engine_paths: EnginePaths,
    /// Whether decompiled functions are kept on disk between runs.
    pub cache_decompilations: bool,
    /// Successfully analysed binaries, newest first. This is local UI state;
    /// it is never sent anywhere and can be cleared from the open menu.
    pub recent_binaries: Vec<std::path::PathBuf>,
    /// Enables the optional, local YARA scanning module.
    pub yara_enabled: bool,
    /// Explicit path to the `yara` command, or empty to search `PATH`.
    pub yara_path: String,
    /// Rules file passed to YARA for every deliberate scan.
    pub yara_rules_path: String,
    /// Whether the linked-library list offers an explanation for each name.
    pub explain_libraries: bool,
    /// Whether the reader's own notes are kept between sessions.
    ///
    /// They are written beside the application's data, keyed by the binary's
    /// digest — never into the binary, and never into its directory.
    pub save_annotations: bool,
    /// Width the navigation menu was last dragged to, in points.
    ///
    /// Kept because it is a choice, not a detail: the menu shows icons alone,
    /// icons with labels, or everything, depending on how much room it was
    /// given, and reopening it narrower than it was left would undo that
    /// choice. Whole points rather than a float so preferences stay comparable
    /// and round-trip exactly.
    pub navigation_width: u16,
    /// What the reader has decided about each installed plugin, by the name of
    /// the directory it lives in.
    ///
    /// Kept here because it is a decision, and one that must survive the
    /// session that made it: a plugin the reader looked at, read the
    /// permissions of and enabled should not ask again tomorrow. A plugin that
    /// was uninstalled leaves its entry behind, which is deliberate —
    /// reinstalling it does not silently inherit the old consent unless it
    /// asks for the same things.
    pub plugins: std::collections::BTreeMap<String, crate::plugins::Consent>,
    /// Whether Desdec may ask GitHub if there is a newer release.
    ///
    /// `None` until the reader has been asked, which is the only state in
    /// which nothing has left this machine and nothing will: a check tells a
    /// server that this copy was started, which is a thing to be agreed to
    /// rather than assumed. `Some(false)` is a decision and is respected for
    /// good; `Some(true)` is what the reader turned on.
    pub check_for_updates: Option<bool>,
    /// A version the reader said they did not want. Offered again only when
    /// something newer than it is published.
    pub skipped_release: Option<String>,
    /// Where a downloaded archive is written, or empty for the usual place.
    pub download_directory: String,
    pub shortcuts: ShortcutBindings,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            language: Language::French,
            show_toolbar: true,
            show_tooltips: true,
            show_operand_hints: true,
            persistence_enabled: true,
            assistant: AssistantPreference::None,
            assistant_model: String::new(),
            ollama_url: String::new(),
            anthropic_key_path: String::new(),
            decompiler: DecompilerPreference::Builtin,
            engine_paths: EnginePaths::default(),
            cache_decompilations: true,
            recent_binaries: Vec::new(),
            yara_enabled: false,
            yara_path: String::new(),
            yara_rules_path: String::new(),
            explain_libraries: true,
            save_annotations: true,
            navigation_width: crate::ui::navigation::DEFAULT_WIDTH,
            plugins: std::collections::BTreeMap::new(),
            check_for_updates: None,
            skipped_release: None,
            download_directory: String::new(),
            shortcuts: ShortcutBindings::default(),
        }
    }
}

pub fn apply_theme(ctx: &egui::Context, preference: ThemePreference) {
    let visuals = match preference {
        ThemePreference::System => match ctx.system_theme() {
            Some(egui::Theme::Light) => light_visuals(),
            Some(egui::Theme::Dark) | None => dark_visuals(),
        },
        ThemePreference::Dark => dark_visuals(),
        ThemePreference::Light => light_visuals(),
        ThemePreference::Catppuccin => catppuccin_visuals(),
    };
    ctx.set_visuals(visuals);
}

/// Semantic colours shared by every widget of a theme.
#[derive(Clone, Copy)]
struct Palette {
    accent: egui::Color32,
    success: egui::Color32,
}

const STANDARD_PALETTE: Palette = Palette {
    accent: egui::Color32::from_rgb(126, 171, 255),
    success: egui::Color32::from_rgb(91, 201, 139),
};

const CATPPUCCIN_PALETTE: Palette = Palette {
    accent: egui::Color32::from_rgb(137, 180, 250),
    success: egui::Color32::from_rgb(166, 227, 161),
};

const fn palette(theme: ThemePreference) -> Palette {
    match theme {
        ThemePreference::Catppuccin => CATPPUCCIN_PALETTE,
        ThemePreference::System | ThemePreference::Dark | ThemePreference::Light => {
            STANDARD_PALETTE
        }
    }
}

#[must_use]
pub const fn accent(theme: ThemePreference) -> egui::Color32 {
    palette(theme).accent
}

#[must_use]
pub const fn success(theme: ThemePreference) -> egui::Color32 {
    palette(theme).success
}

/// Surface and widget colours of a dark theme.
struct DarkSurfaces {
    panel: egui::Color32,
    window: egui::Color32,
    faint: egui::Color32,
    selection: egui::Color32,
    inactive: egui::Color32,
    hovered: egui::Color32,
    active: egui::Color32,
    inactive_outline: egui::Color32,
    hovered_outline: egui::Color32,
    active_outline: egui::Color32,
}

/// Outlines stay discreet at rest and gain contrast on hover or activation, so
/// radio buttons remain readable without dominating the panel.
const INACTIVE_OUTLINE_WIDTH: f32 = 1.25;
const INTERACTED_OUTLINE_WIDTH: f32 = 1.4;

fn dark_visuals_from(surfaces: &DarkSurfaces) -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = surfaces.panel;
    visuals.window_fill = surfaces.window;
    visuals.faint_bg_color = surfaces.faint;
    visuals.selection.bg_fill = surfaces.selection;
    visuals.widgets.inactive.bg_fill = surfaces.inactive;
    visuals.widgets.hovered.bg_fill = surfaces.hovered;
    visuals.widgets.active.bg_fill = surfaces.active;
    visuals.widgets.inactive.bg_stroke =
        egui::Stroke::new(INACTIVE_OUTLINE_WIDTH, surfaces.inactive_outline);
    visuals.widgets.hovered.bg_stroke =
        egui::Stroke::new(INTERACTED_OUTLINE_WIDTH, surfaces.hovered_outline);
    visuals.widgets.active.bg_stroke =
        egui::Stroke::new(INTERACTED_OUTLINE_WIDTH, surfaces.active_outline);
    visuals
}

fn dark_visuals() -> egui::Visuals {
    dark_visuals_from(&DarkSurfaces {
        panel: egui::Color32::from_rgb(17, 20, 31),
        window: egui::Color32::from_rgb(26, 30, 44),
        faint: egui::Color32::from_rgb(31, 36, 52),
        selection: egui::Color32::from_rgb(64, 104, 165),
        inactive: egui::Color32::from_rgb(27, 32, 47),
        hovered: egui::Color32::from_rgb(45, 54, 77),
        active: egui::Color32::from_rgb(54, 74, 112),
        inactive_outline: egui::Color32::from_rgb(102, 112, 143),
        hovered_outline: egui::Color32::from_rgb(126, 171, 255),
        active_outline: egui::Color32::from_rgb(142, 181, 255),
    })
}

fn catppuccin_visuals() -> egui::Visuals {
    let mut visuals = dark_visuals_from(&DarkSurfaces {
        panel: egui::Color32::from_rgb(24, 24, 37),
        window: egui::Color32::from_rgb(30, 30, 46),
        faint: egui::Color32::from_rgb(49, 50, 68),
        selection: egui::Color32::from_rgb(69, 71, 90),
        inactive: egui::Color32::from_rgb(49, 50, 68),
        hovered: egui::Color32::from_rgb(69, 71, 90),
        active: egui::Color32::from_rgb(88, 91, 112),
        inactive_outline: egui::Color32::from_rgb(108, 112, 134),
        hovered_outline: egui::Color32::from_rgb(137, 180, 250),
        active_outline: egui::Color32::from_rgb(153, 191, 255),
    });
    visuals.hyperlink_color = CATPPUCCIN_PALETTE.accent;
    visuals
}

fn light_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = egui::Color32::from_rgb(247, 249, 253);
    visuals.window_fill = egui::Color32::from_rgb(255, 255, 255);
    visuals.selection.bg_fill = egui::Color32::from_rgb(194, 218, 255);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(227, 237, 255);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(208, 225, 255);
    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_system_and_use_french() {
        let defaults = Preferences::default();
        assert_eq!(defaults.theme, ThemePreference::System);
        assert_eq!(defaults.language, Language::French);
        assert!(defaults.show_toolbar);
        assert!(defaults.show_tooltips);
        assert!(defaults.show_operand_hints);
        assert!(defaults.save_annotations);
        assert!(defaults.persistence_enabled);
    }

    #[test]
    fn dark_themes_use_a_discreet_but_visible_radio_outline() {
        assert!(dark_visuals().widgets.inactive.bg_stroke.width > 1.0);
        assert!(catppuccin_visuals().widgets.inactive.bg_stroke.width > 1.0);
    }

    #[test]
    fn catppuccin_keeps_its_own_accent() {
        assert_eq!(
            accent(ThemePreference::Catppuccin),
            CATPPUCCIN_PALETTE.accent
        );
        assert_eq!(accent(ThemePreference::Dark), STANDARD_PALETTE.accent);
        assert_eq!(
            catppuccin_visuals().hyperlink_color,
            CATPPUCCIN_PALETTE.accent
        );
    }

    /// Captured from a real `app.ron` written by an earlier build. Renaming a
    /// field, a `Command` or a `KeyName` would silently reset everyone's saved
    /// settings, so this fixture must keep loading as-is.
    #[test]
    fn preferences_saved_by_an_earlier_build_still_load() {
        const STORED: &str = "(theme:Catppuccin,language:French,show_toolbar:false,\
             show_tooltips:true,persistence_enabled:true,shortcuts:(overrides:[\
             (command:ToggleToolbar,shortcut:Some((key:P,ctrl:true,shift:false,alt:false)))]))";

        let preferences: Preferences =
            ron::from_str(STORED).expect("stored preferences should still load");
        assert_eq!(preferences.theme, ThemePreference::Catppuccin);
        assert_eq!(preferences.language, Language::French);
        assert!(!preferences.show_toolbar);
        assert_eq!(
            preferences
                .shortcuts
                .shortcut_for(crate::commands::Command::ToggleToolbar),
            Some(crate::commands::Shortcut::ctrl(crate::commands::KeyName::P))
        );
    }

    #[test]
    fn preferences_survive_a_serialisation_round_trip() {
        let preferences = Preferences {
            theme: ThemePreference::Catppuccin,
            language: Language::Spanish,
            show_toolbar: false,
            ..Preferences::default()
        };
        let encoded = ron::to_string(&preferences).expect("preferences should serialise");
        let decoded: Preferences = ron::from_str(&encoded).expect("preferences should deserialise");
        assert_eq!(decoded, preferences);
    }
}
