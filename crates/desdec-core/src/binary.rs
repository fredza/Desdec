use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub use crate::bytes::Endianness;
use crate::bytes::{read_u16, read_u32};

/// Bytes read from the start of a file to identify it. Large enough for every
/// header this module inspects, small enough to stay cheap on huge binaries.
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

pub(crate) fn inspect_bytes(bytes: &[u8]) -> (BinaryFormat, Architecture) {
    if bytes.starts_with(elf::MAGIC) {
        return inspect_elf(bytes);
    }
    if bytes.starts_with(pe::DOS_MAGIC) {
        return inspect_pe(bytes);
    }
    inspect_mach_o(bytes)
}

/// Nothing recognisable in the header.
const UNIDENTIFIED: (BinaryFormat, Architecture) = (BinaryFormat::Unknown, Architecture::Unknown);

mod elf {
    pub const MAGIC: &[u8] = b"\x7fELF";
    /// `e_ident[EI_CLASS]`: 1 for 32-bit, 2 for 64-bit.
    pub const CLASS_OFFSET: usize = 4;
    /// `e_ident[EI_DATA]`: 1 for little-endian, 2 for big-endian.
    pub const DATA_OFFSET: usize = 5;
    /// `e_machine`, the processor family.
    pub const MACHINE_OFFSET: usize = 18;

    pub const MACHINE_X86: u16 = 3;
    pub const MACHINE_ARM: u16 = 40;
    pub const MACHINE_X86_64: u16 = 62;
    pub const MACHINE_ARM64: u16 = 183;
}

fn inspect_elf(bytes: &[u8]) -> (BinaryFormat, Architecture) {
    let bits = match bytes.get(elf::CLASS_OFFSET) {
        Some(1) => 32,
        Some(2) => 64,
        _ => 0,
    };
    let endianness = match bytes.get(elf::DATA_OFFSET) {
        Some(1) => Endianness::Little,
        Some(2) => Endianness::Big,
        _ => Endianness::Unknown,
    };
    let architecture = match read_u16(bytes, elf::MACHINE_OFFSET, endianness) {
        Some(elf::MACHINE_X86) => Architecture::X86,
        Some(elf::MACHINE_X86_64) => Architecture::X86_64,
        Some(elf::MACHINE_ARM) => Architecture::Arm,
        Some(elf::MACHINE_ARM64) => Architecture::Arm64,
        _ => Architecture::Unknown,
    };
    (BinaryFormat::Elf { bits, endianness }, architecture)
}

mod pe {
    pub const DOS_MAGIC: &[u8] = b"MZ";
    /// `e_lfanew`: offset of the PE signature inside the DOS header.
    pub const SIGNATURE_POINTER_OFFSET: usize = 0x3c;
    pub const SIGNATURE: &[u8] = b"PE\0\0";
    /// `Machine` field, right after the 4-byte PE signature.
    pub const MACHINE_OFFSET_FROM_SIGNATURE: usize = 4;

    pub const MACHINE_X86: u16 = 0x014c;
    pub const MACHINE_ARM: u16 = 0x01c0;
    pub const MACHINE_X86_64: u16 = 0x8664;
    pub const MACHINE_ARM64: u16 = 0xaa64;
}

fn inspect_pe(bytes: &[u8]) -> (BinaryFormat, Architecture) {
    let Some(signature) = read_u32(bytes, pe::SIGNATURE_POINTER_OFFSET, Endianness::Little) else {
        return UNIDENTIFIED;
    };
    let signature = signature as usize;
    let signature_end = signature.saturating_add(pe::SIGNATURE.len());
    if bytes.get(signature..signature_end) != Some(pe::SIGNATURE) {
        return UNIDENTIFIED;
    }
    let machine_offset = signature.saturating_add(pe::MACHINE_OFFSET_FROM_SIGNATURE);
    let architecture = match read_u16(bytes, machine_offset, Endianness::Little) {
        Some(pe::MACHINE_X86) => Architecture::X86,
        Some(pe::MACHINE_X86_64) => Architecture::X86_64,
        Some(pe::MACHINE_ARM) => Architecture::Arm,
        Some(pe::MACHINE_ARM64) => Architecture::Arm64,
        _ => Architecture::Unknown,
    };
    (BinaryFormat::Pe, architecture)
}

mod mach_o {
    /// `cputype` follows the 4-byte magic.
    pub const CPU_TYPE_OFFSET: usize = 4;
    /// Set on `cputype` for the 64-bit variant of a family.
    pub const CPU_TYPE_64: u32 = 0x0100_0000;

    pub const CPU_TYPE_X86: u32 = 7;
    pub const CPU_TYPE_ARM: u32 = 12;
    pub const CPU_TYPE_X86_64: u32 = CPU_TYPE_64 | CPU_TYPE_X86;
    pub const CPU_TYPE_ARM64: u32 = CPU_TYPE_64 | CPU_TYPE_ARM;
}

fn inspect_mach_o(bytes: &[u8]) -> (BinaryFormat, Architecture) {
    let Some(magic) = bytes.get(..4) else {
        return UNIDENTIFIED;
    };
    let (bits, endianness) = match magic {
        [0xce, 0xfa, 0xed, 0xfe] => (32, Endianness::Little),
        [0xcf, 0xfa, 0xed, 0xfe] => (64, Endianness::Little),
        [0xfe, 0xed, 0xfa, 0xce] => (32, Endianness::Big),
        [0xfe, 0xed, 0xfa, 0xcf] => (64, Endianness::Big),
        _ => return UNIDENTIFIED,
    };
    let architecture = match read_u32(bytes, mach_o::CPU_TYPE_OFFSET, endianness) {
        Some(mach_o::CPU_TYPE_X86) => Architecture::X86,
        Some(mach_o::CPU_TYPE_X86_64) => Architecture::X86_64,
        Some(mach_o::CPU_TYPE_ARM) => Architecture::Arm,
        Some(mach_o::CPU_TYPE_ARM64) => Architecture::Arm64,
        _ => Architecture::Unknown,
    };
    (BinaryFormat::MachO { bits, endianness }, architecture)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_FILE_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temporary_path() -> PathBuf {
        let suffix = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("desdec-header-{}-{suffix}.bin", std::process::id()))
    }

    #[test]
    fn reads_only_the_header_of_a_large_file() {
        let path = temporary_path();
        fs::write(&path, vec![0_u8; HEADER_BYTES * 2]).expect("temporary binary should be written");

        let header = read_header(&path).expect("header should be read");
        fs::remove_file(&path).expect("temporary binary should be removed");
        assert_eq!(header.len(), HEADER_BYTES);
    }

    #[test]
    fn reports_the_size_of_the_whole_file() {
        let path = temporary_path();
        let size = HEADER_BYTES * 3;
        fs::write(&path, vec![0_u8; size]).expect("temporary binary should be written");

        let summary = inspect_path(&path).expect("file should be inspected");
        fs::remove_file(&path).expect("temporary binary should be removed");
        assert_eq!(summary.size, size as u64);
        assert_eq!(summary.format, BinaryFormat::Unknown);
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
    fn identifies_32_bit_big_endian_elf() {
        let bytes = [
            0x7f, b'E', b'L', b'F', 1, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 40,
        ];
        assert_eq!(
            inspect_bytes(&bytes),
            (
                BinaryFormat::Elf {
                    bits: 32,
                    endianness: Endianness::Big
                },
                Architecture::Arm
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
    fn rejects_a_dos_stub_without_a_pe_signature() {
        let mut bytes = vec![0; 0x80];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&(0x40_u32).to_le_bytes());
        assert_eq!(inspect_bytes(&bytes), UNIDENTIFIED);
    }

    #[test]
    fn rejects_a_pe_pointer_beyond_the_header() {
        let mut bytes = vec![0; 0x80];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(inspect_bytes(&bytes), UNIDENTIFIED);
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

    #[test]
    fn reports_an_unknown_format_for_a_truncated_file() {
        assert_eq!(inspect_bytes(b"MZ"), UNIDENTIFIED);
        assert_eq!(inspect_bytes(b""), UNIDENTIFIED);
    }
}
