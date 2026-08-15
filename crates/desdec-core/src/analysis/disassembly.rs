//! Bounded x86/x86-64 decoding, independent of the host platform.
use crate::{Architecture, analysis::Section, binary::BinaryFormat};
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
    let bits = match architecture {
        Architecture::X86 => 32,
        Architecture::X86_64 => 64,
        _ => return Vec::new(),
    };
    if matches!(format, BinaryFormat::Unknown) {
        return Vec::new();
    }
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
