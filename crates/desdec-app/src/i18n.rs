use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    French,
    English,
    Spanish,
}

impl Language {
    pub const ALL: &[Self] = &[Self::French, Self::English, Self::Spanish];
}

/// Declares every visible string once, with its three translations.
///
/// The macro is the single source of truth: it derives the [`Text`] enum, the
/// exhaustive list used by the tests and the lookup table, so a new entry can
/// never be added on one side only.
macro_rules! translations {
    ($($item:ident => [$french:literal, $english:literal, $spanish:literal $(,)?]),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum Text {
            $($item,)+
        }

        #[cfg(test)]
        pub const ALL_TEXT: &[Text] = &[$(Text::$item,)+];

        /// Translations ordered like [`Language::ALL`].
        const fn translations(item: Text) -> [&'static str; Language::ALL.len()] {
            match item {
                $(Text::$item => [$french, $english, $spanish],)+
            }
        }
    };
}

translations! {
    Menu => ["Menu", "Menu", "Menú"],
    OpenBinary => ["Ouvrir un binaire", "Open binary", "Abrir un binario"],
    CloseBinary => ["Fermer le binaire", "Close binary", "Cerrar el binario"],
    Exploration => ["EXPLORATION", "EXPLORATION", "EXPLORACIÓN"],
    Tools => ["OUTILS", "TOOLS", "HERRAMIENTAS"],
    CommandPalette => ["Palette de commandes", "Command palette", "Paleta de comandos"],
    Preferences => ["Préférences", "Preferences", "Preferencias"],
    About => ["À propos de Desdec", "About Desdec", "Acerca de Desdec"],
    CollapseMenu => ["Réduire le menu", "Collapse menu", "Contraer menú"],
    MenuHint => [
        "Le menu est réduit au lancement. Utilisez ☰ pour le rouvrir.",
        "The menu is collapsed at startup. Use ☰ to reopen it.",
        "El menú está contraído al iniciar. Use ☰ para abrirlo.",
    ],
    ReadyToOpen => [
        "Prêt à ouvrir un binaire",
        "Ready to open a binary",
        "Listo para abrir un binario",
    ],
    StatusWorking => ["Analyse en cours…", "Analysing…", "Analizando…"],
    StatusFailed => ["Échec", "Failed", "Error"],
    Overview => ["Vue d’ensemble", "Overview", "Resumen"],
    Segments => ["Segments", "Segments", "Segmentos"],
    Functions => ["Fonctions", "Functions", "Funciones"],
    Strings => ["Chaînes", "Strings", "Cadenas"],
    Disassembly => ["Désassemblage", "Disassembly", "Desensamblado"],
    Decompile => ["Décompiler", "Decompile", "Decompilar"],
    AiAssistance => ["Assistance IA", "AI assistance", "Asistencia de IA"],
    AiAssistanceUnavailable => ["Bientôt disponible : choisissez alors un fournisseur IA dans les préférences.", "Coming soon: choose an AI provider in preferences then.", "Próximamente: elija entonces un proveedor de IA en preferencias."],
    Patches => ["Correctifs", "Patches", "Parches"],
    StartAnalysis => [
        "Commencer une analyse",
        "Start an analysis",
        "Iniciar un análisis",
    ],
    DropFile => [
        "Glissez un fichier ELF, PE ou Mach-O ici, ou ouvrez-le depuis la barre d’actions.",
        "Drop an ELF, PE or Mach-O file here, or open it from the action bar.",
        "Arrastre aquí un archivo ELF, PE o Mach-O, o ábralo desde la barra de acciones.",
    ],
    MenuAvailable => [
        "☰ révèle le menu complet. La barre d’actions reste toujours disponible.",
        "☰ reveals the full menu. The action bar always stays available.",
        "☰ muestra el menú completo. La barra de acciones siempre está disponible.",
    ],
    LegalNotice => [
        "Utilisez uniquement des binaires que vous pouvez légalement analyser.",
        "Only analyse binaries that you are legally allowed to inspect.",
        "Analice solo binarios que tenga autorización legal para inspeccionar.",
    ],
    ActiveFile => ["Fichier actif", "Active file", "Archivo activo"],
    Path => ["Chemin", "Path", "Ruta"],
    Format => ["Format", "Format", "Formato"],
    Architecture => ["Architecture", "Architecture", "Arquitectura"],
    Size => ["Taille", "Size", "Tamaño"],
    OpenFirst => [
        "Ouvrez d’abord un binaire afin d’utiliser cette vue.",
        "Open a binary before using this view.",
        "Abra un binario antes de utilizar esta vista.",
    ],
    ComingSoon => ["en préparation", "coming soon", "en preparación"],
    PatchesInfo => [
        "Les correctifs seront prévisualisés et exportés vers une copie du fichier.",
        "Patches will be previewed and exported to a copy of the file.",
        "Los parches se previsualizarán y exportarán a una copia del archivo.",
    ],
    PaletteTitle => ["Palette de commandes", "Command palette", "Paleta de comandos"],
    SearchAction => ["Rechercher une action", "Search for an action", "Buscar una acción"],
    SearchHint => [
        "Ex. ouvrir, fonctions, thème…",
        "E.g. open, functions, theme…",
        "Ej. abrir, funciones, tema…",
    ],
    Appearance => ["Apparence", "Appearance", "Apariencia"],
    Theme => ["Thème", "Theme", "Tema"],
    Language => ["Langue", "Language", "Idioma"],
    SystemTheme => ["Suivre le système", "Follow system", "Seguir el sistema"],
    DarkTheme => ["Sombre", "Dark", "Oscuro"],
    LightTheme => ["Clair", "Light", "Claro"],
    CatppuccinTheme => ["Catppuccin Mocha", "Catppuccin Mocha", "Catppuccin Mocha"],
    FreeExtensions => ["Extensions gratuites", "Free extensions", "Extensiones gratuitas"],
    FreeExtensionsInfo => [
        "Les futurs thèmes communautaires gratuits s’installeront ici comme extensions.",
        "Future free community themes will be installed here as extensions.",
        "Los futuros temas comunitarios gratuitos se instalarán aquí como extensiones.",
    ],
    PreferencesInfo => [
        "Les préférences sont enregistrées automatiquement pour les prochains lancements.",
        "Preferences are saved automatically for future launches.",
        "Las preferencias se guardan automáticamente para futuros inicios.",
    ],
    AboutTitle => ["À propos de Desdec", "About Desdec", "Acerca de Desdec"],
    AboutDescription => [
        "Explorateur de binaires open source, léger et pédagogique.",
        "A lightweight, open-source and educational binary explorer.",
        "Un explorador de binarios ligero, educativo y de código abierto.",
    ],
    CannotInspect => ["Impossible d’analyser", "Could not inspect", "No se pudo analizar"],
    Shortcuts => ["Raccourcis", "Shortcuts", "Atajos"],
    Behaviour => ["Comportement", "Behaviour", "Comportamiento"],
    ShowToolbar => [
        "Afficher les actions de la barre d’outils",
        "Show toolbar actions",
        "Mostrar acciones de la barra de herramientas",
    ],
    ShowTooltips => [
        "Afficher les infobulles",
        "Show tooltips",
        "Mostrar información sobre herramientas",
    ],
    Persistence => [
        "Enregistrer les préférences",
        "Save preferences",
        "Guardar preferencias",
    ],
    PersistenceInfo => [
        "Désactiver cette option supprime les réglages enregistrés à la fermeture.",
        "Disabling this option clears saved settings when the app closes.",
        "Desactivar esta opción elimina los ajustes guardados al cerrar la aplicación.",
    ],
    ToggleMenu => [
        "Afficher ou réduire le menu",
        "Toggle main menu",
        "Alternar menú principal",
    ],
    ToggleToolbar => [
        "Afficher ou masquer la barre d’outils",
        "Toggle toolbar",
        "Alternar barra de herramientas",
    ],
    ToggleTooltips => [
        "Afficher ou masquer les infobulles",
        "Toggle tooltips",
        "Alternar información sobre herramientas",
    ],
    Modify => ["Modifier", "Change", "Cambiar"],
    ResetDefaults => [
        "Rétablir les raccourcis par défaut",
        "Restore default shortcuts",
        "Restaurar atajos predeterminados",
    ],
    PressShortcut => [
        "Appuyez sur le nouveau raccourci…",
        "Press the new shortcut…",
        "Pulse el nuevo atajo…",
    ],
    Cancel => ["Annuler", "Cancel", "Cancelar"],
    NoShortcut => ["Aucun raccourci", "No shortcut", "Sin atajo"],
    EntryPoint => ["Point d’entrée", "Entry point", "Punto de entrada"],
    Entropy => ["Entropie", "Entropy", "Entropía"],
    Blocks => ["Blocs", "Blocks", "Bloques"],
    ControlFlow => ["Flot de contrôle", "Control flow", "Flujo de control"],
    PseudoCode => ["Pseudo-code local", "Local pseudocode", "Pseudocódigo local"],
    SelectFunction => [
        "Sélectionnez une fonction pour afficher son flot et son pseudo-code.",
        "Select a function to display its flow and pseudocode.",
        "Seleccione una función para mostrar su flujo y pseudocódigo.",
    ],
    NoFunctionBody => [
        "Aucune instruction décodée pour cette fonction.",
        "No decoded instructions for this function.",
        "No hay instrucciones decodificadas para esta función.",
    ],
    Obfuscation => ["Obfuscation", "Obfuscation", "Ofuscación"],
    CodeMayBeObfuscated => [
        "Code possiblement obfusqué ou compacté",
        "Code may be obfuscated or packed",
        "El código puede estar ofuscado o empaquetado",
    ],
    StringsMayBeObfuscated => [
        "Chaînes possiblement chiffrées ou obfusquées",
        "Strings may be encrypted or obfuscated",
        "Las cadenas pueden estar cifradas u ofuscadas",
    ],
    SectionCount => ["Sections", "Sections", "Secciones"],
    StringCount => ["Chaînes lisibles", "Readable strings", "Cadenas legibles"],
    StringReferences => [
        "Références dans le code",
        "Code references",
        "Referencias en el código",
    ],
    NoStringReferences => [
        "Aucune référence directe trouvée dans le code décodé.",
        "No direct reference found in decoded code.",
        "No se encontró ninguna referencia directa en el código decodificado.",
    ],
    StringAddressUnavailable => [
        "Cette chaîne n’est pas située dans une section mappée.",
        "This string is not located in a mapped section.",
        "Esta cadena no está situada en una sección mapeada.",
    ],
    GoToDisassembly => [
        "Voir dans le désassemblage",
        "Show in disassembly",
        "Ver en el desensamblado",
    ],
    AnalysedBytes => ["Octets analysés", "Analysed bytes", "Bytes analizados"],
    Address => ["Adresse", "Address", "Dirección"],
    Offset => ["Décalage", "Offset", "Desplazamiento"],
    Rights => ["Droits", "Rights", "Permisos"],
    Name => ["Nom", "Name", "Nombre"],
    NoSections => [
        "Aucune table de sections lisible dans ce fichier.",
        "This file has no readable section table.",
        "Este archivo no tiene una tabla de secciones legible.",
    ],
    NoStrings => [
        "Aucune chaîne lisible trouvée.",
        "No readable string found.",
        "No se encontró ninguna cadena legible.",
    ],
    FilterStrings => ["Filtrer les chaînes", "Filter strings", "Filtrar cadenas"],
    FilterHint => ["Ex. http, .dll, erreur…", "E.g. http, .dll, error…", "Ej. http, .dll, error…"],
    ShownOfTotal => ["affichées sur", "shown of", "mostradas de"],
    TruncatedAnalysis => [
        "Fichier volumineux : seul le début a été analysé. Les sections et chaînes situées au-delà ne sont pas listées.",
        "Large file: only its beginning was analysed. Sections and strings past that point are not listed.",
        "Archivo grande: solo se analizó su comienzo. Las secciones y cadenas posteriores no se listan.",
    ],
    StringLimitReached => [
        "Limite d’extraction atteinte : toutes les chaînes du fichier ne sont pas listées.",
        "Extraction limit reached: not every string in the file is listed.",
        "Se alcanzó el límite de extracción: no se listan todas las cadenas del archivo.",
    ],
    DenseCodeWarning => [
        "Une section exécutable est très dense. Le code est peut-être obfusqué, compacté ou chiffré, et le désassemblage direct donnera peu de résultats.",
        "An executable section is very dense. The code may be obfuscated, packed or encrypted, and disassembling it directly will reveal little.",
        "Una sección ejecutable es muy densa. El código puede estar ofuscado, empaquetado o cifrado, y desensamblarlo directamente revelará poco.",
    ],
    DenseCodeHint => [
        "L’entropie mesure la densité d’information : au-delà de 7,2 bits par octet, le contenu ressemble à des données compressées plutôt qu’à du code. Ce n’est qu’un indice.",
        "Entropy measures information density: above 7.2 bits per byte, content looks compressed rather than like code. It is only an indication.",
        "La entropía mide la densidad de información: por encima de 7,2 bits por byte, el contenido parece comprimido en lugar de código. Es solo un indicio.",
    ],
    EntryPointIn => ["dans", "in", "en"],
    NotMapped => ["non mappée", "not mapped", "no mapeada"],
    NoFunctionSymbols => ["Aucun symbole de fonction lisible pour ce format ou ce fichier.", "No readable function symbols for this format or file.", "No hay símbolos de función legibles para este formato o archivo."],
    MappedSize => ["Taille en mémoire", "Mapped size", "Tamaño en memoria"],
    WordSize => ["Taille de mot", "Word size", "Tamaño de palabra"],
    ByteOrder => ["Ordre des octets", "Byte order", "Orden de bytes"],
    Interpreter => ["Chargeur", "Loader", "Cargador"],
    Subsystem => ["Sous-système", "Subsystem", "Subsistema"],
    BuildTimestamp => ["Horodatage de compilation", "Build timestamp", "Marca de tiempo"],
    Digest => ["Empreinte SHA-256", "SHA-256 digest", "Huella SHA-256"],
    DigestWithheld => [
        "indisponible : le fichier n’a été lu qu’en partie",
        "unavailable: the file was only read in part",
        "no disponible: el archivo se leyó solo en parte",
    ],
    Hardening => ["Protections", "Hardening", "Protecciones"],
    LinkedLibraries => ["Bibliothèques liées", "Linked libraries", "Bibliotecas enlazadas"],
    NoLinkedLibraries => [
        "Aucune : le binaire est autonome ou lie tout statiquement.",
        "None: the binary is self-contained or statically linked.",
        "Ninguna: el binario es autónomo o enlaza todo estáticamente.",
    ],
    LoadMapping => ["Mappage mémoire", "Load mapping", "Mapeo de memoria"],
    LoadMappingHelp => [
        "Régions telles que le chargeur les place en mémoire, plus grossières que les sections.",
        "Regions as the loader places them in memory, coarser than sections.",
        "Regiones tal como el cargador las coloca en memoria, más amplias que las secciones.",
    ],
    NoLoadMapping => [
        "Ce format décrit son mappage par ses sections.",
        "This format describes its mapping through its sections.",
        "Este formato describe su mapeo mediante sus secciones.",
    ],
    Type => ["Type", "Type", "Tipo"],
    PositionIndependent => [
        "Indépendant de la position (PIE)",
        "Position independent (PIE)",
        "Independiente de la posición (PIE)",
    ],
    NonExecutableStack => ["Pile non exécutable (NX)", "Non-executable stack (NX)", "Pila no ejecutable (NX)"],
    RelroLabel => ["Relocations en lecture seule (RELRO)", "Read-only relocations (RELRO)", "Reubicaciones de solo lectura (RELRO)"],
    StackCanary => ["Détection d’écrasement de pile", "Stack-smashing detection", "Detección de desbordamiento de pila"],
    AddressRandomisation => ["Randomisation d’adresses (ASLR)", "Address randomisation (ASLR)", "Aleatorización de direcciones (ASLR)"],
    DataExecutionPrevention => ["Prévention d’exécution des données (DEP)", "Data execution prevention (DEP)", "Prevención de ejecución de datos (DEP)"],
    ControlFlowGuard => ["Protection du flot de contrôle (CFG)", "Control flow guard (CFG)", "Protección del flujo de control (CFG)"],
    SignedImage => ["Signature intégrée", "Embedded signature", "Firma incrustada"],
    Present => ["oui", "yes", "sí"],
    Absent => ["non", "no", "no"],
    NotApplicable => ["non applicable", "not applicable", "no aplicable"],
    StackCanaryHint => [
        "Déduit des symboles présents dans le binaire : c’est un indice, pas une preuve.",
        "Inferred from the symbols present in the binary: an indication, not proof.",
        "Deducido de los símbolos presentes en el binario: un indicio, no una prueba.",
    ],
    SignatureHint => [
        "La présence d’une signature est constatée ; sa validité n’est pas vérifiée.",
        "The presence of a signature is noted; its validity is not checked.",
        "Se constata la presencia de una firma; su validez no se verifica.",
    ],
    TimestampHint => [
        "Souvent mis à zéro pour des compilations reproductibles, et trivial à falsifier.",
        "Often zeroed for reproducible builds, and trivially forged.",
        "A menudo puesto a cero para compilaciones reproducibles, y trivial de falsificar.",
    ],
    French => ["Français", "French", "Francés"],
    English => ["Anglais", "English", "Inglés"],
    Spanish => ["Espagnol", "Spanish", "Español"],
}

#[must_use]
pub const fn text(language: Language, item: Text) -> &'static str {
    translations(item)[language as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_visible_string_is_translated() {
        for language in Language::ALL {
            for item in ALL_TEXT {
                assert!(!text(*language, *item).is_empty());
            }
        }
    }

    #[test]
    fn french_is_the_default_language() {
        assert_eq!(Language::default(), Language::French);
    }

    #[test]
    fn each_language_selects_its_own_column() {
        assert_eq!(text(Language::French, Text::Overview), "Vue d’ensemble");
        assert_eq!(text(Language::English, Text::Overview), "Overview");
        assert_eq!(text(Language::Spanish, Text::Overview), "Resumen");
    }
}
