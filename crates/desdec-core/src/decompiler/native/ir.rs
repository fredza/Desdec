//! What one instruction does, said once and without a machine in the way.
//!
//! A listing is a sequence of instructions; C is a tree of expressions. Nothing
//! turns one into the other in a single step, and every decompiler that tries
//! ends up where Desdec's line-by-line translation ended up: `rax = rbx;` under
//! `mov`, and a comment where a condition should be. The missing piece is a
//! representation that is neither — small enough that a lifter can be honest
//! about it, structured enough that an emitter can print C from it.
//!
//! That is what is here. Three ideas and nothing else:
//!
//! - **A [`Place`] is something a value can be written to** — a register, a
//!   slot of the frame, a location in memory, one question the flags answer.
//! - **An [`Expr`] is a value being read.** Places read back as values, and so
//!   do constants, arithmetic, and the result of a call.
//! - **A [`Stmt`] is one effect**, and an instruction becomes as many of them
//!   as it really has: `push %rbp` is two, `cmp` is one per question it
//!   settles, and `syscall` is one that says it is a system call and nothing
//!   more.
//!
//! Two properties are deliberate and are what the rest of the pipeline rests
//! on.
//!
//! **The conditions are ordinary places.** On the machine the flags are a
//! special register that only a jump reads; here `rax < rbx` is written by
//! `cmp` like any other place and read by `jl` like any other value. That is
//! what lets the dataflow pass turn a comparison and the branch below it into
//! `if (a <= b)` by the same substitution it uses everywhere else, rather than
//! by a table of instruction pairs. [`Condition`] says why the questions are
//! recorded and not the four flags behind them.
//!
//! **What is not modelled says so.** An instruction the lifter does not know
//! becomes [`Stmt::Opaque`], which carries the assembly verbatim and is
//! understood by every later pass as *an effect of unknown extent*. Nothing
//! is propagated across one, nothing is deleted around one, and the emitter
//! prints it as a comment holding the original text. A decompiler that
//! silently drops what it cannot read produces C that looks complete and is
//! wrong; this one produces C with a hole in it, and the hole is labelled.

use std::fmt;

/// How many bytes a value occupies.
///
/// The widths a general register is addressed at, and the one an SSE register
/// is. Anything wider is not lifted, so there is nothing to name.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Width {
    Byte,
    Word,
    Dword,
    Qword,
    Xmm,
}

impl Width {
    #[must_use]
    pub const fn bytes(self) -> u64 {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
            Self::Dword => 4,
            Self::Qword => 8,
            Self::Xmm => 16,
        }
    }

    #[must_use]
    pub const fn bits(self) -> u32 {
        match self {
            Self::Byte => 8,
            Self::Word => 16,
            Self::Dword => 32,
            Self::Qword => 64,
            Self::Xmm => 128,
        }
    }

    /// The width a byte count names, when it names one.
    #[must_use]
    pub const fn of_bytes(bytes: u64) -> Option<Self> {
        match bytes {
            1 => Some(Self::Byte),
            2 => Some(Self::Word),
            4 => Some(Self::Dword),
            8 => Some(Self::Qword),
            16 => Some(Self::Xmm),
            _ => None,
        }
    }

    /// The C spelling of an integer of this width, signed or not.
    ///
    /// The fixed-width names rather than `int` and `long`: the whole point of
    /// the listing is that a value is thirty-two bits wide, and `int` says
    /// that only on the machines where it happens to be true.
    #[must_use]
    pub const fn c_name(self, signed: bool) -> &'static str {
        match (self, signed) {
            (Self::Byte, true) => "int8_t",
            (Self::Byte, false) => "uint8_t",
            (Self::Word, true) => "int16_t",
            (Self::Word, false) => "uint16_t",
            (Self::Dword, true) => "int32_t",
            (Self::Dword, false) => "uint32_t",
            (Self::Qword, true) => "int64_t",
            (Self::Qword, false) => "uint64_t",
            // No signedness to speak of: what is in it depends entirely on the
            // instruction that put it there.
            (Self::Xmm, _) => "__m128",
        }
    }
}

/// A register, by the whole of it and the part being addressed.
///
/// `%eax`, `%ax` and `%al` are not three registers; they are three windows
/// onto `rax`, and a decompiler that treats them as separate names loses track
/// of a value the moment a compiler narrows it — which compilers do constantly,
/// because a 32-bit operation is a byte shorter to encode. So the whole
/// register is the identity and the width is an attribute of the access.
///
/// `%ah` and its three siblings are the exception the encoding forces: they
/// address the *second* byte, which no `width` can express. They carry
/// [`Register::high_byte`], and the passes that would rewrite a value through
/// a register refuse to cross one rather than pretend it is `%al`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Register {
    /// The 64-bit name — `rax`, `r12`, `xmm0` — whatever width was written.
    pub root: &'static str,
    /// The width the instruction addressed it at.
    pub width: Width,
    /// Set for `%ah`, `%bh`, `%ch`, `%dh`.
    pub high_byte: bool,
}

impl Register {
    #[must_use]
    pub const fn new(root: &'static str, width: Width) -> Self {
        Self {
            root,
            width,
            high_byte: false,
        }
    }

    /// Whether this access covers the whole register, and so whether writing
    /// it destroys everything that was there.
    ///
    /// A 32-bit write does, on x86-64, because the architecture zeroes the top
    /// half — which is why `xor %eax,%eax` clears all sixty-four bits and is
    /// how every compiler writes zero. An 8- or 16-bit write does not: the
    /// rest of the register survives it, and a pass that assumed otherwise
    /// would erase a value that is still live.
    #[must_use]
    pub const fn covers_root(self) -> bool {
        matches!(self.width, Width::Qword | Width::Dword | Width::Xmm) && !self.high_byte
    }

    /// The name the architecture gives this window, for printing.
    #[must_use]
    pub fn name(self) -> String {
        if self.high_byte {
            return match self.root {
                "rax" => "ah".to_owned(),
                "rbx" => "bh".to_owned(),
                "rcx" => "ch".to_owned(),
                "rdx" => "dh".to_owned(),
                other => other.to_owned(),
            };
        }
        if self.root.starts_with("xmm") {
            return self.root.to_owned();
        }
        // `r8`..`r15` narrow with a suffix; the older eight change their
        // prefix, and the four named after their purpose lose theirs entirely
        // at eight bits.
        if let Some(number) = self.root.strip_prefix('r')
            && number.chars().all(|character| character.is_ascii_digit())
        {
            return match self.width {
                Width::Qword => self.root.to_owned(),
                Width::Dword => format!("{}d", self.root),
                Width::Word => format!("{}w", self.root),
                _ => format!("{}b", self.root),
            };
        }
        let stem = &self.root[1..];
        match self.width {
            Width::Qword | Width::Xmm => self.root.to_owned(),
            Width::Dword => format!("e{stem}"),
            Width::Word => stem.to_owned(),
            Width::Byte => match self.root {
                "rax" | "rbx" | "rcx" | "rdx" => format!("{}l", &stem[..1]),
                _ => format!("{stem}l"),
            },
        }
    }
}

/// A question the last arithmetic instruction left an answer to.
///
/// **Not a flag of the processor**, and the difference is the reason this
/// pipeline produces `if (count <= 8)` where the old line-by-line translation
/// produced `if (/* jle condition from flags */)`.
///
/// The machine has `ZF`, `SF`, `CF` and `OF`, and the signed comparisons are
/// combinations of them: `jl` branches on `SF ≠ OF`, `jle` on `ZF | (SF ≠ OF)`.
/// Writing those four flags out and letting the dataflow pass substitute them
/// gives, for `cmp %rbx,%rax` followed by `jl`, the expression
/// `((rax - rbx) < 0) != overflow(rax - rbx)` — which is exactly true, exactly
/// useless to read, and no simplifier of a reasonable size turns it back into
/// `rax < rbx`.
///
/// So what is recorded is the *combination*, one place per question a branch
/// can ask. `cmp %rbx,%rax` writes `Less := rax < rbx` directly, `jl` reads
/// `Less`, and the ordinary substitution every other value goes through does
/// the rest. Ghidra reaches the same place from the other end, with an
/// `SBORROW` primitive its simplifier knows how to fold.
///
/// [`Condition::Sign`], [`Condition::Carry`], [`Condition::Overflow`] and
/// [`Condition::Parity`] stay, because `js`, `jb`, `jo` and `jp` ask for them
/// on their own.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Condition {
    /// `ZF` — the two sides were equal, or the result was zero.
    Zero,
    /// `SF` — the result was negative read as signed.
    Sign,
    /// `CF` — the subtraction borrowed, which for a comparison means the left
    /// side is below the right read as unsigned.
    Carry,
    /// `OF` — the signed result did not fit.
    Overflow,
    /// `PF` — the low byte has an even number of bits set.
    Parity,
    /// `SF ≠ OF` — strictly less, signed. What `jl` and `jge` ask.
    Less,
    /// `ZF | (SF ≠ OF)` — less or equal, signed. What `jle` and `jg` ask.
    LessOrEqual,
    /// `CF | ZF` — below or equal, unsigned. What `jbe` and `ja` ask.
    BelowOrEqual,
}

impl Condition {
    /// How it is printed when nothing settled it in reach — the honest
    /// remainder, spelt the way the manual spells the flags behind it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Zero => "ZF",
            Self::Sign => "SF",
            Self::Carry => "CF",
            Self::Overflow => "OF",
            Self::Parity => "PF",
            Self::Less => "SF_ne_OF",
            Self::LessOrEqual => "ZF_or_SF_ne_OF",
            Self::BelowOrEqual => "CF_or_ZF",
        }
    }

    /// Every condition an arithmetic instruction settles, so a lifter can
    /// invalidate the lot and fill in the ones it can state.
    pub const ALL: &'static [Self] = &[
        Self::Zero,
        Self::Sign,
        Self::Carry,
        Self::Overflow,
        Self::Parity,
        Self::Less,
        Self::LessOrEqual,
        Self::BelowOrEqual,
    ];
}

/// Somewhere a value can be written.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Place {
    Register(Register),
    /// One question the flags answer, settled by an arithmetic instruction and
    /// consulted by a branch. See [`Condition`] for why the questions are
    /// places and the flags themselves are not.
    Condition(Condition),
    /// `*(width *)address`, the address being an arbitrary expression.
    Memory { address: Box<Expr>, width: Width },
    /// A slot of the frame, once [`super::naming`] has recognised one. The
    /// lifter never produces this: it emits the memory access the instruction
    /// wrote, and naming turns the ones that are frame-relative into locals.
    Local { id: u32, width: Width },
}

impl Place {
    #[must_use]
    pub const fn width(&self) -> Width {
        match self {
            Self::Register(register) => register.width,
            // A condition is one bit; a byte is the narrowest thing C will
            // hold it in, and every use of it is a test anyway.
            Self::Condition(_) => Width::Byte,
            Self::Memory { width, .. } | Self::Local { width, .. } => *width,
        }
    }

    /// Whether writing here may change what reading `other` gives.
    ///
    /// Deliberately pessimistic about memory: two addresses that are not
    /// literally the same expression are assumed to be able to overlap,
    /// because deciding otherwise is the aliasing problem and getting it wrong
    /// moves a load across the store that fed it. Registers are exact — they
    /// alias exactly when they share a root.
    #[must_use]
    pub fn may_clobber(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Register(written), Self::Register(read)) => written.root == read.root,
            (Self::Condition(written), Self::Condition(read)) => written == read,
            (Self::Local { id: written, .. }, Self::Local { id: read, .. }) => written == read,
            // A write to memory can reach a local — the frame is memory — and
            // a write to a local is a write to memory.
            (Self::Memory { .. }, Self::Memory { .. } | Self::Local { .. })
            | (Self::Local { .. }, Self::Memory { .. }) => true,
            _ => false,
        }
    }
}

/// An operator taking one value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Unary {
    Negate,
    Not,
    /// `!value`, which is how a flag read becomes a condition.
    LogicalNot,
}

/// An operator taking two.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Binary {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    And,
    Or,
    Xor,
    ShiftLeft,
    ShiftRight,
    /// Arithmetic shift right, which keeps the sign.
    ShiftRightSigned,
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    /// The unsigned comparisons, which x86 spells `jb`/`ja` and C spells with
    /// the same operators over unsigned operands. Kept apart so the emitter
    /// can cast rather than print a comparison that means the other thing.
    Below,
    BelowOrEqual,
    Above,
    AboveOrEqual,
    LogicalAnd,
    LogicalOr,
}

impl Binary {
    /// How C writes it, and how tightly it binds.
    ///
    /// The precedence is C's own, so the emitter can leave out the
    /// parentheses that would only be noise and keep the ones that change the
    /// meaning.
    #[must_use]
    pub const fn spelling(self) -> (&'static str, u8) {
        match self {
            Self::Multiply => ("*", 10),
            Self::Divide => ("/", 10),
            Self::Modulo => ("%", 10),
            Self::Add => ("+", 9),
            Self::Subtract => ("-", 9),
            Self::ShiftLeft => ("<<", 8),
            Self::ShiftRight | Self::ShiftRightSigned => (">>", 8),
            Self::Less | Self::Below => ("<", 7),
            Self::LessOrEqual | Self::BelowOrEqual => ("<=", 7),
            Self::Greater | Self::Above => (">", 7),
            Self::GreaterOrEqual | Self::AboveOrEqual => (">=", 7),
            Self::Equal => ("==", 6),
            Self::NotEqual => ("!=", 6),
            Self::And => ("&", 5),
            Self::Xor => ("^", 4),
            Self::Or => ("|", 3),
            Self::LogicalAnd => ("&&", 2),
            Self::LogicalOr => ("||", 1),
        }
    }

    /// Whether the result is a yes or a no rather than a number.
    #[must_use]
    pub const fn is_predicate(self) -> bool {
        matches!(
            self,
            Self::Equal
                | Self::NotEqual
                | Self::Less
                | Self::LessOrEqual
                | Self::Greater
                | Self::GreaterOrEqual
                | Self::Below
                | Self::BelowOrEqual
                | Self::Above
                | Self::AboveOrEqual
                | Self::LogicalAnd
                | Self::LogicalOr
        )
    }

    /// Whether the two sides read as unsigned, which is what `jb` and `ja`
    /// mean and what C needs a cast to say.
    #[must_use]
    pub const fn is_unsigned_comparison(self) -> bool {
        matches!(
            self,
            Self::Below | Self::BelowOrEqual | Self::Above | Self::AboveOrEqual
        )
    }

    /// The comparison that holds exactly when this one does not.
    ///
    /// What lets a branch be turned around: a compiler emits `jne` over the
    /// body it wants to skip, and the `if` a reader wants is the one testing
    /// the opposite.
    #[must_use]
    pub const fn negated(self) -> Option<Self> {
        Some(match self {
            Self::Equal => Self::NotEqual,
            Self::NotEqual => Self::Equal,
            Self::Less => Self::GreaterOrEqual,
            Self::LessOrEqual => Self::Greater,
            Self::Greater => Self::LessOrEqual,
            Self::GreaterOrEqual => Self::Less,
            Self::Below => Self::AboveOrEqual,
            Self::BelowOrEqual => Self::Above,
            Self::Above => Self::BelowOrEqual,
            Self::AboveOrEqual => Self::Below,
            _ => return None,
        })
    }
}

/// Where a call goes.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Callee {
    /// A name — a function of this file, or one it imports.
    Named(String),
    /// An address nothing names.
    Address(u64),
    /// Through a register or a memory location: a virtual call, a callback, a
    /// table of handlers. What it reads is kept, because that is the only
    /// thing the file says about where it goes.
    Indirect(Box<Expr>),
}

/// A value being read.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Expr {
    /// A number, with the width it was written at so the emitter can print
    /// `-1` rather than `0xFFFFFFFF` where that is what it means.
    Const { value: u64, width: Width },
    /// Reading a place.
    Read(Box<Place>),
    /// `&place`, which is what `lea` computes and what taking the address of a
    /// local produces.
    AddressOf(Box<Place>),
    Unary {
        operator: Unary,
        operand: Box<Expr>,
    },
    Binary {
        operator: Binary,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// A widening or narrowing conversion, `signed` saying which of the two
    /// x86 spellings — `movzx` or `movsx` — asked for it.
    Cast {
        value: Box<Expr>,
        width: Width,
        signed: bool,
    },
    /// A call in a value position, which is what a call whose result is used
    /// becomes once its return register has been propagated into the use.
    Call {
        callee: Callee,
        arguments: Vec<Expr>,
    },
    /// `condition ? when_true : when_false`.
    ///
    /// What a conditional move is. `cmov` cannot be lifted to a branch without
    /// inventing two basic blocks the function does not have — which would
    /// change the shape the graph view draws — and it cannot be lifted to a
    /// plain assignment without claiming it always happens. C has the operator
    /// that says exactly this, so the IR does too.
    Select {
        condition: Box<Expr>,
        when_true: Box<Expr>,
        when_false: Box<Expr>,
    },
    /// A named constant of the file — the address of a string, of a global —
    /// carrying both what to print and the number behind it.
    Symbol { name: String, address: u64 },
    /// Something the lifter modelled no further. Never rewritten, never
    /// evaluated; printed as it stands.
    Unknown(String),
}

impl Expr {
    #[must_use]
    pub fn constant(value: u64, width: Width) -> Self {
        Self::Const { value, width }
    }

    #[must_use]
    pub fn read(place: Place) -> Self {
        Self::Read(Box::new(place))
    }

    #[must_use]
    pub fn register(register: Register) -> Self {
        Self::read(Place::Register(register))
    }

    #[must_use]
    pub fn binary(operator: Binary, left: Self, right: Self) -> Self {
        Self::Binary {
            operator,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    #[must_use]
    pub fn unary(operator: Unary, operand: Self) -> Self {
        Self::Unary {
            operator,
            operand: Box::new(operand),
        }
    }

    /// The width this value has, when it has a definite one.
    #[must_use]
    pub fn width(&self) -> Option<Width> {
        match self {
            Self::Const { width, .. } | Self::Cast { width, .. } => Some(*width),
            Self::Read(place) => Some(place.width()),
            Self::AddressOf(_) | Self::Symbol { .. } => Some(Width::Qword),
            Self::Unary { operand, .. } => operand.width(),
            Self::Binary {
                operator,
                left,
                right,
            } => {
                if operator.is_predicate() {
                    Some(Width::Byte)
                } else {
                    left.width().or_else(|| right.width())
                }
            }
            Self::Select {
                when_true,
                when_false,
                ..
            } => when_true.width().or_else(|| when_false.width()),
            Self::Call { .. } | Self::Unknown(_) => None,
        }
    }

    /// Whether reading this value could give a different answer after `place`
    /// is written.
    ///
    /// The question every rewrite has to ask before moving an expression past
    /// an assignment. An [`Expr::Unknown`] answers yes to everything, which is
    /// what stops anything being moved across an instruction nobody modelled.
    #[must_use]
    pub fn depends_on(&self, place: &Place) -> bool {
        match self {
            Self::Const { .. } | Self::Symbol { .. } => false,
            // An unmodelled instruction is an effect of unbounded extent, and
            // a call reads and writes what its body does — which is not
            // something this file states. Both answer yes to everything, which
            // is what stops any rewrite crossing them.
            Self::Unknown(_) | Self::Call { .. } => true,
            Self::Read(read) => place.may_clobber(read),
            // The address of a place is arithmetic on the frame pointer, not a
            // reading of what is in it — but the arithmetic still names a
            // register, so a memory place's own address must be followed.
            Self::AddressOf(read) => match read.as_ref() {
                Place::Memory { address, .. } => address.depends_on(place),
                Place::Register(_) | Place::Condition(_) | Place::Local { .. } => false,
            },
            Self::Unary { operand, .. } => operand.depends_on(place),
            Self::Binary { left, right, .. } => {
                left.depends_on(place) || right.depends_on(place)
            }
            Self::Cast { value, .. } => value.depends_on(place),
            Self::Select {
                condition,
                when_true,
                when_false,
            } => {
                condition.depends_on(place)
                    || when_true.depends_on(place)
                    || when_false.depends_on(place)
            }
        }
    }

    /// Whether reading it twice, or not at all, changes the program.
    ///
    /// A call does; everything else here is a reading of state. This is what
    /// decides whether a dead assignment may be deleted outright or must be
    /// kept for what computing it does.
    #[must_use]
    pub fn has_effects(&self) -> bool {
        match self {
            // A call has them. An unmodelled instruction is unbounded by
            // definition, which for every pass that asks this question means
            // the same thing: leave it where it is.
            Self::Call { .. } | Self::Unknown(_) => true,
            Self::Const { .. } | Self::Read(_) | Self::AddressOf(_) | Self::Symbol { .. } => false,
            Self::Unary { operand, .. } => operand.has_effects(),
            Self::Binary { left, right, .. } => left.has_effects() || right.has_effects(),
            Self::Cast { value, .. } => value.has_effects(),
            Self::Select {
                condition,
                when_true,
                when_false,
            } => condition.has_effects() || when_true.has_effects() || when_false.has_effects(),
        }
    }

    /// How many places this expression reads, all told.
    ///
    /// Used as a size: an expression substituted into another grows the line
    /// it lands on, and past a point one line of C holding nine nested reads
    /// is less readable than the two lines it came from.
    #[must_use]
    pub fn complexity(&self) -> usize {
        match self {
            Self::Const { .. } | Self::Symbol { .. } | Self::Unknown(_) => 1,
            Self::Read(place) | Self::AddressOf(place) => match place.as_ref() {
                Place::Memory { address, .. } => 1 + address.complexity(),
                _ => 1,
            },
            Self::Unary { operand, .. } => 1 + operand.complexity(),
            Self::Binary { left, right, .. } => 1 + left.complexity() + right.complexity(),
            Self::Cast { value, .. } => 1 + value.complexity(),
            Self::Call { arguments, .. } => {
                1 + arguments.iter().map(Self::complexity).sum::<usize>()
            }
            Self::Select {
                condition,
                when_true,
                when_false,
            } => 1 + condition.complexity() + when_true.complexity() + when_false.complexity(),
        }
    }
}

/// One effect, and the address of the instruction it came from.
///
/// The address is on every statement rather than on a group of them, and that
/// is the whole reason the view built on this can do what an external
/// decompiler cannot: every line of the C it prints knows which instruction it
/// came from, so clicking it goes somewhere. `RetDec` and rz-ghidra publish no
/// such map, which is why the existing view can only offer a button naming the
/// whole function.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Statement {
    pub address: u64,
    pub effect: Stmt,
}

impl Statement {
    #[must_use]
    pub const fn new(address: u64, effect: Stmt) -> Self {
        Self { address, effect }
    }
}

/// What an effect is.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Stmt {
    /// `place = value;`
    Assign { place: Place, value: Expr },
    /// A call whose result is either used through `result` or dropped.
    Call {
        result: Option<Place>,
        callee: Callee,
        arguments: Vec<Expr>,
    },
    /// Leaving the function, with the value in the return register when one is
    /// live and nothing when the function returns void.
    Return(Option<Expr>),
    /// Going somewhere else in this function, on a condition or unconditionally.
    ///
    /// Structuring consumes these: what survives it is a `goto`, and there are
    /// few of those.
    Branch {
        condition: Option<Expr>,
        target: u64,
    },
    /// A jump through a value — a `switch` table, a function pointer — which
    /// structuring cannot follow and the emitter prints as what it is.
    IndirectBranch(Expr),
    /// An instruction that was decoded and not modelled. Carries its own text.
    Opaque(String),
    /// A system call, which is a call to the kernel and not to this file.
    /// `number` is filled in when the code visibly loaded one.
    SystemCall { number: Option<u64> },
    /// An instruction with no effect worth printing — `nop`, `endbr64`, the
    /// `xchg %ax,%ax` a compiler pads with.
    Nothing,
}

impl fmt::Display for Register {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_register_narrows_to_the_name_the_architecture_gives_it() {
        let rax = Register::new("rax", Width::Qword);
        assert_eq!(rax.name(), "rax");
        assert_eq!(Register::new("rax", Width::Dword).name(), "eax");
        assert_eq!(Register::new("rax", Width::Word).name(), "ax");
        assert_eq!(Register::new("rax", Width::Byte).name(), "al");
        assert_eq!(Register::new("rsi", Width::Byte).name(), "sil");
        assert_eq!(Register::new("r12", Width::Dword).name(), "r12d");
        assert_eq!(Register::new("r12", Width::Byte).name(), "r12b");
    }

    /// The one thing about x86-64 that a decompiler cannot get wrong without
    /// producing code that is subtly false: `xor %eax,%eax` clears all
    /// sixty-four bits, and `mov $1,%al` clears none of the other fifty-six.
    #[test]
    fn a_thirty_two_bit_write_covers_the_register_and_an_eight_bit_one_does_not() {
        assert!(Register::new("rax", Width::Dword).covers_root());
        assert!(Register::new("rax", Width::Qword).covers_root());
        assert!(!Register::new("rax", Width::Byte).covers_root());
        assert!(!Register::new("rax", Width::Word).covers_root());
    }

    #[test]
    fn the_windows_of_one_register_clobber_each_other() {
        let eax = Place::Register(Register::new("rax", Width::Dword));
        let al = Place::Register(Register::new("rax", Width::Byte));
        let rbx = Place::Register(Register::new("rbx", Width::Qword));
        assert!(eax.may_clobber(&al));
        assert!(!eax.may_clobber(&rbx));
    }

    /// Two addresses that are not the same expression may be the same
    /// location, and a decompiler that decides otherwise moves a load across
    /// the store that fed it.
    #[test]
    fn any_write_to_memory_is_assumed_to_reach_any_other() {
        let one = Place::Memory {
            address: Box::new(Expr::constant(0x1000, Width::Qword)),
            width: Width::Qword,
        };
        let other = Place::Memory {
            address: Box::new(Expr::constant(0x2000, Width::Qword)),
            width: Width::Qword,
        };
        assert!(one.may_clobber(&other));
    }

    #[test]
    fn nothing_is_moved_across_an_instruction_nobody_modelled() {
        let unknown = Expr::Unknown("fldt 0x1FDE0".to_owned());
        let anything = Place::Register(Register::new("rbx", Width::Qword));
        assert!(unknown.depends_on(&anything));
        assert!(unknown.has_effects());
    }

    #[test]
    fn a_branch_can_be_turned_around() {
        assert_eq!(Binary::NotEqual.negated(), Some(Binary::Equal));
        assert_eq!(Binary::BelowOrEqual.negated(), Some(Binary::Above));
        assert_eq!(Binary::Add.negated(), None);
    }
}
