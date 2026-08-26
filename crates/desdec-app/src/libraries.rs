//! What a linked library is for.
//!
//! A list of `libselinux.so.1`, `libcap.so.2`, `libc.so.6` says what a binary
//! needs without saying what it *does*. This turns each name into a sentence,
//! so the dependency list reads as a description of the program's reach:
//! cryptography, a database, a graphical toolkit, process privileges.
//!
//! Two sources, in this order:
//!
//! - **A file of the user's own**, so a library this catalogue never heard of —
//!   an in-house one, a niche dependency — can be described once and named
//!   ever after. It also overrides anything below.
//! - **A built-in catalogue**, so the feature is useful the moment it is
//!   switched on rather than after an afternoon of typing.
//!
//! A library that neither source describes is said to be undescribed. Guessing
//! from the name would be worse than silence: `libfoo` could be anything, and a
//! confident wrong sentence about a dependency is how a reader ends up looking
//! in the wrong place.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use crate::i18n::Language;

/// What is known about one library.
pub struct LibraryNote {
    /// Where the description came from, which the dialog says plainly.
    pub source: NoteSource,
    pub summary: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoteSource {
    Builtin,
    /// The user's own file, which takes precedence.
    UserFile,
}

/// Descriptions the user wrote, read once and kept for the session.
#[derive(Default)]
pub struct Catalogue {
    user: BTreeMap<String, String>,
    /// Set once the file has been looked for, so a missing file is not
    /// searched for again on every frame.
    loaded: bool,
}

impl Catalogue {
    /// What this library is for, or `None` when nothing describes it.
    pub fn note(&mut self, library: &str, language: Language) -> Option<LibraryNote> {
        self.load_user_file();
        let key = normalise(library);

        // The user's own words win: they know their dependencies better than a
        // table compiled a year ago.
        let mine = self
            .user
            .iter()
            .filter(|(name, _)| describes(name, &key))
            .max_by_key(|(name, _)| name.len());
        if let Some((_, summary)) = mine {
            return Some(LibraryNote {
                source: NoteSource::UserFile,
                summary: summary.clone(),
            });
        }
        builtin(&key).map(|entry| LibraryNote {
            source: NoteSource::Builtin,
            summary: entry.summary(language).to_owned(),
        })
    }

    fn load_user_file(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let Some(path) = user_catalogue_path() else {
            return;
        };
        let Ok(text) = fs::read_to_string(path) else {
            return; // No file is the normal case, not a failure.
        };
        self.user = parse(&text);
    }

    /// Number of descriptions the user's own file contributed.
    pub fn user_entries(&mut self) -> usize {
        self.load_user_file();
        self.user.len()
    }

    /// Reads the file again, so an edit takes effect without a restart.
    pub fn reload(&mut self) {
        self.loaded = false;
        self.user.clear();
    }
}

/// Where the user's descriptions live.
#[must_use]
pub fn user_catalogue_path() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }?;
    Some(base.join("desdec").join("libraries.conf"))
}

/// The user's file exactly as it stands, for the editor to show.
///
/// A file that is not there yet is not a failure: the editor opens on the
/// example instead, so the reader starts from a filled-in format rather than
/// an empty box.
#[must_use]
pub fn read_user_file(language: Language) -> String {
    user_catalogue_path()
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_else(|| example(language))
}

/// Writes back what the editor holds, replacing the file.
///
/// # Errors
///
/// Returns an error when the directory or the file cannot be written.
pub fn save_user_file(text: &str) -> std::io::Result<PathBuf> {
    let path = user_catalogue_path()
        .ok_or_else(|| std::io::Error::other("no configuration directory on this system"))?;
    save_at(&path, text)?;
    Ok(path)
}

fn save_at(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // A trailing newline, so an editor opening the file next does not show a
    // last line that looks unfinished.
    let text = if text.ends_with('\n') || text.is_empty() {
        text.to_owned()
    } else {
        format!("{text}\n")
    };
    fs::write(path, text)
}

/// Writes a starting file, so the format is shown rather than described.
///
/// # Errors
///
/// Returns an error when the directory or the file cannot be written.
pub fn write_example_file(language: Language) -> std::io::Result<PathBuf> {
    let path = user_catalogue_path()
        .ok_or_else(|| std::io::Error::other("no configuration directory on this system"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // An existing file is never overwritten: it is the user's own writing.
    if path.exists() {
        return Ok(path);
    }
    fs::write(&path, example(language))?;
    Ok(path)
}

fn example(language: Language) -> String {
    let (heading, note) = match language {
        Language::French => (
            "# Descriptions de bibliothèques, une par ligne : nom = explication.",
            "# Le nom est comparé sans son numéro de version, et l'emporte sur le catalogue intégré.",
        ),
        Language::English => (
            "# Library descriptions, one per line: name = explanation.",
            "# The name is matched without its version number, and overrides the built-in catalogue.",
        ),
        Language::Spanish => (
            "# Descripciones de bibliotecas, una por línea: nombre = explicación.",
            "# El nombre se compara sin su número de versión y prevalece sobre el catálogo integrado.",
        ),
    };
    format!(
        "{heading}\n{note}\n\n# libmaison = Bibliothèque interne : accès au format de fichier maison.\n"
    )
}

/// The name a library goes by in the file, for an editor filling in a line.
#[must_use]
pub fn catalogue_key(library: &str) -> String {
    normalise(library)
}

/// Whether the text of a file already describes `library`, under any of the
/// names it goes by: a second line for a name already answered would leave two
/// answers in the file.
#[must_use]
pub fn describes_any(text: &str, library: &str) -> bool {
    let name = normalise(library);
    parse(text).keys().any(|key| describes(key, &name))
}

/// Reads `name = description` lines, ignoring blanks and `#` comments.
fn parse(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(name, summary)| (normalise(name.trim()), summary.trim().to_owned()))
        .filter(|(name, summary)| !name.is_empty() && !summary.is_empty())
        .collect()
}

/// Reduces a file name to the library's identity, keeping any version digits.
///
/// `libssl.so.3`, `LIBSSL-3.dll` and `/usr/lib/libssl.dylib` all reduce to
/// `libssl`, so a reader need not describe each spelling separately. The
/// digits inside a name are kept — `libqt6core` and `kernel32` are the names
/// those libraries actually go by — and [`describes`] does the matching.
fn normalise(library: &str) -> String {
    let name = library
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(library)
        .to_lowercase();
    let name = name
        .split_once(".so")
        .map_or(name.as_str(), |(head, _)| head);
    let name = name.strip_suffix(".dylib").unwrap_or(name);
    let name = name.strip_suffix(".dll").unwrap_or(name);
    // A trailing separator and version, as in `libssl-3`, but never the digits
    // that belong to the name itself.
    name.split_once('-')
        .filter(|(_, tail)| tail.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .map_or(name, |(head, _)| head)
        .to_owned()
}

/// Whether a catalogue key describes a library.
///
/// A key matches the whole name, or a prefix of it that ends on a boundary:
/// what follows must be a version or a separator, never more letters. Without
/// that rule `libc` would claim `libcompletelyunknown`, and the reader would
/// be told an unknown dependency is the C standard library. `libqt` still
/// reaches `libqt6core`, whose digits are part of the name.
fn describes(key: &str, name: &str) -> bool {
    let Some(rest) = name.strip_prefix(key) else {
        return false;
    };
    rest.chars().next().is_none_or(|c| !c.is_alphabetic())
}

/// One built-in description, in the three interface languages.
struct Builtin {
    french: &'static str,
    english: &'static str,
    spanish: &'static str,
}

impl Builtin {
    const fn summary(&self, language: Language) -> &'static str {
        match language {
            Language::French => self.french,
            Language::English => self.english,
            Language::Spanish => self.spanish,
        }
    }
}

/// Declares the catalogue once, keyed by the normalised library name.
macro_rules! catalogue {
    ($($name:literal => [$french:literal, $english:literal, $spanish:literal $(,)?]),+ $(,)?) => {
        /// The longest catalogue key that describes `name`.
        fn builtin(name: &str) -> Option<Builtin> {
            let mut best: Option<(&str, Builtin)> = None;
            $(
                if describes($name, name)
                    && best.as_ref().is_none_or(|(key, _)| $name.len() > key.len())
                {
                    best = Some((
                        $name,
                        Builtin { french: $french, english: $english, spanish: $spanish },
                    ));
                }
            )+
            best.map(|(_, entry)| entry)
        }

        #[cfg(test)]
        const CATALOGUE_NAMES: &[&str] = &[$($name,)+];
    };
}

catalogue! {
    "libc" => [
        "Bibliothèque C standard : entrées-sorties, mémoire, chaînes, appels système. Presque tout programme natif en dépend ; sa présence n’apprend rien de particulier.",
        "The standard C library: input and output, memory, strings, system calls. Nearly every native program depends on it, so its presence says little on its own.",
        "Biblioteca C estándar: entrada y salida, memoria, cadenas, llamadas al sistema. Casi todo programa nativo depende de ella.",
    ],
    "libm" => [
        "Fonctions mathématiques en virgule flottante : racines, trigonométrie, exponentielles. Souvent tirée par le compilateur plutôt que demandée explicitement.",
        "Floating-point mathematics: roots, trigonometry, exponentials. Often pulled in by the compiler rather than asked for.",
        "Matemáticas de coma flotante: raíces, trigonometría, exponenciales. A menudo la incluye el compilador.",
    ],
    "libpthread" => [
        "Fils d’exécution POSIX : création de threads, verrous, conditions. Indique un programme conçu pour faire plusieurs choses à la fois. Fondue dans libc sur les systèmes récents.",
        "POSIX threads: thread creation, locks, condition variables. Marks a program built to do several things at once. Folded into libc on recent systems.",
        "Hilos POSIX: creación de hilos, cerrojos, condiciones. Indica un programa concurrente. Integrada en libc en sistemas recientes.",
    ],
    "libdl" => [
        "Chargement dynamique : ouvre une bibliothèque et en résout les symboles à l’exécution. À regarder de près — c’est ainsi qu’un programme charge du code décidé au dernier moment.",
        "Dynamic loading: opens a library and resolves its symbols at run time. Worth a close look — this is how a program loads code chosen at the last moment.",
        "Carga dinámica: abre una biblioteca y resuelve sus símbolos en ejecución. Merece atención: así se carga código decidido en el último momento.",
    ],
    "libstdc++" => [
        "Bibliothèque standard C++ de GNU : conteneurs, flux, exceptions. Sa présence dit que le programme est écrit en C++ ou appelle du C++.",
        "GNU’s C++ standard library: containers, streams, exceptions. Its presence says the program is written in C++, or calls into it.",
        "Biblioteca estándar de C++ de GNU: contenedores, flujos, excepciones. Indica que el programa es C++ o llama a C++.",
    ],
    "libc++" => [
        "Bibliothèque standard C++ de LLVM, équivalente à libstdc++. Usuelle sur macOS et avec clang.",
        "LLVM’s C++ standard library, the counterpart to libstdc++. Usual on macOS and with clang.",
        "Biblioteca estándar de C++ de LLVM, equivalente a libstdc++. Habitual en macOS y con clang.",
    ],
    "libssl" => [
        "TLS/SSL d’OpenSSL : établit des connexions chiffrées. Le programme parle à un réseau et protège ce qu’il envoie — repérez à quels hôtes.",
        "OpenSSL’s TLS/SSL: establishes encrypted connections. The program talks to a network and protects what it sends — look for which hosts.",
        "TLS/SSL de OpenSSL: establece conexiones cifradas. El programa usa la red y protege lo que envía.",
    ],
    "libcrypto" => [
        "Primitives cryptographiques d’OpenSSL : chiffrement, signatures, empreintes, nombres aléatoires. Utilisée seule, elle sert souvent à chiffrer des fichiers ou vérifier des signatures.",
        "OpenSSL’s cryptographic primitives: ciphers, signatures, digests, random numbers. On its own it often means files are encrypted or signatures checked.",
        "Primitivas criptográficas de OpenSSL: cifrado, firmas, resúmenes, números aleatorios.",
    ],
    "libz" => [
        "Compression zlib (deflate) : compresse et décompresse des données. Explique une section dense sans qu’il s’agisse d’un empaqueteur.",
        "zlib compression (deflate): compresses and decompresses data. Explains a dense section without a packer being involved.",
        "Compresión zlib (deflate): comprime y descomprime datos. Explica una sección densa sin que haya empaquetador.",
    ],
    "libcurl" => [
        "Transferts réseau : HTTP, FTP et une douzaine d’autres protocoles. Le programme va chercher ou envoyer des données à distance.",
        "Network transfers: HTTP, FTP and a dozen other protocols. The program fetches or sends data remotely.",
        "Transferencias de red: HTTP, FTP y una docena de protocolos más.",
    ],
    "libsqlite" => [
        "Base de données SQLite, stockée dans un simple fichier. Le programme conserve des données structurées localement.",
        "The SQLite database, kept in a single file. The program stores structured data locally.",
        "Base de datos SQLite, guardada en un solo archivo. El programa almacena datos estructurados localmente.",
    ],
    "libselinux" => [
        "SELinux : consulte et applique la politique de sécurité obligatoire du système. Fréquent dans les utilitaires système sur Fedora et RHEL.",
        "SELinux: reads and applies the system’s mandatory security policy. Common in system utilities on Fedora and RHEL.",
        "SELinux: consulta y aplica la política de seguridad obligatoria del sistema.",
    ],
    "libcap" => [
        "Capacités POSIX : accorde ou retire des privilèges précis, plus fins que « root ». Signale un programme qui manipule ses propres droits.",
        "POSIX capabilities: grants or drops specific privileges, finer-grained than root. Marks a program that manages its own rights.",
        "Capacidades POSIX: concede o retira privilegios concretos, más finos que root.",
    ],
    "libpam" => [
        "Authentification enfichable : vérifie mots de passe et jetons. Le programme décide qui a le droit d’entrer.",
        "Pluggable authentication: checks passwords and tokens. The program decides who may log in.",
        "Autenticación conectable: verifica contraseñas y tokens.",
    ],
    "libcrypt" => [
        "Hachage de mots de passe (crypt). Presque toujours lié à la vérification d’identifiants.",
        "Password hashing (crypt). Almost always tied to checking credentials.",
        "Hash de contraseñas (crypt). Casi siempre ligado a la verificación de credenciales.",
    ],
    "libgtk" => [
        "GTK : boîte à outils graphique. Le programme ouvre des fenêtres — ce n’est pas un outil en ligne de commande.",
        "GTK: a graphical toolkit. The program opens windows — it is not a command-line tool.",
        "GTK: kit gráfico. El programa abre ventanas.",
    ],
    "libqt" => [
        "Qt : cadre applicatif graphique et multiplateforme. Interface fenêtrée, souvent en C++.",
        "Qt: a cross-platform application framework. A windowed interface, usually in C++.",
        "Qt: marco de aplicaciones multiplataforma. Interfaz con ventanas, normalmente en C++.",
    ],
    "libx" => [
        "X11 : protocole d’affichage historique d’Unix. Le programme dessine sur un serveur X.",
        "X11: the historical Unix display protocol. The program draws on an X server.",
        "X11: protocolo de pantalla histórico de Unix.",
    ],
    "libwayland" => [
        "Wayland : protocole d’affichage moderne sous Linux, successeur de X11.",
        "Wayland: the modern Linux display protocol, successor to X11.",
        "Wayland: protocolo de pantalla moderno en Linux, sucesor de X11.",
    ],
    "libsystemd" => [
        "systemd : journal, unités, sockets d’activation. Le programme s’intègre au gestionnaire de services.",
        "systemd: the journal, units, socket activation. The program integrates with the service manager.",
        "systemd: diario, unidades, sockets de activación.",
    ],
    "libudev" => [
        "udev : énumère les périphériques et suit leurs branchements.",
        "udev: enumerates devices and follows them being plugged in and out.",
        "udev: enumera dispositivos y sigue sus conexiones.",
    ],
    "libgcc_s" => [
        "Fonctions d’appoint du compilateur GCC : arithmétique large, déroulement de pile pour les exceptions. Ajoutée par le compilateur, pas demandée par le programme.",
        "GCC’s support routines: wide arithmetic, stack unwinding for exceptions. Added by the compiler, not asked for by the program.",
        "Rutinas de apoyo de GCC: aritmética ancha, desenrollado de pila.",
    ],
    "kernel32" => [
        "Windows : cœur de l’API système — fichiers, mémoire, processus, threads. Présente dans presque tout exécutable Windows.",
        "Windows: the core system API — files, memory, processes, threads. Present in nearly every Windows executable.",
        "Windows: núcleo de la API del sistema — archivos, memoria, procesos, hilos.",
    ],
    "ntdll" => [
        "Windows : interface de plus bas niveau vers le noyau. Un programme qui l’appelle directement contourne les couches usuelles — cela mérite un regard.",
        "Windows: the lowest-level interface to the kernel. A program calling it directly bypasses the usual layers, which is worth a look.",
        "Windows: interfaz de más bajo nivel con el núcleo. Llamarla directamente evita las capas habituales.",
    ],
    "advapi" => [
        "Windows : registre, services, jetons de sécurité et chiffrement hérité.",
        "Windows: the registry, services, security tokens and legacy cryptography.",
        "Windows: registro, servicios, tokens de seguridad y criptografía heredada.",
    ],
    "ws2" => [
        "Windows Sockets : communications réseau. Le programme ouvre des connexions.",
        "Windows Sockets: network communication. The program opens connections.",
        "Windows Sockets: comunicación de red. El programa abre conexiones.",
    ],
    "wininet" => [
        "Windows : client HTTP et FTP de haut niveau. Le programme récupère des ressources sur le réseau.",
        "Windows: a high-level HTTP and FTP client. The program fetches resources over the network.",
        "Windows: cliente HTTP y FTP de alto nivel.",
    ],
    "user" => [
        "Windows : fenêtres, messages, entrées clavier et souris. Programme doté d’une interface graphique.",
        "Windows: windows, messages, keyboard and mouse input. A program with a graphical interface.",
        "Windows: ventanas, mensajes, entrada de teclado y ratón.",
    ],
    "gdi" => [
        "Windows : dessin à l’écran et impression.",
        "Windows: drawing on screen and printing.",
        "Windows: dibujo en pantalla e impresión.",
    ],
    "msvcp" => [
        "Windows : bibliothèque standard C++ de Microsoft. Le programme est écrit en C++.",
        "Windows: Microsoft’s C++ standard library. The program is written in C++.",
        "Windows: biblioteca estándar de C++ de Microsoft. El programa está escrito en C++.",
    ],
    "vcruntime" => [
        "Windows : soutien d’exécution du compilateur Microsoft — exceptions, initialisations.",
        "Windows: the Microsoft compiler’s run-time support — exceptions, initialisation.",
        "Windows: soporte de ejecución del compilador de Microsoft.",
    ],
    "ucrtbase" => [
        "Windows : bibliothèque C universelle — entrées-sorties, mémoire, chaînes. L’équivalent de libc ; presque tout programme compilé récemment en dépend.",
        "Windows: the universal C runtime — input and output, memory, strings. The counterpart of libc; nearly every recently built program depends on it.",
        "Windows: biblioteca C universal — entrada y salida, memoria, cadenas. El equivalente de libc.",
    ],
    "msvcrt" => [
        "Windows : ancienne bibliothèque C de Microsoft, antérieure à ucrtbase. Programme ancien, ou construit avec une chaîne d’outils qui l’est.",
        "Windows: Microsoft’s older C runtime, the one that came before ucrtbase. An old program, or one built with an old toolchain.",
        "Windows: antigua biblioteca C de Microsoft, anterior a ucrtbase.",
    ],
    "api-ms-win" => [
        "Windows : nom de façade renvoyant vers la vraie bibliothèque système au chargement. Il désigne une famille d’appels plutôt qu’un fichier ; ce que le programme fait se lit au suffixe.",
        "Windows: a façade name resolved to the real system library at load time. It names a family of calls rather than a file; the suffix says which.",
        "Windows: nombre de fachada que se resuelve a la biblioteca real al cargar. Designa una familia de llamadas, no un archivo.",
    ],
    "api-ms-win-crt" => [
        "Windows : façade de la bibliothèque C universelle, résolue vers ucrtbase au chargement. Sa présence dit seulement que le programme est en C ou C++.",
        "Windows: a façade for the universal C runtime, resolved to ucrtbase at load time. It says only that the program is C or C++.",
        "Windows: fachada de la biblioteca C universal, resuelta a ucrtbase al cargar.",
    ],
    "kernelbase" => [
        "Windows : socle où vit désormais l’essentiel de l’API système, sous kernel32. Ajoutée par la chaîne d’outils plus que demandée.",
        "Windows: the base where most of the system API now lives, underneath kernel32. Added by the toolchain more than asked for.",
        "Windows: base donde reside hoy la mayor parte de la API del sistema, bajo kernel32.",
    ],
    "win32u" => [
        "Windows : passage direct aux appels systèmes des fenêtres et du dessin. Un programme qui l’appelle lui-même contourne user32 et gdi32 — cela mérite un regard.",
        "Windows: the direct path to the window and drawing system calls. A program calling it itself bypasses user32 and gdi32, which is worth a look.",
        "Windows: paso directo a las llamadas del sistema de ventanas y dibujo, evitando user32 y gdi32.",
    ],
    "sechost" => [
        "Windows : services, journal des événements et jetons de sécurité, séparés d’advapi32.",
        "Windows: services, the event log and security tokens, split out of advapi32.",
        "Windows: servicios, registro de eventos y tokens de seguridad, separados de advapi32.",
    ],
    "shell32" => [
        "Windows : explorateur et bureau — icônes, dossiers spéciaux, lancement de fichiers. Sert aussi à démarrer un autre programme, ce qui vaut d’être vérifié.",
        "Windows: the shell — icons, special folders, launching files. Also how one program starts another, which is worth checking.",
        "Windows: el shell — iconos, carpetas especiales, apertura de archivos. También sirve para lanzar otro programa.",
    ],
    "shlwapi" => [
        "Windows : utilitaires du shell — chemins, URL, registre, chaînes.",
        "Windows: shell helpers — paths, URLs, the registry, strings.",
        "Windows: utilidades del shell — rutas, URL, registro, cadenas.",
    ],
    "shcore" => [
        "Windows : mise à l’échelle de l’affichage et flux de données du shell.",
        "Windows: display scaling and the shell’s data streams.",
        "Windows: escalado de pantalla y flujos de datos del shell.",
    ],
    "comctl32" => [
        "Windows : contrôles communs — listes, arborescences, barres d’outils. Interface graphique classique.",
        "Windows: the common controls — lists, trees, toolbars. A classic graphical interface.",
        "Windows: controles comunes — listas, árboles, barras de herramientas.",
    ],
    "comdlg32" => [
        "Windows : boîtes de dialogue standard — ouvrir, enregistrer, imprimer, choisir une couleur.",
        "Windows: the standard dialogs — open, save, print, choose a colour.",
        "Windows: diálogos estándar — abrir, guardar, imprimir, elegir color.",
    ],
    "uxtheme" => [
        "Windows : apparence des contrôles selon le thème visuel.",
        "Windows: the visual theme applied to controls.",
        "Windows: apariencia de los controles según el tema visual.",
    ],
    "imm32" => [
        "Windows : saisie des écritures qui passent par un éditeur de méthode d’entrée.",
        "Windows: text entry for scripts that go through an input method editor.",
        "Windows: entrada de texto mediante un editor de método de entrada.",
    ],
    "ole32" => [
        "Windows : COM — création d’objets, appels entre processus, presse-papiers, stockage structuré. Le programme parle à des composants du système ou d’un autre logiciel.",
        "Windows: COM — creating objects, calls across processes, the clipboard, structured storage. The program talks to system or third-party components.",
        "Windows: COM — creación de objetos, llamadas entre procesos, portapapeles, almacenamiento estructurado.",
    ],
    "combase" => [
        "Windows : cœur moderne de COM, sous ole32. Sert aussi les API WinRT.",
        "Windows: the modern core of COM, underneath ole32. Also serves the WinRT APIs.",
        "Windows: núcleo moderno de COM, bajo ole32. También sirve a las API WinRT.",
    ],
    "oleaut32" => [
        "Windows : automation OLE — chaînes BSTR, variants, appels par nom. Usuelle avec les scripts et les composants pilotés à distance.",
        "Windows: OLE automation — BSTR strings, variants, calls by name. Usual with scripts and remotely driven components.",
        "Windows: automatización OLE — cadenas BSTR, variantes, llamadas por nombre.",
    ],
    "oleacc" => [
        "Windows : accessibilité — expose l’interface aux lecteurs d’écran, et permet de la lire depuis un autre programme.",
        "Windows: accessibility — exposes the interface to screen readers, and lets another program read it.",
        "Windows: accesibilidad — expone la interfaz a lectores de pantalla.",
    ],
    "propsys" => [
        "Windows : propriétés des fichiers et des éléments du shell — auteur, date, étiquettes.",
        "Windows: the property system for files and shell items — author, date, tags.",
        "Windows: sistema de propiedades de archivos y elementos del shell.",
    ],
    "rpcrt4" => [
        "Windows : appels de procédure à distance, qui portent aussi COM entre processus. Communication entre programmes, parfois entre machines.",
        "Windows: remote procedure calls, which also carry COM between processes. Communication between programs, sometimes between machines.",
        "Windows: llamadas a procedimiento remoto, que también transportan COM entre procesos.",
    ],
    "crypt32" => [
        "Windows : certificats, magasins de certificats et chiffrement de données. Souvent lié à la vérification d’une signature ou à la protection de secrets.",
        "Windows: certificates, certificate stores and data protection. Often tied to checking a signature or protecting secrets.",
        "Windows: certificados, almacenes de certificados y protección de datos.",
    ],
    "bcrypt" => [
        "Windows : primitives cryptographiques modernes — chiffrement, empreintes, aléa. Rien à voir avec le hachage de mots de passe du même nom.",
        "Windows: the modern cryptographic primitives — ciphers, digests, randomness. Unrelated to the password hash of the same name.",
        "Windows: primitivas criptográficas modernas — cifrado, resúmenes, aleatoriedad.",
    ],
    "ncrypt" => [
        "Windows : clés cryptographiques stockées et protégées par le système, y compris par matériel.",
        "Windows: cryptographic keys stored and protected by the system, hardware included.",
        "Windows: claves criptográficas almacenadas y protegidas por el sistema.",
    ],
    "secur32" => [
        "Windows : authentification par le système — Kerberos, NTLM, canal sécurisé. Le programme prouve une identité ou en vérifie une.",
        "Windows: authentication through the system — Kerberos, NTLM, secure channel. The program proves an identity or checks one.",
        "Windows: autenticación del sistema — Kerberos, NTLM, canal seguro.",
    ],
    "wintrust" => [
        "Windows : vérification des signatures de fichiers et de catalogues.",
        "Windows: checking the signatures of files and catalogues.",
        "Windows: verificación de firmas de archivos y catálogos.",
    ],
    "winhttp" => [
        "Windows : client HTTP destiné aux services. Le programme parle à un serveur sans passer par le navigateur.",
        "Windows: an HTTP client meant for services. The program talks to a server without going through the browser.",
        "Windows: cliente HTTP pensado para servicios.",
    ],
    "urlmon" => [
        "Windows : récupération d’une ressource par son URL, héritée d’Internet Explorer. Un téléchargement en une ligne — regardez d’où.",
        "Windows: fetching a resource by URL, inherited from Internet Explorer. A download in one call — look at where from.",
        "Windows: descarga de un recurso por su URL, heredada de Internet Explorer.",
    ],
    "iphlpapi" => [
        "Windows : configuration réseau de la machine — interfaces, adresses, table de routage, connexions ouvertes.",
        "Windows: the machine’s network configuration — interfaces, addresses, routing table, open connections.",
        "Windows: configuración de red del equipo — interfaces, direcciones, rutas, conexiones.",
    ],
    "dnsapi" => [
        "Windows : résolution de noms DNS. Le programme cherche à joindre un hôte par son nom.",
        "Windows: DNS name resolution. The program is reaching a host by name.",
        "Windows: resolución de nombres DNS.",
    ],
    "netapi32" => [
        "Windows : administration réseau — partages, sessions, comptes du domaine. Fréquente dans les outils d’inventaire, et dans ce qui les imite.",
        "Windows: network administration — shares, sessions, domain accounts. Common in inventory tools, and in what imitates them.",
        "Windows: administración de red — recursos compartidos, sesiones, cuentas de dominio.",
    ],
    "mpr" => [
        "Windows : lecteurs et partages réseau — connexion, énumération.",
        "Windows: network drives and shares — connecting to them, listing them.",
        "Windows: unidades y recursos compartidos de red.",
    ],
    "psapi" => [
        "Windows : énumération des processus et de leurs modules chargés. Le programme regarde ce qui tourne sur la machine.",
        "Windows: listing processes and the modules they have loaded. The program looks at what else is running.",
        "Windows: enumeración de procesos y de sus módulos cargados.",
    ],
    "dbghelp" => [
        "Windows : symboles de débogage, piles d’appels et vidages mémoire. Outil de diagnostic — ou programme qui s’intéresse à la mémoire d’un autre.",
        "Windows: debug symbols, call stacks and memory dumps. A diagnostic tool — or a program interested in another’s memory.",
        "Windows: símbolos de depuración, pilas de llamadas y volcados de memoria.",
    ],
    "version" => [
        "Windows : lecture du bloc de version d’un fichier — éditeur, version du produit.",
        "Windows: reading a file’s version block — publisher, product version.",
        "Windows: lectura del bloque de versión de un archivo.",
    ],
    "userenv" => [
        "Windows : profils utilisateur et stratégies de groupe.",
        "Windows: user profiles and group policy.",
        "Windows: perfiles de usuario y directivas de grupo.",
    ],
    "setupapi" => [
        "Windows : installation de pilotes et énumération des périphériques.",
        "Windows: installing drivers and enumerating devices.",
        "Windows: instalación de controladores y enumeración de dispositivos.",
    ],
    "cfgmgr32" => [
        "Windows : gestionnaire de configuration — l’arbre des périphériques présents.",
        "Windows: the configuration manager — the tree of devices present.",
        "Windows: gestor de configuración — el árbol de dispositivos presentes.",
    ],
    "hid" => [
        "Windows : périphériques d’interface humaine — claviers, manettes, lecteurs et autres appareils USB.",
        "Windows: human interface devices — keyboards, controllers, readers and other USB gadgets.",
        "Windows: dispositivos de interfaz humana — teclados, mandos, lectores USB.",
    ],
    "winspool" => [
        "Windows : file d’impression — imprimantes, travaux, pilotes.",
        "Windows: the print spooler — printers, jobs, drivers.",
        "Windows: cola de impresión — impresoras, trabajos, controladores.",
    ],
    "msi" => [
        "Windows : moteur d’installation MSI — installe, répare et interroge les produits installés.",
        "Windows: the MSI installer engine — installs, repairs and queries installed products.",
        "Windows: motor de instalación MSI.",
    ],
    "cabinet" => [
        "Windows : archives CAB — compression et extraction. Explique une section dense, et sert souvent à livrer des fichiers.",
        "Windows: CAB archives — compressing and extracting. Explains a dense section, and is a common way to deliver files.",
        "Windows: archivos CAB — compresión y extracción.",
    ],
    "xmllite" => [
        "Windows : lecture et écriture de XML.",
        "Windows: reading and writing XML.",
        "Windows: lectura y escritura de XML.",
    ],
    "windowscodecs" => [
        "Windows : décodage et encodage d’images — JPEG, PNG, TIFF et le reste.",
        "Windows: decoding and encoding images — JPEG, PNG, TIFF and the rest.",
        "Windows: decodificación y codificación de imágenes.",
    ],
    "winmm" => [
        "Windows : multimédia hérité — son, MIDI, minuteries de précision.",
        "Windows: legacy multimedia — sound, MIDI, high-resolution timers.",
        "Windows: multimedia heredado — sonido, MIDI, temporizadores precisos.",
    ],
    "dsound" => [
        "Windows : DirectSound — lecture et capture audio.",
        "Windows: DirectSound — playing and capturing audio.",
        "Windows: DirectSound — reproducción y captura de audio.",
    ],
    "mfplat" => [
        "Windows : Media Foundation — lecture et encodage audio et vidéo.",
        "Windows: Media Foundation — playing and encoding audio and video.",
        "Windows: Media Foundation — reproducción y codificación de audio y vídeo.",
    ],
    "d3d" => [
        "Windows : Direct3D — rendu graphique accéléré par la carte. Jeu, lecteur vidéo ou interface qui dessine elle-même.",
        "Windows: Direct3D — rendering on the graphics card. A game, a video player, or an interface that draws its own.",
        "Windows: Direct3D — renderizado acelerado por la tarjeta gráfica.",
    ],
    "d3dcompiler" => [
        "Windows : compilation des nuanceurs HLSL, à l’exécution ou à l’avance.",
        "Windows: compiling HLSL shaders, at run time or ahead of it.",
        "Windows: compilación de sombreadores HLSL.",
    ],
    "dxgi" => [
        "Windows : couche commune de DirectX — adaptateurs, écrans, chaînes d’échange.",
        "Windows: the DirectX common layer — adapters, displays, swap chains.",
        "Windows: capa común de DirectX — adaptadores, pantallas, cadenas de intercambio.",
    ],
    "ddraw" => [
        "Windows : DirectDraw — dessin en deux dimensions des premières versions de DirectX.",
        "Windows: DirectDraw — the two-dimensional drawing of the early DirectX versions.",
        "Windows: DirectDraw — dibujo bidimensional de las primeras versiones de DirectX.",
    ],
    "opengl32" => [
        "Windows : OpenGL — rendu graphique portable.",
        "Windows: OpenGL — portable graphics rendering.",
        "Windows: OpenGL — renderizado gráfico portable.",
    ],
    "gdiplus" => [
        "Windows : GDI+ — dessin vectoriel, images et texte lissé.",
        "Windows: GDI+ — vector drawing, images and antialiased text.",
        "Windows: GDI+ — dibujo vectorial, imágenes y texto suavizado.",
    ],
    "mscoree" => [
        "Windows : amorce du moteur .NET. Le fichier n’est pas du code machine mais un assemblage géré : lisez-le avec un décompilateur .NET.",
        "Windows: the .NET runtime shim. The file is not machine code but a managed assembly: read it with a .NET decompiler.",
        "Windows: arranque del motor .NET. El archivo es un ensamblado gestionado, no código máquina.",
    ],
    "ntoskrnl" => [
        "Windows : le noyau lui-même. Un fichier qui l’importe est un pilote, et s’exécute avec tous les droits de la machine.",
        "Windows: the kernel itself. A file importing it is a driver, and runs with the whole machine’s privileges.",
        "Windows: el núcleo mismo. Un archivo que lo importa es un controlador y se ejecuta con todos los privilegios.",
    ],
    "libobjc" => [
        "Objective-C : envoi de messages et introspection des classes. Usuelle sur macOS, y compris depuis d’autres langages.",
        "Objective-C: message dispatch and class introspection. Usual on macOS, including from other languages.",
        "Objective-C: envío de mensajes e introspección de clases. Habitual en macOS.",
    ],
    "libswiftcore" => [
        "Bibliothèque d’exécution de Swift. Le programme est écrit en Swift.",
        "Swift’s runtime library. The program is written in Swift.",
        "Biblioteca de ejecución de Swift. El programa está escrito en Swift.",
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_is_recognised_whatever_its_spelling() {
        // The same library, as each platform and each linker writes it.
        for spelling in [
            "libssl.so.3",
            "libssl.so",
            "/usr/lib64/libssl.so.3",
            "libssl.dylib",
            "LIBSSL-3.dll",
        ] {
            assert_eq!(normalise(spelling), "libssl", "{spelling}");
        }
    }

    #[test]
    fn every_catalogue_entry_is_reachable_and_translated() {
        for name in CATALOGUE_NAMES {
            let entry = builtin(name).expect("a declared entry must be found");
            for language in Language::ALL {
                assert!(!entry.summary(*language).is_empty(), "{name} {language:?}");
            }
        }
    }

    /// A catalogue keyed on names that normalisation would never produce is a
    /// catalogue nothing reaches.
    #[test]
    fn catalogue_keys_survive_normalisation() {
        for name in CATALOGUE_NAMES {
            assert_eq!(&normalise(name), name, "{name} would never be looked up");
        }
    }

    /// The boundary rule, on the names that made it necessary. A prefix must
    /// end on a version or a separator, never on more letters.
    #[test]
    fn a_prefix_only_matches_on_a_boundary() {
        // Real names, as each platform writes them.
        for (library, expected) in [
            ("libc.so.6", Some("libc")),
            ("libcurl.so.4", Some("libcurl")),
            ("libcap.so.2", Some("libcap")),
            ("libcrypto.so.3", Some("libcrypto")),
            ("libQt6Core.so.6", Some("libqt")),
            ("libX11.so.6", Some("libx")),
            ("kernel32.dll", Some("kernel32")),
            ("USER32.dll", Some("user")),
            ("ws2_32.dll", Some("ws2")),
            ("msvcp140.dll", Some("msvcp")),
            ("libsqlite3.so.0", Some("libsqlite")),
            // Nothing in the catalogue may claim these.
            ("libcompletelyunknown.so.1", None),
            ("libcairo.so.2", None),
            ("libuseful.so", None),
        ] {
            let key = normalise(library);
            let matched = CATALOGUE_NAMES
                .iter()
                .filter(|name| describes(name, &key))
                .max_by_key(|name| name.len())
                .copied();
            assert_eq!(matched, expected, "{library} normalised to {key}");
        }
    }

    /// What a Windows binary really imports, taken from the import tables of
    /// the system's own executables.
    ///
    /// Windows names none of its libraries `lib…`, and the catalogue used to
    /// hold barely a handful of them: a reader who opened a PE and asked what
    /// its dependencies were for was told nothing about almost every one.
    #[test]
    fn the_libraries_a_windows_binary_imports_are_described() {
        let mut catalogue = Catalogue {
            loaded: true,
            ..Catalogue::default()
        };
        for library in [
            "KERNEL32.dll",
            "KERNELBASE.dll",
            "ntdll.dll",
            "ucrtbase.dll",
            "MSVCRT.dll",
            "VCRUNTIME140.dll",
            "MSVCP140.dll",
            "api-ms-win-crt-runtime-l1-1-0.dll",
            "api-ms-win-core-synch-l1-2-0.dll",
            "USER32.dll",
            "GDI32.dll",
            "ADVAPI32.dll",
            "sechost.dll",
            "SHELL32.dll",
            "SHLWAPI.dll",
            "COMCTL32.dll",
            "COMDLG32.dll",
            "ole32.dll",
            "combase.dll",
            "OLEAUT32.dll",
            "RPCRT4.dll",
            "WS2_32.dll",
            "WININET.dll",
            "WINHTTP.dll",
            "IPHLPAPI.DLL",
            "CRYPT32.dll",
            "bcrypt.dll",
            "SETUPAPI.dll",
            "VERSION.dll",
            "WINMM.dll",
            "dxgi.dll",
            "d3d11.dll",
            "OPENGL32.dll",
            "mscoree.dll",
            "ntoskrnl.exe",
        ] {
            assert!(
                catalogue.note(library, Language::French).is_some(),
                "{library} reached nothing in the catalogue"
            );
        }
    }

    #[test]
    fn a_library_nobody_described_stays_undescribed() {
        let mut catalogue = Catalogue::default();
        assert!(
            catalogue
                .note("libcompletelyunknown.so.1", Language::French)
                .is_none()
        );
    }

    /// What the editor saves must be what the catalogue reads back: a round
    /// trip through the file is the whole point of editing it in Desdec.
    #[test]
    fn what_the_editor_saves_is_what_the_catalogue_reads() {
        let directory = std::env::temp_dir().join(format!(
            "desdec-libraries-{}-{}",
            std::process::id(),
            line!()
        ));
        let path = directory.join("nested").join("libraries.conf");
        save_at(&path, "libmaison = Format maison.").expect("a temporary file is writable");

        let written = fs::read_to_string(&path).expect("just written");
        // A trailing newline, so the last line never looks unfinished.
        assert!(written.ends_with('\n'), "{written:?}");
        let parsed = parse(&written);
        assert_eq!(
            parsed.get("libmaison").map(String::as_str),
            Some("Format maison.")
        );

        // Saving again replaces the file rather than appending to it.
        save_at(&path, "libmaison = Autre chose.\n").expect("writable");
        let parsed = parse(&fs::read_to_string(&path).expect("just written"));
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed.get("libmaison").map(String::as_str),
            Some("Autre chose.")
        );

        let _ = fs::remove_dir_all(&directory);
    }

    /// The editor opens on something to read even when there is no file yet:
    /// an empty box says nothing about the format.
    #[test]
    fn the_editor_opens_on_the_example_when_there_is_no_file() {
        for language in Language::ALL {
            let example = example(*language);
            assert!(example.contains('='), "{language:?}: {example:?}");
            assert!(example.starts_with('#'), "{language:?}: {example:?}");
        }
    }

    #[test]
    fn the_user_file_is_read_as_name_equals_description() {
        let parsed = parse(
            "# a comment\n\nlibfoo.so.1 = An in-house library.\n  libbar = Another one.  \nrubbish\n",
        );

        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed.get("libfoo").map(String::as_str),
            Some("An in-house library.")
        );
        assert_eq!(
            parsed.get("libbar").map(String::as_str),
            Some("Another one.")
        );
    }

    /// The user knows their own dependencies better than a table compiled a
    /// year ago, so their words win.
    #[test]
    fn the_user_file_overrides_the_builtin_catalogue() {
        let mut catalogue = Catalogue {
            user: parse("libc = Our own words about libc."),
            loaded: true,
        };

        let note = catalogue
            .note("libc.so.6", Language::French)
            .expect("known");
        assert_eq!(note.source, NoteSource::UserFile);
        assert_eq!(note.summary, "Our own words about libc.");
    }

    #[test]
    fn the_builtin_catalogue_answers_when_the_user_file_is_silent() {
        let mut catalogue = Catalogue {
            loaded: true,
            ..Catalogue::default()
        };

        let note = catalogue
            .note("libc.so.6", Language::English)
            .expect("known");
        assert_eq!(note.source, NoteSource::Builtin);
        assert!(note.summary.contains("standard C library"));
    }

    #[test]
    fn a_malformed_user_file_is_ignored_rather_than_fatal() {
        assert!(parse("").is_empty());
        assert!(parse("no equals sign here").is_empty());
        assert!(parse("= no name").is_empty());
        assert!(parse("no description =").is_empty());
    }
}
