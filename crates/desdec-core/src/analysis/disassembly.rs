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

#[must_use]
pub fn decode(
    file: &[u8],
    format: BinaryFormat,
    architecture: Architecture,
    sections: &[Section],
) -> Vec<Instruction> {
    if matches!(format, BinaryFormat::Unknown) {
        return Vec::new();
    }
    match architecture {
        Architecture::X86 => decode_x86(file, sections, 32),
        Architecture::X86_64 => decode_x86(file, sections, 64),
        Architecture::Arm64 => decode_arm64(file, sections),
        Architecture::Arm | Architecture::Unknown => Vec::new(),
    }
}

fn decode_x86(file: &[u8], sections: &[Section], bits: u32) -> Vec<Instruction> {
    let mut output = Vec::new();
    for section in sections.iter().filter(|s| s.permissions.execute) {
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
            break;
        }
    }
    output
}

/// Decodes AArch64 instructions for Apple Silicon Mach-O and ARM64 ELF/PE
/// files. Capstone is used only for this ISA; iced-x86 remains the x86 decoder.
fn decode_arm64(file: &[u8], sections: &[Section]) -> Vec<Instruction> {
    let Ok(engine) = Capstone::new()
        .arm64()
        .mode(arm64::ArchMode::Arm)
        .detail(true)
        .build()
    else {
        return Vec::new();
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
                return output;
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
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Endianness, Permissions};

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

        let instructions = decode(
            &file,
            BinaryFormat::MachO {
                bits: 64,
                endianness: Endianness::Little,
            },
            Architecture::Arm64,
            &sections,
        );

        assert_eq!(instructions.len(), 1);
        assert_eq!(instructions[0].address, 0x1_0000_0000);
        assert_eq!(instructions[0].text, "ret");
    }
}
