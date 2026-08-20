//! Conditions a breakpoint is only worth stopping at.
//!
//! A breakpoint inside a loop that turns ten thousand times is a breakpoint
//! the reader presses "run" at ten thousand times. What they wanted was the
//! turn where `rcx` is zero, or where the pointer being written is the one
//! they are watching — so a breakpoint can carry a condition, and stops only
//! when it holds.
//!
//! The language is small on purpose, and every part of it is something the
//! machine actually holds: a register by name, a byte or a word read through
//! `[…]`, a number, and the comparisons and arithmetic between them. There is
//! no way to call anything, nothing to assign to, and no state carried between
//! evaluations — a condition is a question asked of the state, and asking it
//! must never change the answer.
//!
//! What it deliberately does not do is fail quietly, and it does not answer
//! zero for something it could not read. An expression that does not parse is
//! refused when it is typed, by name and with the position. An expression that
//! reads unmapped memory has **no value** — not zero, which would make
//! `[rax]:1 == 0` true for a pointer that leads nowhere — and a condition with
//! no value does not stop the run. `&&` still short-circuits, so
//! `rax != 0 && [rax]:1 == 1` is safe to write and means what it looks like.

use crate::emulate::{memory::Memory, registers::Registers};

/// Why an expression could not be read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseError {
    /// A character that begins nothing the language has.
    Unexpected { at: usize, found: char },
    /// A name that is not a register.
    UnknownName { at: usize, name: String },
    /// A number that is not one.
    BadNumber { at: usize, text: String },
    /// Something was opened and not closed.
    Unclosed { at: usize, expected: char },
    /// The expression ended in the middle of itself.
    Ended,
    /// There is more after the expression than the expression.
    Trailing { at: usize },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unexpected { at, found } => write!(out, "{at}: {found}"),
            Self::UnknownName { at, name } => write!(out, "{at}: {name}"),
            Self::BadNumber { at, text } => write!(out, "{at}: {text}"),
            Self::Unclosed { at, expected } => write!(out, "{at}: {expected}"),
            Self::Ended => write!(out, "end"),
            Self::Trailing { at } => write!(out, "{at}"),
        }
    }
}

/// One node of a condition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Expression {
    /// A number as it was written.
    Number(u64),
    /// A register, by the name it was written under.
    Register(iced_x86::Register),
    /// The instruction pointer, which is not a general register.
    Pointer,
    /// `[address]`, read as `width` bytes.
    Load {
        address: Box<Self>,
        width: usize,
    },
    Unary {
        operator: Unary,
        operand: Box<Self>,
    },
    Binary {
        operator: Binary,
        left: Box<Self>,
        right: Box<Self>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unary {
    Not,
    Negate,
    LogicalNot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Binary {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    And,
    Or,
    ExclusiveOr,
    ShiftLeft,
    ShiftRight,
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    LogicalAnd,
    LogicalOr,
}

impl Binary {
    /// How tightly it binds. Higher binds first, as in C and in every
    /// calculator a reader has used.
    const fn precedence(self) -> u8 {
        match self {
            Self::LogicalOr => 1,
            Self::LogicalAnd => 2,
            Self::Or => 3,
            Self::ExclusiveOr => 4,
            Self::And => 5,
            Self::Equal | Self::NotEqual => 6,
            Self::Less | Self::LessOrEqual | Self::Greater | Self::GreaterOrEqual => 7,
            Self::ShiftLeft | Self::ShiftRight => 8,
            Self::Add | Self::Subtract => 9,
            Self::Multiply | Self::Divide | Self::Remainder => 10,
        }
    }
}

impl Expression {
    /// Reads an expression, or says where it stopped making sense.
    ///
    /// # Errors
    ///
    /// One of [`ParseError`], each carrying the position in the text so the
    /// interface can point at it.
    pub fn parse(source: &str) -> Result<Self, ParseError> {
        let mut parser = Parser {
            text: source.as_bytes(),
            at: 0,
        };
        parser.spaces();
        let expression = parser.expression(0)?;
        parser.spaces();
        if parser.at < parser.text.len() {
            return Err(ParseError::Trailing { at: parser.at });
        }
        Ok(expression)
    }

    /// What the expression is worth, given a machine's state.
    ///
    /// `None` when some part of it could not be read — which today means an
    /// address that nothing maps. Not zero: see the module's own
    /// documentation.
    #[must_use]
    pub fn value(&self, registers: &Registers, memory: &Memory) -> Option<u64> {
        Some(match self {
            Self::Number(value) => *value,
            Self::Register(register) => registers.get(*register),
            Self::Pointer => registers.instruction_pointer,
            Self::Load { address, width } => {
                let at = address.value(registers, memory)?;
                let mut bytes = [0_u8; 8];
                for (step, slot) in bytes.iter_mut().take(*width).enumerate() {
                    *slot = memory.peek(at.wrapping_add(step as u64))?;
                }
                u64::from_le_bytes(bytes)
            }
            Self::Unary { operator, operand } => {
                let value = operand.value(registers, memory)?;
                match operator {
                    Unary::Not => !value,
                    Unary::Negate => 0_u64.wrapping_sub(value),
                    Unary::LogicalNot => u64::from(value == 0),
                }
            }
            Self::Binary {
                operator,
                left,
                right,
            } => {
                let left = left.value(registers, memory)?;
                // The short-circuiting pair, which have to short-circuit for
                // `rax != 0 && [rax]:1 == 1` to be safe to write: the right
                // side is not read at all when the left has settled it.
                match operator {
                    Binary::LogicalAnd if left == 0 => return Some(0),
                    Binary::LogicalOr if left != 0 => return Some(1),
                    _ => {}
                }
                let right = right.value(registers, memory)?;
                apply(*operator, left, right)
            }
        })
    }

    /// Whether the expression holds, which is the question a breakpoint asks.
    ///
    /// An expression with no value does not hold. A run does not stop on a
    /// question that could not be answered.
    #[must_use]
    pub fn holds(&self, registers: &Registers, memory: &Memory) -> bool {
        self.value(registers, memory)
            .is_some_and(|value| value != 0)
    }
}

/// One binary operator, on two values.
///
/// Signed comparisons read the same bits as a signed number, which is what a
/// reader writing `rax < 0` means, and the shift counts are masked rather than
/// truncated. Both conversions are the operator's own rule.
#[expect(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    reason = "reading the same bits as signed, and masking a shift count, are what these operators do"
)]
fn apply(operator: Binary, left: u64, right: u64) -> u64 {
    let signed = |value: u64| value as i64;
    match operator {
        Binary::Add => left.wrapping_add(right),
        Binary::Subtract => left.wrapping_sub(right),
        Binary::Multiply => left.wrapping_mul(right),
        // A division by zero is zero rather than a panic: a condition is a
        // question, and no question asked of a running program should be able
        // to bring the tool down.
        Binary::Divide => left.checked_div(right).unwrap_or(0),
        Binary::Remainder => left.checked_rem(right).unwrap_or(0),
        Binary::And => left & right,
        Binary::Or => left | right,
        Binary::ExclusiveOr => left ^ right,
        // A count past the width shifts everything out, which is zero — the
        // same answer the hardware gives for a count it does not mask.
        Binary::ShiftLeft => left.checked_shl(right as u32).unwrap_or(0),
        Binary::ShiftRight => left.checked_shr(right as u32).unwrap_or(0),
        Binary::Equal => u64::from(left == right),
        Binary::NotEqual => u64::from(left != right),
        // Signed, because a reader writing `rax < 0` means the sign bit and
        // not "smaller than the largest number there is".
        Binary::Less => u64::from(signed(left) < signed(right)),
        Binary::LessOrEqual => u64::from(signed(left) <= signed(right)),
        Binary::Greater => u64::from(signed(left) > signed(right)),
        Binary::GreaterOrEqual => u64::from(signed(left) >= signed(right)),
        Binary::LogicalAnd => u64::from(left != 0 && right != 0),
        Binary::LogicalOr => u64::from(left != 0 || right != 0),
    }
}

/// A recursive-descent reader over the text of one condition.
struct Parser<'a> {
    text: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn spaces(&mut self) {
        while matches!(self.text.get(self.at), Some(byte) if byte.is_ascii_whitespace()) {
            self.at += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    /// Whether the text at the cursor is `word`, and steps over it if it is.
    fn eat(&mut self, word: &str) -> bool {
        if self.text[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            return true;
        }
        false
    }

    /// An expression binding at least as tightly as `least`.
    fn expression(&mut self, least: u8) -> Result<Expression, ParseError> {
        let mut left = self.unary()?;
        loop {
            self.spaces();
            let Some((operator, length)) = self.operator() else {
                break;
            };
            if operator.precedence() < least {
                break;
            }
            self.at += length;
            self.spaces();
            let right = self.expression(operator.precedence() + 1)?;
            left = Expression::Binary {
                operator,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// The operator at the cursor, if there is one, and how long it is.
    ///
    /// Two characters before one, so `<=` is not read as `<` followed by
    /// something that begins with `=`.
    fn operator(&self) -> Option<(Binary, usize)> {
        let rest = &self.text[self.at.min(self.text.len())..];
        let two: &[(&str, Binary)] = &[
            ("==", Binary::Equal),
            ("!=", Binary::NotEqual),
            ("<=", Binary::LessOrEqual),
            (">=", Binary::GreaterOrEqual),
            ("<<", Binary::ShiftLeft),
            (">>", Binary::ShiftRight),
            ("&&", Binary::LogicalAnd),
            ("||", Binary::LogicalOr),
        ];
        for (word, operator) in two {
            if rest.starts_with(word.as_bytes()) {
                return Some((*operator, 2));
            }
        }
        let one: &[(u8, Binary)] = &[
            (b'+', Binary::Add),
            (b'-', Binary::Subtract),
            (b'*', Binary::Multiply),
            (b'/', Binary::Divide),
            (b'%', Binary::Remainder),
            (b'&', Binary::And),
            (b'|', Binary::Or),
            (b'^', Binary::ExclusiveOr),
            (b'<', Binary::Less),
            (b'>', Binary::Greater),
            (b'=', Binary::Equal),
        ];
        let first = rest.first()?;
        one.iter()
            .find(|(byte, _)| byte == first)
            .map(|(_, operator)| (*operator, 1))
    }

    fn unary(&mut self) -> Result<Expression, ParseError> {
        self.spaces();
        for (word, operator) in [
            ("!", Unary::LogicalNot),
            ("~", Unary::Not),
            ("-", Unary::Negate),
        ] {
            if self.eat(word) {
                let operand = self.unary()?;
                return Ok(Expression::Unary {
                    operator,
                    operand: Box::new(operand),
                });
            }
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expression, ParseError> {
        self.spaces();
        let Some(byte) = self.peek() else {
            return Err(ParseError::Ended);
        };
        match byte {
            b'(' => {
                self.at += 1;
                let inside = self.expression(0)?;
                self.spaces();
                if self.peek() != Some(b')') {
                    return Err(ParseError::Unclosed {
                        at: self.at,
                        expected: ')',
                    });
                }
                self.at += 1;
                Ok(inside)
            }
            b'[' => {
                self.at += 1;
                let address = self.expression(0)?;
                self.spaces();
                if self.peek() != Some(b']') {
                    return Err(ParseError::Unclosed {
                        at: self.at,
                        expected: ']',
                    });
                }
                self.at += 1;
                // A width may follow, as `[rsp]:1` for one byte. Without one
                // it is a word, which is what a pointer is.
                let width = if self.eat(":") {
                    match self.peek() {
                        Some(b'1') => {
                            self.at += 1;
                            1
                        }
                        Some(b'2') => {
                            self.at += 1;
                            2
                        }
                        Some(b'4') => {
                            self.at += 1;
                            4
                        }
                        Some(b'8') => {
                            self.at += 1;
                            8
                        }
                        _ => {
                            return Err(ParseError::Unexpected {
                                at: self.at,
                                found: self.peek().map_or('\0', char::from),
                            });
                        }
                    }
                } else {
                    8
                };
                Ok(Expression::Load {
                    address: Box::new(address),
                    width,
                })
            }
            b'0'..=b'9' => self.number(),
            byte if byte.is_ascii_alphabetic() || byte == b'_' => self.name(),
            found => Err(ParseError::Unexpected {
                at: self.at,
                found: char::from(found),
            }),
        }
    }

    fn number(&mut self) -> Result<Expression, ParseError> {
        let start = self.at;
        let hexadecimal =
            self.text[self.at..].starts_with(b"0x") || self.text[self.at..].starts_with(b"0X");
        if hexadecimal {
            self.at += 2;
        }
        while matches!(self.peek(), Some(byte) if byte.is_ascii_alphanumeric() || byte == b'_') {
            self.at += 1;
        }
        let text: String = String::from_utf8_lossy(&self.text[start..self.at]).replace('_', "");
        let parsed = if hexadecimal {
            u64::from_str_radix(text.trim_start_matches("0x").trim_start_matches("0X"), 16)
        } else {
            text.parse::<u64>()
        };
        parsed
            .map(Expression::Number)
            .map_err(|_| ParseError::BadNumber { at: start, text })
    }

    fn name(&mut self) -> Result<Expression, ParseError> {
        let start = self.at;
        while matches!(self.peek(), Some(byte) if byte.is_ascii_alphanumeric() || byte == b'_') {
            self.at += 1;
        }
        let name = String::from_utf8_lossy(&self.text[start..self.at]).to_lowercase();
        if name == "rip" || name == "eip" || name == "pc" {
            return Ok(Expression::Pointer);
        }
        register_named(&name)
            .map(Expression::Register)
            .ok_or(ParseError::UnknownName { at: start, name })
    }
}

/// The register a name refers to, whatever width it names.
///
/// Written out rather than parsed from iced-x86's own formatter, so a
/// condition accepts exactly the names a reader would write and nothing else —
/// no segment registers, no vector registers, nothing the emulator does not
/// hold and would quietly answer zero for.
#[must_use]
pub fn register_named(name: &str) -> Option<iced_x86::Register> {
    use iced_x86::Register as R;
    Some(match name {
        "rax" => R::RAX,
        "eax" => R::EAX,
        "ax" => R::AX,
        "al" => R::AL,
        "ah" => R::AH,
        "rbx" => R::RBX,
        "ebx" => R::EBX,
        "bx" => R::BX,
        "bl" => R::BL,
        "bh" => R::BH,
        "rcx" => R::RCX,
        "ecx" => R::ECX,
        "cx" => R::CX,
        "cl" => R::CL,
        "ch" => R::CH,
        "rdx" => R::RDX,
        "edx" => R::EDX,
        "dx" => R::DX,
        "dl" => R::DL,
        "dh" => R::DH,
        "rsi" => R::RSI,
        "esi" => R::ESI,
        "si" => R::SI,
        "sil" => R::SIL,
        "rdi" => R::RDI,
        "edi" => R::EDI,
        "di" => R::DI,
        "dil" => R::DIL,
        "rsp" => R::RSP,
        "esp" => R::ESP,
        "sp" => R::SP,
        "spl" => R::SPL,
        "rbp" => R::RBP,
        "ebp" => R::EBP,
        "bp" => R::BP,
        "bpl" => R::BPL,
        "r8" => R::R8,
        "r8d" => R::R8D,
        "r8w" => R::R8W,
        "r8b" => R::R8L,
        "r9" => R::R9,
        "r9d" => R::R9D,
        "r9w" => R::R9W,
        "r9b" => R::R9L,
        "r10" => R::R10,
        "r10d" => R::R10D,
        "r10w" => R::R10W,
        "r10b" => R::R10L,
        "r11" => R::R11,
        "r11d" => R::R11D,
        "r11w" => R::R11W,
        "r11b" => R::R11L,
        "r12" => R::R12,
        "r12d" => R::R12D,
        "r12w" => R::R12W,
        "r12b" => R::R12L,
        "r13" => R::R13,
        "r13d" => R::R13D,
        "r13w" => R::R13W,
        "r13b" => R::R13L,
        "r14" => R::R14,
        "r14d" => R::R14D,
        "r14w" => R::R14W,
        "r14b" => R::R14L,
        "r15" => R::R15,
        "r15d" => R::R15D,
        "r15w" => R::R15W,
        "r15b" => R::R15L,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        Permissions, Section,
        emulate::memory::{Memory, PAGE},
    };
    use iced_x86::Register;

    /// A register file and one writable page, for a condition to read.
    fn state() -> (Registers, Memory) {
        let sections = vec![Section {
            name: String::from(".data"),
            virtual_address: 0x1000,
            file_offset: 0,
            virtual_size: PAGE,
            file_size: 0,
            permissions: Permissions {
                read: true,
                write: true,
                execute: false,
            },
            entropy: None,
        }];
        let memory = Memory::from_sections(Arc::from(Vec::new()), &sections);
        (Registers::new(), memory)
    }

    fn holds(source: &str, registers: &Registers, memory: &Memory) -> bool {
        Expression::parse(source)
            .unwrap_or_else(|error| panic!("{source:?} should parse: {error:?}"))
            .holds(registers, memory)
    }

    #[test]
    fn a_register_is_compared_against_a_number() {
        let (mut registers, memory) = state();
        registers.set(Register::RCX, 3);
        assert!(holds("rcx == 3", &registers, &memory));
        assert!(holds("rcx != 4", &registers, &memory));
        assert!(!holds("rcx == 4", &registers, &memory));
        assert!(holds("rcx == 0x3", &registers, &memory), "hexadecimal too");
    }

    #[test]
    fn a_narrow_register_is_the_narrow_part_of_the_wide_one() {
        let (mut registers, memory) = state();
        registers.set(Register::RAX, 0x1234_5678_9abc_def0);
        assert!(holds("al == 0xf0", &registers, &memory));
        assert!(holds("ax == 0xdef0", &registers, &memory));
        assert!(holds("eax == 0x9abcdef0", &registers, &memory));
    }

    #[test]
    fn comparisons_are_signed_because_that_is_what_a_reader_means() {
        let (mut registers, memory) = state();
        registers.set(Register::RAX, u64::MAX); // -1
        assert!(
            holds("rax < 0", &registers, &memory),
            "-1 is less than zero, not larger than everything"
        );
        assert!(holds("rax == -1", &registers, &memory));
    }

    #[test]
    fn memory_is_read_through_brackets_at_the_width_asked_for() {
        let (mut registers, mut memory) = state();
        for (step, byte) in [0x11_u8, 0x22, 0x33, 0x44].into_iter().enumerate() {
            assert!(memory.poke(0x1000 + step as u64, byte));
        }
        registers.set(Register::RDI, 0x1000);
        assert!(holds("[rdi]:1 == 0x11", &registers, &memory));
        assert!(holds("[rdi]:2 == 0x2211", &registers, &memory));
        assert!(holds("[rdi]:4 == 0x44332211", &registers, &memory));
        assert!(
            holds("[rdi + 1]:1 == 0x22", &registers, &memory),
            "and arithmetic in the address"
        );
    }

    /// Reading somewhere unmapped is false, not an error: the run must not
    /// stop because a condition asked about a pointer that is not one yet.
    /// Reading somewhere unmapped has no value, so nothing built on it holds
    /// — including a comparison against zero, which is the trap: a pointer
    /// that leads nowhere is not a pointer to a zero.
    #[test]
    fn reading_absent_memory_has_no_value_rather_than_a_value_of_zero() {
        let (registers, memory) = state();
        let no_value = |source: &str| {
            let expression = Expression::parse(source).expect("parses");
            assert_eq!(expression.value(&registers, &memory), None, "{source}");
            assert!(!expression.holds(&registers, &memory), "{source}");
        };
        no_value("[0]:1");
        no_value("[0]:1 == 0");
        no_value("[0xdeadbeef]:8 == 0");
        no_value("[0]:1 != 0");
        no_value("[0]:1 + 1 == 1");
    }

    #[test]
    fn and_stops_before_reading_through_a_null_pointer() {
        let (mut registers, memory) = state();
        registers.set(Register::RAX, 0);
        // The right-hand side would read address zero. It must not be reached.
        assert!(!holds("rax != 0 && [rax]:1 == 0", &registers, &memory));
        registers.set(Register::RAX, 0x1000);
        assert!(holds("rax != 0 && [rax]:1 == 0", &registers, &memory));
    }

    #[test]
    fn precedence_is_what_a_calculator_would_do() {
        let (registers, memory) = state();
        assert!(holds("1 + 2 * 3 == 7", &registers, &memory));
        assert!(holds("(1 + 2) * 3 == 9", &registers, &memory));
        assert!(holds("1 == 1 && 2 == 2", &registers, &memory));
        assert!(holds("0 == 1 || 2 == 2", &registers, &memory));
        assert!(holds("!(1 == 2)", &registers, &memory));
        assert!(holds("(1 << 4) == 16", &registers, &memory));
    }

    #[test]
    fn the_instruction_pointer_is_readable_under_its_own_names() {
        let (mut registers, memory) = state();
        registers.instruction_pointer = 0x4000;
        for name in ["rip", "eip", "pc"] {
            assert!(
                holds(&format!("{name} == 0x4000"), &registers, &memory),
                "{name}"
            );
        }
    }

    /// Nothing that does not parse is ever accepted quietly.
    #[test]
    fn what_does_not_parse_says_where_it_stopped() {
        assert!(matches!(
            Expression::parse("rax == "),
            Err(ParseError::Ended)
        ));
        assert!(matches!(
            Expression::parse("nonsense == 1"),
            Err(ParseError::UnknownName { .. })
        ));
        assert!(matches!(
            Expression::parse("(1 + 2"),
            Err(ParseError::Unclosed { expected: ')', .. })
        ));
        assert!(matches!(
            Expression::parse("[rax"),
            Err(ParseError::Unclosed { expected: ']', .. })
        ));
        assert!(matches!(
            Expression::parse("1 2"),
            Err(ParseError::Trailing { .. })
        ));
        assert!(matches!(
            Expression::parse("#"),
            Err(ParseError::Unexpected { .. })
        ));
    }

    /// A condition is a question, and asking it must never bring anything down.
    #[test]
    fn nothing_a_reader_can_write_can_panic() {
        let (mut registers, memory) = state();
        registers.set(Register::RAX, 0);
        for source in [
            "1 / rax",
            "1 % rax",
            "1 << 999",
            "1 >> 999",
            "-rax",
            "~rax",
            "[[[rax]]]",
        ] {
            let expression = Expression::parse(source).unwrap_or_else(|error| {
                panic!("{source:?} should parse: {error:?}");
            });
            let _ = expression.value(&registers, &memory);
        }
    }
}
