//! x86/x86-64 and ARM64 decoding, independent of the host platform.
//!
//! Every executable byte that was read is decoded. There is no cap on the
//! number of instructions: a listing that stopped after so many looked exactly
//! like a program that ends there, and the reader had no way to reach the rest
//! of the code. What bounds the work is the file itself — at most
//! [`crate::ANALYSIS_BYTE_LIMIT`] is ever read from disk — and the listing is
//! virtualised, so its length costs the interface nothing.
use crate::{Architecture, analysis::Section, binary::BinaryFormat};
use capstone::{
    Capstone,
    arch::{BuildsCapstone, arm64},
};
use iced_x86::{Decoder, DecoderOptions, Formatter, GasFormatter, NasmFormatter};
use std::sync::Arc;

/// The machine bytes of one instruction, held inline.
///
/// The longest x86-64 instruction is fifteen bytes and an `AArch64` one is
/// always four, so a heap-allocated vector here meant one allocation per
/// instruction to hold at most fifteen bytes — eighteen million allocations
/// for a large shared library, and more memory spent on the pointer to the
/// bytes than on the bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstructionBytes {
    length: u8,
    bytes: [u8; Self::MAXIMUM],
}

impl InstructionBytes {
    /// The longest instruction any supported architecture encodes.
    pub const MAXIMUM: usize = 15;

    /// Keeps `bytes`, or as much of it as an instruction can be.
    ///
    /// A longer slice is not something a decoder produces; it is refused
    /// rather than silently stored in part, so nothing can present a fragment
    /// as an instruction.
    #[must_use]
    pub fn new(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > Self::MAXIMUM {
            return None;
        }
        let mut stored = [0_u8; Self::MAXIMUM];
        stored[..bytes.len()].copy_from_slice(bytes);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the length was just checked against MAXIMUM, which is 15"
        )]
        let length = bytes.len() as u8;
        Some(Self {
            length,
            bytes: stored,
        })
    }

    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..usize::from(self.length)]
    }
}

impl std::ops::Deref for InstructionBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl<const N: usize> PartialEq<[u8; N]> for InstructionBytes {
    fn eq(&self, other: &[u8; N]) -> bool {
        self.as_slice() == other
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Instruction {
    pub address: u64,
    pub bytes: InstructionBytes,
    pub text: String,
    /// Name of the section this was decoded from, shared between every
    /// instruction of that section.
    ///
    /// A section name copied into each instruction cost more than the
    /// instruction's own bytes: a listing of eighteen million — which a large
    /// shared library really does reach, now that nothing caps it — spent half
    /// a gigabyte repeating `.text`.
    pub section: Arc<str>,
}

/// Re-spells decoded x86 instructions the way NASM writes them.
///
/// The listing is decoded once, in the AT&T spelling GAS uses, and that text
/// is what every reading built on it — the pseudo-code, the jump arrows, the
/// operand hints — parses. A reader who would rather read Intel order gets it
/// here, and gets it from the instruction's own bytes rather than from that
/// text: turning `mov %rax,-0x8(%rbp)` into `mov [rbp-8], rax` by moving words
/// around works until the two syntaxes stop corresponding word for word, and
/// they stop at the string operations, the sign-extending moves, the segment
/// overrides and every operand whose size only the mnemonic suffix carried.
/// Decoding the fifteen bytes again cannot invent an instruction that is not
/// there: what comes out is another spelling of what the processor would run.
///
/// One speller is built and lent to each row that needs it, rather than built
/// per instruction: a formatter carries its options and its own scratch space,
/// and a virtualised listing asks this of every visible row on every frame.
pub struct Nasm {
    bitness: u32,
    formatter: NasmFormatter,
}

impl Nasm {
    /// A speller for `architecture`, or `None` where the question does not
    /// arise: `AArch64` has one spelling and Capstone already prints it, and an
    /// architecture that decodes to nothing has nothing to spell.
    #[must_use]
    pub fn for_architecture(architecture: Architecture) -> Option<Self> {
        let bitness = Self::bitness(architecture)?;
        let mut formatter = NasmFormatter::new();
        // Hexadecimal as the rest of the listing writes it — `0x8`, not NASM's
        // own `8h`, and lower case like the address column beside it. This is
        // a reading, not a source file: what matters is that a number here
        // looks like the same number two columns to the left.
        formatter.options_mut().set_hex_prefix("0x");
        formatter.options_mut().set_hex_suffix("");
        formatter.options_mut().set_uppercase_hex(false);
        formatter
            .options_mut()
            .set_space_after_operand_separator(true);
        // A branch target padded to sixteen digits is an address column, and
        // this listing already has one of those.
        formatter.options_mut().set_branch_leading_zeros(false);
        Some(Self { bitness, formatter })
    }

    /// Whether this architecture has two spellings to choose between.
    ///
    /// Asked before a speller is built, so an interface can leave the choice
    /// out where there is none to make rather than offer one that does
    /// nothing.
    #[must_use]
    pub const fn spells(architecture: Architecture) -> bool {
        Self::bitness(architecture).is_some()
    }

    /// The mode this architecture decodes in, or `None` for one iced-x86 does
    /// not read at all.
    const fn bitness(architecture: Architecture) -> Option<u32> {
        match architecture {
            Architecture::X86 => Some(32),
            Architecture::X86_64 => Some(64),
            Architecture::Arm64 | Architecture::Arm | Architecture::Unknown => None,
        }
    }

    /// `instruction` as NASM writes it, or `None` when its own bytes do not
    /// decode back to it.
    ///
    /// `None` rather than an empty line or a guess: the caller keeps the text
    /// the listing already holds, which for bytes that decode to nothing is
    /// GAS saying so.
    pub fn spell(&mut self, instruction: &Instruction) -> Option<String> {
        let bytes = instruction.bytes.as_slice();
        let mut decoder = Decoder::with_ip(
            self.bitness,
            bytes,
            instruction.address,
            DecoderOptions::NONE,
        );
        if !decoder.can_decode() {
            return None;
        }
        let read = decoder.decode();
        // The same bytes must decode to the same one instruction. A shorter
        // read means these bytes were decoded in another context — a prefix
        // the listing attached to the row before — and spelling the fragment
        // would put an instruction on this line that is not the one there.
        if read.is_invalid() || read.len() != bytes.len() {
            return None;
        }
        let mut text = String::new();
        self.formatter.format(&read, &mut text);
        Some(text)
    }
}

/// Instructions decoded from a file, and whether any code was left undecoded.
///
/// The flag is not cosmetic: a listing that stops short looks exactly like a
/// program that ends there, and a reader must not mistake the one for the
/// other. It is set when an executable section lies beyond the bytes that were
/// read — a file larger than the analysis limit — and never because the
/// decoder gave up on its own.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Decoded {
    /// Ordered by address, so a listing reads like the image and a lookup can
    /// bisect instead of scanning.
    pub instructions: Vec<Instruction>,
    /// Set when executable bytes existed that could not be read.
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
    let mut output = Vec::with_capacity(estimated_instructions(sections, 3));
    let mut truncated = false;
    for section in sections.iter().filter(|s| s.permissions.execute) {
        let Some(bytes) = section.bytes_in(file) else {
            // The section is past the end of what was read: its code exists,
            // and is not in this listing.
            truncated |= section.file_size > 0;
            continue;
        };
        let name: Arc<str> = Arc::from(section.name.as_str());
        let mut decoder =
            Decoder::with_ip(bits, bytes, section.virtual_address, DecoderOptions::NONE);
        let mut formatter = GasFormatter::new();
        while decoder.can_decode() {
            let instruction = decoder.decode();
            let length = instruction.len();
            if length == 0 {
                break;
            }
            let start = usize::try_from(instruction.ip().saturating_sub(section.virtual_address))
                .unwrap_or(usize::MAX);
            let mut text = String::new();
            formatter.format(&instruction, &mut text);
            let Some(machine_bytes) = bytes
                .get(start..start.saturating_add(length))
                .and_then(InstructionBytes::new)
            else {
                // Neither a short read nor an over-long encoding is an
                // instruction; the listing takes what it can trust.
                continue;
            };
            output.push(Instruction {
                address: instruction.ip(),
                bytes: machine_bytes,
                text,
                section: Arc::clone(&name),
            });
        }
    }
    Decoded {
        instructions: output,
        truncated,
    }
}

/// Roughly how many instructions the executable sections hold, to size the
/// listing once instead of growing it a few thousand times on a large image.
///
/// `bytes_per_instruction` is a floor — 3 for x86, where instructions are
/// rarely shorter, and 4 for `AArch64`, where they are exactly that — so the
/// guess errs towards asking for slightly too much rather than reallocating.
fn estimated_instructions(sections: &[Section], bytes_per_instruction: u64) -> usize {
    let code: u64 = sections
        .iter()
        .filter(|section| section.permissions.execute)
        .map(|section| section.file_size)
        .sum();
    usize::try_from(code / bytes_per_instruction.max(1)).unwrap_or(0)
}

/// Decodes `AArch64` instructions for Apple Silicon Mach-O and ARM64 ELF/PE
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
    let mut output = Vec::with_capacity(estimated_instructions(sections, 4));
    let mut truncated = false;
    for section in sections
        .iter()
        .filter(|section| section.permissions.execute)
    {
        let Some(bytes) = section.bytes_in(file) else {
            truncated |= section.file_size > 0;
            continue;
        };
        let Ok(instructions) = engine.disasm_all(bytes, section.virtual_address) else {
            continue;
        };
        let name: Arc<str> = Arc::from(section.name.as_str());
        for instruction in instructions.iter() {
            let mnemonic = instruction.mnemonic().unwrap_or_default();
            let operands = instruction.op_str().unwrap_or_default();
            let text = if operands.is_empty() {
                mnemonic.to_owned()
            } else {
                format!("{mnemonic} {operands}")
            };
            let Some(machine_bytes) = InstructionBytes::new(instruction.bytes()) else {
                continue;
            };
            output.push(Instruction {
                address: instruction.address(),
                bytes: machine_bytes,
                text,
                section: Arc::clone(&name),
            });
        }
    }
    Decoded {
        instructions: output,
        truncated,
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
        bytes: InstructionBytes::new(bytes.get(..length)?)?,
        text,
        section: Arc::from(""),
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
        bytes: InstructionBytes::new(instruction.bytes())?,
        text: if operands.is_empty() {
            mnemonic.to_owned()
        } else {
            format!("{mnemonic} {operands}")
        },
        section: Arc::from(""),
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

    /// Every executable byte that was read is decoded, however many that is.
    ///
    /// There used to be a cap of a hundred thousand instructions, which a
    /// medium-sized program passes: the rest of its code simply could not be
    /// reached from the interface, and the listing looked complete.
    #[test]
    fn a_long_program_is_decoded_to_its_last_instruction() {
        const COUNT: usize = 250_000;
        let file = vec![0x90; COUNT]; // One `nop` per byte.
        let sections = [Section {
            name: ".text".to_owned(),
            virtual_address: 0x40_1000,
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
            BinaryFormat::Elf {
                bits: 64,
                endianness: Endianness::Little,
            },
            Architecture::X86_64,
            &sections,
        );

        assert_eq!(decoded.instructions.len(), COUNT);
        assert!(!decoded.truncated, "nothing was left out");
        let last = decoded
            .instructions
            .last()
            .expect("the listing is not empty");
        assert_eq!(last.address, 0x40_1000 + COUNT as u64 - 1);
    }

    /// Code that lies past the bytes the analysis read is another matter: it
    /// really is missing, and the listing has to say so.
    #[test]
    fn code_beyond_what_was_read_is_reported_as_missing() {
        let file = vec![0x90; 16];
        let sections = [Section {
            name: ".text".to_owned(),
            virtual_address: 0x40_1000,
            // The header claims far more than the file holds, as a truncated
            // read of a large binary leaves it.
            file_offset: 0,
            virtual_size: 4096,
            file_size: 4096,
            permissions: Permissions {
                read: true,
                execute: true,
                ..Permissions::default()
            },
            entropy: None,
        }];

        let decoded = decode(
            &file,
            BinaryFormat::Elf {
                bits: 64,
                endianness: Endianness::Little,
            },
            Architecture::X86_64,
            &sections,
        );

        assert!(decoded.instructions.is_empty());
        assert!(decoded.truncated, "the missing code must be announced");
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

    fn x86_64() -> Nasm {
        Nasm::for_architecture(Architecture::X86_64).expect("x86-64 has two spellings")
    }

    fn one(bytes: &[u8]) -> Instruction {
        decode_one(bytes, Architecture::X86_64, 0x40_1000).expect("one whole instruction")
    }

    /// The point of the switch: the operands come back in the other order,
    /// the register loses its `%`, and the memory operand gains the brackets
    /// GAS left off — none of which could be had by moving the words of the
    /// AT&T text around.
    #[test]
    fn an_instruction_is_respelled_from_its_own_bytes() {
        let store = one(&[0x48, 0x89, 0x45, 0xf8]);
        assert_eq!(store.text, "mov %rax,-8(%rbp)");
        assert_eq!(x86_64().spell(&store).as_deref(), Some("mov [rbp-8], rax"));

        let load = one(&[0x48, 0x8b, 0x04, 0x25, 0x00, 0x10, 0x00, 0x00]);
        assert_eq!(load.text, "mov 0x1000,%rax");
        assert_eq!(x86_64().spell(&load).as_deref(), Some("mov rax, [0x1000]"));
    }

    /// A branch target is written the way the listing writes an address
    /// everywhere else, and without the sixteen digits of padding: the row
    /// already carries an address column, and a second one inside the
    /// instruction reads as a number nobody can compare at a glance.
    #[test]
    fn a_branch_target_keeps_the_listings_own_hexadecimal() {
        let call = one(&[0xe8, 0x00, 0x01, 0x00, 0x00]);
        assert_eq!(x86_64().spell(&call).as_deref(), Some("call 0x401105"));
    }

    /// `AArch64` has one spelling, which Capstone already wrote. The choice is
    /// not offered there rather than offered and ignored.
    #[test]
    fn an_architecture_with_one_spelling_is_not_offered_another() {
        assert!(!Nasm::spells(Architecture::Arm64));
        assert!(!Nasm::spells(Architecture::Unknown));
        assert!(Nasm::spells(Architecture::X86));
        assert!(Nasm::spells(Architecture::X86_64));
        assert!(Nasm::for_architecture(Architecture::Arm64).is_none());
    }

    /// Bytes that do not decode back to this one instruction are not spelled
    /// at all: the caller keeps what the listing holds, which is the decoder
    /// saying what it made of them.
    #[test]
    fn bytes_that_do_not_decode_back_are_left_to_the_listing() {
        let invalid = Instruction {
            address: 0x40_1000,
            // `push es`, which 64-bit mode does not encode.
            bytes: InstructionBytes::new(&[0x06]).expect("one byte fits"),
            text: "(bad)".to_owned(),
            section: Arc::from(".text"),
        };
        assert_eq!(x86_64().spell(&invalid), None);

        // Two instructions in one row's bytes: spelling the first would put
        // an instruction on the line that is not the whole of what is there.
        let two = Instruction {
            address: 0x40_1000,
            bytes: InstructionBytes::new(&[0x90, 0x90]).expect("two bytes fit"),
            text: "nop".to_owned(),
            section: Arc::from(".text"),
        };
        assert_eq!(x86_64().spell(&two), None);
    }

    /// One speller, many rows: the formatter is reused, and reusing it must
    /// not leave anything of the previous row in the next one.
    #[test]
    fn one_speller_writes_every_row_from_scratch() {
        let mut nasm = x86_64();
        let store = one(&[0x48, 0x89, 0x45, 0xf8]);
        let first = nasm.spell(&store);
        assert_eq!(nasm.spell(&one(&[0xc3])).as_deref(), Some("ret"));
        assert_eq!(nasm.spell(&store), first);
    }
}
