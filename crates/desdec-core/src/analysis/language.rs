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
    from_libraries(details, &mut found);

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
            "__swift5_types" | "__swift5_proto" | ".sw5tymd" => SourceLanguage::Swift,
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

    #[test]
    fn scanning_never_panics_on_any_input() {
        for file in [Vec::new(), vec![0_u8; 1], vec![0xff_u8; 8192]] {
            let _ = detect(&file, &[], &[], &BinaryDetails::default());
        }
    }
}
