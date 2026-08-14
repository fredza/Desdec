//! Mach-O header flags, segments and linked dylibs.

use super::{BinaryDetails, FileKind, Segment};
use crate::{
    analysis::sections::{self, Permissions, read_word},
    binary::Endianness,
    bytes::{read_c_string, read_padded_name, read_u32},
};

/// `filetype`.
const FILE_TYPE_OFFSET: usize = 12;
const MH_OBJECT: u32 = 1;
const MH_EXECUTE: u32 = 2;
const MH_CORE: u32 = 4;
const MH_DYLIB: u32 = 6;
const MH_BUNDLE: u32 = 8;

/// `flags`.
const FLAGS_OFFSET: usize = 24;
const MH_PIE: u32 = 0x0020_0000;

/// Load commands read here.
const LC_SEGMENT: u32 = 0x1;
const LC_SEGMENT_64: u32 = 0x19;
const LC_LOAD_DYLIB: u32 = 0xc;
const LC_LOAD_WEAK_DYLIB: u32 = 0x8000_0018;
const LC_CODE_SIGNATURE: u32 = 0x1d;

const COMMAND_HEADER: usize = 8;
const NAME_WIDTH: usize = 16;
const MAXIMUM_NAME: usize = 256;

/// Offsets inside a segment command.
struct Layout {
    virtual_address: usize,
    virtual_size: usize,
    file_offset: usize,
    file_size: usize,
    protection: usize,
}

const SEGMENT_32: Layout = Layout {
    virtual_address: 24,
    virtual_size: 28,
    file_offset: 32,
    file_size: 36,
    protection: 44,
};

const SEGMENT_64: Layout = Layout {
    virtual_address: 24,
    virtual_size: 32,
    file_offset: 40,
    file_size: 48,
    protection: 60,
};

/// `initprot` bits.
const VM_PROT_READ: u32 = 1;
const VM_PROT_WRITE: u32 = 2;
const VM_PROT_EXECUTE: u32 = 4;

const fn layout(bits: u8) -> &'static Layout {
    if bits == 64 { &SEGMENT_64 } else { &SEGMENT_32 }
}

pub fn details(file: &[u8], bits: u8, endianness: Endianness) -> BinaryDetails {
    let layout = layout(bits);
    let wanted_segment = if bits == 64 {
        LC_SEGMENT_64
    } else {
        LC_SEGMENT
    };

    let mut segments = Vec::new();
    let mut libraries = Vec::new();
    let mut signed = false;

    sections::for_each_mach_o_command(file, bits, endianness, |command, offset| {
        match command {
            _ if command == wanted_segment => {
                if let Some(segment) = read_segment(file, layout, offset, bits, endianness) {
                    segments.push(segment);
                }
            }
            LC_LOAD_DYLIB | LC_LOAD_WEAK_DYLIB => {
                if let Some(name) = dylib_name(file, offset, endianness) {
                    libraries.push(name);
                }
            }
            LC_CODE_SIGNATURE => signed = true,
            _ => {}
        }
        true
    });

    let flags = read_u32(file, FLAGS_OFFSET, endianness);
    let mut details = BinaryDetails {
        file_kind: file_kind(read_u32(file, FILE_TYPE_OFFSET, endianness)),
        bits,
        endianness,
        segments,
        linked_libraries: libraries,
        ..BinaryDetails::default()
    };
    details.hardening.position_independent = flags.map(|flags| flags & MH_PIE != 0);
    details.hardening.signed = Some(signed);
    details
}

const fn file_kind(value: Option<u32>) -> FileKind {
    match value {
        Some(MH_EXECUTE) => FileKind::Executable,
        Some(MH_DYLIB) => FileKind::SharedLibrary,
        Some(MH_OBJECT) => FileKind::ObjectFile,
        Some(MH_CORE) => FileKind::CoreDump,
        Some(MH_BUNDLE) => FileKind::Bundle,
        _ => FileKind::Unknown,
    }
}

fn read_segment(
    file: &[u8],
    layout: &Layout,
    command: usize,
    bits: u8,
    endianness: Endianness,
) -> Option<Segment> {
    let name = read_padded_name(file, command + COMMAND_HEADER, NAME_WIDTH)?;
    let read = |offset: usize| read_word(file, command.checked_add(offset)?, bits, endianness);
    let protection = read_u32(file, command + layout.protection, endianness).unwrap_or(0);

    Some(Segment {
        kind: name,
        virtual_address: read(layout.virtual_address)?,
        virtual_size: read(layout.virtual_size)?,
        file_offset: read(layout.file_offset)?,
        file_size: read(layout.file_size)?,
        permissions: Permissions {
            read: protection & VM_PROT_READ != 0,
            write: protection & VM_PROT_WRITE != 0,
            execute: protection & VM_PROT_EXECUTE != 0,
        },
    })
}

/// A dylib command stores its name inline, at an offset counted from the start
/// of the command itself.
fn dylib_name(file: &[u8], command: usize, endianness: Endianness) -> Option<String> {
    let name_offset = read_u32(file, command + COMMAND_HEADER, endianness)? as usize;
    let start = command.checked_add(name_offset)?;
    read_c_string(file, start, MAXIMUM_NAME).filter(|name| !name.is_empty())
}
