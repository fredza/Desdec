use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::{
    app::WorkspaceView,
    i18n::{Language, Text, text},
};

/// Declares the command registry once.
///
/// Every user-triggered feature must have an entry here, so the command palette
/// remains a complete, keyboard-accessible index of the application.
///
/// Each entry names the [`Text`] fragments forming its visible label — several
/// fragments are joined with `: ` — and its factory-default shortcut. The macro
/// derives the enum, the ordered [`Command::ALL`] list used by the palette and
/// the preferences page, and both lookup tables, so a command can never exist
/// without a label or be missing from the registry.
macro_rules! commands {
    ($($variant:ident => [$($label:ident),+], $shortcut:expr),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
        #[expect(
            clippy::enum_variant_names,
            reason = "variant names are persisted by serde and must stay stable"
        )]
        pub enum Command {
            $($variant,)+
        }

        impl Command {
            pub const ALL: &[Self] = &[$(Self::$variant,)+];

            const fn labels(self) -> &'static [Text] {
                match self {
                    $(Self::$variant => &[$(Text::$label,)+],)+
                }
            }

            #[must_use]
            pub const fn default_shortcut(self) -> Option<Shortcut> {
                match self {
                    $(Self::$variant => $shortcut,)+
                }
            }
        }
    };
}

commands! {
    OpenBinary => [OpenBinary], Some(Shortcut::ctrl(KeyName::O)),
    // Named for the whole opening, which is what it abandons: the dialog
    // waiting on a choice as well as the analysis of what was chosen.
    CancelAnalysis => [CancelOpening], None,
    CloseBinary => [CloseBinary], Some(Shortcut::ctrl(KeyName::W)),
    ToggleNavigation => [ToggleMenu], Some(Shortcut::ctrl(KeyName::B)),
    ToggleToolbar => [ToggleToolbar], Some(Shortcut::ctrl_alt(KeyName::T)),
    ToggleTooltips => [ToggleTooltips], Some(Shortcut::ctrl_alt(KeyName::I)),
    CommandPalette => [CommandPalette], Some(Shortcut::ctrl_shift(KeyName::P)),
    Preferences => [Preferences], Some(Shortcut::ctrl(KeyName::Comma)),
    // The account of the session: what was opened, what was written, what
    // failed. A window rather than a view, because it is about the
    // application rather than about the file.
    Output => [Output], Some(Shortcut::ctrl(KeyName::L)),
    About => [About], Some(Shortcut::plain(KeyName::F1)),
    Overview => [Overview], Some(Shortcut::ctrl(KeyName::Num1)),
    Disassembly => [Disassembly], Some(Shortcut::ctrl(KeyName::Num2)),
    // The transport of the static walk. Debugger keys, because that is what a
    // reader's fingers already know — F7 steps in, F8 steps over — even though
    // nothing here runs: the walk follows the flow, it does not execute it.
    WalkStepInto => [Disassembly, StepInto], Some(Shortcut::plain(KeyName::F7)),
    WalkStepOver => [Disassembly, StepOver], Some(Shortcut::plain(KeyName::F8)),
    WalkStepOut => [Disassembly, StepOut], Some(Shortcut::shift(KeyName::F8)),
    WalkBack => [Disassembly, StepBack], Some(Shortcut::shift(KeyName::F7)),
    WalkToEntry => [Disassembly, WalkToEntry], None,
    DisassemblyStartEntry => [Disassembly, EntryPoint], None,
    DisassemblyStartMain => [Disassembly, MainFunction], None,
    DisassemblyStartProbable => [Disassembly, ProbableFunction], None,
    // The reader's own reading of the file: a name for an address, a sentence
    // about it, a mark to come back to.
    // Who names an address, and finding what the listing will not scroll to.
    References => [Disassembly, References], Some(Shortcut::ctrl(KeyName::R)),
    Search => [Search], Some(Shortcut::ctrl(KeyName::F)),
    EditAnnotation => [Disassembly, EditNote], Some(Shortcut::plain(KeyName::F2)),
    ToggleBookmark => [Disassembly, Bookmark], Some(Shortcut::ctrl(KeyName::D)),
    WalkClear => [Disassembly, WalkClear], None,
    Decompile => [Decompile], Some(Shortcut::ctrl_shift(KeyName::D)),
    // The bytes themselves, sixteen to a row.
    Dump => [Dump], Some(Shortcut::ctrl(KeyName::Num7)),
    AiAssistance => [AiAssistance], Some(Shortcut::ctrl_alt(KeyName::A)),
    AskAboutBinary => [AiAssistance, AskAboutBinary], None,
    AskAboutFunction => [AiAssistance, AskAboutFunction], None,
    AskAboutInstruction => [AiAssistance, AskAboutInstruction], None,
    Functions => [Functions], Some(Shortcut::ctrl(KeyName::Num3)),
    Strings => [Strings], Some(Shortcut::ctrl(KeyName::Num4)),
    StringsScopeAll => [Strings, StringsScopeAll], None,
    StringsScopeUsed => [Strings, StringsScopeUsed], None,
    StringsScopeMappedUnreferenced => [Strings, StringsScopeMappedUnreferenced], None,
    StringsScopeUnmapped => [Strings, StringsScopeUnmapped], None,
    StringsClearFilter => [Strings, ClearFilter], None,
    Symbols => [Symbols], None,
    Classes => [Classes], None,
    Patches => [Patches], Some(Shortcut::ctrl(KeyName::Num5)),
    Segments => [Segments], Some(Shortcut::ctrl(KeyName::Num6)),
    ExportPatched => [Patches, ExportPatched], None,
    // The reader's own work, in a file beside the binary. Ctrl+S is what every
    // application on the machine binds to saving, and nothing here used it.
    SaveSession => [Session, SaveSession], Some(Shortcut::ctrl(KeyName::S)),
    OpenSession => [Session, OpenSession], None,
    DiscardPatches => [Patches, DiscardPatches], None,
    DecompilerBuiltin => [Decompiler, BuiltinDecompiler], None,
    DecompilerRzGhidra => [Decompiler, RzGhidraEngine], None,
    DecompilerRetDec => [Decompiler, RetDecEngine], None,
    RerunDecompilation => [Decompiler, RerunDecompiler], None,
    ShowDecompilationAssembly => [Decompiler, AssemblyPreview], None,
    CopyPseudoCode => [Decompiler, CopyPseudoCode], None,
    DecompilerPreferences => [Decompiler, Preferences], None,
    ToggleDecompilationCache => [Decompiler, CacheDecompilations], None,
    ClearDecompilationCache => [Decompiler, ClearCache], None,
    ThemeSystem => [Theme, SystemTheme], None,
    ThemeDark => [Theme, DarkTheme], None,
    ThemeLight => [Theme, LightTheme], None,
    ThemeCatppuccin => [Theme, CatppuccinTheme], None,
    ThemeAbyss => [Theme, AbyssTheme], None,
    LanguageFrench => [Language, French], None,
    LanguageEnglish => [Language, English], None,
    LanguageSpanish => [Language, Spanish], None,
    TogglePersistence => [Persistence], None,
    // The reader's own rule, written once and run over the whole file. Its own
    // window rather than a view: a script is written about the listing behind
    // it, and both have to be on screen at once.
    Script => [Script], Some(Shortcut::ctrl_shift(KeyName::S)),
    RunScript => [Script, RunScript], Some(Shortcut::plain(KeyName::F5)),
    Plugins => [Plugins], None,
    ReloadPlugins => [Plugins, ReloadPlugins], None,
    // The emulated processor. Its keys are the ones a debugger has had since
    // Turbo Debugger — F9 runs, F11 steps into, F10 steps over — and they are
    // deliberately not the walk's F7 and F8: one follows the flow by reading,
    // the other carries it out, and a reader must never press one meaning the
    // other.
    Machine => [Machine], Some(Shortcut::ctrl(KeyName::Num8)),
    MachineRun => [Machine, Run], Some(Shortcut::plain(KeyName::F9)),
    MachineStepInto => [Machine, StepInto], Some(Shortcut::plain(KeyName::F11)),
    MachineStepOver => [Machine, StepOver], Some(Shortcut::plain(KeyName::F10)),
    MachineStepOut => [Machine, StepOut], Some(Shortcut::shift(KeyName::F11)),
    MachineRunToCursor => [Machine, RunToCursor], Some(Shortcut::ctrl(KeyName::F10)),
    MachineRestart => [Machine, Restart], Some(Shortcut::ctrl(KeyName::F9)),
    // The one a debugger attached to a process cannot offer at all.
    MachineStepBack => [Machine, StepBackOne], Some(Shortcut::ctrl(KeyName::F11)),
    MachineToggleBreakpoint => [Machine, ToggleBreakpoint], Some(Shortcut::ctrl(KeyName::F2)),
    // Running until the state says so, rather than pressing a step key until
    // it does. x64dbg's conditional trace, over the same language its
    // breakpoint conditions are written in.
    MachineTraceUntil => [Machine, TraceUntil], Some(Shortcut::ctrl_shift(KeyName::F11)),
    // One function drawn as its control flow. Its own view rather than a pane,
    // because a graph wants the whole workspace.
    Graph => [Graph], Some(Shortcut::ctrl(KeyName::Num9)),
    // The reader's own arithmetic over the machine's state: x64dbg's
    // calculator, in the language its conditions are already written in.
    Expression => [Expression], Some(Shortcut::ctrl_shift(KeyName::E)),
    // What the bytes at an address mean, which is the one thing the file does
    // not state and the reader does know. Ctrl+0 because it sits at the end of
    // the row of view keys, where a tenth view has to go.
    Structures => [Structures], Some(Shortcut::ctrl(KeyName::Num0)),
    // Looking for a newer release. Never on a shortcut by default: it reaches
    // the network, and a key that does that should be one the reader chose.
    CheckForUpdates => [Updates, CheckForUpdates], None,
    // Handing a function to an assembler IDE. No default key: it writes a file
    // and starts another program, and a reader should have chosen the key that
    // does that.
    SendToAsmStudio => [Disassembly, SendToAsmStudio], None,
    Yara => [Yara], None,
    RunYara => [Yara, RunYara], None,
    ToggleYaraModule => [Yara, ToggleYaraModule], None,
}

impl Command {
    #[must_use]
    pub fn label(self, language: Language) -> String {
        self.labels()
            .iter()
            .map(|item| text(language, *item))
            .collect::<Vec<_>>()
            .join(": ")
    }

    /// Whether a key combination can be assigned to this command.
    ///
    /// Everything the application actually does, not only what shipped with a
    /// default: a command with no factory shortcut used to be one no reader
    /// could ever give a key to, which put half the registry — every theme,
    /// every language, the exports, the string filters — out of reach of the
    /// keyboard for good. The one exception is a command that does nothing
    /// yet: a key bound to it would swallow the press and answer nothing.
    #[must_use]
    pub const fn configurable_shortcut(self) -> bool {
        self.implemented()
    }

    /// Whether the command is implemented at all.
    ///
    /// Separate from whether it can run *right now*, which depends on the
    /// state and is answered by [`crate::app::DesdecApp::can_run`]. The
    /// palette lists everything either way, so what exists stays visible, but
    /// it never lets an entry be chosen that would answer nothing: a highlight
    /// on which `Enter` does nothing reads as a broken palette.
    ///
    /// Everything is implemented today. The distinction is kept because the
    /// palette is written against it: the next command to be sketched before
    /// it works has somewhere to say so.
    #[must_use]
    pub const fn implemented(self) -> bool {
        true
    }

    /// A sentence about what the command does, for the few whose label leaves
    /// the question open.
    ///
    /// Deliberately not one per command: a list where every entry carries a
    /// paragraph is a list nobody reads, and "Open binary" explains itself.
    /// These two do not — what a `.dcl` is, and that Desdec already keeps the
    /// notes without one, is exactly what a reader meeting the entry does not
    /// know.
    #[must_use]
    pub const fn explanation(self) -> Option<crate::i18n::Text> {
        match self {
            Self::SaveSession => Some(crate::i18n::Text::SaveSessionHint),
            Self::OpenSession => Some(crate::i18n::Text::OpenSessionHint),
            _ => None,
        }
    }

    /// Whether the command acts on a loaded binary, and so needs one.
    ///
    /// Switching view is deliberately not in this list: with no binary open a
    /// view says so plainly, which is a visible answer rather than a keystroke
    /// that vanished, and the palette should never lock out navigation.
    #[must_use]
    pub const fn needs_a_binary(self) -> bool {
        matches!(
            self,
            Self::CloseBinary
                | Self::ExportPatched
                // Both act on the binary open: there is nothing to save the
                // work on, and nowhere to read it back beside, without one.
                | Self::SaveSession
                | Self::OpenSession
                | Self::DiscardPatches
                | Self::RunYara
                | Self::AskAboutBinary
                | Self::AskAboutFunction
                | Self::AskAboutInstruction
                | Self::StringsScopeAll
                | Self::StringsScopeUsed
                | Self::StringsScopeMappedUnreferenced
                | Self::StringsScopeUnmapped
                | Self::StringsClearFilter
                | Self::WalkStepInto
                | Self::WalkStepOver
                | Self::WalkStepOut
                | Self::WalkBack
                | Self::WalkToEntry
                | Self::WalkClear
                | Self::EditAnnotation
                | Self::ToggleBookmark
                | Self::References
                | Self::Search
                | Self::RunScript
                | Self::MachineRun
                | Self::MachineStepInto
                | Self::MachineStepOver
                | Self::MachineStepOut
                | Self::MachineRunToCursor
                | Self::MachineRestart
                | Self::MachineStepBack
                | Self::MachineToggleBreakpoint
                | Self::RerunDecompilation
                | Self::ShowDecompilationAssembly
                | Self::CopyPseudoCode
        )
    }

    /// Whether the command acts on the pending patches, and so needs some.
    #[must_use]
    pub const fn needs_patches(self) -> bool {
        matches!(self, Self::ExportPatched | Self::DiscardPatches)
    }

    /// The workspace view this command opens, if it opens one.
    ///
    /// Declared rather than discovered by running the command: a test that
    /// ran every command to see where each one led also ran the ones that
    /// open a file dialog, which put seven of them on the user's screen.
    #[must_use]
    pub const fn opens_view(self) -> Option<WorkspaceView> {
        Some(match self {
            Self::Overview => WorkspaceView::Overview,
            Self::Segments => WorkspaceView::Segments,
            Self::Functions => WorkspaceView::Functions,
            Self::Strings
            | Self::StringsScopeAll
            | Self::StringsScopeUsed
            | Self::StringsScopeMappedUnreferenced
            | Self::StringsScopeUnmapped
            | Self::StringsClearFilter => WorkspaceView::Strings,
            Self::Symbols => WorkspaceView::Symbols,
            Self::Classes => WorkspaceView::Classes,
            // The walk moves the selection in the listing, so it shows it.
            Self::Dump => WorkspaceView::Dump,
            Self::Disassembly
            | Self::WalkStepInto
            | Self::WalkStepOver
            | Self::WalkStepOut
            | Self::WalkBack
            | Self::WalkToEntry
            | Self::WalkClear
            | Self::EditAnnotation
            | Self::ToggleBookmark
            // Running to the cursor and setting a breakpoint are both about a
            // row of the listing, so they leave the reader looking at it.
            | Self::MachineRunToCursor
            | Self::MachineToggleBreakpoint => WorkspaceView::Disassembly,
            Self::Decompile
            | Self::RerunDecompilation
            | Self::ShowDecompilationAssembly
            | Self::CopyPseudoCode => WorkspaceView::Decompile,
            Self::AiAssistance
            | Self::AskAboutBinary
            | Self::AskAboutFunction
            | Self::AskAboutInstruction => WorkspaceView::Assistant,
            // Exporting shows the patches it is about to write.
            Self::Patches | Self::ExportPatched => WorkspaceView::Patches,
            Self::Yara => WorkspaceView::Yara,
            Self::Machine
            | Self::MachineRun
            | Self::MachineStepInto
            | Self::MachineStepOver
            | Self::MachineStepOut
            | Self::MachineRestart
            | Self::MachineStepBack
            // A conditional trace is a run: it leaves the reader looking at
            // where the run now stands.
            | Self::MachineTraceUntil => WorkspaceView::Machine,
            Self::Graph => WorkspaceView::Graph,
            Self::Structures => WorkspaceView::Structures,
            _ => return None,
        })
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
            ctrl: true,
            ..Self::plain(key)
        }
    }

    pub const fn shift(key: KeyName) -> Self {
        Self {
            shift: true,
            ..Self::plain(key)
        }
    }

    pub const fn ctrl_shift(key: KeyName) -> Self {
        Self {
            shift: true,
            ..Self::ctrl(key)
        }
    }

    pub const fn ctrl_alt(key: KeyName) -> Self {
        Self {
            alt: true,
            ..Self::ctrl(key)
        }
    }

    #[must_use]
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

    /// Reads the first key press of the frame, whatever the combination.
    pub fn capture(ctx: &egui::Context) -> Option<Self> {
        ctx.input(|input| input.events.iter().find_map(Self::from_event))
    }

    pub fn pressed(self, ctx: &egui::Context) -> bool {
        ctx.input(|input| {
            input
                .events
                .iter()
                .filter_map(Self::from_event)
                .any(|pressed| pressed == self)
        })
    }

    fn from_event(event: &egui::Event) -> Option<Self> {
        match event {
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
        }
    }
}

/// Declares the bindable keys once, reusing the `egui` variant names.
macro_rules! keys {
    ($($variant:ident => $label:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
        pub enum KeyName {
            $($variant,)+
        }

        impl KeyName {
            #[must_use]
            pub const fn label(self) -> &'static str {
                match self {
                    $(Self::$variant => $label,)+
                }
            }

            #[must_use]
            pub const fn from_egui(key: egui::Key) -> Option<Self> {
                match key {
                    $(egui::Key::$variant => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

keys! {
    A => "A", B => "B", C => "C", D => "D", E => "E", F => "F", G => "G",
    H => "H", I => "I", J => "J", K => "K", L => "L", M => "M", N => "N",
    O => "O", P => "P", Q => "Q", R => "R", S => "S", T => "T", U => "U",
    V => "V", W => "W", X => "X", Y => "Y", Z => "Z",
    Num0 => "0", Num1 => "1", Num2 => "2", Num3 => "3", Num4 => "4",
    Num5 => "5", Num6 => "6", Num7 => "7", Num8 => "8", Num9 => "9",
    Comma => ",",
    F1 => "F1", F2 => "F2", F3 => "F3", F4 => "F4", F5 => "F5", F6 => "F6",
    F7 => "F7", F8 => "F8", F9 => "F9", F10 => "F10", F11 => "F11",
    F12 => "F12",
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
    #[must_use]
    pub fn shortcut_for(&self, command: Command) -> Option<Shortcut> {
        self.overrides
            .iter()
            .find(|binding| binding.command == command)
            .map_or_else(|| command.default_shortcut(), |binding| binding.shortcut)
    }

    /// Assigns `shortcut` to `command`, explicitly disabling every other
    /// command that used the same combination.
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

    /// Takes the key combination away from `command`, default included.
    ///
    /// Recorded as an explicit `None` rather than by forgetting the override:
    /// forgetting it would bring the factory shortcut back, which is the
    /// opposite of what removing one means.
    pub fn clear(&mut self, command: Command) {
        self.overrides.retain(|binding| binding.command != command);
        self.overrides.push(ShortcutBinding {
            command,
            shortcut: None,
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
        ] {
            assert!(command.default_shortcut().is_some());
        }
    }

    /// The palette is meant to be a complete index of the application, so
    /// every button a view offers has a command behind it — and the two are
    /// worded the same, or the palette lists an action nobody can find.
    #[test]
    fn every_action_a_view_offers_is_also_a_command_worded_the_same() {
        for (command, button) in [
            (Command::AskAboutBinary, Text::AskAboutBinary),
            (Command::AskAboutFunction, Text::AskAboutFunction),
            (Command::AskAboutInstruction, Text::AskAboutInstruction),
            (Command::StringsScopeUsed, Text::StringsScopeUsed),
            (
                Command::StringsScopeMappedUnreferenced,
                Text::StringsScopeMappedUnreferenced,
            ),
            (Command::StringsClearFilter, Text::ClearFilter),
            (Command::RunYara, Text::RunYara),
            (Command::ExportPatched, Text::ExportPatched),
        ] {
            for language in Language::ALL {
                let label = command.label(*language);
                assert!(
                    label.contains(text(*language, button)),
                    "{command:?} is not worded like the button that does the same thing: {label}"
                );
            }
        }
    }

    /// Every command the application can actually run must be reachable from
    /// the keyboard, whether or not it shipped with a key of its own.
    #[test]
    fn every_working_command_can_be_given_a_shortcut() {
        for command in Command::ALL.iter().copied().filter(|c| c.implemented()) {
            assert!(
                command.configurable_shortcut(),
                "{command:?} can run but can never be given a key"
            );
        }
    }

    /// Removing a shortcut must remove it, not fall back on the factory one.
    #[test]
    fn a_cleared_shortcut_does_not_come_back_as_the_default() {
        let mut bindings = ShortcutBindings::default();
        assert!(bindings.shortcut_for(Command::OpenBinary).is_some());

        bindings.clear(Command::OpenBinary);

        assert_eq!(bindings.shortcut_for(Command::OpenBinary), None);
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

    #[test]
    fn default_shortcuts_are_unique() {
        for command in Command::ALL {
            let Some(shortcut) = command.default_shortcut() else {
                continue;
            };
            let owners = Command::ALL
                .iter()
                .filter(|other| other.default_shortcut() == Some(shortcut))
                .count();
            assert_eq!(owners, 1, "{command:?} shares {}", shortcut.label());
        }
    }

    #[test]
    fn every_command_is_labelled_in_every_language() {
        for language in Language::ALL {
            for command in Command::ALL {
                assert!(!command.label(*language).is_empty());
            }
        }
    }

    #[test]
    fn composed_labels_join_their_fragments() {
        assert_eq!(
            Command::ThemeDark.label(Language::English),
            "Theme: Dark".to_owned()
        );
    }

    #[test]
    fn shortcut_constructors_keep_their_modifiers() {
        assert_eq!(Shortcut::ctrl_shift(KeyName::P).label(), "Ctrl+Shift+P");
        assert_eq!(Shortcut::ctrl_alt(KeyName::E).label(), "Ctrl+Alt+E");
        assert_eq!(Shortcut::plain(KeyName::F1).label(), "F1");
    }
}
