//! Turning a typed instruction back into bytes.
//!
//! The patch editor works in bytes, because bytes are what gets written; but
//! nobody thinks in bytes. This assembles a line of assembly into the bytes
//! that encode it, using the same library that decodes the listing — and the
//! editor then decodes those bytes back and shows what they became, so what
//! is written is judged on what a processor will read rather than on what the
//! typist meant.
//!
//! It understands a deliberately small language: the instructions a patch is
//! actually made of. Anything outside it is refused by name rather than
//! guessed at, and the byte field is always there for the rest.
//!
//! Both spellings are accepted. The listing is printed in AT&T syntax, so that
//! is what a reader will copy — `mov %rsp,%rbp`, source first, `$` before an
//! immediate. Intel order is taken when nothing in the line says otherwise.

use iced_x86::code_asm::{self, CodeAssembler};

use crate::Architecture;

/// Why a line could not be assembled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    /// The built-in assembler only encodes x86 and x86-64.
    UnsupportedArchitecture,
    /// Nothing was typed.
    Empty,
    /// The mnemonic is not one of the forms this assembler knows.
    UnknownMnemonic(String),
    /// An operand is not a register this assembler knows, nor a number.
    UnknownOperand(String),
    /// The mnemonic is known, but not with operands of these kinds.
    WrongOperands(String),
    /// The encoder refused the instruction — an immediate too wide for the
    /// register, a branch too far to reach.
    Refused(String),
}

/// The registers and numbers of one line, already read.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Operand {
    Reg64(code_asm::AsmRegister64),
    Reg32(code_asm::AsmRegister32),
    Number(i64),
}

/// Encodes `text` as it would be encoded at `address`.
///
/// The address matters: a branch is stored as the distance to its target, so
/// the same line assembles to different bytes at different addresses.
///
/// # Errors
///
/// Says which part it could not read, or what the encoder refused.
pub fn assemble(text: &str, architecture: Architecture, address: u64) -> Result<Vec<u8>, Error> {
    let bitness = match architecture {
        Architecture::X86 => 32,
        Architecture::X86_64 => 64,
        Architecture::Arm | Architecture::Arm64 | Architecture::Unknown => {
            return Err(Error::UnsupportedArchitecture);
        }
    };
    let (mnemonic, operands) = read_line(text)?;
    let mut assembler =
        CodeAssembler::new(bitness).map_err(|error| Error::Refused(error.to_string()))?;
    encode(&mut assembler, &mnemonic, &operands, address)?;
    assembler
        .assemble(address)
        .map_err(|error| Error::Refused(error.to_string()))
}

/// The mnemonic and the operands, in destination-first order whatever the
/// syntax they were written in.
fn read_line(text: &str) -> Result<(String, Vec<Operand>), Error> {
    let line = text.split(['#', ';']).next().unwrap_or_default().trim();
    let (mnemonic, rest) = match line.split_once(char::is_whitespace) {
        Some((mnemonic, rest)) => (mnemonic, rest),
        None if line.is_empty() => return Err(Error::Empty),
        None => (line, ""),
    };
    // `%rax` and `$0x10` are AT&T, and AT&T writes the destination last.
    let att = rest.contains('%') || rest.contains('$');
    let mut operands: Vec<Operand> = Vec::new();
    for word in rest.split(',').filter(|word| !word.trim().is_empty()) {
        operands.push(read_operand(word.trim())?);
    }
    if att && operands.len() == 2 {
        operands.swap(0, 1);
    }
    Ok((mnemonic.to_lowercase(), operands))
}

fn read_operand(word: &str) -> Result<Operand, Error> {
    let bare = word.trim_start_matches(['$', '%']);
    if let Some(register) = gpr64(bare) {
        return Ok(Operand::Reg64(register));
    }
    if let Some(register) = gpr32(bare) {
        return Ok(Operand::Reg32(register));
    }
    read_number(bare).map(Operand::Number)
}

fn read_number(word: &str) -> Result<i64, Error> {
    let unknown = || Error::UnknownOperand(word.to_owned());
    let (negative, digits) = match word.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, word),
    };
    let magnitude = match digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        // A literal written the way the listing prints one — `0xffffffffffffffff`
        // — does not fit a signed sixty-four-bit integer, so it is taken for
        // the bit pattern it is rather than refused.
        Some(hexadecimal) => i64::from_str_radix(hexadecimal, 16)
            .or_else(|_| {
                u64::from_str_radix(hexadecimal, 16)
                    .map(|value| i64::from_le_bytes(value.to_le_bytes()))
            })
            .map_err(|_| unknown())?,
        None => digits.parse::<i64>().map_err(|_| unknown())?,
    };
    Ok(if negative { -magnitude } else { magnitude })
}

/// Adds the instruction to the assembler, or says why it cannot.
fn encode(
    assembler: &mut CodeAssembler,
    mnemonic: &str,
    operands: &[Operand],
    address: u64,
) -> Result<(), Error> {
    // GAS prints a size on the mnemonic — `movq`, `pushq`, `retq`. The whole
    // word is tried first, so `jl` is a jump and not a suffixed `j`.
    let stem = mnemonic
        .strip_suffix(['b', 'w', 'l', 'q'])
        .unwrap_or(mnemonic);
    for name in [mnemonic, stem] {
        match attempt(assembler, name, operands, address) {
            // Not this spelling; try the next.
            Err(Error::UnknownMnemonic(_)) => {}
            outcome => return outcome,
        }
    }
    Err(Error::UnknownMnemonic(mnemonic.to_owned()))
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per instruction the assembler knows; a table reads better than the indirection that would shorten it"
)]
fn attempt(
    assembler: &mut CodeAssembler,
    mnemonic: &str,
    operands: &[Operand],
    address: u64,
) -> Result<(), Error> {
    use Operand::{Number, Reg32, Reg64};

    let refused = |error: iced_x86::IcedError| Error::Refused(error.to_string());
    let wrong = || Error::WrongOperands(mnemonic.to_owned());

    /// Encodes one two-operand instruction in each of its accepted shapes.
    macro_rules! binary {
        ($method:ident) => {
            match operands {
                [Reg64(left), Reg64(right)] => assembler.$method(*left, *right).map_err(refused),
                [Reg32(left), Reg32(right)] => assembler.$method(*left, *right).map_err(refused),
                [Reg64(left), Number(value)] => {
                    let value = i32::try_from(*value).map_err(|_| wrong())?;
                    assembler.$method(*left, value).map_err(refused)
                }
                [Reg32(left), Number(value)] => {
                    let value = i32::try_from(*value).map_err(|_| wrong())?;
                    assembler.$method(*left, value).map_err(refused)
                }
                _ => Err(wrong()),
            }
        };
    }

    /// The same for one operand.
    macro_rules! unary {
        ($method:ident) => {
            match operands {
                [Reg64(register)] => assembler.$method(*register).map_err(refused),
                [Reg32(register)] => assembler.$method(*register).map_err(refused),
                _ => Err(wrong()),
            }
        };
    }

    /// A branch, whose operand is where it goes.
    macro_rules! branch {
        ($method:ident) => {
            match operands {
                [Number(target)] => {
                    let target = u64::try_from(*target).unwrap_or(address);
                    assembler.$method(target).map_err(refused)
                }
                _ => Err(wrong()),
            }
        };
    }

    /// One with no operands at all.
    macro_rules! nullary {
        ($method:ident) => {
            if operands.is_empty() {
                assembler.$method().map_err(refused)
            } else {
                Err(wrong())
            }
        };
    }

    match mnemonic {
        "mov" => match operands {
            [Reg64(left), Reg64(right)] => assembler.mov(*left, *right).map_err(refused),
            [Reg32(left), Reg32(right)] => assembler.mov(*left, *right).map_err(refused),
            // A 64-bit register takes a full-width literal; `movabs` is what
            // the encoder reaches for when it does not fit in thirty-two bits.
            [Reg64(left), Number(value)] => assembler.mov(*left, *value).map_err(refused),
            [Reg32(left), Number(value)] => {
                let value = u32::try_from(*value).map_err(|_| wrong())?;
                assembler.mov(*left, value).map_err(refused)
            }
            _ => Err(wrong()),
        },
        "add" => binary!(add),
        "sub" => binary!(sub),
        "cmp" => binary!(cmp),
        "and" => binary!(and),
        "or" => binary!(or),
        "xor" => binary!(xor),
        "test" => binary!(test),
        "inc" => unary!(inc),
        "dec" => unary!(dec),
        "neg" => unary!(neg),
        "not" => unary!(not),
        "push" => match operands {
            [Reg64(register)] => assembler.push(*register).map_err(refused),
            [Reg32(register)] => assembler.push(*register).map_err(refused),
            [Number(value)] => {
                let value = i32::try_from(*value).map_err(|_| wrong())?;
                assembler.push(value).map_err(refused)
            }
            _ => Err(wrong()),
        },
        "pop" => unary!(pop),
        "jmp" => branch!(jmp),
        "je" | "jz" => branch!(je),
        "jne" | "jnz" => branch!(jne),
        "ja" => branch!(ja),
        "jae" | "jnb" => branch!(jae),
        "jb" | "jnae" => branch!(jb),
        "jbe" => branch!(jbe),
        "jg" => branch!(jg),
        "jge" => branch!(jge),
        "jl" => branch!(jl),
        "jle" => branch!(jle),
        "js" => branch!(js),
        "jns" => branch!(jns),
        "call" => branch!(call),
        "ret" => nullary!(ret),
        "nop" => nullary!(nop),
        "int3" => nullary!(int3),
        "hlt" => nullary!(hlt),
        "leave" => nullary!(leave),
        "ud2" => nullary!(ud2),
        "syscall" => nullary!(syscall),
        "cdq" => nullary!(cdq),
        "cqo" => nullary!(cqo),
        _ => Err(Error::UnknownMnemonic(mnemonic.to_owned())),
    }
}

/// The byte a shortfall is filled with, so a patch that encodes shorter than
/// what it replaces does not move everything after it.
pub const PADDING: u8 = 0x90;

fn gpr64(name: &str) -> Option<code_asm::AsmRegister64> {
    use code_asm::gpr64::{
        r8, r9, r10, r11, r12, r13, r14, r15, rax, rbp, rbx, rcx, rdi, rdx, rsi, rsp,
    };
    Some(match name {
        "rax" => rax,
        "rbx" => rbx,
        "rcx" => rcx,
        "rdx" => rdx,
        "rsi" => rsi,
        "rdi" => rdi,
        "rbp" => rbp,
        "rsp" => rsp,
        "r8" => r8,
        "r9" => r9,
        "r10" => r10,
        "r11" => r11,
        "r12" => r12,
        "r13" => r13,
        "r14" => r14,
        "r15" => r15,
        _ => return None,
    })
}

fn gpr32(name: &str) -> Option<code_asm::AsmRegister32> {
    use code_asm::gpr32::{
        eax, ebp, ebx, ecx, edi, edx, esi, esp, r8d, r9d, r10d, r11d, r12d, r13d, r14d, r15d,
    };
    Some(match name {
        "eax" => eax,
        "ebx" => ebx,
        "ecx" => ecx,
        "edx" => edx,
        "esi" => esi,
        "edi" => edi,
        "ebp" => ebp,
        "esp" => esp,
        "r8d" => r8d,
        "r9d" => r9d,
        "r10d" => r10d,
        "r11d" => r11d,
        "r12d" => r12d,
        "r13d" => r13d,
        "r14d" => r14d,
        "r15d" => r15d,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(text: &str) -> Vec<u8> {
        assemble(text, Architecture::X86_64, 0x40_1000).expect(text)
    }

    /// The listing is printed in AT&T, so that is what a reader copies out of
    /// it — and AT&T writes the destination last.
    #[test]
    fn both_spellings_of_one_instruction_encode_the_same_bytes() {
        assert_eq!(bytes("mov %rsp,%rbp"), bytes("mov rbp, rsp"));
        assert_eq!(bytes("movq %rsp,%rbp"), bytes("mov rbp, rsp"));
        assert_eq!(bytes("mov $0x1,%eax"), bytes("mov eax, 1"));
    }

    /// What comes out must be what a processor reads: the bytes are decoded
    /// back with the same decoder the listing uses.
    #[test]
    fn what_is_assembled_decodes_back_to_what_was_typed() {
        for (text, expected) in [
            ("mov rbp, rsp", "mov %rsp,%rbp"),
            ("ret", "ret"),
            ("nop", "nop"),
            ("int3", "int3"),
            ("push rbp", "push %rbp"),
            ("xor eax, eax", "xor %eax,%eax"),
        ] {
            let bytes = bytes(text);
            let decoded = crate::decode_one(&bytes, Architecture::X86_64, 0x40_1000)
                .unwrap_or_else(|| panic!("{text} must decode back"));
            // The formatter pads its mnemonics into a column; what matters
            // here is the instruction, not the spacing it is printed with.
            let printed: Vec<&str> = decoded.text.split_whitespace().collect();
            assert_eq!(printed.join(" "), expected, "{text}");
        }
    }

    /// A branch is stored as the distance to where it goes, so the same line
    /// is different bytes at a different address.
    #[test]
    fn a_branch_is_encoded_against_the_address_it_sits_at() {
        let here = assemble("jmp 0x401010", Architecture::X86_64, 0x40_1000).expect("a jump");
        let elsewhere = assemble("jmp 0x401010", Architecture::X86_64, 0x40_2000).expect("a jump");

        assert_ne!(here, elsewhere, "the distance is part of the encoding");
        let decoded = crate::decode_one(&here, Architecture::X86_64, 0x40_1000).expect("decodes");
        assert!(decoded.text.contains("401010"), "{}", decoded.text);
    }

    /// Refused by name rather than guessed at.
    #[test]
    fn what_it_cannot_read_it_says_it_cannot_read() {
        assert_eq!(
            assemble("frobnicate %rax", Architecture::X86_64, 0),
            Err(Error::UnknownMnemonic("frobnicate".to_owned()))
        );
        assert_eq!(
            assemble("mov %rax,(%rbx)", Architecture::X86_64, 0),
            Err(Error::UnknownOperand("(%rbx)".to_owned())),
            "memory operands are outside what this assembler covers"
        );
        assert_eq!(
            assemble("ret %rax", Architecture::X86_64, 0),
            Err(Error::WrongOperands("ret".to_owned()))
        );
        assert_eq!(assemble("", Architecture::X86_64, 0), Err(Error::Empty));
        assert_eq!(
            assemble("nop", Architecture::Arm64, 0),
            Err(Error::UnsupportedArchitecture),
            "this assembler encodes x86 alone, and says so"
        );
    }
}
