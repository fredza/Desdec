//! What an instruction leaves in the condition flags, and what it consults.
//!
//! A conditional jump is the one place a listing stops reading top to bottom,
//! and the flags are what decides it. They are also the part of the machine a
//! static tool can say the least about: Desdec never runs the binary, so no
//! flag here ever holds a value. What can be read from the bytes is *who
//! settles what*, and that is what this module answers:
//!
//! - **Which flags this instruction settles**, and how. A `cmp` leaves an
//!   answer in six of them that only the operands decide; an `xor` of a
//!   register with itself is known to clear two of them whatever runs; a `mul`
//!   touches several and leaves them meaningless. Those are different
//!   statements and are kept apart — see [`Outcome`].
//! - **Which flags it consults.** `jne` reads the zero flag and nothing else,
//!   `adc` reads the carry.
//! - **Which instruction last settled a flag**, by reading back through the
//!   preceding instructions, so the reader standing on a `jne` can see the
//!   comparison it belongs to.
//!
//! On x86 the answer comes from the decoder itself rather than from a table
//! written here: the same library that decodes the listing knows, per opcode,
//! which flags are written, cleared, set, read or left undefined. `AArch64`
//! is a short table, because the architecture makes it one — only the
//! flag-setting forms of a handful of instructions touch `NZCV` at all.
//!
//! Reading back for the last write shares the limit of [`crate::operand`]: it
//! follows the listing in address order, which is the executed order only
//! while nothing jumps into the middle of it, and it stops after
//! [`LOOK_BACK`] instructions rather than crossing the whole file.

use iced_x86::{Decoder, DecoderOptions, RflagsBits};

use crate::{
    Architecture,
    analysis::{Analysis, Instruction},
};

/// How far back a flag is followed, for the same reason a register is:
/// reading further crosses more branches, and each one makes the answer less
/// true.
pub const LOOK_BACK: usize = 64;

/// One condition flag.
///
/// The two architectures share four of them under different names — what x86
/// calls the sign flag `AArch64` calls negative — so they are one value here
/// and spelt per architecture by [`Flag::short_name`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Flag {
    Carry,
    Parity,
    Adjust,
    Zero,
    Sign,
    Overflow,
    Direction,
}

impl Flag {
    /// x86, in the order the flags sit in `EFLAGS`.
    const X86: &'static [Self] = &[
        Self::Carry,
        Self::Parity,
        Self::Adjust,
        Self::Zero,
        Self::Sign,
        Self::Overflow,
        Self::Direction,
    ];

    /// `AArch64`, in the order `NZCV` names them.
    const ARM64: &'static [Self] = &[Self::Sign, Self::Zero, Self::Carry, Self::Overflow];

    /// The flags an architecture has, in the order its own manual lists them.
    ///
    /// Empty for an architecture Desdec does not decode: a row of flags drawn
    /// for a file whose instructions it cannot read would be a row of claims
    /// about nothing.
    #[must_use]
    pub const fn of(architecture: Architecture) -> &'static [Self] {
        match architecture {
            Architecture::X86 | Architecture::X86_64 => Self::X86,
            Architecture::Arm64 => Self::ARM64,
            Architecture::Arm | Architecture::Unknown => &[],
        }
    }

    /// How the architecture's own manual writes it, which is what the reader
    /// will have seen in it.
    #[must_use]
    pub const fn short_name(self, architecture: Architecture) -> &'static str {
        match architecture {
            Architecture::Arm64 => match self {
                Self::Sign => "N",
                Self::Zero => "Z",
                Self::Carry => "C",
                Self::Overflow => "V",
                // Not flags `AArch64` has; never reached through
                // [`Flag::of`], and named rather than left blank.
                Self::Parity => "P",
                Self::Adjust => "A",
                Self::Direction => "D",
            },
            _ => match self {
                Self::Carry => "CF",
                Self::Parity => "PF",
                Self::Adjust => "AF",
                Self::Zero => "ZF",
                Self::Sign => "SF",
                Self::Overflow => "OF",
                Self::Direction => "DF",
            },
        }
    }

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// A handful of flags, held in one byte rather than a vector: an effect is
/// computed for every instruction on screen, sixty times a second.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FlagSet(u8);

impl FlagSet {
    /// No flag at all.
    pub const EMPTY: Self = Self(0);

    #[must_use]
    pub const fn contains(self, flag: Flag) -> bool {
        self.0 & flag.bit() != 0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    const fn with(self, flag: Flag) -> Self {
        Self(self.0 | flag.bit())
    }

    fn of(flags: &[Flag]) -> Self {
        flags.iter().fold(Self::EMPTY, |set, flag| set.with(*flag))
    }
}

/// What an instruction leaves in one flag.
///
/// The distinction is the whole point of the module. "Written" and "cleared"
/// look alike in a manual's summary table and are not the same statement at
/// all: one says a value exists that only a run would know, the other says the
/// value is zero however the program got here. Reporting them alike would let
/// a reader take a guess for a fact.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Outcome {
    /// Left exactly as the instruction found it.
    #[default]
    Untouched,
    /// Settled by this instruction, to whatever its operands work out to —
    /// which is not known without running the program.
    Written,
    /// Always zero after this instruction, whatever ran before it.
    Cleared,
    /// Always one after this instruction, whatever ran before it.
    Set,
    /// Touched and left meaningless: the architecture states no value, so
    /// anything read from it afterwards is not an answer.
    Undefined,
}

impl Outcome {
    #[must_use]
    pub const fn touches(self) -> bool {
        !matches!(self, Self::Untouched)
    }
}

/// What one instruction does to the flags.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Effect {
    read: FlagSet,
    written: FlagSet,
    cleared: FlagSet,
    set: FlagSet,
    undefined: FlagSet,
}

impl Effect {
    /// Whether the instruction consults this flag to decide what it does.
    #[must_use]
    pub const fn reads(self, flag: Flag) -> bool {
        self.read.contains(flag)
    }

    /// What it leaves in it.
    #[must_use]
    pub const fn outcome(self, flag: Flag) -> Outcome {
        // A flag known to end at a fixed value is reported as that, in
        // preference to the vaguer statement that something was written.
        if self.cleared.contains(flag) {
            Outcome::Cleared
        } else if self.set.contains(flag) {
            Outcome::Set
        } else if self.undefined.contains(flag) {
            Outcome::Undefined
        } else if self.written.contains(flag) {
            Outcome::Written
        } else {
            Outcome::Untouched
        }
    }

    /// Whether the instruction has anything at all to do with the flags.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.read.is_empty()
            && self.written.is_empty()
            && self.cleared.is_empty()
            && self.set.is_empty()
            && self.undefined.is_empty()
    }
}

/// What this instruction does to the flags.
///
/// Nothing at all for an architecture Desdec cannot decode, and nothing for
/// bytes that do not decode: an instruction whose effect is unknown is
/// reported as touching no flag rather than as clearing them all.
#[must_use]
pub fn effect(instruction: &Instruction, architecture: Architecture) -> Effect {
    match architecture {
        Architecture::X86 => x86(instruction, 32),
        Architecture::X86_64 => x86(instruction, 64),
        Architecture::Arm64 => arm64(&instruction.text),
        Architecture::Arm | Architecture::Unknown => Effect::default(),
    }
}

/// The instruction that last settled a flag, before a given point.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlagWrite {
    pub address: u64,
    pub text: String,
    /// What it left there.
    pub outcome: Outcome,
}

/// Reads back through the listing for the instruction that last settled a
/// flag: the comparison a conditional jump belongs to, usually a row or two
/// above it.
///
/// `None` when nothing within [`LOOK_BACK`] instructions touched it, which is
/// an honest "not found here" and not a claim that nothing ever did.
#[must_use]
pub fn last_write(
    analysis: &Analysis,
    address: u64,
    flag: Flag,
    architecture: Architecture,
) -> Option<FlagWrite> {
    let position = analysis.instruction_index(address)?;

    for instruction in analysis.instructions[..position]
        .iter()
        .rev()
        .take(LOOK_BACK)
    {
        let outcome = effect(instruction, architecture).outcome(flag);
        if !outcome.touches() {
            continue;
        }
        return Some(FlagWrite {
            address: instruction.address,
            text: instruction.text.clone(),
            outcome,
        });
    }
    None
}

/// x86, asked of the decoder rather than of a table: it holds the answer per
/// opcode, and a table written here would be a second, worse copy of it.
fn x86(instruction: &Instruction, bits: u32) -> Effect {
    let mut source = Decoder::with_ip(
        bits,
        instruction.bytes.as_slice(),
        instruction.address,
        DecoderOptions::NONE,
    );
    if !source.can_decode() {
        return Effect::default();
    }
    let decoded = source.decode();
    if decoded.is_invalid() {
        return Effect::default();
    }
    Effect {
        read: from_rflags(decoded.rflags_read()),
        written: from_rflags(decoded.rflags_written()),
        cleared: from_rflags(decoded.rflags_cleared()),
        set: from_rflags(decoded.rflags_set()),
        undefined: from_rflags(decoded.rflags_undefined()),
    }
}

/// The decoder's own bit layout, in this module's terms.
fn from_rflags(bits: u32) -> FlagSet {
    const PAIRS: &[(u32, Flag)] = &[
        (RflagsBits::CF, Flag::Carry),
        (RflagsBits::PF, Flag::Parity),
        (RflagsBits::AF, Flag::Adjust),
        (RflagsBits::ZF, Flag::Zero),
        (RflagsBits::SF, Flag::Sign),
        (RflagsBits::OF, Flag::Overflow),
        (RflagsBits::DF, Flag::Direction),
    ];
    PAIRS
        .iter()
        .filter(|(bit, _)| bits & bit != 0)
        .fold(FlagSet::EMPTY, |set, (_, flag)| set.with(*flag))
}

/// `AArch64`, from the mnemonic.
///
/// Short on purpose: the architecture settles `NZCV` together or not at all,
/// and only through the flag-setting forms — the `s` suffix, the comparisons,
/// and the two instructions that move the whole register. Everything else
/// leaves the flags alone, so a mnemonic this does not recognise is reported
/// as touching nothing, which is the truth for the overwhelming majority.
fn arm64(text: &str) -> Effect {
    let mnemonic = text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let all = FlagSet::of(Flag::ARM64);

    // A conditional branch names its condition after the dot; the conditional
    // select and set families name it as their last operand.
    let condition = if let Some(after_the_dot) = mnemonic.strip_prefix("b.") {
        after_the_dot.trim()
    } else if reads_a_condition(&mnemonic) {
        text.rsplit(',').next().unwrap_or_default().trim()
    } else {
        ""
    };
    let read = condition_flags(condition);

    // `ccmp` and `ccmn` are the two that do both: they read a condition to
    // decide whether to compare, and settle the flags either way.
    let writes = matches!(
        mnemonic.as_str(),
        "cmp"
            | "cmn"
            | "tst"
            | "ccmp"
            | "ccmn"
            | "adds"
            | "subs"
            | "ands"
            | "bics"
            | "adcs"
            | "sbcs"
            | "negs"
            | "ngcs"
    ) || moves_the_whole_register(text, &mnemonic, true);

    Effect {
        read: if moves_the_whole_register(text, &mnemonic, false) {
            all
        } else {
            read
        },
        written: if writes { all } else { FlagSet::EMPTY },
        ..Effect::default()
    }
}

/// Whether a mnemonic decides what it does from the condition flags.
fn reads_a_condition(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "csel"
            | "csinc"
            | "csinv"
            | "csneg"
            | "cset"
            | "csetm"
            | "cinc"
            | "cinv"
            | "cneg"
            | "ccmp"
            | "ccmn"
    )
}

/// `msr nzcv, x0` writes the four flags at once, and `mrs x0, nzcv` reads
/// them: the two ways a program handles them as a value rather than as a
/// condition.
fn moves_the_whole_register(text: &str, mnemonic: &str, writing: bool) -> bool {
    if mnemonic != if writing { "msr" } else { "mrs" } {
        return false;
    }
    text.to_ascii_lowercase().contains("nzcv")
}

/// The flags an `AArch64` condition consults.
fn condition_flags(condition: &str) -> FlagSet {
    use Flag::{Carry, Overflow, Sign, Zero};

    let flags: &[Flag] = match condition {
        "eq" | "ne" => &[Zero],
        "cs" | "hs" | "cc" | "lo" => &[Carry],
        "mi" | "pl" => &[Sign],
        "vs" | "vc" => &[Overflow],
        "hi" | "ls" => &[Carry, Zero],
        "ge" | "lt" => &[Sign, Overflow],
        "gt" | "le" => &[Sign, Zero, Overflow],
        // `al` and `nv` are unconditional, and anything else is not a
        // condition at all.
        _ => &[],
    };
    FlagSet::of(flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::InstructionBytes;
    use std::sync::Arc;

    fn instruction(bytes: &[u8], text: &str) -> Instruction {
        Instruction {
            address: 0x0040_1000,
            bytes: InstructionBytes::new(bytes).expect("short enough"),
            text: text.to_owned(),
            section: Arc::from(".text"),
        }
    }

    /// A comparison settles six flags to values only the operands decide, and
    /// the jump below it consults exactly one of them. That pairing is what
    /// the whole feature exists to show.
    #[test]
    fn a_comparison_settles_what_the_jump_below_it_reads() {
        // cmp %esi,%edi
        let compare = effect(
            &instruction(&[0x39, 0xf7], "cmp %esi,%edi"),
            Architecture::X86_64,
        );
        for flag in [
            Flag::Carry,
            Flag::Parity,
            Flag::Adjust,
            Flag::Zero,
            Flag::Sign,
            Flag::Overflow,
        ] {
            assert_eq!(compare.outcome(flag), Outcome::Written, "{flag:?}");
        }
        assert_eq!(compare.outcome(Flag::Direction), Outcome::Untouched);

        // jne, two bytes further on
        let jump = effect(
            &instruction(&[0x75, 0x10], "jne 0x401014"),
            Architecture::X86_64,
        );
        assert!(jump.reads(Flag::Zero));
        assert!(!jump.reads(Flag::Carry));
        assert_eq!(
            jump.outcome(Flag::Zero),
            Outcome::Untouched,
            "a jump decides nothing about the flags"
        );
    }

    /// `xor %eax,%eax` is the compiler's way of writing zero, and two of the
    /// flags it leaves are known whatever ran before it. Saying only that they
    /// were "written" would hide a fact the bytes really do state.
    #[test]
    fn a_flag_known_to_end_at_zero_is_not_merely_written() {
        let cleared = effect(
            &instruction(&[0x31, 0xc0], "xor %eax,%eax"),
            Architecture::X86_64,
        );

        assert_eq!(cleared.outcome(Flag::Carry), Outcome::Cleared);
        assert_eq!(cleared.outcome(Flag::Overflow), Outcome::Cleared);
        // The decoder knows this form always yields zero, so the zero flag
        // is not merely written either: it ends at one.
        assert_eq!(cleared.outcome(Flag::Zero), Outcome::Set);
        // The manual states no value for it, and neither does Desdec.
        assert_eq!(cleared.outcome(Flag::Adjust), Outcome::Undefined);
    }

    /// The overwhelming majority of instructions leave the flags alone, and
    /// must be reported as doing so rather than as unknown.
    #[test]
    fn a_move_touches_nothing() {
        let moved = effect(
            &instruction(&[0x48, 0x89, 0xc3], "mov %rax,%rbx"),
            Architecture::X86_64,
        );
        assert!(moved.is_empty());
    }

    /// Bytes that decode to nothing, and an architecture with no decoder, are
    /// both reported as no effect: an invented one would be a claim about an
    /// instruction nobody could read.
    #[test]
    fn what_cannot_be_decoded_claims_nothing() {
        assert!(effect(&instruction(&[0xff, 0xff], "(bad)"), Architecture::X86_64).is_empty());
        assert!(effect(&instruction(&[0x90], "nop"), Architecture::Arm).is_empty());
        assert!(Flag::of(Architecture::Unknown).is_empty());
    }

    /// `AArch64` reads from the mnemonic: the flag-setting forms settle all
    /// four, a conditional branch reads what its condition needs, and an
    /// ordinary instruction touches nothing.
    #[test]
    fn arm64_reads_the_flag_setting_forms() {
        let compare = arm64("cmp w0, w1");
        for flag in Flag::ARM64 {
            assert_eq!(compare.outcome(*flag), Outcome::Written, "{flag:?}");
        }

        let branch = arm64("b.gt #0x4008");
        assert!(
            branch.reads(Flag::Sign) && branch.reads(Flag::Zero) && branch.reads(Flag::Overflow)
        );
        assert!(!branch.reads(Flag::Carry));

        let select = arm64("csel x0, x1, x2, eq");
        assert!(select.reads(Flag::Zero));
        assert!(!select.reads(Flag::Sign));

        assert!(arm64("add x0, x1, x2").is_empty(), "no `s`, no flags");
        assert!(arm64("b #0x4008").is_empty(), "an unconditional branch");
        assert!(arm64("mrs x0, nzcv").reads(Flag::Zero), "read as a value");
        assert_eq!(arm64("msr nzcv, x0").outcome(Flag::Zero), Outcome::Written);
    }

    /// The names are the ones the reader saw in the architecture's manual:
    /// the same flag is `SF` in one and `N` in the other.
    #[test]
    fn each_architecture_spells_its_own_flags() {
        assert_eq!(Flag::Sign.short_name(Architecture::X86_64), "SF");
        assert_eq!(Flag::Sign.short_name(Architecture::Arm64), "N");
        assert_eq!(Flag::of(Architecture::Arm64).len(), 4);
        assert_eq!(Flag::of(Architecture::X86).len(), 7);
    }
}
