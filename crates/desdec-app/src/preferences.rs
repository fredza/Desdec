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

pub const fn accent(theme: ThemePreference) -> egui::Color32 {
    match theme {
        ThemePreference::Catppuccin => egui::Color32::from_rgb(137, 180, 250),
        ThemePreference::System | ThemePreference::Dark | ThemePreference::Light => {
            egui::Color32::from_rgb(126, 171, 255)
        }
    }
}

pub const fn success(theme: ThemePreference) -> egui::Color32 {
    match theme {
        ThemePreference::Catppuccin => egui::Color32::from_rgb(166, 227, 161),
        ThemePreference::System | ThemePreference::Dark | ThemePreference::Light => {
            egui::Color32::from_rgb(91, 201, 139)
        }
    }
}

fn dark_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_rgb(17, 20, 31);
    visuals.window_fill = egui::Color32::from_rgb(26, 30, 44);
    visuals.faint_bg_color = egui::Color32::from_rgb(31, 36, 52);
    visuals.selection.bg_fill = egui::Color32::from_rgb(64, 104, 165);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(27, 32, 47);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(45, 54, 77);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(54, 74, 112);
    visuals.widgets.inactive.bg_stroke =
        egui::Stroke::new(1.25_f32, egui::Color32::from_rgb(102, 112, 143));
    visuals.widgets.hovered.bg_stroke =
        egui::Stroke::new(1.4_f32, egui::Color32::from_rgb(126, 171, 255));
    visuals.widgets.active.bg_stroke =
        egui::Stroke::new(1.4_f32, egui::Color32::from_rgb(142, 181, 255));
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

fn catppuccin_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_rgb(24, 24, 37);
    visuals.window_fill = egui::Color32::from_rgb(30, 30, 46);
    visuals.faint_bg_color = egui::Color32::from_rgb(49, 50, 68);
    visuals.selection.bg_fill = egui::Color32::from_rgb(69, 71, 90);
    visuals.hyperlink_color = egui::Color32::from_rgb(137, 180, 250);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(49, 50, 68);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(69, 71, 90);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(88, 91, 112);
    visuals.widgets.inactive.bg_stroke =
        egui::Stroke::new(1.25_f32, egui::Color32::from_rgb(108, 112, 134));
    visuals.widgets.hovered.bg_stroke =
        egui::Stroke::new(1.4_f32, egui::Color32::from_rgb(137, 180, 250));
    visuals.widgets.active.bg_stroke =
        egui::Stroke::new(1.4_f32, egui::Color32::from_rgb(153, 191, 255));
    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_system_and_use_french() {
        assert_eq!(Preferences::default().theme, ThemePreference::System);
        assert_eq!(Preferences::default().language, Language::French);
        assert!(Preferences::default().show_toolbar);
        assert!(Preferences::default().show_tooltips);
        assert!(Preferences::default().persistence_enabled);
    }

    #[test]
    fn dark_themes_use_a_discreet_but_visible_radio_outline() {
        assert!(dark_visuals().widgets.inactive.bg_stroke.width > 1.0);
        assert!(catppuccin_visuals().widgets.inactive.bg_stroke.width > 1.0);
    }
}
