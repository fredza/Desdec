//! From one decoded instruction to what it does.
//!
//! The listing is already there — `analysis::disassembly` decodes every
//! executable byte and formats it in AT&T syntax — so this reads that text
//! rather than the bytes. That is a deliberate choice and it has one cost and
//! two benefits. The cost is a parser for the syntax. The benefits are that
//! the decompiler and the listing can never disagree about what an instruction
//! *is*, which is the failure a reader has no way to see past, and that the
//! whole pipeline works on anything the listing can show.
//!
//! # What is modelled, and what is not
//!
//! The integer core of x86-64: moves, arithmetic, logic, shifts, the stack,
//! comparisons, branches, calls, returns, conditional moves and set-on-
//! condition, the sign-extension instructions, and the two system-call
//! spellings. That is nearly all of what a compiler emits for ordinary code.
//!
//! Everything else — the x87 stack, the string instructions, most of SSE, the
//! atomics — becomes [`Stmt::Opaque`] carrying its own text. This is the
//! module's central honesty: an instruction that is not understood is *said*
//! not to be understood, and every later pass treats it as an effect of
//! unbounded extent. Nothing is propagated across it and nothing around it is
//! deleted. C with a labelled hole in it can be read; C that quietly omits an
//! instruction cannot be trusted anywhere.
//!
//! # `AArch64`
//!
//! Not lifted in this release. Its instructions reach [`Stmt::Opaque`] by the
//! same door everything unmodelled does, so a `AArch64` function decompiles to
//! its own listing inside a C frame rather than to nothing — and the emitter,
//! the structurer and the naming all work on it, because they work on the IR
//! and not on the architecture.

use crate::{
    Architecture, Instruction,
    decompiler::native::ir::{
        Binary, Callee, Condition, Expr, Place, Register, Statement, Stmt, Unary, Width,
    },
};

/// What the lifter needs to know that one instruction does not say.
pub struct Context<'a> {
    pub architecture: Architecture,
    /// What the file calls an address, when it calls it anything. Used for the
    /// target of a call and for the address a `lea` computes.
    pub name_of: &'a dyn Fn(u64) -> Option<String>,
}

impl Context<'_> {
    /// The width of a pointer, which is the width the stack moves by.
    #[must_use]
    pub const fn pointer(&self) -> Width {
        match self.architecture {
            Architecture::X86_64 | Architecture::Arm64 => Width::Qword,
            _ => Width::Dword,
        }
    }

    const fn stack_pointer(&self) -> Register {
        Register::new(
            if matches!(self.architecture, Architecture::X86_64) {
                "rsp"
            } else {
                "esp"
            },
            self.pointer(),
        )
    }
}

/// Turns one instruction into the effects it has.
///
/// Never empty: an instruction with nothing worth printing yields
/// [`Stmt::Nothing`], so the count of statements never falls out of step with
/// the listing and every address keeps a row of its own.
#[must_use]
pub fn lift(instruction: &Instruction, context: &Context<'_>) -> Vec<Statement> {
    let at = instruction.address;
    let effects = effects_of(instruction, context);
    if effects.is_empty() {
        return vec![Statement::new(at, Stmt::Nothing)];
    }
    effects
        .into_iter()
        .map(|effect| Statement::new(at, effect))
        .collect()
}

fn effects_of(instruction: &Instruction, context: &Context<'_>) -> Vec<Stmt> {
    if !matches!(
        context.architecture,
        Architecture::X86 | Architecture::X86_64
    ) {
        return vec![Stmt::Opaque(instruction.text.clone())];
    }
    let (mnemonic, operands) = split(&instruction.text);
    let parsed: Vec<Operand> = operands.iter().map(|text| operand(text)).collect();
    x86(mnemonic, &parsed, instruction, context)
}

// ---------------------------------------------------------------------------
// Reading the AT&T text
// ---------------------------------------------------------------------------

/// One operand as the syntax writes it.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Operand {
    Register(Register),
    /// `$0x10`.
    Immediate(u64),
    /// `-0x8(%rbp)`, `(%rax,%rbx,4)`, `%fs:0x28`.
    Memory(MemoryRef),
    /// A bare number: the target of a branch, or an absolute address. Which of
    /// the two it is depends on the instruction, so both readings are left to
    /// the caller.
    Number(u64),
    /// `*%rax`, `*0x24F70` — the star of an indirect call or jump.
    Indirect(Box<Operand>),
    /// `%st(1)`, and anything else the syntax has that this does not read.
    Other(String),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct MemoryRef {
    displacement: i64,
    base: Option<Register>,
    index: Option<Register>,
    scale: u64,
    /// `%fs` or `%gs`, which no ordinary address arithmetic reaches — the
    /// stack canary lives behind one.
    segment: Option<String>,
}

/// Splits an instruction into its mnemonic and its operands.
///
/// The comma that separates operands may also appear inside the parentheses of
/// an address — `testb $0x20,1(%rsi,%rax,2)` has three commas and two operands
/// — so the split counts depth rather than cutting at every comma.
fn split(text: &str) -> (&str, Vec<&str>) {
    let text = text.trim();
    let Some(space) = text.find(char::is_whitespace) else {
        return (text, Vec::new());
    };
    let (mnemonic, rest) = text.split_at(space);
    let rest = rest.trim();
    let mut operands = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    for (index, character) in rest.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                operands.push(rest[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    if start < rest.len() {
        operands.push(rest[start..].trim());
    }
    (mnemonic, operands)
}

fn operand(text: &str) -> Operand {
    let text = text.trim();
    if let Some(inner) = text.strip_prefix('*') {
        return Operand::Indirect(Box::new(operand(inner)));
    }
    if let Some(literal) = text.strip_prefix('$') {
        return number(literal).map_or_else(|| Operand::Other(text.to_owned()), Operand::Immediate);
    }
    if text.starts_with('%') && !text.contains('(') && !text.contains(':') {
        return register_named(&text[1..])
            .map_or_else(|| Operand::Other(text.to_owned()), Operand::Register);
    }
    // A segment override — `%fs:0x28` — is an address in a space of its own,
    // and reading it as ordinary arithmetic would make the stack canary look
    // like a load from address 0x28.
    let (segment, rest) = match text.split_once(':') {
        Some((prefix, rest)) if prefix.starts_with('%') => {
            (Some(prefix[1..].to_owned()), rest.trim())
        }
        _ => (None, text),
    };
    if let Some(open) = rest.find('(') {
        let (displacement, inside) = rest.split_at(open);
        let inside = inside.trim_start_matches('(').trim_end_matches(')');
        let mut parts = inside.split(',').map(str::trim);
        let base = parts.next().filter(|part| !part.is_empty());
        let index = parts.next().filter(|part| !part.is_empty());
        let scale = parts.next().and_then(number).unwrap_or(1);
        return Operand::Memory(MemoryRef {
            displacement: signed(displacement.trim()).unwrap_or(0),
            base: base.and_then(|name| register_named(name.trim_start_matches('%'))),
            index: index.and_then(|name| register_named(name.trim_start_matches('%'))),
            scale,
            segment,
        });
    }
    if let Some(value) = number(rest) {
        if segment.is_some() {
            return Operand::Memory(MemoryRef {
                displacement: i64::try_from(value).unwrap_or_default(),
                scale: 1,
                segment,
                ..MemoryRef::default()
            });
        }
        return Operand::Number(value);
    }
    Operand::Other(text.to_owned())
}

/// A number as the formatter writes it: `8`, `0x1F`, `-0xAE`.
fn number(text: &str) -> Option<u64> {
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let value = if let Some(hexadecimal) = digits.strip_prefix("0x").or(digits.strip_prefix("0X")) {
        u64::from_str_radix(hexadecimal, 16).ok()?
    } else {
        digits.parse::<u64>().ok()?
    };
    Some(if negative {
        value.wrapping_neg()
    } else {
        value
    })
}

fn signed(text: &str) -> Option<i64> {
    if text.is_empty() {
        return Some(0);
    }
    #[expect(
        clippy::cast_possible_wrap,
        reason = "a displacement is a signed value written as its two's complement"
    )]
    number(text).map(|value| value as i64)
}

/// The register a name designates, as the whole register and the width.
#[must_use]
pub fn register_named(name: &str) -> Option<Register> {
    // The sixteen general registers, at each of the four widths the encoding
    // addresses them. Written as a table rather than as string surgery: the
    // four that lose their prefix entirely at eight bits (`al`, `bl`, `cl`,
    // `dl`) and the four that gain an `l` (`sil`, `dil`, `bpl`, `spl`) have no
    // rule between them.
    const GENERAL: &[(&str, [&str; 4])] = &[
        ("rax", ["rax", "eax", "ax", "al"]),
        ("rbx", ["rbx", "ebx", "bx", "bl"]),
        ("rcx", ["rcx", "ecx", "cx", "cl"]),
        ("rdx", ["rdx", "edx", "dx", "dl"]),
        ("rsi", ["rsi", "esi", "si", "sil"]),
        ("rdi", ["rdi", "edi", "di", "dil"]),
        ("rbp", ["rbp", "ebp", "bp", "bpl"]),
        ("rsp", ["rsp", "esp", "sp", "spl"]),
    ];

    let name = name.trim().trim_start_matches('%');
    for (root, spellings) in GENERAL {
        for (position, spelling) in spellings.iter().enumerate() {
            if *spelling == name {
                let width = match position {
                    0 => Width::Qword,
                    1 => Width::Dword,
                    2 => Width::Word,
                    _ => Width::Byte,
                };
                return Some(Register::new(root, width));
            }
        }
    }
    // `%ah` and its three siblings address the second byte, which no width
    // expresses. They are recognised so that nothing mistakes them for `%al`.
    for (spelling, root) in [("ah", "rax"), ("bh", "rbx"), ("ch", "rcx"), ("dh", "rdx")] {
        if spelling == name {
            return Some(Register {
                root,
                width: Width::Byte,
                high_byte: true,
            });
        }
    }
    if let Some(rest) = name.strip_prefix('r')
        && let Some((digits, width)) = numbered(rest)
        && (8..=15).contains(&digits)
    {
        return Some(Register::new(NUMBERED[digits - 8], width));
    }
    // `%xmm0`, `%ymm0` and `%zmm0` are one register at three sizes, so they
    // share a root and differ by width — the same rule `%rax`, `%eax` and
    // `%al` follow. Claiming `Xmm` for all three, as this did, made a
    // thirty-two byte move look like a sixteen-byte one.
    if let Some(width) = match &name[..name.len().min(3)] {
        "xmm" => Some(Width::Xmm),
        "ymm" => Some(Width::Ymm),
        "zmm" => Some(Width::Zmm),
        _ => None,
    } {
        return Some(Register::new(interned_vector(name)?, width));
    }
    if name == "rip" || name == "eip" {
        return Some(Register::new("rip", Width::Qword));
    }
    None
}

/// `r8`..`r15`, whose roots have to outlive the call that reads their name.
const NUMBERED: [&str; 8] = ["r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15"];
const VECTORS: [&str; 16] = [
    "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7", "xmm8", "xmm9", "xmm10",
    "xmm11", "xmm12", "xmm13", "xmm14", "xmm15",
];

/// `8`, `12d`, `9w`, `10b` — the number of one of the newer registers and the
/// width its suffix asks for.
fn numbered(rest: &str) -> Option<(usize, Width)> {
    let (digits, width) = match rest.strip_suffix('d') {
        Some(digits) => (digits, Width::Dword),
        None => match rest.strip_suffix('w') {
            Some(digits) => (digits, Width::Word),
            None => match rest.strip_suffix('b') {
                Some(digits) => (digits, Width::Byte),
                None => (rest, Width::Qword),
            },
        },
    };
    Some((digits.parse::<usize>().ok()?, width))
}

/// The vector registers are named per file, so their roots must be `'static`
/// too. Anything past `xmm15` is not one this reads.
fn interned_vector(name: &str) -> Option<&'static str> {
    let number: usize = name.get(3..)?.parse().ok()?;
    VECTORS.get(number).copied()
}

// ---------------------------------------------------------------------------
// From an operand to a value
// ---------------------------------------------------------------------------

/// The address a memory operand computes, as arithmetic on what it names.
fn address_of(reference: &MemoryRef, pointer: Width) -> Expr {
    let mut sum: Option<Expr> = reference
        .base
        .map(|register| Expr::register(Register::new(register.root, pointer)));
    if let Some(index) = reference.index {
        let scaled = if reference.scale > 1 {
            Expr::binary(
                Binary::Multiply,
                Expr::register(Register::new(index.root, pointer)),
                Expr::constant(reference.scale, pointer),
            )
        } else {
            Expr::register(Register::new(index.root, pointer))
        };
        sum = Some(match sum {
            Some(base) => Expr::binary(Binary::Add, base, scaled),
            None => scaled,
        });
    }
    match (sum, reference.displacement) {
        (None, displacement) =>
        {
            #[expect(
                clippy::cast_sign_loss,
                reason = "an absolute address is the displacement read as unsigned"
            )]
            Expr::constant(displacement as u64, pointer)
        }
        (Some(base), 0) => base,
        (Some(base), displacement) if displacement > 0 => Expr::binary(
            Binary::Add,
            base,
            #[expect(
                clippy::cast_sign_loss,
                reason = "the sign was just tested; the value is positive"
            )]
            Expr::constant(displacement as u64, pointer),
        ),
        (Some(base), displacement) => Expr::binary(
            Binary::Subtract,
            base,
            Expr::constant(displacement.unsigned_abs(), pointer),
        ),
    }
}

/// What an operand designates as a place, when it designates one.
fn place_of(operand: &Operand, width: Width, context: &Context<'_>) -> Option<Place> {
    match operand {
        Operand::Register(register) => Some(Place::Register(*register)),
        Operand::Memory(reference) => {
            if reference.segment.is_some() {
                return None;
            }
            Some(Place::Memory {
                address: Box::new(address_of(reference, context.pointer())),
                width,
            })
        }
        Operand::Number(value) => Some(Place::Memory {
            address: Box::new(Expr::constant(*value, context.pointer())),
            width,
        }),
        _ => None,
    }
}

/// What an operand reads as a value.
fn value_of(operand: &Operand, width: Width, context: &Context<'_>) -> Expr {
    match operand {
        Operand::Immediate(value) => Expr::constant(*value, width),
        Operand::Memory(reference) if reference.segment.is_some() => {
            // The canary and the thread-local block. Named as what they are:
            // there is no arithmetic here to model, and reading it as an
            // address would be a claim about a space this file never names.
            let segment = reference.segment.clone().unwrap_or_default();
            Expr::Unknown(format!(
                "*({} *)({segment}:{:#x})",
                width.c_name(false),
                reference.displacement
            ))
        }
        Operand::Indirect(inner) => value_of(inner, width, context),
        Operand::Other(text) => Expr::Unknown(text.clone()),
        _ => place_of(operand, width, context)
            .map_or_else(|| Expr::Unknown(format!("{operand:?}")), Expr::read),
    }
}

/// The width an instruction works at, from its operands and failing that from
/// the suffix on its mnemonic.
///
/// `mov %eax,%ecx` states it twice over; `movl $0x18,-0xF0(%rbp)` states it
/// only in the `l`, which is exactly why the suffix is there.
fn width_of(mnemonic: &str, operands: &[Operand], context: &Context<'_>) -> Width {
    for operand in operands {
        if let Operand::Register(register) = operand {
            return register.width;
        }
    }
    match mnemonic.chars().last() {
        Some('b') if mnemonic.len() > 1 => Width::Byte,
        Some('w') if mnemonic.len() > 1 => Width::Word,
        Some('l') if mnemonic.len() > 1 => Width::Dword,
        Some('q') if mnemonic.len() > 1 => Width::Qword,
        _ => context.pointer(),
    }
}

/// The mnemonic with any size suffix taken off, so one arm can serve `add`,
/// `addl` and `addq`.
fn stem(mnemonic: &str) -> &str {
    // `movabs` is not a size suffix and not a family of its own: it is the
    // spelling AT&T gives `mov` when the immediate needs all sixty-four bits.
    // Read as anything else it is a load of a constant left unread, and a
    // constant is the one thing a reader most wants named.
    if mnemonic == "movabs" || mnemonic == "movabsq" {
        return "mov";
    }
    for base in [
        "mov", "add", "sub", "and", "or", "xor", "cmp", "test", "push", "pop", "inc", "dec", "neg",
        "not", "imul", "mul", "idiv", "div", "shl", "sal", "shr", "sar", "rol", "ror", "call",
        "jmp", "ret", "lea", "adc", "sbb", "nop", "xchg",
    ] {
        if mnemonic == base {
            return base;
        }
        if let Some(suffix) = mnemonic.strip_prefix(base)
            && matches!(suffix, "b" | "w" | "l" | "q")
        {
            return base;
        }
    }
    mnemonic
}

/// The whole-register vector moves, in the legacy spelling and the VEX one.
///
/// Aligned and unaligned, integer and floating, are all the same assignment:
/// the difference between `movaps` and `movups` is what the processor does
/// when the address is not aligned, which is a fault or not a fault and never
/// a different value.
const VECTOR_MOVES: &[&str] = &[
    "movaps",
    "movapd",
    "movups",
    "movupd",
    "movdqa",
    "movdqu",
    "lddqu",
    "vmovaps",
    "vmovapd",
    "vmovups",
    "vmovupd",
    "vmovdqa",
    "vmovdqu",
    "vlddqu",
    // The AVX-512 spellings name the element width they will *later* be read
    // at, which changes nothing about how many bytes moved. A masked form
    // carries an operand this does not read and falls through to `Opaque`.
    "vmovdqa32",
    "vmovdqa64",
    "vmovdqu8",
    "vmovdqu16",
    "vmovdqu32",
    "vmovdqu64",
];

// ---------------------------------------------------------------------------
// The instructions themselves
// ---------------------------------------------------------------------------

#[expect(
    clippy::too_many_lines,
    reason = "one arm per instruction family; splitting it would put the table in three places"
)]
fn x86(
    mnemonic: &str,
    operands: &[Operand],
    instruction: &Instruction,
    context: &Context<'_>,
) -> Vec<Stmt> {
    let opaque = || vec![Stmt::Opaque(instruction.text.clone())];
    let width = width_of(mnemonic, operands, context);
    let base = stem(mnemonic);

    // The two-operand shape AT&T writes source-first. Almost every arm below
    // wants exactly this pair, so it is read once.
    let pair = |width: Width| -> Option<(Expr, Place)> {
        let source = operands.first()?;
        let destination = operands.get(1)?;
        Some((
            value_of(source, width, context),
            place_of(destination, width, context)?,
        ))
    };

    match base {
        // -- moves ------------------------------------------------------
        "mov" => pair(width).map_or_else(opaque, |(value, place)| {
            // An immediate the file has a name for is that name. This is how
            // `mov $0x401230,%esi` becomes `"usage: sample WORD\n"` rather
            // than a number, which is often the most informative thing in a
            // decompiled function — and it is safe, because `name_of` answers
            // only where the file really says something about the address.
            let value = match operands.first() {
                Some(Operand::Immediate(literal)) => {
                    (context.name_of)(*literal).map_or(value, |name| Expr::Symbol {
                        name,
                        address: *literal,
                    })
                }
                _ => value,
            };
            vec![Stmt::Assign { place, value }]
        }),
        "lea" => {
            let Some((Operand::Memory(reference), destination)) =
                operands.first().zip(operands.get(1))
            else {
                // `lea 0x1234,%rax`, the formatter having already resolved a
                // `%rip`-relative operand to the address it names. That is a
                // constant, and often one the file has a name for.
                return match (operands.first(), operands.get(1)) {
                    (Some(Operand::Number(address)), Some(destination)) => {
                        let value = (context.name_of)(*address).map_or_else(
                            || Expr::constant(*address, context.pointer()),
                            |name| Expr::Symbol {
                                name,
                                address: *address,
                            },
                        );
                        place_of(destination, context.pointer(), context)
                            .map_or_else(opaque, |place| vec![Stmt::Assign { place, value }])
                    }
                    _ => opaque(),
                };
            };
            // The arithmetic itself, not `&*(…)`: `lea 1(%rax),%rdx` is an
            // increment written as an address, and every compiler uses it that
            // way.
            place_of(destination, context.pointer(), context).map_or_else(opaque, |place| {
                vec![Stmt::Assign {
                    place,
                    value: address_of(reference, context.pointer()),
                }]
            })
        }
        // -- the vector moves --------------------------------------------
        // `movaps`, `movdqu` and the dozen other spellings of the same act:
        // sixteen, thirty-two or sixty-four bytes copied without being looked
        // at. Which spelling a compiler picked says how the address is
        // expected to be aligned and whether the bytes will later be read as
        // integers or as floats — neither of which changes what moved — so
        // they all lift to one assignment at the width the register names.
        // Together they were the largest single thing this lifter did not
        // read: two thirds of the unmodelled instructions in an optimised
        // build, because that is what `memcpy`, every string comparison and
        // every vectorised loop are made of.
        //
        // The *partial* vector moves are deliberately not here. `movss`
        // writes four bytes of a sixteen-byte register and leaves the other
        // twelve standing, and this IR has no way to say "the low quarter of
        // `%xmm0`": lifting one as a whole-register assignment would claim
        // twelve bytes were overwritten that were not. They stay unread, and
        // the output says so.
        _ if VECTOR_MOVES.contains(&base) => {
            let Some(width) = operands.iter().find_map(|operand| match operand {
                Operand::Register(register) if register.root.starts_with("xmm") => {
                    Some(register.width)
                }
                _ => None,
            }) else {
                // No vector register named: an AVX-512 form carrying a mask,
                // or a spelling this does not read.
                return opaque();
            };
            if operands.len() != 2 {
                // The three-operand VEX forms merge rather than copy.
                return opaque();
            }
            pair(width).map_or_else(opaque, |(value, place)| vec![Stmt::Assign { place, value }])
        }
        // The widening moves state both widths in their name: `movzbl` reads a
        // byte and writes a long.
        _ if mnemonic.starts_with("movz") || mnemonic.starts_with("movs") => {
            let signed = mnemonic.starts_with("movs");
            let Some((from, to)) = extension_widths(mnemonic) else {
                return opaque();
            };
            let Some((source, destination)) = operands.first().zip(operands.get(1)) else {
                return opaque();
            };
            let Some(place) = place_of(destination, to, context) else {
                return opaque();
            };
            vec![Stmt::Assign {
                value: Expr::Cast {
                    value: Box::new(value_of(source, from, context)),
                    width: to,
                    signed,
                },
                place,
            }]
        }
        // The one-instruction sign extensions, which have no operands at all
        // and are how a compiler prepares a signed division.
        "cltq" | "cdqe" => vec![assign_extended("rax", Width::Dword, Width::Qword)],
        "cwtl" | "cwde" => vec![assign_extended("rax", Width::Word, Width::Dword)],
        "cbtw" | "cbw" => vec![assign_extended("rax", Width::Byte, Width::Word)],
        // These fill a second register with the sign of the first, so that the
        // pair reads as one wider value.
        "cltd" | "cdq" => vec![sign_spread("rdx", "rax", Width::Dword)],
        "cqto" | "cqo" => vec![sign_spread("rdx", "rax", Width::Qword)],

        // -- arithmetic and logic ---------------------------------------
        "add" | "sub" | "and" | "or" | "xor" | "adc" | "sbb" | "shl" | "sal" | "shr" | "sar"
        | "rol" | "ror" | "imul"
            if operands.len() == 2 =>
        {
            let Some((value, place)) = pair(width) else {
                return opaque();
            };
            arithmetic(base, place, value)
        }
        // `imul $0x38,%rax,%rdx` — the three-operand form, which unlike the
        // others does not read its destination.
        "imul" if operands.len() == 3 => {
            let Some(place) = operands.get(2).and_then(|it| place_of(it, width, context)) else {
                return opaque();
            };
            let left = value_of(&operands[1], width, context);
            let right = value_of(&operands[0], width, context);
            let product = Expr::binary(Binary::Multiply, left, right);
            let mut statements = vec![Stmt::Assign {
                place: place.clone(),
                value: product,
            }];
            statements.extend(conditions_of_result(&Expr::read(place), Flags::Arithmetic));
            statements
        }
        "inc" | "dec" if operands.len() == 1 => {
            let Some(place) = place_of(&operands[0], width, context) else {
                return opaque();
            };
            let one = Expr::constant(1, width);
            let operator = if base == "inc" {
                Binary::Add
            } else {
                Binary::Subtract
            };
            let mut statements = vec![Stmt::Assign {
                place: place.clone(),
                value: Expr::binary(operator, Expr::read(place.clone()), one),
            }];
            // `inc` and `dec` deliberately leave the carry alone — that is
            // what they are for — so only the questions that do not rest on it
            // are settled.
            statements.extend(conditions_of_result(&Expr::read(place), Flags::Arithmetic));
            statements
        }
        "neg" | "not" if operands.len() == 1 => {
            let Some(place) = place_of(&operands[0], width, context) else {
                return opaque();
            };
            let operator = if base == "neg" {
                Unary::Negate
            } else {
                Unary::Not
            };
            let mut statements = vec![Stmt::Assign {
                place: place.clone(),
                value: Expr::unary(operator, Expr::read(place.clone())),
            }];
            if base == "neg" {
                statements.extend(conditions_of_result(&Expr::read(place), Flags::Arithmetic));
            }
            statements
        }
        // One-operand multiply and divide write the implicit pair. The high
        // half of a product, and the pair a division consumes, are not
        // expressible in C without a wider type; what is stated is the half
        // that ordinary code actually uses, and the other is named as unknown
        // rather than left out.
        "mul" | "imul" if operands.len() == 1 => {
            let accumulator = Place::Register(Register::new("rax", width));
            let value = value_of(&operands[0], width, context);
            vec![
                Stmt::Assign {
                    place: accumulator.clone(),
                    value: Expr::binary(Binary::Multiply, Expr::read(accumulator), value),
                },
                Stmt::Assign {
                    place: Place::Register(Register::new("rdx", width)),
                    value: Expr::Unknown(format!("high half of {}", instruction.text)),
                },
            ]
        }
        "div" | "idiv" if operands.len() == 1 => {
            let accumulator = Place::Register(Register::new("rax", width));
            let remainder = Place::Register(Register::new("rdx", width));
            let divisor = value_of(&operands[0], width, context);
            vec![
                Stmt::Assign {
                    place: remainder,
                    value: Expr::binary(
                        Binary::Modulo,
                        Expr::read(accumulator.clone()),
                        divisor.clone(),
                    ),
                },
                Stmt::Assign {
                    place: accumulator.clone(),
                    value: Expr::binary(Binary::Divide, Expr::read(accumulator), divisor),
                },
            ]
        }

        // -- what settles the conditions --------------------------------
        "cmp" if operands.len() == 2 => {
            // AT&T writes the subtrahend first: `cmp $0x10,%rax` asks about
            // `rax` against `0x10`, and reading it the other way round turns
            // every comparison in the output backwards.
            let right = value_of(&operands[0], width, context);
            let left = value_of(&operands[1], width, context);
            conditions_of_comparison(&left, &right)
        }
        "test" if operands.len() == 2 => {
            let right = value_of(&operands[0], width, context);
            let left = value_of(&operands[1], width, context);
            // `test %rax,%rax` is how every compiler asks "is it zero?", and
            // `rax & rax` is not what a reader wants to see.
            let result = if left == right {
                left
            } else {
                Expr::binary(Binary::And, left, right)
            };
            conditions_of_result(&result, Flags::Logical)
        }

        // -- the stack ---------------------------------------------------
        "push" if operands.len() == 1 => {
            let pointer = context.pointer();
            let stack = Place::Register(context.stack_pointer());
            vec![
                Stmt::Assign {
                    place: stack.clone(),
                    value: Expr::binary(
                        Binary::Subtract,
                        Expr::read(stack.clone()),
                        Expr::constant(pointer.bytes(), pointer),
                    ),
                },
                Stmt::Assign {
                    place: Place::Memory {
                        address: Box::new(Expr::read(stack)),
                        width: pointer,
                    },
                    value: value_of(&operands[0], pointer, context),
                },
            ]
        }
        "pop" if operands.len() == 1 => {
            let pointer = context.pointer();
            let stack = Place::Register(context.stack_pointer());
            let Some(place) = place_of(&operands[0], pointer, context) else {
                return opaque();
            };
            vec![
                Stmt::Assign {
                    place,
                    value: Expr::read(Place::Memory {
                        address: Box::new(Expr::read(stack.clone())),
                        width: pointer,
                    }),
                },
                Stmt::Assign {
                    place: stack.clone(),
                    value: Expr::binary(
                        Binary::Add,
                        Expr::read(stack),
                        Expr::constant(pointer.bytes(), pointer),
                    ),
                },
            ]
        }
        // `leave` is `mov %rbp,%rsp` and `pop %rbp` in one byte.
        "leave" => {
            let pointer = context.pointer();
            let stack = Place::Register(context.stack_pointer());
            let frame = Place::Register(Register::new("rbp", pointer));
            vec![
                Stmt::Assign {
                    place: stack.clone(),
                    value: Expr::read(frame.clone()),
                },
                Stmt::Assign {
                    place: frame,
                    value: Expr::read(Place::Memory {
                        address: Box::new(Expr::read(stack.clone())),
                        width: pointer,
                    }),
                },
                Stmt::Assign {
                    place: stack.clone(),
                    value: Expr::binary(
                        Binary::Add,
                        Expr::read(stack),
                        Expr::constant(pointer.bytes(), pointer),
                    ),
                },
            ]
        }

        // -- leaving, and going elsewhere --------------------------------
        // What is returned is decided by naming, which knows the ABI; the
        // instruction itself says only that the function ends.
        "ret" => vec![Stmt::Return(None)],
        // `rep ret` and `repz retq`: the prefix means nothing on a return —
        // it is padding for a branch predictor that mispredicted a bare one —
        // and the instruction is the return it looks like. Every *other* `rep`
        // form is a loop, and a loop is not lifted by dropping its prefix, so
        // those stay unread.
        "rep" | "repz" | "repe" | "repnz" | "repne"
            if operands.len() == 1
                && matches!(operands.first(), Some(Operand::Other(text)) if stem(text) == "ret") =>
        {
            vec![Stmt::Return(None)]
        }
        // `ud2` raises, and nothing below it runs. A compiler puts one
        // wherever it has proved control cannot arrive: the far side of a
        // diverging call, an unreachable match arm, the panic path of a bounds
        // check. In an optimised Rust binary there are thousands.
        "ud2" | "ud1" => vec![Stmt::Trap],
        "call" => vec![call_of(operands.first(), instruction, context)],
        "jmp" => match operands.first() {
            Some(Operand::Number(target)) => vec![Stmt::Branch {
                condition: None,
                target: *target,
            }],
            Some(indirect) => vec![Stmt::IndirectBranch(value_of(
                indirect,
                context.pointer(),
                context,
            ))],
            None => opaque(),
        },
        "syscall" | "sysenter" => vec![Stmt::SystemCall { number: None }],
        "int" => match operands.first() {
            Some(Operand::Immediate(0x80)) => vec![Stmt::SystemCall { number: None }],
            _ => opaque(),
        },

        // -- what the conditions are read by -----------------------------
        _ if mnemonic.starts_with('j') => {
            let Some(condition) = condition_of(&mnemonic[1..]) else {
                return opaque();
            };
            match operands.first() {
                Some(Operand::Number(target)) => vec![Stmt::Branch {
                    condition: Some(condition),
                    target: *target,
                }],
                _ => opaque(),
            }
        }
        _ if mnemonic.starts_with("set") => {
            let Some(condition) = condition_of(&mnemonic[3..]) else {
                return opaque();
            };
            operands
                .first()
                .and_then(|it| place_of(it, Width::Byte, context))
                .map_or_else(opaque, |place| {
                    vec![Stmt::Assign {
                        place,
                        value: condition,
                    }]
                })
        }
        _ if mnemonic.starts_with("cmov") => {
            let Some(condition) = condition_of(&mnemonic[4..]) else {
                return opaque();
            };
            let Some((value, place)) = pair(width) else {
                return opaque();
            };
            vec![Stmt::Assign {
                value: Expr::Select {
                    condition: Box::new(condition),
                    when_true: Box::new(value),
                    when_false: Box::new(Expr::read(place.clone())),
                },
                place,
            }]
        }

        // -- the bit instructions with an exact C spelling ---------------
        // Only the ones that are exact. `lzcnt`, `tzcnt`, `bsf` and `bsr` are
        // not here on purpose: each of them answers something different for a
        // zero operand than the C builtin it resembles does, and a decompiler
        // that papers over that disagreement is wrong precisely where the
        // reader would never think to check.
        "popcnt" | "bswap" if !operands.is_empty() => {
            let Some((source, destination)) = (match base {
                // `bswap` reads and writes the one register it names.
                "bswap" => operands.first().zip(operands.first()),
                _ => operands.first().zip(operands.get(1)),
            }) else {
                return opaque();
            };
            let Some(place) = place_of(destination, width, context) else {
                return opaque();
            };
            let name = match (base, place.width()) {
                ("bswap", Width::Qword) => "__builtin_bswap64",
                ("bswap", _) => "__builtin_bswap32",
                (_, Width::Qword) => "__builtin_popcountll",
                _ => "__builtin_popcount",
            };
            vec![Stmt::Assign {
                place,
                value: Expr::Call {
                    callee: Callee::Named(name.to_owned()),
                    arguments: vec![value_of(source, width, context)],
                },
            }]
        }
        // `bt $5,%eax` answers one question and writes nothing: the carry
        // holds bit five. Said as the shift it is, the `jb` below it reads an
        // ordinary condition instead of an unmodelled one — which is the
        // whole point of conditions being places.
        "bt" if operands.len() == 2 => {
            let (Some(index), Some(Operand::Register(register))) =
                (operands.first(), operands.get(1))
            else {
                // The memory form addresses a bit *string*, whose index runs
                // past the operand it names. That is not this.
                return opaque();
            };
            let bit = match index {
                Operand::Immediate(value) => {
                    Expr::constant(value % u64::from(register.width.bits()), width)
                }
                other => value_of(other, width, context),
            };
            vec![Stmt::Assign {
                place: Place::Condition(Condition::Carry),
                value: Expr::binary(
                    Binary::NotEqual,
                    Expr::binary(
                        Binary::And,
                        Expr::binary(
                            Binary::ShiftRight,
                            Expr::read(Place::Register(*register)),
                            bit,
                        ),
                        Expr::constant(1, width),
                    ),
                    Expr::constant(0, width),
                ),
            }]
        }

        // -- what a compiler pads with -----------------------------------
        "nop" | "endbr64" | "endbr32" | "hint" => vec![Stmt::Nothing],
        // `xchg %ax,%ax` and `xchg %rax,%rax` are the two-byte and three-byte
        // spellings of a no-operation; a genuine exchange is not one.
        "xchg" if operands.len() == 2 && operands[0] == operands[1] => vec![Stmt::Nothing],

        _ => opaque(),
    }
}

/// A widening move's two widths, from the letters at the end of its name.
///
/// `movzbl` is byte to long, `movslq` long to quadword. The letters are the
/// AT&T size codes and the pair is always the last two of them.
fn extension_widths(mnemonic: &str) -> Option<(Width, Width)> {
    let letters = mnemonic.get(4..)?;
    let mut characters = letters.chars();
    let from = size_code(characters.next()?)?;
    let to = size_code(characters.next()?)?;
    Some((from, to))
}

const fn size_code(letter: char) -> Option<Width> {
    Some(match letter {
        'b' => Width::Byte,
        'w' => Width::Word,
        'l' => Width::Dword,
        'q' => Width::Qword,
        _ => return None,
    })
}

fn assign_extended(root: &'static str, from: Width, to: Width) -> Stmt {
    Stmt::Assign {
        place: Place::Register(Register::new(root, to)),
        value: Expr::Cast {
            value: Box::new(Expr::register(Register::new(root, from))),
            width: to,
            signed: true,
        },
    }
}

/// `cltd` and `cqto`: fill one register with the sign bit of another, so the
/// pair reads as a value twice as wide.
fn sign_spread(into: &'static str, from: &'static str, width: Width) -> Stmt {
    Stmt::Assign {
        place: Place::Register(Register::new(into, width)),
        value: Expr::binary(
            Binary::ShiftRightSigned,
            Expr::register(Register::new(from, width)),
            Expr::constant(u64::from(width.bits() - 1), Width::Byte),
        ),
    }
}

/// How much of the flags an instruction really settles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Flags {
    /// `test`, `and`, `or`, `xor`: the carry and the overflow are *cleared*,
    /// so every question that rests on them has an exact answer.
    Logical,
    /// `add`, `inc`, `shl`: the zero and the sign are exact; the carry and the
    /// overflow are not modelled, and the questions resting on them are left
    /// unsettled rather than answered wrongly.
    Arithmetic,
}

/// What `cmp` leaves behind: one answer per question a branch can ask, each
/// stated in terms of the two operands rather than of the flags.
///
/// This is the single most important function in the pipeline. Everything the
/// output has that the old line-by-line translation lacked — `if (i < n)`,
/// `while (*p != 0)`, a `for` with a bound — comes from here, by way of the
/// ordinary substitution the dataflow pass applies to every other value.
fn conditions_of_comparison(left: &Expr, right: &Expr) -> Vec<Stmt> {
    let settle = |condition: Condition, operator: Binary| Stmt::Assign {
        place: Place::Condition(condition),
        value: Expr::binary(operator, left.clone(), right.clone()),
    };
    vec![
        settle(Condition::Zero, Binary::Equal),
        settle(Condition::Less, Binary::Less),
        settle(Condition::LessOrEqual, Binary::LessOrEqual),
        settle(Condition::Carry, Binary::Below),
        settle(Condition::BelowOrEqual, Binary::BelowOrEqual),
        // `js` asks about the sign of the difference, which is not the same
        // question as `jl` the moment the subtraction overflows.
        Stmt::Assign {
            place: Place::Condition(Condition::Sign),
            value: Expr::binary(
                Binary::Less,
                Expr::binary(Binary::Subtract, left.clone(), right.clone()),
                Expr::constant(0, Width::Qword),
            ),
        },
        unsettled(Condition::Overflow),
        unsettled(Condition::Parity),
    ]
}

/// What an instruction that produced a result leaves behind, which is the same
/// set of questions asked of that result against zero.
fn conditions_of_result(result: &Expr, flags: Flags) -> Vec<Stmt> {
    let zero = Expr::constant(0, result.width().unwrap_or(Width::Qword));
    let settle = |condition: Condition, operator: Binary| Stmt::Assign {
        place: Place::Condition(condition),
        value: Expr::binary(operator, result.clone(), zero.clone()),
    };
    let mut statements = vec![
        settle(Condition::Zero, Binary::Equal),
        settle(Condition::Sign, Binary::Less),
    ];
    match flags {
        Flags::Logical => {
            // The carry and the overflow are cleared outright, so `jl` reduces
            // to the sign and `jbe` to the zero. Both are then exact.
            statements.push(settle(Condition::Less, Binary::Less));
            statements.push(settle(Condition::LessOrEqual, Binary::LessOrEqual));
            statements.push(Stmt::Assign {
                place: Place::Condition(Condition::Carry),
                value: Expr::constant(0, Width::Byte),
            });
            statements.push(Stmt::Assign {
                place: Place::Condition(Condition::Overflow),
                value: Expr::constant(0, Width::Byte),
            });
            statements.push(settle(Condition::BelowOrEqual, Binary::Equal));
        }
        Flags::Arithmetic => {
            // Exact while nothing overflows, which is the reading a person
            // makes of the same two instructions.
            statements.push(settle(Condition::Less, Binary::Less));
            statements.push(settle(Condition::LessOrEqual, Binary::LessOrEqual));
            statements.push(unsettled(Condition::Carry));
            statements.push(unsettled(Condition::Overflow));
            statements.push(unsettled(Condition::BelowOrEqual));
        }
    }
    statements.push(unsettled(Condition::Parity));
    statements
}

/// A question this instruction leaves without an answer this reads.
///
/// Written down rather than left out: a later `jp` must not pick up the answer
/// some earlier instruction left, and an assignment of an unknown is what
/// stops it.
fn unsettled(condition: Condition) -> Stmt {
    Stmt::Assign {
        place: Place::Condition(condition),
        value: Expr::Unknown(condition.name().to_owned()),
    }
}

/// The two-operand arithmetic instructions, which read their destination.
fn arithmetic(base: &str, place: Place, value: Expr) -> Vec<Stmt> {
    let (operator, flags) = match base {
        "add" => (Binary::Add, Flags::Arithmetic),
        "sub" => (Binary::Subtract, Flags::Arithmetic),
        "and" => (Binary::And, Flags::Logical),
        "or" => (Binary::Or, Flags::Logical),
        "xor" => (Binary::Xor, Flags::Logical),
        "imul" => (Binary::Multiply, Flags::Arithmetic),
        "shl" | "sal" => (Binary::ShiftLeft, Flags::Arithmetic),
        "shr" => (Binary::ShiftRight, Flags::Arithmetic),
        "sar" => (Binary::ShiftRightSigned, Flags::Arithmetic),
        // The carry-propagating pair, and the rotates: the operation is real
        // but is not one C has, so it is named rather than mistranslated.
        _ => {
            return vec![Stmt::Assign {
                value: Expr::Unknown(format!("{base} of {} and …", describe(&place))),
                place,
            }];
        }
    };
    // `xor %eax,%eax` clears the register whatever was in it, and every
    // compiler writes zero that way. Left as `eax = eax ^ eax` it survives
    // every later pass, because the value does depend on the register.
    let cleared = operator == Binary::Xor && value == Expr::read(place.clone());
    let result = if cleared {
        Expr::constant(0, place.width())
    } else {
        Expr::binary(operator, Expr::read(place.clone()), value)
    };
    let mut statements = vec![Stmt::Assign {
        place: place.clone(),
        value: result,
    }];
    statements.extend(conditions_of_result(&Expr::read(place), flags));
    statements
}

fn describe(place: &Place) -> String {
    match place {
        Place::Register(register) => register.name(),
        Place::Condition(condition) => condition.name().to_owned(),
        Place::Local { id, .. } => format!("local_{id}"),
        Place::Memory { .. } => "memory".to_owned(),
    }
}

/// A call, and where it goes.
fn call_of(target: Option<&Operand>, instruction: &Instruction, context: &Context<'_>) -> Stmt {
    let callee = match target {
        Some(Operand::Number(address)) => {
            (context.name_of)(*address).map_or(Callee::Address(*address), Callee::Named)
        }
        Some(Operand::Indirect(inner)) => match inner.as_ref() {
            // `callq *0x24F70` — through a slot the loader fills in, which is
            // how every imported function is reached. The name of the slot is
            // the name of the function, and the caller supplies it.
            Operand::Number(address) => (context.name_of)(*address).map_or_else(
                || {
                    Callee::Indirect(Box::new(Expr::read(Place::Memory {
                        address: Box::new(Expr::constant(*address, context.pointer())),
                        width: context.pointer(),
                    })))
                },
                Callee::Named,
            ),
            other => Callee::Indirect(Box::new(value_of(other, context.pointer(), context))),
        },
        Some(other) => Callee::Indirect(Box::new(value_of(other, context.pointer(), context))),
        None => return Stmt::Opaque(instruction.text.clone()),
    };
    Stmt::Call {
        // Every call is given the return register as its result. Whether the
        // function returns anything is settled later, by whether anything
        // reads it before it is written again — which is the only evidence a
        // file without types offers.
        result: Some(Place::Register(Register::new("rax", context.pointer()))),
        callee,
        arguments: Vec::new(),
    }
}

/// The condition a `j`, `set` or `cmov` suffix asks about.
///
/// Every spelling the manual gives, including the pairs that mean the same
/// thing — `jnae` is `jb`, and a compiler picks between them by whim.
fn condition_of(suffix: &str) -> Option<Expr> {
    let read = |condition: Condition| Expr::read(Place::Condition(condition));
    let not = |condition: Condition| Expr::unary(Unary::LogicalNot, read(condition));
    Some(match suffix {
        "e" | "z" => read(Condition::Zero),
        "ne" | "nz" => not(Condition::Zero),
        "l" | "nge" => read(Condition::Less),
        "ge" | "nl" => not(Condition::Less),
        "le" | "ng" => read(Condition::LessOrEqual),
        "g" | "nle" => not(Condition::LessOrEqual),
        "b" | "c" | "nae" => read(Condition::Carry),
        "ae" | "nc" | "nb" => not(Condition::Carry),
        "be" | "na" => read(Condition::BelowOrEqual),
        "a" | "nbe" => not(Condition::BelowOrEqual),
        "s" => read(Condition::Sign),
        "ns" => not(Condition::Sign),
        "o" => read(Condition::Overflow),
        "no" => not(Condition::Overflow),
        "p" | "pe" => read(Condition::Parity),
        "np" | "po" => not(Condition::Parity),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn context() -> Context<'static> {
        Context {
            architecture: Architecture::X86_64,
            name_of: &|_| None,
        }
    }

    fn instruction(text: &str) -> Instruction {
        Instruction {
            address: 0x1000,
            bytes: crate::InstructionBytes::new(&[0x90]).expect("one byte is an instruction"),
            text: text.to_owned(),
            section: Arc::from(".text"),
        }
    }

    fn effects(text: &str) -> Vec<Stmt> {
        lift(&instruction(text), &context())
            .into_iter()
            .map(|statement| statement.effect)
            .collect()
    }

    /// The largest single thing this lifter did not read. `movaps` and its
    /// dozen spellings are what `memcpy`, every string comparison and every
    /// vectorised loop are made of, and leaving them unread put a hole in the
    /// middle of exactly the functions a reader opens the view for.
    #[test]
    fn a_vector_move_is_one_assignment_of_the_width_the_register_names() {
        let Stmt::Assign { place, value } = &effects("movdqu (%rsi),%xmm1")[0] else {
            panic!("a vector move assigns");
        };
        assert_eq!(*place, Place::Register(Register::new("xmm1", Width::Xmm)));
        let Expr::Read(read) = value else {
            panic!("it reads what the address names");
        };
        assert_eq!(read.width(), Width::Xmm, "sixteen bytes moved");
    }

    /// `%xmm0`, `%ymm0` and `%zmm0` are one register at three sizes. Read as
    /// one width they would all be sixteen bytes, and half of what a
    /// thirty-two byte move moved would go unmentioned.
    #[test]
    fn a_wide_vector_move_is_as_wide_as_the_register_it_names() {
        let Stmt::Assign { place, .. } = &effects("vmovdqa %ymm3,%ymm7")[0] else {
            panic!("a vector move assigns");
        };
        assert_eq!(*place, Place::Register(Register::new("xmm7", Width::Ymm)));
        assert_eq!(place.width().bytes(), 32);
        let Stmt::Assign { place, .. } = &effects("vmovdqu64 %zmm1,%zmm2")[0] else {
            panic!("the AVX-512 spelling is a move too");
        };
        assert_eq!(place.width().bytes(), 64);
    }

    /// The other half of being exact about the vector moves: `movss` writes
    /// four bytes of a sixteen-byte register and leaves the other twelve
    /// standing, and nothing here can say that. So it says nothing.
    #[test]
    fn a_partial_vector_move_is_left_unread_rather_than_claimed_whole() {
        assert!(
            matches!(effects("movss %xmm0,%xmm1")[0], Stmt::Opaque(_)),
            "a four-byte write to a sixteen-byte register is not a copy of it"
        );
        assert!(matches!(effects("vmovsd (%rax),%xmm2")[0], Stmt::Opaque(_)));
    }

    /// `movabs` is not a family of its own: it is what AT&T calls `mov` when
    /// the immediate needs all sixty-four bits. Read as anything else, the one
    /// thing a reader most wants named — a constant — went unread.
    #[test]
    fn a_wide_immediate_is_the_move_it_is() {
        let Stmt::Assign { place, value } = &effects("movabs $0x123456789,%rax")[0] else {
            panic!("a wide immediate is still a move");
        };
        assert_eq!(*place, Place::Register(Register::new("rax", Width::Qword)));
        assert_eq!(*value, Expr::constant(0x1_2345_6789, Width::Qword));
    }

    /// A compiler puts `ud2` wherever it has proved control cannot arrive.
    /// There are thousands in an optimised Rust binary, and every one of them
    /// used to be a hole in the output.
    #[test]
    fn an_instruction_that_only_raises_says_that_and_not_nothing() {
        assert_eq!(effects("ud2")[0], Stmt::Trap);
        assert_ne!(
            effects("ud2")[0],
            Stmt::Nothing,
            "something happens: this is not padding"
        );
    }

    /// The prefix means nothing on a return — it is padding for a branch
    /// predictor — and the instruction is the return it looks like. Every
    /// other `rep` form is a loop, and a loop is not lifted by dropping its
    /// prefix.
    #[test]
    fn a_repeated_return_is_a_return_and_a_repeated_move_is_not_a_move() {
        assert_eq!(effects("repz retq")[0], Stmt::Return(None));
        assert!(
            matches!(effects("rep stos %al,%es:(%rdi)")[0], Stmt::Opaque(_)),
            "a string operation is a loop, and dropping the prefix would hide it"
        );
    }

    /// `bt` answers one question and writes nothing. Said as the shift it is,
    /// the `jb` below it reads an ordinary condition instead of an unmodelled
    /// one — which is the whole point of conditions being places.
    #[test]
    fn a_bit_test_settles_the_carry_the_branch_below_it_reads() {
        let Stmt::Assign { place, .. } = &effects("bt $0x5,%eax")[0] else {
            panic!("a bit test writes the carry");
        };
        assert_eq!(*place, Place::Condition(Condition::Carry));
    }

    /// Only the ones that are exact. `lzcnt` and its family answer something
    /// different for a zero operand than the C builtin they resemble, and are
    /// left unread on purpose.
    #[test]
    fn the_bit_counts_with_an_exact_c_spelling_get_it_and_the_others_do_not() {
        let Stmt::Assign { value, .. } = &effects("popcnt %rax,%rdx")[0] else {
            panic!("a population count assigns");
        };
        let Expr::Call { callee, .. } = value else {
            panic!("it is written as the builtin it is");
        };
        assert_eq!(*callee, Callee::Named("__builtin_popcountll".to_owned()));
        assert!(
            matches!(effects("lzcnt %eax,%edx")[0], Stmt::Opaque(_)),
            "the zero case disagrees with __builtin_clz, so it is not claimed"
        );
    }

    /// The comma inside an address is not the comma between operands.
    #[test]
    fn operands_are_split_outside_the_parentheses() {
        let (mnemonic, operands) = split("testb $0x20,1(%rsi,%rax,2)");
        assert_eq!(mnemonic, "testb");
        assert_eq!(operands, vec!["$0x20", "1(%rsi,%rax,2)"]);
    }

    #[test]
    fn an_address_is_read_as_its_parts() {
        let Operand::Memory(reference) = operand("-0x8(%rbp)") else {
            panic!("an address is a memory operand");
        };
        assert_eq!(reference.displacement, -8);
        assert_eq!(reference.base, Some(Register::new("rbp", Width::Qword)));
        assert!(reference.index.is_none());

        let Operand::Memory(scaled) = operand("1(%rsi,%rax,2)") else {
            panic!("an address is a memory operand");
        };
        assert_eq!(scaled.displacement, 1);
        assert_eq!(scaled.base, Some(Register::new("rsi", Width::Qword)));
        assert_eq!(scaled.index, Some(Register::new("rax", Width::Qword)));
        assert_eq!(scaled.scale, 2);
    }

    /// Read the other way round, every comparison in the output would be
    /// backwards — the single most damaging mistake this parser could make.
    #[test]
    fn att_writes_the_subtrahend_first() {
        let Stmt::Assign { value, .. } = &effects("cmp $0x10,%rax")[0] else {
            panic!("a comparison settles the conditions");
        };
        let Expr::Binary {
            operator,
            left,
            right,
        } = value
        else {
            panic!("the zero condition is a comparison");
        };
        assert_eq!(*operator, Binary::Equal);
        assert_eq!(**left, Expr::register(Register::new("rax", Width::Qword)));
        assert_eq!(**right, Expr::constant(0x10, Width::Qword));
    }

    #[test]
    fn a_comparison_settles_every_question_a_branch_can_ask() {
        let settled: Vec<Condition> = effects("cmp %rbx,%rax")
            .iter()
            .filter_map(|effect| match effect {
                Stmt::Assign {
                    place: Place::Condition(condition),
                    ..
                } => Some(*condition),
                _ => None,
            })
            .collect();
        for condition in Condition::ALL {
            assert!(
                settled.contains(condition),
                "{condition:?} was left carrying whatever an earlier instruction put there"
            );
        }
    }

    /// How every compiler writes zero. Left as an exclusive-or of a register
    /// with itself it survives every later pass, because the value really does
    /// depend on the register.
    #[test]
    fn xor_of_a_register_with_itself_is_a_constant() {
        let Stmt::Assign { value, place } = &effects("xor %eax,%eax")[0] else {
            panic!("an exclusive-or assigns");
        };
        assert_eq!(*place, Place::Register(Register::new("rax", Width::Dword)));
        assert_eq!(*value, Expr::constant(0, Width::Dword));
    }

    /// `test %rax,%rax` is a compiler asking "is it zero?", and `rax & rax` is
    /// not what a reader wants to see.
    #[test]
    fn a_register_tested_against_itself_is_not_written_as_an_and() {
        let Stmt::Assign { value, .. } = &effects("test %rax,%rax")[0] else {
            panic!("a test settles the conditions");
        };
        let Expr::Binary { left, .. } = value else {
            panic!("the zero condition is a comparison");
        };
        assert_eq!(**left, Expr::register(Register::new("rax", Width::Qword)));
    }

    #[test]
    fn a_push_moves_the_pointer_and_writes_through_it() {
        let effects = effects("push %rbp");
        assert_eq!(effects.len(), 2);
        assert!(matches!(
            &effects[0],
            Stmt::Assign {
                place: Place::Register(register),
                ..
            } if register.root == "rsp"
        ));
        assert!(matches!(
            &effects[1],
            Stmt::Assign {
                place: Place::Memory { .. },
                ..
            }
        ));
    }

    #[test]
    fn a_branch_reads_the_question_its_suffix_names() {
        let Stmt::Branch {
            condition: Some(condition),
            target,
        } = &effects("jle 0x0000000000001234")[0]
        else {
            panic!("a conditional jump branches on a condition");
        };
        assert_eq!(*target, 0x1234);
        assert_eq!(
            *condition,
            Expr::read(Place::Condition(Condition::LessOrEqual))
        );
    }

    #[test]
    fn an_unconditional_jump_carries_no_condition() {
        assert_eq!(
            effects("jmp 0x0000000000001234")[0],
            Stmt::Branch {
                condition: None,
                target: 0x1234
            }
        );
    }

    /// The widening moves state both widths in their own name.
    #[test]
    fn a_widening_move_reads_narrow_and_writes_wide() {
        let Stmt::Assign { value, place } = &effects("movzwl -0xAE(%rbp),%eax")[0] else {
            panic!("a widening move assigns");
        };
        assert_eq!(*place, Place::Register(Register::new("rax", Width::Dword)));
        let Expr::Cast {
            width,
            signed,
            value,
        } = value
        else {
            panic!("a widening move is a conversion");
        };
        assert_eq!(*width, Width::Dword);
        assert!(!signed, "movz is the unsigned spelling");
        assert_eq!(value.width(), Some(Width::Word));
    }

    /// `lea 1(%rax),%rdx` is an increment written as an address, and every
    /// compiler uses it that way.
    #[test]
    fn lea_is_the_arithmetic_and_not_an_address_of() {
        let Stmt::Assign { value, .. } = &effects("lea 1(%rax),%rdx")[0] else {
            panic!("lea assigns");
        };
        assert_eq!(
            *value,
            Expr::binary(
                Binary::Add,
                Expr::register(Register::new("rax", Width::Qword)),
                Expr::constant(1, Width::Qword)
            )
        );
    }

    /// The whole point of the module: what is not understood says so, rather
    /// than being dropped or guessed at.
    #[test]
    fn an_instruction_that_is_not_modelled_keeps_its_own_text() {
        assert_eq!(
            effects("fldt 0x1FDE0")[0],
            Stmt::Opaque("fldt 0x1FDE0".to_owned())
        );
    }

    #[test]
    fn padding_has_no_effect_worth_printing() {
        assert_eq!(effects("nop")[0], Stmt::Nothing);
        assert_eq!(effects("endbr64")[0], Stmt::Nothing);
        assert_eq!(effects("xchg %ax,%ax")[0], Stmt::Nothing);
    }

    #[test]
    fn the_canary_is_named_and_not_read_as_an_address() {
        let Stmt::Assign { value, .. } = &effects("mov %fs:0x28,%rax")[0] else {
            panic!("the canary is loaded");
        };
        let Expr::Unknown(text) = value else {
            panic!("a segment-relative load is named, not modelled");
        };
        assert!(text.contains("fs:"), "{text}");
    }

    #[test]
    fn a_conditional_move_is_a_choice_and_not_an_assignment() {
        let Stmt::Assign { value, .. } = &effects("cmovbe %edi,%eax")[0] else {
            panic!("a conditional move assigns");
        };
        assert!(matches!(value, Expr::Select { .. }));
    }
}
