//! ELF program headers and dynamic section.
//!
//! Sections describe the file for the linker; program headers describe it for
//! the loader. Hardening lives almost entirely in the latter.

use super::{BinaryDetails, FileKind, MAXIMUM_ENTRIES, Relro, Segment};
use crate::{
    analysis::sections::{Permissions, read_word},
    binary::Endianness,
    bytes::{read_c_string, read_slice, read_u16, read_u32},
};

/// Offsets inside the ELF header for the 32-bit and 64-bit layouts.
struct Layout {
    program_header_offset: usize,
    program_header_size: usize,
    program_header_count: usize,
    /// Offsets inside one program header entry.
    flags: usize,
    offset: usize,
    virtual_address: usize,
    file_size: usize,
    memory_size: usize,
}

const ELF32: Layout = Layout {
    program_header_offset: 28,
    program_header_size: 42,
    program_header_count: 44,
    flags: 24,
    offset: 4,
    virtual_address: 8,
    file_size: 16,
    memory_size: 20,
};

const ELF64: Layout = Layout {
    program_header_offset: 32,
    program_header_size: 54,
    program_header_count: 56,
    flags: 4,
    offset: 8,
    virtual_address: 16,
    file_size: 32,
    memory_size: 40,
};

/// `e_type`.
const ET_REL: u16 = 1;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const ET_CORE: u16 = 4;
const E_TYPE_OFFSET: usize = 16;

/// `p_type`.
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_NOTE: u32 = 4;
const PT_PHDR: u32 = 6;
const PT_TLS: u32 = 7;
const PT_GNU_EH_FRAME: u32 = 0x6474_e550;
const PT_GNU_STACK: u32 = 0x6474_e551;
const PT_GNU_RELRO: u32 = 0x6474_e552;
const PT_GNU_PROPERTY: u32 = 0x6474_e553;

/// `p_flags`.
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

/// `d_tag` entries of the dynamic section.
const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_STRTAB: u64 = 5;
const DT_STRSZ: u64 = 10;
const DT_BIND_NOW: u64 = 24;
const DT_FLAGS: u64 = 30;
const DT_FLAGS_1: u64 = 0x6fff_fffb;
const DF_BIND_NOW: u64 = 0x8;
const DF_1_NOW: u64 = 0x1;

const MAXIMUM_NAME: usize = 256;

const fn layout(bits: u8) -> &'static Layout {
    if bits == 64 { &ELF64 } else { &ELF32 }
}

pub fn details(file: &[u8], bits: u8, endianness: Endianness) -> BinaryDetails {
    let headers = program_headers(file, bits, endianness);
    let dynamic = Dynamic::read(file, bits, endianness, &headers);

    let mut details = BinaryDetails {
        bits,
        endianness,
        file_kind: file_kind(file, endianness, &headers),
        interpreter: interpreter(file, &headers),
        segments: headers.iter().map(ProgramHeader::to_segment).collect(),
        linked_libraries: dynamic.needed.clone(),
        ..BinaryDetails::default()
    };

    details.hardening.position_independent =
        read_u16(file, E_TYPE_OFFSET, endianness).map(|kind| kind == ET_DYN);
    details.hardening.non_executable_stack = headers
        .iter()
        .find(|header| header.kind == PT_GNU_STACK)
        .map(|header| header.flags & PF_X == 0);
    details.hardening.relro = Some(relro(&headers, &dynamic));
    details
}

/// Distinguishes a position-independent executable from a shared library:
/// both are `ET_DYN`, only the executable names an interpreter.
fn file_kind(file: &[u8], endianness: Endianness, headers: &[ProgramHeader]) -> FileKind {
    let has_interpreter = headers.iter().any(|header| header.kind == PT_INTERP);
    match read_u16(file, E_TYPE_OFFSET, endianness) {
        Some(ET_EXEC) => FileKind::Executable,
        Some(ET_DYN) if has_interpreter => FileKind::Executable,
        Some(ET_DYN) => FileKind::SharedLibrary,
        Some(ET_REL) => FileKind::ObjectFile,
        Some(ET_CORE) => FileKind::CoreDump,
        _ => FileKind::Unknown,
    }
}

fn interpreter(file: &[u8], headers: &[ProgramHeader]) -> Option<String> {
    let header = headers.iter().find(|header| header.kind == PT_INTERP)?;
    let offset = usize::try_from(header.file_offset).ok()?;
    let length = usize::try_from(header.file_size).ok()?.min(MAXIMUM_NAME);
    read_c_string(file, offset, length).filter(|path| !path.is_empty())
}

/// Full RELRO needs both the protected region and eager binding; with only the
/// region, the PLT stays writable.
fn relro(headers: &[ProgramHeader], dynamic: &Dynamic) -> Relro {
    if !headers.iter().any(|header| header.kind == PT_GNU_RELRO) {
        return Relro::None;
    }
    if dynamic.bind_now {
        Relro::Full
    } else {
        Relro::Partial
    }
}

struct ProgramHeader {
    kind: u32,
    flags: u32,
    file_offset: u64,
    file_size: u64,
    virtual_address: u64,
    memory_size: u64,
}

impl ProgramHeader {
    fn to_segment(&self) -> Segment {
        Segment {
            kind: kind_label(self.kind),
            virtual_address: self.virtual_address,
            virtual_size: self.memory_size,
            file_offset: self.file_offset,
            file_size: self.file_size,
            permissions: Permissions {
                read: self.flags & PF_R != 0,
                write: self.flags & PF_W != 0,
                execute: self.flags & PF_X != 0,
            },
        }
    }
}

fn kind_label(kind: u32) -> String {
    match kind {
        PT_LOAD => "LOAD".to_owned(),
        PT_DYNAMIC => "DYNAMIC".to_owned(),
        PT_INTERP => "INTERP".to_owned(),
        PT_NOTE => "NOTE".to_owned(),
        PT_PHDR => "PHDR".to_owned(),
        PT_TLS => "TLS".to_owned(),
        PT_GNU_EH_FRAME => "GNU_EH_FRAME".to_owned(),
        PT_GNU_STACK => "GNU_STACK".to_owned(),
        PT_GNU_RELRO => "GNU_RELRO".to_owned(),
        PT_GNU_PROPERTY => "GNU_PROPERTY".to_owned(),
        other => format!("{other:#x}"),
    }
}

fn program_headers(file: &[u8], bits: u8, endianness: Endianness) -> Vec<ProgramHeader> {
    let layout = layout(bits);
    let Some(start) = read_word(file, layout.program_header_offset, bits, endianness)
        .and_then(|offset| usize::try_from(offset).ok())
    else {
        return Vec::new();
    };
    let (Some(entry_size), Some(count)) = (
        read_u16(file, layout.program_header_size, endianness).map(usize::from),
        read_u16(file, layout.program_header_count, endianness).map(usize::from),
    ) else {
        return Vec::new();
    };
    if entry_size == 0 {
        return Vec::new();
    }

    (0..count.min(MAXIMUM_ENTRIES))
        .filter_map(|index| {
            let header = start.checked_add(index.checked_mul(entry_size)?)?;
            let read =
                |offset: usize| read_word(file, header.checked_add(offset)?, bits, endianness);
            Some(ProgramHeader {
                kind: read_u32(file, header, endianness)?,
                flags: read_u32(file, header.checked_add(layout.flags)?, endianness)?,
                file_offset: read(layout.offset)?,
                file_size: read(layout.file_size)?,
                virtual_address: read(layout.virtual_address)?,
                memory_size: read(layout.memory_size)?,
            })
        })
        .collect()
}

/// The entries of `PT_DYNAMIC` that matter here.
#[derive(Default)]
struct Dynamic {
    needed: Vec<String>,
    bind_now: bool,
}

impl Dynamic {
    fn read(file: &[u8], bits: u8, endianness: Endianness, headers: &[ProgramHeader]) -> Self {
        let Some(header) = headers.iter().find(|header| header.kind == PT_DYNAMIC) else {
            return Self::default();
        };
        let Ok(start) = usize::try_from(header.file_offset) else {
            return Self::default();
        };
        let entry_size = if bits == 64 { 16 } else { 8 };
        let entries = usize::try_from(header.file_size).unwrap_or(0) / entry_size.max(1);

        // Two passes: the string table location may be declared after the
        // entries that reference it.
        let mut pairs = Vec::new();
        for index in 0..entries.min(MAXIMUM_ENTRIES) {
            let Some(entry) = start.checked_add(index * entry_size) else {
                break;
            };
            let (Some(tag), Some(value)) = (
                read_word(file, entry, bits, endianness),
                read_word(file, entry + entry_size / 2, bits, endianness),
            ) else {
                break;
            };
            if tag == DT_NULL {
                break;
            }
            pairs.push((tag, value));
        }

        let strings = string_table(file, headers, &pairs);
        let bind_now = pairs.iter().any(|(tag, value)| match *tag {
            DT_BIND_NOW => true,
            DT_FLAGS => value & DF_BIND_NOW != 0,
            DT_FLAGS_1 => value & DF_1_NOW != 0,
            _ => false,
        });
        let needed = pairs
            .iter()
            .filter(|(tag, _)| *tag == DT_NEEDED)
            .filter_map(|(_, value)| {
                let offset = usize::try_from(*value).ok()?;
                read_c_string(strings?, offset, MAXIMUM_NAME)
            })
            .filter(|name| !name.is_empty())
            .collect();

        Self { needed, bind_now }
    }
}

/// Locates `.dynstr`, which `DT_NEEDED` entries index into. Its address is
/// virtual, so it goes through the load mapping to become a file offset.
fn string_table<'a>(
    file: &'a [u8],
    headers: &[ProgramHeader],
    entries: &[(u64, u64)],
) -> Option<&'a [u8]> {
    let find = |wanted: u64| {
        entries
            .iter()
            .find(|(tag, _)| *tag == wanted)
            .map(|(_, value)| *value)
    };
    let address = find(DT_STRTAB)?;
    let size = find(DT_STRSZ)?;

    let offset = file_offset_of(headers, address)?;
    read_slice(
        file,
        usize::try_from(offset).ok()?,
        usize::try_from(size).ok()?,
    )
}

/// Converts a virtual address to a file offset using the `PT_LOAD` mapping.
fn file_offset_of(headers: &[ProgramHeader], address: u64) -> Option<u64> {
    headers
        .iter()
        .filter(|header| header.kind == PT_LOAD && header.file_size > 0)
        .find_map(|header| {
            let end = header.virtual_address.checked_add(header.file_size)?;
            (header.virtual_address..end)
                .contains(&address)
                .then(|| header.file_offset + (address - header.virtual_address))
        })
}
