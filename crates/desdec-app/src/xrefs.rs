//! Who names an address.
//!
//! "What is here?" the listing answers by itself. "Who gets here?" it cannot:
//! the call that reaches a function may be a hundred thousand rows away, and
//! the pointer that holds its address is not code at all. This index answers
//! the second question for every address in the file, built once when the
//! binary is opened.
//!
//! Two kinds of answer, and they are not equally strong. An instruction that
//! computes an address is arithmetic on decoded bytes and is exact. A word in
//! a data section that happens to hold a value inside the image is a *likely*
//! pointer — a vtable entry, a relocation, a jump table — but it may also be a
//! number that looks like one, so it is reported as what it is and never
//! merged with the calls.

use desdec_core::{Analysis, Architecture, Instruction, operand};

/// How an address is named.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Kind {
    /// A call: the flow goes there and is expected back.
    Call,
    /// A branch: the flow goes there.
    Jump,
    /// An instruction that computes the address without going to it — a `lea`
    /// taking the address of a string, a load from a table.
    Reads,
    /// A word in a data section holding this address.
    Pointer,
}

/// One reference, and what kind it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reference {
    /// Where the reference is: an instruction, or the word holding it.
    pub from: u64,
    pub kind: Kind,
}

/// Every reference in the file, by the address it names.
#[derive(Clone, Debug, Default)]
pub struct Index {
    /// `(target, from, kind)`, sorted by target so one address's references
    /// are a contiguous run the search can bisect to.
    entries: Vec<(u64, u64, Kind)>,
}

impl Index {
    /// Builds the index for one binary. Done once when it is opened: the scan
    /// walks every decoded instruction and every data word.
    #[must_use]
    pub fn of(analysis: &Analysis, file: &[u8]) -> Self {
        let mut entries: Vec<(u64, u64, Kind)> = Vec::new();
        for instruction in &analysis.instructions {
            // Only a value that lands somewhere in the image is an address.
            // Without this the index listed every mask and every large
            // constant — `and $0xffffffff0000,%rax` is not a reference to
            // anything, and a reader asking who reaches an address should not
            // have to sort arithmetic out of the answer.
            // A branch's target as well as an ordinary operand's: AArch64
            // writes every branch target as an immediate, which the general
            // reader refuses on purpose, so the index held no branches at all
            // on those files.
            let Some(target) = operand::branch_target(instruction)
                .or_else(|| operand::target_address(instruction))
                .filter(|at| analysis.section_at(*at).is_some())
            else {
                continue;
            };
            entries.push((target, instruction.address, kind_of(instruction)));
        }
        pointers(analysis, file, &mut entries);
        entries.sort_unstable();
        entries.dedup();
        entries.shrink_to_fit();
        Self { entries }
    }

    /// The references naming `target`, as one contiguous run of the table.
    fn span(&self, target: u64) -> &[(u64, u64, Kind)] {
        let start = self.entries.partition_point(|(at, _, _)| *at < target);
        let end = self.entries.partition_point(|(at, _, _)| *at <= target);
        &self.entries[start..end]
    }

    /// Everything that names `target`, in address order.
    pub fn to(&self, target: u64) -> impl Iterator<Item = Reference> + '_ {
        self.span(target).iter().map(|(_, from, kind)| Reference {
            from: *from,
            kind: *kind,
        })
    }

    #[must_use]
    pub fn count(&self, target: u64) -> usize {
        self.span(target).len()
    }
}

/// What an instruction does with the address it names.
fn kind_of(instruction: &Instruction) -> Kind {
    let mnemonic = instruction
        .text
        .split_whitespace()
        .next()
        .unwrap_or_default();
    if mnemonic.starts_with("call") || matches!(mnemonic, "bl" | "blr") {
        return Kind::Call;
    }
    if mnemonic.starts_with('j')
        || mnemonic.starts_with("b.")
        || matches!(mnemonic, "b" | "br" | "cbz" | "cbnz" | "tbz" | "tbnz")
    {
        return Kind::Jump;
    }
    Kind::Reads
}

/// Words in data that hold an address inside the image.
///
/// Only sections that are mapped and not executable — code is read as code,
/// and a byte sequence in the middle of an instruction stream that happens to
/// look like an address is noise. Only aligned words, for the same reason:
/// an unaligned scan finds a "pointer" every few bytes in any dense data.
fn pointers(analysis: &Analysis, file: &[u8], entries: &mut Vec<(u64, u64, Kind)>) {
    let width = match analysis.summary.architecture {
        Architecture::X86 | Architecture::Arm => 4,
        Architecture::X86_64 | Architecture::Arm64 | Architecture::Unknown => 8,
    };
    for section in &analysis.sections {
        if section.permissions.execute || !section.is_mapped() {
            continue;
        }
        let Some(bytes) = section.bytes_in(file) else {
            continue;
        };
        for (index, word) in bytes.chunks_exact(width).enumerate() {
            let value = word
                .iter()
                .rev()
                .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
            if value == 0 || analysis.section_at(value).is_none() {
                continue;
            }
            let Ok(within) = u64::try_from(index * width) else {
                continue;
            };
            let at = section.virtual_address.saturating_add(within);
            entries.push((value, at, Kind::Pointer));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A call, a jump and a load are three different answers to "who gets
    /// here?", and a reader following a flow needs to know which they have.
    #[test]
    fn an_instruction_is_read_for_what_it_does_with_the_address() {
        let instruction = |text: &str| Instruction {
            address: 0x1000,
            bytes: desdec_core::InstructionBytes::new(&[0x90]).expect("one byte"),
            text: text.to_owned(),
            section: std::sync::Arc::from(".text"),
        };

        assert_eq!(kind_of(&instruction("callq 0x401230")), Kind::Call);
        assert_eq!(kind_of(&instruction("bl 0x401230")), Kind::Call);
        assert_eq!(kind_of(&instruction("jne 0x401230")), Kind::Jump);
        assert_eq!(kind_of(&instruction("b.eq 0x401230")), Kind::Jump);
        assert_eq!(kind_of(&instruction("lea 0x2f61(%rip),%rdi")), Kind::Reads);
    }

    /// The whole point of the index: an address is reached from somewhere, and
    /// the somewhere may be anywhere in the file.
    #[test]
    fn the_index_finds_who_calls_a_real_function() {
        let analysis = crate::testing::reference_analysis();
        let index = Index::of(analysis, crate::testing::reference_bytes());
        let Some((target, from)) = analysis.instructions.iter().find_map(|instruction| {
            let mnemonic = instruction.text.split_whitespace().next()?;
            if !mnemonic.starts_with("call") && mnemonic != "bl" {
                return None;
            }
            let target = operand::branch_target(instruction)?;
            Some((target, instruction.address))
        }) else {
            return; // Nothing on this host calls a fixed address.
        };

        let references: Vec<Reference> = index.to(target).collect();
        assert!(
            references
                .iter()
                .any(|reference| reference.from == from && reference.kind == Kind::Call),
            "the call must be indexed against the address it calls"
        );
    }

    /// An address nothing names has no references, rather than the nearest
    /// ones: an answer rounded to the neighbouring run would send a reader to
    /// a function that has nothing to do with what they asked about.
    #[test]
    fn a_lookup_answers_for_one_address_and_never_its_neighbours() {
        let index = Index {
            entries: vec![
                (0x1000, 0x400, Kind::Call),
                (0x1000, 0x500, Kind::Jump),
                (0x2000, 0x600, Kind::Call),
            ],
        };

        assert_eq!(index.count(0x1000), 2);
        assert_eq!(
            index.count(0x1800),
            0,
            "between two runs is neither of them"
        );
        assert_eq!(index.count(0x2000), 1);
        assert_eq!(
            index.to(0x2000).next(),
            Some(Reference {
                from: 0x600,
                kind: Kind::Call
            })
        );
    }

    /// A mask is not a reference: the index only claims values that land
    /// somewhere in the image.
    #[test]
    fn a_constant_that_is_not_an_address_is_not_a_reference() {
        let analysis = crate::testing::reference_analysis();
        let index = Index::of(analysis, crate::testing::reference_bytes());

        for target in [0xffff_ffff_0000_u64, 0xffff_ffff_ffff_f000] {
            if analysis.section_at(target).is_some() {
                continue; // Improbable, but this host would make it a real one.
            }
            assert_eq!(index.count(target), 0, "{target:#x} is arithmetic");
        }
    }
}
