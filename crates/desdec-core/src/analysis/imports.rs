//! Where each imported name is expected to be filled in.
//!
//! A call into another file does not go straight there. It reads its target
//! from a slot, and a loader writes the real address into that slot before the
//! program runs. Desdec has no loader, so the slots hold zero and a call
//! through one lands on the first page — which the emulator reports, and could
//! only ever report as "a call into another file", since the address it left
//! for is zero whatever the call was to.
//!
//! The file itself says which name each slot belongs to, and that is what is
//! read here: an ELF states it in its relocations, a PE in the thunk arrays of
//! its import directory, and a Mach-O in the indirect symbol table its pointer
//! sections index into. The three are the same fact in three shapes — an
//! address and the name whose address belongs there — so all of them are
//! reported as one list, keyed by the address the call reads.
//!
//! A Mach-O states it one of two ways depending on how new it is — an indirect
//! symbol table, or chained fixups, where the pointers themselves are the
//! chain — and both are read. What a format states some other way still is not,
//! and the answer is then simply shorter: an ELF whose relocations are
//! reachable through `PT_DYNAMIC` alone leaves those slots unnamed rather than
//! wrongly named.

use crate::{
    binary::{BinaryFormat, Endianness},
    bytes::{read_c_string, read_slice, read_u16, read_u32, read_u64},
};

use super::symbols::PeHeaders;

/// An address a call reads its target from, and the name that belongs there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportSlot {
    /// Where the target is read from, as the image is mapped.
    pub address: u64,
    /// The imported name whose address the loader would write there.
    pub name: String,
}

/// Most slots read from one file, so a malformed table cannot grow without
/// bound. Larger than any real import table: a heavy desktop binary names a
/// few thousand.
const MAXIMUM_SLOTS: usize = 20_000;

/// Bounds on the chained-fixups walk, so a corrupted header cannot drive a
/// long loop: a real file stays far below each of them.
const MAXIMUM_SEGMENTS: usize = 4096;
const MAXIMUM_PAGES: u64 = 65_535;
const MAXIMUM_CHAINS: usize = 4096;
/// A chain runs through one page, so it cannot be longer than a large page
/// holds links.
const MAXIMUM_LINKS: usize = 16_384;

#[must_use]
pub fn extract(file: &[u8], format: BinaryFormat) -> Vec<ImportSlot> {
    let mut slots = match format {
        BinaryFormat::Elf { bits, endianness } => elf(file, bits, endianness),
        BinaryFormat::Pe => pe(file),
        BinaryFormat::MachO { bits, endianness } => mach_o(file, bits, endianness),
        BinaryFormat::Unknown => Vec::new(),
    };
    // One answer per address, in one order, whatever order the tables were in.
    slots.sort_by(|a, b| a.address.cmp(&b.address).then(a.name.cmp(&b.name)));
    slots.dedup_by(|a, b| a.address == b.address);
    slots
}

/// ELF relocations, which name the slot and the symbol together.
///
/// Every relocation that names a symbol is reported, not only the jump slots
/// of the procedure linkage table: a call may go through the global offset
/// table directly — which is what a `call *0x…(%rip)` against a `GLOB_DAT`
/// relocation is — and that slot is as unfilled as any other.
fn elf(file: &[u8], bits: u8, order: Endianness) -> Vec<ImportSlot> {
    /// `SHT_RELA`, whose entries carry an addend, and `SHT_REL`, whose do not.
    const RELOCATION_KINDS: [u32; 2] = [4, 9];

    let (table_at, entry_at, count_at, header_size) = if bits == 64 {
        (40, 58, 60, 64)
    } else {
        (32, 46, 48, 40)
    };
    let table = word(file, table_at, bits, order).and_then(|v| usize::try_from(v).ok());
    let entry_size = read_u16(file, entry_at, order).map(usize::from);
    let count = read_u16(file, count_at, order).map(usize::from);
    let (Some(table), Some(entry_size), Some(count)) = (table, entry_size, count) else {
        return Vec::new();
    };
    if entry_size < header_size {
        return Vec::new();
    }
    // Offsets come from the file and are added saturatingly throughout: a
    // header naming a table near the top of the address space must not
    // overflow the addition before the bounds check can refuse the read.
    let header_of = |index: usize| table.checked_add(index.saturating_mul(entry_size));

    let mut out = Vec::new();
    for index in 0..count.min(4096) {
        let Some(header) = header_of(index) else {
            break;
        };
        if !read_u32(file, header.saturating_add(4), order)
            .is_some_and(|kind| RELOCATION_KINDS.contains(&kind))
        {
            continue;
        }
        let at = |offset64: usize, offset32: usize| {
            header.saturating_add(if bits == 64 { offset64 } else { offset32 })
        };
        // `sh_link` on a relocation section is the symbol table its entries
        // index into, and `sh_link` on that one is the strings the names live
        // in. Followed rather than assumed: a file may carry several.
        let symbols = read_u32(file, at(40, 24), order).map(|v| v as usize);
        let offset = word(file, at(24, 16), bits, order).and_then(|v| usize::try_from(v).ok());
        let size = word(file, at(32, 20), bits, order).and_then(|v| usize::try_from(v).ok());
        let step = word(file, at(56, 36), bits, order).and_then(|v| usize::try_from(v).ok());
        let (Some(symbols), Some(offset), Some(size), Some(step)) = (symbols, offset, size, step)
        else {
            continue;
        };
        let Some((table_of_symbols, symbol_size, strings)) =
            symbol_table(file, table, entry_size, symbols, bits, order)
        else {
            continue;
        };
        if step == 0 || symbol_size == 0 {
            continue;
        }

        let info_at = if bits == 64 { 8 } else { 4 };
        for n in 0..(size / step).min(MAXIMUM_SLOTS.saturating_sub(out.len())) {
            let Some(entry) = offset.checked_add(n.saturating_mul(step)) else {
                break;
            };
            let Some(address) = word(file, entry, bits, order) else {
                continue;
            };
            let Some(info) = word(file, entry.saturating_add(info_at), bits, order) else {
                continue;
            };
            // The symbol index is the top half of `r_info` on a 64-bit file
            // and its top three bytes on a 32-bit one; the rest is the
            // relocation type, which says how to write the address rather
            // than whose it is.
            let symbol = if bits == 64 { info >> 32 } else { info >> 8 };
            let Ok(symbol) = usize::try_from(symbol) else {
                continue;
            };
            // Index zero is the undefined symbol: a relocation naming it, such
            // as a `RELATIVE` one, is an address the file computes for itself
            // and not an import at all.
            if symbol == 0 {
                continue;
            }
            let Some(record) = table_of_symbols.checked_add(symbol.saturating_mul(symbol_size))
            else {
                continue;
            };
            let Some(name_at) = read_u32(file, record, order).map(|v| v as usize) else {
                continue;
            };
            let Some(name) = read_c_string(strings, name_at, 512).filter(|name| !name.is_empty())
            else {
                continue;
            };
            out.push(ImportSlot { address, name });
        }
    }
    out
}

/// The symbol table a relocation section points at: where its records start,
/// how long each is, and the strings their names live in.
fn symbol_table(
    file: &[u8],
    table: usize,
    entry_size: usize,
    index: usize,
    bits: u8,
    order: Endianness,
) -> Option<(usize, usize, &[u8])> {
    let header = table.checked_add(index.saturating_mul(entry_size))?;
    let at = |offset64: usize, offset32: usize| {
        header.saturating_add(if bits == 64 { offset64 } else { offset32 })
    };
    let offset = word(file, at(24, 16), bits, order).and_then(|v| usize::try_from(v).ok())?;
    let size = word(file, at(56, 36), bits, order).and_then(|v| usize::try_from(v).ok())?;
    let strings_index = read_u32(file, at(40, 24), order)? as usize;

    let strings_header = table.checked_add(strings_index.saturating_mul(entry_size))?;
    let string_at = |offset64: usize, offset32: usize| {
        strings_header.saturating_add(if bits == 64 { offset64 } else { offset32 })
    };
    let strings_offset =
        word(file, string_at(24, 16), bits, order).and_then(|v| usize::try_from(v).ok())?;
    let strings_size =
        word(file, string_at(32, 20), bits, order).and_then(|v| usize::try_from(v).ok())?;
    let strings = read_slice(file, strings_offset, strings_size)?;
    Some((offset, size, strings))
}

/// PE import thunks, whose array is the table of slots itself.
///
/// A descriptor names two parallel arrays: the lookup table, which says what
/// each entry is an import of, and the address table, which is where the
/// loader writes and therefore where a call reads. Walking them together is
/// what pairs a name with the address it belongs at.
fn pe(file: &[u8]) -> Vec<ImportSlot> {
    let order = Endianness::Little; // PE is little-endian on every target.
    let Some(headers) = PeHeaders::read(file) else {
        return Vec::new();
    };
    let Some((address, _)) = headers.directory(file, order, 1) else {
        return Vec::new();
    };
    let Some(table) = headers.offset_of(address) else {
        return Vec::new();
    };

    let step = if headers.wide() { 8 } else { 4 };
    let mut out = Vec::new();
    // The descriptor array ends at an all-zero entry rather than a count.
    for index in 0_usize..4096 {
        let Some(entry) = table.checked_add(index.saturating_mul(20)) else {
            return out;
        };
        // Where the loader writes, which is what a call reads. Without it
        // there is nothing to key a name by, however well the names read.
        let Some(first_thunk) =
            read_u32(file, entry.saturating_add(16), order).filter(|value| *value != 0)
        else {
            return out; // The terminating entry.
        };
        // The lookup table says what each entry imports. Some linkers leave it
        // out, in which case the address table holds those records too — until
        // a loader overwrites them, which has not happened to a file on disk.
        let names = read_u32(file, entry, order)
            .filter(|value| *value != 0)
            .unwrap_or(first_thunk);
        let Some(mut thunk) = headers.offset_of(names) else {
            continue;
        };

        let mut slot = headers.image_base().saturating_add(u64::from(first_thunk));
        for _ in 0..MAXIMUM_SLOTS {
            if out.len() >= MAXIMUM_SLOTS {
                return out;
            }
            let value = if headers.wide() {
                read_u64(file, thunk, order)
            } else {
                read_u32(file, thunk, order).map(u64::from)
            };
            let Some(value) = value.filter(|value| *value != 0) else {
                break; // End of this library's list.
            };
            // The top bit means the import is by ordinal, which names nothing.
            let by_ordinal = value & (1 << (step * 8 - 1)) != 0;
            if !by_ordinal
                && let Ok(rva) = u32::try_from(value & 0x7fff_ffff)
                && let Some(at) = headers.offset_of(rva)
                && let Some(name) =
                    read_c_string(file, at.saturating_add(2), 512).filter(|n| !n.is_empty())
            {
                out.push(ImportSlot {
                    address: slot,
                    name,
                });
            }
            let (Some(next), Some(next_slot)) =
                (thunk.checked_add(step), slot.checked_add(step as u64))
            else {
                return out;
            };
            thunk = next;
            slot = next_slot;
        }
    }
    out
}

/// What it takes to name a Mach-O slot, gathered from the load commands.
struct MachOTables<'a> {
    /// Where the symbol records start.
    symbols: usize,
    /// The blob their names live in.
    strings: &'a [u8],
    /// Where the indirect symbol table starts, and how many entries it has.
    indirect: (usize, usize),
    /// Every section of pointers: its address, its length, and where its own
    /// run begins in the indirect table.
    sections: Vec<(u64, u64, usize)>,
}

/// Reads the three tables a Mach-O names its slots through.
///
/// Gathered in one walk and joined afterwards rather than resolved in place:
/// the load commands come in whatever order the linker wrote them, and the
/// symbol table may well be named after the sections that index into it.
fn mach_o_tables(file: &[u8], bits: u8, order: Endianness) -> Option<MachOTables<'_>> {
    const LC_SEGMENT: u32 = 0x1;
    const LC_SEGMENT_64: u32 = 0x19;
    const LC_SYMTAB: u32 = 0x2;
    const LC_DYSYMTAB: u32 = 0xb;
    /// `S_NON_LAZY_SYMBOL_POINTERS` and `S_LAZY_SYMBOL_POINTERS`, the two
    /// section types whose contents are addresses a call reads through.
    const POINTER_SECTIONS: [u32; 2] = [6, 7];
    /// The low byte of `flags` says what kind of section it is; the rest are
    /// attributes.
    const SECTION_TYPE: u32 = 0xff;
    /// Upper bound on the sections read from one segment, as elsewhere: a
    /// corrupted count must not drive a very long loop.
    const MAXIMUM_SECTIONS: u32 = 4096;

    let (first_section, section_size, count_at) = if bits == 64 {
        (72_usize, 80_usize, 64_usize)
    } else {
        (56, 68, 48)
    };
    let (length_at, flags_at, reserved_at) = if bits == 64 {
        (40_usize, 64_usize, 68_usize)
    } else {
        (36, 56, 60)
    };

    let mut symbols = None;
    let mut strings = None;
    let mut indirect = None;
    let mut sections: Vec<(u64, u64, usize)> = Vec::new();
    super::sections::for_each_mach_o_command(file, bits, order, |command, at| {
        match command {
            LC_SYMTAB => {
                symbols = read_u32(file, at + 8, order).map(|v| v as usize);
                let blob = read_u32(file, at + 16, order).map(|v| v as usize);
                let length = read_u32(file, at + 20, order).map(|v| v as usize);
                strings = blob
                    .zip(length)
                    .and_then(|(at, length)| read_slice(file, at, length));
            }
            LC_DYSYMTAB => {
                let table = read_u32(file, at + 56, order).map(|v| v as usize);
                let count = read_u32(file, at + 60, order).map(|v| v as usize);
                indirect = table.zip(count);
            }
            LC_SEGMENT | LC_SEGMENT_64 => {
                let Some(count) = read_u32(file, at + count_at, order) else {
                    return true;
                };
                for index in 0..count.min(MAXIMUM_SECTIONS) as usize {
                    let Some(section) =
                        at.checked_add(first_section + index.saturating_mul(section_size))
                    else {
                        break;
                    };
                    let kind = read_u32(file, section + flags_at, order).map(|f| f & SECTION_TYPE);
                    if !kind.is_some_and(|kind| POINTER_SECTIONS.contains(&kind)) {
                        continue;
                    }
                    let address = word(file, section + 32, bits, order);
                    let size = word(file, section + length_at, bits, order);
                    let start = read_u32(file, section + reserved_at, order).map(|v| v as usize);
                    if let (Some(address), Some(size), Some(start)) = (address, size, start) {
                        sections.push((address, size, start));
                    }
                }
            }
            _ => {}
        }
        true
    });

    Some(MachOTables {
        symbols: symbols?,
        strings: strings?,
        indirect: indirect?,
        sections,
    })
}

/// Mach-O slots, named the two ways a Mach-O can name them.
///
/// A file states its fixups one way or the other depending on how new it is,
/// and a few state some of both, so both are read and the answers joined:
/// [`extract`] keeps one name per address, and the two agree where they meet.
fn mach_o(file: &[u8], bits: u8, order: Endianness) -> Vec<ImportSlot> {
    let mut out = mach_o_indirect(file, bits, order);
    out.extend(mach_o_chained(file, order));
    out
}

/// Mach-O pointer sections, named through the indirect symbol table.
///
/// A Mach-O states this in three pieces, and the walk is what joins them. A
/// section of pointers says, in `reserved1`, where its own run begins in the
/// indirect symbol table; each entry of that table is an index into the symbol
/// table; and the symbol there carries the name. The slots themselves are the
/// section's own addresses, one pointer apart.
///
/// Both kinds of pointer section are read — lazy and non-lazy — since a call
/// reads through either. Stubs are not: a stub is code to be called, not a
/// place a target is read from, and it reaches its own pointer section anyway.
fn mach_o_indirect(file: &[u8], bits: u8, order: Endianness) -> Vec<ImportSlot> {
    /// Indirect entries that name no import: a local address, or an absolute
    /// one the loader leaves alone.
    const INDIRECT_LOCAL: u32 = 0x8000_0000;
    const INDIRECT_ABSOLUTE: u32 = 0x4000_0000;

    let Some(tables) = mach_o_tables(file, bits, order) else {
        return Vec::new();
    };
    let (indirect, indirect_count) = tables.indirect;
    let pointer = if bits == 64 { 8_usize } else { 4 };
    let record = if bits == 64 { 16_usize } else { 12 };

    let mut out = Vec::new();
    for (address, size, start) in tables.sections {
        let slots = usize::try_from(size / pointer as u64).unwrap_or(0);
        for index in 0..slots.min(MAXIMUM_SLOTS.saturating_sub(out.len())) {
            let Some(entry) = start.checked_add(index).filter(|at| *at < indirect_count) else {
                break;
            };
            let Some(symbol) = read_u32(file, indirect.saturating_add(entry * 4), order) else {
                break;
            };
            if symbol & (INDIRECT_LOCAL | INDIRECT_ABSOLUTE) != 0 {
                continue; // Not an import: an address the file already holds.
            }
            let Some(at) = tables
                .symbols
                .checked_add((symbol as usize).saturating_mul(record))
            else {
                continue;
            };
            let Some(name_at) = read_u32(file, at, order).map(|v| v as usize) else {
                continue;
            };
            // Mach-O prefixes C symbols with an underscore the source never
            // had; the other two formats report source names, so it is
            // stripped here as it is where the symbols are read.
            let Some(name) = read_c_string(tables.strings, name_at, 512)
                .map(|name| name.strip_prefix('_').unwrap_or(&name).to_owned())
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            out.push(ImportSlot {
                address: address.saturating_add((index * pointer) as u64),
                name,
            });
        }
    }
    out
}

/// How one link of a chain of fixups is read.
///
/// The words themselves are bitfields, and which bits mean what depends on the
/// pointer format the segment declares. Only the fields that say "this link
/// binds an import", "which import", and "where the next link is" are read;
/// the rest — the addend, the authentication bits — say how to write the
/// address, which is a loader's business and not a reader's.
struct ChainFormat {
    /// What a step of `next` is worth, in bytes.
    stride: u64,
    /// Whether a link is eight bytes rather than four.
    wide: bool,
    next_shift: u32,
    next_mask: u64,
    bind_bit: u32,
    ordinal_mask: u64,
}

/// The formats that carry binds. The cache and firmware ones are left out:
/// they hold rebases only, so there is no import in them to name.
const fn chain_format(format: u16) -> Option<ChainFormat> {
    match format {
        // ARM64E, in its three userland flavours: links eight bytes apart.
        1 | 7 | 9 => Some(ChainFormat {
            stride: 8,
            wide: true,
            next_shift: 51,
            next_mask: 0x7ff,
            bind_bit: 62,
            ordinal_mask: 0xffff,
        }),
        // The same, with an ordinal wide enough for a large program.
        12 => Some(ChainFormat {
            stride: 8,
            wide: true,
            next_shift: 51,
            next_mask: 0x7ff,
            bind_bit: 62,
            ordinal_mask: 0x00ff_ffff,
        }),
        // Plain 64-bit, and the same counted from the image's base.
        2 | 6 => Some(ChainFormat {
            stride: 4,
            wide: true,
            next_shift: 51,
            next_mask: 0xfff,
            bind_bit: 63,
            ordinal_mask: 0x00ff_ffff,
        }),
        3 => Some(ChainFormat {
            stride: 4,
            wide: false,
            next_shift: 26,
            next_mask: 0x1f,
            bind_bit: 31,
            ordinal_mask: 0x000f_ffff,
        }),
        _ => None,
    }
}

/// Where the imports of a chained-fixups blob are, and how they are written.
struct ChainedImports {
    /// Where the blob begins in the file. Every offset inside it is from here.
    base: usize,
    imports: usize,
    format: u32,
    count: usize,
    symbols: usize,
}

impl ChainedImports {
    /// The name an ordinal stands for, as the source wrote it.
    fn name(&self, file: &[u8], order: Endianness, ordinal: u64) -> Option<String> {
        let index = usize::try_from(ordinal)
            .ok()
            .filter(|at| *at < self.count)?;
        // Three shapes, differing in what they carry beside the name: the name
        // itself is a bitfield of the first word in all of them.
        let (record, at) = match self.format {
            1 => (4_usize, 9_u32),
            2 => (8, 9),
            3 => (16, 32),
            _ => return None,
        };
        let entry = self
            .base
            .checked_add(self.imports)?
            .checked_add(index * record)?;
        let name_offset = if self.format == 3 {
            read_u64(file, entry, order)? >> at
        } else {
            u64::from(read_u32(file, entry, order)? >> at)
        };
        let at = self
            .base
            .checked_add(self.symbols)?
            .checked_add(usize::try_from(name_offset).ok()?)?;
        read_c_string(file, at, 512)
            .map(|name| name.strip_prefix('_').unwrap_or(&name).to_owned())
            .filter(|name| !name.is_empty())
    }
}

/// Mach-O slots named by chained fixups, which is how a recent build states
/// them.
///
/// Instead of a table of relocations, the pointers themselves hold the chain:
/// each word says how far the next fixup is, so a page's worth of them is a
/// linked list running through the data the loader is going to write over. A
/// link that binds carries the ordinal of an import rather than an address,
/// and that ordinal is what names it.
///
/// The walk is therefore: every segment that has fixups, every page of it that
/// starts a chain, then each link until one says there is no next.
fn mach_o_chained(file: &[u8], order: Endianness) -> Vec<ImportSlot> {
    let Some((segments, base)) = mach_o_fixup_blob(file, order) else {
        return Vec::new();
    };
    let imports = ChainedImports {
        base,
        imports: read_u32(file, base + 8, order).unwrap_or(0) as usize,
        format: read_u32(file, base + 20, order).unwrap_or(0),
        count: read_u32(file, base + 16, order).unwrap_or(0) as usize,
        symbols: read_u32(file, base + 12, order).unwrap_or(0) as usize,
    };
    let Some(starts) = read_u32(file, base + 4, order).map(|at| base.saturating_add(at as usize))
    else {
        return Vec::new();
    };
    let Some(count) = read_u32(file, starts, order) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for index in 0..(count as usize).min(MAXIMUM_SEGMENTS) {
        let Some(offset) = read_u32(file, starts + 4 + index * 4, order).filter(|at| *at != 0)
        else {
            continue; // A segment with no fixups in it at all.
        };
        let Some(segment) = segments.get(index) else {
            break;
        };
        walk_chained_segment(
            file,
            order,
            starts.saturating_add(offset as usize),
            *segment,
            &imports,
            &mut out,
        );
    }
    out
}

/// Follows every chain a segment declares, page by page.
fn walk_chained_segment(
    file: &[u8],
    order: Endianness,
    at: usize,
    segment: MachOSegment,
    imports: &ChainedImports,
    out: &mut Vec<ImportSlot>,
) {
    /// A page that starts no chain.
    const NO_CHAIN: u16 = 0xffff;
    /// A page whose starts are listed elsewhere in the same array.
    const SEVERAL: u16 = 0x8000;

    let fields = (
        read_u16(file, at + 4, order),  // page size
        read_u16(file, at + 6, order),  // pointer format
        read_u16(file, at + 20, order), // number of pages
    );
    let (Some(page_size), Some(format), Some(pages)) = fields else {
        return;
    };
    let Some(shape) = chain_format(format) else {
        return; // A format that holds no binds, or none this reader knows.
    };
    let page_size = u64::from(page_size);
    if page_size == 0 {
        return;
    }

    for page in 0..u64::from(pages).min(MAXIMUM_PAGES) {
        let entry = at.saturating_add(22 + usize::try_from(page).unwrap_or(0) * 2);
        let Some(start) = read_u16(file, entry, order).filter(|start| *start != NO_CHAIN) else {
            continue;
        };
        // A page holding several chains lists their starts further along the
        // same array; the last of them is marked rather than counted.
        let starts: Vec<u16> = if start & SEVERAL == 0 {
            vec![start]
        } else {
            let mut listed = Vec::new();
            let first = at.saturating_add(22 + usize::from(start & !SEVERAL) * 2);
            for step in 0..MAXIMUM_CHAINS {
                let Some(next) = read_u16(file, first + step * 2, order) else {
                    break;
                };
                listed.push(next & !SEVERAL);
                if next & SEVERAL != 0 {
                    break; // The end of the list, which is how it is marked.
                }
            }
            listed
        };

        for start in starts {
            walk_chain(
                file,
                order,
                &shape,
                segment,
                page * page_size + u64::from(start),
                imports,
                out,
            );
        }
    }
}

/// Follows one chain from its first link, naming every link that binds.
fn walk_chain(
    file: &[u8],
    order: Endianness,
    shape: &ChainFormat,
    segment: MachOSegment,
    from: u64,
    imports: &ChainedImports,
    out: &mut Vec<ImportSlot>,
) {
    let mut offset = from;
    for _ in 0..MAXIMUM_LINKS {
        if out.len() >= MAXIMUM_SLOTS {
            return;
        }
        let Some(at) = usize::try_from(segment.file_offset.saturating_add(offset)).ok() else {
            return;
        };
        let link = if shape.wide {
            read_u64(file, at, order)
        } else {
            read_u32(file, at, order).map(u64::from)
        };
        let Some(link) = link else {
            return; // The chain runs past what the file holds.
        };
        if link >> shape.bind_bit & 1 == 1
            && let Some(name) = imports.name(file, order, link & shape.ordinal_mask)
        {
            out.push(ImportSlot {
                address: segment.address.saturating_add(offset),
                name,
            });
        }
        let next = link >> shape.next_shift & shape.next_mask;
        if next == 0 {
            return; // The last link of this chain.
        }
        offset = offset.saturating_add(next.saturating_mul(shape.stride));
    }
}

/// Where a segment sits, in memory and in the file.
#[derive(Clone, Copy)]
struct MachOSegment {
    address: u64,
    file_offset: u64,
}

/// The segments of a Mach-O, in the order they are declared, and where its
/// chained-fixups blob begins.
///
/// The order matters: the fixups name a segment by its index in this list.
fn mach_o_fixup_blob(file: &[u8], order: Endianness) -> Option<(Vec<MachOSegment>, usize)> {
    const LC_SEGMENT: u32 = 0x1;
    const LC_SEGMENT_64: u32 = 0x19;
    const LC_DYLD_CHAINED_FIXUPS: u32 = 0x8000_0034;

    let mut segments = Vec::new();
    let mut blob = None;
    // Chained fixups are a 64-bit affair; the walk reads the 64-bit layout.
    super::sections::for_each_mach_o_command(file, 64, order, |command, at| {
        match command {
            LC_SEGMENT | LC_SEGMENT_64 => {
                let wide = command == LC_SEGMENT_64;
                let address = word(file, at + 24, if wide { 64 } else { 32 }, order);
                let file_offset = word(
                    file,
                    at + if wide { 40 } else { 32 },
                    if wide { 64 } else { 32 },
                    order,
                );
                if let (Some(address), Some(file_offset)) = (address, file_offset) {
                    segments.push(MachOSegment {
                        address,
                        file_offset,
                    });
                }
            }
            LC_DYLD_CHAINED_FIXUPS => {
                blob = read_u32(file, at + 8, order).map(|at| at as usize);
            }
            _ => {}
        }
        true
    });
    blob.map(|blob| (segments, blob))
}

fn word(file: &[u8], offset: usize, bits: u8, order: Endianness) -> Option<u64> {
    if bits == 64 {
        read_u64(file, offset, order)
    } else {
        read_u32(file, offset, order).map(u64::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the forged ELF puts each of its parts.
    const STRINGS: usize = 64;
    const SYMBOLS: usize = STRINGS + 8;
    const RELOCATIONS: usize = SYMBOLS + 48;
    const HEADERS: usize = RELOCATIONS + 48;
    /// The address the forged relocation names, which is what a call through
    /// that slot would read its target from.
    const SLOT: u64 = 0x4018;

    /// A 64-bit little-endian ELF carrying a relocation, the symbol it names,
    /// and the strings that symbol's name lives in.
    ///
    /// Forged rather than taken from the fixtures: what is under test is the
    /// walk from a relocation to a name, and the smallest file that has one is
    /// the one that says plainly which fields it is a walk over.
    fn elf_with_a_relocation() -> Vec<u8> {
        let mut file = vec![0_u8; HEADERS + 4 * 64];
        file[..4].copy_from_slice(b"\x7fELF");
        file[4] = 2; // 64-bit.
        file[5] = 1; // Little-endian.
        file[40..48].copy_from_slice(&(HEADERS as u64).to_le_bytes()); // e_shoff
        file[58..60].copy_from_slice(&64_u16.to_le_bytes()); // e_shentsize
        file[60..62].copy_from_slice(&4_u16.to_le_bytes()); // e_shnum

        file[STRINGS..STRINGS + 8].copy_from_slice(b"\0getenv\0");
        // One symbol, named at offset 1 of the strings. Index zero is the
        // undefined one every ELF symbol table starts with.
        file[SYMBOLS + 24..SYMBOLS + 28].copy_from_slice(&1_u32.to_le_bytes());
        // The relocation itself: a jump slot belonging to symbol one, and a
        // relative one, which names no symbol and is not an import.
        file[RELOCATIONS..RELOCATIONS + 8].copy_from_slice(&SLOT.to_le_bytes());
        file[RELOCATIONS + 8..RELOCATIONS + 16].copy_from_slice(&((1_u64 << 32) | 7).to_le_bytes());
        file[RELOCATIONS + 24..RELOCATIONS + 32].copy_from_slice(&0x4020_u64.to_le_bytes());
        file[RELOCATIONS + 32..RELOCATIONS + 40].copy_from_slice(&8_u64.to_le_bytes());

        let mut header =
            |index: usize, kind: u32, link: u32, offset: usize, size: usize, entry: u64| {
                let at = HEADERS + index * 64;
                file[at + 4..at + 8].copy_from_slice(&kind.to_le_bytes());
                file[at + 24..at + 32].copy_from_slice(&(offset as u64).to_le_bytes());
                file[at + 32..at + 40].copy_from_slice(&(size as u64).to_le_bytes());
                file[at + 40..at + 44].copy_from_slice(&link.to_le_bytes());
                file[at + 56..at + 64].copy_from_slice(&entry.to_le_bytes());
            };
        header(1, 3, 0, STRINGS, 8, 0); // .dynstr
        header(2, 11, 1, SYMBOLS, 48, 24); // .dynsym, whose names are in [1]
        header(3, 4, 2, RELOCATIONS, 48, 24); // .rela.plt, over [2]
        file
    }

    #[test]
    fn an_elf_relocation_names_the_slot_a_call_reads() {
        let file = elf_with_a_relocation();
        let slots = extract(
            &file,
            BinaryFormat::Elf {
                bits: 64,
                endianness: Endianness::Little,
            },
        );
        assert_eq!(
            slots,
            vec![ImportSlot {
                address: SLOT,
                name: String::from("getenv"),
            }],
            "the relative relocation names no symbol and is not an import"
        );
    }

    #[test]
    fn a_pe_pairs_each_imported_name_with_the_address_it_belongs_at() {
        let fixture = crate::fixtures::pe_x86_64();
        let slots = extract(&fixture.bytes, BinaryFormat::Pe);
        for name in fixture.imported_functions {
            let found = slots.iter().find(|slot| slot.name == name);
            let Some(found) = found else {
                panic!("{name} is imported but has no slot: {slots:?}");
            };
            assert!(found.address > 0, "a slot has an address to be read from");
        }
        // The address table is an array: consecutive imports are one address
        // apart, which is what says the slots were walked rather than guessed.
        let mut addresses: Vec<u64> = slots.iter().map(|slot| slot.address).collect();
        addresses.dedup();
        assert_eq!(addresses.len(), slots.len(), "one name per address");
    }

    /// Where the forged Mach-O puts each of its parts.
    const SEGMENT: usize = 32;
    const SYMTAB: usize = SEGMENT + 152;
    const DYSYMTAB: usize = SYMTAB + 24;
    const RECORDS: usize = DYSYMTAB + 80;
    const BLOB: usize = RECORDS + 48;
    const INDIRECT: usize = BLOB + 14;
    /// The address of the forged pointer section, whose two slots are the two
    /// addresses a call through it would read.
    const POINTERS: u64 = 0x4000;

    /// A 64-bit little-endian Mach-O with one section of lazy pointers, the
    /// indirect symbol table it indexes into, and the symbols named there.
    ///
    /// Three load commands, because it takes all three to name one slot: the
    /// section says where its run of indirect entries begins, the indirect
    /// table says which symbol each is, and the symbol table says its name.
    fn mach_o_with_pointer_slots() -> Vec<u8> {
        let mut file = vec![0_u8; INDIRECT + 12];
        file[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        file[4..8].copy_from_slice(&0x0100_000c_u32.to_le_bytes()); // arm64
        file[12..16].copy_from_slice(&2_u32.to_le_bytes()); // MH_EXECUTE
        file[16..20].copy_from_slice(&3_u32.to_le_bytes()); // Three commands.

        // One segment holding one section of lazy symbol pointers, whose run
        // in the indirect table starts one entry in.
        file[SEGMENT..SEGMENT + 4].copy_from_slice(&0x19_u32.to_le_bytes()); // LC_SEGMENT_64
        file[SEGMENT + 4..SEGMENT + 8].copy_from_slice(&152_u32.to_le_bytes());
        file[SEGMENT + 64..SEGMENT + 68].copy_from_slice(&1_u32.to_le_bytes()); // nsects
        let section = SEGMENT + 72;
        file[section..section + 15].copy_from_slice(b"__la_symbol_ptr");
        file[section + 32..section + 40].copy_from_slice(&POINTERS.to_le_bytes());
        file[section + 40..section + 48].copy_from_slice(&16_u64.to_le_bytes()); // Two slots.
        file[section + 64..section + 68].copy_from_slice(&7_u32.to_le_bytes()); // Lazy pointers.
        file[section + 68..section + 72].copy_from_slice(&1_u32.to_le_bytes()); // reserved1

        file[SYMTAB..SYMTAB + 4].copy_from_slice(&2_u32.to_le_bytes()); // LC_SYMTAB
        file[SYMTAB + 4..SYMTAB + 8].copy_from_slice(&24_u32.to_le_bytes());
        file[SYMTAB + 8..SYMTAB + 12]
            .copy_from_slice(&u32::try_from(RECORDS).expect("forged small").to_le_bytes());
        file[SYMTAB + 12..SYMTAB + 16].copy_from_slice(&3_u32.to_le_bytes());
        file[SYMTAB + 16..SYMTAB + 20]
            .copy_from_slice(&u32::try_from(BLOB).expect("forged small").to_le_bytes());
        file[SYMTAB + 20..SYMTAB + 24].copy_from_slice(&14_u32.to_le_bytes());

        file[DYSYMTAB..DYSYMTAB + 4].copy_from_slice(&0xb_u32.to_le_bytes()); // LC_DYSYMTAB
        file[DYSYMTAB + 4..DYSYMTAB + 8].copy_from_slice(&80_u32.to_le_bytes());
        file[DYSYMTAB + 56..DYSYMTAB + 60]
            .copy_from_slice(&u32::try_from(INDIRECT).expect("forged small").to_le_bytes());
        file[DYSYMTAB + 60..DYSYMTAB + 64].copy_from_slice(&3_u32.to_le_bytes());

        // Two named symbols, and the strings they are named in.
        file[RECORDS + 16..RECORDS + 20].copy_from_slice(&1_u32.to_le_bytes());
        file[RECORDS + 32..RECORDS + 36].copy_from_slice(&7_u32.to_le_bytes());
        file[BLOB..BLOB + 14].copy_from_slice(b"\0_open\0_close\0");

        // The indirect entries: a local one this section's run starts after,
        // then one per slot.
        file[INDIRECT..INDIRECT + 4].copy_from_slice(&0x8000_0000_u32.to_le_bytes());
        file[INDIRECT + 4..INDIRECT + 8].copy_from_slice(&1_u32.to_le_bytes());
        file[INDIRECT + 8..INDIRECT + 12].copy_from_slice(&2_u32.to_le_bytes());
        file
    }

    #[test]
    fn a_mach_o_pointer_section_names_each_slot_through_the_indirect_table() {
        let file = mach_o_with_pointer_slots();
        let slots = extract(
            &file,
            BinaryFormat::MachO {
                bits: 64,
                endianness: Endianness::Little,
            },
        );
        assert_eq!(
            slots,
            vec![
                ImportSlot {
                    address: POINTERS,
                    // Reported as the source wrote it, like the other two
                    // formats: Mach-O's leading underscore is the container's.
                    name: String::from("open"),
                },
                ImportSlot {
                    address: POINTERS + 8,
                    name: String::from("close"),
                },
            ]
        );
    }

    /// Where the forged chained-fixups file puts each of its parts.
    const FIXUPS_COMMAND: usize = 32 + 72;
    const CHAIN: usize = FIXUPS_COMMAND + 16;
    const FIXUPS: usize = CHAIN + 16;
    /// Offsets inside the blob, which is what every one of its fields counts
    /// from.
    const PAGES_AT: usize = 28;
    const ORDINALS: usize = 60;
    const NAMES: usize = 68;
    /// The address of the forged segment, and so of the first link.
    const CHAINED: u64 = 0x4000;

    /// A 64-bit Mach-O whose fixups are chained: one segment of two pointers
    /// that both bind, and the blob that says which imports they bind to.
    ///
    /// The pointers are the chain — the first says how far the second is —
    /// which is the whole of what makes this format different from a table.
    fn mach_o_with_chained_fixups() -> Vec<u8> {
        let mut file = vec![0_u8; FIXUPS + NAMES + 17];
        file[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        file[4..8].copy_from_slice(&0x0100_000c_u32.to_le_bytes()); // arm64
        file[12..16].copy_from_slice(&2_u32.to_le_bytes()); // MH_EXECUTE
        file[16..20].copy_from_slice(&2_u32.to_le_bytes()); // Two commands.

        // One segment, holding nothing but the chain.
        file[32..36].copy_from_slice(&0x19_u32.to_le_bytes()); // LC_SEGMENT_64
        file[36..40].copy_from_slice(&72_u32.to_le_bytes());
        file[56..64].copy_from_slice(&CHAINED.to_le_bytes()); // vmaddr
        file[72..80].copy_from_slice(&(CHAIN as u64).to_le_bytes()); // fileoff
        file[80..88].copy_from_slice(&16_u64.to_le_bytes()); // filesize

        file[FIXUPS_COMMAND..FIXUPS_COMMAND + 4].copy_from_slice(&0x8000_0034_u32.to_le_bytes()); // LC_DYLD_CHAINED_FIXUPS
        file[FIXUPS_COMMAND + 4..FIXUPS_COMMAND + 8].copy_from_slice(&16_u32.to_le_bytes());
        file[FIXUPS_COMMAND + 8..FIXUPS_COMMAND + 12]
            .copy_from_slice(&u32::try_from(FIXUPS).expect("forged small").to_le_bytes());

        // The chain itself, in `DYLD_CHAINED_PTR_64`: both links bind, and the
        // first says the next one is two strides — eight bytes — further on.
        let bind = |ordinal: u64, next: u64| (1_u64 << 63) | (next << 51) | ordinal;
        file[CHAIN..CHAIN + 8].copy_from_slice(&bind(0, 2).to_le_bytes());
        file[CHAIN + 8..CHAIN + 16].copy_from_slice(&bind(1, 0).to_le_bytes());

        // The blob: its header, then where the chains start, then the imports
        // and the names they are of.
        let mut header = |at: usize, value: u32| {
            file[FIXUPS + at..FIXUPS + at + 4].copy_from_slice(&value.to_le_bytes());
        };
        header(4, u32::try_from(PAGES_AT).expect("forged small"));
        header(8, u32::try_from(ORDINALS).expect("forged small"));
        header(12, u32::try_from(NAMES).expect("forged small"));
        header(16, 2); // Two imports.
        header(20, 1); // The plain import format.

        let starts = FIXUPS + PAGES_AT;
        file[starts..starts + 4].copy_from_slice(&1_u32.to_le_bytes()); // One segment.
        file[starts + 4..starts + 8].copy_from_slice(&8_u32.to_le_bytes()); // Its info follows.
        let segment = starts + 8;
        file[segment + 4..segment + 6].copy_from_slice(&0x1000_u16.to_le_bytes()); // page size
        file[segment + 6..segment + 8].copy_from_slice(&2_u16.to_le_bytes()); // pointer format
        file[segment + 20..segment + 22].copy_from_slice(&1_u16.to_le_bytes()); // One page,
        file[segment + 22..segment + 24].copy_from_slice(&0_u16.to_le_bytes()); // starting at 0.

        let imports = FIXUPS + ORDINALS;
        file[imports..imports + 4].copy_from_slice(&(0_u32 << 9).to_le_bytes());
        file[imports + 4..imports + 8].copy_from_slice(&(8_u32 << 9).to_le_bytes());
        let symbols = FIXUPS + NAMES;
        file[symbols..symbols + 17].copy_from_slice(b"_socket\0_connect\0");
        file
    }

    #[test]
    fn a_mach_o_chain_names_every_link_of_it_that_binds() {
        let file = mach_o_with_chained_fixups();
        let slots = extract(
            &file,
            BinaryFormat::MachO {
                bits: 64,
                endianness: Endianness::Little,
            },
        );
        assert_eq!(
            slots,
            vec![
                ImportSlot {
                    address: CHAINED,
                    name: String::from("socket"),
                },
                ImportSlot {
                    // Eight bytes on, which is where the first link said the
                    // next one was — not where a fixed stride would put it.
                    address: CHAINED + 8,
                    name: String::from("connect"),
                },
            ]
        );
    }

    #[test]
    fn a_file_that_states_no_slots_answers_with_nothing_rather_than_rubbish() {
        assert!(extract(&[0; 64], BinaryFormat::Unknown).is_empty());
        assert!(
            extract(
                &crate::fixtures::mach_o_arm64().bytes,
                BinaryFormat::MachO {
                    bits: 64,
                    endianness: Endianness::Little,
                },
            )
            .is_empty(),
            "the fixture carries symbols but no pointer section to read through"
        );
    }
}
