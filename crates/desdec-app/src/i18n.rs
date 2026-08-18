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

    /// The language named in itself, for a reader that is not this interface.
    ///
    /// The assistant is told which language to answer in, and it is told in
    /// that language: "French" and "français" are not equally clear
    /// instructions to a model being asked to write French.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::French => "français",
            Self::English => "English",
            Self::Spanish => "español",
        }
    }
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
    RecentBinaries => ["Fichiers récents", "Recent binaries", "Archivos recientes"],
    ClearRecentBinaries => ["Effacer l’historique", "Clear history", "Borrar historial"],
    CloseBinary => ["Fermer le binaire", "Close binary", "Cerrar el binario"],
    Exploration => ["EXPLORATION", "EXPLORATION", "EXPLORACIÓN"],
    Tools => ["OUTILS", "TOOLS", "HERRAMIENTAS"],
    CommandPalette => ["Palette de commandes", "Command palette", "Paleta de comandos"],
    Preferences => ["Préférences", "Preferences", "Preferencias"],
    About => ["À propos de Desdec", "About Desdec", "Acerca de Desdec"],
    CollapseMenu => ["Réduire le menu", "Collapse menu", "Contraer menú"],
    NarrowMenu => [
        "Réduire le menu aux icônes",
        "Narrow the menu to icons",
        "Reducir el menú a iconos",
    ],
    WidenMenu => [
        "Élargir le menu et afficher les libellés",
        "Widen the menu and show the labels",
        "Ampliar el menú y mostrar las etiquetas",
    ],
    DragToResizeMenu => [
        "Le bord du menu se tire à la largeur voulue ; elle est conservée.",
        "Drag the menu's edge to the width you want; it is kept.",
        "Arrastre el borde del menú al ancho que quiera; se conserva.",
    ],
    ReadyToOpen => [
        "Prêt à ouvrir un binaire",
        "Ready to open a binary",
        "Listo para abrir un binario",
    ],
    StatusWorking => ["Analyse en cours…", "Analysing…", "Analizando…"],
    CancelAnalysis => ["Arrêter l’analyse", "Stop analysis", "Detener el análisis"],
    CancelChoosing => [
        "Abandonner l’ouverture",
        "Give up opening",
        "Abandonar la apertura",
    ],
    CancelOpening => [
        "Abandonner l’ouverture en cours",
        "Give up the opening under way",
        "Abandonar la apertura en curso",
    ],
    StatusChoosing => [
        "Sélection d’un fichier…",
        "Choosing a file…",
        "Seleccionando un archivo…",
    ],
    StatusFailed => ["Échec", "Failed", "Error"],
    Overview => ["Vue d’ensemble", "Overview", "Resumen"],
    Segments => ["Segments", "Segments", "Segmentos"],
    Functions => ["Fonctions", "Functions", "Funciones"],
    Strings => ["Chaînes", "Strings", "Cadenas"],
    Disassembly => ["Désassemblage", "Disassembly", "Desensamblado"],
    Decompile => ["Décompiler", "Decompile", "Decompilar"],
    AiAssistance => ["Assistance IA", "AI assistance", "Asistencia de IA"],
    AiAssistanceIntro => [
        "Un modèle relit le désassemblage déjà décodé. Sa réponse est une lecture proposée, jamais un fait établi : vérifiez-la contre le listing.",
        "A model reads back the disassembly Desdec decoded. Its answer is a proposed reading, never an established fact: check it against the listing.",
        "Un modelo relee el desensamblado ya decodificado. Su respuesta es una lectura propuesta, nunca un hecho: verifíquela con el listado.",
    ],
    ProposedReading => [
        "Lecture proposée — à vérifier dans le listing",
        "Proposed reading — check it against the listing",
        "Lectura propuesta — verifíquela en el listado",
    ],
    AnsweredBy => ["Répondu par", "Answered by", "Respondido por"],
    CopiedToClipboard => [
        "Copié dans le presse-papiers",
        "Copied to the clipboard",
        "Copiado al portapapeles",
    ],
    CopyFullPath => [
        "Cliquer pour copier le chemin complet",
        "Click to copy the full path",
        "Clic para copiar la ruta completa",
    ],
    AssistantTruncated => [
        "Réponse interrompue à la limite de jetons : la fin manque. Posez une question plus étroite.",
        "The answer stopped at the token limit: the end is missing. Ask a narrower question.",
        "La respuesta se detuvo en el límite de tokens: falta el final. Haga una pregunta más concreta.",
    ],
    AskAboutBinary => ["Pistes d’analyse", "Where to start", "Por dónde empezar"],
    AskAboutFunction => ["Expliquer la fonction", "Explain the function", "Explicar la función"],
    AskAboutInstruction => [
        "Expliquer l’instruction",
        "Explain the instruction",
        "Explicar la instrucción",
    ],
    Asking => ["Interrogation du modèle…", "Asking the model…", "Consultando al modelo…"],
    NothingAskedYet => [
        "Choisissez une question ci-dessus.",
        "Pick a question above.",
        "Elija una pregunta arriba.",
    ],
    ShowWhatIsSent => ["Voir ce qui est envoyé", "Show what is sent", "Ver lo que se envía"],
    SelectFunctionFirst => [
        "Sélectionnez d’abord une fonction dans la vue Fonctions.",
        "Select a function in the Functions view first.",
        "Seleccione antes una función en la vista Funciones.",
    ],
    SelectInstructionFirst => [
        "Sélectionnez d’abord une instruction dans le désassemblage.",
        "Select an instruction in the disassembly first.",
        "Seleccione antes una instrucción en el desensamblado.",
    ],
    AssistantLeavesMachine => [
        "Ce fournisseur envoie les faits extraits — instructions, symboles, chaînes — à un service distant. Le fichier lui-même ne part jamais.",
        "This provider sends the extracted facts — instructions, symbols, strings — to a remote service. The file itself never leaves.",
        "Este proveedor envía los datos extraídos — instrucciones, símbolos, cadenas — a un servicio remoto. El archivo nunca sale.",
    ],
    AssistantStaysLocal => [
        "Ce modèle tourne sur cette machine : rien ne part sur le réseau.",
        "This model runs on this machine: nothing goes over the network.",
        "Este modelo se ejecuta en esta máquina: nada sale a la red.",
    ],
    AssistantNotConfigured => [
        "Aucun assistant configuré. Choisissez un fournisseur dans Préférences → Assistance IA.",
        "No assistant is configured. Choose a provider in Preferences → AI assistance.",
        "No hay asistente configurado. Elija un proveedor en Preferencias → Asistencia de IA.",
    ],
    AssistantNoKey => [
        "Aucune clé API : définissez ANTHROPIC_API_KEY, ou indiquez un fichier de clé dans les préférences.",
        "No API key: set ANTHROPIC_API_KEY, or name a key file in the preferences.",
        "Sin clave API: defina ANTHROPIC_API_KEY o indique un archivo de clave en las preferencias.",
    ],
    AssistantUnreachable => [
        "Fournisseur injoignable :",
        "The provider could not be reached:",
        "No se pudo contactar al proveedor:",
    ],
    AssistantRejected => ["Requête refusée :", "The request was rejected:", "Solicitud rechazada:"],
    AssistantTimedOut => [
        "Le fournisseur n’a pas répondu dans le délai imparti. Essayez une question plus petite.",
        "The provider did not answer in time. Try a smaller question.",
        "El proveedor no respondió a tiempo. Pruebe una pregunta más pequeña.",
    ],
    AssistantDeclined => [
        "Le modèle a décliné cette question.",
        "The model declined this question.",
        "El modelo declinó esta pregunta.",
    ],
    AssistantUnreadable => [
        "Réponse illisible :",
        "The answer could not be read:",
        "No se pudo leer la respuesta:",
    ],
    ClearFilter => ["Effacer le filtre", "Clear filter", "Borrar el filtro"],
    AllStrings => ["Toutes les chaînes", "All strings", "Todas las cadenas"],
    CriteriaChosen => ["critères", "criteria", "criterios"],
    FilterCriteriaHelp => [
        "Restreindre la liste ; plusieurs critères se cumulent.",
        "Narrow the list; several criteria apply together.",
        "Restringir la lista; varios criterios se aplican juntos.",
    ],
    FilterUnmappedHelp => [
        "Ne garde que les chaînes situées dans une section chargée en mémoire.",
        "Keeps only strings inside a section the loader maps.",
        "Conserva solo las cadenas dentro de una sección que el cargador mapea.",
    ],
    FilterUnreferencedHelp => [
        "Ne garde que les chaînes qu’une instruction décodée désigne directement.",
        "Keeps only strings a decoded instruction points at directly.",
        "Conserva solo las cadenas a las que apunta directamente una instrucción decodificada.",
    ],
    AssistantProvider => ["Fournisseur", "Provider", "Proveedor"],
    NoAssistant => ["Aucun (désactivé)", "None (off)", "Ninguno (desactivado)"],
    LocalModel => [
        "Modèle local (Ollama)",
        "Local model (Ollama)",
        "Modelo local (Ollama)",
    ],
    ClaudeApi => [
        "API Claude (Anthropic) — réseau",
        "Claude API (Anthropic) — network",
        "API Claude (Anthropic) — red",
    ],
    CheckProvider => ["Vérifier", "Check", "Comprobar"],
    AssistantModel => ["Modèle", "Model", "Modelo"],
    AssistantModelHint => [
        "Laisser vide pour le modèle par défaut",
        "Leave empty for the default model",
        "Dejar vacío para el modelo predeterminado",
    ],
    OllamaUrl => ["Adresse du serveur", "Server address", "Dirección del servidor"],
    ApiKeyFile => ["Fichier de clé API", "API key file", "Archivo de clave API"],
    ApiKeyFileHint => [
        "ANTHROPIC_API_KEY est lue en premier ; la clé n’est jamais écrite dans les préférences.",
        "ANTHROPIC_API_KEY is read first; the key is never written to the preferences.",
        "ANTHROPIC_API_KEY se lee primero; la clave nunca se escribe en las preferencias.",
    ],
    Patches => ["Correctifs", "Patches", "Parches"],
    Yara => ["YARA", "YARA", "YARA"],
    EnableYara => ["Activer le module YARA", "Enable the YARA module", "Activar el módulo YARA"],
    ToggleYaraModule => ["Activer ou désactiver le module", "Toggle module", "Activar o desactivar el módulo"],
    YaraInfo => [
        "YARA analyse statiquement le fichier avec vos règles locales ; le binaire n’est jamais exécuté.",
        "YARA scans the file statically with your local rules; the binary is never executed.",
        "YARA analiza estáticamente el archivo con sus reglas locales; el binario nunca se ejecuta.",
    ],
    YaraProgramPath => ["Commande YARA", "YARA command", "Comando YARA"],
    YaraRulesPath => ["Fichier de règles", "Rules file", "Archivo de reglas"],
    YaraRulesHint => ["Chemin vers vos règles .yar", "Path to your .yar rules", "Ruta a sus reglas .yar"],
    RunYara => ["Analyser avec YARA", "Scan with YARA", "Analizar con YARA"],
    YaraDisabled => ["Le module YARA est désactivé. Activez-le dans les préférences ou la palette.", "The YARA module is disabled. Enable it in Preferences or the command palette.", "El módulo YARA está desactivado. Actívelo en Preferencias o en la paleta de comandos."],
    YaraNotConfigured => ["Configurez une commande YARA et un fichier de règles dans les préférences.", "Configure a YARA command and rules file in Preferences.", "Configure un comando YARA y un archivo de reglas en Preferencias."],
    YaraScanning => ["Analyse YARA en cours…", "YARA scan in progress…", "Análisis YARA en curso…"],
    YaraNoMatches => ["Aucune règle ne correspond à ce fichier.", "No rule matches this file.", "Ninguna regla coincide con este archivo."],
    YaraMatches => ["Règles correspondantes", "Matching rules", "Reglas coincidentes"],
    YaraFailed => ["L’analyse YARA a échoué :", "YARA scan failed:", "El análisis YARA falló:"],
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
        "Le bouton en haut à gauche ouvre le menu complet. La barre d’actions reste toujours disponible.",
        "The button at the top left opens the full menu. The action bar always stays available.",
        "El botón de arriba a la izquierda abre el menú completo. La barra de acciones siempre está disponible.",
    ],
    Repository => ["Dépôt du projet", "Project repository", "Repositorio del proyecto"],
    LicenceLine => [
        "Sous licence Apache-2.0 ou MIT, au choix.",
        "Licensed under Apache-2.0 or MIT, at your option.",
        "Bajo licencia Apache-2.0 o MIT, a su elección.",
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
    ComingSoon => ["en préparation", "coming soon", "en preparación"],
    Decompiler => ["Décompilateur", "Decompiler", "Decompilador"],
    BuiltinDecompiler => [
        "Intégré (pseudo-code déterministe)",
        "Built-in (deterministic pseudo-code)",
        "Integrado (pseudocódigo determinista)",
    ],
    DecompilerInfo => [
        "Les moteurs externes sont des programmes libres, lancés uniquement si vous les choisissez. Ils analysent le fichier sans jamais l’exécuter.",
        "External engines are free programs, run only if you choose one. They analyse the file statically and never execute it.",
        "Los motores externos son programas libres, ejecutados solo si los elige. Analizan el archivo de forma estática y nunca lo ejecutan.",
    ],
    EngineAvailable => ["Détecté", "Detected", "Detectado"],
    EngineMissing => ["Introuvable", "Not found", "No encontrado"],
    EngineIncomplete => ["Incomplet", "Incomplete", "Incompleto"],
    EngineInstallWith => ["Installation :", "Install with:", "Instalación:"],
    EngineMissingPlugin => [
        "Le greffon de décompilation manque ; la commande pdg est absente.",
        "The decompilation plugin is missing; the pdg command is absent.",
        "Falta el complemento de decompilación; el comando pdg no existe.",
    ],
    EnginePath => ["Chemin personnalisé", "Custom path", "Ruta personalizada"],
    EnginePathHint => [
        "Laisser vide pour chercher dans le PATH",
        "Leave empty to search the PATH",
        "Dejar vacío para buscar en el PATH",
    ],
    EngineUnavailable => [
        "Ce décompilateur n’est pas installé. Installation :",
        "This decompiler is not installed. Install with:",
        "Este decompilador no está instalado. Instalación:",
    ],
    Function => ["Fonction", "Function", "Función"],
    StrippedEntryPoint => [
        "Aucun symbole de fonction : le point d’entrée est décompilé.",
        "No function symbols: the entry point is decompiled instead.",
        "Sin símbolos de función: se decompila el punto de entrada.",
    ],
    Decompiling => ["Décompilation en cours…", "Decompiling…", "Decompilando…"],
    FromCache => ["· en cache", "· cached", "· en caché"],
    CacheDecompilations => [
        "Conserver les fonctions décompilées sur le disque",
        "Keep decompiled functions on disk",
        "Conservar las funciones decompiladas en el disco",
    ],
    CacheInfo => [
        "Un moteur externe met plusieurs secondes par fonction, essentiellement à démarrer. Les réponses sont conservées et réutilisées tant que le binaire est inchangé : son empreinte SHA-256 fait partie de la clé.",
        "An external engine takes seconds per function, mostly to start up. Answers are kept and reused as long as the binary is unchanged: its SHA-256 digest is part of the key.",
        "Un motor externo tarda segundos por función, sobre todo en arrancar. Las respuestas se conservan y reutilizan mientras el binario no cambie: su huella SHA-256 forma parte de la clave.",
    ],
    ClearCache => ["Vider le cache", "Clear the cache", "Vaciar la caché"],
    RzGhidraEngine => ["rizin + rz-ghidra", "rizin + rz-ghidra", "rizin + rz-ghidra"],
    RetDecEngine => ["RetDec", "RetDec", "RetDec"],
    CacheCleared => ["Cache vidé :", "Cache cleared:", "Caché vaciada:"],
    CacheEntries => ["entrées", "entries", "entradas"],
    DecompilerFailed => ["Le décompilateur a échoué :", "The decompiler failed:", "El decompilador falló:"],
    DecompiledBy => ["Produit par", "Produced by", "Generado por"],
    EditInstruction => ["Modifier les octets", "Edit bytes", "Editar los bytes"],
    EditingInstruction => ["Instruction modifiée", "Instruction being edited", "Instrucción en edición"],
    PatchBytes => ["Octets (hexadécimal)", "Bytes (hexadecimal)", "Bytes (hexadecimal)"],
    PatchBecomes => ["Devient", "Becomes", "Se convierte en"],
    PatchNotAnInstruction => [
        "Ces octets ne forment pas une instruction complète.",
        "These bytes are not one whole instruction.",
        "Estos bytes no forman una instrucción completa.",
    ],
    PatchLengthRule => [
        "Un correctif remplace des octets sur place : la longueur doit rester identique, sinon tout ce qui suit serait décalé.",
        "A patch replaces bytes in place: the length must stay the same, or everything after it would shift.",
        "Un parche reemplaza bytes en su lugar: la longitud debe mantenerse, o todo lo posterior se desplazaría.",
    ],
    PatchLengthMismatch => ["Longueur attendue :", "Expected length:", "Longitud esperada:"],
    ApplyPatch => ["Enregistrer le correctif", "Record patch", "Guardar el parche"],
    RevertPatch => ["Rétablir", "Revert", "Restablecer"],
    CancelEdit => ["Annuler", "Cancel", "Cancelar"],
    NoPatches => [
        "Aucun correctif. Sélectionnez une instruction dans le désassemblage puis « Modifier les octets ».",
        "No patches. Select an instruction in the disassembly, then “Edit bytes”.",
        "Sin parches. Seleccione una instrucción en el desensamblado y luego «Editar los bytes».",
    ],
    PendingPatches => ["Correctifs en attente", "Pending patches", "Parches pendientes"],
    ExportPatched => [
        "Exporter le binaire corrigé…",
        "Export patched binary…",
        "Exportar el binario parcheado…",
    ],
    ExportInfo => [
        "L’export écrit une copie. Le fichier analysé n’est jamais modifié.",
        "Exporting writes a copy. The analysed file is never modified.",
        "La exportación escribe una copia. El archivo analizado nunca se modifica.",
    ],
    ExportSucceeded => ["Copie corrigée écrite :", "Patched copy written:", "Copia parcheada escrita:"],
    CopyPath => ["Copier le chemin", "Copy path", "Copiar ruta"],
    ExportFailed => ["L’export a échoué :", "Export failed:", "La exportación falló:"],
    DiscardPatches => ["Tout effacer", "Discard all", "Descartar todo"],
    PatchedColumn => ["Corrigé", "Patched", "Parcheado"],
    OriginalBytes => ["Octets d’origine", "Original bytes", "Bytes originales"],
    NotPatchable => [
        "Cette instruction n’occupe aucun octet du fichier et ne peut pas être corrigée.",
        "This instruction occupies no bytes in the file and cannot be patched.",
        "Esta instrucción no ocupa bytes del archivo y no puede parchearse.",
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
    RemoveShortcut => ["Retirer", "Remove", "Quitar"],
    RemoveShortcutHint => [
        "Laisse la commande sans raccourci ; elle reste dans la palette.",
        "Leaves the command with no shortcut; it stays in the palette.",
        "Deja el comando sin atajo; sigue en la paleta.",
    ],
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
    PseudoCodeHelp => [
        "Traduction déterministe du flot observé, sans code source inventé.",
        "Deterministic translation of the observed flow, with no invented source.",
        "Traducción determinista del flujo observado, sin código fuente inventado.",
    ],
    AssemblyPreview => ["Désassemblage correspondant", "Corresponding disassembly", "Ensamblado correspondiente"],
    JumpToAssembly => ["Sauter vers l’assembleur", "Jump to assembly", "Saltar al ensamblador"],
    WholeFunctionAssembly => [
        "Ces moteurs décompilent une fonction entière et n’associent aucune ligne à une adresse : c’est le désassemblage de toute la fonction choisie qui s’ouvre.",
        "These engines decompile a whole function and map no line to an address: what opens is the assembly of the whole selected function.",
        "Estos motores descompilan una función entera y no asocian ninguna línea a una dirección: se abre el ensamblado de toda la función elegida.",
    ],
    TruncatedDisassembly => [
        "Une partie du code se trouve au-delà des octets analysés : elle n’a pas été désassemblée.",
        "Some of the code lies beyond the analysed bytes and was not disassembled.",
        "Parte del código está más allá de los bytes analizados y no se desensambló.",
    ],
    MoreInstructions => [
        "instructions supplémentaires, non affichées ici.",
        "further instructions, not shown here.",
        "instrucciones más, no mostradas aquí.",
    ],
    NoDisassembly => [
        "Le désassemblage est disponible pour les binaires x86, x86-64 et ARM64 ; ce fichier utilise une autre architecture ou ne contient aucune section exécutable lisible.",
        "Disassembly is available for x86, x86-64 and ARM64 binaries; this file uses another architecture or has no readable executable section.",
        "El desensamblado está disponible para binarios x86, x86-64 y ARM64; este archivo usa otra arquitectura o no tiene ninguna sección ejecutable legible.",
    ],
    LocalDecoders => [
        "Décodeurs locaux : iced-x86 (x86/x86-64) et Capstone (ARM64, dont Apple Silicon).",
        "Local decoders: iced-x86 (x86/x86-64) and Capstone (ARM64, including Apple Silicon).",
        "Decodificadores locales: iced-x86 (x86/x86-64) y Capstone (ARM64, incluido Apple Silicon).",
    ],
    WhatIsThisFor => ["À quoi sert cette bibliothèque ?", "What is this library for?", "¿Para qué sirve esta biblioteca?"],
    InspectOperand => ["Ce que désigne l’opérande", "What the operand designates", "Lo que designa el operando"],
    OperandInspection => ["Opérande et registres", "Operand and registers", "Operando y registros"],
    Designates => ["Désigne", "Designates", "Designa"],
    NoTargetResolved => [
        "Cette instruction ne calcule aucune adresse résoluble statiquement.",
        "This instruction computes no address that can be resolved statically.",
        "Esta instrucción no calcula ninguna dirección resoluble estáticamente.",
    ],
    TargetSection => ["Section", "Section", "Sección"],
    TargetSymbol => ["Symbole", "Symbol", "Símbolo"],
    TargetText => ["Texte", "Text", "Texto"],
    TargetBytes => ["Octets", "Bytes", "Bytes"],
    LastWriteTo => ["Dernière écriture dans", "Last write to", "Última escritura en"],
    WrittenValue => ["Valeur écrite", "Value written", "Valor escrito"],
    ValueUnknown => [
        "inconnue : elle dépend d’autres registres",
        "unknown: it depends on other registers",
        "desconocido: depende de otros registros",
    ],
    NoWriteFound => [
        "Aucune écriture trouvée dans les instructions précédentes.",
        "No write found in the preceding instructions.",
        "No se encontró ninguna escritura en las instrucciones anteriores.",
    ],
    StaticOnlyWarning => [
        "Desdec n’exécute jamais le binaire : ces réponses sont lues dans les octets. Le suivi d’un registre remonte le flot local et cesse d’être fiable si un saut arrive au milieu.",
        "Desdec never runs the binary: these answers are read from the bytes. Following a register walks the local flow, and stops being reliable if a branch lands in the middle of it.",
        "Desdec nunca ejecuta el binario: estas respuestas se leen de los bytes. Seguir un registro recorre el flujo local y deja de ser fiable si un salto cae en medio.",
    ],
    LibraryExplanation => ["Bibliothèque liée", "Linked library", "Biblioteca enlazada"],
    NoteFromCatalogue => ["CATALOGUE INTÉGRÉ", "BUILT-IN CATALOGUE", "CATÁLOGO INTEGRADO"],
    NoteFromYourFile => ["VOTRE FICHIER", "YOUR OWN FILE", "SU PROPIO ARCHIVO"],
    LibraryUndescribed => [
        "Cette bibliothèque n’est décrite ni par le catalogue intégré ni par votre fichier. Rien n’est deviné à partir du nom : une phrase confiante mais fausse enverrait chercher au mauvais endroit.",
        "Neither the built-in catalogue nor your own file describes this library. Nothing is guessed from the name: a confident but wrong sentence would send you looking in the wrong place.",
        "Ni el catálogo integrado ni su archivo describen esta biblioteca. Nada se adivina a partir del nombre.",
    ],
    DescribeItYourself => [
        "Vous pouvez la décrire vous-même : voir Préférences → Comportement.",
        "You can describe it yourself: see Preferences → Behaviour.",
        "Puede describirla usted mismo: véase Preferencias → Comportamiento.",
    ],
    ExplainLibraries => [
        "Expliquer les bibliothèques liées",
        "Explain the linked libraries",
        "Explicar las bibliotecas enlazadas",
    ],
    ExplainLibrariesInfo => [
        "Ajoute un bouton « ? » à côté de chaque bibliothèque de la vue d’ensemble. Vos propres descriptions, dans le fichier ci-dessous, l’emportent sur le catalogue intégré.",
        "Adds a “?” button next to each library in the Overview. Your own descriptions, in the file below, take precedence over the built-in catalogue.",
        "Añade un botón «?» junto a cada biblioteca en el Resumen. Sus descripciones, en el archivo de abajo, prevalecen sobre el catálogo integrado.",
    ],
    CreateLibraryFile => ["Créer le fichier", "Create the file", "Crear el archivo"],
    LibraryFileEntries => ["descriptions à vous", "descriptions of your own", "descripciones propias"],
    ReloadLibraryFile => ["Relire le fichier", "Reload the file", "Releer el archivo"],
    SourceLanguage => ["Langage source", "Source language", "Lenguaje fuente"],
    Toolchain => ["Produit par", "Built with", "Generado con"],
    LanguageUnknown => ["indéterminé", "undetermined", "indeterminado"],
    LanguageUnknownHint => [
        "Le fichier ne porte aucune marque de son langage d’origine. Les binaires dépouillés en perdent souvent la trace ; rien n’est supposé ici.",
        "The file carries no mark of the language it came from. Stripped binaries often lose that trace; nothing is assumed here.",
        "El archivo no lleva ninguna marca de su lenguaje de origen. Los binarios despojados suelen perder ese rastro; aquí no se supone nada.",
    ],
    Certainty => ["Certitude", "Certainty", "Certeza"],
    AlsoTraces => ["porte aussi la trace de", "also carries traces of", "también lleva rastros de"],
    AlsoTracesHint => [
        "Un programme Rust, Go ou C++ embarque le moteur d’exécution du C, et un binaire en porte donc la trace sans avoir été écrit dedans. Ce n’est pas une seconde réponse, c’est ce qui a été lié.",
        "A Rust, Go or C++ program carries the C runtime, so a binary shows its traces without having been written in it. This is not a second answer; it is what was linked in.",
        "Un programa en Rust, Go o C++ incorpora el motor de ejecución de C, así que un binario lleva su rastro sin haber sido escrito en él. No es una segunda respuesta: es lo que se enlazó.",
    ],
    EvidenceCertain => ["certain", "certain", "seguro"],
    EvidenceLikely => ["probable", "likely", "probable"],
    EvidencePossible => ["possible", "possible", "posible"],
    Bytes => ["Octets", "Bytes", "Bytes"],
    Instruction => ["Instruction", "Instruction", "Instrucción"],
    Section => ["Section", "Section", "Sección"],
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
    CopyAddress => ["Copier l’adresse", "Copy the address", "Copiar la dirección"],
    ReferenceHelp => [
        "Clic sur une ligne pour l’ouvrir dans le désassemblage ; clic droit pour le reste.",
        "Click a line to open it in the disassembly; right-click for the rest.",
        "Haga clic en una línea para abrirla en el desensamblado; con el botón derecho, el resto.",
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
    FilterUnmappedStrings => [
        "Masquer les non mappées",
        "Hide unmapped",
        "Ocultar las no mapeadas",
    ],
    FilterUnreferencedStrings => [
        "Masquer les sans référence",
        "Hide unreferenced",
        "Ocultar las sin referencia",
    ],
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
    Stack => ["Pile", "Stack", "Pila"],
    StackHelp => [
        "Profondeur de la pile avant chaque instruction, comptée depuis le début du cadre en suivant les déplacements du pointeur de pile. Lecture locale : elle ne vaut que tant que rien ne saute au milieu du cadre.",
        "Stack depth before each instruction, counted from the start of the frame by following the moves of the stack pointer. A local reading: it holds only while nothing jumps into the middle of the frame.",
        "Profundidad de la pila antes de cada instrucción, contada desde el inicio del marco siguiendo los movimientos del puntero de pila. Lectura local: solo vale mientras nada salte al medio del marco.",
    ],
    StackUnknown => [
        "Le pointeur de pile a été déplacé d’une quantité que le texte ne dit pas ; la profondeur n’est plus connue jusqu’à la fin du cadre.",
        "The stack pointer was moved by an amount the text does not state; the depth is no longer known until the frame ends.",
        "El puntero de pila se movió una cantidad que el texto no indica; la profundidad ya no se conoce hasta el final del marco.",
    ],
    StackEmpty => [
        "Rien de connu sur la pile à ce point.",
        "Nothing known about the stack at this point.",
        "Nada conocido sobre la pila en este punto.",
    ],
    StackFrameNotReached => [
        "Le début du cadre n’a pas été atteint : ce qui est en dessous n’est pas listé.",
        "The start of the frame was not reached: what lies below is not listed.",
        "No se alcanzó el inicio del marco: lo que hay debajo no se lista.",
    ],
    StackReturnAddress => ["adresse de retour", "return address", "dirección de retorno"],
    StackSaved => ["sauvegardé", "saved", "guardado"],
    StackReserved => ["réservé", "reserved", "reservado"],
    StackPushed => ["empilé", "pushed", "apilado"],
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
