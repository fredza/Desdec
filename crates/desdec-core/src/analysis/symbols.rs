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
        BinaryFormat::MachO { bits, endianness } => mach_o(file, bits, endianness),
        BinaryFormat::Pe => pe(file),
        BinaryFormat::Unknown => Vec::new(),
    }
}

/// Sorts by name and drops repeats, so every format answers in one order.
fn tidy(mut symbols: Vec<Symbol>) -> Vec<Symbol> {
    symbols.sort_by(|a, b| a.name.cmp(&b.name));
    symbols.dedup_by(|a, b| a.name == b.name && a.imported == b.imported);
    symbols
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
    // Offsets come from the file, so they can be anything at all. They are
    // added saturatingly throughout: a header claiming a table near the top of
    // the address space would otherwise overflow the addition itself, before
    // the bounds check on the read could refuse it.
    let mut out = Vec::new();
    for index in 0..count.min(4096) {
        let Some(header) = table.checked_add(index.saturating_mul(entry_size)) else {
            break;
        };
        let kind = read_u32(file, header.saturating_add(4), order);
        if !matches!(kind, Some(2 | 11)) {
            continue;
        } // SHT_SYMTAB / SHT_DYNSYM
        let at = |offset64: usize, offset32: usize| {
            header.saturating_add(if bits == 64 { offset64 } else { offset32 })
        };
        let strings_index = read_u32(file, at(40, 24), order).map(|v| v as usize);
        let sym_off = word(file, at(24, 16), bits, order).and_then(|v| usize::try_from(v).ok());
        let sym_size = word(file, at(32, 20), bits, order).and_then(|v| usize::try_from(v).ok());
        let sym_ent = word(file, at(56, 36), bits, order).and_then(|v| usize::try_from(v).ok());
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
        let string_at = |offset64: usize, offset32: usize| {
            str_header.saturating_add(if bits == 64 { offset64 } else { offset32 })
        };
        let str_off =
            word(file, string_at(24, 16), bits, order).and_then(|v| usize::try_from(v).ok());
        let str_size =
            word(file, string_at(32, 20), bits, order).and_then(|v| usize::try_from(v).ok());
        let Some(strings) = str_off
            .zip(str_size)
            .and_then(|(o, s)| read_slice(file, o, s))
        else {
            continue;
        };
        for n in 0..(sym_size / sym_ent).min(MAXIMUM_SYMBOLS.saturating_sub(out.len())) {
            let Some(entry) = sym_off.checked_add(n.saturating_mul(sym_ent)) else {
                break;
            };
            let Some(name_at) = read_u32(file, entry, order).map(|v| v as usize) else {
                continue;
            };
            let info = file
                .get(entry.saturating_add(if bits == 64 { 4 } else { 12 }))
                .copied()
                .unwrap_or(0);
            if info & 0x0f != 2 {
                continue;
            }
            let (value_at, size_at, section_at) = if bits == 64 {
                (
                    entry.saturating_add(8),
                    entry.saturating_add(16),
                    entry.saturating_add(6),
                )
            } else {
                (
                    entry.saturating_add(4),
                    entry.saturating_add(8),
                    entry.saturating_add(14),
                )
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
    tidy(out)
}

/// Mach-O symbols, read from the `LC_SYMTAB` load command.
///
/// The command names one flat array of `nlist` records and one string blob;
/// each record says where its name starts and whether the symbol is defined
/// here or expected from a library.
///
/// A release build is usually stripped of its local symbols, leaving the
/// dynamic ones — which is still the useful half: they name what the image
/// calls out to.
fn mach_o(file: &[u8], bits: u8, order: Endianness) -> Vec<Symbol> {
    /// Debug entries, which describe source lines rather than code.
    const N_STAB: u8 = 0xe0;
    /// The bits saying where a symbol lives.
    const N_TYPE: u8 = 0x0e;
    /// Not defined here: the loader resolves it from a library.
    const N_UNDF: u8 = 0x0;
    /// Defined in one of this image's sections.
    const N_SECT: u8 = 0xe;
    const LC_SYMTAB: u32 = 0x2;
    /// `n_strx`, `n_type`, `n_sect`, `n_desc`, then a pointer-sized value.
    const RECORD_32: usize = 12;
    const RECORD_64: usize = 16;

    let mut out = Vec::new();
    super::sections::for_each_mach_o_command(file, bits, order, |command, at| {
        if command != LC_SYMTAB {
            return true;
        }
        let fields = (
            read_u32(file, at + 8, order),  // symbol table offset
            read_u32(file, at + 12, order), // number of records
            read_u32(file, at + 16, order), // string blob offset
            read_u32(file, at + 20, order), // string blob length
        );
        let (Some(table), Some(count), Some(strings_at), Some(strings_len)) = fields else {
            return true;
        };
        let strings = read_slice(file, strings_at as usize, strings_len as usize);
        let (Some(strings), Ok(table)) = (strings, usize::try_from(table)) else {
            return true;
        };

        let record = if bits == 64 { RECORD_64 } else { RECORD_32 };
        let wanted = (count as usize).min(MAXIMUM_SYMBOLS.saturating_sub(out.len()));
        for index in 0..wanted {
            let Some(entry) = table.checked_add(index.saturating_mul(record)) else {
                break;
            };
            let kind = file.get(entry + 4).copied().unwrap_or(0);
            // Debug entries describe the source, not the program.
            if kind & N_STAB != 0 {
                continue;
            }
            let Some(name_at) = read_u32(file, entry, order) else {
                continue;
            };
            // Mach-O prefixes C symbols with an underscore the source never
            // had; the ELF side reports source names, so strip it to match.
            let Some(name) = read_c_string(strings, name_at as usize, 512)
                .map(|name| name.strip_prefix('_').unwrap_or(&name).to_owned())
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            let placement = kind & N_TYPE;
            if !matches!(placement, N_UNDF | N_SECT) {
                continue;
            }
            let imported = placement == N_UNDF;
            out.push(Symbol {
                name,
                // An undefined symbol has no address here: its zero value is
                // a placeholder the loader fills in, not a location.
                address: (!imported)
                    .then(|| word(file, entry + 8, bits, order))
                    .flatten()
                    .filter(|value| *value != 0),
                // Mach-O records no size; the disassembly bounds a function by
                // the next symbol instead.
                size: 0,
                imported,
            });
        }
        true
    });
    tidy(out)
}

/// PE symbols, read from the export and import directories.
///
/// A Windows build almost never keeps its COFF symbol table, so the two
/// directories are what remains: exports name what the image offers, imports
/// name what it calls. Together they are the same question the ELF symbol
/// table answers.
fn pe(file: &[u8]) -> Vec<Symbol> {
    let order = Endianness::Little; // PE is little-endian on every target.
    let Some(headers) = PeHeaders::read(file) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    headers.read_exports(file, order, &mut out);
    headers.read_imports(file, order, &mut out);
    tidy(out)
}

/// The header fields needed to walk a PE's directories.
///
/// Read here and shared with [`super::imports`], which walks the same import
/// directory for the addresses this one takes the names from.
pub(crate) struct PeHeaders {
    optional: usize,
    /// Where the image is mapped, so an export's address matches the
    /// disassembly rather than being a bare offset.
    image_base: u64,
    /// `(virtual address, virtual size, file offset, raw size)` per section,
    /// which is what turns a directory's address into a place in the file.
    sections: Vec<(u32, u32, u32, u32)>,
    /// 8 bytes per address on PE32+, 4 on PE32.
    plus: bool,
}

impl PeHeaders {
    /// Where the image is mapped.
    pub(crate) const fn image_base(&self) -> u64 {
        self.image_base
    }

    /// Whether addresses in this image are eight bytes wide rather than four.
    pub(crate) const fn wide(&self) -> bool {
        self.plus
    }

    pub(crate) fn read(file: &[u8]) -> Option<Self> {
        let order = Endianness::Little;
        let signature = read_u32(file, 0x3c, order)? as usize;
        if read_u32(file, signature, order)? != 0x0000_4550 {
            return None; // "PE\0\0"
        }
        let coff = signature + 4;
        let section_count = read_u16(file, coff + 2, order)? as usize;
        let optional_size = read_u16(file, coff + 16, order)? as usize;
        let optional = coff + 20;
        let plus = match read_u16(file, optional, order)? {
            0x20b => true,
            0x10b => false,
            _ => return None,
        };
        let image_base = if plus {
            read_u64(file, optional + 24, order)?
        } else {
            u64::from(read_u32(file, optional + 28, order)?)
        };

        let table = optional + optional_size;
        let mut sections = Vec::new();
        for index in 0..section_count.min(4096) {
            let header = table.checked_add(index.saturating_mul(40))?;
            let (Some(size), Some(address), Some(raw), Some(raw_at)) = (
                read_u32(file, header + 8, order),
                read_u32(file, header + 12, order),
                read_u32(file, header + 16, order),
                read_u32(file, header + 20, order),
            ) else {
                break;
            };
            sections.push((address, size, raw_at, raw));
        }
        Some(Self {
            optional,
            image_base,
            sections,
            plus,
        })
    }

    /// One of the sixteen directories the optional header points at.
    pub(crate) fn directory(
        &self,
        file: &[u8],
        order: Endianness,
        index: usize,
    ) -> Option<(u32, u32)> {
        let at = self.optional + if self.plus { 112 } else { 96 } + index * 8;
        let address = read_u32(file, at, order)?;
        let size = read_u32(file, at + 4, order)?;
        (address != 0 && size != 0).then_some((address, size))
    }

    /// Turns an address in the loaded image into a place in the file.
    ///
    /// The two differ: a section is padded on disk differently from memory, so
    /// reading a directory at its virtual address would land on the wrong
    /// bytes for everything past the first section.
    pub(crate) fn offset_of(&self, address: u32) -> Option<usize> {
        self.sections.iter().find_map(|(start, size, raw_at, raw)| {
            let span = (*size).max(*raw);
            let within = address >= *start && address < start.saturating_add(span);
            // Only what is actually present on disk can be read.
            (within && address - start < *raw)
                .then(|| usize::try_from(raw_at + (address - start)).ok())
                .flatten()
        })
    }

    fn read_exports(&self, file: &[u8], order: Endianness, out: &mut Vec<Symbol>) {
        let Some((address, size)) = self.directory(file, order, 0) else {
            return;
        };
        let Some(table) = self.offset_of(address) else {
            return;
        };
        // An export whose target lands back inside this directory is a
        // forwarder: the bytes there are a string naming another library, not
        // code. Its address would point at text, which the disassembly would
        // then be asked to decode.
        let forwarders = address..address.saturating_add(size);
        let fields = (
            read_u32(file, table + 24, order), // number of names
            read_u32(file, table + 28, order), // address of the function table
            read_u32(file, table + 32, order), // address of the name table
            read_u32(file, table + 36, order), // address of the ordinal table
        );
        let (Some(count), Some(functions), Some(names), Some(ordinals)) = fields else {
            return;
        };
        let (Some(functions), Some(names), Some(ordinals)) = (
            self.offset_of(functions),
            self.offset_of(names),
            self.offset_of(ordinals),
        ) else {
            return;
        };

        let wanted = (count as usize).min(MAXIMUM_SYMBOLS.saturating_sub(out.len()));
        for index in 0..wanted {
            let name_at = read_u32(file, names + index * 4, order)
                .and_then(|address| self.offset_of(address));
            let Some(name) = name_at
                .and_then(|at| read_c_string(file, at, 512))
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            // The name table and the function table are joined through the
            // ordinal table; they are not parallel arrays.
            let entry = read_u16(file, ordinals + index * 2, order).map(usize::from);
            let target = entry.and_then(|entry| read_u32(file, functions + entry * 4, order));
            out.push(Symbol {
                name,
                // A forwarded export keeps its name but has no code here, so
                // it is reported without an address rather than with one that
                // points at a string.
                address: target
                    .filter(|rva| !forwarders.contains(rva))
                    .map(|rva| self.image_base.saturating_add(u64::from(rva))),
                size: 0,
                imported: false,
            });
        }
    }

    fn read_imports(&self, file: &[u8], order: Endianness, out: &mut Vec<Symbol>) {
        let Some((address, _)) = self.directory(file, order, 1) else {
            return;
        };
        let Some(table) = self.offset_of(address) else {
            return;
        };

        // The descriptor array ends at an all-zero entry rather than a count.
        for index in 0..4096 {
            let Some(entry) = table.checked_add(index * 20) else {
                return;
            };
            let names = read_u32(file, entry, order).filter(|value| *value != 0);
            // Bound to the first thunk when the lookup table is absent, as
            // some linkers leave it.
            let names = names.or_else(|| read_u32(file, entry + 16, order));
            let Some(names) = names.filter(|value| *value != 0) else {
                return; // The terminating entry.
            };
            let Some(mut thunk) = self.offset_of(names) else {
                return;
            };

            let step = if self.plus { 8 } else { 4 };
            for _ in 0..MAXIMUM_SYMBOLS {
                if out.len() >= MAXIMUM_SYMBOLS {
                    return;
                }
                let value = if self.plus {
                    read_u64(file, thunk, order)
                } else {
                    read_u32(file, thunk, order).map(u64::from)
                };
                let Some(value) = value.filter(|value| *value != 0) else {
                    break; // End of this library's list.
                };
                // The top bit means the import is by ordinal, with no name to
                // report; anything else points at a hint/name record.
                let by_ordinal = if self.plus {
                    value & (1 << 63) != 0
                } else {
                    value & (1 << 31) != 0
                };
                if !by_ordinal
                    && let Ok(rva) = u32::try_from(value & 0x7fff_ffff)
                    && let Some(at) = self.offset_of(rva)
                    && let Some(name) = read_c_string(file, at + 2, 512).filter(|n| !n.is_empty())
                {
                    out.push(Symbol {
                        name,
                        address: None,
                        size: 0,
                        imported: true,
                    });
                }
                let Some(next) = thunk.checked_add(step) else {
                    return;
                };
                thunk = next;
            }
        }
    }
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

    /// A 64-bit little-endian Mach-O carrying one `LC_SYMTAB`: one symbol
    /// defined in a section, one expected from a library.
    fn mach_o_with_symbols() -> Vec<u8> {
        const HEADER: usize = 32;
        const COMMAND: usize = HEADER;
        const RECORDS: usize = COMMAND + 24;
        const STRINGS: usize = RECORDS + 32;
        let mut file = vec![0_u8; STRINGS + 32];

        file[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        file[4..8].copy_from_slice(&0x0100_000c_u32.to_le_bytes()); // arm64
        file[12..16].copy_from_slice(&2_u32.to_le_bytes()); // MH_EXECUTE
        file[16..20].copy_from_slice(&1_u32.to_le_bytes()); // one command

        file[COMMAND..COMMAND + 4].copy_from_slice(&2_u32.to_le_bytes()); // LC_SYMTAB
        file[COMMAND + 4..COMMAND + 8].copy_from_slice(&24_u32.to_le_bytes());
        file[COMMAND + 8..COMMAND + 12]
            .copy_from_slice(&u32::try_from(RECORDS).unwrap().to_le_bytes());
        file[COMMAND + 12..COMMAND + 16].copy_from_slice(&2_u32.to_le_bytes()); // two records
        file[COMMAND + 16..COMMAND + 20]
            .copy_from_slice(&u32::try_from(STRINGS).unwrap().to_le_bytes());
        file[COMMAND + 20..COMMAND + 24].copy_from_slice(&32_u32.to_le_bytes());

        // Defined here, in a section, at a real address.
        file[RECORDS..RECORDS + 4].copy_from_slice(&1_u32.to_le_bytes());
        file[RECORDS + 4] = 0x0e; // N_SECT
        file[RECORDS + 5] = 1;
        file[RECORDS + 8..RECORDS + 16].copy_from_slice(&0x1_0000_1000_u64.to_le_bytes());

        // Undefined: the loader resolves it elsewhere.
        file[RECORDS + 16..RECORDS + 20].copy_from_slice(&16_u32.to_le_bytes());
        file[RECORDS + 20] = 0x00; // N_UNDF

        file[STRINGS + 1..STRINGS + 11].copy_from_slice(b"_local_fn\0");
        file[STRINGS + 16..STRINGS + 24].copy_from_slice(b"_printf\0");
        file
    }

    #[test]
    fn mach_o_reads_defined_and_undefined_symbols() {
        let symbols = extract(
            &mach_o_with_symbols(),
            BinaryFormat::MachO {
                bits: 64,
                endianness: Endianness::Little,
            },
        );

        assert_eq!(symbols.len(), 2);
        let local = symbols
            .iter()
            .find(|s| s.name == "local_fn")
            .expect("defined");
        assert_eq!(local.address, Some(0x1_0000_1000));
        assert!(!local.imported);

        let printf = symbols
            .iter()
            .find(|s| s.name == "printf")
            .expect("imported");
        assert!(printf.imported);
        // An undefined symbol's zero value is a placeholder, not a location.
        assert_eq!(printf.address, None);
    }

    /// Mach-O prefixes C symbols with an underscore the source never had.
    #[test]
    fn the_mach_o_underscore_prefix_is_not_part_of_the_name() {
        let symbols = extract(
            &mach_o_with_symbols(),
            BinaryFormat::MachO {
                bits: 64,
                endianness: Endianness::Little,
            },
        );
        assert!(
            symbols.iter().all(|s| !s.name.starts_with('_')),
            "{symbols:?}"
        );
    }

    /// A PE32+ with an export directory holding one real export and one
    /// forwarder, plus an import descriptor naming one function.
    fn pe_with_directories() -> Vec<u8> {
        const SIGNATURE: usize = 0x80;
        const OPTIONAL: usize = SIGNATURE + 24;
        const OPTIONAL_SIZE: usize = 0xf0;
        const TABLE: usize = OPTIONAL + OPTIONAL_SIZE;
        const RAW: usize = TABLE + 40;
        /// The section maps this address to [`RAW`].
        const BASE_RVA: u32 = 0x1000;

        let at = |rva: u32| RAW + (rva - BASE_RVA) as usize;
        let mut file = vec![0_u8; RAW + 0x400];

        file[..2].copy_from_slice(b"MZ");
        file[0x3c..0x40].copy_from_slice(&u32::try_from(SIGNATURE).unwrap().to_le_bytes());
        file[SIGNATURE..SIGNATURE + 4].copy_from_slice(b"PE\0\0");
        file[SIGNATURE + 4..SIGNATURE + 6].copy_from_slice(&0x8664_u16.to_le_bytes());
        file[SIGNATURE + 6..SIGNATURE + 8].copy_from_slice(&1_u16.to_le_bytes());
        file[SIGNATURE + 20..SIGNATURE + 22]
            .copy_from_slice(&u16::try_from(OPTIONAL_SIZE).unwrap().to_le_bytes());

        file[OPTIONAL..OPTIONAL + 2].copy_from_slice(&0x20b_u16.to_le_bytes()); // PE32+
        file[OPTIONAL + 24..OPTIONAL + 32].copy_from_slice(&0x1_4000_0000_u64.to_le_bytes());
        // Data directory 0 is the exports, 1 the imports.
        file[OPTIONAL + 112..OPTIONAL + 116].copy_from_slice(&0x1000_u32.to_le_bytes());
        file[OPTIONAL + 116..OPTIONAL + 120].copy_from_slice(&0x100_u32.to_le_bytes());
        file[OPTIONAL + 120..OPTIONAL + 124].copy_from_slice(&0x1100_u32.to_le_bytes());
        file[OPTIONAL + 124..OPTIONAL + 128].copy_from_slice(&0x100_u32.to_le_bytes());

        file[TABLE..TABLE + 8].copy_from_slice(b".rdata\0\0");
        file[TABLE + 8..TABLE + 12].copy_from_slice(&0x400_u32.to_le_bytes()); // virtual size
        file[TABLE + 12..TABLE + 16].copy_from_slice(&BASE_RVA.to_le_bytes());
        file[TABLE + 16..TABLE + 20].copy_from_slice(&0x400_u32.to_le_bytes()); // raw size
        file[TABLE + 20..TABLE + 24].copy_from_slice(&u32::try_from(RAW).unwrap().to_le_bytes());

        // Export directory.
        let exports = at(0x1000);
        file[exports + 20..exports + 24].copy_from_slice(&2_u32.to_le_bytes()); // functions
        file[exports + 24..exports + 28].copy_from_slice(&2_u32.to_le_bytes()); // names
        file[exports + 28..exports + 32].copy_from_slice(&0x1030_u32.to_le_bytes());
        file[exports + 32..exports + 36].copy_from_slice(&0x1040_u32.to_le_bytes());
        file[exports + 36..exports + 40].copy_from_slice(&0x1048_u32.to_le_bytes());

        // One target outside the directory, one inside it — a forwarder.
        let functions = at(0x1030);
        file[functions..functions + 4].copy_from_slice(&0x2000_u32.to_le_bytes());
        file[functions + 4..functions + 8].copy_from_slice(&0x1080_u32.to_le_bytes());

        let names = at(0x1040);
        file[names..names + 4].copy_from_slice(&0x1050_u32.to_le_bytes());
        file[names + 4..names + 8].copy_from_slice(&0x1060_u32.to_le_bytes());

        let ordinals = at(0x1048);
        file[ordinals..ordinals + 2].copy_from_slice(&0_u16.to_le_bytes());
        file[ordinals + 2..ordinals + 4].copy_from_slice(&1_u16.to_le_bytes());

        file[at(0x1050)..at(0x1050) + 9].copy_from_slice(b"real_fn\0\0");
        file[at(0x1060)..at(0x1060) + 10].copy_from_slice(b"sent_away\0");

        // Import descriptor, then the all-zero entry that ends the array.
        let imports = at(0x1100);
        file[imports..imports + 4].copy_from_slice(&0x1120_u32.to_le_bytes()); // lookup table
        file[imports + 12..imports + 16].copy_from_slice(&0x1140_u32.to_le_bytes()); // library
        file[imports + 16..imports + 20].copy_from_slice(&0x1160_u32.to_le_bytes()); // first thunk

        let thunks = at(0x1120);
        file[thunks..thunks + 8].copy_from_slice(&0x1180_u64.to_le_bytes());
        file[at(0x1140)..at(0x1140) + 8].copy_from_slice(b"lib.dll\0");
        // A hint/name record: two bytes of hint, then the name.
        file[at(0x1180) + 2..at(0x1180) + 12].copy_from_slice(b"CreateFil\0");
        file
    }

    #[test]
    fn pe_reads_its_exports_and_imports() {
        let symbols = extract(&pe_with_directories(), BinaryFormat::Pe);

        let exported = symbols
            .iter()
            .find(|s| s.name == "real_fn")
            .expect("export");
        assert!(!exported.imported);
        assert_eq!(exported.address, Some(0x1_4000_2000));

        let imported = symbols
            .iter()
            .find(|s| s.name == "CreateFil")
            .expect("import");
        assert!(imported.imported);
        assert_eq!(imported.address, None);
    }

    /// A forwarded export names another library rather than code here, so its
    /// target is a string. Reporting it as an address would send the
    /// disassembly to decode text.
    #[test]
    fn a_forwarded_export_is_named_without_an_address() {
        let symbols = extract(&pe_with_directories(), BinaryFormat::Pe);

        let forwarded = symbols
            .iter()
            .find(|s| s.name == "sent_away")
            .expect("export");
        assert!(!forwarded.imported, "it is still exported by this image");
        assert_eq!(forwarded.address, None);
    }

    /// Every parser must be total: no input may panic, whatever it holds.
    #[test]
    fn malformed_input_yields_nothing_rather_than_panicking() {
        let formats = [
            BinaryFormat::Pe,
            BinaryFormat::MachO {
                bits: 64,
                endianness: Endianness::Little,
            },
            BinaryFormat::MachO {
                bits: 32,
                endianness: Endianness::Big,
            },
            BinaryFormat::Elf {
                bits: 64,
                endianness: Endianness::Little,
            },
        ];
        for format in formats {
            for file in [
                Vec::new(),
                vec![0_u8; 1],
                vec![0xff_u8; 64],
                mach_o_with_symbols()[..20].to_vec(),
                pe_with_directories()[..200].to_vec(),
            ] {
                let _ = extract(&file, format);
            }
        }
    }

    /// Truncating a well-formed file at every length must never panic: this is
    /// what a hostile file looks like from the parser's side.
    #[test]
    fn every_truncation_of_a_valid_file_is_survivable() {
        for (file, format) in [
            (pe_with_directories(), BinaryFormat::Pe),
            (
                mach_o_with_symbols(),
                BinaryFormat::MachO {
                    bits: 64,
                    endianness: Endianness::Little,
                },
            ),
        ] {
            for length in 0..file.len() {
                let _ = extract(&file[..length], format);
            }
        }
    }
}
