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
        ir::{Callee, Expr, Place, Register, Statement, Stmt, Unary, Width},
        lift::register_named,
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
///
/// Twice, and the second time is not waste. What a register has to be called
/// depends on the widest window the finished text uses on it — a function that
/// writes `%eax` and later reads `%rax` has one variable, sixty-four bits
/// wide, and the narrow accesses are windows onto it. That is not known until
/// the text exists, so the text is written once to find out and once to say
/// it. Both passes are pure and take microseconds.
#[must_use]
pub fn write(source: &Source<'_>) -> Decompiled {
    let first = render(source, &Registers::default());
    let registers = Registers::of(&first.lines);
    if registers.is_empty() {
        return first;
    }
    render(source, &registers)
}

fn render(source: &Source<'_>, registers: &Registers) -> Decompiled {
    let mut writer = Writer {
        source,
        lines: Vec::new(),
        locals: source
            .naming
            .locals
            .iter()
            .map(|local| (local.id, local.name.clone()))
            .collect(),
        registers,
        unmodelled: 0,
        instructions: 0,
    };
    writer.signature();
    writer.declarations();
    writer.structured(&source.structure.root, 1);
    writer.lines.push(Line::new(0, None, "}".to_owned()));
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
        if let Some(rest) = line
            .text
            .strip_prefix("goto ")
            .or_else(|| line.text.split_once(") goto ").map(|(_, rest)| rest))
            && let Some(hexadecimal) = rest.trim_end_matches(';').strip_prefix("label_")
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
    registers: &'a Registers,
    unmodelled: usize,
    instructions: usize,
}

/// The registers that survived every pass and are printed under their own
/// names, each with the widest window the output uses on it.
///
/// Without this the output named `eax` and declared nothing: three quarters of
/// the functions in an optimised binary referred to a variable that was
/// nowhere, which is C that does not compile and, worse, C in which `eax` and
/// `rax` look like two things when the machine has one. The declaration is per
/// *register*, and the narrow accesses are written as what they are — windows
/// onto it.
///
/// The vector registers are kept by name rather than by register: `%xmm0` and
/// `%ymm0` are one register too, but the merge that a narrow write to one
/// performs cannot be written as a mask over a value C has no literal for. A
/// function that addresses the same vector register at two widths — which is
/// the AVX/SSE transition every compiler works to avoid — therefore gets two
/// variables, and that much is not exact.
#[derive(Debug, Default)]
struct Registers {
    general: HashMap<&'static str, Width>,
    vectors: Vec<(String, Width)>,
}

impl Registers {
    /// Reads back the registers a first pass printed.
    ///
    /// From the text rather than from the statements, because what reaches the
    /// text is what the structurer kept: a block it dropped, or an arm the
    /// dataflow pass emptied, must not leave a declaration standing for a
    /// value nothing mentions. The names are a closed set and the comments —
    /// which hold the assembly, and so hold every register — are skipped.
    fn of(lines: &[Line]) -> Self {
        let mut general: HashMap<&'static str, Width> = HashMap::new();
        let mut vectors: Vec<(String, Width)> = Vec::new();
        for line in lines {
            let text = line.text.split("/*").next().unwrap_or_default();
            let text = text.split("//").next().unwrap_or_default();
            for word in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
                let Some(register) = register_named(word) else {
                    continue;
                };
                if register.name() != word || register.root == "rip" {
                    continue;
                }
                if register.root.starts_with("xmm") {
                    if !vectors.iter().any(|(name, _)| name == word) {
                        vectors.push((word.to_owned(), register.width));
                    }
                    continue;
                }
                let entry = general.entry(register.root).or_insert(register.width);
                *entry = (*entry).max(register.width);
            }
        }
        vectors.sort();
        Self { general, vectors }
    }

    fn is_empty(&self) -> bool {
        self.general.is_empty() && self.vectors.is_empty()
    }

    /// The width the variable standing for this register is declared at, when
    /// one is.
    fn declared(&self, register: Register) -> Option<Width> {
        self.general.get(register.root).copied()
    }

    /// The variable's own name, which is the register at its declared width.
    fn variable(&self, register: Register) -> Option<String> {
        self.declared(register)
            .map(|width| Register::new(register.root, width).name())
    }
}

/// A register being read, as a window onto the variable declared for it.
///
/// `%al` of a register the output also uses whole is `(uint8_t)rax`, which is
/// exactly what the instruction read. Where the access *is* the declared width
/// there is no window and the name stands alone.
fn read_register(register: Register, registers: &Registers) -> String {
    let Some(name) = registers.variable(register) else {
        return register.name();
    };
    let Some(declared) = registers.declared(register) else {
        return register.name();
    };
    if register.high_byte {
        return format!("(uint8_t)({name} >> 8)");
    }
    if register.width >= declared {
        return name;
    }
    format!("({}){name}", register.width.c_name(false))
}

/// A register being written, as the whole assignment.
///
/// A narrow write is not a narrow assignment. Writing `%eax` on x86-64 clears
/// the top half of `%rax`, so `rax = (uint32_t)value` says it exactly; writing
/// `%al` or `%ax` clears nothing, and the only exact spelling is the merge the
/// machine performs. Getting this wrong is the difference between code that
/// reads right and code that is quietly false, which is why it is written out
/// rather than approximated by an assignment to a name that does not exist.
fn write_register(register: Register, value: &str, registers: &Registers) -> String {
    let (Some(name), Some(declared)) = (registers.variable(register), registers.declared(register))
    else {
        return format!("{} = {value};", register.name());
    };
    let narrow = register.width.c_name(false);
    if register.high_byte {
        let mask = mask_outside(declared, 8, 8);
        return format!("{name} = ({name} & {mask:#x}) | ((uint64_t)(uint8_t)({value}) << 8);");
    }
    if register.width >= declared {
        return format!("{name} = {value};");
    }
    // The one narrowing the architecture makes total: a 32-bit write zeroes
    // the other thirty-two bits, so the assignment alone is the whole truth.
    if register.width == Width::Dword {
        return format!("{name} = ({narrow})({value});");
    }
    let mask = mask_outside(declared, 0, register.width.bits());
    format!("{name} = ({name} & {mask:#x}) | ({narrow})({value});")
}

/// The bits of a `declared`-wide value that a write of `bits` bits starting at
/// `offset` leaves standing.
fn mask_outside(declared: Width, offset: u32, bits: u32) -> u64 {
    let whole = match declared.bits() {
        64 => u64::MAX,
        other => (1_u64 << other) - 1,
    };
    let window = match bits {
        64 => u64::MAX,
        other => (1_u64 << other) - 1,
    };
    whole & !(window << offset)
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
                .map(|parameter| format!("{} {}", parameter.width.c_name(false), parameter.name))
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
        if self.source.naming.locals.is_empty() && self.registers.is_empty() {
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
        // Then the registers nothing gave a name to. They are declared for the
        // same reason the frame is: the text names them, and a name the text
        // does not declare is a hole the reader has to fill from the listing.
        let mut general: Vec<(&str, Width)> = self
            .registers
            .general
            .iter()
            .map(|(root, width)| (*root, *width))
            .collect();
        general.sort_unstable();
        for (root, width) in general {
            let name = Register::new(root, width).name();
            self.lines.push(Line::new(
                1,
                None,
                format!(
                    "{} {name};  // %{root}, which nothing named",
                    width.c_name(false)
                ),
            ));
        }
        for (name, width) in &self.registers.vectors {
            self.lines.push(Line::new(
                1,
                None,
                format!(
                    "{} {name};  // %{name}, which nothing named",
                    width.c_name(false)
                ),
            ));
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
                    format!(
                        "if ({}) {{",
                        expression(
                            condition,
                            0,
                            &self.locals,
                            self.source.naming,
                            self.registers
                        )
                    ),
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
                        expression(
                            condition,
                            0,
                            &self.locals,
                            self.source.naming,
                            self.registers
                        )
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
                        expression(
                            condition,
                            0,
                            &self.locals,
                            self.source.naming,
                            self.registers
                        )
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
            Structured::Break => self
                .lines
                .push(Line::new(indent, None, "break;".to_owned())),
            Structured::Continue => {
                self.lines
                    .push(Line::new(indent, None, "continue;".to_owned()))
            }
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
        let registers = self.registers;
        Some(match &statement.effect {
            Stmt::Nothing => return None,
            // The spelling every compiler that emits `ud2` also understands,
            // and the one that says what it does: this raises, and nothing
            // below it runs.
            Stmt::Trap => "__builtin_trap();".to_owned(),
            Stmt::Assign { place, value } => {
                let value = expression(value, 0, locals, naming, registers);
                match place {
                    Place::Register(register) if naming.name_of(*register).is_none() => {
                        write_register(*register, &value, registers)
                    }
                    other => format!(
                        "{} = {value};",
                        self::place(other, locals, naming, registers)
                    ),
                }
            }
            Stmt::Call {
                result,
                callee,
                arguments,
            } => {
                let call = format!(
                    "{}({})",
                    self::callee(callee, locals, naming, registers),
                    arguments
                        .iter()
                        .map(|argument| expression(argument, 0, locals, naming, registers))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                // The result is written only where something reads it: after
                // the dataflow pass a call whose answer is unused has had its
                // result taken off, and printing `rax = puts(…)` for one would
                // put a register back into the output for no reason.
                match result {
                    Some(place) if reads_result(place) => {
                        format!(
                            "{} = {call};",
                            self::place(place, locals, naming, registers)
                        )
                    }
                    _ => format!("{call};"),
                }
            }
            Stmt::Return(Some(value)) => {
                format!(
                    "return {};",
                    expression(value, 0, locals, naming, registers)
                )
            }
            Stmt::Return(None) => "return;".to_owned(),
            Stmt::Branch {
                condition: Some(condition),
                target,
            } => format!(
                "if ({}) goto {};",
                expression(condition, 0, locals, naming, registers),
                label(*target)
            ),
            Stmt::Branch {
                condition: None,
                target,
            } => format!("goto {};", label(*target)),
            Stmt::IndirectBranch(value) => format!(
                "goto *{};  // through a value: a switch table, or a function pointer",
                expression(value, 12, locals, naming, registers)
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

fn callee(
    callee: &Callee,
    locals: &HashMap<u32, String>,
    naming: &Naming,
    registers: &Registers,
) -> String {
    match callee {
        Callee::Named(name) => name.clone(),
        Callee::Address(address) => format!("function_{address:x}"),
        Callee::Indirect(value) => {
            format!("({})", expression(value, 0, locals, naming, registers))
        }
    }
}

fn place(
    place: &Place,
    locals: &HashMap<u32, String>,
    naming: &Naming,
    registers: &Registers,
) -> String {
    match place {
        // A register the interface pass named is that name — it is a
        // parameter, and it is declared in the signature. Anything else is a
        // variable of this function's own, and a narrow access to one is a
        // window onto it rather than a name of its own.
        Place::Register(register) => naming
            .name_of(*register)
            .map_or_else(|| read_register(*register, registers), ToOwned::to_owned),
        Place::Condition(condition) => condition.name().to_owned(),
        Place::Local { id, .. } => locals
            .get(id)
            .cloned()
            .unwrap_or_else(|| format!("local_{id}")),
        Place::Memory { address, width } => format!(
            "*({} *)({})",
            width.c_name(false),
            expression(address, 0, locals, naming, registers)
        ),
    }
}

/// An expression, bracketed only where the precedence needs it.
fn expression(
    value: &Expr,
    outer: u8,
    locals: &HashMap<u32, String>,
    naming: &Naming,
    registers: &Registers,
) -> String {
    match value {
        Expr::Const { value, width } => constant(*value, *width),
        Expr::Read(inner) => place(inner, locals, naming, registers),
        Expr::AddressOf(inner) => {
            // In C an array's name already is the address of its first
            // element, and `&` in front of one has a different type from the
            // pointer the code is actually passing.
            if let Place::Local { id, .. } = inner.as_ref()
                && naming.is_buffer(*id)
            {
                place(inner, locals, naming, registers)
            } else {
                format!("&{}", place(inner, locals, naming, registers))
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
            format!(
                "{symbol}{}",
                expression(operand, 11, locals, naming, registers)
            )
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
                    unsigned(left, precedence, locals, naming, registers),
                    unsigned(right, precedence, locals, naming, registers),
                )
            } else {
                (
                    expression(left, precedence, locals, naming, registers),
                    expression(right, precedence + 1, locals, naming, registers),
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
            expression(value, 11, locals, naming, registers)
        ),
        Expr::Call { callee, arguments } => format!(
            "{}({})",
            self::callee(callee, locals, naming, registers),
            arguments
                .iter()
                .map(|argument| expression(argument, 0, locals, naming, registers))
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
                expression(condition, 3, locals, naming, registers),
                expression(when_true, 0, locals, naming, registers),
                expression(when_false, 0, locals, naming, registers)
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
    registers: &Registers,
) -> String {
    let width = value.width().unwrap_or(Width::Qword);
    if matches!(value, Expr::Const { .. }) {
        return expression(value, precedence, locals, naming, registers);
    }
    format!(
        "({}){}",
        width.c_name(false),
        expression(value, 11, locals, naming, registers)
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
        expression(value, 0, &HashMap::new(), &naming(), &Registers::default())
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
        let sum_then_product = Expr::binary(Binary::Multiply, Expr::binary(Binary::Add, a, b), c);
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
