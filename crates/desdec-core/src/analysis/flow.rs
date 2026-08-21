//! What one instruction does to the flow of control.
//!
//! One answer, in one place. This used to be decided three times over — by the
//! walk that follows the flow without running it, by the code that cuts a
//! function into basic blocks, and by the jump arrows in the listing's margin
//! — and the three did not agree: one of them read `loop` as an ordinary
//! instruction, another knew nothing of the `AArch64` spellings. A reader
//! looking at an arrow, a block and a walk of the same instruction has to be
//! shown the same thing by all three.
//!
//! Nothing here reads an operand. Where a branch *goes* is
//! [`crate::operand::branch_target`]'s answer, and it is a separate question:
//! a mnemonic says whether the flow leaves, an operand says where to.

/// What an instruction does to the flow, before its operands are read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    /// Leaves an address behind to come back to.
    Call,
    /// Goes back to one.
    Return,
    /// Always leaves for somewhere else.
    Jump,
    /// Leaves only if a condition holds, which no static reading knows.
    Conditional,
    /// Falls through to the next instruction.
    Ordinary,
}

impl Kind {
    /// Whether an instruction of this kind ends a basic block.
    ///
    /// A call does not: it comes back, and a block cut at every call would
    /// turn an ordinary function into a column of one-line boxes saying
    /// nothing about its shape. That is x64dbg's reading of a block too.
    #[must_use]
    pub const fn ends_a_block(self) -> bool {
        matches!(self, Self::Return | Self::Jump | Self::Conditional)
    }
}

/// What the mnemonic of an instruction does to the flow.
///
/// Both instruction sets the tool decodes are here: the x86 `j*`/`call`/`ret`
/// family, and the `AArch64` `b`, `bl`, `br`, `blr`, `ret`, the conditional
/// `b.<cond>` spellings, and the compare-and-branch forms.
#[must_use]
pub fn kind(mnemonic: &str) -> Kind {
    let mnemonic = mnemonic.trim();
    if mnemonic.starts_with("ret") || mnemonic == "eret" {
        return Kind::Return;
    }
    if mnemonic.starts_with("call") || matches!(mnemonic, "bl" | "blr") {
        return Kind::Call;
    }
    if mnemonic.starts_with("jmp") || matches!(mnemonic, "b" | "br") {
        return Kind::Jump;
    }
    // `loop` and its two conditional spellings branch on `rcx`, which makes
    // them exactly as conditional as `jne` however little they look it.
    if mnemonic.starts_with('j')
        || mnemonic.starts_with("b.")
        || mnemonic.starts_with("loop")
        || matches!(mnemonic, "cbz" | "cbnz" | "tbz" | "tbnz")
    {
        return Kind::Conditional;
    }
    Kind::Ordinary
}

/// The same, read from the whole text of an instruction.
#[must_use]
pub fn kind_of(text: &str) -> Kind {
    kind(text.split_whitespace().next().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::{Kind, kind, kind_of};

    #[test]
    fn the_x86_family_is_read_as_what_it_does() {
        assert_eq!(kind("call"), Kind::Call);
        assert_eq!(kind("callq"), Kind::Call);
        assert_eq!(kind("ret"), Kind::Return);
        assert_eq!(kind("retq"), Kind::Return);
        assert_eq!(kind("jmp"), Kind::Jump);
        assert_eq!(kind("jmpq"), Kind::Jump);
        assert_eq!(kind("jne"), Kind::Conditional);
        assert_eq!(kind("mov"), Kind::Ordinary);
    }

    /// `loop` branches on `rcx`; read as ordinary it made the last instruction
    /// of a loop body fall out of the block it ends.
    #[test]
    fn loop_is_conditional_however_little_it_looks_it() {
        assert_eq!(kind("loop"), Kind::Conditional);
        assert_eq!(kind("loopne"), Kind::Conditional);
    }

    #[test]
    fn the_aarch64_spellings_are_read_too() {
        assert_eq!(kind("b"), Kind::Jump);
        assert_eq!(kind("br"), Kind::Jump);
        assert_eq!(kind("bl"), Kind::Call);
        assert_eq!(kind("blr"), Kind::Call);
        assert_eq!(kind("b.eq"), Kind::Conditional);
        assert_eq!(kind("cbnz"), Kind::Conditional);
        assert_eq!(kind("tbz"), Kind::Conditional);
        assert_eq!(kind("add"), Kind::Ordinary);
    }

    /// A call comes back, so it does not end a block: cutting at every call
    /// turns an ordinary function into a column of one-line boxes.
    #[test]
    fn only_what_leaves_for_good_ends_a_block() {
        assert!(Kind::Return.ends_a_block());
        assert!(Kind::Jump.ends_a_block());
        assert!(Kind::Conditional.ends_a_block());
        assert!(!Kind::Call.ends_a_block());
        assert!(!Kind::Ordinary.ends_a_block());
    }

    #[test]
    fn a_whole_line_is_read_by_its_first_word() {
        assert_eq!(kind_of("jne 0x401000"), Kind::Conditional);
        assert_eq!(kind_of("mov rax, 1"), Kind::Ordinary);
        assert_eq!(kind_of(""), Kind::Ordinary);
    }
}
