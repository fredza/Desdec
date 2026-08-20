//! Deep analysis of a loaded binary.
//!
//! Where [`crate::inspect_path`] answers "what is this file?" from its first
//! few bytes, this module answers "what is inside it?": the section table, the
//! entry point, the printable strings, and how dense each region looks.
//!
//! Three properties are deliberate:
//!
//! - **Bounded.** Reading stops at [`ANALYSIS_BYTE_LIMIT`], section tables at
//!   4096 entries, strings at 20 000. A hostile header cannot make the analysis
//!   allocate without limit or loop forever.
//! - **Total.** No input panics. Unreadable structures produce empty lists, and
//!   [`Analysis::truncated`] states plainly when the file was only read in part.
//! - **Read-only.** The file is opened for reading and never written to.

use std::{
    fs,
    io::{self, Read},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub mod details;
pub mod disassembly;
pub mod discover;
pub mod entropy;
pub mod flags;
pub mod hash;
pub mod language;
pub mod operand;
pub mod sections;
pub mod stack;
pub mod strings;
pub mod symbols;

pub use details::{BinaryDetails, FileKind, Hardening, ImportedLibrary, Relro, Segment};
pub use disassembly::{Decoded, Instruction, InstructionBytes, decode_one};
pub use language::{Confidence, LanguageEvidence, SourceLanguage};
pub use operand::{LastWrite, Target};
pub use sections::{Permissions, Section};
pub use stack::{StackSlot, StackState, Trace};
pub use strings::{ExtractedString, StringEncoding};
pub use symbols::Symbol;

use crate::binary::{BinarySummary, inspect_bytes};

/// Most bytes read from one file. Beyond this the analysis reports what it saw
/// and marks the result truncated, rather than mapping a multi-gigabyte image
/// into memory.
pub const ANALYSIS_BYTE_LIMIT: u64 = 256 * 1024 * 1024;

/// Everything the current milestone can tell about a binary.
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    pub summary: BinarySummary,
    /// Virtual address execution starts at, when the format states one.
    pub entry_point: Option<u64>,
    pub sections: Vec<Section>,
    pub strings: Vec<ExtractedString>,
    pub symbols: Vec<Symbol>,
    /// Decoded instructions, ordered by address.
    pub instructions: Vec<Instruction>,
    /// Set when executable bytes were never read — a file larger than
    /// [`ANALYSIS_BYTE_LIMIT`] — so the listing is not the whole program.
    /// Nothing else stops the decoder: every executable byte that was read is
    /// decoded, however many instructions that turns out to be.
    pub code_truncated: bool,
    /// Loader-level facts: file kind, mapping, dependencies, hardening.
    pub details: BinaryDetails,
    /// What the file says about the language it was built from, strongest
    /// evidence first. Empty when it says nothing.
    pub languages: Vec<LanguageEvidence>,
    /// SHA-256 of the file, and `None` when only part of it was read — a
    /// digest of a prefix would be mistaken for the file's identity.
    pub sha256: Option<[u8; 32]>,
    /// Entropy of the analysed bytes as a whole.
    pub entropy: Option<f32>,
    /// How many bytes were actually read and analysed.
    pub analysed_bytes: u64,
    /// Set when the file is larger than [`ANALYSIS_BYTE_LIMIT`], meaning
    /// sections and strings past that point were not examined.
    pub truncated: bool,
}

impl Analysis {
    /// Sections carrying executable code.
    pub fn executable_sections(&self) -> impl Iterator<Item = &Section> {
        self.sections
            .iter()
            .filter(|section| section.permissions.execute)
    }

    /// Sections dense enough to suggest compressed or encrypted content.
    ///
    /// This is a lead to follow, not a conclusion: packers produce such
    /// sections, and so do embedded archives and media.
    pub fn dense_sections(&self) -> impl Iterator<Item = &Section> {
        self.sections
            .iter()
            .filter(|section| section.entropy.is_some_and(entropy::suggests_packing))
    }

    /// Whether an executable section is dense enough to warrant a closer look —
    /// the usual signature of a packed binary.
    #[must_use]
    pub fn suggests_packing(&self) -> bool {
        self.executable_sections()
            .any(|section| section.entropy.is_some_and(entropy::suggests_packing))
    }

    /// The decoded instruction at exactly this address, if there is one.
    ///
    /// [`Self::instructions`] is sorted, so this bisects rather than scanning:
    /// the interface asks this question of every frame it draws.
    #[must_use]
    pub fn instruction_at(&self, address: u64) -> Option<&Instruction> {
        self.instruction_index(address)
            .map(|index| &self.instructions[index])
    }

    /// Position of the instruction at exactly this address, for callers that
    /// need to walk the listing from there.
    #[must_use]
    pub fn instruction_index(&self, address: u64) -> Option<usize> {
        self.instructions
            .binary_search_by_key(&address, |instruction| instruction.address)
            .ok()
    }

    /// Where the instructions of an address range sit in the listing.
    ///
    /// Returned as positions rather than a slice so a caller can keep them
    /// past the borrow — a view holds them for as long as the binary is open.
    #[must_use]
    pub fn instruction_span(&self, range: std::ops::Range<u64>) -> std::ops::Range<usize> {
        if range.is_empty() {
            return 0..0;
        }
        let start = self
            .instructions
            .partition_point(|instruction| instruction.address < range.start);
        let end = self
            .instructions
            .partition_point(|instruction| instruction.address < range.end);
        start..end.max(start)
    }

    /// The decoded instructions whose addresses fall in `range`, as a slice of
    /// the listing itself — no copy, no filter over the whole program.
    #[must_use]
    pub fn instructions_in(&self, range: std::ops::Range<u64>) -> &[Instruction] {
        &self.instructions[self.instruction_span(range)]
    }

    /// Section containing a given virtual address, for locating an entry point
    /// or a cross-reference. Only mapped sections are considered.
    #[must_use]
    pub fn section_at(&self, address: u64) -> Option<&Section> {
        self.sections.iter().find(|section| {
            let end = section
                .virtual_address
                .saturating_add(section.virtual_size.max(section.file_size));
            section.is_mapped() && (section.virtual_address..end).contains(&address)
        })
    }

    /// Where an address's byte sits in the file, when the file holds it.
    ///
    /// `None` for an address in a region the file stores nothing for — `.bss`
    /// is mapped but empty on disk, and pointing at a byte of the next section
    /// instead would be a lie about where anything is.
    #[must_use]
    pub fn file_offset_of(&self, address: u64) -> Option<u64> {
        self.sections.iter().find_map(|section| {
            let end = section.virtual_address.saturating_add(section.file_size);
            (section.is_mapped()
                && section.file_size > 0
                && (section.virtual_address..end).contains(&address))
            .then(|| {
                section
                    .file_offset
                    .saturating_add(address.saturating_sub(section.virtual_address))
            })
        })
    }

    /// Where a byte of the file is mapped, when it is mapped at all.
    ///
    /// The other direction of [`Analysis::file_offset_of`], and just as
    /// partial: headers, symbol tables and debug sections are in the file and
    /// nowhere in memory.
    #[must_use]
    pub fn address_at(&self, file_offset: u64) -> Option<(u64, &Section)> {
        self.sections.iter().find_map(|section| {
            let end = section.file_offset.saturating_add(section.file_size);
            (section.is_mapped()
                && section.file_size > 0
                && (section.file_offset..end).contains(&file_offset))
            .then(|| {
                (
                    section
                        .virtual_address
                        .saturating_add(file_offset.saturating_sub(section.file_offset)),
                    section,
                )
            })
        })
    }
}

/// Reads a binary and analyses it in depth.
///
/// # Errors
///
/// Returns an error if the file metadata cannot be read, or if the file cannot
/// be opened or read.
pub fn analyse_path(path: impl AsRef<Path>) -> io::Result<Analysis> {
    analyse_path_cancellable(path, &AtomicBool::new(false)).map(|analysis| {
        // A token created unset cannot be cancelled, so `None` is impossible.
        analysis.expect("an unset cancellation token cannot stop an analysis")
    })
}

/// Reads and analyses a binary until `cancelled` is set.
///
/// `Ok(None)` means cancellation, not malformed input or a partial result. The
/// caller must discard it rather than presenting bytes that look authoritative.
/// The file read itself checks the token every 64 KiB; once CPU analysis has
/// started, the caller can still abandon its result immediately.
pub fn analyse_path_cancellable(
    path: impl AsRef<Path>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Analysis>> {
    const READ_CHUNK: usize = 64 * 1024;
    let path = path.as_ref();
    let size = fs::metadata(path)?.len();
    let limit = usize::try_from(size.min(ANALYSIS_BYTE_LIMIT)).unwrap_or(0);
    let mut bytes = Vec::with_capacity(limit);
    let mut file = fs::File::open(path)?;
    let mut chunk = [0_u8; READ_CHUNK];

    while bytes.len() < limit {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let wanted = (limit - bytes.len()).min(chunk.len());
        let read = file.read(&mut chunk[..wanted])?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    if cancelled.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let analysis = analyse_bytes(path, size, &bytes);
    if cancelled.load(Ordering::Relaxed) {
        Ok(None)
    } else {
        Ok(Some(analysis))
    }
}

/// Above this, spreading the analysis across cores pays for itself.
///
/// Below it the threads cost more than they save — measured, not assumed: at
/// 1.5 MB the parallel path was consistently slower than the sequential one,
/// and from 3 MB up it was consistently faster. Small binaries take a few
/// milliseconds either way, so they keep the sequential path rather than
/// trade their latency for nothing.
const PARALLEL_THRESHOLD: usize = 2 * 1024 * 1024;

/// Analyses bytes already in memory, where `size` is the size of the whole
/// file even when `bytes` holds only its beginning.
///
/// Above [`PARALLEL_THRESHOLD`], the independent parts run on separate
/// threads, so a large binary takes about as long as its slowest part instead
/// of the sum of all of them. The digest is usually that slowest part —
/// SHA-256 is sequential by construction and cannot be split — so everything
/// else is arranged to proceed alongside it.
///
/// Only the scheduling differs between the two paths. Each part computes
/// exactly what it computed before, and the assembled [`Analysis`] is
/// identical whichever path ran, which
/// [`tests::both_paths_produce_the_same_analysis`] holds.
#[must_use]
pub fn analyse_bytes(path: &Path, size: u64, bytes: &[u8]) -> Analysis {
    if bytes.len() < PARALLEL_THRESHOLD {
        return sequentially(path, size, bytes);
    }
    concurrently(path, size, bytes)
}

/// One thread, in the order the analysis has always run in.
fn sequentially(path: &Path, size: u64, bytes: &[u8]) -> Analysis {
    let (format, architecture) = inspect_bytes(bytes);
    let truncated = size > bytes.len() as u64;

    let strings = strings::extract(bytes);
    let symbols = symbols::extract(bytes, format);
    let sections = sections::parse(bytes, format);
    let code = disassembly::decode(bytes, format, architecture, &sections);
    let mut details = details::parse(bytes, format);
    details::note_stack_canary(&mut details, &strings);
    let languages = language::detect(bytes, &sections, &symbols, &details);

    Analysis {
        summary: summary(path, size, bytes),
        entry_point: sections::entry_point(bytes, format),
        sections,
        strings,
        symbols,
        instructions: code.instructions,
        code_truncated: code.truncated,
        details,
        languages,
        sha256: (!truncated).then(|| hash::sha256(bytes)),
        entropy: entropy::shannon(bytes),
        analysed_bytes: bytes.len() as u64,
        truncated,
    }
}

/// The same work, spread over the machine's cores.
fn concurrently(path: &Path, size: u64, bytes: &[u8]) -> Analysis {
    let (format, architecture) = inspect_bytes(bytes);
    let truncated = size > bytes.len() as u64;

    std::thread::scope(|scope| {
        // Spawned first: the longest pole, so the rest fills the time it takes
        // instead of queueing behind it.
        let digest = scope.spawn(move || (!truncated).then(|| hash::sha256(bytes)));
        let entropy = scope.spawn(move || entropy::shannon(bytes));
        // The canary check reads the extracted strings, so these two stay on
        // one thread rather than synchronising for a single boolean.
        let described = scope.spawn(move || {
            let strings = strings::extract(bytes);
            let mut details = details::parse(bytes, format);
            details::note_stack_canary(&mut details, &strings);
            (strings, details)
        });
        let symbols = scope.spawn(move || symbols::extract(bytes, format));
        let entry_point = scope.spawn(move || sections::entry_point(bytes, format));
        // Decoding needs the section table, so it follows it on this thread.
        let code = scope.spawn(move || {
            let sections = sections::parse(bytes, format);
            let decoded = disassembly::decode(bytes, format, architecture, &sections);
            (sections, decoded)
        });

        let (strings, details) = join(described);
        let (sections, decoded) = join(code);
        let symbols = join(symbols);
        // Reads the three results above, so it waits for them rather than
        // running as a seventh thread.
        let languages = language::detect(bytes, &sections, &symbols, &details);
        Analysis {
            summary: summary(path, size, bytes),
            entry_point: join(entry_point),
            sections,
            strings,
            symbols,
            instructions: decoded.instructions,
            code_truncated: decoded.truncated,
            details,
            languages,
            sha256: join(digest),
            entropy: join(entropy),
            analysed_bytes: bytes.len() as u64,
            truncated,
        }
    })
}

fn summary(path: &Path, size: u64, bytes: &[u8]) -> BinarySummary {
    let (format, architecture) = inspect_bytes(bytes);
    BinarySummary {
        path: path.to_path_buf(),
        size,
        format,
        architecture,
    }
}

/// Waits for one part of the analysis, letting a panic keep its original
/// backtrace instead of being reported as a mysterious failure.
fn join<T>(handle: std::thread::ScopedJoinHandle<'_, T>) -> T {
    handle
        .join()
        .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Architecture, BinaryFormat, Endianness};
    use std::path::PathBuf;

    fn analyse(bytes: &[u8]) -> Analysis {
        analyse_bytes(Path::new("test.bin"), bytes.len() as u64, bytes)
    }

    /// The threaded path must answer exactly what the sequential one does.
    ///
    /// This is the property the whole parallel arrangement rests on: which
    /// path ran is an implementation detail of the machine, and a report that
    /// changed with the core count could not be compared against another run.
    /// Both are called directly, so the test does not depend on where
    /// [`PARALLEL_THRESHOLD`] happens to sit.
    #[test]
    fn a_cancelled_analysis_returns_no_partial_result() {
        let cancelled = AtomicBool::new(true);
        let path = std::env::current_exe().expect("the test binary has a path");

        assert!(
            analyse_path_cancellable(path, &cancelled)
                .expect("metadata stays readable")
                .is_none()
        );
    }

    #[test]
    fn both_paths_produce_the_same_analysis() {
        // A real binary as well as the fixtures: the fixtures carry almost
        // nothing, so on their own they would let a whole part of the analysis
        // differ between the paths unnoticed.
        let real = std::fs::read(std::env::current_exe().expect("the test binary has a path"))
            .expect("the test binary is readable");
        let analysed = analyse(&real);
        // Strings are read straight from the bytes, so a real binary yields
        // them whatever executable format the platform uses.
        assert!(
            !analysed.strings.is_empty(),
            "the reference binary must exercise the analysis"
        );
        // Symbols are now read from all three formats, so this holds on every
        // platform the tests run on. It was briefly conditional on Linux,
        // while PE and Mach-O returned nothing.
        assert!(
            !analysed.symbols.is_empty(),
            "the reference binary must reach the symbol table"
        );

        let path = Path::new("test.bin");
        for bytes in [
            real,
            elf_fixture(),
            elf_fixture_with_text(&[0x90; 4096]),
            pe_fixture(),
            Vec::new(),
            vec![0_u8; 8192],
        ] {
            let size = bytes.len() as u64;
            assert_eq!(
                sequentially(path, size, &bytes),
                concurrently(path, size, &bytes),
                "the two paths disagreed on a {} byte input",
                bytes.len()
            );
        }
    }

    /// The listing is bisected by every view that reads it, so an address must
    /// map to exactly its own instruction, and a range to exactly its own
    /// instructions — never to a neighbour.
    #[test]
    fn instructions_are_found_and_bounded_by_address() {
        let analysis = analyse(&elf_fixture_with_text(&[0x90; 16])); // 16 `nop`s.
        let addresses: Vec<u64> = analysis
            .instructions
            .iter()
            .map(|instruction| instruction.address)
            .collect();
        assert_eq!(addresses.len(), 16, "the fixture decodes one nop per byte");
        let first = addresses[0];

        for (index, address) in addresses.iter().enumerate() {
            assert_eq!(analysis.instruction_index(*address), Some(index));
            assert_eq!(
                analysis.instruction_at(*address).map(|found| found.address),
                Some(*address)
            );
        }
        assert_eq!(analysis.instruction_at(first - 1), None);
        assert_eq!(analysis.instruction_at(first + 1000), None);

        // A range takes the instructions inside it and stops at its end.
        let middle = analysis.instructions_in(first + 3..first + 7);
        assert_eq!(middle.len(), 4);
        assert_eq!(middle[0].address, first + 3);
        assert!(analysis.instructions_in(first..first).is_empty());
        assert_eq!(analysis.instructions_in(0..u64::MAX).len(), 16);
    }

    /// A truncated file has no digest on either path: a digest of a prefix
    /// would be taken for the file's identity.
    #[test]
    fn both_paths_agree_on_a_truncated_file() {
        let path = Path::new("test.bin");
        let bytes = elf_fixture();
        // A size larger than the bytes on hand is what marks it truncated.
        let size = bytes.len() as u64 * 4;

        let sequential = sequentially(path, size, &bytes);
        let concurrent = concurrently(path, size, &bytes);

        assert!(sequential.truncated);
        assert_eq!(sequential.sha256, None);
        assert_eq!(sequential, concurrent);
    }

    /// Both paths stay reachable in practice, whichever way the threshold is
    /// later tuned: too low and a small binary pays for threads it does not
    /// need, too high and no binary ever uses the cores.
    #[test]
    fn the_threshold_sits_between_a_small_and_a_large_binary() {
        let limit = usize::try_from(ANALYSIS_BYTE_LIMIT).unwrap_or(usize::MAX);
        assert!((64 * 1024..limit).contains(&PARALLEL_THRESHOLD));
    }

    /// Ordinary machine code: a function prologue and a return.
    const PLAIN_CODE: &[u8] = &[
        0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 0x10, 0x31, 0xc0, 0xc9, 0xc3, 0x00, 0x00, 0x00,
        0x00,
    ];

    fn elf_fixture() -> Vec<u8> {
        elf_fixture_with_text(PLAIN_CODE)
    }

    /// A minimal but structurally valid 64-bit little-endian ELF holding a
    /// section table of three entries — the mandatory null entry, `.text` with
    /// the given bytes, and the name table.
    fn elf_fixture_with_text(text: &[u8]) -> Vec<u8> {
        const HEADER: usize = 64;
        const ENTRY_SIZE: usize = 64;
        let names = b"\0.text\0.bss\0.shstrtab\0";
        let table = HEADER;
        let name_table_offset = table + ENTRY_SIZE * 3;
        let text_offset = name_table_offset + names.len();

        let mut file = vec![0_u8; text_offset + text.len()];
        file[..4].copy_from_slice(b"\x7fELF");
        file[4] = 2; // 64-bit
        file[5] = 1; // little-endian
        file[18..20].copy_from_slice(&62_u16.to_le_bytes()); // x86-64
        file[24..32].copy_from_slice(&0x40_1000_u64.to_le_bytes()); // e_entry
        file[40..48].copy_from_slice(&(table as u64).to_le_bytes()); // e_shoff
        file[58..60].copy_from_slice(&u16::try_from(ENTRY_SIZE).unwrap().to_le_bytes());
        file[60..62].copy_from_slice(&3_u16.to_le_bytes()); // e_shnum
        file[62..64].copy_from_slice(&2_u16.to_le_bytes()); // e_shstrndx

        // Entry 0 is the mandatory null section, left zeroed.
        // Entry 1: .text, allocated and executable.
        let header = table + ENTRY_SIZE;
        file[header..header + 4].copy_from_slice(&1_u32.to_le_bytes()); // sh_name -> ".text"
        file[header + 4..header + 8].copy_from_slice(&1_u32.to_le_bytes()); // SHT_PROGBITS
        file[header + 8..header + 16].copy_from_slice(&0x6_u64.to_le_bytes()); // ALLOC | EXECINSTR
        file[header + 16..header + 24].copy_from_slice(&0x40_1000_u64.to_le_bytes());
        file[header + 24..header + 32].copy_from_slice(&(text_offset as u64).to_le_bytes());
        file[header + 32..header + 40].copy_from_slice(&(text.len() as u64).to_le_bytes());

        // Entry 2: .shstrtab itself.
        let strtab = table + ENTRY_SIZE * 2;
        file[strtab..strtab + 4].copy_from_slice(&12_u32.to_le_bytes()); // ".shstrtab"
        file[strtab + 4..strtab + 8].copy_from_slice(&3_u32.to_le_bytes()); // SHT_STRTAB
        file[strtab + 24..strtab + 32].copy_from_slice(&(name_table_offset as u64).to_le_bytes());
        file[strtab + 32..strtab + 40].copy_from_slice(&(names.len() as u64).to_le_bytes());

        file[name_table_offset..text_offset].copy_from_slice(names);
        file[text_offset..].copy_from_slice(text);
        file
    }

    #[test]
    fn elf_sections_carry_their_name_address_and_rights() {
        let analysis = analyse(&elf_fixture());

        assert_eq!(
            analysis.summary.format,
            BinaryFormat::Elf {
                bits: 64,
                endianness: Endianness::Little
            }
        );
        assert_eq!(analysis.summary.architecture, Architecture::X86_64);
        assert_eq!(analysis.entry_point, Some(0x40_1000));

        let text = analysis
            .sections
            .iter()
            .find(|section| section.name == ".text")
            .expect("the fixture defines .text");
        assert_eq!(text.virtual_address, 0x40_1000);
        assert_eq!(text.file_size, PLAIN_CODE.len() as u64);
        assert_eq!(text.permissions.label(), "r-x");
        assert_eq!(text.bytes_in(&elf_fixture()), Some(PLAIN_CODE));
        assert!(text.entropy.is_some());
        assert!(text.is_mapped());

        assert_eq!(analysis.executable_sections().count(), 1);
        assert!(!analysis.suggests_packing());
    }

    #[test]
    fn an_address_is_traced_back_to_its_section() {
        let analysis = analyse(&elf_fixture());
        let entry = analysis
            .entry_point
            .expect("the fixture has an entry point");

        assert_eq!(
            analysis
                .section_at(entry)
                .map(|section| section.name.clone()),
            Some(".text".to_owned())
        );
        assert!(
            analysis.section_at(0).is_none(),
            "unmapped sections such as .shstrtab sit at address 0 and must not match"
        );
        assert!(analysis.section_at(entry - 1).is_none());
    }

    /// A PE with one section header, an image base and an entry point.
    fn pe_fixture() -> Vec<u8> {
        const SIGNATURE: usize = 0x80;
        const OPTIONAL: usize = SIGNATURE + 24;
        const OPTIONAL_SIZE: usize = 0xf0;
        let table = OPTIONAL + OPTIONAL_SIZE;

        let mut file = vec![0_u8; table + 40 + 64];
        file[..2].copy_from_slice(b"MZ");
        file[0x3c..0x40].copy_from_slice(&u32::try_from(SIGNATURE).unwrap().to_le_bytes());
        file[SIGNATURE..SIGNATURE + 4].copy_from_slice(b"PE\0\0");
        file[SIGNATURE + 4..SIGNATURE + 6].copy_from_slice(&0x8664_u16.to_le_bytes());
        file[SIGNATURE + 6..SIGNATURE + 8].copy_from_slice(&1_u16.to_le_bytes()); // one section
        file[SIGNATURE + 20..SIGNATURE + 22]
            .copy_from_slice(&u16::try_from(OPTIONAL_SIZE).unwrap().to_le_bytes());

        file[OPTIONAL..OPTIONAL + 2].copy_from_slice(&0x20b_u16.to_le_bytes()); // PE32+
        file[OPTIONAL + 16..OPTIONAL + 20].copy_from_slice(&0x1000_u32.to_le_bytes()); // entry RVA
        file[OPTIONAL + 24..OPTIONAL + 32].copy_from_slice(&0x1_4000_0000_u64.to_le_bytes());
        file[OPTIONAL + 68..OPTIONAL + 70].copy_from_slice(&3_u16.to_le_bytes()); // console
        // DYNAMIC_BASE | NX_COMPAT | GUARD_CF
        file[OPTIONAL + 70..OPTIONAL + 72].copy_from_slice(&0x4140_u16.to_le_bytes());
        file[SIGNATURE + 8..SIGNATURE + 12].copy_from_slice(&0x6000_0000_u32.to_le_bytes());
        file[SIGNATURE + 22..SIGNATURE + 24].copy_from_slice(&0x0002_u16.to_le_bytes()); // EXE

        file[table..table + 8].copy_from_slice(b".text\0\0\0");
        file[table + 8..table + 12].copy_from_slice(&0x200_u32.to_le_bytes()); // virtual size
        file[table + 12..table + 16].copy_from_slice(&0x1000_u32.to_le_bytes()); // RVA
        file[table + 16..table + 20].copy_from_slice(&0x40_u32.to_le_bytes()); // raw size
        file[table + 20..table + 24]
            .copy_from_slice(&(u32::try_from(table).unwrap() + 40).to_le_bytes());
        file[table + 36..table + 40].copy_from_slice(&0x6000_0020_u32.to_le_bytes());
        file
    }

    #[test]
    fn pe_sections_are_placed_relative_to_the_image_base() {
        let analysis = analyse(&pe_fixture());

        assert_eq!(analysis.summary.format, BinaryFormat::Pe);
        assert_eq!(analysis.entry_point, Some(0x1_4000_1000));
        assert_eq!(analysis.sections.len(), 1);

        let text = &analysis.sections[0];
        assert_eq!(text.name, ".text");
        assert_eq!(text.virtual_address, 0x1_4000_1000);
        assert_eq!(text.permissions.label(), "r-x");
    }

    /// A 64-bit little-endian Mach-O with one `LC_SEGMENT_64` holding one
    /// section, followed by an `LC_MAIN`.
    fn mach_o_fixture() -> Vec<u8> {
        const HEADER: usize = 32;
        const SEGMENT_SIZE: usize = 72 + 80;
        const MAIN_SIZE: usize = 24;
        const DYLIB_SIZE: usize = 32;
        const SIGNATURE_SIZE: usize = 16;
        let segment = HEADER;
        let section = segment + 72;
        let main = segment + SEGMENT_SIZE;
        let dylib = main + MAIN_SIZE;
        let signature = dylib + DYLIB_SIZE;

        let mut file = vec![0_u8; signature + SIGNATURE_SIZE];
        file[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]); // 64-bit little-endian
        file[4..8].copy_from_slice(&0x0100_0007_u32.to_le_bytes()); // x86-64
        file[16..20].copy_from_slice(&4_u32.to_le_bytes()); // ncmds
        file[24..28].copy_from_slice(&0x0020_0000_u32.to_le_bytes()); // MH_PIE
        file[12..16].copy_from_slice(&2_u32.to_le_bytes()); // MH_EXECUTE

        file[segment..segment + 4].copy_from_slice(&0x19_u32.to_le_bytes()); // LC_SEGMENT_64
        file[segment + 4..segment + 8]
            .copy_from_slice(&u32::try_from(SEGMENT_SIZE).unwrap().to_le_bytes());
        file[segment + 8..segment + 16].copy_from_slice(b"__TEXT\0\0");
        file[segment + 24..segment + 32].copy_from_slice(&0x1_0000_0000_u64.to_le_bytes()); // vmaddr
        file[segment + 32..segment + 40].copy_from_slice(&0x1000_u64.to_le_bytes()); // vmsize
        file[segment + 40..segment + 48].copy_from_slice(&0_u64.to_le_bytes()); // fileoff
        file[segment + 48..segment + 56].copy_from_slice(&0x1000_u64.to_le_bytes()); // filesize
        file[segment + 60..segment + 64].copy_from_slice(&5_u32.to_le_bytes()); // initprot r-x
        file[segment + 64..segment + 68].copy_from_slice(&1_u32.to_le_bytes()); // nsects

        file[section..section + 16].copy_from_slice(b"__text\0\0\0\0\0\0\0\0\0\0");
        file[section + 16..section + 32].copy_from_slice(b"__TEXT\0\0\0\0\0\0\0\0\0\0");
        file[section + 32..section + 40].copy_from_slice(&0x1_0000_1000_u64.to_le_bytes()); // addr
        file[section + 40..section + 48].copy_from_slice(&0x20_u64.to_le_bytes()); // size
        file[section + 48..section + 52].copy_from_slice(&0_u32.to_le_bytes()); // offset

        file[main..main + 4].copy_from_slice(&0x8000_0028_u32.to_le_bytes()); // LC_MAIN
        file[main + 4..main + 8].copy_from_slice(&u32::try_from(MAIN_SIZE).unwrap().to_le_bytes());
        file[main + 8..main + 16].copy_from_slice(&0x1000_u64.to_le_bytes()); // entryoff

        file[dylib..dylib + 4].copy_from_slice(&0xc_u32.to_le_bytes()); // LC_LOAD_DYLIB
        file[dylib + 4..dylib + 8]
            .copy_from_slice(&u32::try_from(DYLIB_SIZE).unwrap().to_le_bytes());
        file[dylib + 8..dylib + 12].copy_from_slice(&24_u32.to_le_bytes()); // name offset
        file[dylib + 24..dylib + 32].copy_from_slice(b"/lib.dy\0");

        file[signature..signature + 4].copy_from_slice(&0x1d_u32.to_le_bytes()); // LC_CODE_SIGNATURE
        file[signature + 4..signature + 8]
            .copy_from_slice(&u32::try_from(SIGNATURE_SIZE).unwrap().to_le_bytes());
        file
    }

    #[test]
    fn mach_o_sections_are_named_after_their_segment() {
        let analysis = analyse(&mach_o_fixture());

        assert_eq!(
            analysis.summary.format,
            BinaryFormat::MachO {
                bits: 64,
                endianness: Endianness::Little
            }
        );
        assert_eq!(analysis.sections.len(), 1);

        let text = &analysis.sections[0];
        assert_eq!(text.name, "__TEXT,__text");
        assert_eq!(text.virtual_address, 0x1_0000_1000);
        assert_eq!(text.permissions.label(), "r-x");

        // `entryoff` counts from the start of the file, so the address is the
        // __TEXT *segment* base (0x1_0000_0000) plus that offset — not the base
        // of its first section.
        assert_eq!(analysis.entry_point, Some(0x1_0000_1000));
        assert_eq!(
            analysis
                .section_at(0x1_0000_1000)
                .map(|section| section.name.clone()),
            Some("__TEXT,__text".to_owned())
        );
    }

    #[test]
    fn a_load_command_that_never_advances_is_rejected() {
        let mut file = mach_o_fixture();
        // A command size of zero would loop forever if it were trusted.
        file[36..40].copy_from_slice(&0_u32.to_le_bytes());

        let analysis = analyse(&file);
        assert!(analysis.sections.is_empty());
    }

    #[test]
    fn pe_details_report_subsystem_and_mitigations() {
        let analysis = analyse(&pe_fixture());
        let details = &analysis.details;

        assert_eq!(details.file_kind, FileKind::Executable);
        assert_eq!(details.bits, 64);
        assert_eq!(details.subsystem, Some("Windows console"));
        assert_eq!(details.timestamp, Some(0x6000_0000));
        assert_eq!(details.hardening.address_space_randomisation, Some(true));
        assert_eq!(details.hardening.data_execution_prevention, Some(true));
        assert_eq!(details.hardening.control_flow_guard, Some(true));
        assert_eq!(details.hardening.signed, Some(false));
        // PE has no separate program headers: its sections are the mapping.
        assert!(details.segments.is_empty());
        // ELF-only notions must stay unknown rather than be reported absent.
        assert_eq!(details.hardening.relro, None);
        assert_eq!(details.hardening.non_executable_stack, None);
    }

    #[test]
    fn mach_o_details_report_dylibs_and_signature() {
        let analysis = analyse(&mach_o_fixture());
        let details = &analysis.details;

        assert_eq!(details.file_kind, FileKind::Executable);
        assert_eq!(details.hardening.position_independent, Some(true));
        assert_eq!(details.hardening.signed, Some(true));
        assert_eq!(details.linked_libraries, vec!["/lib.dy".to_owned()]);
        assert_eq!(details.segments.len(), 1);
        assert_eq!(details.segments[0].kind, "__TEXT");
        assert_eq!(details.segments[0].permissions.label(), "r-x");
        assert_eq!(details.hardening.control_flow_guard, None);
    }

    #[test]
    fn a_digest_is_withheld_when_only_part_of_the_file_was_read() {
        let bytes = elf_fixture();
        let whole = analyse(&bytes);
        assert_eq!(whole.sha256, Some(hash::sha256(&bytes)));

        let partial = analyse_bytes(Path::new("big.bin"), ANALYSIS_BYTE_LIMIT * 2, &bytes);
        assert_eq!(
            partial.sha256, None,
            "a digest of a prefix would be mistaken for the file's identity"
        );
    }

    #[test]
    fn details_of_an_unreadable_file_claim_nothing() {
        for bytes in [&b""[..], &b"MZ"[..], &b"not a binary"[..]] {
            let details = analyse(bytes).details;
            assert_eq!(details.file_kind, FileKind::Unknown);
            assert!(details.segments.is_empty());
            assert!(details.linked_libraries.is_empty());
            assert_eq!(details.hardening.position_independent, None);
            assert_eq!(details.hardening.relro, None);
        }
    }

    #[test]
    fn strings_are_collected_from_the_whole_file() {
        let mut file = elf_fixture();
        file.extend_from_slice(b"https://example.invalid/licence\0");

        let analysis = analyse(&file);
        assert!(
            analysis
                .strings
                .iter()
                .any(|string| string.value == "https://example.invalid/licence"),
            "got {:?}",
            analysis.strings
        );
    }

    #[test]
    fn a_partially_read_file_says_so() {
        let bytes = elf_fixture();
        let analysis = analyse_bytes(Path::new("big.bin"), ANALYSIS_BYTE_LIMIT * 2, &bytes);

        assert!(analysis.truncated);
        assert_eq!(analysis.analysed_bytes, bytes.len() as u64);
        assert_eq!(analysis.summary.size, ANALYSIS_BYTE_LIMIT * 2);
    }

    #[test]
    fn ordinary_code_is_not_mistaken_for_packed_content() {
        let analysis = analyse(&elf_fixture());

        assert!(!analysis.suggests_packing());
        assert_eq!(analysis.dense_sections().count(), 0);
    }

    #[test]
    fn a_dense_executable_section_is_flagged() {
        // Every byte value exactly once: maximum entropy, the signature of
        // compressed or encrypted content in a section meant to hold code.
        let packed: Vec<u8> = (0..=255).collect();
        let analysis = analyse(&elf_fixture_with_text(&packed));

        let text = analysis
            .sections
            .iter()
            .find(|section| section.name == ".text")
            .expect("the fixture defines .text");
        assert_eq!(text.entropy, Some(entropy::MAXIMUM));
        assert!(analysis.suggests_packing());
        assert_eq!(analysis.dense_sections().count(), 1);
    }

    /// Fixtures are hand-built and can encode the same misunderstanding twice.
    /// The test binary is a real executable produced by a real linker, in the
    /// native format of whichever platform this runs on.
    #[test]
    fn a_real_executable_parses_end_to_end() {
        let path = std::env::current_exe().expect("the test binary has a path");
        let analysis = analyse_path(&path).expect("the test binary should be analysed");

        assert_ne!(
            analysis.summary.format,
            BinaryFormat::Unknown,
            "the running executable must be a recognised format"
        );
        assert!(
            !analysis.sections.is_empty(),
            "a linked executable always has sections"
        );
        assert!(
            analysis.executable_sections().next().is_some(),
            "a linked executable always has executable code"
        );

        assert_eq!(
            analysis.details.file_kind,
            FileKind::Executable,
            "the test binary is an executable, not a library"
        );
        assert!(
            !analysis.details.linked_libraries.is_empty(),
            "a test binary links against at least the system C library"
        );
        assert_eq!(
            analysis.sha256,
            Some(hash::sha256(
                &fs::read(&path).expect("the test binary is readable")
            )),
            "the reported digest must cover the whole file"
        );

        if matches!(analysis.summary.format, BinaryFormat::Elf { .. }) {
            let details = &analysis.details;
            assert!(
                !details.segments.is_empty(),
                "an ELF executable is mapped through program headers"
            );
            assert!(
                details.interpreter.is_some(),
                "a dynamically linked executable names its loader"
            );
            assert!(details.hardening.relro.is_some());
            assert!(details.hardening.non_executable_stack.is_some());
        }

        let entry = analysis
            .entry_point
            .expect("an executable has an entry point");
        let section = analysis
            .section_at(entry)
            .expect("the entry point falls inside a mapped section");
        assert!(
            section.permissions.execute,
            "the entry point lands in {} which is not executable",
            section.name
        );
        assert!(
            analysis
                .strings
                .iter()
                .any(|string| string.value.contains('/') || string.value.contains('\\')),
            "a test binary embeds at least one path-like string"
        );
    }

    #[test]
    fn unreadable_input_yields_an_empty_analysis_instead_of_failing() {
        for bytes in [&b""[..], &b"MZ"[..], &b"not a binary at all"[..]] {
            let analysis = analyse(bytes);
            assert!(analysis.sections.is_empty());
            assert_eq!(analysis.entry_point, None);
            assert!(!analysis.suggests_packing());
        }
    }

    #[test]
    fn a_corrupted_section_count_cannot_run_away() {
        let mut file = elf_fixture();
        // Claim 65 535 sections in a file that holds three.
        file[60..62].copy_from_slice(&u16::MAX.to_le_bytes());

        let analysis = analyse(&file);
        assert!(analysis.sections.len() <= 4096);
    }

    #[test]
    fn analysing_a_real_file_reads_it_from_disk() {
        let path = std::env::temp_dir().join(format!("desdec-analysis-{}.bin", std::process::id()));
        fs::write(&path, elf_fixture()).expect("fixture should be written");

        let analysis = analyse_path(&path).expect("fixture should be analysed");
        fs::remove_file(&path).expect("fixture should be removed");

        assert_eq!(analysis.summary.path, PathBuf::from(&path));
        assert!(!analysis.truncated);
        assert!(
            analysis
                .sections
                .iter()
                .any(|section| section.name == ".text")
        );
    }
}
