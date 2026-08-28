//! Whether a file has been packed, protected or obfuscated, and by what.
//!
//! A packer rewrites a program into a stub plus a compressed blob: what the
//! disassembly shows is then the stub, not the program, and every other view
//! in Desdec is reading the wrapper rather than the code the reader came for.
//! That is worth saying loudly and early, which is why this is its own reading
//! rather than a line in the entropy gauge.
//!
//! It follows the same three rules as [`super::language`]:
//!
//! - **Nothing is inferred from absence.** A file no rule matches is reported
//!   as carrying no known protection, never as "clean".
//! - **Evidence is quoted.** `section UPX1` is what the file says; the reader
//!   sees that and can check it in the section table.
//! - **A lead is not a verdict.** A named product found by its own marker is
//!   [`Confidence::Certain`]; a writable code section is a shape many honest
//!   programs also have, and is reported as [`Confidence::Possible`] with the
//!   product left unnamed.
//!
//! # Why the table is masked
//!
//! A scanner pointed at itself finds itself. Every marker below would sit in
//! Desdec's own binary as a plain string, and Desdec analysing its own
//! executable — which its test suite does on every run — would read them back
//! and report itself as packed by all of them at once. The same trap already
//! caught [`super::language`], which quoted its own `rustc version ` pattern.
//!
//! So the table is stored masked: each marker is `XOR`ed with [`MASK`] at
//! compile time, and the comparison unmasks one byte at a time. The plain
//! spelling exists in the source, never in the compiled image.

use super::{Section, Symbol, details::BinaryDetails, entropy, language::Confidence};

/// What a marker says the file was put through.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectionKind {
    /// Compresses the program and unpacks it at run time.
    Packer,
    /// Defends the program against being read or altered: anti-debug,
    /// licensing, integrity checks.
    Protector,
    /// Rewrites the code into a form that decodes to something else, usually
    /// a virtual machine of the product's own.
    Virtualiser,
    /// Scrambles names, control flow or strings, leaving ordinary code.
    Obfuscator,
    /// Carries a payload it writes out and runs: an installer, a self-
    /// extracting archive, a bundled interpreter.
    Bundler,
    /// The shape of a protected file, with nothing naming the product.
    Unidentified,
}

impl ProtectionKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Packer => "packer",
            Self::Protector => "protector",
            Self::Virtualiser => "virtualiser",
            Self::Obfuscator => "obfuscator",
            Self::Bundler => "bundler",
            Self::Unidentified => "unidentified",
        }
    }
}

/// One reason to believe the file was protected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Protection {
    /// The product, when a marker names one. Empty for a structural lead,
    /// which says the shape without naming a culprit.
    pub name: String,
    pub kind: ProtectionKind,
    pub confidence: Confidence,
    /// What was found, in the file's own words where possible.
    pub evidence: String,
}

impl Protection {
    /// Whether this finding names a product rather than describing a shape.
    #[must_use]
    pub fn names_a_product(&self) -> bool {
        !self.name.is_empty()
    }
}

/// The mask the table is stored under. Any non-zero byte would do; this one
/// keeps the masked bytes out of the printable range, so a hex dump of Desdec
/// does not show the table as text either.
const MASK: u8 = 0x5A;

/// Masks a marker at compile time, so the plain spelling never reaches the
/// compiled image.
const fn masked<const N: usize>(plain: &[u8; N]) -> [u8; N] {
    let mut out = [0_u8; N];
    let mut index = 0;
    while index < N {
        out[index] = plain[index] ^ MASK;
        index += 1;
    }
    out
}

/// Reads a masked marker back, for the moment it has to be shown.
fn unmasked(marker: &[u8]) -> String {
    String::from_utf8_lossy(&marker.iter().map(|byte| byte ^ MASK).collect::<Vec<u8>>())
        .into_owned()
}

/// Whether this masked marker stands at `at`, unmasking as it compares.
fn starts_at(haystack: &[u8], at: usize, marker: &[u8]) -> bool {
    !marker.is_empty()
        && haystack
            .get(at..at + marker.len())
            .is_some_and(|window| window.iter().zip(marker).all(|(byte, m)| *byte == m ^ MASK))
}

/// Whether `haystack` holds this masked marker anywhere. Only the tests ask
/// this: the scan itself walks the file once and asks about every marker at
/// each position.
#[cfg(test)]
fn holds(haystack: &[u8], marker: &[u8]) -> bool {
    (0..haystack.len()).any(|at| starts_at(haystack, at, marker))
}

/// Whether a name is exactly this masked marker, ignoring case as PE section
/// names do.
fn is(name: &str, marker: &[u8]) -> bool {
    name.len() == marker.len()
        && name
            .bytes()
            .zip(marker)
            .all(|(byte, m)| byte.eq_ignore_ascii_case(&(m ^ MASK)))
}

/// Whether a name starts with this masked marker, for the products that number
/// their sections — `UPX0`, `UPX1`, `.vmp0`, `.vmp1`.
fn starts_with(name: &str, marker: &[u8]) -> bool {
    name.len() >= marker.len()
        && name
            .bytes()
            .zip(marker)
            .all(|(byte, m)| byte.eq_ignore_ascii_case(&(m ^ MASK)))
}

/// A section name only one product writes.
struct SectionMarker {
    /// The product, masked.
    name: &'static [u8],
    kind: ProtectionKind,
    /// The section name, masked. Matched as a prefix when the product numbers
    /// its sections, and whole otherwise.
    section: &'static [u8],
    prefix: bool,
}

/// Section names that belong to one product and nothing else.
///
/// Every entry here is a name no compiler emits: a linker writes `.text`,
/// `.data` and `.rodata`, and a file carrying `UPX1` carries it because UPX
/// put it there.
static SECTION_MARKERS: &[SectionMarker] = &[
    SectionMarker {
        name: &masked(b"UPX"),
        kind: ProtectionKind::Packer,
        section: &masked(b"UPX"),
        prefix: true,
    },
    SectionMarker {
        name: &masked(b"ASPack"),
        kind: ProtectionKind::Packer,
        section: &masked(b".aspack"),
        prefix: false,
    },
    SectionMarker {
        name: &masked(b"ASPack"),
        kind: ProtectionKind::Packer,
        section: &masked(b".adata"),
        prefix: false,
    },
    SectionMarker {
        name: &masked(b"VMProtect"),
        kind: ProtectionKind::Virtualiser,
        section: &masked(b".vmp"),
        prefix: true,
    },
    SectionMarker {
        name: &masked(b"Themida / WinLicense"),
        kind: ProtectionKind::Virtualiser,
        section: &masked(b".themida"),
        prefix: false,
    },
    SectionMarker {
        name: &masked(b"Themida / WinLicense"),
        kind: ProtectionKind::Virtualiser,
        section: &masked(b".winlice"),
        prefix: false,
    },
    SectionMarker {
        name: &masked(b"Enigma Protector"),
        kind: ProtectionKind::Protector,
        section: &masked(b".enigma"),
        prefix: true,
    },
    SectionMarker {
        name: &masked(b"PEtite"),
        kind: ProtectionKind::Packer,
        section: &masked(b".petite"),
        prefix: false,
    },
    SectionMarker {
        name: &masked(b"MPRESS"),
        kind: ProtectionKind::Packer,
        section: &masked(b".MPRESS"),
        prefix: true,
    },
    SectionMarker {
        name: &masked(b"NsPack"),
        kind: ProtectionKind::Packer,
        section: &masked(b".nsp"),
        prefix: true,
    },
    SectionMarker {
        name: &masked(b"PECompact"),
        kind: ProtectionKind::Packer,
        section: &masked(b"PEC2"),
        prefix: true,
    },
    SectionMarker {
        name: &masked(b"PESpin"),
        kind: ProtectionKind::Protector,
        section: &masked(b".taz"),
        prefix: false,
    },
    SectionMarker {
        name: &masked(b"Y0da Crypter"),
        kind: ProtectionKind::Packer,
        section: &masked(b".yP"),
        prefix: false,
    },
    SectionMarker {
        name: &masked(b"Upack"),
        kind: ProtectionKind::Packer,
        section: &masked(b".ByDwing"),
        prefix: false,
    },
    SectionMarker {
        name: &masked(b"WWPack32"),
        kind: ProtectionKind::Packer,
        section: &masked(b".WWP32"),
        prefix: false,
    },
    SectionMarker {
        name: &masked(b"SVK Protector"),
        kind: ProtectionKind::Protector,
        section: &masked(b".svkp"),
        prefix: false,
    },
    SectionMarker {
        name: &masked(b"Shrinker"),
        kind: ProtectionKind::Packer,
        section: &masked(b".shrink"),
        prefix: true,
    },
    SectionMarker {
        name: &masked(b"TSULoader"),
        kind: ProtectionKind::Packer,
        section: &masked(b".tsuarch"),
        prefix: false,
    },
    SectionMarker {
        name: &masked(b"NeoLite"),
        kind: ProtectionKind::Packer,
        section: &masked(b".neolit"),
        prefix: true,
    },
    SectionMarker {
        name: &masked(b"Perplex PE Protector"),
        kind: ProtectionKind::Protector,
        section: &masked(b".perplex"),
        prefix: false,
    },
    SectionMarker {
        name: &masked(b"StarForce"),
        kind: ProtectionKind::Protector,
        section: &masked(b".sforce"),
        prefix: true,
    },
    SectionMarker {
        name: &masked(b"Obsidium"),
        kind: ProtectionKind::Protector,
        section: &masked(b".obsidium"),
        prefix: false,
    },
];

/// A run of bytes only one product writes into a file.
struct StringMarker {
    name: &'static [u8],
    kind: ProtectionKind,
    needle: &'static [u8],
    confidence: Confidence,
}

/// Markers found in the raw bytes, which is what a stripped file still has.
///
/// Kept to strings a product writes about itself — a banner, a copyright line,
/// a magic word its own stub reads back. A word that merely happens to appear
/// in protected files is not here: it would flag every program that mentions
/// it in a message of its own.
static STRING_MARKERS: &[StringMarker] = &[
    StringMarker {
        name: &masked(b"UPX"),
        kind: ProtectionKind::Packer,
        // The stub's own magic, which UPX reads back to find its blob.
        needle: &masked(b"UPX!"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"UPX"),
        kind: ProtectionKind::Packer,
        needle: &masked(b"This file is packed with the UPX"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"VMProtect"),
        kind: ProtectionKind::Virtualiser,
        needle: &masked(b"VMProtect"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"Themida / WinLicense"),
        kind: ProtectionKind::Virtualiser,
        needle: &masked(b"WinLicense"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"ASProtect"),
        kind: ProtectionKind::Protector,
        needle: &masked(b"ASProtect"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"Armadillo"),
        kind: ProtectionKind::Protector,
        needle: &masked(b"ARMADILLO"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"Obsidium"),
        kind: ProtectionKind::Protector,
        needle: &masked(b"Obsidium"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"MoleBox"),
        kind: ProtectionKind::Packer,
        needle: &masked(b"MoleBox"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"ConfuserEx"),
        kind: ProtectionKind::Obfuscator,
        needle: &masked(b"ConfuserEx"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b".NET Reactor"),
        kind: ProtectionKind::Obfuscator,
        needle: &masked(b".NET Reactor"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"Dotfuscator"),
        kind: ProtectionKind::Obfuscator,
        needle: &masked(b"DotfuscatorAttribute"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"Denuvo"),
        kind: ProtectionKind::Protector,
        needle: &masked(b"Denuvo"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"Sentinel HASP"),
        kind: ProtectionKind::Protector,
        needle: &masked(b"hasp_login"),
        confidence: Confidence::Likely,
    },
    StringMarker {
        name: &masked(b"CodeMeter"),
        kind: ProtectionKind::Protector,
        needle: &masked(b"CodeMeter"),
        confidence: Confidence::Likely,
    },
    StringMarker {
        name: &masked(b"kkrunchy"),
        kind: ProtectionKind::Packer,
        needle: &masked(b"kkrunchy"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"PyInstaller"),
        kind: ProtectionKind::Bundler,
        // The bundle's own table of contents marker.
        needle: &masked(b"MEI\x0c\x0b\x0a\x0b\x0e"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"PyInstaller"),
        kind: ProtectionKind::Bundler,
        needle: &masked(b"pyi-runtime-tmpdir"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"Nuitka"),
        kind: ProtectionKind::Bundler,
        needle: &masked(b"__nuitka_"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"Nullsoft Install System"),
        kind: ProtectionKind::Bundler,
        needle: &masked(b"NullsoftInst"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"Inno Setup"),
        kind: ProtectionKind::Bundler,
        needle: &masked(b"Inno Setup Setup Data"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"WinRAR self-extracting archive"),
        kind: ProtectionKind::Bundler,
        needle: &masked(b"WinRAR self-extracting archive"),
        confidence: Confidence::Certain,
    },
    StringMarker {
        name: &masked(b"7-Zip self-extracting archive"),
        kind: ProtectionKind::Bundler,
        needle: &masked(b"7-Zip Self-Extracting"),
        confidence: Confidence::Certain,
    },
];

/// A run-time name that only a protected program asks the system for.
struct ImportMarker {
    name: &'static [u8],
    reason: &'static [u8],
}

/// Names whose presence says the program watches for a debugger.
///
/// Not a protection in itself — a legitimate program may check — so this is
/// reported as a lead, never as a product.
static ANTI_DEBUG_IMPORTS: &[ImportMarker] = &[
    ImportMarker {
        name: &masked(b"IsDebuggerPresent"),
        reason: &masked(b"asks Windows whether it is being debugged"),
    },
    ImportMarker {
        name: &masked(b"CheckRemoteDebuggerPresent"),
        reason: &masked(b"asks Windows whether it is being debugged"),
    },
    ImportMarker {
        name: &masked(b"NtQueryInformationProcess"),
        reason: &masked(b"reads process information a debugger check uses"),
    },
    ImportMarker {
        name: &masked(b"NtSetInformationThread"),
        reason: &masked(b"can detach a thread from a debugger"),
    },
    ImportMarker {
        name: &masked(b"OutputDebugStringA"),
        reason: &masked(b"is used as a debugger-presence probe"),
    },
];

/// Most bytes scanned for markers, at each end of the file.
///
/// A stub writes its banner near the beginning and its blob near the end, and
/// a 150 MB image should not be walked whole for a handful of short strings.
const SCAN_WINDOW: usize = 4 * 1024 * 1024;

/// Everything the file says about having been protected, strongest first.
#[must_use]
pub fn detect(
    file: &[u8],
    sections: &[Section],
    symbols: &[Symbol],
    details: &BinaryDetails,
    entry_point: Option<u64>,
) -> Vec<Protection> {
    let mut found = Vec::new();

    from_sections(sections, &mut found);
    from_bytes(file, &mut found);
    from_imports(details, &mut found);
    from_shape(sections, symbols, details, entry_point, &mut found);

    // Strongest first, then by name, so the reader's eye lands on the firmest
    // statement about the file.
    found.sort_by(|a, b| {
        b.confidence
            .cmp(&a.confidence)
            .then_with(|| a.name.cmp(&b.name))
    });
    // One product is reported once, keeping its strongest evidence. Structural
    // leads carry no name and each says something different, so they are kept
    // apart by their evidence.
    found.dedup_by(|a, b| a.name == b.name && (!a.name.is_empty() || a.evidence == b.evidence));
    found
}

/// A section a product wrote under its own name.
fn from_sections(sections: &[Section], found: &mut Vec<Protection>) {
    for section in sections {
        for marker in SECTION_MARKERS {
            let matched = if marker.prefix {
                starts_with(&section.name, marker.section)
            } else {
                is(&section.name, marker.section)
            };
            if matched {
                found.push(Protection {
                    name: unmasked(marker.name),
                    kind: marker.kind,
                    confidence: Confidence::Certain,
                    evidence: format!("section {}", section.name),
                });
            }
        }
    }
}

/// A marker a product wrote into the bytes.
///
/// One walk over the scanned region rather than one per marker. Twenty-odd
/// markers over eight megabytes is a hundred and seventy million comparisons
/// done the naive way, on every file opened; the byte a marker can start with
/// answers almost all of them without touching the marker at all.
fn from_bytes(file: &[u8], found: &mut Vec<Protection>) {
    // Which bytes any marker can begin with, unmasked. A position whose byte
    // is not one of these cannot start any of them.
    let mut opens = [false; 256];
    for marker in STRING_MARKERS {
        if let Some(first) = marker.needle.first() {
            opens[usize::from(first ^ MASK)] = true;
        }
    }

    let head = file.len().min(SCAN_WINDOW);
    let tail = file.len().saturating_sub(SCAN_WINDOW).max(head);
    let mut seen = [false; STRING_MARKERS.len()];
    for (from, to) in [(0, head), (tail, file.len())] {
        let Some(region) = file.get(from..to) else {
            continue;
        };
        for at in 0..region.len() {
            if !opens[usize::from(region[at])] {
                continue;
            }
            for (index, marker) in STRING_MARKERS.iter().enumerate() {
                if !seen[index] && starts_at(region, at, marker.needle) {
                    seen[index] = true;
                }
            }
        }
    }

    for (index, marker) in STRING_MARKERS.iter().enumerate() {
        if seen[index] {
            found.push(Protection {
                name: unmasked(marker.name),
                kind: marker.kind,
                confidence: marker.confidence,
                evidence: format!("marker {}", unmasked(marker.needle)),
            });
        }
    }
}

/// Names the file asks the system for that a protected program uses.
fn from_imports(details: &BinaryDetails, found: &mut Vec<Protection>) {
    let mut seen: Vec<&ImportMarker> = Vec::new();
    for library in &details.imports {
        for function in &library.functions {
            for marker in ANTI_DEBUG_IMPORTS {
                if is(function, marker.name)
                    && !seen.iter().any(|kept| kept.reason == marker.reason)
                {
                    seen.push(marker);
                }
            }
        }
    }
    for marker in seen {
        found.push(Protection {
            name: String::new(),
            kind: ProtectionKind::Protector,
            // A check is a check, not a product: an ordinary program may make
            // the same call, so this points without settling anything.
            confidence: Confidence::Possible,
            evidence: format!(
                "imports {}, which {}",
                unmasked(marker.name),
                unmasked(marker.reason)
            ),
        });
    }
}

/// The shape of a packed file, with nothing naming the product.
///
/// Each of these is a lead on its own and none is proof: a JIT writes into its
/// own code, an ordinary Go binary imports nothing at all, and a compressed
/// resource raises the entropy of a section that holds no code. They are here
/// because together they are the picture a packed file makes, and because a
/// reader looking at a stub deserves to be told the listing may not be the
/// program.
fn from_shape(
    sections: &[Section],
    symbols: &[Symbol],
    details: &BinaryDetails,
    entry_point: Option<u64>,
    found: &mut Vec<Protection>,
) {
    // Code that can rewrite itself. A packer's stub decompresses into the
    // section it runs from, which is why the section carries both rights.
    for section in sections {
        if section.permissions.execute && section.permissions.write {
            found.push(Protection {
                name: String::new(),
                kind: ProtectionKind::Unidentified,
                confidence: Confidence::Possible,
                evidence: format!("section {} is both writable and executable", section.name),
            });
        }
    }

    // Execution starting outside every executable section: the loader is being
    // sent somewhere the section table does not describe as code.
    if let Some(address) = entry_point
        && !sections.is_empty()
        && !sections.iter().any(|section| {
            let end = section
                .virtual_address
                .saturating_add(section.virtual_size.max(section.file_size));
            section.is_mapped()
                && section.permissions.execute
                && (section.virtual_address..end).contains(&address)
        })
    {
        found.push(Protection {
            name: String::new(),
            kind: ProtectionKind::Unidentified,
            confidence: Confidence::Possible,
            evidence: format!("entry point {address:#x} is in no executable section"),
        });
    }

    // A Windows program that asks for almost nothing: the real import table is
    // rebuilt by the stub at run time, so the one in the file names the handful
    // of routines the stub itself needs.
    let imported: usize = details
        .imports
        .iter()
        .map(|library| library.functions.len())
        .sum();
    if !details.imports.is_empty() && imported > 0 && imported <= MINIMAL_IMPORTS {
        found.push(Protection {
            name: String::new(),
            kind: ProtectionKind::Unidentified,
            confidence: Confidence::Possible,
            evidence: format!("only {imported} imported functions across the whole import table"),
        });
    }

    // Every executable section dense enough to be compressed or encrypted, and
    // no symbol left to read: the two together are what a stub looks like.
    let executable: Vec<&Section> = sections
        .iter()
        .filter(|section| section.permissions.execute && section.file_size > 0)
        .collect();
    let all_dense = !executable.is_empty()
        && executable
            .iter()
            .all(|section| section.entropy.is_some_and(entropy::suggests_packing));
    if all_dense {
        let names: Vec<&str> = executable
            .iter()
            .map(|section| section.name.as_str())
            .collect();
        let defined = symbols
            .iter()
            .filter(|symbol| !symbol.imported && symbol.address.is_some())
            .count();
        found.push(Protection {
            name: String::new(),
            kind: ProtectionKind::Unidentified,
            // A file whose code is uniformly dense *and* nameless is a stub far
            // more often than it is a program; density alone is not.
            confidence: if defined == 0 {
                Confidence::Likely
            } else {
                Confidence::Possible
            },
            evidence: format!(
                "every executable section is compressed or encrypted ({})",
                names.join(", ")
            ),
        });
    }
}

/// At or below this many imported functions, a Windows import table is the
/// stub's, not the program's. A hello-world built by MSVC names dozens.
const MINIMAL_IMPORTS: usize = 6;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Permissions;

    fn section(name: &str, entropy: Option<f32>, execute: bool, write: bool) -> Section {
        Section {
            name: name.to_owned(),
            virtual_address: 0x1000,
            file_offset: 0x400,
            virtual_size: 0x1000,
            file_size: 0x1000,
            permissions: Permissions {
                read: true,
                write,
                execute,
            },
            entropy,
        }
    }

    fn detect_sections(sections: &[Section]) -> Vec<Protection> {
        detect(&[], sections, &[], &BinaryDetails::default(), None)
    }

    // The markers these tests need, masked exactly like the table's own.
    //
    // A test that writes `b"UPX!"` puts that marker in the test binary as a
    // plain string — and this suite analyses the test binary. The scanner then
    // finds it, and `the_scanner_does_not_flag_its_own_binary` fails reporting
    // Desdec as packed by UPX, VMProtect and Themida at once. That is the very
    // trap the whole module is masked against; the table was masked and the
    // tests were not, so the tests reintroduced it.
    //
    // Held as `const` so the masking really happens at compile time: written
    // inline, `masked(b"UPX!")` may be worked out at run time and leaves the
    // plain spelling in the image after all.
    const UPX_MAGIC: [u8; 4] = masked(b"UPX!");
    const UPX_NAME: [u8; 3] = masked(b"UPX");
    const VMPROTECT: [u8; 9] = masked(b"VMProtect");
    const THEMIDA: [u8; 20] = masked(b"Themida / WinLicense");

    /// One masked marker, back as the bytes it was written from.
    fn plain(marker: &[u8]) -> Vec<u8> {
        marker.iter().map(|byte| byte ^ MASK).collect()
    }

    #[test]
    fn a_packer_s_own_section_names_it() {
        let found = detect_sections(&[section("UPX1", None, true, true)]);

        let upx = found
            .iter()
            .find(|item| item.name == "UPX")
            .expect("UPX1 is a section only UPX writes");
        assert_eq!(upx.confidence, Confidence::Certain);
        assert_eq!(upx.kind, ProtectionKind::Packer);
        assert_eq!(upx.evidence, "section UPX1");
    }

    #[test]
    fn a_numbered_section_matches_its_family() {
        for name in ["vmp0", ".vmp0", ".vmp1", ".VMP2"] {
            let found = detect_sections(&[section(name, None, true, false)]);
            let named = found
                .iter()
                .any(|item| item.name.as_bytes() == plain(&VMPROTECT));
            // `vmp0` without the dot is not the marker: the prefix is `.vmp`.
            assert_eq!(named, name.starts_with('.'), "{name}");
        }
    }

    #[test]
    fn an_ordinary_section_table_names_no_product() {
        let found = detect_sections(&[
            section(".text", Some(6.1), true, false),
            section(".rodata", Some(4.8), false, false),
            section(".data", Some(2.0), false, true),
        ]);

        assert!(
            found.iter().all(|item| item.name.is_empty()),
            "an ordinary file must not be attributed to a product: {found:?}"
        );
    }

    #[test]
    fn a_marker_in_the_bytes_names_the_product() {
        let mut file = vec![0_u8; 512];
        file.extend_from_slice(&plain(&UPX_MAGIC));
        file.extend_from_slice(&[0_u8; 512]);
        let found = detect(&file, &[], &[], &BinaryDetails::default(), None);

        assert!(
            found
                .iter()
                .any(|item| item.name.as_bytes() == plain(&UPX_NAME)),
            "the stub's own magic was not read: {found:?}"
        );
    }

    #[test]
    fn writable_executable_code_is_a_lead_without_a_name() {
        let found = detect_sections(&[section(".text", Some(6.0), true, true)]);

        let lead = found
            .iter()
            .find(|item| item.kind == ProtectionKind::Unidentified)
            .expect("a writable code section is a lead");
        assert!(lead.name.is_empty(), "a shape names no product");
        assert_eq!(lead.confidence, Confidence::Possible);
    }

    #[test]
    fn an_entry_point_outside_every_executable_section_is_a_lead() {
        let found = detect(
            &[],
            &[section(".text", Some(6.0), true, false)],
            &[],
            &BinaryDetails::default(),
            Some(0xdead_0000),
        );

        assert!(
            found
                .iter()
                .any(|item| item.evidence.contains("in no executable section")),
            "{found:?}"
        );
    }

    #[test]
    fn an_entry_point_inside_the_code_is_not_a_lead() {
        let found = detect(
            &[],
            &[section(".text", Some(6.0), true, false)],
            &[],
            &BinaryDetails::default(),
            Some(0x1100),
        );

        assert!(
            !found
                .iter()
                .any(|item| item.evidence.contains("in no executable section")),
            "{found:?}"
        );
    }

    /// Uniformly dense code with nothing named in it is a stub far more often
    /// than it is a program, and says so more firmly than density alone.
    #[test]
    fn dense_and_nameless_code_is_firmer_than_dense_code_alone() {
        let sections = [section(".text", Some(7.6), true, false)];
        let nameless = detect(&[], &sections, &[], &BinaryDetails::default(), None);
        let named = detect(
            &[],
            &sections,
            &[Symbol {
                name: "main".to_owned(),
                address: Some(0x1000),
                ..Symbol::default()
            }],
            &BinaryDetails::default(),
            None,
        );

        let confidence = |found: &[Protection]| {
            found
                .iter()
                .find(|item| item.evidence.starts_with("every executable section"))
                .map(|item| item.confidence)
        };
        assert_eq!(confidence(&nameless), Some(Confidence::Likely));
        assert_eq!(confidence(&named), Some(Confidence::Possible));
    }

    /// One product found twice — its section and its marker — is one finding.
    #[test]
    fn a_product_found_twice_is_reported_once() {
        let mut file = vec![0_u8; 64];
        file.extend_from_slice(&plain(&UPX_MAGIC));
        let found = detect(
            &file,
            &[section("UPX1", None, true, false)],
            &[],
            &BinaryDetails::default(),
            None,
        );

        assert_eq!(
            found
                .iter()
                .filter(|item| item.name.as_bytes() == plain(&UPX_NAME))
                .count(),
            1,
            "{found:?}"
        );
    }

    /// The masking is what keeps the table out of the compiled image. Reading
    /// one back has to give the spelling it was written with, or every finding
    /// names the wrong product.
    #[test]
    fn a_masked_marker_reads_back_as_itself() {
        assert_eq!(unmasked(&THEMIDA).as_bytes(), plain(&THEMIDA));

        let mut haystack = b"....".to_vec();
        haystack.extend_from_slice(&plain(&VMPROTECT));
        haystack.extend_from_slice(b" begin....");
        assert!(holds(&haystack, &VMPROTECT));
        assert!(!holds(b"an honest program", &VMPROTECT));
    }

    /// The trap this module was written around: Desdec analysing its own
    /// executable must not read this very table back out of itself and report
    /// itself as packed by twenty products at once.
    #[test]
    fn the_scanner_does_not_flag_its_own_binary() {
        let path = std::env::current_exe().expect("the test binary has a path");
        let analysis = crate::analyse_path(&path).expect("analysable");

        let named: Vec<&Protection> = analysis
            .protections
            .iter()
            .filter(|item| item.names_a_product())
            .collect();
        assert!(
            named.is_empty(),
            "the test binary is an ordinary Rust program: {named:?}"
        );
    }

    #[test]
    fn scanning_never_panics_on_any_input() {
        for file in [Vec::new(), vec![0_u8; 1], vec![0xff_u8; 8192]] {
            let _ = detect(&file, &[], &[], &BinaryDetails::default(), Some(0));
        }
    }
}
