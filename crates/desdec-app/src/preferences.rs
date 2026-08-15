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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: ThemePreference,
    pub language: Language,
    pub show_toolbar: bool,
    pub show_tooltips: bool,
    pub persistence_enabled: bool,
    pub ai_assistance: bool,
    pub shortcuts: ShortcutBindings,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            language: Language::French,
            show_toolbar: true,
            show_tooltips: true,
            persistence_enabled: true,
            ai_assistance: false,
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
