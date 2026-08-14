use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::i18n::{Language, Text, text};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Command {
    OpenBinary,
    CloseBinary,
    ToggleNavigation,
    ToggleToolbar,
    ToggleTooltips,
    CommandPalette,
    Preferences,
    About,
    Overview,
    Disassembly,
    Functions,
    Strings,
    Patches,
    ToggleExpertMode,
    ThemeSystem,
    ThemeDark,
    ThemeLight,
    ThemeCatppuccin,
    LanguageFrench,
    LanguageEnglish,
    LanguageSpanish,
    TogglePersistence,
}

impl Command {
    pub const ALL: &[Self] = &[
        Self::OpenBinary,
        Self::CloseBinary,
        Self::ToggleNavigation,
        Self::ToggleToolbar,
        Self::ToggleTooltips,
        Self::CommandPalette,
        Self::Preferences,
        Self::About,
        Self::Overview,
        Self::Disassembly,
        Self::Functions,
        Self::Strings,
        Self::Patches,
        Self::ToggleExpertMode,
        Self::ThemeSystem,
        Self::ThemeDark,
        Self::ThemeLight,
        Self::ThemeCatppuccin,
        Self::LanguageFrench,
        Self::LanguageEnglish,
        Self::LanguageSpanish,
        Self::TogglePersistence,
    ];

    pub const fn default_shortcut(self) -> Option<Shortcut> {
        match self {
            Self::OpenBinary => Some(Shortcut::ctrl(KeyName::O)),
            Self::CloseBinary => Some(Shortcut::ctrl(KeyName::W)),
            Self::ToggleNavigation => Some(Shortcut::ctrl(KeyName::B)),
            Self::ToggleToolbar => Some(Shortcut::ctrl_alt(KeyName::T)),
            Self::ToggleTooltips => Some(Shortcut::ctrl_alt(KeyName::I)),
            Self::CommandPalette => Some(Shortcut::ctrl_shift(KeyName::P)),
            Self::Preferences => Some(Shortcut::ctrl(KeyName::Comma)),
            Self::About => Some(Shortcut::plain(KeyName::F1)),
            Self::Overview => Some(Shortcut::ctrl(KeyName::Num1)),
            Self::Disassembly => Some(Shortcut::ctrl(KeyName::Num2)),
            Self::Functions => Some(Shortcut::ctrl(KeyName::Num3)),
            Self::Strings => Some(Shortcut::ctrl(KeyName::Num4)),
            Self::Patches => Some(Shortcut::ctrl(KeyName::Num5)),
            Self::ToggleExpertMode => Some(Shortcut::ctrl_alt(KeyName::E)),
            Self::ThemeSystem
            | Self::ThemeDark
            | Self::ThemeLight
            | Self::ThemeCatppuccin
            | Self::LanguageFrench
            | Self::LanguageEnglish
            | Self::LanguageSpanish
            | Self::TogglePersistence => None,
        }
    }

    pub fn label(self, language: Language) -> String {
        match self {
            Self::OpenBinary => text(language, Text::OpenBinary).to_owned(),
            Self::CloseBinary => text(language, Text::CloseBinary).to_owned(),
            Self::ToggleNavigation => text(language, Text::ToggleMenu).to_owned(),
            Self::ToggleToolbar => text(language, Text::ToggleToolbar).to_owned(),
            Self::ToggleTooltips => text(language, Text::ToggleTooltips).to_owned(),
            Self::CommandPalette => text(language, Text::CommandPalette).to_owned(),
            Self::Preferences => text(language, Text::Preferences).to_owned(),
            Self::About => text(language, Text::About).to_owned(),
            Self::Overview => text(language, Text::Overview).to_owned(),
            Self::Disassembly => text(language, Text::Disassembly).to_owned(),
            Self::Functions => text(language, Text::Functions).to_owned(),
            Self::Strings => text(language, Text::Strings).to_owned(),
            Self::Patches => text(language, Text::Patches).to_owned(),
            Self::ToggleExpertMode => text(language, Text::ToggleExpertMode).to_owned(),
            Self::ThemeSystem => format!(
                "{}: {}",
                text(language, Text::Theme),
                text(language, Text::SystemTheme)
            ),
            Self::ThemeDark => format!(
                "{}: {}",
                text(language, Text::Theme),
                text(language, Text::DarkTheme)
            ),
            Self::ThemeLight => format!(
                "{}: {}",
                text(language, Text::Theme),
                text(language, Text::LightTheme)
            ),
            Self::ThemeCatppuccin => format!(
                "{}: {}",
                text(language, Text::Theme),
                text(language, Text::CatppuccinTheme)
            ),
            Self::LanguageFrench => format!(
                "{}: {}",
                text(language, Text::Language),
                text(language, Text::French)
            ),
            Self::LanguageEnglish => format!(
                "{}: {}",
                text(language, Text::Language),
                text(language, Text::English)
            ),
            Self::LanguageSpanish => format!(
                "{}: {}",
                text(language, Text::Language),
                text(language, Text::Spanish)
            ),
            Self::TogglePersistence => text(language, Text::Persistence).to_owned(),
        }
    }

    pub const fn configurable_shortcut(self) -> bool {
        self.default_shortcut().is_some()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Shortcut {
    pub key: KeyName,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Shortcut {
    pub const fn plain(key: KeyName) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    pub const fn ctrl(key: KeyName) -> Self {
        Self {
            key,
            ctrl: true,
            shift: false,
            alt: false,
        }
    }

    pub const fn ctrl_shift(key: KeyName) -> Self {
        Self {
            key,
            ctrl: true,
            shift: true,
            alt: false,
        }
    }

    pub const fn ctrl_alt(key: KeyName) -> Self {
        Self {
            key,
            ctrl: true,
            shift: false,
            alt: true,
        }
    }

    pub fn label(self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        parts.push(self.key.label());
        parts.join("+")
    }

    pub fn capture(ctx: &egui::Context) -> Option<Self> {
        ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => KeyName::from_egui(*key).map(|key| Self {
                    key,
                    ctrl: modifiers.ctrl || modifiers.command,
                    shift: modifiers.shift,
                    alt: modifiers.alt,
                }),
                _ => None,
            })
        })
    }

    pub fn pressed(self, ctx: &egui::Context) -> bool {
        ctx.input(|input| {
            input.events.iter().any(|event| match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    KeyName::from_egui(*key) == Some(self.key)
                        && (modifiers.ctrl || modifiers.command) == self.ctrl
                        && modifiers.shift == self.shift
                        && modifiers.alt == self.alt
                }
                _ => false,
            })
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum KeyName {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    Comma,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

impl KeyName {
    pub const fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::E => "E",
            Self::F => "F",
            Self::G => "G",
            Self::H => "H",
            Self::I => "I",
            Self::J => "J",
            Self::K => "K",
            Self::L => "L",
            Self::M => "M",
            Self::N => "N",
            Self::O => "O",
            Self::P => "P",
            Self::Q => "Q",
            Self::R => "R",
            Self::S => "S",
            Self::T => "T",
            Self::U => "U",
            Self::V => "V",
            Self::W => "W",
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
            Self::Num0 => "0",
            Self::Num1 => "1",
            Self::Num2 => "2",
            Self::Num3 => "3",
            Self::Num4 => "4",
            Self::Num5 => "5",
            Self::Num6 => "6",
            Self::Num7 => "7",
            Self::Num8 => "8",
            Self::Num9 => "9",
            Self::Comma => ",",
            Self::F1 => "F1",
            Self::F2 => "F2",
            Self::F3 => "F3",
            Self::F4 => "F4",
            Self::F5 => "F5",
            Self::F6 => "F6",
            Self::F7 => "F7",
            Self::F8 => "F8",
            Self::F9 => "F9",
            Self::F10 => "F10",
            Self::F11 => "F11",
            Self::F12 => "F12",
        }
    }

    pub const fn from_egui(key: egui::Key) -> Option<Self> {
        match key {
            egui::Key::A => Some(Self::A),
            egui::Key::B => Some(Self::B),
            egui::Key::C => Some(Self::C),
            egui::Key::D => Some(Self::D),
            egui::Key::E => Some(Self::E),
            egui::Key::F => Some(Self::F),
            egui::Key::G => Some(Self::G),
            egui::Key::H => Some(Self::H),
            egui::Key::I => Some(Self::I),
            egui::Key::J => Some(Self::J),
            egui::Key::K => Some(Self::K),
            egui::Key::L => Some(Self::L),
            egui::Key::M => Some(Self::M),
            egui::Key::N => Some(Self::N),
            egui::Key::O => Some(Self::O),
            egui::Key::P => Some(Self::P),
            egui::Key::Q => Some(Self::Q),
            egui::Key::R => Some(Self::R),
            egui::Key::S => Some(Self::S),
            egui::Key::T => Some(Self::T),
            egui::Key::U => Some(Self::U),
            egui::Key::V => Some(Self::V),
            egui::Key::W => Some(Self::W),
            egui::Key::X => Some(Self::X),
            egui::Key::Y => Some(Self::Y),
            egui::Key::Z => Some(Self::Z),
            egui::Key::Num0 => Some(Self::Num0),
            egui::Key::Num1 => Some(Self::Num1),
            egui::Key::Num2 => Some(Self::Num2),
            egui::Key::Num3 => Some(Self::Num3),
            egui::Key::Num4 => Some(Self::Num4),
            egui::Key::Num5 => Some(Self::Num5),
            egui::Key::Num6 => Some(Self::Num6),
            egui::Key::Num7 => Some(Self::Num7),
            egui::Key::Num8 => Some(Self::Num8),
            egui::Key::Num9 => Some(Self::Num9),
            egui::Key::Comma => Some(Self::Comma),
            egui::Key::F1 => Some(Self::F1),
            egui::Key::F2 => Some(Self::F2),
            egui::Key::F3 => Some(Self::F3),
            egui::Key::F4 => Some(Self::F4),
            egui::Key::F5 => Some(Self::F5),
            egui::Key::F6 => Some(Self::F6),
            egui::Key::F7 => Some(Self::F7),
            egui::Key::F8 => Some(Self::F8),
            egui::Key::F9 => Some(Self::F9),
            egui::Key::F10 => Some(Self::F10),
            egui::Key::F11 => Some(Self::F11),
            egui::Key::F12 => Some(Self::F12),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShortcutBinding {
    pub command: Command,
    /// `None` explicitly disables a built-in shortcut after another command
    /// receives the same key combination.
    pub shortcut: Option<Shortcut>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShortcutBindings {
    pub overrides: Vec<ShortcutBinding>,
}

impl ShortcutBindings {
    pub fn shortcut_for(&self, command: Command) -> Option<Shortcut> {
        self.overrides
            .iter()
            .find(|binding| binding.command == command)
            .map(|binding| binding.shortcut)
            .unwrap_or_else(|| command.default_shortcut())
    }

    pub fn set(&mut self, command: Command, shortcut: Shortcut) {
        let displaced = Command::ALL
            .iter()
            .copied()
            .filter(|other| *other != command && self.shortcut_for(*other) == Some(shortcut))
            .collect::<Vec<_>>();

        self.overrides
            .retain(|binding| binding.command != command && !displaced.contains(&binding.command));
        self.overrides
            .extend(displaced.into_iter().map(|command| ShortcutBinding {
                command,
                shortcut: None,
            }));
        self.overrides.push(ShortcutBinding {
            command,
            shortcut: Some(shortcut),
        });
    }

    pub fn reset(&mut self) {
        self.overrides.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_primary_command_has_a_default_shortcut() {
        for command in [
            Command::OpenBinary,
            Command::CloseBinary,
            Command::ToggleNavigation,
            Command::ToggleToolbar,
            Command::ToggleTooltips,
            Command::CommandPalette,
            Command::Preferences,
            Command::Overview,
            Command::Disassembly,
            Command::Functions,
            Command::Strings,
            Command::Patches,
            Command::ToggleExpertMode,
        ] {
            assert!(command.default_shortcut().is_some());
        }
    }

    #[test]
    fn custom_shortcuts_replace_conflicts() {
        let mut bindings = ShortcutBindings::default();
        bindings.set(Command::CommandPalette, Shortcut::ctrl(KeyName::O));
        assert_eq!(bindings.shortcut_for(Command::OpenBinary), None);
        assert_eq!(
            bindings.shortcut_for(Command::CommandPalette),
            Some(Shortcut::ctrl(KeyName::O))
        );
    }
}
