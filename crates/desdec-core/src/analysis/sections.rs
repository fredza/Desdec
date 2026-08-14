//! Structured section and segment tables for ELF, PE and Mach-O.
//!
//! The three formats describe the same idea in three layouts: named regions,
//! each mapped at a virtual address with its own permissions. This module
//! normalises them into one [`Section`] type so the rest of the tool never has
//! to branch on the container format.
//!
//! Every field is read through the bounds-checked helpers in
//! [`crate::bytes`], and every table walk is capped, so a malformed or hostile
//! header yields a short list rather than a hang or a panic.

use crate::{
    binary::{BinaryFormat, Endianness},
    bytes::{read_c_string, read_padded_name, read_slice, read_u16, read_u32, read_u64},
};

/// Upper bound on the number of sections reported. Real binaries stay far
/// below; a corrupted count would otherwise drive a very long loop.
const MAXIMUM_SECTIONS_U32: u32 = 4096;
const MAXIMUM_SECTIONS: usize = MAXIMUM_SECTIONS_U32 as usize;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Permissions {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl Permissions {
    /// Compact `rwx` notation, with `-` for each missing right.
    #[must_use]
    pub fn label(self) -> String {
        let flags = [(self.read, 'r'), (self.write, 'w'), (self.execute, 'x')];
        flags
            .into_iter()
            .map(|(granted, letter)| if granted { letter } else { '-' })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub name: String,
    /// Where the section is mapped at run time. `0` when it is not mapped.
    pub virtual_address: u64,
    /// Where its bytes start in the file, if it has any.
    pub file_offset: u64,
    /// Size once mapped, which exceeds `file_size` for zero-filled sections.
    pub virtual_size: u64,
    /// Bytes actually stored in the file: `0` for `.bss` and friends.
    pub file_size: u64,
    pub permissions: Permissions,
    /// Shannon entropy of the stored bytes, `None` when the section stores
    /// none or its range falls outside the analysed part of the file.
    pub entropy: Option<f32>,
}

impl Section {
    /// Whether the section is actually mapped into memory at run time.
    ///
    /// Sections that only serve the linker or a debugger — `.shstrtab`,
    /// `.symtab`, `.debug_*` — carry no rights and are stored with a virtual
    /// address of zero. Treating those as living at address 0 would make every
    /// address lookup match the wrong region.
    #[must_use]
    pub const fn is_mapped(&self) -> bool {
        let permissions = self.permissions;
        (permissions.read || permissions.write || permissions.execute) && self.virtual_size > 0
    }

    /// Bytes this section occupies in the file, when they are all present.
    #[must_use]
    pub fn bytes_in<'a>(&self, file: &'a [u8]) -> Option<&'a [u8]> {
        let offset = usize::try_from(self.file_offset).ok()?;
        let length = usize::try_from(self.file_size).ok()?;
        read_slice(file, offset, length)
    }
}

/// Reads the section table, or an empty list when the format is unknown or the
/// headers are unusable.
#[must_use]
pub fn parse(file: &[u8], format: BinaryFormat) -> Vec<Section> {
    let mut sections = match format {
        BinaryFormat::Elf { bits, endianness } => elf::sections(file, bits, endianness),
        BinaryFormat::Pe => pe::sections(file),
        BinaryFormat::MachO { bits, endianness } => mach_o::sections(file, bits, endianness),
        BinaryFormat::Unknown => Vec::new(),
    };

    for section in &mut sections {
        section.entropy = section.bytes_in(file).and_then(super::entropy::shannon);
    }
    sections
}

/// Locates the PE signature, shared with the details parser.
pub(super) fn pe_signature(file: &[u8]) -> Option<usize> {
    pe::signature(file)
}

/// Preferred load address of a PE image, shared with the details parser.
pub(super) fn pe_image_base(file: &[u8], signature: usize) -> Option<u64> {
    pe::image_base(file, signature)
}

/// Walks Mach-O load commands, shared with the details parser.
pub(super) fn for_each_mach_o_command(
    file: &[u8],
    bits: u8,
    endianness: Endianness,
    visit: impl FnMut(u32, usize) -> bool,
) {
    mach_o::for_each_command(file, bits, endianness, visit);
}

/// Virtual address the process starts executing at, when the format states it.
#[must_use]
pub fn entry_point(file: &[u8], format: BinaryFormat) -> Option<u64> {
    match format {
        BinaryFormat::Elf { bits, endianness } => elf::entry_point(file, bits, endianness),
        BinaryFormat::Pe => pe::entry_point(file),
        BinaryFormat::MachO { bits, endianness } => mach_o::entry_point(file, bits, endianness),
        BinaryFormat::Unknown => None,
    }
}

/// Reads a pointer-sized field: 4 bytes on a 32-bit container, 8 on a 64-bit one.
pub(super) fn read_word(
    file: &[u8],
    offset: usize,
    bits: u8,
    endianness: Endianness,
) -> Option<u64> {
    if bits == 64 {
        read_u64(file, offset, endianness)
    } else {
        read_u32(file, offset, endianness).map(u64::from)
    }
}

mod elf {
    use super::{
        Endianness, MAXIMUM_SECTIONS, Permissions, Section, read_c_string, read_u16, read_u32,
        read_word,
    };

    /// Offsets inside the ELF header, for the 32-bit and 64-bit layouts.
    struct Layout {
        entry: usize,
        section_header_offset: usize,
        section_header_size: usize,
        section_count: usize,
        name_table_index: usize,
        /// Offsets inside one section header entry.
        flags: usize,
        address: usize,
        offset: usize,
        size: usize,
    }

    const ELF32: Layout = Layout {
        entry: 24,
        section_header_offset: 32,
        section_header_size: 46,
        section_count: 48,
        name_table_index: 50,
        flags: 8,
        address: 12,
        offset: 16,
        size: 20,
    };

    const ELF64: Layout = Layout {
        entry: 24,
        section_header_offset: 40,
        section_header_size: 58,
        section_count: 60,
        name_table_index: 62,
        flags: 8,
        address: 16,
        offset: 24,
        size: 32,
    };

    /// `sh_type`: occupies no space in the file, such as `.bss`.
    const SHT_NOBITS: u32 = 8;

    /// `sh_flags`.
    const SHF_WRITE: u64 = 0x1;
    const SHF_ALLOC: u64 = 0x2;
    const SHF_EXECINSTR: u64 = 0x4;

    /// Longest section name read from the string table.
    const MAXIMUM_NAME: usize = 256;

    const fn layout(bits: u8) -> &'static Layout {
        if bits == 64 { &ELF64 } else { &ELF32 }
    }

    pub fn entry_point(file: &[u8], bits: u8, endianness: Endianness) -> Option<u64> {
        let entry = read_word(file, layout(bits).entry, bits, endianness)?;
        (entry != 0).then_some(entry)
    }

    pub fn sections(file: &[u8], bits: u8, endianness: Endianness) -> Vec<Section> {
        let layout = layout(bits);
        let Some(table) = table_position(file, layout, bits, endianness) else {
            return Vec::new();
        };

        let names = name_table(file, layout, &table, bits, endianness);
        (0..table.count)
            .filter_map(|index| {
                let header = table
                    .start
                    .checked_add(index.checked_mul(table.entry_size)?)?;
                read_section(file, layout, header, bits, endianness, names)
            })
            .collect()
    }

    struct Table {
        start: usize,
        entry_size: usize,
        count: usize,
        name_table_index: usize,
    }

    fn table_position(
        file: &[u8],
        layout: &Layout,
        bits: u8,
        endianness: Endianness,
    ) -> Option<Table> {
        let start = usize::try_from(read_word(
            file,
            layout.section_header_offset,
            bits,
            endianness,
        )?)
        .ok()?;
        let entry_size = read_u16(file, layout.section_header_size, endianness)? as usize;
        let count = read_u16(file, layout.section_count, endianness)? as usize;
        let name_table_index = read_u16(file, layout.name_table_index, endianness)? as usize;

        (entry_size > 0 && count > 0).then_some(Table {
            start,
            entry_size,
            count: count.min(MAXIMUM_SECTIONS),
            name_table_index,
        })
    }

    /// Locates `.shstrtab`, the table holding every section name.
    fn name_table<'a>(
        file: &'a [u8],
        layout: &Layout,
        table: &Table,
        bits: u8,
        endianness: Endianness,
    ) -> Option<&'a [u8]> {
        let header = table
            .start
            .checked_add(table.name_table_index.checked_mul(table.entry_size)?)?;
        let offset =
            usize::try_from(read_word(file, header + layout.offset, bits, endianness)?).ok()?;
        let size =
            usize::try_from(read_word(file, header + layout.size, bits, endianness)?).ok()?;
        crate::bytes::read_slice(file, offset, size)
    }

    fn read_section(
        file: &[u8],
        layout: &Layout,
        header: usize,
        bits: u8,
        endianness: Endianness,
        names: Option<&[u8]>,
    ) -> Option<Section> {
        let name_offset = read_u32(file, header, endianness)? as usize;
        let kind = read_u32(file, header + 4, endianness)?;
        let flags = read_word(file, header + layout.flags, bits, endianness)?;
        let address = read_word(file, header + layout.address, bits, endianness)?;
        let offset = read_word(file, header + layout.offset, bits, endianness)?;
        let size = read_word(file, header + layout.size, bits, endianness)?;

        let name = names
            .and_then(|names| read_c_string(names, name_offset, MAXIMUM_NAME))
            .unwrap_or_default();
        // A NOBITS section is mapped but stores nothing on disk.
        let file_size = if kind == SHT_NOBITS { 0 } else { size };

        Some(Section {
            name,
            virtual_address: address,
            file_offset: offset,
            virtual_size: size,
            file_size,
            permissions: Permissions {
                read: flags & SHF_ALLOC != 0,
                write: flags & SHF_WRITE != 0,
                execute: flags & SHF_EXECINSTR != 0,
            },
            entropy: None,
        })
    }
}

mod pe {
    use super::{
        Endianness, MAXIMUM_SECTIONS, Permissions, Section, read_padded_name, read_u16, read_u32,
        read_u64,
    };

    /// Everything in a PE file is little-endian.
    const ORDER: Endianness = Endianness::Little;

    /// `e_lfanew`: where the DOS header points to the PE signature.
    const SIGNATURE_POINTER: usize = 0x3c;
    /// Bytes from the signature to the COFF fields that follow it.
    const SECTION_COUNT: usize = 6;
    const OPTIONAL_HEADER_SIZE: usize = 20;
    const OPTIONAL_HEADER: usize = 24;

    /// `Magic` distinguishing the 32-bit and 64-bit optional headers.
    const PE32_PLUS: u16 = 0x20b;
    /// Offsets inside the optional header.
    const ENTRY_POINT: usize = 16;
    const IMAGE_BASE_32: usize = 28;
    const IMAGE_BASE_64: usize = 24;

    /// One section header entry.
    const ENTRY_SIZE: usize = 40;
    const NAME_WIDTH: usize = 8;
    const VIRTUAL_SIZE: usize = 8;
    const VIRTUAL_ADDRESS: usize = 12;
    const RAW_SIZE: usize = 16;
    const RAW_POINTER: usize = 20;
    const CHARACTERISTICS: usize = 36;

    const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
    const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;
    const IMAGE_SCN_MEM_WRITE: u32 = 0x8000_0000;

    pub(super) fn signature(file: &[u8]) -> Option<usize> {
        let offset = usize::try_from(read_u32(file, SIGNATURE_POINTER, ORDER)?).ok()?;
        (crate::bytes::read_slice(file, offset, 4)? == b"PE\0\0").then_some(offset)
    }

    /// The base address the image prefers to be loaded at; section addresses
    /// are stored relative to it.
    pub(super) fn image_base(file: &[u8], signature: usize) -> Option<u64> {
        let optional = signature.checked_add(OPTIONAL_HEADER)?;
        if read_u16(file, optional, ORDER)? == PE32_PLUS {
            read_u64(file, optional + IMAGE_BASE_64, ORDER)
        } else {
            read_u32(file, optional + IMAGE_BASE_32, ORDER).map(u64::from)
        }
    }

    pub fn entry_point(file: &[u8]) -> Option<u64> {
        let signature = signature(file)?;
        let optional = signature.checked_add(OPTIONAL_HEADER)?;
        let relative = read_u32(file, optional + ENTRY_POINT, ORDER)?;
        (relative != 0).then(|| image_base(file, signature).unwrap_or(0) + u64::from(relative))
    }

    pub fn sections(file: &[u8]) -> Vec<Section> {
        let Some(signature) = signature(file) else {
            return Vec::new();
        };
        let base = image_base(file, signature).unwrap_or(0);

        let Some(table) = table_position(file, signature) else {
            return Vec::new();
        };
        (0..table.count)
            .filter_map(|index| {
                let header = table.start.checked_add(index.checked_mul(ENTRY_SIZE)?)?;
                read_section(file, header, base)
            })
            .collect()
    }

    struct Table {
        start: usize,
        count: usize,
    }

    fn table_position(file: &[u8], signature: usize) -> Option<Table> {
        let count = read_u16(file, signature.checked_add(SECTION_COUNT)?, ORDER)? as usize;
        let optional_size =
            read_u16(file, signature.checked_add(OPTIONAL_HEADER_SIZE)?, ORDER)? as usize;
        let start = signature
            .checked_add(OPTIONAL_HEADER)?
            .checked_add(optional_size)?;

        (count > 0).then_some(Table {
            start,
            count: count.min(MAXIMUM_SECTIONS),
        })
    }

    fn read_section(file: &[u8], header: usize, base: u64) -> Option<Section> {
        let name = read_padded_name(file, header, NAME_WIDTH)?;
        let virtual_size = read_u32(file, header + VIRTUAL_SIZE, ORDER)?;
        let relative_address = read_u32(file, header + VIRTUAL_ADDRESS, ORDER)?;
        let raw_size = read_u32(file, header + RAW_SIZE, ORDER)?;
        let raw_pointer = read_u32(file, header + RAW_POINTER, ORDER)?;
        let characteristics = read_u32(file, header + CHARACTERISTICS, ORDER)?;

        Some(Section {
            name,
            virtual_address: base + u64::from(relative_address),
            file_offset: u64::from(raw_pointer),
            virtual_size: u64::from(virtual_size),
            file_size: u64::from(raw_size),
            permissions: Permissions {
                read: characteristics & IMAGE_SCN_MEM_READ != 0,
                write: characteristics & IMAGE_SCN_MEM_WRITE != 0,
                execute: characteristics & IMAGE_SCN_MEM_EXECUTE != 0,
            },
            entropy: None,
        })
    }
}

mod mach_o {
    use super::{
        Endianness, MAXIMUM_SECTIONS, MAXIMUM_SECTIONS_U32, Permissions, Section, read_padded_name,
        read_u32, read_u64, read_word,
    };

    /// Where the list of load commands begins, after the header.
    const COMMANDS_32: usize = 28;
    const COMMANDS_64: usize = 32;
    /// `ncmds`, the number of load commands.
    const COMMAND_COUNT: usize = 16;

    const LC_SEGMENT: u32 = 0x1;
    const LC_SEGMENT_64: u32 = 0x19;
    const LC_MAIN: u32 = 0x8000_0028;

    /// Smallest meaningful load command: `cmd` and `cmdsize` alone.
    const COMMAND_HEADER: usize = 8;
    const NAME_WIDTH: usize = 16;

    /// Offsets inside a segment command, and the size of one section entry.
    struct Layout {
        virtual_address: usize,
        virtual_size: usize,
        file_offset: usize,
        file_size: usize,
        protection: usize,
        section_count: usize,
        first_section: usize,
        section_size: usize,
        section_address: usize,
        section_length: usize,
        section_offset: usize,
    }

    const SEGMENT_32: Layout = Layout {
        virtual_address: 24,
        virtual_size: 28,
        file_offset: 32,
        file_size: 36,
        protection: 44,
        section_count: 48,
        first_section: 56,
        section_size: 68,
        section_address: 32,
        section_length: 36,
        section_offset: 40,
    };

    const SEGMENT_64: Layout = Layout {
        virtual_address: 24,
        virtual_size: 32,
        file_offset: 40,
        file_size: 48,
        protection: 60,
        section_count: 64,
        first_section: 72,
        section_size: 80,
        section_address: 32,
        section_length: 40,
        section_offset: 48,
    };

    /// `initprot` bits.
    const VM_PROT_READ: u32 = 1;
    const VM_PROT_WRITE: u32 = 2;
    const VM_PROT_EXECUTE: u32 = 4;

    const fn layout(bits: u8) -> &'static Layout {
        if bits == 64 { &SEGMENT_64 } else { &SEGMENT_32 }
    }

    const fn commands_start(bits: u8) -> usize {
        if bits == 64 { COMMANDS_64 } else { COMMANDS_32 }
    }

    /// Walks the load commands, handing each one to `visit` until it stops.
    pub(super) fn for_each_command(
        file: &[u8],
        bits: u8,
        endianness: Endianness,
        mut visit: impl FnMut(u32, usize) -> bool,
    ) {
        let Some(count) = read_u32(file, COMMAND_COUNT, endianness) else {
            return;
        };
        let mut offset = commands_start(bits);

        for _ in 0..count.min(MAXIMUM_SECTIONS_U32) {
            let (Some(command), Some(size)) = (
                read_u32(file, offset, endianness),
                read_u32(file, offset + 4, endianness),
            ) else {
                return;
            };
            // A command smaller than its own header would never advance.
            let Ok(size) = usize::try_from(size) else {
                return;
            };
            if size < COMMAND_HEADER {
                return;
            }
            if !visit(command, offset) {
                return;
            }
            let Some(next) = offset.checked_add(size) else {
                return;
            };
            offset = next;
        }
    }

    pub fn entry_point(file: &[u8], bits: u8, endianness: Endianness) -> Option<u64> {
        let layout = layout(bits);
        let wanted_segment = if bits == 64 {
            LC_SEGMENT_64
        } else {
            LC_SEGMENT
        };
        let mut file_offset = None;
        let mut text_base = None;

        for_each_command(file, bits, endianness, |command, offset| {
            if command == LC_MAIN {
                file_offset = read_u64(file, offset + COMMAND_HEADER, endianness);
            } else if command == wanted_segment && text_base.is_none() {
                text_base = text_segment_base(file, layout, offset, bits, endianness);
            }
            // Both may appear in either order, so keep walking until we have
            // what we need.
            file_offset.is_none() || text_base.is_none()
        });

        // `entryoff` counts from the start of the file. The segment that maps
        // file offset zero — `__TEXT` — turns it into a virtual address. Note
        // this is the segment's base, not its first section's.
        Some(text_base.unwrap_or(0).saturating_add(file_offset?))
    }

    /// Virtual address of a segment, when that segment maps the start of the
    /// file and therefore anchors file offsets.
    fn text_segment_base(
        file: &[u8],
        layout: &Layout,
        command: usize,
        bits: u8,
        endianness: Endianness,
    ) -> Option<u64> {
        let maps_file_start = read_word(file, command + layout.file_offset, bits, endianness)? == 0;
        let has_content = read_word(file, command + layout.file_size, bits, endianness)? > 0;
        (maps_file_start && has_content)
            .then(|| read_word(file, command + layout.virtual_address, bits, endianness))
            .flatten()
    }

    pub fn sections(file: &[u8], bits: u8, endianness: Endianness) -> Vec<Section> {
        let layout = layout(bits);
        let wanted = if bits == 64 {
            LC_SEGMENT_64
        } else {
            LC_SEGMENT
        };
        let mut sections = Vec::new();

        for_each_command(file, bits, endianness, |command, offset| {
            if command == wanted {
                read_segment(file, layout, offset, bits, endianness, &mut sections);
            }
            sections.len() < MAXIMUM_SECTIONS
        });
        sections
    }

    /// Reads one segment and the sections it contains.
    ///
    /// A segment with no sections — `__PAGEZERO`, for instance — is reported as
    /// a region of its own, so the mapping stays complete.
    fn read_segment(
        file: &[u8],
        layout: &Layout,
        command: usize,
        bits: u8,
        endianness: Endianness,
        sections: &mut Vec<Section>,
    ) {
        let Some(name) = read_padded_name(file, command + COMMAND_HEADER, NAME_WIDTH) else {
            return;
        };
        let protection = read_u32(file, command + layout.protection, endianness).unwrap_or(0);
        let permissions = Permissions {
            read: protection & VM_PROT_READ != 0,
            write: protection & VM_PROT_WRITE != 0,
            execute: protection & VM_PROT_EXECUTE != 0,
        };
        let count =
            read_u32(file, command + layout.section_count, endianness).unwrap_or(0) as usize;

        if count == 0 {
            let read = |offset: usize| read_word(file, command + offset, bits, endianness);
            sections.push(Section {
                name,
                virtual_address: read(layout.virtual_address).unwrap_or(0),
                file_offset: read(layout.file_offset).unwrap_or(0),
                virtual_size: read(layout.virtual_size).unwrap_or(0),
                file_size: read(layout.file_size).unwrap_or(0),
                permissions,
                entropy: None,
            });
            return;
        }

        for index in 0..count.min(MAXIMUM_SECTIONS) {
            let Some(header) = command.checked_add(layout.first_section).and_then(|start| {
                index
                    .checked_mul(layout.section_size)
                    .and_then(|shift| start.checked_add(shift))
            }) else {
                return;
            };
            if let Some(section) = read_section(file, layout, header, bits, endianness, permissions)
            {
                sections.push(section);
            }
        }
    }

    fn read_section(
        file: &[u8],
        layout: &Layout,
        header: usize,
        bits: u8,
        endianness: Endianness,
        permissions: Permissions,
    ) -> Option<Section> {
        // Sections carry their own short name plus the segment they belong to.
        let section_name = read_padded_name(file, header, NAME_WIDTH)?;
        let segment_name = read_padded_name(file, header + NAME_WIDTH, NAME_WIDTH)?;
        let address = read_word(file, header + layout.section_address, bits, endianness)?;
        let size = read_word(file, header + layout.section_length, bits, endianness)?;
        let offset = read_u32(file, header + layout.section_offset, endianness)?;

        Some(Section {
            name: format!("{segment_name},{section_name}"),
            virtual_address: address,
            file_offset: u64::from(offset),
            virtual_size: size,
            file_size: size,
            permissions,
            entropy: None,
        })
    }
}
