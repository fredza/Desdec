//! Reading a structure out of the code that walks it.
//!
//! A reader who knows a function is handed a pointer still has to work out
//! what it points at, and the way they do it by hand is always the same: read
//! down the listing, write `+0x18, eight bytes, read` in a notebook, and after
//! forty lines the shape of the structure is on the page. It is mechanical,
//! and it is exactly what a tool should do.
//!
//! So this does it. Given the body of a function and the register holding the
//! pointer, it collects every access through that register — `0x18(%rbx)`,
//! `[x0, #0x18]` — and lays them out as a structure whose members sit where
//! the code says they sit. The result is a starting point, not an answer: the
//! names are offsets and the types are widths, and the reader who knows what
//! the program is renames them.
//!
//! What it will not do is fill in what the code did not say:
//!
//! - **A gap is a gap.** An offset nothing touched becomes named padding, not
//!   a member. Leaving it out instead would move every member after it.
//! - **An access whose width the text does not state makes no member.** It is
//!   reported separately, so a reader can see there is something at that
//!   offset the listing did not describe.
//! - **An index register means an array, and the code states its element size
//!   and never its length.** Those accesses are reported apart from the
//!   members, because a made-up length lays out everything after it wrongly.

use std::collections::BTreeMap;

use crate::{
    Architecture, Instruction,
    types::{Definition, Member, Primitive, Registry, Type},
};

/// One access through the base register, as the listing states it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Access {
    /// Bytes from the pointer. Negative when the code reaches below it.
    pub offset: i64,
    /// How wide the access is, when the text says: from the register it moves
    /// through, or from the mnemonic's own size suffix.
    pub width: Option<u64>,
    /// Whether an index register was part of the address, which is what an
    /// array subscript looks like.
    pub indexed: bool,
    /// Whether the instruction writes there.
    ///
    /// Read from where the operand sits and what the mnemonic does with it,
    /// which is exact for the moves and the arithmetic and deliberately says
    /// "read" for anything it does not recognise: a member reported as written
    /// when it is only compared would have the reader looking for a write that
    /// never happens.
    pub writes: bool,
    /// The instruction it was read from.
    pub at: u64,
}

/// A structure read out of a function, and what could not be read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Inferred {
    /// The members, at the offsets the code puts them, with padding named
    /// where nothing was touched.
    pub definition: Definition,
    /// Every access that was read, in address order.
    pub accesses: Vec<Access>,
    /// Accesses whose width the text does not state, so they made no member.
    pub unstated: Vec<Access>,
    /// Accesses through an index register: an array, whose element size the
    /// code states and whose length it does not.
    pub indexed: Vec<Access>,
    /// Accesses below the pointer, which are not part of what it points at —
    /// a frame pointer's locals, most often, and a sign the wrong register was
    /// asked about.
    pub below: Vec<Access>,
    /// Offsets that fall inside a wider member: either a union, or two
    /// different things reached through one register.
    pub overlapping: Vec<u64>,
}

impl Inferred {
    /// Whether the code said anything at all about what the register points
    /// at.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.definition.members().is_empty()
    }
}

/// Reads what `base` points at, from what `body` does with it.
///
/// `base` is a register name as the listing writes it, without a `%`.
#[must_use]
pub fn from_body(
    name: &str,
    base: &str,
    body: &[Instruction],
    architecture: Architecture,
) -> Inferred {
    let mut accesses = Vec::new();
    for instruction in body {
        accesses.extend(accesses_in(instruction, base, architecture));
    }

    let mut unstated = Vec::new();
    let mut indexed = Vec::new();
    let mut below = Vec::new();
    // The widest access seen at each offset. Two reads of different widths at
    // one offset is a byte being read out of a word as often as it is two
    // members, and the wider one is the one that keeps the offsets after it
    // right.
    let mut widths: BTreeMap<u64, u64> = BTreeMap::new();

    for access in &accesses {
        if access.indexed {
            indexed.push(*access);
            continue;
        }
        if access.offset < 0 {
            below.push(*access);
            continue;
        }
        let Some(width) = access.width else {
            unstated.push(*access);
            continue;
        };
        let offset = access.offset.unsigned_abs();
        let slot = widths.entry(offset).or_insert(width);
        *slot = (*slot).max(width);
    }

    let (members, overlapping) = members_from(&widths);
    Inferred {
        definition: Definition::Struct {
            name: name.to_owned(),
            members,
        },
        accesses,
        unstated,
        indexed,
        below,
        overlapping,
    }
}

/// The members of a structure whose offsets and widths are known, with the
/// untouched bytes between them named as padding.
fn members_from(widths: &BTreeMap<u64, u64>) -> (Vec<Member>, Vec<u64>) {
    let mut members = Vec::new();
    let mut overlapping = Vec::new();
    let mut end = 0u64;

    for (offset, width) in widths {
        if *offset < end {
            // Inside the member before it: a union, or two different things
            // reached through one register. Said rather than laid out.
            overlapping.push(*offset);
            continue;
        }
        if *offset > end {
            members.push(Member::new(
                format!("gap_{end:x}"),
                Type::Array(
                    Box::new(Type::primitive(Primitive::UnsignedChar)),
                    offset - end,
                ),
            ));
        }
        members.push(Member::new(format!("field_{offset:x}"), of_width(*width)));
        end = offset.saturating_add(*width);
    }
    (members, overlapping)
}

/// The unsigned type of a given width.
///
/// Unsigned, and never a pointer or a signed number, because the width is all
/// the code stated: `mov 0x8(%rbx),%rax` moves eight bytes and says nothing
/// about what they mean. Guessing `void *` because a member is eight bytes
/// wide would put a guess in front of the reader dressed as a reading.
fn of_width(width: u64) -> Type {
    Type::primitive(match width {
        1 => Primitive::UnsignedChar,
        2 => Primitive::UnsignedShort,
        4 => Primitive::UnsignedInt,
        8 => Primitive::UnsignedLongLong,
        // Anything else is a wide move — an SSE register's worth — and is kept
        // as the bytes it is.
        _ => return Type::Array(Box::new(Type::primitive(Primitive::UnsignedChar)), width),
    })
}

/// One instruction, and the member of a type it touches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Named {
    /// The instruction, so a listing can find its own row.
    pub at: u64,
    /// The member, named the way [`Registry::member_at`] names it:
    /// `header.count`, `entries[3].tag`.
    pub path: String,
    /// Bytes past the start of the member the access begins at. `0` for an
    /// access that lands on it exactly, which is nearly all of them.
    pub into: u64,
    /// Whether the instruction writes there.
    pub writes: bool,
}

/// Names every access `body` makes through `base`, read against `kind`.
///
/// This is what turns a column of `0x18(%rbx)` into a column of
/// `header.count` — the thing a reader is doing in their head anyway, and the
/// reason to have written the structure down at all.
///
/// An access the type does not cover names nothing: below the pointer, past
/// the end of the structure, or in the padding between two members. Naming the
/// nearest member instead would put a name in the listing that the code is not
/// touching.
#[must_use]
pub fn name_accesses(
    body: &[Instruction],
    base: &str,
    architecture: Architecture,
    registry: &Registry,
    kind: &Type,
) -> Vec<Named> {
    let mut named = Vec::new();
    for instruction in body {
        for access in accesses_in(instruction, base, architecture) {
            // An index register means the address is not this offset at all,
            // and a negative one is not inside what the pointer points at.
            if access.indexed || access.offset < 0 {
                continue;
            }
            let Some(found) = registry.member_at(kind, access.offset.unsigned_abs()) else {
                continue;
            };
            named.push(Named {
                at: instruction.address,
                path: found.path,
                into: found.into,
                writes: access.writes,
            });
        }
    }
    named
}

/// Which register a slot of the frame is reached through.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Frame {
    /// The frame pointer: `rbp`, `ebp`, `x29`. Where a compiler that keeps one
    /// puts the locals, at negative offsets, and the arguments it was passed
    /// on the stack, at positive ones.
    BasePointer,
    /// The stack pointer: `rsp`, `esp`, `sp`. Where the locals are in a frame
    /// built without a frame pointer, which is most of them once the optimiser
    /// has been over the code.
    StackPointer,
}

impl Frame {
    /// How it is written in an address, so a reader can find it in the
    /// listing.
    #[must_use]
    pub const fn label(self, architecture: Architecture) -> &'static str {
        match (self, architecture) {
            (Self::BasePointer, Architecture::X86) => "ebp",
            (Self::BasePointer, Architecture::Arm64) => "x29",
            (Self::BasePointer, _) => "rbp",
            (Self::StackPointer, Architecture::X86) => "esp",
            (Self::StackPointer, Architecture::Arm64) => "sp",
            (Self::StackPointer, _) => "rsp",
        }
    }
}

/// One slot of a function's frame, as its own code uses it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Local {
    pub frame: Frame,
    /// Bytes from the register. Negative below it, which is where the locals
    /// of a frame-pointer function are.
    pub offset: i64,
    /// How wide the accesses to it are, when the text says.
    pub width: Option<u64>,
    pub reads: usize,
    pub writes: usize,
    /// The first instruction that touches it, so the listing can be taken
    /// there.
    pub first: u64,
}

impl Local {
    /// Where it sits, as the listing writes it: `rbp-0x18`.
    #[must_use]
    pub fn label(&self, architecture: Architecture) -> String {
        let register = self.frame.label(architecture);
        if self.offset < 0 {
            return format!("{register}-{:#x}", self.offset.unsigned_abs());
        }
        format!("{register}+{:#x}", self.offset)
    }
}

/// The slots of one function's frame, from what its own code does with them.
///
/// Both registers are followed, because whether a function keeps a frame
/// pointer is the compiler's choice and not the reader's: a frame built
/// without one puts its locals at positive offsets from `rsp`, and looking
/// only at `rbp` would find nothing at all in most optimised code.
///
/// This is a reading of the text, not a measurement. An access through an
/// index register is left out — it is a stack array, whose length the code
/// does not state — and so is anything below the stack pointer, which is not
/// part of the frame.
#[must_use]
pub fn locals(body: &[Instruction], architecture: Architecture) -> Vec<Local> {
    let mut found: BTreeMap<(Frame, i64), Local> = BTreeMap::new();

    for (frame, register) in [
        (Frame::BasePointer, Frame::BasePointer.label(architecture)),
        (Frame::StackPointer, Frame::StackPointer.label(architecture)),
    ] {
        for instruction in body {
            for access in accesses_in(instruction, register, architecture) {
                if access.indexed {
                    continue;
                }
                // Below the stack pointer is the red zone at best and nothing
                // at all at worst; it is not a slot of this frame.
                if frame == Frame::StackPointer && access.offset < 0 {
                    continue;
                }
                let slot = found.entry((frame, access.offset)).or_insert(Local {
                    frame,
                    offset: access.offset,
                    width: access.width,
                    reads: 0,
                    writes: 0,
                    first: instruction.address,
                });
                // The widest access is the one that says how much room the
                // slot takes, the same way it does for a member.
                slot.width = match (slot.width, access.width) {
                    (Some(had), Some(width)) => Some(had.max(width)),
                    (had, width) => had.or(width),
                };
                slot.first = slot.first.min(instruction.address);
                if access.writes {
                    slot.writes += 1;
                } else {
                    slot.reads += 1;
                }
            }
        }
    }
    found.into_values().collect()
}

/// The accesses through `base` one instruction makes.
fn accesses_in(instruction: &Instruction, base: &str, architecture: Architecture) -> Vec<Access> {
    match architecture {
        Architecture::X86 | Architecture::X86_64 => at_and_t(instruction, base),
        Architecture::Arm64 => aarch64(instruction, base),
        Architecture::Arm | Architecture::Unknown => Vec::new(),
    }
}

/// `0x18(%rbx)`, `(%rax)`, `-0x8(%rbp)`, `0x10(%rbx,%rcx,4)`.
fn at_and_t(instruction: &Instruction, base: &str) -> Vec<Access> {
    let text = &instruction.text;
    let mut found = Vec::new();
    let mut rest = text.as_str();

    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open..].find(')') else {
            break;
        };
        let inside = &rest[open + 1..open + close];
        let before = &rest[..open];
        let after = &rest[open + close + 1..];

        let mut parts = inside.split(',');
        let named = parts.next().unwrap_or("").trim().trim_start_matches('%');
        let indexed = parts.next().is_some_and(|index| !index.trim().is_empty());
        if named == base {
            found.push(Access {
                offset: displacement(before),
                width: width_of(text, before, after),
                indexed,
                writes: writes_there(text, after),
                at: instruction.address,
            });
        }
        rest = after;
    }
    found
}

/// The number written immediately before the bracket, which is the
/// displacement.
fn displacement(before: &str) -> i64 {
    let tail: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_hexdigit() || matches!(c, 'x' | 'X' | '-' | '+'))
        .collect();
    let written: String = tail.chars().rev().collect();
    read_number(&written).unwrap_or(0)
}

/// A number as a listing writes it: `0x18`, `-0x8`, `24`.
fn read_number(written: &str) -> Option<i64> {
    let written = written.trim();
    let (negative, digits) = match written.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, written.trim_start_matches('+')),
    };
    let value = match digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        Some(hex) => i64::from_str_radix(hex, 16).ok()?,
        None => digits.parse::<i64>().ok()?,
    };
    Some(if negative { -value } else { value })
}

/// How wide an x86 access is, from what the instruction states.
///
/// The register on the other side of the move says it — `%rax` is eight bytes,
/// `%eax` four — and when there is no register, GAS's own size suffix does:
/// `movl $0x0,0x8(%rax)` states four in the mnemonic because nothing else in
/// the line would.
fn width_of(text: &str, before: &str, after: &str) -> Option<u64> {
    let operands = format!("{before} {after}");
    for word in operands.split(|c: char| !(c.is_ascii_alphanumeric() || c == '%')) {
        if let Some(name) = word.strip_prefix('%') {
            if let Some(width) = register_width(name) {
                return Some(width);
            }
        }
    }
    let mnemonic = text.split_whitespace().next()?;
    suffix_width(mnemonic)
}

/// How many bytes an x86 register holds.
fn register_width(name: &str) -> Option<u64> {
    const BYTE: &[&str] = &[
        "al", "bl", "cl", "dl", "ah", "bh", "ch", "dh", "sil", "dil", "bpl", "spl",
    ];
    const HALF: &[&str] = &["ax", "bx", "cx", "dx", "si", "di", "bp", "sp"];

    if name.starts_with("xmm") {
        return Some(16);
    }
    if name.starts_with("ymm") {
        return Some(32);
    }
    if BYTE.contains(&name) {
        return Some(1);
    }
    if HALF.contains(&name) {
        return Some(2);
    }
    if let Some(numbered) = name.strip_prefix('r') {
        // `r8`..`r15` and their narrower halves, `r8d`, `r8w`, `r8b`.
        if let Some(width) = numbered
            .strip_suffix('d')
            .map(|_| 4)
            .or_else(|| numbered.strip_suffix('w').map(|_| 2))
            .or_else(|| numbered.strip_suffix('b').map(|_| 1))
        {
            if numbered[..numbered.len() - 1].parse::<u8>().is_ok() {
                return Some(width);
            }
        }
        if numbered.parse::<u8>().is_ok() || numbered.len() == 2 {
            // `rax`, `rsp`, `r8`.
            return Some(8);
        }
    }
    if let Some(rest) = name.strip_prefix('e') {
        if rest.len() == 2 {
            return Some(4);
        }
    }
    None
}

/// Whether an x86 instruction writes to the memory operand.
///
/// In AT&T order the destination is last, so a memory operand with nothing
/// after it is the destination — unless the mnemonic is one that reads its
/// destination without writing to it.
fn writes_there(text: &str, after: &str) -> bool {
    // `cmp` and `test` set flags and leave both operands alone; `push` reads
    // its operand and writes to the stack.
    const READS_ONLY: &[&str] = &["cmp", "test", "push", "bt"];

    if after.contains(',') {
        return false;
    }
    let Some(mnemonic) = text.split_whitespace().next() else {
        return false;
    };
    let stem = mnemonic.trim_end_matches(['b', 'w', 'l', 'q']);
    !READS_ONLY.contains(&stem) && !READS_ONLY.contains(&mnemonic)
}

/// GAS's size suffix, when the mnemonic carries one.
fn suffix_width(mnemonic: &str) -> Option<u64> {
    // Only the mnemonics that take one, and only when nothing else in the line
    // states the width: `call`, `jbe` and `nop` all end in letters that would
    // otherwise be read as sizes.
    const CARRIES: &[&str] = &[
        "mov", "add", "sub", "cmp", "test", "and", "or", "xor", "inc", "dec", "not", "neg", "push",
        "pop", "imul", "mul", "shl", "shr", "sar", "adc", "sbb",
    ];
    let (stem, suffix) = mnemonic.split_at(mnemonic.len().saturating_sub(1));
    if !CARRIES.contains(&stem) {
        return None;
    }
    Some(match suffix {
        "b" => 1,
        "w" => 2,
        "l" => 4,
        "q" => 8,
        _ => return None,
    })
}

/// `[x0, #0x18]`, `[x0]`, `[x0, x1, lsl #3]`.
fn aarch64(instruction: &Instruction, base: &str) -> Vec<Access> {
    let text = &instruction.text;
    let Some(open) = text.find('[') else {
        return Vec::new();
    };
    let Some(close) = text[open..].find(']') else {
        return Vec::new();
    };
    let inside = &text[open + 1..open + close];
    let mut parts = inside.split(',');
    let named = parts.next().unwrap_or("").trim();
    if named != base {
        return Vec::new();
    }

    let mut offset = 0;
    let mut indexed = false;
    for part in parts {
        let part = part.trim();
        match part.strip_prefix('#') {
            Some(written) => offset = read_number(written).unwrap_or(0),
            // A register, a shift, or a write-back marker: all of them mean
            // the address is not a fixed displacement from the base.
            None if !part.is_empty() => indexed = true,
            None => {}
        }
    }
    let mnemonic = text.split_whitespace().next().unwrap_or_default();
    vec![Access {
        offset,
        width: aarch64_width(text),
        indexed,
        writes: mnemonic.starts_with("st"),
        at: instruction.address,
    }]
}

/// How wide an `AArch64` load or store is, from its mnemonic and its register.
fn aarch64_width(text: &str) -> Option<u64> {
    let mut words = text.split_whitespace();
    let mnemonic = words.next()?;
    // The size is in the mnemonic for the narrow forms, and in the register
    // bank for the rest: `ldrb w0` is one byte, `ldr x0` is eight.
    if let Some(width) = mnemonic
        .strip_suffix("b")
        .map(|_| 1)
        .or_else(|| mnemonic.strip_suffix("h").map(|_| 2))
    {
        if mnemonic.starts_with("ldr") || mnemonic.starts_with("str") {
            return Some(width);
        }
    }
    if !(mnemonic.starts_with("ldr")
        || mnemonic.starts_with("str")
        || mnemonic.starts_with("ldp")
        || mnemonic.starts_with("stp"))
    {
        return None;
    }
    let register = words.next()?.trim_start_matches(',').trim();
    let bank = register.chars().next()?;
    // A pair moves two registers' worth.
    let pair = u64::from(mnemonic.starts_with("ldp") || mnemonic.starts_with("stp")) + 1;
    Some(
        pair * match bank {
            'x' | 'd' => 8,
            'w' | 's' => 4,
            'q' => 16,
            'h' => 2,
            'b' => 1,
            _ => return None,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InstructionBytes;

    fn body(lines: &[(u64, &str)]) -> Vec<Instruction> {
        lines
            .iter()
            .map(|(address, text)| Instruction {
                address: *address,
                bytes: InstructionBytes::new(&[0x90]).expect("one byte"),
                text: (*text).to_owned(),
                section: std::sync::Arc::from(".text"),
            })
            .collect()
    }

    /// The members of what was read, as `name: type` pairs.
    fn members(inferred: &Inferred) -> Vec<(String, String)> {
        inferred
            .definition
            .members()
            .iter()
            .map(|member| (member.name.clone(), member.kind.label()))
            .collect()
    }

    fn from(lines: &[(u64, &str)], base: &str) -> Inferred {
        from_body("guessed", base, &body(lines), Architecture::X86_64)
    }

    /// The whole point: a function that walks a structure describes it.
    #[test]
    fn the_offsets_the_code_reads_become_the_members_at_those_offsets() {
        let inferred = from(
            &[
                (0x10, "mov (%rdi),%rax"),
                (0x14, "mov 0x8(%rdi),%ecx"),
                (0x18, "movzbl 0xc(%rdi),%edx"),
            ],
            "rdi",
        );
        assert_eq!(
            members(&inferred),
            vec![
                ("field_0".to_owned(), "unsigned long long".to_owned()),
                ("field_8".to_owned(), "unsigned int".to_owned()),
                ("field_c".to_owned(), "unsigned int".to_owned()),
            ]
        );
    }

    /// An offset nothing touched is named padding. Leaving it out would move
    /// every member after it.
    #[test]
    fn an_offset_nothing_touches_becomes_padding_rather_than_disappearing() {
        let inferred = from(
            &[(0x10, "mov (%rbx),%rax"), (0x14, "mov 0x18(%rbx),%rax")],
            "rbx",
        );
        assert_eq!(
            members(&inferred),
            vec![
                ("field_0".to_owned(), "unsigned long long".to_owned()),
                ("gap_8".to_owned(), "unsigned char[16]".to_owned()),
                ("field_18".to_owned(), "unsigned long long".to_owned()),
            ]
        );
    }

    /// Below the pointer is not part of what it points at: it is the frame,
    /// and it is a sign the wrong register was asked about.
    #[test]
    fn what_the_code_reaches_below_the_pointer_makes_no_member() {
        let inferred = from(
            &[(0x10, "mov -0x8(%rbp),%rax"), (0x14, "mov 0x10(%rbp),%rax")],
            "rbp",
        );
        assert_eq!(inferred.below.len(), 1);
        assert_eq!(inferred.below[0].offset, -8);
        assert_eq!(
            members(&inferred).len(),
            2,
            "the gap, and the member at 0x10"
        );
    }

    /// An index register is an array subscript, and the code states the
    /// element size and never the length.
    #[test]
    fn an_indexed_access_is_reported_rather_than_laid_out_at_a_made_up_length() {
        let inferred = from(&[(0x10, "mov 0x10(%rbx,%rcx,4),%eax")], "rbx");
        assert_eq!(inferred.indexed.len(), 1);
        assert_eq!(inferred.indexed[0].offset, 0x10);
        assert!(inferred.is_empty(), "and it makes no member of its own");
    }

    /// GAS puts the width in the mnemonic when nothing else in the line
    /// carries it.
    #[test]
    fn the_size_suffix_states_the_width_when_no_register_does() {
        let inferred = from(
            &[
                (0x10, "movl $0x0,0x8(%rax)"),
                (0x14, "movb $0x1,0x10(%rax)"),
            ],
            "rax",
        );
        assert_eq!(
            members(&inferred)
                .into_iter()
                .filter(|(name, _)| name.starts_with("field"))
                .collect::<Vec<_>>(),
            vec![
                ("field_8".to_owned(), "unsigned int".to_owned()),
                ("field_10".to_owned(), "unsigned char".to_owned()),
            ]
        );
        assert!(inferred.unstated.is_empty());
    }

    #[test]
    fn an_access_whose_width_is_never_stated_makes_no_member() {
        let inferred = from(&[(0x10, "inc 0x8(%rax)")], "rax");
        assert_eq!(inferred.unstated.len(), 1);
        assert!(inferred.is_empty());
    }

    /// Two members that sit on top of one another is either a union or the
    /// wrong base register, and both are worth saying rather than laying out.
    #[test]
    fn an_offset_inside_a_wider_member_is_reported_as_overlapping() {
        let inferred = from(
            &[(0x10, "mov (%rbx),%rax"), (0x14, "mov 0x4(%rbx),%ecx")],
            "rbx",
        );
        assert_eq!(inferred.overlapping, vec![4]);
        assert_eq!(members(&inferred).len(), 1);
    }

    /// The widest read at one offset is the one that keeps the offsets after
    /// it right.
    #[test]
    fn two_widths_at_one_offset_keep_the_wider() {
        let inferred = from(
            &[(0x10, "mov (%rbx),%al"), (0x14, "mov (%rbx),%rax")],
            "rbx",
        );
        assert_eq!(
            members(&inferred),
            vec![("field_0".to_owned(), "unsigned long long".to_owned())]
        );
    }

    #[test]
    fn a_register_that_is_never_used_as_a_base_says_nothing() {
        let inferred = from(&[(0x10, "mov 0x8(%rdi),%rax")], "rbx");
        assert!(inferred.is_empty());
        assert!(inferred.accesses.is_empty());
    }

    #[test]
    fn the_same_is_read_out_of_aarch64() {
        let inferred = from_body(
            "guessed",
            "x0",
            &body(&[
                (0x10, "ldr x1, [x0]"),
                (0x14, "ldr w2, [x0, #0x8]"),
                (0x18, "ldrb w3, [x0, #0xc]"),
                (0x1c, "ldr x4, [x0, x5, lsl #3]"),
            ]),
            Architecture::Arm64,
        );
        assert_eq!(
            members(&inferred),
            vec![
                ("field_0".to_owned(), "unsigned long long".to_owned()),
                ("field_8".to_owned(), "unsigned int".to_owned()),
                ("field_c".to_owned(), "unsigned char".to_owned()),
            ]
        );
        assert_eq!(inferred.indexed.len(), 1, "the subscripted one is apart");
    }

    /// Whether an access writes decides whether the reader is looking at
    /// where a value comes from or where it goes.
    #[test]
    fn where_the_operand_sits_says_whether_the_instruction_writes_there() {
        let read = from(&[(0x10, "mov 0x8(%rbx),%rax")], "rbx");
        assert!(!read.accesses[0].writes);

        let written = from(&[(0x10, "mov %rax,0x8(%rbx)")], "rbx");
        assert!(written.accesses[0].writes);

        // `cmp` has the operand last and writes to neither side.
        let compared = from(&[(0x10, "cmpl $0x0,0x8(%rbx)")], "rbx");
        assert!(!compared.accesses[0].writes, "a comparison writes nothing");

        let stored = from_body(
            "guessed",
            "x0",
            &body(&[(0x10, "str x1, [x0, #0x8]"), (0x14, "ldr x2, [x0]")]),
            Architecture::Arm64,
        );
        assert!(stored.accesses[0].writes);
        assert!(!stored.accesses[1].writes);
    }

    /// The frame of a function, from what its own code does with it.
    #[test]
    fn the_slots_of_a_frame_are_read_from_both_of_the_registers_that_reach_it() {
        let found = locals(
            &body(&[
                (0x10, "mov %edi,-0x14(%rbp)"),
                (0x13, "mov -0x14(%rbp),%eax"),
                (0x16, "mov %rax,0x8(%rsp)"),
                (0x1a, "mov -0x8(%rsp),%rax"),
            ]),
            Architecture::X86_64,
        );
        assert_eq!(
            found.len(),
            2,
            "one slot of each frame, and nothing below rsp"
        );

        let local = &found[0];
        assert_eq!(local.frame, Frame::BasePointer);
        assert_eq!(local.offset, -0x14);
        assert_eq!(local.width, Some(4));
        assert_eq!((local.reads, local.writes), (1, 1));
        assert_eq!(local.first, 0x10);
        assert_eq!(local.label(Architecture::X86_64), "rbp-0x14");

        let argument = &found[1];
        assert_eq!(argument.frame, Frame::StackPointer);
        assert_eq!(argument.label(Architecture::X86_64), "rsp+0x8");
        assert_eq!((argument.reads, argument.writes), (0, 1));
    }

    /// A frame written without a frame pointer — most optimised code — has its
    /// locals at positive offsets from the stack pointer, and looking only at
    /// `rbp` would find none of them.
    #[test]
    fn a_frame_with_no_frame_pointer_still_has_its_slots_read() {
        let found = locals(
            &body(&[
                (0x10, "sub $0x28,%rsp"),
                (0x14, "movl $0x0,0x14(%rsp)"),
                (0x1c, "mov 0x14(%rsp),%eax"),
                (0x20, "mov %rax,0x18(%rsp)"),
            ]),
            Architecture::X86_64,
        );
        assert_eq!(
            found
                .iter()
                .map(|local| local.label(Architecture::X86_64))
                .collect::<Vec<_>>(),
            vec!["rsp+0x14".to_owned(), "rsp+0x18".to_owned()],
            "the immediate of the `sub` is not an address and makes no slot"
        );
    }

    #[test]
    fn a_stack_array_is_left_out_rather_than_given_a_made_up_length() {
        let found = locals(
            &body(&[(0x10, "mov -0x20(%rbp,%rax,4),%ecx")]),
            Architecture::X86_64,
        );
        assert!(found.is_empty());
    }

    /// The column of offsets a reader translates in their head, translated.
    #[test]
    fn the_accesses_of_a_body_are_named_against_the_type_they_go_through() {
        let mut registry = crate::types::Registry::new(crate::types::Model::default());
        for definition in crate::types::parse::definitions(
            "struct Header {
                 unsigned long long head;
                 unsigned int count;
                 unsigned int flags;
             };",
        )
        .expect("the definitions read")
        {
            registry.define(definition);
        }
        let kind = Type::Named("Header".to_owned());

        let named = name_accesses(
            &body(&[
                (0x10, "mov (%rbx),%rax"),
                (0x13, "mov 0x8(%rbx),%eax"),
                (0x16, "mov %ecx,0xc(%rbx)"),
                // Past the end of the structure, and through an index: both
                // name nothing rather than the member nearest them.
                (0x19, "mov 0x40(%rbx),%eax"),
                (0x1c, "mov 0x8(%rbx,%rcx,4),%eax"),
            ]),
            "rbx",
            Architecture::X86_64,
            &registry,
            &kind,
        );

        assert_eq!(
            named
                .iter()
                .map(|one| (one.at, one.path.as_str(), one.writes))
                .collect::<Vec<_>>(),
            vec![
                (0x10, "head", false),
                (0x13, "count", false),
                (0x16, "flags", true),
            ]
        );
    }

    /// A structure read out of code is C the reader can edit, which is the
    /// only useful form for it to take.
    #[test]
    fn what_is_read_is_a_definition_the_parser_reads_back() {
        let inferred = from(
            &[(0x10, "mov (%rdi),%rax"), (0x14, "mov 0x10(%rdi),%ecx")],
            "rdi",
        );
        let mut registry = crate::types::Registry::new(crate::types::Model::default());
        registry.define(inferred.definition.clone());
        let source = registry.to_source();

        let read = crate::types::parse::definitions(&source).expect("it reads back");
        assert_eq!(read, vec![inferred.definition]);
        assert_eq!(
            registry
                .layout(&Type::Named("guessed".to_owned()))
                .expect("laid out")
                .size,
            0x18,
            "and lays out over what the code read, rounded up to its own alignment"
        );
    }
}
