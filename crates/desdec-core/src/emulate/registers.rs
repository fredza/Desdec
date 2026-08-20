//! The registers of the emulated processor.
//!
//! Sixteen general-purpose registers, the instruction pointer, and the flags.
//! They are held as sixty-four bit values and read at the width the operand
//! asks for, because that is what the architecture does: `eax` is not a
//! register, it is the low half of one, and `al` and `ah` are two different
//! halves of that half.
//!
//! One rule of x86-64 is easy to get wrong and is therefore enforced in one
//! place here: **writing a thirty-two bit register clears the top half of the
//! sixty-four bit one**, while writing an eight or sixteen bit register leaves
//! the rest alone. `mov $1,%eax` sets `rax` to 1; `mov $1,%al` does not.

use iced_x86::Register;

/// How many general-purpose registers the emulator holds.
const COUNT: usize = 16;

/// One bit of the flags register, by the name the manuals give it.
///
/// Only the flags an emulated instruction sets or reads are here. The rest of
/// `rflags` — the ones an operating system owns — has no meaning without one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Flag {
    /// Carry: an unsigned result did not fit.
    Carry,
    /// Parity: the low eight bits of the result hold an even number of ones.
    Parity,
    /// Adjust: a carry out of the low four bits, for decimal arithmetic.
    Adjust,
    /// Zero: the result was zero.
    Zero,
    /// Sign: the result's top bit is set.
    Sign,
    /// Direction: string instructions walk backwards.
    Direction,
    /// Overflow: a signed result did not fit.
    Overflow,
}

impl Flag {
    /// Every flag, in the order the interface shows them.
    pub const ALL: [Self; 7] = [
        Self::Carry,
        Self::Parity,
        Self::Adjust,
        Self::Zero,
        Self::Sign,
        Self::Direction,
        Self::Overflow,
    ];

    /// The two-letter name the manuals use.
    #[must_use]
    pub const fn short_name(self) -> &'static str {
        match self {
            Self::Carry => "CF",
            Self::Parity => "PF",
            Self::Adjust => "AF",
            Self::Zero => "ZF",
            Self::Sign => "SF",
            Self::Direction => "DF",
            Self::Overflow => "OF",
        }
    }

    /// Where the flag sits in `rflags`, so the whole register can be shown.
    const fn bit(self) -> u32 {
        match self {
            Self::Carry => 0,
            Self::Parity => 2,
            Self::Adjust => 4,
            Self::Zero => 6,
            Self::Sign => 7,
            Self::Direction => 10,
            Self::Overflow => 11,
        }
    }
}

/// The register file.
#[derive(Clone, Debug)]
pub struct Registers {
    general: [u64; COUNT],
    /// The address of the instruction about to run.
    pub instruction_pointer: u64,
    flags: [bool; 7],
}

impl Default for Registers {
    fn default() -> Self {
        Self::new()
    }
}

impl Registers {
    /// A register file with everything at zero.
    ///
    /// Not "whatever the machine had": a fresh process does not inherit
    /// values, and a register that has never been written holding zero is the
    /// one assumption that makes a run reproducible.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            general: [0; COUNT],
            instruction_pointer: 0,
            flags: [false; 7],
        }
    }

    /// Every general-purpose register, in the architecture's own order, with
    /// the name to show it under.
    pub fn general(&self) -> impl Iterator<Item = (&'static str, u64)> + '_ {
        NAMES
            .iter()
            .zip(self.general.iter())
            .map(|(name, value)| (*name, *value))
    }

    /// Reads a register at the width its name implies.
    ///
    /// A register the emulator does not hold — a segment, a vector register —
    /// reads as zero rather than stopping the run: the instructions that use
    /// them are refused by the interpreter itself, with a better message than
    /// a register file could give.
    #[must_use]
    pub fn get(&self, register: Register) -> u64 {
        if register == Register::RIP || register == Register::EIP {
            return self.instruction_pointer;
        }
        let Some(index) = index_of(register) else {
            return 0;
        };
        let whole = self.general[index];
        if is_high_byte(register) {
            return (whole >> 8) & 0xff;
        }
        match register.size() {
            1 => whole & 0xff,
            2 => whole & 0xffff,
            4 => whole & 0xffff_ffff,
            _ => whole,
        }
    }

    /// Writes a register at the width its name implies, following the rule
    /// that only a thirty-two bit write clears the top half.
    pub fn set(&mut self, register: Register, value: u64) {
        if register == Register::RIP || register == Register::EIP {
            self.instruction_pointer = value;
            return;
        }
        let Some(index) = index_of(register) else {
            return;
        };
        let whole = self.general[index];
        self.general[index] = if is_high_byte(register) {
            (whole & !0xff00) | ((value & 0xff) << 8)
        } else {
            match register.size() {
                1 => (whole & !0xff) | (value & 0xff),
                2 => (whole & !0xffff) | (value & 0xffff),
                // The one that surprises people, and the reason this is not
                // written out at every call site.
                4 => value & 0xffff_ffff,
                _ => value,
            }
        };
    }

    /// The stack pointer, which enough of the emulator reads to deserve a name.
    #[must_use]
    pub fn stack_pointer(&self) -> u64 {
        self.get(Register::RSP)
    }

    /// Moves the stack pointer.
    pub fn set_stack_pointer(&mut self, value: u64) {
        self.set(Register::RSP, value);
    }

    #[must_use]
    pub const fn flag(&self, flag: Flag) -> bool {
        self.flags[flag as usize]
    }

    pub const fn set_flag(&mut self, flag: Flag, value: bool) {
        self.flags[flag as usize] = value;
    }

    /// The flags as the one number `rflags` is, for a view that shows it.
    ///
    /// Bit 1 is always set, as the architecture requires; the flags the
    /// emulator does not model are left at zero rather than invented.
    #[must_use]
    pub fn rflags(&self) -> u64 {
        let mut value = 0b10_u64;
        for flag in Flag::ALL {
            if self.flag(flag) {
                value |= 1 << flag.bit();
            }
        }
        value
    }
}

/// The names of the general-purpose registers, in the architecture's order.
const NAMES: [&str; COUNT] = [
    "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13",
    "r14", "r15",
];

/// Which of the sixteen registers a name refers to, whatever its width.
fn index_of(register: Register) -> Option<usize> {
    if !register.is_gpr() {
        return None;
    }
    let whole = register.full_register();
    let number = whole.number();
    (number < COUNT).then_some(number)
}

/// Whether the name refers to the second byte rather than the first.
const fn is_high_byte(register: Register) -> bool {
    matches!(
        register,
        Register::AH | Register::CH | Register::DH | Register::BH
    )
}
