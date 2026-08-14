//! PE optional header, import table and security flags.
//!
//! Windows records its mitigations as `DllCharacteristics` bits, and names the
//! libraries it needs in an import table addressed by RVA — an address in the
//! mapped image, which the section table converts back to a file offset.

use super::{BinaryDetails, FileKind, MAXIMUM_ENTRIES};
use crate::{
    analysis::sections::{self, Section},
    binary::{BinaryFormat, Endianness},
    bytes::{read_c_string, read_u16, read_u32},
};

/// Everything in a PE file is little-endian.
const ORDER: Endianness = Endianness::Little;

/// Offsets from the PE signature.
const TIMESTAMP: usize = 8;
const CHARACTERISTICS: usize = 22;
const OPTIONAL_HEADER: usize = 24;

/// `Characteristics`.
const IMAGE_FILE_DLL: u16 = 0x2000;
const IMAGE_FILE_EXECUTABLE: u16 = 0x0002;

/// Offsets inside the optional header.
const MAGIC: usize = 0;
const PE32_PLUS: u16 = 0x20b;
const SUBSYSTEM: usize = 68;
const DLL_CHARACTERISTICS: usize = 70;
/// The data directory array starts after the size fields, whose width differs
/// between the 32-bit and 64-bit optional headers.
const DATA_DIRECTORY_32: usize = 96;
const DATA_DIRECTORY_64: usize = 112;

/// `DllCharacteristics`.
const DYNAMIC_BASE: u16 = 0x0040;
const FORCE_INTEGRITY: u16 = 0x0080;
const NX_COMPAT: u16 = 0x0100;
const GUARD_CF: u16 = 0x4000;

/// Data directory indices.
const IMPORT_DIRECTORY: usize = 1;
const CERTIFICATE_DIRECTORY: usize = 4;
const DIRECTORY_ENTRY_SIZE: usize = 8;

/// One import descriptor, whose `Name` field holds the library name's RVA.
const IMPORT_DESCRIPTOR_SIZE: usize = 20;
const IMPORT_NAME: usize = 12;
const MAXIMUM_NAME: usize = 256;

pub fn details(file: &[u8]) -> BinaryDetails {
    let Some(signature) = sections::pe_signature(file) else {
        return BinaryDetails::default();
    };
    let optional = signature + OPTIONAL_HEADER;
    let sections = sections::parse(file, BinaryFormat::Pe);

    let characteristics = read_u16(file, signature + CHARACTERISTICS, ORDER).unwrap_or(0);
    let dll_characteristics = read_u16(file, optional + DLL_CHARACTERISTICS, ORDER);

    let mut details = BinaryDetails {
        file_kind: file_kind(characteristics),
        bits: if read_u16(file, optional + MAGIC, ORDER) == Some(PE32_PLUS) {
            64
        } else {
            32
        },
        endianness: ORDER,
        linked_libraries: imports(file, signature, optional, &sections),
        subsystem: subsystem(read_u16(file, optional + SUBSYSTEM, ORDER)),
        timestamp: read_u32(file, signature + TIMESTAMP, ORDER).filter(|stamp| *stamp != 0),
        ..BinaryDetails::default()
    };

    details.hardening.position_independent =
        dll_characteristics.map(|flags| flags & DYNAMIC_BASE != 0);
    details.hardening.address_space_randomisation =
        dll_characteristics.map(|flags| flags & DYNAMIC_BASE != 0);
    details.hardening.data_execution_prevention =
        dll_characteristics.map(|flags| flags & NX_COMPAT != 0);
    details.hardening.control_flow_guard = dll_characteristics.map(|flags| flags & GUARD_CF != 0);
    details.hardening.signed = Some(
        has_certificate(file, optional)
            || dll_characteristics.is_some_and(|flags| flags & FORCE_INTEGRITY != 0),
    );
    details
}

const fn file_kind(characteristics: u16) -> FileKind {
    if characteristics & IMAGE_FILE_DLL != 0 {
        FileKind::SharedLibrary
    } else if characteristics & IMAGE_FILE_EXECUTABLE != 0 {
        FileKind::Executable
    } else {
        FileKind::ObjectFile
    }
}

const fn subsystem(value: Option<u16>) -> Option<&'static str> {
    match value {
        Some(1) => Some("Native"),
        Some(2) => Some("Windows GUI"),
        Some(3) => Some("Windows console"),
        Some(7) => Some("POSIX console"),
        Some(9) => Some("Windows CE GUI"),
        Some(10) => Some("EFI application"),
        Some(11) => Some("EFI boot service driver"),
        Some(12) => Some("EFI runtime driver"),
        Some(13) => Some("EFI ROM"),
        Some(14) => Some("Xbox"),
        Some(16) => Some("Windows boot application"),
        _ => None,
    }
}

fn data_directory(file: &[u8], optional: usize, index: usize) -> Option<(u32, u32)> {
    let base = if read_u16(file, optional + MAGIC, ORDER)? == PE32_PLUS {
        DATA_DIRECTORY_64
    } else {
        DATA_DIRECTORY_32
    };
    let entry = optional
        .checked_add(base)?
        .checked_add(index.checked_mul(DIRECTORY_ENTRY_SIZE)?)?;
    Some((
        read_u32(file, entry, ORDER)?,
        read_u32(file, entry + 4, ORDER)?,
    ))
}

/// The certificate directory is one of the rare fields holding a file offset
/// rather than an RVA.
fn has_certificate(file: &[u8], optional: usize) -> bool {
    data_directory(file, optional, CERTIFICATE_DIRECTORY)
        .is_some_and(|(offset, size)| offset != 0 && size != 0)
}

/// Walks the import descriptors, collecting the name of each library.
fn imports(file: &[u8], signature: usize, optional: usize, sections: &[Section]) -> Vec<String> {
    let Some((address, _)) = data_directory(file, optional, IMPORT_DIRECTORY) else {
        return Vec::new();
    };
    // Section addresses already include the image base, so the directory's
    // RVA has to be raised to the same absolute space before lookup.
    let base = sections::pe_image_base(file, signature).unwrap_or(0);
    let Some(table) = file_offset_of(sections, base + u64::from(address)) else {
        return Vec::new();
    };

    let mut libraries = Vec::new();
    for index in 0..MAXIMUM_ENTRIES {
        let Some(descriptor) = usize::try_from(table)
            .ok()
            .and_then(|table| table.checked_add(index.checked_mul(IMPORT_DESCRIPTOR_SIZE)?))
        else {
            break;
        };
        // The table ends with an all-zero descriptor.
        let Some(name_address) = read_u32(file, descriptor + IMPORT_NAME, ORDER) else {
            break;
        };
        if name_address == 0 {
            break;
        }
        let name = file_offset_of(sections, base + u64::from(name_address))
            .and_then(|offset| usize::try_from(offset).ok())
            .and_then(|offset| read_c_string(file, offset, MAXIMUM_NAME))
            .filter(|name| !name.is_empty());
        match name {
            Some(name) => libraries.push(name),
            None => break,
        }
    }
    libraries
}

/// Converts an address in the mapped image to a file offset, using the section
/// that covers it.
fn file_offset_of(sections: &[Section], address: u64) -> Option<u64> {
    sections.iter().find_map(|section| {
        let start = section.virtual_address;
        let end = start.checked_add(section.virtual_size.max(section.file_size))?;
        (start..end)
            .contains(&address)
            .then(|| section.file_offset + (address - start))
    })
}
