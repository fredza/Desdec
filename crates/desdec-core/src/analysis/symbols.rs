//! Named function symbols exposed by an executable format.

use crate::{
    binary::{BinaryFormat, Endianness},
    bytes::{read_c_string, read_slice, read_u16, read_u32, read_u64},
};

/// A function name advertised by the binary.  An undefined symbol is imported;
/// a defined one is exported or local to the image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub address: Option<u64>,
    pub size: u64,
    pub imported: bool,
}

const MAXIMUM_SYMBOLS: usize = 20_000;

#[must_use]
pub fn extract(file: &[u8], format: BinaryFormat) -> Vec<Symbol> {
    match format {
        BinaryFormat::Elf { bits, endianness } => elf(file, bits, endianness),
        // PE and Mach-O need format-specific RVA/string-table traversal. Keep
        // this honest until those parsers land rather than guessing from text.
        BinaryFormat::Pe | BinaryFormat::MachO { .. } | BinaryFormat::Unknown => Vec::new(),
    }
}

fn elf(file: &[u8], bits: u8, order: Endianness) -> Vec<Symbol> {
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
    let mut out = Vec::new();
    for index in 0..count.min(4096) {
        let Some(header) = table.checked_add(index.saturating_mul(entry_size)) else {
            break;
        };
        let kind = read_u32(file, header + 4, order);
        if !matches!(kind, Some(2 | 11)) {
            continue;
        } // SHT_SYMTAB / SHT_DYNSYM
        let strings_index =
            read_u32(file, header + if bits == 64 { 40 } else { 24 }, order).map(|v| v as usize);
        let sym_off = word(file, header + if bits == 64 { 24 } else { 16 }, bits, order)
            .and_then(|v| usize::try_from(v).ok());
        let sym_size = word(file, header + if bits == 64 { 32 } else { 20 }, bits, order)
            .and_then(|v| usize::try_from(v).ok());
        let sym_ent = word(file, header + if bits == 64 { 56 } else { 36 }, bits, order)
            .and_then(|v| usize::try_from(v).ok());
        let (Some(strings_index), Some(sym_off), Some(sym_size), Some(sym_ent)) =
            (strings_index, sym_off, sym_size, sym_ent)
        else {
            continue;
        };
        if sym_ent == 0 {
            continue;
        }
        let Some(str_header) = table.checked_add(strings_index.saturating_mul(entry_size)) else {
            continue;
        };
        let str_off = word(
            file,
            str_header + if bits == 64 { 24 } else { 16 },
            bits,
            order,
        )
        .and_then(|v| usize::try_from(v).ok());
        let str_size = word(
            file,
            str_header + if bits == 64 { 32 } else { 20 },
            bits,
            order,
        )
        .and_then(|v| usize::try_from(v).ok());
        let Some(strings) = str_off
            .zip(str_size)
            .and_then(|(o, s)| read_slice(file, o, s))
        else {
            continue;
        };
        for n in 0..(sym_size / sym_ent).min(MAXIMUM_SYMBOLS - out.len()) {
            let Some(at) = sym_off.checked_add(n.saturating_mul(sym_ent)) else {
                break;
            };
            let Some(name_at) = read_u32(file, at, order).map(|v| v as usize) else {
                continue;
            };
            let info = file
                .get(at + if bits == 64 { 4 } else { 12 })
                .copied()
                .unwrap_or(0);
            if info & 0x0f != 2 {
                continue;
            }
            let (value_at, size_at, section_at) = if bits == 64 {
                (at + 8, at + 16, at + 6)
            } else {
                (at + 4, at + 8, at + 14)
            };
            let Some(name) = read_c_string(strings, name_at, 512).filter(|n| !n.is_empty()) else {
                continue;
            };
            let section = read_u16(file, section_at, order).unwrap_or(0);
            out.push(Symbol {
                name,
                address: word(file, value_at, bits, order).filter(|v| *v != 0),
                size: word(file, size_at, bits, order).unwrap_or(0),
                imported: section == 0,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name && a.imported == b.imported);
    out
}

fn word(file: &[u8], offset: usize, bits: u8, order: Endianness) -> Option<u64> {
    if bits == 64 {
        read_u64(file, offset, order)
    } else {
        read_u32(file, offset, order).map(u64::from)
    }
}
