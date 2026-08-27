//! Writing a decoded listing back out as assembler source.
//!
//! Desdec reads; an assembler IDE writes. Between the two there has to be a
//! file, and this makes it — the instructions of one function, rendered as
//! something an assembler will accept rather than as the listing a reader
//! looks at.
//!
//! **The syntax is not the listing's.** The listing is printed in AT&T, which
//! is what the decoder produces and what a reader copies from the screen; NASM
//! reads Intel, source second and no sigils. Re-spelling the text would be
//! guesswork, so the *bytes* are decoded again and printed by the same library
//! with its NASM formatter. What comes out is therefore a reading of the same
//! machine code, not a translation of a string.
//!
//! **What this is not.** It is not a round trip. Assembling the output will
//! not reproduce the original bytes in general: an address the code jumps to
//! is written as a label only where the target is inside what was exported,
//! every `%rip`-relative operand has already been resolved to the address it
//! named, and anything the file's data sections held is absent. It is a
//! starting point for reading and editing in an assembler, and it says so at
//! the top of every file it writes.

use std::fmt::Write as _;

use iced_x86::{Decoder, DecoderOptions, Formatter, NasmFormatter, SymbolResolver, SymbolResult};

use crate::{Architecture, Instruction};

/// Why a listing could not be written out.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    /// NASM is an x86 assembler, and this is the one thing the caller cannot
    /// work around by trying again.
    UnsupportedArchitecture(Architecture),
    /// Nothing was selected, or the selection held no decoded instruction.
    Empty,
}

impl std::fmt::Display for Error {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedArchitecture(architecture) => {
                write!(out, "NASM assembles x86 only, not {architecture:?}")
            }
            Self::Empty => out.write_str("there is nothing decoded here to write out"),
        }
    }
}

impl std::error::Error for Error {}

/// What is being written out, for the header the file opens with.
pub struct Source<'a> {
    /// The binary the instructions were decoded from.
    pub binary: &'a str,
    /// What to call the exported routine.
    pub name: &'a str,
    pub architecture: Architecture,
}

/// Renders instructions as NASM source.
///
/// Deterministic and self-contained: the same instructions give the same text,
/// nothing is executed, and nothing is read beyond what the caller passed in.
///
/// # Errors
///
/// [`Error::UnsupportedArchitecture`] for anything but x86, and [`Error::Empty`]
/// when there is no instruction to write.
pub fn nasm(body: &[Instruction], source: &Source<'_>) -> Result<String, Error> {
    let bitness = match source.architecture {
        Architecture::X86_64 => 64,
        Architecture::X86 => 32,
        other => return Err(Error::UnsupportedArchitecture(other)),
    };
    if body.is_empty() {
        return Err(Error::Empty);
    }

    // Which addresses are jumped to from inside this body, so those and only
    // those become labels. A branch leaving what was exported keeps its
    // address: inventing a label for a place that is not in the file would be
    // a name for nothing.
    let addresses: Vec<u64> = body.iter().map(|instruction| instruction.address).collect();
    let targets = branch_targets(body, &addresses);

    let mut out = String::new();
    header(&mut out, source, body, bitness);
    let _ = writeln!(out, "section .text");
    let _ = writeln!(out, "global {}", source.name);
    let _ = writeln!(out);

    // The labels are supplied through the formatter's own symbol resolver
    // rather than by editing the text it produced: the branch target is
    // printed at full width — `0x0000000000001005` — so looking for the
    // address as a string finds nothing, and a near-miss there would leave a
    // `jmp` at an absolute address, which NASM assembles to a branch
    // somewhere else entirely.
    let mut formatter = NasmFormatter::with_options(
        Some(Box::new(Labels {
            targets: targets.clone(),
            text: String::new(),
        })),
        None,
    );
    // Hexadecimal the way NASM writes it, and no needless width prefixes: the
    // output is read by a person before it is read by an assembler.
    formatter.options_mut().set_hex_prefix("0x");
    formatter.options_mut().set_hex_suffix("");
    formatter.options_mut().set_uppercase_hex(false);
    formatter
        .options_mut()
        .set_space_after_operand_separator(true);
    // `call 0x2000`, not `call 0x0000000000002000`. The padding is what a
    // disassembler pads a column with; a source file has no column to keep.
    formatter.options_mut().set_leading_zeros(false);
    formatter.options_mut().set_branch_leading_zeros(false);

    let _ = writeln!(out, "{}:", source.name);
    for instruction in body {
        if targets.contains(&instruction.address) && instruction.address != body[0].address {
            let _ = writeln!(out, "{}:", label(instruction.address));
        }
        let (text, decoded) = spell(instruction, bitness, &mut formatter);
        if decoded {
            let _ = writeln!(out, "    {text}");
        } else {
            // An instruction this decoder cannot read again is written as its
            // own bytes, which assemble to exactly what was there. A comment
            // keeps what the listing called it, so the reader is not left with
            // a row of hexadecimal and no idea what it was.
            let _ = writeln!(
                out,
                "    db {}    ; {}",
                bytes_of(instruction),
                instruction.text.trim()
            );
        }
    }
    Ok(out)
}

/// The comment the file opens with: what this is, where it came from, and what
/// it is not.
fn header(out: &mut String, source: &Source<'_>, body: &[Instruction], bitness: u32) {
    let first = body.first().map_or(0, |instruction| instruction.address);
    let last = body.last().map_or(0, |instruction| instruction.address);
    let _ = writeln!(out, "; {} — {}", source.name, source.binary);
    let _ = writeln!(
        out,
        "; {:#018x}..{:#018x}, {} instructions, decoded by Desdec",
        first,
        last,
        body.len()
    );
    let _ = writeln!(out, ";");
    let _ = writeln!(
        out,
        "; NASM syntax, re-spelled from the bytes rather than from the AT&T"
    );
    let _ = writeln!(
        out,
        "; listing. This is a reading, not a round trip: assembling it will not"
    );
    let _ = writeln!(
        out,
        "; reproduce the original bytes. Addresses outside this routine are"
    );
    let _ = writeln!(
        out,
        "; left as numbers, and the data the code refers to is not here."
    );
    let _ = writeln!(out, ";");
    let _ = writeln!(out, "bits {bitness}");
    let _ = writeln!(out);
}

/// One instruction as NASM writes it, and whether it could be read again at
/// all.
fn spell(instruction: &Instruction, bitness: u32, formatter: &mut NasmFormatter) -> (String, bool) {
    let bytes = instruction.bytes.as_slice();
    let mut decoder = Decoder::with_ip(bitness, bytes, instruction.address, DecoderOptions::NONE);
    if !decoder.can_decode() {
        return (String::new(), false);
    }
    let read = decoder.decode();
    if read.is_invalid() {
        return (String::new(), false);
    }
    let mut text = String::new();
    formatter.format(&read, &mut text);
    (text, true)
}

/// Names the branch targets that are inside what is being written out.
///
/// Only a branch's own target: the same address may well appear as a memory
/// operand somewhere, and a label in front of a load would be a claim about
/// what that load reads.
struct Labels {
    targets: Vec<u64>,
    /// The label the last lookup produced. The result borrows from the
    /// resolver, so it has to live somewhere that outlives the call.
    text: String,
}

impl SymbolResolver for Labels {
    fn symbol(
        &mut self,
        instruction: &iced_x86::Instruction,
        _operand: u32,
        _instruction_operand: Option<u32>,
        address: u64,
        _address_size: u32,
    ) -> Option<SymbolResult<'_>> {
        if near_branch(instruction) != Some(address) || !self.targets.contains(&address) {
            return None;
        }
        self.text = label(address);
        Some(SymbolResult::with_str(address, &self.text))
    }
}

/// The address a branch or call goes to, when it names one directly.
fn near_branch(decoded: &iced_x86::Instruction) -> Option<u64> {
    use iced_x86::OpKind;
    match decoded.op0_kind() {
        OpKind::NearBranch16 | OpKind::NearBranch32 | OpKind::NearBranch64 => {
            Some(decoded.near_branch_target())
        }
        _ => None,
    }
}

/// Every address inside the body that something inside the body branches to.
fn branch_targets(body: &[Instruction], addresses: &[u64]) -> Vec<u64> {
    let mut found: Vec<u64> = Vec::new();
    for instruction in body {
        let bytes = instruction.bytes.as_slice();
        let mut decoder = Decoder::with_ip(64, bytes, instruction.address, DecoderOptions::NONE);
        if !decoder.can_decode() {
            continue;
        }
        let read = decoder.decode();
        if read.is_invalid() {
            continue;
        }
        if let Some(target) = near_branch(&read)
            && addresses.contains(&target)
        {
            found.push(target);
        }
    }
    found.sort_unstable();
    found.dedup();
    found
}

fn label(address: u64) -> String {
    format!(".L{address:x}")
}

fn bytes_of(instruction: &Instruction) -> String {
    instruction
        .bytes
        .as_slice()
        .iter()
        .map(|byte| format!("0x{byte:02x}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn instruction(address: u64, bytes: &[u8], text: &str) -> Instruction {
        Instruction {
            address,
            bytes: crate::InstructionBytes::new(bytes).expect("an instruction's worth of bytes"),
            text: text.to_owned(),
            section: Arc::from(".text"),
        }
    }

    fn source() -> Source<'static> {
        Source {
            binary: "example",
            name: "check_password",
            architecture: Architecture::X86_64,
        }
    }

    /// The point of the whole module: the listing is AT&T and NASM is not, so
    /// the bytes are read again rather than the text rewritten.
    #[test]
    fn the_bytes_are_respelt_in_intel_order_and_not_the_att_text() {
        // 48 89 e5 — `mov %rsp,%rbp` in the listing, `mov rbp, rsp` in NASM.
        let body = [instruction(0x1000, &[0x48, 0x89, 0xe5], "mov %rsp,%rbp")];
        let text = nasm(&body, &source()).expect("x86-64 is written out");
        assert!(
            text.contains("mov rbp, rsp"),
            "destination first, no sigils:\n{text}"
        );
        assert!(
            !text.contains("%rsp"),
            "nothing of the AT&T spelling survives:\n{text}"
        );
    }

    /// NASM will not take an absolute address as a jump target: a number there
    /// assembles to a branch somewhere else entirely. A jump inside what was
    /// exported has to become a label.
    #[test]
    fn a_jump_inside_the_body_becomes_a_label_and_one_leaving_it_stays_a_number() {
        let body = [
            // eb 03: 0x1000 + 2 + 3 = 0x1005, the third instruction below.
            instruction(0x1000, &[0xeb, 0x03], "jmp 0x1005"),
            instruction(0x1002, &[0x48, 0x31, 0xc0], "xor %rax,%rax"),
            instruction(0x1005, &[0x48, 0x31, 0xc9], "xor %rcx,%rcx"),
            instruction(0x1008, &[0xc3], "ret"),
        ];
        let text = nasm(&body, &source()).expect("x86-64 is written out");
        assert!(
            text.contains(".L1005:"),
            "the target is labelled where it lands:\n{text}"
        );
        assert!(
            text.contains("jmp .L1005") || text.contains("jmp short .L1005"),
            "and the branch names the label rather than an address NASM would \
             read as somewhere else:\n{text}"
        );

        // A call far outside the body keeps the address it names: a label for
        // a place that is not here would be a name for nothing.
        let leaving = [instruction(
            0x1000,
            &[0xe8, 0xfb, 0x0f, 0x00, 0x00],
            "call 0x2000",
        )];
        let text = nasm(&leaving, &source()).expect("x86-64 is written out");
        assert!(
            text.contains("0x2000"),
            "an address outside stays an address:\n{text}"
        );
        assert!(
            !text.contains(".L2000"),
            "and is not given a label that names nothing:\n{text}"
        );
    }

    /// The file has to say what it is. Someone opening it a week later must
    /// not take it for something that assembles back to the original binary.
    #[test]
    fn the_file_says_where_it_came_from_and_that_it_is_not_a_round_trip() {
        let body = [instruction(0x0040_1000, &[0xc3], "ret")];
        let text = nasm(&body, &source()).expect("x86-64 is written out");
        assert!(text.contains("check_password"), "{text}");
        assert!(text.contains("example"), "the binary is named:\n{text}");
        assert!(text.contains("0x0000000000401000"), "{text}");
        assert!(
            text.contains("not a round trip"),
            "the caveat is in the file, not only in the documentation:\n{text}"
        );
        assert!(text.contains("section .text"), "{text}");
    }

    /// An instruction the decoder cannot read again must not vanish, and must
    /// not be guessed at either: its bytes assemble to exactly what was there.
    #[test]
    fn an_instruction_that_cannot_be_read_again_is_written_as_its_own_bytes() {
        let body = [instruction(0x1000, &[0xff, 0xff], "(bad)")];
        let text = nasm(&body, &source()).expect("x86-64 is written out");
        assert!(
            text.contains("db 0xff, 0xff"),
            "the bytes are kept:\n{text}"
        );
        assert!(
            text.contains("(bad)"),
            "and what the listing called it is kept beside them:\n{text}"
        );
    }

    /// NASM is an x86 assembler. Saying so is better than writing a file that
    /// cannot be assembled and looks like it should.
    #[test]
    fn an_architecture_nasm_does_not_assemble_is_refused_by_name() {
        let body = [instruction(0x1000, &[0xc0, 0x03, 0x5f, 0xd6], "ret")];
        let refused = nasm(
            &body,
            &Source {
                architecture: Architecture::Arm64,
                ..source()
            },
        );
        assert_eq!(
            refused,
            Err(Error::UnsupportedArchitecture(Architecture::Arm64))
        );
        assert_eq!(nasm(&[], &source()), Err(Error::Empty));
    }
}
