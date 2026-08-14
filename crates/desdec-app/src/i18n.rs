use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    French,
    English,
    Spanish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Text {
    GuidedAnalysis,
    Menu,
    OpenBinary,
    CloseBinary,
    Exploration,
    Tools,
    CommandPalette,
    Preferences,
    About,
    CollapseMenu,
    MenuHint,
    ReadyToOpen,
    NoPatches,
    Guided,
    Expert,
    Overview,
    Segments,
    Functions,
    Strings,
    Disassembly,
    Patches,
    StartAnalysis,
    DropFile,
    MenuAvailable,
    LegalNotice,
    ActiveFile,
    Path,
    Format,
    Architecture,
    Size,
    NextStep,
    NextStepDetail,
    OpenFirst,
    ComingSoon,
    SegmentsInfo,
    FunctionsInfo,
    StringsInfo,
    DisassemblyInfo,
    PatchesInfo,
    GuidedHelp,
    PaletteTitle,
    SearchAction,
    SearchHint,
    Appearance,
    Theme,
    Language,
    SystemTheme,
    DarkTheme,
    LightTheme,
    CatppuccinTheme,
    FreeExtensions,
    FreeExtensionsInfo,
    PreferencesInfo,
    AboutTitle,
    AboutDescription,
    CannotInspect,
    Shortcuts,
    Behaviour,
    ShowToolbar,
    ShowTooltips,
    Persistence,
    PersistenceInfo,
    ToggleMenu,
    ToggleToolbar,
    ToggleTooltips,
    ToggleExpertMode,
    Modify,
    ResetDefaults,
    PressShortcut,
    Cancel,
    NoShortcut,
    French,
    English,
    Spanish,
}

#[cfg(test)]
pub const ALL_TEXT: &[Text] = &[
    Text::GuidedAnalysis,
    Text::Menu,
    Text::OpenBinary,
    Text::CloseBinary,
    Text::Exploration,
    Text::Tools,
    Text::CommandPalette,
    Text::Preferences,
    Text::About,
    Text::CollapseMenu,
    Text::MenuHint,
    Text::ReadyToOpen,
    Text::NoPatches,
    Text::Guided,
    Text::Expert,
    Text::Overview,
    Text::Segments,
    Text::Functions,
    Text::Strings,
    Text::Disassembly,
    Text::Patches,
    Text::StartAnalysis,
    Text::DropFile,
    Text::MenuAvailable,
    Text::LegalNotice,
    Text::ActiveFile,
    Text::Path,
    Text::Format,
    Text::Architecture,
    Text::Size,
    Text::NextStep,
    Text::NextStepDetail,
    Text::OpenFirst,
    Text::ComingSoon,
    Text::SegmentsInfo,
    Text::FunctionsInfo,
    Text::StringsInfo,
    Text::DisassemblyInfo,
    Text::PatchesInfo,
    Text::GuidedHelp,
    Text::PaletteTitle,
    Text::SearchAction,
    Text::SearchHint,
    Text::Appearance,
    Text::Theme,
    Text::Language,
    Text::SystemTheme,
    Text::DarkTheme,
    Text::LightTheme,
    Text::CatppuccinTheme,
    Text::FreeExtensions,
    Text::FreeExtensionsInfo,
    Text::PreferencesInfo,
    Text::AboutTitle,
    Text::AboutDescription,
    Text::CannotInspect,
    Text::Shortcuts,
    Text::Behaviour,
    Text::ShowToolbar,
    Text::ShowTooltips,
    Text::Persistence,
    Text::PersistenceInfo,
    Text::ToggleMenu,
    Text::ToggleToolbar,
    Text::ToggleTooltips,
    Text::ToggleExpertMode,
    Text::Modify,
    Text::ResetDefaults,
    Text::PressShortcut,
    Text::Cancel,
    Text::NoShortcut,
    Text::French,
    Text::English,
    Text::Spanish,
];

pub const fn text(language: Language, item: Text) -> &'static str {
    match (language, item) {
        (Language::French, Text::GuidedAnalysis) => "analyse guidée",
        (Language::English, Text::GuidedAnalysis) => "guided analysis",
        (Language::Spanish, Text::GuidedAnalysis) => "análisis guiado",
        (Language::French, Text::Menu) => "Menu",
        (Language::English, Text::Menu) => "Menu",
        (Language::Spanish, Text::Menu) => "Menú",
        (Language::French, Text::OpenBinary) => "Ouvrir un binaire",
        (Language::English, Text::OpenBinary) => "Open binary",
        (Language::Spanish, Text::OpenBinary) => "Abrir un binario",
        (Language::French, Text::CloseBinary) => "Fermer le binaire",
        (Language::English, Text::CloseBinary) => "Close binary",
        (Language::Spanish, Text::CloseBinary) => "Cerrar el binario",
        (Language::French, Text::Exploration) => "EXPLORATION",
        (Language::English, Text::Exploration) => "EXPLORATION",
        (Language::Spanish, Text::Exploration) => "EXPLORACIÓN",
        (Language::French, Text::Tools) => "OUTILS",
        (Language::English, Text::Tools) => "TOOLS",
        (Language::Spanish, Text::Tools) => "HERRAMIENTAS",
        (Language::French, Text::CommandPalette) => "Palette de commandes",
        (Language::English, Text::CommandPalette) => "Command palette",
        (Language::Spanish, Text::CommandPalette) => "Paleta de comandos",
        (Language::French, Text::Preferences) => "Préférences",
        (Language::English, Text::Preferences) => "Preferences",
        (Language::Spanish, Text::Preferences) => "Preferencias",
        (Language::French, Text::About) => "À propos de Desdec",
        (Language::English, Text::About) => "About Desdec",
        (Language::Spanish, Text::About) => "Acerca de Desdec",
        (Language::French, Text::CollapseMenu) => "Réduire le menu",
        (Language::English, Text::CollapseMenu) => "Collapse menu",
        (Language::Spanish, Text::CollapseMenu) => "Contraer menú",
        (Language::French, Text::MenuHint) => {
            "Le menu est réduit au lancement. Utilisez ☰ pour le rouvrir."
        }
        (Language::English, Text::MenuHint) => {
            "The menu is collapsed at startup. Use ☰ to reopen it."
        }
        (Language::Spanish, Text::MenuHint) => {
            "El menú está contraído al iniciar. Use ☰ para abrirlo."
        }
        (Language::French, Text::ReadyToOpen) => "Prêt à ouvrir un binaire",
        (Language::English, Text::ReadyToOpen) => "Ready to open a binary",
        (Language::Spanish, Text::ReadyToOpen) => "Listo para abrir un binario",
        (Language::French, Text::NoPatches) => "Aucun correctif appliqué",
        (Language::English, Text::NoPatches) => "No patches applied",
        (Language::Spanish, Text::NoPatches) => "No hay parches aplicados",
        (Language::French, Text::Guided) => "Guidé",
        (Language::English, Text::Guided) => "Guided",
        (Language::Spanish, Text::Guided) => "Guiado",
        (Language::French, Text::Expert) => "Expert",
        (Language::English, Text::Expert) => "Expert",
        (Language::Spanish, Text::Expert) => "Experto",
        (Language::French, Text::Overview) => "Vue d’ensemble",
        (Language::English, Text::Overview) => "Overview",
        (Language::Spanish, Text::Overview) => "Resumen",
        (Language::French, Text::Segments) => "Segments",
        (Language::English, Text::Segments) => "Segments",
        (Language::Spanish, Text::Segments) => "Segmentos",
        (Language::French, Text::Functions) => "Fonctions",
        (Language::English, Text::Functions) => "Functions",
        (Language::Spanish, Text::Functions) => "Funciones",
        (Language::French, Text::Strings) => "Chaînes",
        (Language::English, Text::Strings) => "Strings",
        (Language::Spanish, Text::Strings) => "Cadenas",
        (Language::French, Text::Disassembly) => "Désassemblage",
        (Language::English, Text::Disassembly) => "Disassembly",
        (Language::Spanish, Text::Disassembly) => "Desensamblado",
        (Language::French, Text::Patches) => "Correctifs",
        (Language::English, Text::Patches) => "Patches",
        (Language::Spanish, Text::Patches) => "Parches",
        (Language::French, Text::StartAnalysis) => "Commencer une analyse",
        (Language::English, Text::StartAnalysis) => "Start an analysis",
        (Language::Spanish, Text::StartAnalysis) => "Iniciar un análisis",
        (Language::French, Text::DropFile) => {
            "Glissez un fichier ELF, PE ou Mach-O ici, ou ouvrez-le depuis la barre d’actions."
        }
        (Language::English, Text::DropFile) => {
            "Drop an ELF, PE or Mach-O file here, or open it from the action bar."
        }
        (Language::Spanish, Text::DropFile) => {
            "Arrastre aquí un archivo ELF, PE o Mach-O, o ábralo desde la barra de acciones."
        }
        (Language::French, Text::MenuAvailable) => {
            "☰ révèle le menu complet. La barre d’actions reste toujours disponible."
        }
        (Language::English, Text::MenuAvailable) => {
            "☰ reveals the full menu. The action bar always stays available."
        }
        (Language::Spanish, Text::MenuAvailable) => {
            "☰ muestra el menú completo. La barra de acciones siempre está disponible."
        }
        (Language::French, Text::LegalNotice) => {
            "Utilisez uniquement des binaires que vous pouvez légalement analyser."
        }
        (Language::English, Text::LegalNotice) => {
            "Only analyse binaries that you are legally allowed to inspect."
        }
        (Language::Spanish, Text::LegalNotice) => {
            "Analice solo binarios que tenga autorización legal para inspeccionar."
        }
        (Language::French, Text::ActiveFile) => "Fichier actif",
        (Language::English, Text::ActiveFile) => "Active file",
        (Language::Spanish, Text::ActiveFile) => "Archivo activo",
        (Language::French, Text::Path) => "Chemin",
        (Language::English, Text::Path) => "Path",
        (Language::Spanish, Text::Path) => "Ruta",
        (Language::French, Text::Format) => "Format",
        (Language::English, Text::Format) => "Format",
        (Language::Spanish, Text::Format) => "Formato",
        (Language::French, Text::Architecture) => "Architecture",
        (Language::English, Text::Architecture) => "Architecture",
        (Language::Spanish, Text::Architecture) => "Arquitectura",
        (Language::French, Text::Size) => "Taille",
        (Language::English, Text::Size) => "Size",
        (Language::Spanish, Text::Size) => "Tamaño",
        (Language::French, Text::NextStep) => "Étape suivante",
        (Language::English, Text::NextStep) => "Next step",
        (Language::Spanish, Text::NextStep) => "Siguiente paso",
        (Language::French, Text::NextStepDetail) => {
            "Sélectionnez Désassemblage dans la barre d’actions pour préparer l’analyse x86-64."
        }
        (Language::English, Text::NextStepDetail) => {
            "Select Disassembly in the action bar to prepare x86-64 analysis."
        }
        (Language::Spanish, Text::NextStepDetail) => {
            "Seleccione Desensamblado en la barra de acciones para preparar el análisis x86-64."
        }
        (Language::French, Text::OpenFirst) => {
            "Ouvrez d’abord un binaire afin d’utiliser cette vue."
        }
        (Language::English, Text::OpenFirst) => "Open a binary before using this view.",
        (Language::Spanish, Text::OpenFirst) => "Abra un binario antes de utilizar esta vista.",
        (Language::French, Text::ComingSoon) => "en préparation",
        (Language::English, Text::ComingSoon) => "coming soon",
        (Language::Spanish, Text::ComingSoon) => "en preparación",
        (Language::French, Text::SegmentsInfo) => {
            "La lecture structurée des sections et segments est le prochain module de formats."
        }
        (Language::English, Text::SegmentsInfo) => {
            "Structured section and segment parsing is the next format module."
        }
        (Language::Spanish, Text::SegmentsInfo) => {
            "El análisis estructurado de secciones y segmentos es el próximo módulo de formatos."
        }
        (Language::French, Text::FunctionsInfo) => {
            "La détection de fonctions suivra la lecture des sections exécutables."
        }
        (Language::English, Text::FunctionsInfo) => {
            "Function detection will follow executable-section parsing."
        }
        (Language::Spanish, Text::FunctionsInfo) => {
            "La detección de funciones seguirá al análisis de las secciones ejecutables."
        }
        (Language::French, Text::StringsInfo) => {
            "Les chaînes seront indexées avec leurs références croisées."
        }
        (Language::English, Text::StringsInfo) => {
            "Strings will be indexed with their cross-references."
        }
        (Language::Spanish, Text::StringsInfo) => {
            "Las cadenas se indexarán con sus referencias cruzadas."
        }
        (Language::French, Text::DisassemblyInfo) => {
            "Le désassemblage x86-64 sera ajouté après l’adressage virtuel et les sections."
        }
        (Language::English, Text::DisassemblyInfo) => {
            "x86-64 disassembly will follow virtual addressing and section support."
        }
        (Language::Spanish, Text::DisassemblyInfo) => {
            "El desensamblado x86-64 se añadirá tras el direccionamiento virtual y las secciones."
        }
        (Language::French, Text::PatchesInfo) => {
            "Les correctifs seront prévisualisés et exportés vers une copie du fichier."
        }
        (Language::English, Text::PatchesInfo) => {
            "Patches will be previewed and exported to a copy of the file."
        }
        (Language::Spanish, Text::PatchesInfo) => {
            "Los parches se previsualizarán y exportarán a una copia del archivo."
        }
        (Language::French, Text::GuidedHelp) => {
            "Mode guidé : cette vue expliquera les notions avant de montrer les détails bruts."
        }
        (Language::English, Text::GuidedHelp) => {
            "Guided mode explains concepts before showing raw details."
        }
        (Language::Spanish, Text::GuidedHelp) => {
            "El modo guiado explica los conceptos antes de mostrar los detalles sin procesar."
        }
        (Language::French, Text::PaletteTitle) => "Palette de commandes",
        (Language::English, Text::PaletteTitle) => "Command palette",
        (Language::Spanish, Text::PaletteTitle) => "Paleta de comandos",
        (Language::French, Text::SearchAction) => "Rechercher une action",
        (Language::English, Text::SearchAction) => "Search for an action",
        (Language::Spanish, Text::SearchAction) => "Buscar una acción",
        (Language::French, Text::SearchHint) => "Ex. ouvrir, fonctions, thème…",
        (Language::English, Text::SearchHint) => "E.g. open, functions, theme…",
        (Language::Spanish, Text::SearchHint) => "Ej. abrir, funciones, tema…",
        (Language::French, Text::Appearance) => "Apparence",
        (Language::English, Text::Appearance) => "Appearance",
        (Language::Spanish, Text::Appearance) => "Apariencia",
        (Language::French, Text::Theme) => "Thème",
        (Language::English, Text::Theme) => "Theme",
        (Language::Spanish, Text::Theme) => "Tema",
        (Language::French, Text::Language) => "Langue",
        (Language::English, Text::Language) => "Language",
        (Language::Spanish, Text::Language) => "Idioma",
        (Language::French, Text::SystemTheme) => "Suivre le système",
        (Language::English, Text::SystemTheme) => "Follow system",
        (Language::Spanish, Text::SystemTheme) => "Seguir el sistema",
        (Language::French, Text::DarkTheme) => "Sombre",
        (Language::English, Text::DarkTheme) => "Dark",
        (Language::Spanish, Text::DarkTheme) => "Oscuro",
        (Language::French, Text::LightTheme) => "Clair",
        (Language::English, Text::LightTheme) => "Light",
        (Language::Spanish, Text::LightTheme) => "Claro",
        (Language::French, Text::CatppuccinTheme) => "Catppuccin Mocha",
        (Language::English, Text::CatppuccinTheme) => "Catppuccin Mocha",
        (Language::Spanish, Text::CatppuccinTheme) => "Catppuccin Mocha",
        (Language::French, Text::FreeExtensions) => "Extensions gratuites",
        (Language::English, Text::FreeExtensions) => "Free extensions",
        (Language::Spanish, Text::FreeExtensions) => "Extensiones gratuitas",
        (Language::French, Text::FreeExtensionsInfo) => {
            "Les futurs thèmes communautaires gratuits s’installeront ici comme extensions."
        }
        (Language::English, Text::FreeExtensionsInfo) => {
            "Future free community themes will be installed here as extensions."
        }
        (Language::Spanish, Text::FreeExtensionsInfo) => {
            "Los futuros temas comunitarios gratuitos se instalarán aquí como extensiones."
        }
        (Language::French, Text::PreferencesInfo) => {
            "Les préférences sont enregistrées automatiquement pour les prochains lancements."
        }
        (Language::English, Text::PreferencesInfo) => {
            "Preferences are saved automatically for future launches."
        }
        (Language::Spanish, Text::PreferencesInfo) => {
            "Las preferencias se guardan automáticamente para futuros inicios."
        }
        (Language::French, Text::AboutTitle) => "À propos de Desdec",
        (Language::English, Text::AboutTitle) => "About Desdec",
        (Language::Spanish, Text::AboutTitle) => "Acerca de Desdec",
        (Language::French, Text::AboutDescription) => {
            "Explorateur de binaires open source, léger et pédagogique."
        }
        (Language::English, Text::AboutDescription) => {
            "A lightweight, open-source and educational binary explorer."
        }
        (Language::Spanish, Text::AboutDescription) => {
            "Un explorador de binarios ligero, educativo y de código abierto."
        }
        (Language::French, Text::CannotInspect) => "Impossible d’analyser",
        (Language::English, Text::CannotInspect) => "Could not inspect",
        (Language::Spanish, Text::CannotInspect) => "No se pudo analizar",
        (Language::French, Text::Shortcuts) => "Raccourcis",
        (Language::English, Text::Shortcuts) => "Shortcuts",
        (Language::Spanish, Text::Shortcuts) => "Atajos",
        (Language::French, Text::Behaviour) => "Comportement",
        (Language::English, Text::Behaviour) => "Behaviour",
        (Language::Spanish, Text::Behaviour) => "Comportamiento",
        (Language::French, Text::ShowToolbar) => "Afficher les actions de la barre d’outils",
        (Language::English, Text::ShowToolbar) => "Show toolbar actions",
        (Language::Spanish, Text::ShowToolbar) => "Mostrar acciones de la barra de herramientas",
        (Language::French, Text::ShowTooltips) => "Afficher les infobulles",
        (Language::English, Text::ShowTooltips) => "Show tooltips",
        (Language::Spanish, Text::ShowTooltips) => "Mostrar información sobre herramientas",
        (Language::French, Text::Persistence) => "Enregistrer les préférences",
        (Language::English, Text::Persistence) => "Save preferences",
        (Language::Spanish, Text::Persistence) => "Guardar preferencias",
        (Language::French, Text::PersistenceInfo) => {
            "Désactiver cette option supprime les réglages enregistrés à la fermeture."
        }
        (Language::English, Text::PersistenceInfo) => {
            "Disabling this option clears saved settings when the app closes."
        }
        (Language::Spanish, Text::PersistenceInfo) => {
            "Desactivar esta opción elimina los ajustes guardados al cerrar la aplicación."
        }
        (Language::French, Text::ToggleMenu) => "Afficher ou réduire le menu",
        (Language::English, Text::ToggleMenu) => "Toggle main menu",
        (Language::Spanish, Text::ToggleMenu) => "Alternar menú principal",
        (Language::French, Text::ToggleToolbar) => "Afficher ou masquer la barre d’outils",
        (Language::English, Text::ToggleToolbar) => "Toggle toolbar",
        (Language::Spanish, Text::ToggleToolbar) => "Alternar barra de herramientas",
        (Language::French, Text::ToggleTooltips) => "Afficher ou masquer les infobulles",
        (Language::English, Text::ToggleTooltips) => "Toggle tooltips",
        (Language::Spanish, Text::ToggleTooltips) => "Alternar información sobre herramientas",
        (Language::French, Text::ToggleExpertMode) => "Basculer le mode guidé / expert",
        (Language::English, Text::ToggleExpertMode) => "Toggle guided / expert mode",
        (Language::Spanish, Text::ToggleExpertMode) => "Alternar modo guiado / experto",
        (Language::French, Text::Modify) => "Modifier",
        (Language::English, Text::Modify) => "Change",
        (Language::Spanish, Text::Modify) => "Cambiar",
        (Language::French, Text::ResetDefaults) => "Rétablir les raccourcis par défaut",
        (Language::English, Text::ResetDefaults) => "Restore default shortcuts",
        (Language::Spanish, Text::ResetDefaults) => "Restaurar atajos predeterminados",
        (Language::French, Text::PressShortcut) => "Appuyez sur le nouveau raccourci…",
        (Language::English, Text::PressShortcut) => "Press the new shortcut…",
        (Language::Spanish, Text::PressShortcut) => "Pulse el nuevo atajo…",
        (Language::French, Text::Cancel) => "Annuler",
        (Language::English, Text::Cancel) => "Cancel",
        (Language::Spanish, Text::Cancel) => "Cancelar",
        (Language::French, Text::NoShortcut) => "Aucun raccourci",
        (Language::English, Text::NoShortcut) => "No shortcut",
        (Language::Spanish, Text::NoShortcut) => "Sin atajo",
        (Language::French, Text::French) => "Français",
        (Language::English, Text::French) => "French",
        (Language::Spanish, Text::French) => "Francés",
        (Language::French, Text::English) => "Anglais",
        (Language::English, Text::English) => "English",
        (Language::Spanish, Text::English) => "Inglés",
        (Language::French, Text::Spanish) => "Espagnol",
        (Language::English, Text::Spanish) => "Spanish",
        (Language::Spanish, Text::Spanish) => "Español",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_visible_string_is_translated() {
        for language in [Language::French, Language::English, Language::Spanish] {
            for item in ALL_TEXT {
                assert!(!text(language, *item).is_empty());
            }
        }
    }

    #[test]
    fn french_is_the_default_language() {
        assert_eq!(Language::default(), Language::French);
    }
}
