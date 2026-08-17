//! Bounded x86/x86-64 and ARM64 decoding, independent of the host platform.
use crate::{Architecture, analysis::Section, binary::BinaryFormat};
use capstone::{
    Capstone,
    arch::{BuildsCapstone, arm64},
};
use iced_x86::{Decoder, DecoderOptions, Formatter, GasFormatter};

pub const MAXIMUM_INSTRUCTIONS: usize = 100_000;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Instruction {
    pub address: u64,
    pub bytes: Vec<u8>,
    pub text: String,
    pub section: String,
}

/// Instructions decoded from a file, and whether the decoder ran out of room.
///
/// The flag is not cosmetic: a listing that stops at the limit looks exactly
/// like a program that ends there, and a reader must not mistake the one for
/// the other.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Decoded {
    /// Ordered by address, so a listing reads like the image and a lookup can
    /// bisect instead of scanning.
    pub instructions: Vec<Instruction>,
    /// Set when [`MAXIMUM_INSTRUCTIONS`] was reached with code left to decode.
    pub truncated: bool,
}

#[must_use]
pub fn decode(
    file: &[u8],
    format: BinaryFormat,
    architecture: Architecture,
    sections: &[Section],
) -> Decoded {
    if matches!(format, BinaryFormat::Unknown) {
        return Decoded::default();
    }
    let mut decoded = match architecture {
        Architecture::X86 => decode_x86(file, sections, 32),
        Architecture::X86_64 => decode_x86(file, sections, 64),
        Architecture::Arm64 => decode_arm64(file, sections),
        Architecture::Arm | Architecture::Unknown => Decoded::default(),
    };
    // Sections are decoded in table order, which is not always address order.
    // Sorting once here is what lets every caller bisect the listing.
    decoded
        .instructions
        .sort_by_key(|instruction| instruction.address);
    decoded
}

fn decode_x86(file: &[u8], sections: &[Section], bits: u32) -> Decoded {
    let mut output = Vec::new();
    let mut executable = sections.iter().filter(|s| s.permissions.execute).peekable();
    while let Some(section) = executable.next() {
        let Some(bytes) = section.bytes_in(file) else {
            continue;
        };
        let mut decoder =
            Decoder::with_ip(bits, bytes, section.virtual_address, DecoderOptions::NONE);
        let mut formatter = GasFormatter::new();
        while decoder.can_decode() && output.len() < MAXIMUM_INSTRUCTIONS {
            let instruction = decoder.decode();
            let length = instruction.len();
            if length == 0 {
                break;
            }
            let start = usize::try_from(instruction.ip().saturating_sub(section.virtual_address))
                .unwrap_or(usize::MAX);
            let mut text = String::new();
            formatter.format(&instruction, &mut text);
            output.push(Instruction {
                address: instruction.ip(),
                bytes: bytes
                    .get(start..start.saturating_add(length))
                    .unwrap_or_default()
                    .to_vec(),
                text,
                section: section.name.clone(),
            });
        }
        if output.len() == MAXIMUM_INSTRUCTIONS {
            // Truncated only if something was actually left behind: this
            // section still had bytes, or another executable one follows.
            let truncated = decoder.can_decode() || executable.peek().is_some();
            return Decoded {
                instructions: output,
                truncated,
            };
        }
    }
    Decoded {
        instructions: output,
        truncated: false,
    }
}

/// Decodes AArch64 instructions for Apple Silicon Mach-O and ARM64 ELF/PE
/// files. Capstone is used only for this ISA; iced-x86 remains the x86 decoder.
fn decode_arm64(file: &[u8], sections: &[Section]) -> Decoded {
    let Ok(engine) = Capstone::new()
        .arm64()
        .mode(arm64::ArchMode::Arm)
        .detail(true)
        .build()
    else {
        return Decoded::default();
    };
    let mut output = Vec::new();
    for section in sections
        .iter()
        .filter(|section| section.permissions.execute)
    {
        let Some(bytes) = section.bytes_in(file) else {
            continue;
        };
        let Ok(instructions) = engine.disasm_all(bytes, section.virtual_address) else {
            continue;
        };
        for instruction in instructions.iter() {
            if output.len() == MAXIMUM_INSTRUCTIONS {
                // This very instruction is being dropped, so something was
                // certainly left behind.
                return Decoded {
                    instructions: output,
                    truncated: true,
                };
            }
            let mnemonic = instruction.mnemonic().unwrap_or_default();
            let operands = instruction.op_str().unwrap_or_default();
            let text = if operands.is_empty() {
                mnemonic.to_owned()
            } else {
                format!("{mnemonic} {operands}")
            };
            output.push(Instruction {
                address: instruction.address(),
                bytes: instruction.bytes().to_vec(),
                text,
                section: section.name.clone(),
            });
        }
    }
    Decoded {
        instructions: output,
        truncated: false,
    }
}

/// Decodes a single instruction from `bytes`, as it would read at `address`.
///
/// Used to show what an edited instruction became, so a patch is judged on the
/// decoder's answer rather than on the editor's intent. Returns `None` when the
/// bytes do not form one whole instruction: a partial decode would describe
/// something the processor will not execute.
#[must_use]
pub fn decode_one(bytes: &[u8], architecture: Architecture, address: u64) -> Option<Instruction> {
    if bytes.is_empty() {
        return None;
    }
    let decoded = match architecture {
        Architecture::X86 => decode_one_x86(bytes, address, 32),
        Architecture::X86_64 => decode_one_x86(bytes, address, 64),
        Architecture::Arm64 => decode_one_arm64(bytes, address),
        Architecture::Arm | Architecture::Unknown => None,
    }?;
    // The bytes must be exactly one instruction: trailing bytes would be a
    // second, unshown instruction, and a short read a truncated one.
    (decoded.bytes.len() == bytes.len()).then_some(decoded)
}

fn decode_one_x86(bytes: &[u8], address: u64, bits: u32) -> Option<Instruction> {
    let mut decoder = Decoder::with_ip(bits, bytes, address, DecoderOptions::NONE);
    if !decoder.can_decode() {
        return None;
    }
    let instruction = decoder.decode();
    let length = instruction.len();
    if length == 0 || instruction.is_invalid() {
        return None;
    }
    let mut text = String::new();
    GasFormatter::new().format(&instruction, &mut text);
    Some(Instruction {
        address,
        bytes: bytes.get(..length)?.to_vec(),
        text,
        section: String::new(),
    })
}

fn decode_one_arm64(bytes: &[u8], address: u64) -> Option<Instruction> {
    let engine = Capstone::new()
        .arm64()
        .mode(arm64::ArchMode::Arm)
        .detail(true)
        .build()
        .ok()?;
    let decoded = engine.disasm_count(bytes, address, 1).ok()?;
    let instruction = decoded.iter().next()?;
    let mnemonic = instruction.mnemonic().unwrap_or_default();
    let operands = instruction.op_str().unwrap_or_default();
    Some(Instruction {
        address,
        bytes: instruction.bytes().to_vec(),
        text: if operands.is_empty() {
            mnemonic.to_owned()
        } else {
            format!("{mnemonic} {operands}")
        },
        section: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Endianness, Permissions};

    #[test]
    fn decodes_a_single_x86_instruction_for_a_patch_preview() {
        let nop = decode_one(&[0x90], Architecture::X86_64, 0x40_1000)
            .expect("0x90 is one whole instruction");
        assert_eq!(nop.text, "nop");
        assert_eq!(nop.address, 0x40_1000);
        assert_eq!(nop.bytes, [0x90]);
    }

    /// Bytes that decode to something shorter hide a second instruction; the
    /// preview must not show only the first.
    #[test]
    fn trailing_bytes_are_refused_rather_than_partly_decoded() {
        assert_eq!(decode_one(&[0x90, 0x90], Architecture::X86_64, 0), None);
        assert_eq!(decode_one(&[], Architecture::X86_64, 0), None);
    }

    #[test]
    fn undecodable_bytes_have_no_preview() {
        // An incomplete instruction: the operand bytes are missing.
        assert_eq!(decode_one(&[0x48, 0x8b], Architecture::X86_64, 0), None);
    }

    #[test]
    fn decodes_a_single_arm64_instruction() {
        let ret = decode_one(&[0xc0, 0x03, 0x5f, 0xd6], Architecture::Arm64, 0x1_0000)
            .expect("this word is an AArch64 `ret`");
        assert_eq!(ret.text, "ret");
        assert_eq!(ret.bytes.len(), 4);
    }

    #[test]
    fn decodes_an_arm64_return_instruction() {
        let file = [0xc0, 0x03, 0x5f, 0xd6]; // `ret` in little-endian AArch64.
        let sections = [Section {
            name: "__text".to_owned(),
            virtual_address: 0x1_0000_0000,
            file_offset: 0,
            virtual_size: file.len() as u64,
            file_size: file.len() as u64,
            permissions: Permissions {
                read: true,
                execute: true,
                ..Permissions::default()
            },
            entropy: None,
        }];

        let decoded = decode(
            &file,
            BinaryFormat::MachO {
                bits: 64,
                endianness: Endianness::Little,
            },
            Architecture::Arm64,
            &sections,
        );

        assert_eq!(decoded.instructions.len(), 1);
        assert_eq!(decoded.instructions[0].address, 0x1_0000_0000);
        assert_eq!(decoded.instructions[0].text, "ret");
        assert!(!decoded.truncated, "one instruction is the whole section");
    }

    /// Sections are decoded in table order; the listing must still come out in
    /// address order, because every lookup bisects it.
    #[test]
    fn instructions_come_out_in_address_order() {
        let file = vec![0x90; 64]; // `nop`, whatever the offset.
        let executable = |name: &str, virtual_address: u64, file_offset: u64| Section {
            name: name.to_owned(),
            virtual_address,
            file_offset,
            virtual_size: 32,
            file_size: 32,
            permissions: Permissions {
                read: true,
                execute: true,
                ..Permissions::default()
            },
            entropy: None,
        };
        // The later address is listed first, as a linker is free to do.
        let sections = [
            executable(".text.hot", 0x40_2000, 32),
            executable(".text", 0x40_1000, 0),
        ];

        let decoded = decode(
            &file,
            BinaryFormat::Elf {
                bits: 64,
                endianness: Endianness::Little,
            },
            Architecture::X86_64,
            &sections,
        );

        assert_eq!(decoded.instructions.len(), 64);
        assert!(
            decoded
                .instructions
                .windows(2)
                .all(|pair| pair[0].address <= pair[1].address),
            "the listing must be sorted by address"
        );
        assert_eq!(decoded.instructions[0].address, 0x40_1000);
    }
}
