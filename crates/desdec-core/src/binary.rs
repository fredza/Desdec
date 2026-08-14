use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

const HEADER_BYTES: usize = 4096;

/// Processor family inferred from a file header. This is intentionally a hint:
/// later analysis will provide the authoritative architecture and mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Architecture {
    X86,
    X86_64,
    Arm,
    Arm64,
    Unknown,
}

impl Architecture {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::X86 => "x86",
            Self::X86_64 => "x86-64",
            Self::Arm => "ARM",
            Self::Arm64 => "ARM64",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Endianness {
    Little,
    Big,
    Unknown,
}

impl Endianness {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Little => "little-endian",
            Self::Big => "big-endian",
            Self::Unknown => "unknown endianness",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryFormat {
    Elf { bits: u8, endianness: Endianness },
    Pe,
    MachO { bits: u8, endianness: Endianness },
    Unknown,
}

impl BinaryFormat {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Elf { .. } => "ELF",
            Self::Pe => "PE",
            Self::MachO { .. } => "Mach-O",
            Self::Unknown => "Unrecognised",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinarySummary {
    pub path: PathBuf,
    pub size: u64,
    pub format: BinaryFormat,
    pub architecture: Architecture,
}

/// Read only the header area needed for initial identification.
///
/// # Errors
///
/// Returns an error if the file metadata or its header cannot be read.
pub fn inspect_path(path: impl AsRef<Path>) -> io::Result<BinarySummary> {
    let path = path.as_ref();
    let metadata = fs::metadata(path)?;
    let bytes = read_header(path)?;

    let (format, architecture) = inspect_bytes(&bytes);
    Ok(BinarySummary {
        path: path.to_path_buf(),
        size: metadata.len(),
        format,
        architecture,
    })
}

fn read_header(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(HEADER_BYTES);
    fs::File::open(path)?
        .take(HEADER_BYTES as u64)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn inspect_bytes(bytes: &[u8]) -> (BinaryFormat, Architecture) {
    if bytes.starts_with(b"\x7fELF") {
        return inspect_elf(bytes);
    }
    if bytes.starts_with(b"MZ") {
        return inspect_pe(bytes);
    }
    inspect_mach_o(bytes)
}

fn inspect_elf(bytes: &[u8]) -> (BinaryFormat, Architecture) {
    let bits = match bytes.get(4) {
        Some(1) => 32,
        Some(2) => 64,
        _ => 0,
    };
    let endianness = match bytes.get(5) {
        Some(1) => Endianness::Little,
        Some(2) => Endianness::Big,
        _ => Endianness::Unknown,
    };
    let machine = read_u16(bytes, 18, endianness);
    let architecture = match machine {
        Some(3) => Architecture::X86,
        Some(62) => Architecture::X86_64,
        Some(40) => Architecture::Arm,
        Some(183) => Architecture::Arm64,
        _ => Architecture::Unknown,
    };
    (BinaryFormat::Elf { bits, endianness }, architecture)
}

fn inspect_pe(bytes: &[u8]) -> (BinaryFormat, Architecture) {
    let Some(offset) = read_u32(bytes, 0x3c, Endianness::Little) else {
        return (BinaryFormat::Unknown, Architecture::Unknown);
    };
    let offset = offset as usize;
    if bytes.get(offset..offset.saturating_add(4)) != Some(b"PE\0\0") {
        return (BinaryFormat::Unknown, Architecture::Unknown);
    }
    let architecture = match read_u16(bytes, offset.saturating_add(4), Endianness::Little) {
        Some(0x014c) => Architecture::X86,
        Some(0x8664) => Architecture::X86_64,
        Some(0x01c0) => Architecture::Arm,
        Some(0xaa64) => Architecture::Arm64,
        _ => Architecture::Unknown,
    };
    (BinaryFormat::Pe, architecture)
}

fn inspect_mach_o(bytes: &[u8]) -> (BinaryFormat, Architecture) {
    let Some(magic) = bytes.get(..4) else {
        return (BinaryFormat::Unknown, Architecture::Unknown);
    };
    let (bits, endianness) = match magic {
        [0xce, 0xfa, 0xed, 0xfe] => (32, Endianness::Little),
        [0xcf, 0xfa, 0xed, 0xfe] => (64, Endianness::Little),
        [0xfe, 0xed, 0xfa, 0xce] => (32, Endianness::Big),
        [0xfe, 0xed, 0xfa, 0xcf] => (64, Endianness::Big),
        _ => return (BinaryFormat::Unknown, Architecture::Unknown),
    };
    let architecture = match read_u32(bytes, 4, endianness) {
        Some(7) => Architecture::X86,
        Some(0x0100_0007) => Architecture::X86_64,
        Some(12) => Architecture::Arm,
        Some(0x0100_000c) => Architecture::Arm64,
        _ => Architecture::Unknown,
    };
    (BinaryFormat::MachO { bits, endianness }, architecture)
}

fn read_u16(bytes: &[u8], offset: usize, endianness: Endianness) -> Option<u16> {
    let input: [u8; 2] = bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
    Some(match endianness {
        Endianness::Little => u16::from_le_bytes(input),
        Endianness::Big => u16::from_be_bytes(input),
        Endianness::Unknown => return None,
    })
}

fn read_u32(bytes: &[u8], offset: usize, endianness: Endianness) -> Option<u32> {
    let input: [u8; 4] = bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(match endianness {
        Endianness::Little => u32::from_le_bytes(input),
        Endianness::Big => u32::from_be_bytes(input),
        Endianness::Unknown => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_FILE_COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn reads_only_the_header_of_a_large_file() {
        let suffix = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("desdec-header-{}-{suffix}.bin", std::process::id()));
        fs::write(&path, vec![0_u8; HEADER_BYTES * 2]).expect("temporary binary should be written");

        let header = read_header(&path).expect("header should be read");
        fs::remove_file(&path).expect("temporary binary should be removed");
        assert_eq!(header.len(), HEADER_BYTES);
    }

    #[test]
    fn identifies_64_bit_little_endian_elf() {
        let bytes = [
            0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 62, 0,
        ];
        assert_eq!(
            inspect_bytes(&bytes),
            (
                BinaryFormat::Elf {
                    bits: 64,
                    endianness: Endianness::Little
                },
                Architecture::X86_64
            )
        );
    }

    #[test]
    fn identifies_pe_machine() {
        let mut bytes = vec![0; 0x80];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&(0x40_u32).to_le_bytes());
        bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
        bytes[0x44..0x46].copy_from_slice(&(0x8664_u16).to_le_bytes());
        assert_eq!(
            inspect_bytes(&bytes),
            (BinaryFormat::Pe, Architecture::X86_64)
        );
    }

    #[test]
    fn identifies_arm64_mach_o() {
        let bytes = [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00, 0x00, 0x01];
        assert_eq!(
            inspect_bytes(&bytes),
            (
                BinaryFormat::MachO {
                    bits: 64,
                    endianness: Endianness::Little
                },
                Architecture::Arm64
            )
        );
    }
}
