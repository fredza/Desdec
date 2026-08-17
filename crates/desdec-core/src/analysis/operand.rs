//! What an instruction's operands designate, without running anything.
//!
//! Desdec never executes the file, so a register has no value here. Two
//! questions can still be answered from the bytes alone, and they are the ones
//! a reader actually asks:
//!
//! - **What does this operand point at?** A `%rip`-relative or absolute
//!   address is arithmetic on known numbers, so the target is exact: its
//!   section, the symbol it falls in, and the bytes — often a string — that
//!   live there.
//! - **What was last written to this register?** Reading back through the
//!   preceding instructions finds the write, and sometimes the constant it
//!   wrote.
//!
//! The second is a local answer and says so. Following the instructions above
//! a point is only sound while nothing jumps into the middle of them, and a
//! branch target is not tracked here — so the finding names the instruction it
//! found and, when it cannot say what value that left behind, says that
//! instead of inventing one.

use crate::{
    Architecture,
    analysis::{Analysis, Instruction},
};

/// Where an operand points, and what is there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Target {
    pub address: u64,
    pub section: Option<String>,
    /// The symbol the address falls in, with its offset when it is not exact.
    pub symbol: Option<String>,
    /// Printable text at that address, when the bytes read as text.
    pub text: Option<String>,
    /// The first bytes there, for anything that is not text.
    pub bytes: Vec<u8>,
}

/// The instruction that last wrote to a register before a given point.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LastWrite {
    pub address: u64,
    pub text: String,
    /// The constant written, when the instruction wrote a literal one.
    pub value: Option<u64>,
}

/// Bytes shown at a target, and the longest text read from them.
const PREVIEW: usize = 32;

/// How far back a register is followed.
///
/// A bound rather than the whole function: reading back further crosses more
/// branches, and each one makes the answer less true.
const LOOK_BACK: usize = 64;

/// The registers an instruction names, in the order they are written.
#[must_use]
pub fn registers(instruction: &Instruction, architecture: Architecture) -> Vec<String> {
    let operands = instruction
        .text
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest);
    let mut found: Vec<String> = Vec::new();

    for word in operands.split(|c: char| !is_register_char(c)) {
        let name = word.trim_start_matches('%');
        if name.is_empty() || !looks_like_a_register(name, architecture) {
            continue;
        }
        if !found.iter().any(|seen| seen == name) {
            found.push(name.to_owned());
        }
    }
    found
}

const fn is_register_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '%' || c == '_'
}

fn looks_like_a_register(name: &str, architecture: Architecture) -> bool {
    match architecture {
        Architecture::X86 | Architecture::X86_64 => {
            const X86: &[&str] = &[
                "rax", "rbx", "rcx", "rdx", "rsi", "rdi", "rbp", "rsp", "rip", "eax", "ebx", "ecx",
                "edx", "esi", "edi", "ebp", "esp", "ax", "bx", "cx", "dx", "al", "bl", "cl", "dl",
                "ah", "bh", "ch", "dh", "sil", "dil", "bpl", "spl",
            ];
            X86.contains(&name)
                || (name.starts_with('r')
                    && name[1..]
                        .trim_end_matches(['d', 'w', 'b'])
                        .parse::<u8>()
                        .is_ok_and(|number| (8..=15).contains(&number)))
        }
        Architecture::Arm64 => {
            if matches!(name, "sp" | "lr" | "pc" | "fp" | "xzr" | "wzr") {
                return true;
            }
            let mut characters = name.chars();
            let bank = characters.next();
            bank.is_some_and(|bank| matches!(bank, 'x' | 'w' | 'v' | 'd' | 's' | 'q'))
                && !name[1..].is_empty()
                && name[1..].chars().all(|c| c.is_ascii_digit())
        }
        Architecture::Arm | Architecture::Unknown => false,
    }
}

/// The address an instruction's operand computes, when it computes one.
///
/// `%rip`-relative operands are resolved against the following instruction, as
/// the processor does; absolute operands are taken as written.
#[must_use]
pub fn target_address(instruction: &Instruction) -> Option<u64> {
    if let Some(target) = rip_relative(instruction) {
        return Some(target);
    }
    // A bare hexadecimal operand: a call or jump destination, or an absolute
    // address. Only one is considered, so an instruction with several numbers
    // is left alone rather than guessed at.
    let numbers: Vec<u64> = instruction
        .text
        .split(|c: char| !c.is_ascii_hexdigit() && c != 'x' && c != 'X')
        .filter_map(read_hex)
        .collect();
    (numbers.len() == 1).then(|| numbers[0])
}

fn read_hex(word: &str) -> Option<u64> {
    let digits = word
        .strip_prefix("0x")
        .or_else(|| word.strip_prefix("0X"))?;
    u64::from_str_radix(digits, 16).ok()
}

/// Resolves a `%rip`-relative operand against the next instruction's address.
fn rip_relative(instruction: &Instruction) -> Option<u64> {
    let operand = instruction
        .text
        .split_whitespace()
        .skip(1)
        .find(|part| part.contains("%rip") || part.contains("rip,"))?;
    let displacement = operand.split('(').next()?.trim_start_matches('$');
    let (negative, digits) = match displacement.strip_prefix('-') {
        Some(value) => (true, value),
        None => (false, displacement),
    };
    let magnitude = i64::try_from(read_hex(digits)?).ok()?;
    instruction
        .address
        .saturating_add(instruction.bytes.len() as u64)
        .checked_add_signed(if negative { -magnitude } else { magnitude })
}

/// Everything known about what an operand designates.
#[must_use]
pub fn resolve(analysis: &Analysis, instruction: &Instruction, file: &[u8]) -> Option<Target> {
    let address = target_address(instruction)?;
    let section = analysis.section_at(address);

    // A symbol names the target only when the address really falls inside it.
    // Taking the nearest preceding symbol instead reported things like
    // `error_at_line+0x23c60` for an address in `.got`, a hundred and fifty
    // kilobytes away and in another section entirely — a name that sends the
    // reader to the wrong function.
    let symbol = analysis
        .symbols
        .iter()
        .filter(|symbol| !symbol.imported)
        .filter_map(|symbol| Some((symbol.address?, symbol.size, &symbol.name)))
        .find(|(start, size, _)| {
            // An exact hit always counts. Beyond that, only a symbol whose
            // extent is recorded can claim an address inside it; a size of
            // zero means the format did not say, which is not a licence to
            // assume the symbol reaches this far.
            *start == address
                || (*size > 0 && (*start..start.saturating_add(*size)).contains(&address))
        })
        .map(|(start, _, name)| {
            let offset = address - start;
            if offset == 0 {
                name.clone()
            } else {
                format!("{name}+{offset:#x}")
            }
        });

    let bytes = section
        .and_then(|section| {
            let within = usize::try_from(address.checked_sub(section.virtual_address)?).ok()?;
            let at = usize::try_from(section.file_offset)
                .ok()?
                .checked_add(within)?;
            file.get(at..at.saturating_add(PREVIEW).min(file.len()))
        })
        .unwrap_or_default()
        .to_vec();

    Some(Target {
        address,
        section: section.map(|section| section.name.clone()),
        symbol,
        text: printable(&bytes),
        bytes,
    })
}

/// The text at a target, when the bytes read as a string rather than as data.
fn printable(bytes: &[u8]) -> Option<String> {
    let text: String = bytes
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| char::from(*byte))
        .collect();
    // Two characters is noise; a run of printable bytes is a string.
    let readable = text.len() >= 2 && text.chars().all(|c| c.is_ascii_graphic() || c == ' ');
    readable.then_some(text)
}

/// The last instruction to write `register` before `address`.
#[must_use]
pub fn last_write(
    analysis: &Analysis,
    address: u64,
    register: &str,
    architecture: Architecture,
) -> Option<LastWrite> {
    let position = analysis.instruction_index(address)?;

    for instruction in analysis.instructions[..position]
        .iter()
        .rev()
        .take(LOOK_BACK)
    {
        if !writes_to(instruction, register, architecture) {
            continue;
        }
        return Some(LastWrite {
            address: instruction.address,
            text: instruction.text.clone(),
            value: written_constant(instruction, architecture),
        });
    }
    None
}

/// Whether an instruction's destination is this register.
///
/// The destination sits at opposite ends in the two syntaxes: last in the AT&T
/// text the x86 formatter produces, first in what Capstone prints for ARM64.
fn writes_to(instruction: &Instruction, register: &str, architecture: Architecture) -> bool {
    let Some((mnemonic, operands)) = instruction.text.split_once(char::is_whitespace) else {
        return false;
    };
    // A comparison or a store reads its operands without writing a register.
    if mnemonic.starts_with("cmp")
        || mnemonic.starts_with("test")
        || mnemonic.starts_with("push")
        || mnemonic.starts_with("str")
        || mnemonic.starts_with('j')
        || mnemonic.starts_with("b.")
    {
        return false;
    }
    let parts: Vec<&str> = operands.split(',').map(str::trim).collect();
    let destination = match architecture {
        Architecture::X86 | Architecture::X86_64 => parts.last(),
        _ => parts.first(),
    };
    // Only a bare register is a write; `(%rax)` is a memory destination.
    destination.is_some_and(|part| part.trim_start_matches('%') == register)
}

/// The literal an instruction moved into a register, when it moved one.
fn written_constant(instruction: &Instruction, architecture: Architecture) -> Option<u64> {
    let (mnemonic, operands) = instruction.text.split_once(char::is_whitespace)?;
    if !mnemonic.starts_with("mov") {
        return None;
    }
    let parts: Vec<&str> = operands.split(',').map(str::trim).collect();
    let source = match architecture {
        Architecture::X86 | Architecture::X86_64 => parts.first(),
        _ => parts.last(),
    }?;
    // `$0x2a` in AT&T, `#0x2a` in ARM64 assembly.
    let literal = source
        .strip_prefix('$')
        .or_else(|| source.strip_prefix('#'))?;
    read_hex(literal).or_else(|| literal.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instruction(address: u64, text: &str, length: usize) -> Instruction {
        Instruction {
            address,
            bytes: vec![0x90; length],
            text: text.to_owned(),
            section: ".text".to_owned(),
        }
    }

    #[test]
    fn a_rip_relative_operand_resolves_against_the_next_instruction() {
        // 0x400ff0 + 7 bytes + 0x1009 = 0x402000.
        let lea = instruction(0x40_0ff0, "leaq 0x1009(%rip),%rax", 7);
        assert_eq!(target_address(&lea), Some(0x40_2000));
    }

    #[test]
    fn an_absolute_operand_is_taken_as_written() {
        let call = instruction(0x40_1000, "callq 0x401040", 5);
        assert_eq!(target_address(&call), Some(0x40_1040));
    }

    /// Two numbers in one instruction is ambiguous, so nothing is reported
    /// rather than the wrong one.
    #[test]
    fn an_ambiguous_operand_resolves_to_nothing() {
        let odd = instruction(0x40_1000, "movq $0x10,0x20(%rax)", 8);
        assert_eq!(target_address(&odd), None);
    }

    #[test]
    fn registers_are_read_from_both_syntaxes() {
        let att = instruction(0x1000, "mov -0x8(%rbp),%rax", 4);
        assert_eq!(
            registers(&att, Architecture::X86_64),
            vec!["rbp".to_owned(), "rax".to_owned()]
        );

        let arm = instruction(0x1000, "ldr x1, [sp, #0x10]", 4);
        assert_eq!(
            registers(&arm, Architecture::Arm64),
            vec!["x1".to_owned(), "sp".to_owned()]
        );
    }

    /// The destination is last in AT&T and first in ARM64 assembly; reading
    /// the wrong end would report the source as having been written.
    #[test]
    fn the_destination_is_found_at_the_right_end_of_each_syntax() {
        let att = instruction(0x1000, "mov %rbx,%rax", 3);
        assert!(writes_to(&att, "rax", Architecture::X86_64));
        assert!(!writes_to(&att, "rbx", Architecture::X86_64));

        let arm = instruction(0x1000, "mov x0, x1", 4);
        assert!(writes_to(&arm, "x0", Architecture::Arm64));
        assert!(!writes_to(&arm, "x1", Architecture::Arm64));
    }

    /// A comparison leaves its operands alone, and a store writes memory
    /// rather than the register naming it.
    #[test]
    fn instructions_that_write_no_register_are_not_counted() {
        let compare = instruction(0x1000, "cmp $0x0,%eax", 3);
        assert!(!writes_to(&compare, "eax", Architecture::X86_64));

        let store = instruction(0x1000, "mov %rax,(%rbx)", 3);
        assert!(!writes_to(&store, "rbx", Architecture::X86_64));
    }

    #[test]
    fn a_moved_literal_is_reported_and_a_computed_value_is_not() {
        let literal = instruction(0x1000, "mov $0x2a,%eax", 5);
        assert_eq!(written_constant(&literal, Architecture::X86_64), Some(0x2a));

        // The value depends on another register, so there is none to report.
        let computed = instruction(0x1000, "add %ebx,%eax", 2);
        assert_eq!(written_constant(&computed, Architecture::X86_64), None);
    }

    /// A symbol may only claim an address that really falls inside it.
    ///
    /// Taking the nearest preceding symbol named a function a hundred and
    /// fifty kilobytes away, in a different section, for an address in `.got`.
    #[test]
    fn a_symbol_only_claims_an_address_within_its_own_extent() {
        use crate::analysis::Symbol;

        let sized = Symbol {
            name: "known_size".to_owned(),
            address: Some(0x1000),
            size: 0x20,
            imported: false,
        };
        let unsized_symbol = Symbol {
            name: "no_size".to_owned(),
            address: Some(0x2000),
            size: 0,
            imported: false,
        };
        let symbols = [sized, unsized_symbol];
        let name_for = |address: u64| {
            symbols
                .iter()
                .filter_map(|symbol| Some((symbol.address?, symbol.size, &symbol.name)))
                .find(|(start, size, _)| {
                    *start == address
                        || (*size > 0 && (*start..start.saturating_add(*size)).contains(&address))
                })
                .map(|(_, _, name)| name.clone())
        };

        assert_eq!(name_for(0x1000), Some("known_size".to_owned()));
        assert_eq!(name_for(0x1010), Some("known_size".to_owned()));
        // Past its recorded end, and far beyond it.
        assert_eq!(name_for(0x1020), None);
        assert_eq!(name_for(0x30_0000), None);
        // A symbol of unknown extent claims its own address and nothing more.
        assert_eq!(name_for(0x2000), Some("no_size".to_owned()));
        assert_eq!(name_for(0x2001), None);
    }

    #[test]
    fn text_at_a_target_is_reported_only_when_it_reads_as_text() {
        assert_eq!(printable(b"usage: %s\0rest"), Some("usage: %s".to_owned()));
        assert_eq!(printable(&[0x48, 0x89, 0xe5, 0x00]), None);
        assert_eq!(printable(&[0x00]), None);
    }
}
