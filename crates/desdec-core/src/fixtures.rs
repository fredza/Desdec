//! Synthetic binaries, built byte by byte, for tests that must not depend on
//! the machine they run on.
//!
//! The interface tests long used the test executable itself as their sample
//! binary. That is a real, rich file — and exactly one format and one
//! architecture: whatever the host happens to be. A Linux run never exercised
//! the Mach-O reader, and a reader that quietly returned nothing there would
//! have looked no different from a binary that names no functions.
//!
//! Each fixture is small, deliberate, and complete enough to reach the whole
//! analysis: real machine code in an executable section, named function
//! symbols, printable strings, and a linked library. What a fixture holds is
//! declared in the [`Fixture`] beside it, so a test asserts against names
//! rather than against offsets forged here.
//!
//! Available under the `fixtures` feature, and always to this crate's own
//! tests.

use crate::binary::Endianness;

/// A synthetic binary and what it was built to contain.
#[derive(Clone, Debug)]
pub struct Fixture {
    /// Format and architecture, for test messages.
    pub label: &'static str,
    pub bytes: Vec<u8>,
    /// Functions the file names, with the address of each.
    pub functions: Vec<(&'static str, u64)>,
    /// Printable strings the file carries.
    pub strings: Vec<&'static str>,
    /// Libraries the file says it links against.
    pub libraries: Vec<&'static str>,
    /// Functions the file says it imports, for the formats whose dependency
    /// list names them. Empty for ELF and Mach-O, which name only the library.
    pub imported_functions: Vec<&'static str>,
    /// Where execution starts.
    pub entry_point: u64,
}

/// Every fixture: one per format the analysis reads.
#[must_use]
pub fn all() -> Vec<Fixture> {
    vec![elf_x86_64(), pe_x86_64(), mach_o_arm64()]
}

/// Two x86-64 functions: `main`, which branches, and `helper`, which does not.
///
/// The branch is what gives the control-flow view something to draw — `main`
/// decodes into more than one basic block.
const X86_CODE: &[u8] = &[
    // main:
    0x55, // push %rbp
    0x48, 0x89, 0xe5, // mov %rsp,%rbp
    0x31, 0xc0, // xor %eax,%eax
    0x85, 0xc0, // test %eax,%eax
    0x74, 0x02, // je +2
    0x31, 0xc0, // xor %eax,%eax
    0x5d, // pop %rbp
    0xc3, // ret
    // helper:
    0x55, // push %rbp
    0x48, 0x89, 0xe5, // mov %rsp,%rbp
    0xb8, 0x2a, 0x00, 0x00, 0x00, // mov $42,%eax
    0x5d, // pop %rbp
    0xc3, // ret
];
const X86_MAIN_SIZE: u64 = 14;
const X86_HELPER_AT: u64 = 14;
const X86_HELPER_SIZE: u64 = 11;

/// The same two functions in `AArch64`, for the Apple Silicon reader.
///
/// `main` branches here too: basic blocks are found from the mnemonics, and
/// the ARM ones look nothing like the x86 ones, so a fixture without a branch
/// would leave that half untested.
const ARM64_CODE: &[u8] = &[
    // main:
    0xfd, 0x7b, 0xbf, 0xa9, // stp x29, x30, [sp, #-16]!
    0x00, 0x00, 0x80, 0x52, // mov w0, #0
    0x40, 0x00, 0x00, 0x54, // b.eq +8
    0x1f, 0x20, 0x03, 0xd5, // nop
    0xfd, 0x7b, 0xc1, 0xa8, // ldp x29, x30, [sp], #16
    0xc0, 0x03, 0x5f, 0xd6, // ret
    // helper:
    0x40, 0x05, 0x80, 0x52, // mov w0, #42
    0xc0, 0x03, 0x5f, 0xd6, // ret
];
const ARM64_HELPER_AT: u64 = 24;

/// Strings every fixture carries in a read-only section.
const STRINGS: [&str; 2] = ["Desdec fixture binary", "hello from a fixture"];

fn strings_blob() -> Vec<u8> {
    let mut blob = Vec::new();
    for text in STRINGS {
        blob.extend_from_slice(text.as_bytes());
        blob.push(0);
    }
    blob
}

/// Narrows a forged constant to the width its header field has.
///
/// Every value passed here is written into this file, not read from an input.
fn small(value: usize) -> u16 {
    u16::try_from(value).expect("fixtures stay small")
}

/// Appends `bytes` and returns where they landed.
fn place(file: &mut Vec<u8>, bytes: &[u8]) -> usize {
    let at = file.len();
    file.extend_from_slice(bytes);
    at
}

/// Writes little-endian words into a buffer that is already the right size.
struct Writer<'a>(&'a mut [u8]);

impl Writer<'_> {
    fn u16(&mut self, at: usize, value: u16) {
        self.0[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, at: usize, value: u32) {
        self.0[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, at: usize, value: u64) {
        self.0[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn bytes(&mut self, at: usize, value: &[u8]) {
        self.0[at..at + value.len()].copy_from_slice(value);
    }
}

/// A null-terminated string table, and where each name sits in it.
fn string_table(names: &[&str]) -> (Vec<u8>, Vec<u32>) {
    let mut blob = vec![0_u8]; // Index 0 is the empty name, by definition.
    let mut offsets = Vec::new();
    for name in names {
        offsets.push(u32::try_from(blob.len()).expect("fixtures stay small"));
        blob.extend_from_slice(name.as_bytes());
        blob.push(0);
    }
    (blob, offsets)
}

// ---------------------------------------------------------------- ELF -------

const ELF_BASE: u64 = 0x40_0000;

/// A 64-bit little-endian ELF: one loadable image carrying code, strings, a
/// symbol table, and a dynamic section naming one needed library.
///
/// The whole file is mapped by a single `PT_LOAD` at [`ELF_BASE`], so an
/// address is the base plus a file offset — which keeps the forging here
/// readable without making the file any less valid.
/// # Panics
///
/// Panics if a fixture is edited into something a header field of its format
/// cannot address. That is a mistake in the forging below, never in an input.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "forging a container format is one long sequence of field writes"
)]
pub fn elf_x86_64() -> Fixture {
    const HEADER: usize = 64;
    const PROGRAM_ENTRY: usize = 56;
    const SECTION_ENTRY: usize = 64;
    const SECTIONS: usize = 8;
    const SYMBOL_ENTRY: usize = 24;
    let library = "libfixture.so.1";

    let program_headers = HEADER;
    let section_headers = program_headers + PROGRAM_ENTRY * 2;
    let blobs = section_headers + SECTION_ENTRY * SECTIONS;

    // The section name table, whose own offsets the headers refer to.
    let (names, name_at) = string_table(&[
        ".text",
        ".rodata",
        ".dynstr",
        ".dynamic",
        ".symtab",
        ".strtab",
        ".shstrtab",
    ]);
    let (symbol_names, symbol_name_at) = string_table(&["main", "helper"]);
    let (dynamic_strings, dynamic_string_at) = string_table(&[library]);

    let mut file = vec![0_u8; blobs];
    let text_at = place(&mut file, X86_CODE);
    let rodata_at = place(&mut file, &strings_blob());
    let rodata_size = file.len() - rodata_at;
    let dynstr_at = place(&mut file, &dynamic_strings);

    // The dynamic section: what is needed, and where the names of the needed
    // things are. `DT_STRTAB` is an address, so it goes through the mapping.
    let mut dynamic = Vec::new();
    let mut entry = |tag: u64, value: u64| {
        dynamic.extend_from_slice(&tag.to_le_bytes());
        dynamic.extend_from_slice(&value.to_le_bytes());
    };
    entry(1, u64::from(dynamic_string_at[0])); // DT_NEEDED
    entry(5, ELF_BASE + dynstr_at as u64); // DT_STRTAB
    entry(10, dynamic_strings.len() as u64); // DT_STRSZ
    entry(0, 0); // DT_NULL
    let dynamic_at = place(&mut file, &dynamic);
    let dynamic_size = file.len() - dynamic_at;

    // The symbol table: a mandatory null entry, then one per function.
    let mut symbols = vec![0_u8; SYMBOL_ENTRY];
    for (index, (address, size)) in [
        (ELF_BASE + text_at as u64, X86_MAIN_SIZE),
        (ELF_BASE + text_at as u64 + X86_HELPER_AT, X86_HELPER_SIZE),
    ]
    .into_iter()
    .enumerate()
    {
        let mut symbol = vec![0_u8; SYMBOL_ENTRY];
        let mut write = Writer(&mut symbol);
        write.u32(0, symbol_name_at[index]);
        symbol[4] = 0x12; // STB_GLOBAL | STT_FUNC
        let mut write = Writer(&mut symbol);
        write.u16(6, 1); // Defined in .text, section index 1.
        write.u64(8, address);
        write.u64(16, size);
        symbols.extend_from_slice(&symbol);
    }
    let symtab_at = place(&mut file, &symbols);
    let strtab_at = place(&mut file, &symbol_names);
    let shstrtab_at = place(&mut file, &names);
    let total = file.len();

    let mut write = Writer(&mut file);
    write.bytes(0, b"\x7fELF");
    write.bytes(4, &[2, 1, 1, 0]); // 64-bit, little-endian, version 1
    write.u16(16, 2); // ET_EXEC
    write.u16(18, 62); // x86-64
    write.u32(20, 1);
    write.u64(24, ELF_BASE + text_at as u64); // e_entry
    write.u64(32, program_headers as u64);
    write.u64(40, section_headers as u64);
    write.u16(52, small(HEADER));
    write.u16(54, small(PROGRAM_ENTRY));
    write.u16(56, 2); // e_phnum
    write.u16(58, small(SECTION_ENTRY));
    write.u16(60, small(SECTIONS));
    write.u16(62, 7); // e_shstrndx -> .shstrtab

    // PT_LOAD over the whole file, then PT_DYNAMIC over the dynamic section.
    let mut program = |index: usize, kind: u32, offset: usize, size: usize, flags: u32| {
        let at = program_headers + PROGRAM_ENTRY * index;
        write.u32(at, kind);
        write.u32(at + 4, flags);
        write.u64(at + 8, offset as u64);
        write.u64(at + 16, ELF_BASE + offset as u64);
        write.u64(at + 24, ELF_BASE + offset as u64);
        write.u64(at + 32, size as u64);
        write.u64(at + 40, size as u64);
        write.u64(at + 48, 0x1000);
    };
    program(0, 1, 0, total, 5); // PT_LOAD, r-x
    program(1, 2, dynamic_at, dynamic_size, 6); // PT_DYNAMIC, rw-

    // Section 0 is the mandatory null entry, left zeroed.
    let mut section = |index: usize,
                       name: u32,
                       kind: u32,
                       flags: u64,
                       offset: usize,
                       size: usize,
                       link: u32,
                       entry_size: u64| {
        let at = section_headers + SECTION_ENTRY * index;
        write.u32(at, name);
        write.u32(at + 4, kind);
        write.u64(at + 8, flags);
        // Only allocated sections are mapped, and only those get an address.
        write.u64(
            at + 16,
            if flags & 2 == 0 {
                0
            } else {
                ELF_BASE + offset as u64
            },
        );
        write.u64(at + 24, offset as u64);
        write.u64(at + 32, size as u64);
        write.u32(at + 40, link);
        write.u64(at + 48, 1);
        write.u64(at + 56, entry_size);
    };
    section(1, name_at[0], 1, 0x6, text_at, X86_CODE.len(), 0, 0); // .text
    section(2, name_at[1], 1, 0x2, rodata_at, rodata_size, 0, 0); // .rodata
    section(
        3,
        name_at[2],
        3,
        0x2,
        dynstr_at,
        dynamic_strings.len(),
        0,
        0,
    ); // .dynstr
    section(4, name_at[3], 6, 0x3, dynamic_at, dynamic_size, 3, 16); // .dynamic
    section(5, name_at[4], 2, 0, symtab_at, symbols.len(), 6, 24); // .symtab
    section(6, name_at[5], 3, 0, strtab_at, symbol_names.len(), 0, 0); // .strtab
    section(7, name_at[6], 3, 0, shstrtab_at, names.len(), 0, 0); // .shstrtab

    Fixture {
        label: "ELF x86-64",
        functions: vec![
            ("main", ELF_BASE + text_at as u64),
            ("helper", ELF_BASE + text_at as u64 + X86_HELPER_AT),
        ],
        strings: STRINGS.to_vec(),
        libraries: vec![library],
        imported_functions: Vec::new(),
        entry_point: ELF_BASE + text_at as u64,
        bytes: file,
    }
}

// ----------------------------------------------------------------- PE -------

const PE_BASE: u64 = 0x1_4000_0000;

/// The functions the PE fixture asks each of its two libraries for.
///
/// The second is `ntdll.dll` on purpose: it is the one library whose import
/// list is the finding rather than a detail, and the reader treats it as such.
const PE_IMPORTS: &[&str] = &["FixtureEntry", "FixtureHelper"];
const PE_NATIVE_LIBRARY: &str = "ntdll.dll";
const PE_NATIVE_IMPORTS: &[&str] = &["NtCreateFile", "NtQuerySystemInformation"];
const PE_TEXT_RVA: u32 = 0x1000;
const PE_DATA_RVA: u32 = 0x2000;

/// A 64-bit PE with a code section and a data section holding its strings, an
/// export directory naming its two functions, and an import descriptor naming
/// the library it calls into.
///
/// Windows builds keep no COFF symbol table, so those two directories are
/// where a PE's names really live — which is what the reader looks at.
/// # Panics
///
/// Panics if a fixture is edited into something a header field of its format
/// cannot address. That is a mistake in the forging below, never in an input.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "forging a container format is one long sequence of field writes"
)]
pub fn pe_x86_64() -> Fixture {
    const SIGNATURE: usize = 0x80;
    const OPTIONAL_SIZE: usize = 0xf0;
    const SECTION_ENTRY: usize = 40;
    let optional = SIGNATURE + 24;
    let table = optional + OPTIONAL_SIZE;
    let headers_end = table + SECTION_ENTRY * 2;
    let library = "FIXTURE.dll";

    // The data section is assembled first: its contents are addressed by the
    // directories, so their offsets have to be known before it is written.
    let mut data = Vec::new();
    let data_rva = |data: &Vec<u8>| PE_DATA_RVA + u32::try_from(data.len()).expect("small");
    data.extend_from_slice(&strings_blob());

    let export_name_rva = data_rva(&data);
    data.extend_from_slice(b"fixture.exe\0");
    let main_name_rva = data_rva(&data);
    data.extend_from_slice(b"main\0");
    let helper_name_rva = data_rva(&data);
    data.extend_from_slice(b"helper\0");
    let library_rva = data_rva(&data);
    data.extend_from_slice(library.as_bytes());
    data.push(0);
    let native_library_rva = data_rva(&data);
    data.extend_from_slice(PE_NATIVE_LIBRARY.as_bytes());
    data.push(0);

    // Export directory: parallel name and ordinal tables, joined to the
    // address table through the ordinals.
    let functions_rva = data_rva(&data);
    let helper_rva = PE_TEXT_RVA + u32::try_from(X86_HELPER_AT).expect("fixtures stay small");
    for address in [PE_TEXT_RVA, helper_rva] {
        data.extend_from_slice(&address.to_le_bytes());
    }
    let names_rva = data_rva(&data);
    for name in [main_name_rva, helper_name_rva] {
        data.extend_from_slice(&name.to_le_bytes());
    }
    let ordinals_rva = data_rva(&data);
    for ordinal in [0_u16, 1] {
        data.extend_from_slice(&ordinal.to_le_bytes());
    }
    let export_rva = data_rva(&data);
    let mut export = vec![0_u8; 40];
    let mut write = Writer(&mut export);
    write.u32(12, export_name_rva);
    write.u32(16, 1); // Ordinal base.
    write.u32(20, 2); // Number of functions.
    write.u32(24, 2); // Number of names.
    write.u32(28, functions_rva);
    write.u32(32, names_rva);
    write.u32(36, ordinals_rva);
    data.extend_from_slice(&export);

    // Hint/name structures, then the import lookup table pointing at them: a
    // descriptor that named only its library left the reader with no way to
    // say what the program actually asks that library for.
    let lookup_table = |data: &mut Vec<u8>, names: &[&str]| {
        let mut hint_names = Vec::new();
        for name in names {
            hint_names.push(PE_DATA_RVA + u32::try_from(data.len()).expect("small"));
            data.extend_from_slice(&0_u16.to_le_bytes()); // Hint.
            data.extend_from_slice(name.as_bytes());
            data.push(0);
            if data.len() % 2 == 1 {
                data.push(0); // Each structure is two-byte aligned.
            }
        }
        let lookup_rva = PE_DATA_RVA + u32::try_from(data.len()).expect("small");
        for rva in &hint_names {
            data.extend_from_slice(&u64::from(*rva).to_le_bytes());
        }
        data.extend_from_slice(&0_u64.to_le_bytes()); // Terminating entry.
        lookup_rva
    };
    let lookup_rva = lookup_table(&mut data, PE_IMPORTS);
    let native_lookup_rva = lookup_table(&mut data, PE_NATIVE_IMPORTS);

    // Import directory: one descriptor per library, then the terminating
    // all-zero one.
    let import_rva = data_rva(&data);
    let mut import = vec![0_u8; 60];
    let mut write = Writer(&mut import);
    write.u32(0, lookup_rva); // Import lookup table.
    write.u32(12, library_rva); // Name of the library.
    write.u32(16, lookup_rva); // Import address table.
    write.u32(20, native_lookup_rva);
    write.u32(32, native_library_rva);
    write.u32(36, native_lookup_rva);
    data.extend_from_slice(&import);
    let export_size = u32::try_from(export.len()).expect("small");
    let data_size = u32::try_from(data.len()).expect("small");

    let mut file = vec![0_u8; headers_end];
    let text_at = place(&mut file, X86_CODE);
    // Sections are aligned on disk; the padding is what the loader expects and
    // what `offset_of` assumes when it maps an address back to the file.
    file.resize(text_at + 0x200, 0);
    let data_at = place(&mut file, &data);
    file.resize(data_at + 0x200, 0);

    let mut write = Writer(&mut file);
    write.bytes(0, b"MZ");
    write.u32(0x3c, u32::try_from(SIGNATURE).expect("small"));
    write.bytes(SIGNATURE, b"PE\0\0");
    write.u16(SIGNATURE + 4, 0x8664); // x86-64
    write.u16(SIGNATURE + 6, 2); // Two sections.
    write.u32(SIGNATURE + 8, 0x6000_0000); // Timestamp, any value.
    write.u16(SIGNATURE + 20, u16::try_from(OPTIONAL_SIZE).expect("small"));
    write.u16(SIGNATURE + 22, 0x0002); // EXECUTABLE_IMAGE

    write.u16(optional, 0x20b); // PE32+
    write.u32(optional + 16, PE_TEXT_RVA); // Entry point.
    write.u64(optional + 24, PE_BASE);
    write.u16(optional + 68, 3); // Console subsystem.
    write.u16(optional + 70, 0x4140); // DYNAMIC_BASE | NX_COMPAT | GUARD_CF
    let directories = optional + 112;
    write.u32(directories, export_rva);
    write.u32(directories + 4, export_size);
    write.u32(directories + 8, import_rva);
    write.u32(directories + 12, 40);

    let mut section = |index: usize, name: &[u8], rva: u32, at: usize, size: u32, flags: u32| {
        let header = table + SECTION_ENTRY * index;
        write.bytes(header, name);
        write.u32(header + 8, size); // Virtual size.
        write.u32(header + 12, rva);
        write.u32(header + 16, size); // Raw size.
        write.u32(header + 20, u32::try_from(at).expect("small"));
        write.u32(header + 36, flags);
    };
    section(
        0,
        b".text\0\0\0",
        PE_TEXT_RVA,
        text_at,
        u32::try_from(X86_CODE.len()).expect("small"),
        0x6000_0020, // CODE | EXECUTE | READ
    );
    section(
        1,
        b".rdata\0\0",
        PE_DATA_RVA,
        data_at,
        data_size,
        0x4000_0040, // INITIALIZED_DATA | READ
    );

    Fixture {
        label: "PE x86-64",
        functions: vec![
            ("main", PE_BASE + u64::from(PE_TEXT_RVA)),
            ("helper", PE_BASE + u64::from(PE_TEXT_RVA) + X86_HELPER_AT),
        ],
        strings: STRINGS.to_vec(),
        libraries: vec![library, PE_NATIVE_LIBRARY],
        imported_functions: PE_NATIVE_IMPORTS.to_vec(),
        entry_point: PE_BASE + u64::from(PE_TEXT_RVA),
        bytes: file,
    }
}

// -------------------------------------------------------------- Mach-O ------

const MACH_O_BASE: u64 = 0x1_0000_0000;

/// A 64-bit little-endian `AArch64` Mach-O: one `__TEXT` segment with code and
/// strings, an `LC_SYMTAB` naming both functions, and one `LC_LOAD_DYLIB`.
///
/// `AArch64` on purpose: it is the one architecture whose decoder never runs on
/// an x86 host otherwise, and Apple Silicon is where it matters.
/// # Panics
///
/// Panics if a fixture is edited into something a header field of its format
/// cannot address. That is a mistake in the forging below, never in an input.
#[must_use]
pub fn mach_o_arm64() -> Fixture {
    const HEADER: usize = 32;
    const SEGMENT_SIZE: usize = 72 + 80 * 2;
    const SYMTAB_SIZE: usize = 24;
    const MAIN_SIZE: usize = 24;
    const NLIST: usize = 16;
    let library = "/usr/lib/libfixture.dylib";
    let dylib_size = 24 + library.len() + 1;
    let dylib_size = dylib_size + (8 - dylib_size % 8) % 8; // Commands are aligned.

    let segment = HEADER;
    let text_section = segment + 72;
    let data_section = text_section + 80;
    let symtab = segment + SEGMENT_SIZE;
    let main = symtab + SYMTAB_SIZE;
    let dylib = main + MAIN_SIZE;
    let commands_end = dylib + dylib_size;

    let (name_blob, name_offsets) = string_table(&["_main", "_helper"]);

    let mut file = vec![0_u8; commands_end];
    let text_at = place(&mut file, ARM64_CODE);
    let strings_at = place(&mut file, &strings_blob());
    let strings_size = file.len() - strings_at;

    // One `nlist_64` per function: defined in a section, so `N_SECT`.
    let mut symbols = Vec::new();
    for (index, address) in [
        MACH_O_BASE + text_at as u64,
        MACH_O_BASE + text_at as u64 + ARM64_HELPER_AT,
    ]
    .into_iter()
    .enumerate()
    {
        let mut symbol = vec![0_u8; NLIST];
        let mut write = Writer(&mut symbol);
        write.u32(0, name_offsets[index]);
        symbol[4] = 0x0e; // N_SECT | N_EXT
        symbol[5] = 1; // Section 1, __text.
        let mut write = Writer(&mut symbol);
        write.u64(8, address);
        symbols.extend_from_slice(&symbol);
    }
    let nlist_at = place(&mut file, &symbols);
    let name_blob_at = place(&mut file, &name_blob);
    let total = file.len();

    let mut write = Writer(&mut file);
    write.bytes(0, &[0xcf, 0xfa, 0xed, 0xfe]); // 64-bit little-endian
    write.u32(4, 0x0100_000c); // ARM64
    write.u32(12, 2); // MH_EXECUTE
    write.u32(16, 4); // Four load commands.
    write.u32(20, u32::try_from(commands_end - HEADER).expect("small"));
    write.u32(24, 0x0020_0000); // MH_PIE

    write.u32(segment, 0x19); // LC_SEGMENT_64
    write.u32(segment + 4, u32::try_from(SEGMENT_SIZE).expect("small"));
    write.bytes(segment + 8, b"__TEXT\0\0");
    write.u64(segment + 24, MACH_O_BASE); // vmaddr
    write.u64(segment + 32, total as u64); // vmsize
    write.u64(segment + 48, total as u64); // filesize
    write.u32(segment + 56, 5); // maxprot r-x
    write.u32(segment + 60, 5); // initprot r-x
    write.u32(segment + 64, 2); // Two sections.

    write.bytes(text_section, b"__text\0\0\0\0\0\0\0\0\0\0");
    write.bytes(text_section + 16, b"__TEXT\0\0\0\0\0\0\0\0\0\0");
    write.u64(text_section + 32, MACH_O_BASE + text_at as u64);
    write.u64(text_section + 40, ARM64_CODE.len() as u64);
    write.u32(text_section + 48, u32::try_from(text_at).expect("small"));
    write.u32(text_section + 64, 0x8000_0400); // PURE_INSTRUCTIONS | SOME_INSTRUCTIONS

    write.bytes(data_section, b"__cstring\0\0\0\0\0\0\0");
    write.bytes(data_section + 16, b"__TEXT\0\0\0\0\0\0\0\0\0\0");
    write.u64(data_section + 32, MACH_O_BASE + strings_at as u64);
    write.u64(data_section + 40, strings_size as u64);
    write.u32(data_section + 48, u32::try_from(strings_at).expect("small"));
    write.u32(data_section + 64, 2); // S_CSTRING_LITERALS

    write.u32(symtab, 0x2); // LC_SYMTAB
    write.u32(symtab + 4, u32::try_from(SYMTAB_SIZE).expect("small"));
    write.u32(symtab + 8, u32::try_from(nlist_at).expect("small"));
    write.u32(symtab + 12, 2); // Two symbols.
    write.u32(symtab + 16, u32::try_from(name_blob_at).expect("small"));
    write.u32(symtab + 20, u32::try_from(name_blob.len()).expect("small"));

    write.u32(main, 0x8000_0028); // LC_MAIN
    write.u32(main + 4, u32::try_from(MAIN_SIZE).expect("small"));
    write.u64(main + 8, text_at as u64); // entryoff, from the file's start

    write.u32(dylib, 0xc); // LC_LOAD_DYLIB
    write.u32(dylib + 4, u32::try_from(dylib_size).expect("small"));
    write.u32(dylib + 8, 24); // Offset of the name inside the command.
    write.bytes(dylib + 24, library.as_bytes());

    Fixture {
        label: "Mach-O arm64",
        functions: vec![
            ("main", MACH_O_BASE + text_at as u64),
            ("helper", MACH_O_BASE + text_at as u64 + ARM64_HELPER_AT),
        ],
        strings: STRINGS.to_vec(),
        libraries: vec![library],
        imported_functions: Vec::new(),
        entry_point: MACH_O_BASE + text_at as u64,
        bytes: file,
    }
}

/// The endianness every fixture is written in.
pub const ORDER: Endianness = Endianness::Little;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyse_bytes;
    use std::path::Path;

    /// A fixture that does not actually carry what it claims would let a test
    /// pass for the wrong reason, so each one is held to its own declaration.
    #[test]
    fn every_fixture_carries_what_it_declares() {
        for fixture in all() {
            let label = fixture.label;
            let analysis = analyse_bytes(
                Path::new("fixture.bin"),
                fixture.bytes.len() as u64,
                &fixture.bytes,
            );

            assert_eq!(
                analysis.entry_point,
                Some(fixture.entry_point),
                "{label}: entry point"
            );
            assert!(
                analysis.executable_sections().count() >= 1,
                "{label}: no executable section"
            );
            assert!(
                !analysis.instructions.is_empty(),
                "{label}: nothing decoded"
            );

            for (name, address) in &fixture.functions {
                let symbol = analysis
                    .symbols
                    .iter()
                    .find(|symbol| symbol.name == *name)
                    .unwrap_or_else(|| panic!("{label}: {name} is not in the symbol table"));
                assert_eq!(symbol.address, Some(*address), "{label}: address of {name}");
                assert!(!symbol.imported, "{label}: {name} is defined here");
                assert!(
                    analysis.instruction_at(*address).is_some(),
                    "{label}: no instruction decoded at {name}"
                );
            }

            for text in &fixture.strings {
                assert!(
                    analysis
                        .strings
                        .iter()
                        .any(|found| found.value.contains(text)),
                    "{label}: string {text:?} was not extracted"
                );
            }
            for library in &fixture.libraries {
                assert!(
                    analysis
                        .details
                        .linked_libraries
                        .iter()
                        .any(|found| found.contains(library.rsplit('/').next().unwrap_or(library))),
                    "{label}: library {library:?} was not read, found {:?}",
                    analysis.details.linked_libraries
                );
            }
            for function in &fixture.imported_functions {
                assert!(
                    analysis
                        .details
                        .imports
                        .iter()
                        .any(|entry| entry.functions.iter().any(|found| found == function)),
                    "{label}: imported function {function:?} was not read, found {:?}",
                    analysis.details.imports
                );
            }
        }
    }

    /// The point of the fixtures: three formats, and an architecture the host
    /// does not have to provide.
    #[test]
    fn the_fixtures_cover_the_formats_the_host_does_not() {
        use crate::{Architecture, BinaryFormat};

        let formats: Vec<(BinaryFormat, Architecture)> = all()
            .into_iter()
            .map(|fixture| {
                let analysis = analyse_bytes(
                    Path::new("fixture.bin"),
                    fixture.bytes.len() as u64,
                    &fixture.bytes,
                );
                (analysis.summary.format, analysis.summary.architecture)
            })
            .collect();

        assert!(formats.iter().any(|(format, architecture)| matches!(
            format,
            BinaryFormat::Elf { .. }
        ) && *architecture
            == Architecture::X86_64));
        assert!(
            formats
                .iter()
                .any(|(format, _)| matches!(format, BinaryFormat::Pe))
        );
        assert!(formats.iter().any(|(format, architecture)| matches!(
            format,
            BinaryFormat::MachO { .. }
        ) && *architecture
            == Architecture::Arm64));
    }
}
