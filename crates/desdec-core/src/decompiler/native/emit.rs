//! Writing the C out, one line at a time, each knowing where it came from.
//!
//! The output is a list of [`Line`]s rather than a string, and every line
//! carries the address of the instruction it was produced from. That is the
//! one thing this decompiler has that an external engine cannot give: `RetDec`
//! and rz-ghidra publish no line-to-address map, which is why the view built
//! on them can only offer a button naming the whole function and says so.
//! Here, a click on `if (count <= 8)` can go to the `cmp` it came from.
//!
//! Two rules govern what is printed.
//!
//! **Parentheses only where they change the meaning.** C's own precedence
//! table is in [`Binary::spelling`], and an expression is wrapped only when
//! its operator binds more loosely than the one above it. `a + b * c` prints
//! as itself; `(a + b) * c` keeps its brackets.
//!
//! **Nothing is printed that was not derived.** An instruction the lifter did
//! not model appears as a comment holding its own assembly, not as a plausible
//! C statement. A width that was never established is not invented. This makes
//! the output look less finished than a decompiler that guesses, and it is the
//! reason a reader can act on it.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::{
    analysis::blocks::BasicBlock,
    decompiler::native::{
        ir::{Callee, Expr, Place, Statement, Stmt, Unary, Width},
        naming::Naming,
        structure::{Structure, Structured},
    },
};

/// One line of the output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Line {
    pub text: String,
    /// The instruction this line was produced from, when one line came from
    /// one instruction. A brace, a declaration and the signature have none.
    pub address: Option<u64>,
    pub indent: usize,
}

impl Line {
    fn new(indent: usize, address: Option<u64>, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            address,
            indent,
        }
    }

    /// The line as it is written, indented.
    #[must_use]
    pub fn rendered(&self) -> String {
        format!("{}{}", "    ".repeat(self.indent), self.text)
    }
}

/// One decompiled function.
#[derive(Clone, Debug)]
pub struct Decompiled {
    pub name: String,
    pub address: u64,
    pub lines: Vec<Line>,
    /// Instructions the lifter did not model, so the view can say how much of
    /// the function this is a reading of rather than leaving the reader to
    /// count comments.
    pub unmodelled: usize,
    pub instructions: usize,
}

impl Decompiled {
    /// The whole function as text, for copying and for the tests.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            let _ = writeln!(out, "{}", line.rendered());
        }
        out
    }

    /// How much of the body was understood, as a fraction, for the view to
    /// show. `None` when there was nothing to understand.
    #[must_use]
    pub fn coverage(&self) -> Option<f32> {
        if self.instructions == 0 {
            return None;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "a ratio of two instruction counts, shown to one decimal place"
        )]
        Some(1.0 - (self.unmodelled as f32 / self.instructions as f32))
    }
}

/// What the emitter needs besides the structure.
pub struct Source<'a> {
    pub name: &'a str,
    pub address: u64,
    pub blocks: &'a [BasicBlock],
    pub statements: &'a [Vec<Statement>],
    pub naming: &'a Naming,
    pub structure: &'a Structure,
}

/// Writes one function out.
#[must_use]
pub fn write(source: &Source<'_>) -> Decompiled {
    let mut writer = Writer {
        source,
        lines: Vec::new(),
        locals: source
            .naming
            .locals
            .iter()
            .map(|local| (local.id, local.name.clone()))
            .collect(),
        unmodelled: 0,
        instructions: 0,
    };
    writer.signature();
    writer.declarations();
    writer.structured(&source.structure.root, 1);
    writer
        .lines
        .push(Line::new(0, None, "}".to_owned()));
    label_every_goto(&mut writer.lines);
    Decompiled {
        name: source.name.to_owned(),
        address: source.address,
        lines: writer.lines,
        unmodelled: writer.unmodelled,
        instructions: writer.instructions,
    }
}

/// Puts a label in front of every block a printed `goto` names.
///
/// The structurer says which blocks it could not account for, and those are
/// labelled as they are written. A `goto` can also survive as a block's own
/// terminator — a two-armed branch whose condition was not modelled keeps it —
/// and one naming a label that is nowhere is C that does not compile. Cheaper
/// and surer to check the finished text than to have every path that can emit
/// a branch remember to say so.
fn label_every_goto(lines: &mut Vec<Line>) {
    let mut named: Vec<u64> = Vec::new();
    for line in lines.iter() {
        if let Some(rest) = line.text.strip_prefix("goto ").or_else(|| {
            line.text
                .split_once(") goto ")
                .map(|(_, rest)| rest)
        }) && let Some(hexadecimal) = rest.trim_end_matches(';').strip_prefix("label_")
            && let Ok(address) = u64::from_str_radix(hexadecimal, 16)
        {
            named.push(address);
        }
    }
    named.sort_unstable();
    named.dedup();
    for address in named {
        if lines
            .iter()
            .any(|line| line.text == format!("{}:", label(address)))
        {
            continue;
        }
        let Some(position) = lines
            .iter()
            .position(|line| line.address == Some(address) && line.indent > 0)
        else {
            continue;
        };
        // The block it names is the one the structurer reached by another
        // route; the label goes at its first line, where the flow arrives.
        let indent = lines[position].indent.saturating_sub(1);
        lines.insert(
            position,
            Line::new(indent, Some(address), format!("{}:", label(address))),
        );
    }
}

struct Writer<'a> {
    source: &'a Source<'a>,
    lines: Vec<Line>,
    locals: HashMap<u32, String>,
    unmodelled: usize,
    instructions: usize,
}

impl Writer<'_> {
    fn signature(&mut self) {
        let naming = self.source.naming;
        let answer = naming
            .returns
            .map_or_else(|| "void".to_owned(), |width| width.c_name(true).to_owned());
        let parameters = if naming.parameters.is_empty() {
            "void".to_owned()
        } else {
            naming
                .parameters
                .iter()
                .map(|parameter| {
                    format!("{} {}", parameter.width.c_name(false), parameter.name)
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        self.lines.push(Line::new(
            0,
            Some(self.source.address),
            format!("{answer} {}({parameters})", self.source.name),
        ));
        self.lines.push(Line::new(0, None, "{".to_owned()));
    }

    /// The frame, declared at the top the way C requires and a reader expects,
    /// each slot saying where in the listing it lives.
    fn declarations(&mut self) {
        if self.source.naming.locals.is_empty() {
            return;
        }
        for local in &self.source.naming.locals {
            // A slot nothing ever read is a buffer, and its extent came from
            // the frame rather than from an instruction. Declared as what it
            // is: `uint8_t local_408[0x400]` says both that it is an array and
            // how the size was arrived at, which `uint64_t local_408` says
            // neither of.
            let declaration = match local.buffer {
                Some(bytes) => format!(
                    "uint8_t {}[{bytes:#x}];  // {} — extent from the frame",
                    local.name, local.label
                ),
                None => format!(
                    "{} {};  // {}",
                    local.width.c_name(false),
                    local.name,
                    local.label
                ),
            };
            self.lines.push(Line::new(1, None, declaration));
        }
        self.lines.push(Line::new(1, None, String::new()));
    }

    fn structured(&mut self, item: &Structured, indent: usize) {
        match item {
            Structured::Nothing => {}
            Structured::Sequence(items) => {
                for inner in items {
                    self.structured(inner, indent);
                }
            }
            Structured::Block {
                index,
                with_terminator,
            } => self.block(*index, *with_terminator, indent),
            Structured::If {
                condition,
                at,
                then_branch,
                else_branch,
            } => {
                self.lines.push(Line::new(
                    indent,
                    Some(*at),
                    format!("if ({}) {{", expression(condition, 0, &self.locals, self.source.naming)),
                ));
                self.structured(then_branch, indent + 1);
                // An `else` the structurer found and that has nothing in it is
                // an arm whose every statement was dead. The branch is still
                // real and the `if` says so; two braces around nothing do not.
                if let Some(other) = else_branch
                    && **other != Structured::Nothing
                {
                    self.lines
                        .push(Line::new(indent, None, "} else {".to_owned()));
                    self.structured(other, indent + 1);
                }
                self.lines.push(Line::new(indent, None, "}".to_owned()));
            }
            Structured::While {
                condition,
                at,
                body,
            } => {
                self.lines.push(Line::new(
                    indent,
                    Some(*at),
                    format!(
                        "while ({}) {{",
                        expression(condition, 0, &self.locals, self.source.naming)
                    ),
                ));
                self.structured(body, indent + 1);
                self.lines.push(Line::new(indent, None, "}".to_owned()));
            }
            Structured::DoWhile {
                body,
                condition,
                at,
            } => {
                self.lines.push(Line::new(indent, None, "do {".to_owned()));
                self.structured(body, indent + 1);
                self.lines.push(Line::new(
                    indent,
                    Some(*at),
                    format!(
                        "}} while ({});",
                        expression(condition, 0, &self.locals, self.source.naming)
                    ),
                ));
            }
            Structured::Loop { body } => {
                // `while (1)`, because the loop's own ways out are `break`s and
                // `return`s. Saying `for (;;)` would be the same thing said
                // less plainly.
                self.lines
                    .push(Line::new(indent, None, "while (1) {".to_owned()));
                self.structured(body, indent + 1);
                self.lines.push(Line::new(indent, None, "}".to_owned()));
            }
            Structured::Break => self.lines.push(Line::new(indent, None, "break;".to_owned())),
            Structured::Continue => self
                .lines
                .push(Line::new(indent, None, "continue;".to_owned())),
            Structured::Goto { target } => self.lines.push(Line::new(
                indent,
                Some(*target),
                format!("goto {};", label(*target)),
            )),
            Structured::Label { target, body } => {
                self.lines.push(Line::new(
                    indent.saturating_sub(1),
                    Some(*target),
                    format!("{}:", label(*target)),
                ));
                self.structured(body, indent);
            }
        }
    }

    /// The statements of one block.
    fn block(&mut self, index: usize, with_terminator: bool, indent: usize) {
        let Some(statements) = self.source.statements.get(index) else {
            return;
        };
        if let Some(block) = self.source.blocks.get(index) {
            self.instructions += block.instruction_count();
            if self.source.structure.labelled.contains(&block.start) {
                self.lines.push(Line::new(
                    indent.saturating_sub(1),
                    Some(block.start),
                    format!("{}:", label(block.start)),
                ));
            }
        }
        for statement in statements {
            if !with_terminator && matches!(statement.effect, Stmt::Branch { .. }) {
                // Where the flow goes from here is what the `if`, the `while`
                // or the line below already says. Printing the branch as well
                // would say it twice — and as a `goto`, which is the one thing
                // structuring exists to remove.
                continue;
            }
            if let Some(text) = self.statement(statement) {
                self.lines
                    .push(Line::new(indent, Some(statement.address), text));
            }
        }
    }

    fn statement(&mut self, statement: &Statement) -> Option<String> {
        let naming = self.source.naming;
        let locals = &self.locals;
        Some(match &statement.effect {
            Stmt::Nothing => return None,
            Stmt::Assign { place, value } => format!(
                "{} = {};",
                self::place(place, locals, naming),
                expression(value, 0, locals, naming)
            ),
            Stmt::Call {
                result,
                callee,
                arguments,
            } => {
                let call = format!(
                    "{}({})",
                    self::callee(callee, locals, naming),
                    arguments
                        .iter()
                        .map(|argument| expression(argument, 0, locals, naming))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                // The result is written only where something reads it: after
                // the dataflow pass a call whose answer is unused has had its
                // result taken off, and printing `rax = puts(…)` for one would
                // put a register back into the output for no reason.
                match result {
                    Some(place) if reads_result(place) => {
                        format!("{} = {call};", self::place(place, locals, naming))
                    }
                    _ => format!("{call};"),
                }
            }
            Stmt::Return(Some(value)) => {
                format!("return {};", expression(value, 0, locals, naming))
            }
            Stmt::Return(None) => "return;".to_owned(),
            Stmt::Branch {
                condition: Some(condition),
                target,
            } => format!(
                "if ({}) goto {};",
                expression(condition, 0, locals, naming),
                label(*target)
            ),
            Stmt::Branch {
                condition: None,
                target,
            } => format!("goto {};", label(*target)),
            Stmt::IndirectBranch(value) => format!(
                "goto *{};  // through a value: a switch table, or a function pointer",
                expression(value, 12, locals, naming)
            ),
            Stmt::SystemCall { number } => match number {
                Some(number) => format!("system_call({number:#x});"),
                None => "system_call();  // the number is in the registers above".to_owned(),
            },
            Stmt::Opaque(text) => {
                self.unmodelled += 1;
                format!("/* not modelled: {text} */")
            }
        })
    }
}

/// Whether a call's result place is one worth printing.
///
/// A register left over from lifting is not: it says `rax` where the answer
/// went nowhere, and where it did go somewhere the dataflow pass has already
/// moved the whole call into the line that reads it. A local or a memory
/// location is, because something really did keep the answer there.
const fn reads_result(place: &Place) -> bool {
    matches!(place, Place::Local { .. } | Place::Memory { .. })
}

fn label(address: u64) -> String {
    format!("label_{address:x}")
}

fn callee(callee: &Callee, locals: &HashMap<u32, String>, naming: &Naming) -> String {
    match callee {
        Callee::Named(name) => name.clone(),
        Callee::Address(address) => format!("function_{address:x}"),
        Callee::Indirect(value) => {
            format!("({})", expression(value, 0, locals, naming))
        }
    }
}

fn place(place: &Place, locals: &HashMap<u32, String>, naming: &Naming) -> String {
    match place {
        Place::Register(register) => naming
            .name_of(*register)
            .map_or_else(|| register.name(), ToOwned::to_owned),
        Place::Condition(condition) => condition.name().to_owned(),
        Place::Local { id, .. } => locals
            .get(id)
            .cloned()
            .unwrap_or_else(|| format!("local_{id}")),
        Place::Memory { address, width } => format!(
            "*({} *)({})",
            width.c_name(false),
            expression(address, 0, locals, naming)
        ),
    }
}

/// An expression, bracketed only where the precedence needs it.
fn expression(
    value: &Expr,
    outer: u8,
    locals: &HashMap<u32, String>,
    naming: &Naming,
) -> String {
    match value {
        Expr::Const { value, width } => constant(*value, *width),
        Expr::Read(inner) => place(inner, locals, naming),
        Expr::AddressOf(inner) => {
            // In C an array's name already is the address of its first
            // element, and `&` in front of one has a different type from the
            // pointer the code is actually passing.
            if let Place::Local { id, .. } = inner.as_ref()
                && naming.is_buffer(*id)
            {
                place(inner, locals, naming)
            } else {
                format!("&{}", place(inner, locals, naming))
            }
        }
        Expr::Symbol { name, .. } => name.clone(),
        Expr::Unknown(text) => text.clone(),
        Expr::Unary { operator, operand } => {
            let symbol = match operator {
                Unary::Negate => "-",
                Unary::Not => "~",
                Unary::LogicalNot => "!",
            };
            format!("{symbol}{}", expression(operand, 11, locals, naming))
        }
        Expr::Binary {
            operator,
            left,
            right,
        } => {
            let (symbol, precedence) = operator.spelling();
            // An unsigned comparison is a different question from a signed one
            // and C tells them apart by the type of the operands, so the cast
            // is the operator: without it `jb` would print as `<` and read as
            // the signed test it is not.
            let (left, right) = if operator.is_unsigned_comparison() {
                (
                    unsigned(left, precedence, locals, naming),
                    unsigned(right, precedence, locals, naming),
                )
            } else {
                (
                    expression(left, precedence, locals, naming),
                    expression(right, precedence + 1, locals, naming),
                )
            };
            let text = format!("{left} {symbol} {right}");
            if precedence < outer {
                format!("({text})")
            } else {
                text
            }
        }
        Expr::Cast {
            value,
            width,
            signed,
        } => format!(
            "({}){}",
            width.c_name(*signed),
            expression(value, 11, locals, naming)
        ),
        Expr::Call { callee, arguments } => format!(
            "{}({})",
            self::callee(callee, locals, naming),
            arguments
                .iter()
                .map(|argument| expression(argument, 0, locals, naming))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expr::Select {
            condition,
            when_true,
            when_false,
        } => {
            let text = format!(
                "{} ? {} : {}",
                expression(condition, 3, locals, naming),
                expression(when_true, 0, locals, naming),
                expression(when_false, 0, locals, naming)
            );
            if outer > 0 { format!("({text})") } else { text }
        }
    }
}

/// One side of an unsigned comparison, cast so that C asks the question the
/// instruction asked.
fn unsigned(
    value: &Expr,
    precedence: u8,
    locals: &HashMap<u32, String>,
    naming: &Naming,
) -> String {
    let width = value.width().unwrap_or(Width::Qword);
    if matches!(value, Expr::Const { .. }) {
        return expression(value, precedence, locals, naming);
    }
    format!(
        "({}){}",
        width.c_name(false),
        expression(value, 11, locals, naming)
    )
}

/// A number, written the way it is most readable.
///
/// Three readings, and the choice between them matters more than it sounds:
/// `if (count <= 0xa)` and `if (count <= 10)` are the same condition, and only
/// one of them reads like something a person wrote.
///
/// - **A small negative number in decimal.** `-1` is what `0xFFFFFFFF` means,
///   and a comparison against the latter is unreadable.
/// - **A mask or an address in hexadecimal.** A power of two, one less than a
///   power of two, and anything large: those are bit patterns and places, and
///   their hexadecimal is the form that shows what they are.
/// - **Everything else in decimal**, because it is a count, a size, or an
///   index — which is what a small number that is not a bit pattern is.
fn constant(value: u64, width: Width) -> String {
    if value <= 9 {
        return value.to_string();
    }
    let bits = width.bits();
    if bits < 64 {
        let sign = 1_u64 << (bits - 1);
        if value & sign != 0 {
            let magnitude = (1_u64 << bits) - value;
            if magnitude <= 0x1000 {
                return format!("-{magnitude}");
            }
        }
    } else if value >= u64::MAX - 0x1000 {
        return format!("-{}", u64::MAX - value + 1);
    }
    if value < 4096 && !looks_like_a_bit_pattern(value) {
        return value.to_string();
    }
    format!("{value:#x}")
}

/// Whether a number is a mask rather than a count.
///
/// A power of two is a single bit, and one less than a power of two is a run
/// of them: `0x20` and `0xFF` are what the code is doing, and `32` and `255`
/// hide it.
const fn looks_like_a_bit_pattern(value: u64) -> bool {
    value.is_power_of_two() || (value + 1).is_power_of_two()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decompiler::native::ir::{Binary, Register};
    use crate::decompiler::native::naming::{Convention, recognise};

    fn naming() -> Naming {
        recognise(&[], Convention::SystemV)
    }

    fn rendered(value: &Expr) -> String {
        expression(value, 0, &HashMap::new(), &naming())
    }

    /// C's own precedence, so the output does not read like a machine wrote it.
    #[test]
    fn brackets_appear_only_where_they_change_the_meaning() {
        let a = Expr::register(Register::new("rax", Width::Qword));
        let b = Expr::register(Register::new("rbx", Width::Qword));
        let c = Expr::register(Register::new("rcx", Width::Qword));
        let product_then_sum = Expr::binary(
            Binary::Add,
            a.clone(),
            Expr::binary(Binary::Multiply, b.clone(), c.clone()),
        );
        assert_eq!(rendered(&product_then_sum), "rax + rbx * rcx");
        let sum_then_product = Expr::binary(
            Binary::Multiply,
            Expr::binary(Binary::Add, a, b),
            c,
        );
        assert_eq!(rendered(&sum_then_product), "(rax + rbx) * rcx");
    }

    /// `jb` is not `jl`, and C tells the two apart by the type of the operands.
    #[test]
    fn an_unsigned_comparison_is_cast_so_that_c_asks_the_same_question() {
        let comparison = Expr::binary(
            Binary::Below,
            Expr::register(Register::new("rax", Width::Qword)),
            Expr::constant(0x20, Width::Qword),
        );
        assert_eq!(rendered(&comparison), "(uint64_t)rax < 0x20");
    }

    /// `0xFFFFFFFF` compared against is unreadable; `-1` is what it means.
    #[test]
    fn a_small_negative_number_is_written_as_one() {
        assert_eq!(constant(0xFFFF_FFFF, Width::Dword), "-1");
        assert_eq!(constant(8, Width::Dword), "8");
        assert_eq!(constant(0x1000, Width::Dword), "0x1000");
        assert_eq!(constant(0x0040_10A0, Width::Qword), "0x4010a0");
    }

    #[test]
    fn a_conditional_move_prints_as_the_operator_c_has_for_it() {
        let select = Expr::Select {
            condition: Box::new(Expr::binary(
                Binary::Less,
                Expr::register(Register::new("rax", Width::Dword)),
                Expr::constant(0, Width::Dword),
            )),
            when_true: Box::new(Expr::constant(0, Width::Dword)),
            when_false: Box::new(Expr::register(Register::new("rbx", Width::Dword))),
        };
        assert_eq!(rendered(&select), "eax < 0 ? 0 : ebx");
    }

    #[test]
    fn a_memory_read_says_how_wide_it_is() {
        let read = Expr::read(Place::Memory {
            address: Box::new(Expr::binary(
                Binary::Add,
                Expr::register(Register::new("rax", Width::Qword)),
                Expr::constant(8, Width::Qword),
            )),
            width: Width::Dword,
        });
        assert_eq!(rendered(&read), "*(uint32_t *)(rax + 8)");
    }
}
