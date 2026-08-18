//! Everything an experienced reader wants before opening a disassembler: what
//! kind of file this is, how it will be mapped, what it links against, and
//! which hardening the compiler and linker applied.
//!
//! Each hardening field is a tri-state. `Some(true)` and `Some(false)` mean the
//! format states the answer; `None` means the notion does not apply to this
//! format, or the structure that would answer was unreadable. Reporting an
//! unknown as "absent" would be a security claim we cannot back.

use crate::{
    analysis::{sections::Permissions, strings::ExtractedString},
    binary::{BinaryFormat, Endianness},
};

mod elf;
mod mach_o;
mod pe;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FileKind {
    /// A program the system can start directly.
    Executable,
    /// A library loaded into another program: `.so`, `.dll`, `.dylib`.
    SharedLibrary,
    /// Compiler output, not yet linked.
    ObjectFile,
    /// A snapshot of a crashed process.
    CoreDump,
    /// A macOS plug-in bundle.
    Bundle,
    #[default]
    Unknown,
}

impl FileKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Executable => "Executable",
            Self::SharedLibrary => "Shared library",
            Self::ObjectFile => "Object file",
            Self::CoreDump => "Core dump",
            Self::Bundle => "Bundle",
            Self::Unknown => "Unknown",
        }
    }
}

/// How much of the relocation data is made read-only after loading.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Relro {
    /// Relocations stay writable for the whole run.
    None,
    /// Part of the relocations is protected, but the PLT stays writable.
    Partial,
    /// Everything is resolved at load time, then made read-only.
    Full,
}

impl Relro {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Partial => "partial",
            Self::Full => "full",
        }
    }
}

/// Mitigations recorded in the file. See the module documentation for what
/// `None` means.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Hardening {
    /// The image can be loaded at any address (PIE / `DYNAMIC_BASE`).
    pub position_independent: Option<bool>,
    /// The stack is mapped without execute permission.
    pub non_executable_stack: Option<bool>,
    pub relro: Option<Relro>,
    /// Stack-smashing protection. Detected from linked symbol names, so this
    /// is an indication rather than proof.
    pub stack_canary: Option<bool>,
    /// Windows: the loader may randomise the image base.
    pub address_space_randomisation: Option<bool>,
    /// Windows: pages are marked non-executable where possible.
    pub data_execution_prevention: Option<bool>,
    /// Windows: indirect calls are checked against a table of valid targets.
    pub control_flow_guard: Option<bool>,
    /// The file carries an embedded signature. Its validity is not checked.
    pub signed: Option<bool>,
}

/// The functions one library is asked for, as the import table names them.
///
/// One entry per import descriptor, not per library name: a file may name the
/// same library several times, each descriptor asking for its own functions,
/// and installers routinely do. Merging them by name loses which descriptor
/// asked for what, and leaves the reader a list that no part of the file
/// actually states.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportedLibrary {
    /// The library's name, exactly as the file spells it.
    pub library: String,
    /// The functions taken from it, in import-table order.
    pub functions: Vec<String>,
    /// Set when the table held more names than were kept.
    pub truncated: bool,
}

/// A region as the loader will map it, which is coarser than a section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Segment {
    pub kind: String,
    pub virtual_address: u64,
    pub virtual_size: u64,
    pub file_offset: u64,
    pub file_size: u64,
    pub permissions: Permissions,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BinaryDetails {
    pub file_kind: FileKind,
    /// 32 or 64; `0` when the format does not say.
    pub bits: u8,
    pub endianness: Endianness,
    /// Load-time mapping: ELF program headers, Mach-O segments. Empty for PE,
    /// whose sections are its mapping.
    pub segments: Vec<Segment>,
    /// Libraries this file needs at run time.
    pub linked_libraries: Vec<String>,
    /// What is taken from each of those libraries, where the format records it.
    ///
    /// PE only for now: its import table names every function, which is the
    /// difference between knowing a program links `ntdll.dll` — most Windows
    /// binaries do, whether or not they asked to — and knowing it calls
    /// `NtCreateThreadEx`. ELF and Mach-O leave this empty, so a reader is
    /// never shown an empty list as if it were an answer.
    ///
    /// In import-table order, one entry per descriptor, running parallel to
    /// [`Self::linked_libraries`]: the two are built from the same walk, so a
    /// library listed there has its imports at the same index here.
    pub imports: Vec<ImportedLibrary>,
    pub hardening: Hardening,
    /// ELF: the dynamic loader that will start this program.
    pub interpreter: Option<String>,
    /// Windows: the environment the image expects.
    pub subsystem: Option<&'static str>,
    /// Windows: build timestamp, seconds since the Unix epoch. Frequently
    /// zeroed for reproducible builds, and trivially forged.
    pub timestamp: Option<u32>,
}

impl Default for BinaryDetails {
    fn default() -> Self {
        Self {
            file_kind: FileKind::Unknown,
            bits: 0,
            endianness: Endianness::Unknown,
            segments: Vec::new(),
            linked_libraries: Vec::new(),
            imports: Vec::new(),
            hardening: Hardening::default(),
            interpreter: None,
            subsystem: None,
            timestamp: None,
        }
    }
}

/// Upper bound on entries read from any table, matching the section cap.
pub(super) const MAXIMUM_ENTRIES: usize = 4096;

/// Reads the format-specific details, or the neutral default when the headers
/// cannot be read.
#[must_use]
pub fn parse(file: &[u8], format: BinaryFormat) -> BinaryDetails {
    match format {
        BinaryFormat::Elf { bits, endianness } => elf::details(file, bits, endianness),
        BinaryFormat::Pe => pe::details(file),
        BinaryFormat::MachO { bits, endianness } => mach_o::details(file, bits, endianness),
        BinaryFormat::Unknown => BinaryDetails::default(),
    }
}

/// Marks the stack-canary indication, which needs the linked symbol names that
/// only the caller has gathered.
pub(super) fn note_stack_canary(details: &mut BinaryDetails, strings: &[ExtractedString]) {
    const CANARY_SYMBOLS: &[&str] = &["__stack_chk_fail", "__stack_chk_guard", "__security_cookie"];

    if details.hardening.stack_canary.is_some() {
        return;
    }
    details.hardening.stack_canary = Some(strings.iter().any(|string| {
        CANARY_SYMBOLS
            .iter()
            .any(|known| string.value.contains(known))
    }));
}
