//! What language a binary was built from, and what produced it.
//!
//! A compiled file does not record its source language as a field, so this
//! reports **evidence rather than a verdict**: each finding names what was
//! actually found in the file, and the caller shows it next to the language it
//! points at. A reader can then judge the claim instead of trusting it.
//!
//! Three properties keep that honest:
//!
//! - **Nothing is inferred from absence.** A file with no marker is reported
//!   as unknown, never as C because nothing else matched.
//! - **Evidence is quoted, not paraphrased.** `rustc version 1.97.1` is what
//!   the file says; the reader sees that string.
//! - **Several languages can be true at once.** A Rust or Go program carries a
//!   C runtime, and a C++ one is linked against a C library. Findings are
//!   ordered with the strongest first rather than reduced to a single answer.

use crate::analysis::{Section, Symbol, details::BinaryDetails};

/// A language a binary shows signs of having been built from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceLanguage {
    C,
    Cpp,
    Rust,
    Go,
    Zig,
    Swift,
    ObjectiveC,
    DotNet,
    D,
    Fortran,
    Haskell,
    Pascal,
    Nim,
    OCaml,
    Ada,
    /// A Python program with an interpreter bundled around it, which is what
    /// a "compiled" Python file always is.
    Python,
    /// Written in assembly and put together by an assembler, with no compiler
    /// and no runtime in between.
    Assembly,
}

impl SourceLanguage {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Rust => "Rust",
            Self::Go => "Go",
            Self::Zig => "Zig",
            Self::Swift => "Swift",
            Self::ObjectiveC => "Objective-C",
            Self::DotNet => ".NET",
            Self::D => "D",
            Self::Fortran => "Fortran",
            Self::Haskell => "Haskell",
            Self::Pascal => "Pascal/Delphi",
            Self::Nim => "Nim",
            Self::OCaml => "OCaml",
            Self::Ada => "Ada",
            Self::Python => "Python",
            Self::Assembly => "Assembly",
        }
    }
}

/// How firmly the evidence points at the language.
///
/// Ordered weakest first, so the strongest statement about a file is the one
/// shown. The distinction matters: a C++ program and a C one are built by the
/// same compiler, and only the runtime it links tells them apart.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Confidence {
    /// Names the toolchain without settling the language: a C compiler builds
    /// C++ too, and links the runtime of most other languages.
    Possible,
    /// A runtime the language cannot do without, which a program in another
    /// language would only carry by calling into one.
    Likely,
    /// A marker only that toolchain emits, such as its own section, its
    /// version banner, or its name mangling.
    Certain,
}

/// One reason to believe a language was used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanguageEvidence {
    pub language: SourceLanguage,
    pub confidence: Confidence,
    /// What was found, in the file's own words where possible.
    pub evidence: String,
    /// The compiler and version, when the file states them.
    pub toolchain: Option<String>,
}

/// Most bytes scanned for producer strings.
///
/// Producer markers live in the metadata near the ends of a file, so both ends
/// are read rather than the whole image: a 150 MB binary would otherwise be
/// walked twice over for a handful of short strings.
const SCAN_WINDOW: usize = 4 * 1024 * 1024;

/// Everything the file says about how it was built, strongest evidence first.
#[must_use]
pub fn detect(
    file: &[u8],
    sections: &[Section],
    symbols: &[Symbol],
    details: &BinaryDetails,
) -> Vec<LanguageEvidence> {
    let mut found = Vec::new();

    from_sections(file, sections, &mut found);
    from_producer_strings(file, &mut found);
    from_symbols(symbols, &mut found);
    from_untyped_symbols(symbols, &mut found);
    from_runtime_names(symbols, &mut found);
    from_libraries(details, &mut found);
    from_imported_functions(details, &mut found);

    // The strongest statement about each language is the one worth showing.
    found.sort_by(|a, b| {
        b.confidence
            .cmp(&a.confidence)
            .then_with(|| a.language.label().cmp(b.language.label()))
    });
    found.dedup_by(|a, b| a.language == b.language);
    found
}

/// Sections a toolchain writes under its own name, which nothing else emits.
fn from_sections(file: &[u8], sections: &[Section], found: &mut Vec<LanguageEvidence>) {
    for section in sections {
        let language = match section.name.as_str() {
            ".gopclntab" | ".go.buildinfo" | ".note.go.buildid" | "__gopclntab" => {
                SourceLanguage::Go
            }
            ".gosymtab" | ".gostring" => SourceLanguage::Go,
            "__swift5_types" | "__swift5_proto" | "__swift5_fieldmd" | ".sw5tymd" => {
                SourceLanguage::Swift
            }
            // `rustc` writes its crate metadata under its own name. Only a
            // Rust compilation carries it, and it survives a strip that takes
            // the symbol table with it.
            ".rustc" | "__rustc" => SourceLanguage::Rust,
            "__objc_classlist" | "__objc_imageinfo" | ".objc_classlist" => {
                SourceLanguage::ObjectiveC
            }
            _ => continue,
        };
        found.push(LanguageEvidence {
            language,
            confidence: Confidence::Certain,
            evidence: format!("section {}", section.name),
            toolchain: None,
        });
    }

    // ELF records its producers in `.comment`, one NUL-separated string each.
    if let Some(comment) = sections.iter().find(|section| section.name == ".comment")
        && let Some(bytes) = comment.bytes_in(file)
    {
        for entry in bytes.split(|byte| *byte == 0) {
            let Ok(text) = std::str::from_utf8(entry) else {
                continue;
            };
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            if let Some(evidence) = producer(text) {
                found.push(evidence);
            }
        }
    }
}

/// Reads a producer string, such as a `.comment` entry or a version banner.
fn producer(text: &str) -> Option<LanguageEvidence> {
    let lower = text.to_lowercase();
    let (language, confidence) = if lower.starts_with("rustc ") {
        (SourceLanguage::Rust, Confidence::Certain)
    } else if lower.starts_with("zig ") {
        (SourceLanguage::Zig, Confidence::Certain)
    } else if lower.starts_with("free pascal") || lower.starts_with("fpc ") {
        (SourceLanguage::Pascal, Confidence::Certain)
    } else if lower.contains("gfortran") {
        (SourceLanguage::Fortran, Confidence::Certain)
    } else if lower.starts_with("ghc ") || lower.contains("glasgow haskell") {
        (SourceLanguage::Haskell, Confidence::Certain)
    } else if lower.starts_with("the netwide assembler") || lower.starts_with("nasm ") {
        // NASM writes its banner into `.comment`, which is a product naming
        // itself — the strongest kind of evidence this module has.
        (SourceLanguage::Assembly, Confidence::Certain)
    } else if lower.starts_with("yasm ") {
        (SourceLanguage::Assembly, Confidence::Certain)
    } else if lower.starts_with("gcc:") || lower.starts_with("clang version") {
        // A C compiler also builds C++, and links the runtime of Rust, Go and
        // the rest — so it names the toolchain without settling the language.
        (SourceLanguage::C, Confidence::Possible)
    } else {
        return None;
    };
    Some(LanguageEvidence {
        language,
        confidence,
        evidence: text.to_owned(),
        toolchain: Some(text.to_owned()),
    })
}

/// A file nothing typed, which is what an assembler leaves behind.
///
/// This module infers nothing from absence — that rule is what keeps it from
/// answering "C" whenever it recognises nothing — and this is not an
/// exception to it. `STT_NOTYPE` is a statement the symbol table makes about
/// every name it holds, and it is a statement no compiler makes: `gcc`,
/// `clang`, `rustc` and the rest emit `.type` for each function and object
/// they define, because the linker and the debugger need it. An assembler
/// emits it only where the author wrote the directive by hand.
///
/// So: names, all of them defined, none of them typed. A stripped binary has
/// no names and gets no finding, which is right — it says nothing about how it
/// was built. Reported as `Likely` rather than `Certain` because a hand-written
/// `.type` in one file of a program would take it away, and because a compiler
/// output stripped down to a couple of untyped labels could reach it.
fn from_untyped_symbols(symbols: &[Symbol], found: &mut Vec<LanguageEvidence>) {
    /// Below this a file says too little. Two labels are what a linker script
    /// or a partially stripped object can leave behind on their own.
    const ENOUGH: usize = 3;

    let defined: Vec<&Symbol> = symbols
        .iter()
        .filter(|symbol| !symbol.imported && symbol.address.is_some())
        .collect();
    if defined.len() < ENOUGH {
        return;
    }
    if defined
        .iter()
        .any(|symbol| symbol.kind != crate::analysis::SymbolKind::Untyped)
    {
        return;
    }
    found.push(LanguageEvidence {
        language: SourceLanguage::Assembly,
        confidence: Confidence::Likely,
        evidence: format!(
            "{} defined names, none of them typed: a compiler emits .type for every one",
            defined.len()
        ),
        toolchain: None,
    });
}

/// Markers left in the raw bytes, which is what a stripped binary still has.
fn from_producer_strings(file: &[u8], found: &mut Vec<LanguageEvidence>) {
    // A Rust binary bakes the compiler's own source path into its panic
    // messages, whatever the executable format, and it survives stripping.
    //
    // The banner is only quoted when a version number really follows it. A
    // scanner finds its own patterns when pointed at itself — Desdec analysing
    // its own binary read back the literal `rustc version ` from this very
    // function, and quoted whatever the linker had placed after it.
    if let Some(version) = scan_for(file, b"rustc version ").filter(followed_by_a_version) {
        found.push(LanguageEvidence {
            language: SourceLanguage::Rust,
            confidence: Confidence::Certain,
            evidence: version.clone(),
            toolchain: Some(version),
        });
    } else if scan_for(file, b"/rustc/").is_some() {
        found.push(LanguageEvidence {
            language: SourceLanguage::Rust,
            confidence: Confidence::Certain,
            evidence: "compiler source paths (/rustc/…)".to_owned(),
            toolchain: None,
        });
    }
    if let Some(version) = scan_for(file, b"go1.").filter(followed_by_a_version) {
        found.push(LanguageEvidence {
            language: SourceLanguage::Go,
            confidence: Confidence::Possible,
            evidence: version.clone(),
            toolchain: Some(version),
        });
    }
}

/// Whether a producer banner is followed by something shaped like a version.
///
/// Without this the banner alone is enough to quote whatever bytes follow it,
/// which is how a scanner ends up reporting its own search patterns.
fn followed_by_a_version(text: &String) -> bool {
    // A version number, wherever it sits: `1.97.1` after a space, or the
    // `1.22` glued straight onto `go1.`.
    text.as_bytes()
        .windows(3)
        .any(|window| window[0].is_ascii_digit() && window[1] == b'.' && window[2].is_ascii_digit())
}

/// Finds `needle` near either end of the file and returns the printable run it
/// starts, so the caller can quote what the file actually says.
fn scan_for(file: &[u8], needle: &[u8]) -> Option<String> {
    let head = file.len().min(SCAN_WINDOW);
    let tail = file.len().saturating_sub(SCAN_WINDOW).max(head);
    let windows = [(0, head), (tail, file.len())];

    for (start, end) in windows {
        let region = file.get(start..end)?;
        if let Some(at) = region
            .windows(needle.len())
            .position(|window| window == needle)
        {
            let text: Vec<u8> = region[at..]
                .iter()
                .take(96)
                .take_while(|byte| byte.is_ascii_graphic() || **byte == b' ')
                .copied()
                .collect();
            let text = String::from_utf8(text).ok()?;
            return Some(text.trim().to_owned());
        }
    }
    None
}

/// Name mangling, which encodes the language that produced the symbol.
fn from_symbols(symbols: &[Symbol], found: &mut Vec<LanguageEvidence>) {
    let mut rust = 0_usize;
    let mut cpp = 0_usize;
    let mut swift = 0_usize;
    let mut d = 0_usize;

    for symbol in symbols {
        let name = symbol.name.as_str();
        if name.starts_with("_ZN") || name.starts_with("_Z") {
            // Rust's legacy scheme is C++ mangling with a hash appended, so
            // the hash is what tells them apart.
            if has_rust_hash(name) {
                rust += 1;
            } else {
                cpp += 1;
            }
        } else if name.starts_with("_R") && name.len() > 4 {
            rust += 1;
        } else if name.starts_with("$s") || name.starts_with("_$s") {
            swift += 1;
        } else if name.starts_with("_D") && name.ends_with("Zv") {
            d += 1;
        }
    }

    let mangled = [
        (rust, SourceLanguage::Rust, "Rust name mangling"),
        (cpp, SourceLanguage::Cpp, "C++ name mangling"),
        (swift, SourceLanguage::Swift, "Swift name mangling"),
        (d, SourceLanguage::D, "D name mangling"),
    ];
    for (count, language, description) in mangled {
        if count > 0 {
            found.push(LanguageEvidence {
                language,
                confidence: Confidence::Certain,
                evidence: format!("{count} symbols with {description}"),
                toolchain: None,
            });
        }
    }
}

/// A name the runtime declares, the language it belongs to, and how firmly
/// it points there.
///
/// Exact names are matched whole; prefixes match a family of names — every
/// OCaml symbol starts `caml`, and listing them is not possible.
struct Runtime {
    name: &'static str,
    prefix: bool,
    language: SourceLanguage,
    confidence: Confidence,
}

const RUNTIMES: &[Runtime] = &[
    // Go names its runtime in the symbol table even when nothing else
    // survives, and no other language writes these.
    Runtime {
        name: "runtime.goexit",
        prefix: false,
        language: SourceLanguage::Go,
        confidence: Confidence::Certain,
    },
    Runtime {
        name: "runtime.morestack",
        prefix: false,
        language: SourceLanguage::Go,
        confidence: Confidence::Certain,
    },
    Runtime {
        name: "go:buildid",
        prefix: false,
        language: SourceLanguage::Go,
        confidence: Confidence::Certain,
    },
    // Rust's own runtime hooks, which the compiler emits and no source
    // file writes by hand.
    Runtime {
        name: "rust_eh_personality",
        prefix: false,
        language: SourceLanguage::Rust,
        confidence: Confidence::Certain,
    },
    Runtime {
        name: "__rust_alloc",
        prefix: true,
        language: SourceLanguage::Rust,
        confidence: Confidence::Certain,
    },
    Runtime {
        name: "rust_begin_unwind",
        prefix: false,
        language: SourceLanguage::Rust,
        confidence: Confidence::Certain,
    },
    // The C++ personality routine: the unwinder the Itanium ABI installs.
    Runtime {
        name: "__gxx_personality_v0",
        prefix: false,
        language: SourceLanguage::Cpp,
        confidence: Confidence::Likely,
    },
    Runtime {
        name: "_ZSt9terminatev",
        prefix: false,
        language: SourceLanguage::Cpp,
        confidence: Confidence::Likely,
    },
    // Zig's stack probe and panic handler.
    Runtime {
        name: "__zig_probe_stack",
        prefix: false,
        language: SourceLanguage::Zig,
        confidence: Confidence::Certain,
    },
    Runtime {
        name: "zig_panic",
        prefix: false,
        language: SourceLanguage::Zig,
        confidence: Confidence::Likely,
    },
    // Swift's runtime, called by every Swift program and by nothing else.
    Runtime {
        name: "swift_allocObject",
        prefix: false,
        language: SourceLanguage::Swift,
        confidence: Confidence::Certain,
    },
    Runtime {
        name: "swift_retain",
        prefix: false,
        language: SourceLanguage::Swift,
        confidence: Confidence::Certain,
    },
    // Objective-C's message send: the whole language goes through it.
    Runtime {
        name: "objc_msgSend",
        prefix: false,
        language: SourceLanguage::ObjectiveC,
        confidence: Confidence::Certain,
    },
    // Nim generates these around the program it compiles.
    Runtime {
        name: "NimMain",
        prefix: false,
        language: SourceLanguage::Nim,
        confidence: Confidence::Certain,
    },
    Runtime {
        name: "nimFrame",
        prefix: false,
        language: SourceLanguage::Nim,
        confidence: Confidence::Certain,
    },
    Runtime {
        name: "nimGCvisit",
        prefix: false,
        language: SourceLanguage::Nim,
        confidence: Confidence::Certain,
    },
    // Every OCaml symbol carries the runtime's prefix.
    Runtime {
        name: "caml_main",
        prefix: false,
        language: SourceLanguage::OCaml,
        confidence: Confidence::Certain,
    },
    Runtime {
        name: "caml_garbage_collection",
        prefix: false,
        language: SourceLanguage::OCaml,
        confidence: Confidence::Certain,
    },
    // GNAT's runtime, which every Ada binary it builds carries.
    Runtime {
        name: "__gnat_",
        prefix: true,
        language: SourceLanguage::Ada,
        confidence: Confidence::Certain,
    },
    Runtime {
        name: "ada__",
        prefix: true,
        language: SourceLanguage::Ada,
        confidence: Confidence::Likely,
    },
    // GHC's runtime system.
    Runtime {
        name: "hs_init",
        prefix: false,
        language: SourceLanguage::Haskell,
        confidence: Confidence::Likely,
    },
    Runtime {
        name: "stg_returnToStackTop",
        prefix: false,
        language: SourceLanguage::Haskell,
        confidence: Confidence::Certain,
    },
    // Fortran's runtime library.
    Runtime {
        name: "_gfortran_",
        prefix: true,
        language: SourceLanguage::Fortran,
        confidence: Confidence::Certain,
    },
    // Free Pascal's runtime, which spells its unit initialisers this way.
    Runtime {
        name: "SYSTEM_$$_",
        prefix: true,
        language: SourceLanguage::Pascal,
        confidence: Confidence::Certain,
    },
    Runtime {
        name: "fpc_initializeunits",
        prefix: false,
        language: SourceLanguage::Pascal,
        confidence: Confidence::Certain,
    },
    // A bundled CPython. The program is Python; the file is its interpreter.
    Runtime {
        name: "Py_Initialize",
        prefix: false,
        language: SourceLanguage::Python,
        confidence: Confidence::Likely,
    },
    Runtime {
        name: "PyRun_SimpleString",
        prefix: false,
        language: SourceLanguage::Python,
        confidence: Confidence::Likely,
    },
    // glibc's entry into `main`. It names the C runtime and settles
    // nothing — every language above links it — but a file that has this
    // and no other marker is a C program more often than not.
    Runtime {
        name: "__libc_start_main",
        prefix: false,
        language: SourceLanguage::C,
        confidence: Confidence::Possible,
    },
    Runtime {
        name: "__isoc99_",
        prefix: true,
        language: SourceLanguage::C,
        confidence: Confidence::Possible,
    },
];

/// Runtime entry points a language's own runtime declares.
///
/// Mangling says which compiler wrote a name; this says which runtime the
/// program carries. It is the reading that still works when the mangling is
/// gone — a Go binary mangles nothing, a Nim one emits plain C names — and the
/// one that reaches languages whose compilers leave no producer banner.
///
/// Every name here belongs to one runtime and is written by its compiler, not
/// by the programmer: `__libc_start_main` is glibc's own entry into `main`,
/// `NimMain` is what the Nim compiler generates around a program's body.
fn from_runtime_names(symbols: &[Symbol], found: &mut Vec<LanguageEvidence>) {
    for runtime in RUNTIMES {
        let Some(symbol) = symbols.iter().find(|symbol| {
            if runtime.prefix {
                symbol.name.starts_with(runtime.name)
            } else {
                symbol.name == runtime.name
            }
        }) else {
            continue;
        };
        found.push(LanguageEvidence {
            language: runtime.language,
            confidence: runtime.confidence,
            evidence: format!("runtime symbol {}", symbol.name),
            toolchain: None,
        });
    }
}

/// What a Windows binary asks other libraries for, function by function.
///
/// PE records the names, not just the libraries, which is the difference
/// between "links a C runtime" and "calls `_Py_Initialize`". Nothing here is
/// read from ELF or Mach-O, whose import tables name libraries alone —
/// [`from_libraries`] is what covers those.
fn from_imported_functions(details: &BinaryDetails, found: &mut Vec<LanguageEvidence>) {
    for library in &details.imports {
        for function in &library.functions {
            let (language, confidence) = if function.starts_with("Py_")
                || function.starts_with("PyEval_")
            {
                (SourceLanguage::Python, Confidence::Certain)
            } else if function.starts_with("_CorExeMain") || function.starts_with("_CorDllMain") {
                (SourceLanguage::DotNet, Confidence::Certain)
            } else if function.starts_with("swift_") {
                (SourceLanguage::Swift, Confidence::Certain)
            } else if function.starts_with("objc_") {
                (SourceLanguage::ObjectiveC, Confidence::Certain)
            } else {
                continue;
            };
            found.push(LanguageEvidence {
                language,
                confidence,
                evidence: format!("imports {} from {}", function, library.library),
                toolchain: None,
            });
        }
    }
}

/// Rust's legacy mangling ends a name with `17h` and sixteen hex digits.
fn has_rust_hash(name: &str) -> bool {
    let Some(at) = name.rfind("17h") else {
        return false;
    };
    let tail = &name[at + 3..];
    let digits = tail.strip_suffix('E').unwrap_or(tail);
    digits.len() == 16 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// A runtime library a language cannot do without.
fn from_libraries(details: &BinaryDetails, found: &mut Vec<LanguageEvidence>) {
    for library in &details.linked_libraries {
        let name = library.to_lowercase();
        // `msvcp` is the Microsoft C++ runtime; `msvcr` next to it is the C
        // one, and naming only the first keeps C++ from being read into a
        // plain C program.
        let language =
            if name.contains("libstdc++") || name.contains("libc++") || name.contains("msvcp") {
                SourceLanguage::Cpp
            } else if name.contains("swiftcore") {
                SourceLanguage::Swift
            } else if name.contains("libobjc") {
                SourceLanguage::ObjectiveC
            } else if name.contains("libgfortran") {
                SourceLanguage::Fortran
            } else if name.contains("mscoree") {
                SourceLanguage::DotNet
            } else {
                continue;
            };
        found.push(LanguageEvidence {
            language,
            // Strong, but not settled: a program in another language links
            // this too when it calls into one.
            confidence: Confidence::Likely,
            evidence: format!("links {library}"),
            toolchain: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Permissions;

    fn symbol(name: &str) -> Symbol {
        Symbol {
            name: name.to_owned(),
            address: Some(0x1000),
            size: 0,
            imported: false,
            ..Symbol::default()
        }
    }

    /// A binary an assembler produced answers "Assembly", where it used to
    /// answer nothing at all.
    ///
    /// Reported from what the symbol table *states* — `STT_NOTYPE` on every
    /// defined name — and not from the absence of a compiler marker, which is
    /// the rule this module is built on. Checked against a real one: `nasm -f
    /// elf64` and `ld` produce a file whose five defined names are all
    /// untyped, and Desdec's own binary, whose names are typed, gets no such
    /// finding.
    #[test]
    fn a_file_nothing_typed_is_reported_as_assembly() {
        let untyped = |name: &str| Symbol {
            kind: crate::analysis::SymbolKind::Untyped,
            ..symbol(name)
        };
        let hand_written = [
            untyped("_start"),
            untyped("msg"),
            untyped("msglen"),
            untyped("__bss_start"),
        ];
        let mut found = Vec::new();
        from_untyped_symbols(&hand_written, &mut found);
        assert_eq!(
            found
                .iter()
                .map(|evidence| (evidence.language, evidence.confidence))
                .collect::<Vec<_>>(),
            vec![(SourceLanguage::Assembly, Confidence::Likely)]
        );

        // One name a compiler typed is enough to say a compiler was there.
        let mut compiled = hand_written.clone();
        compiled[1].kind = crate::analysis::SymbolKind::Function;
        let mut found = Vec::new();
        from_untyped_symbols(&compiled, &mut found);
        assert!(found.is_empty(), "{found:?}");

        // And a file with almost no names says nothing about how it was
        // built — a stripped binary must not come out as hand-written.
        let mut found = Vec::new();
        from_untyped_symbols(&hand_written[..2], &mut found);
        assert!(found.is_empty(), "{found:?}");
    }

    fn named_section(name: &str) -> Section {
        Section {
            name: name.to_owned(),
            virtual_address: 0x1000,
            file_offset: 0,
            virtual_size: 0x10,
            file_size: 0x10,
            permissions: Permissions {
                read: true,
                ..Permissions::default()
            },
            entropy: None,
        }
    }

    fn detect_symbols(names: &[&str]) -> Vec<LanguageEvidence> {
        let symbols: Vec<Symbol> = names.iter().map(|name| symbol(name)).collect();
        detect(&[], &[], &symbols, &BinaryDetails::default())
    }

    /// Rust's legacy mangling is C++ mangling with a hash appended, so the two
    /// are only told apart by that hash.
    #[test]
    fn rust_mangling_is_not_taken_for_cpp() {
        let found = detect_symbols(&["_ZN4core3fmt5write17h0123456789abcdefE"]);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].language, SourceLanguage::Rust);
    }

    #[test]
    fn cpp_mangling_is_not_taken_for_rust() {
        let found = detect_symbols(&["_ZNSt6vectorIiSaIiEE9push_backERKi"]);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].language, SourceLanguage::Cpp);
    }

    #[test]
    fn a_hash_of_the_wrong_shape_is_not_a_rust_symbol() {
        // Too short, and not hexadecimal: neither is Rust's scheme.
        assert!(!has_rust_hash("_ZN4test17hdeadbeefE"));
        assert!(!has_rust_hash("_ZN4test17hzzzzzzzzzzzzzzzzE"));
        assert!(has_rust_hash("_ZN4test17h0123456789abcdefE"));
    }

    #[test]
    fn a_go_section_settles_the_language() {
        let found = detect(
            &[],
            &[named_section(".gopclntab")],
            &[],
            &BinaryDetails::default(),
        );

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].language, SourceLanguage::Go);
        assert_eq!(found[0].confidence, Confidence::Certain);
    }

    /// A C++ program links a C++ runtime, but so does a C program that calls
    /// into one, so this points without settling.
    #[test]
    fn a_cpp_runtime_is_a_lead_rather_than_a_verdict() {
        let details = BinaryDetails {
            linked_libraries: vec!["libstdc++.so.6".to_owned()],
            ..BinaryDetails::default()
        };
        let found = detect(&[], &[], &[], &details);

        assert_eq!(found[0].language, SourceLanguage::Cpp);
        assert_eq!(found[0].confidence, Confidence::Likely);
    }

    /// The compiler's own version is worth quoting: it is the file's word,
    /// not an inference.
    #[test]
    fn a_producer_string_reports_the_toolchain() {
        let evidence = producer("rustc version 1.97.1 (8bab26f4f 2026-07-14)").expect("rust");

        assert_eq!(evidence.language, SourceLanguage::Rust);
        assert_eq!(evidence.confidence, Confidence::Certain);
        assert!(evidence.toolchain.is_some_and(|t| t.contains("1.97.1")));
    }

    /// A C compiler builds C++ and links every other language's runtime, so it
    /// names the toolchain without settling the source language.
    #[test]
    fn a_c_compiler_banner_does_not_settle_the_language() {
        let evidence = producer("GCC: (GNU) 16.1.1 20260515").expect("gcc");

        assert_eq!(evidence.language, SourceLanguage::C);
        assert_eq!(evidence.confidence, Confidence::Possible);
    }

    /// Nothing found must stay nothing: reporting C for a file with no marker
    /// would be a guess dressed as a finding.
    #[test]
    fn a_file_with_no_marker_is_reported_as_unknown() {
        assert!(detect(&[], &[], &[], &BinaryDetails::default()).is_empty());
        assert!(detect(&[0xff; 4096], &[], &[], &BinaryDetails::default()).is_empty());
    }

    /// One language is reported once, keeping its strongest evidence.
    #[test]
    fn repeated_evidence_for_one_language_collapses_to_its_strongest() {
        let details = BinaryDetails {
            linked_libraries: vec!["libstdc++.so.6".to_owned()],
            ..BinaryDetails::default()
        };
        let symbols = [symbol("_ZNSt6vectorIiSaIiEE9push_backERKi")];
        let found = detect(&[], &[], &symbols, &details);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].language, SourceLanguage::Cpp);
        assert_eq!(found[0].confidence, Confidence::Certain);
    }

    /// A banner with nothing version-shaped after it is not evidence.
    ///
    /// This is how Desdec read its own search patterns back out of its own
    /// binary: the literal `rustc version ` sits in this module, and the
    /// linker placed unrelated text after it.
    #[test]
    fn a_banner_without_a_version_after_it_is_not_quoted() {
        assert!(!followed_by_a_version(
            &"rustc version /rustc/compiler source paths (/rustc/".to_owned()
        ));
        assert!(followed_by_a_version(
            &"rustc version 1.97.1 (8bab26f4f 2026-07-14)".to_owned()
        ));
        assert!(followed_by_a_version(&"go1.22.3".to_owned()));
    }

    /// Analysing this very binary must not quote the patterns this module
    /// searches for. A scanner pointed at itself finds itself.
    #[test]
    fn the_scanner_does_not_report_its_own_patterns() {
        let path = std::env::current_exe().expect("the test binary has a path");
        let analysis = crate::analyse_path(&path).expect("analysable");

        for found in &analysis.languages {
            let Some(toolchain) = &found.toolchain else {
                continue;
            };
            assert!(
                followed_by_a_version(toolchain),
                "quoted a banner with no version in it: {toolchain}"
            );
        }
    }

    /// The end-to-end check, on a file whose language is known for certain:
    /// the test binary is this Rust program. It holds on every platform the
    /// tests run on, since the marker survives in ELF, PE and Mach-O alike.
    #[test]
    fn the_test_binary_is_recognised_as_rust() {
        let path = std::env::current_exe().expect("the test binary has a path");
        let analysis = crate::analyse_path(&path).expect("the test binary is analysable");

        let rust = analysis
            .languages
            .iter()
            .find(|found| found.language == SourceLanguage::Rust)
            .expect("a Rust binary must be recognised as Rust");
        assert_eq!(rust.confidence, Confidence::Certain);
        // Strongest evidence leads, so Rust is what the reader sees first.
        assert_eq!(analysis.languages[0].language, SourceLanguage::Rust);
    }

    /// A runtime's own entry points name the language when the mangling is
    /// gone: Go mangles nothing at all, and a stripped Nim binary carries
    /// plain C names.
    #[test]
    fn a_runtime_entry_point_names_the_language() {
        for (name, language) in [
            ("runtime.goexit", SourceLanguage::Go),
            ("NimMain", SourceLanguage::Nim),
            ("caml_main", SourceLanguage::OCaml),
            ("swift_allocObject", SourceLanguage::Swift),
            ("__zig_probe_stack", SourceLanguage::Zig),
            ("rust_eh_personality", SourceLanguage::Rust),
        ] {
            let found = detect_symbols(&[name]);
            assert_eq!(
                found.first().map(|evidence| evidence.language),
                Some(language),
                "{name}"
            );
            assert_eq!(found[0].confidence, Confidence::Certain, "{name}");
        }
    }

    /// A prefix stands for the family of names a runtime writes: every GNAT
    /// symbol starts `__gnat_`, and listing them is not possible.
    #[test]
    fn a_runtime_prefix_matches_the_whole_family() {
        let found = detect_symbols(&["__gnat_rcheck_CE_Overflow_Check"]);

        assert_eq!(found[0].language, SourceLanguage::Ada);
        assert!(found[0].evidence.contains("__gnat_rcheck"), "{found:?}");
    }

    /// The C runtime's entry into `main` is in every language's binary, so it
    /// points at C without settling anything.
    #[test]
    fn the_c_runtime_entry_point_does_not_settle_the_language() {
        let found = detect_symbols(&["__libc_start_main"]);

        assert_eq!(found[0].language, SourceLanguage::C);
        assert_eq!(found[0].confidence, Confidence::Possible);
    }

    /// PE names the functions it imports, which says far more than the library
    /// they came from: `Py_Initialize` is a bundled interpreter, and
    /// `python311.dll` alone could have been anything.
    #[test]
    fn an_imported_function_names_the_language() {
        let details = BinaryDetails {
            imports: vec![crate::analysis::ImportedLibrary {
                library: "python311.dll".to_owned(),
                functions: vec!["Py_Initialize".to_owned()],
                truncated: false,
            }],
            ..BinaryDetails::default()
        };
        let found = detect(&[], &[], &[], &details);

        assert_eq!(found[0].language, SourceLanguage::Python);
        assert_eq!(found[0].confidence, Confidence::Certain);
        assert!(found[0].evidence.contains("python311.dll"), "{found:?}");
    }

    /// `.rustc` is the crate metadata `rustc` writes under its own name, and
    /// it outlives the strip that takes the symbol table away.
    #[test]
    fn the_rust_metadata_section_survives_a_strip() {
        let found = detect(
            &[],
            &[named_section(".rustc")],
            &[],
            &BinaryDetails::default(),
        );

        assert_eq!(found[0].language, SourceLanguage::Rust);
        assert_eq!(found[0].confidence, Confidence::Certain);
    }

    #[test]
    fn scanning_never_panics_on_any_input() {
        for file in [Vec::new(), vec![0_u8; 1], vec![0xff_u8; 8192]] {
            let _ = detect(&file, &[], &[], &BinaryDetails::default());
        }
    }
}
