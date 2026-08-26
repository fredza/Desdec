//! Turning registers and offsets into things with names.
//!
//! After lifting, a function is arithmetic on `rdi` and reads through
//! `rbp - 0x18`. Both are facts about the machine; neither is what the source
//! said. This module puts back the three things a calling convention lets one
//! recover — what the function was *given*, what it *keeps*, and what it
//! *answers* — and it does so before the dataflow pass runs, because that
//! order is what makes the result readable.
//!
//! The order matters more than it looks. A call, in the IR the lifter
//! produces, reads nothing: `call printf` is one statement with no operands,
//! because the instruction has none. Run the dataflow pass on that and the
//! assignments that loaded `rdi` and `rsi` are dead stores, correctly deleted,
//! and the output says `printf()` — a call to a variadic function with no
//! arguments, which is worse than useless. So the arguments are recognised
//! *first*, from the registers the convention names, and by the time
//! substitution runs they are ordinary reads inside the call, get the values
//! moved into them like anything else, and come out as
//! `printf("%s: %d\n", name, count)`.
//!
//! # What is recovered, and how sure each part is
//!
//! - **The locals.** Exact. An access at a fixed offset from the frame or
//!   stack pointer is a slot, and two accesses at the same offset are the same
//!   slot. What is *not* claimed is a type: the width comes from how wide the
//!   accesses are, and nothing else.
//! - **The parameters.** A reading, and a good one. A register the convention
//!   passes arguments in, read by the function before it writes it, held a
//!   value the caller put there — there is no other way for it to have one.
//!   The count is the longest run of them, because a convention fills them in
//!   order.
//! - **The arguments of a call.** A reading, and a weaker one: the registers
//!   loaded before the call, in the order the convention names. A function
//!   whose third argument happens to already hold the right value gets one
//!   argument too few. This is the same evidence every decompiler without type
//!   information works from, and it is right far more often than not.
//! - **The return.** A reading. A function that writes the return register on
//!   its way out returns something; one that does not returns `void`.

use std::collections::{BTreeMap, HashMap};

use crate::{
    Architecture, BinaryFormat,
    decompiler::native::ir::{Expr, Place, Register, Statement, Stmt, Width},
};

/// Which convention a file's functions follow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Convention {
    /// System V AMD64: ELF and Mach-O on x86-64.
    SystemV,
    /// The Microsoft x64 convention: PE on x86-64. Four registers rather than
    /// six, and a different four.
    Microsoft,
    /// `AAPCS64`, for completeness: `x0`..`x7`.
    Aapcs64,
    /// Nothing is claimed. A 32-bit x86 file passes its arguments on the
    /// stack, and which of the several conventions it uses is not something
    /// the file states.
    Unstated,
}

impl Convention {
    /// The convention a file of this format and architecture uses.
    #[must_use]
    pub const fn of(format: BinaryFormat, architecture: Architecture) -> Self {
        match (format, architecture) {
            (BinaryFormat::Pe, Architecture::X86_64) => Self::Microsoft,
            (_, Architecture::X86_64) => Self::SystemV,
            (_, Architecture::Arm64) => Self::Aapcs64,
            _ => Self::Unstated,
        }
    }

    /// The registers arguments arrive in, in order.
    #[must_use]
    pub const fn argument_registers(self) -> &'static [&'static str] {
        match self {
            Self::SystemV => &["rdi", "rsi", "rdx", "rcx", "r8", "r9"],
            Self::Microsoft => &["rcx", "rdx", "r8", "r9"],
            Self::Aapcs64 => &["x0", "x1", "x2", "x3", "x4", "x5", "x6", "x7"],
            Self::Unstated => &[],
        }
    }

    /// Where an answer is left.
    #[must_use]
    pub const fn return_register(self) -> Option<&'static str> {
        match self {
            Self::SystemV | Self::Microsoft => Some("rax"),
            Self::Aapcs64 => Some("x0"),
            Self::Unstated => None,
        }
    }

    /// The registers a function must give back as it found them, which are
    /// therefore live when it returns.
    #[must_use]
    pub const fn preserved(self) -> &'static [&'static str] {
        match self {
            Self::SystemV => &["rbx", "rbp", "rsp", "r12", "r13", "r14", "r15"],
            Self::Microsoft => &[
                "rbx", "rbp", "rsp", "rdi", "rsi", "r12", "r13", "r14", "r15",
            ],
            Self::Aapcs64 => &["x19", "x20", "x21", "x22", "x23", "x24", "x25", "x26"],
            Self::Unstated => &[],
        }
    }
}

/// One slot of the frame, with the name it will carry in the output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Local {
    pub id: u32,
    pub name: String,
    pub width: Width,
    /// How the listing writes the address, so a reader can find it there.
    pub label: String,
}

/// One value the function was given.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub root: &'static str,
    pub width: Width,
}

/// Everything recovered about one function's interface and frame.
#[derive(Clone, Debug)]
pub struct Naming {
    pub convention: Convention,
    pub parameters: Vec<Parameter>,
    pub locals: Vec<Local>,
    /// The width of what the function answers, and `None` for `void`.
    pub returns: Option<Width>,
    /// Register root → the name it carries, for the parameters.
    by_register: HashMap<&'static str, String>,
}

impl Naming {
    /// What to call a register in the output, when it is a parameter.
    #[must_use]
    pub fn name_of(&self, register: Register) -> Option<&str> {
        self.by_register.get(register.root).map(String::as_str)
    }

    /// What is live when the function returns.
    ///
    /// The preserved registers, and nothing else. The *answer* is deliberately
    /// not here: [`apply`] has already turned each `return` into one that
    /// carries the value, so the return register is live exactly where it is
    /// read and dead everywhere else. Listing it as escaping as well kept the
    /// `mov -0x4(%rbp),%eax` that a compiler puts in front of a `ret`, so the
    /// output said `eax = found; return found;`.
    #[must_use]
    pub fn escaping(&self) -> Vec<crate::decompiler::native::dataflow::Key> {
        use crate::decompiler::native::dataflow::Key;
        self.convention
            .preserved()
            .iter()
            .map(|root| Key::Register(root))
            .collect()
    }
}

/// Reads a function's interface and frame off its lifted body.
#[must_use]
pub fn recognise(blocks: &[Vec<Statement>], convention: Convention) -> Naming {
    let locals = frame_slots(blocks);
    let parameters = incoming(blocks, convention);
    let returns = answered(blocks, convention);
    let by_register = parameters
        .iter()
        .map(|parameter| (parameter.root, parameter.name.clone()))
        .collect();
    Naming {
        convention,
        parameters,
        locals: locals.into_values().collect(),
        returns,
        by_register,
    }
}

/// Rewrites a body in the names just recovered.
///
/// Three changes, and each is what a later pass needs to have already
/// happened: frame accesses become locals, calls read the arguments the
/// convention says they take, and a return carries the value the function
/// leaves behind.
pub fn apply(naming: &Naming, blocks: &mut [Vec<Statement>]) {
    let slots: HashMap<(String, i64), u32> = naming
        .locals
        .iter()
        .filter_map(|local| Some((split_label(&local.label)?, local.id)))
        .collect();
    let widths: HashMap<u32, Width> = naming
        .locals
        .iter()
        .map(|local| (local.id, local.width))
        .collect();

    for statements in blocks.iter_mut() {
        for statement in statements.iter_mut() {
            rewrite_places(&mut statement.effect, &slots, &widths);
        }
    }
    arguments_of_calls(blocks, naming.convention);
    if let Some(root) = naming
        .returns
        .and_then(|_| naming.convention.return_register())
    {
        let width = naming.returns.unwrap_or(Width::Qword);
        for statements in blocks.iter_mut() {
            for statement in statements.iter_mut() {
                if matches!(statement.effect, Stmt::Return(None)) {
                    statement.effect =
                        Stmt::Return(Some(Expr::register(Register::new(root, width))));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The frame
// ---------------------------------------------------------------------------

/// Every fixed offset from the frame or stack pointer the body touches.
fn frame_slots(blocks: &[Vec<Statement>]) -> BTreeMap<(String, i64), Local> {
    let mut found: BTreeMap<(String, i64), Local> = BTreeMap::new();
    let mut next = 0_u32;
    for statements in blocks {
        for statement in statements {
            for (place, width) in memory_places(&statement.effect) {
                let Some((root, offset)) = frame_offset(&place) else {
                    continue;
                };
                let key = (root.clone(), offset);
                let entry = found.entry(key).or_insert_with(|| {
                    let id = next;
                    next += 1;
                    Local {
                        id,
                        name: slot_name(&root, offset),
                        width,
                        label: label_of(&root, offset),
                    }
                });
                // The widest access is the one that says how big the slot is:
                // a byte read out of a word is a read of part of it.
                if width > entry.width {
                    entry.width = width;
                }
            }
        }
    }
    found
}

/// What to call a slot.
///
/// Below the frame pointer is a local; above it, on a machine whose call
/// pushes the return address, is an argument the caller left on the stack.
/// Above the stack pointer is either, and is named for where it is rather than
/// for a guess about which.
fn slot_name(root: &str, offset: i64) -> String {
    if root == "rbp" || root == "ebp" {
        if offset < 0 {
            return format!("local_{:x}", offset.unsigned_abs());
        }
        return format!("stack_arg_{:x}", offset.unsigned_abs());
    }
    format!("frame_{:x}", offset.unsigned_abs())
}

fn label_of(root: &str, offset: i64) -> String {
    if offset < 0 {
        format!("{root}-{:#x}", offset.unsigned_abs())
    } else {
        format!("{root}+{offset:#x}")
    }
}

fn split_label(label: &str) -> Option<(String, i64)> {
    if let Some((root, rest)) = label.split_once('-') {
        let offset = i64::from_str_radix(rest.trim_start_matches("0x"), 16).ok()?;
        return Some((root.to_owned(), -offset));
    }
    let (root, rest) = label.split_once('+')?;
    let offset = i64::from_str_radix(rest.trim_start_matches("0x"), 16).ok()?;
    Some((root.to_owned(), offset))
}

/// The frame register and offset a memory place names, when it names one.
fn frame_offset(place: &Place) -> Option<(String, i64)> {
    let Place::Memory { address, .. } = place else {
        return None;
    };
    match address.as_ref() {
        Expr::Read(register) => match register.as_ref() {
            Place::Register(register) if is_frame(register.root) => {
                Some((register.root.to_owned(), 0))
            }
            _ => None,
        },
        Expr::Binary {
            operator,
            left,
            right,
        } => {
            let Expr::Read(base) = left.as_ref() else {
                return None;
            };
            let Place::Register(base) = base.as_ref() else {
                return None;
            };
            if !is_frame(base.root) {
                return None;
            }
            let Expr::Const { value, .. } = right.as_ref() else {
                return None;
            };
            let offset = i64::try_from(*value).ok()?;
            match operator {
                crate::decompiler::native::ir::Binary::Add => Some((base.root.to_owned(), offset)),
                crate::decompiler::native::ir::Binary::Subtract => {
                    Some((base.root.to_owned(), -offset))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn is_frame(root: &str) -> bool {
    matches!(root, "rbp" | "ebp" | "rsp" | "esp" | "x29" | "sp")
}

/// Every memory place a statement names, with the width it is accessed at.
fn memory_places(effect: &Stmt) -> Vec<(Place, Width)> {
    let mut found = Vec::new();
    match effect {
        Stmt::Assign { place, value } => {
            if let Place::Memory { width, .. } = place {
                found.push((place.clone(), *width));
            }
            gather_memory(value, &mut found);
        }
        Stmt::Call {
            result, arguments, ..
        } => {
            if let Some(place @ Place::Memory { width, .. }) = result {
                found.push((place.clone(), *width));
            }
            for argument in arguments {
                gather_memory(argument, &mut found);
            }
        }
        Stmt::Return(Some(value)) | Stmt::IndirectBranch(value) => {
            gather_memory(value, &mut found);
        }
        Stmt::Branch {
            condition: Some(condition),
            ..
        } => gather_memory(condition, &mut found),
        _ => {}
    }
    found
}

fn gather_memory(expression: &Expr, into: &mut Vec<(Place, Width)>) {
    match expression {
        Expr::Read(place) | Expr::AddressOf(place) => {
            if let Place::Memory { width, address } = place.as_ref() {
                into.push(((**place).clone(), *width));
                gather_memory(address, into);
            }
        }
        Expr::Unary { operand, .. } => gather_memory(operand, into),
        Expr::Binary { left, right, .. } => {
            gather_memory(left, into);
            gather_memory(right, into);
        }
        Expr::Cast { value, .. } => gather_memory(value, into),
        Expr::Call { arguments, .. } => {
            for argument in arguments {
                gather_memory(argument, into);
            }
        }
        Expr::Select {
            condition,
            when_true,
            when_false,
        } => {
            gather_memory(condition, into);
            gather_memory(when_true, into);
            gather_memory(when_false, into);
        }
        Expr::Const { .. } | Expr::Symbol { .. } | Expr::Unknown(_) => {}
    }
}

/// Replaces recognised frame accesses with the slots they name.
fn rewrite_places(
    effect: &mut Stmt,
    slots: &HashMap<(String, i64), u32>,
    widths: &HashMap<u32, Width>,
) {
    let convert = |place: &mut Place| {
        if let Some(key) = frame_offset(place)
            && let Some(id) = slots.get(&key)
        {
            let width = widths.get(id).copied().unwrap_or_else(|| place.width());
            *place = Place::Local { id: *id, width };
        }
    };
    match effect {
        Stmt::Assign { place, value } => {
            convert(place);
            rewrite_expression(value, slots, widths);
        }
        Stmt::Call {
            result, arguments, ..
        } => {
            if let Some(place) = result {
                convert(place);
            }
            for argument in arguments {
                rewrite_expression(argument, slots, widths);
            }
        }
        Stmt::Return(Some(value)) | Stmt::IndirectBranch(value) => {
            rewrite_expression(value, slots, widths);
        }
        Stmt::Branch {
            condition: Some(condition),
            ..
        } => rewrite_expression(condition, slots, widths),
        _ => {}
    }
}

fn rewrite_expression(
    expression: &mut Expr,
    slots: &HashMap<(String, i64), u32>,
    widths: &HashMap<u32, Width>,
) {
    match expression {
        Expr::Read(place) | Expr::AddressOf(place) => {
            if let Some(key) = frame_offset(place)
                && let Some(id) = slots.get(&key)
            {
                let width = widths.get(id).copied().unwrap_or_else(|| place.width());
                **place = Place::Local { id: *id, width };
                return;
            }
            if let Place::Memory { address, .. } = place.as_mut() {
                rewrite_expression(address, slots, widths);
            }
        }
        Expr::Unary { operand, .. } => rewrite_expression(operand, slots, widths),
        Expr::Binary { left, right, .. } => {
            rewrite_expression(left, slots, widths);
            rewrite_expression(right, slots, widths);
        }
        Expr::Cast { value, .. } => rewrite_expression(value, slots, widths),
        Expr::Call { arguments, .. } => {
            for argument in arguments {
                rewrite_expression(argument, slots, widths);
            }
        }
        Expr::Select {
            condition,
            when_true,
            when_false,
        } => {
            rewrite_expression(condition, slots, widths);
            rewrite_expression(when_true, slots, widths);
            rewrite_expression(when_false, slots, widths);
        }
        Expr::Const { .. } | Expr::Symbol { .. } | Expr::Unknown(_) => {}
    }
}

// ---------------------------------------------------------------------------
// The interface
// ---------------------------------------------------------------------------

/// The parameters, from the argument registers the body reads before writing.
///
/// The count is the longest run rather than the set: a convention fills its
/// registers in order, so a function that reads the first and the third was
/// given three arguments and ignores the second. Declaring two would give it a
/// signature no caller matches.
fn incoming(blocks: &[Vec<Statement>], convention: Convention) -> Vec<Parameter> {
    let registers = convention.argument_registers();
    if registers.is_empty() {
        return Vec::new();
    }
    let mut used = vec![None; registers.len()];
    let mut written: Vec<&'static str> = Vec::new();
    // Address order, which is the executed order while nothing jumps into the
    // middle of it — the same limit `crate::operand` draws for the same
    // reason, and it is exact for a prologue, which is where this looks.
    for statements in blocks {
        for statement in statements {
            for (root, width) in registers_read(&statement.effect) {
                if written.contains(&root) {
                    continue;
                }
                if let Some(position) = registers.iter().position(|name| *name == root)
                    && used[position].is_none()
                {
                    used[position] = Some(width);
                }
            }
            if let Some(Place::Register(register)) = written_place(&statement.effect)
                && register.covers_root()
            {
                written.push(register.root);
            }
        }
    }
    let last = used.iter().rposition(Option::is_some);
    let Some(last) = last else {
        return Vec::new();
    };
    (0..=last)
        .map(|position| Parameter {
            name: format!("argument_{}", position + 1),
            root: registers[position],
            width: used[position].unwrap_or(Width::Qword),
        })
        .collect()
}

/// Whether the function leaves an answer behind, and how wide it is.
///
/// A file without types does not state this, and no amount of reading settles
/// it: the return register is live after the call or it is not, and only the
/// caller knows. What *is* stated is whether the function touches the register
/// at all. A function that never writes it cannot be answering with it, and
/// `void` is then exact. A function that writes it is taken to be answering,
/// which is the reading every decompiler without type information makes —
/// Ghidra spells the same guess `undefined8`.
///
/// The width is the widest write, because a function that writes `eax` on one
/// path and `rax` on another answers with the whole of it.
fn answered(blocks: &[Vec<Statement>], convention: Convention) -> Option<Width> {
    let root = convention.return_register()?;
    let returns = blocks.iter().any(|statements| {
        statements
            .iter()
            .any(|statement| matches!(statement.effect, Stmt::Return(_)))
    });
    if !returns {
        return None;
    }
    // Two answers, and the first one that exists wins. The width written in a
    // block that returns is the width of the answer — a function computing an
    // `int` writes `eax` there whatever it did with `rax` on the way. Only
    // where no returning block writes it at all does the widest write
    // anywhere stand in.
    let mut on_the_way_out: Option<Width> = None;
    let mut last_in_any_block: Option<Width> = None;
    for statements in blocks {
        let leaves = statements
            .iter()
            .any(|statement| matches!(statement.effect, Stmt::Return(_)));
        let mut last_here: Option<Width> = None;
        for statement in statements {
            match &statement.effect {
                Stmt::Assign {
                    place: Place::Register(register),
                    ..
                }
                | Stmt::Call {
                    result: Some(Place::Register(register)),
                    ..
                } if register.root == root => {
                    // The *last* write of the block and not the widest: a
                    // function computing an `int` loads pointers through the
                    // whole of `rax` on the way and then puts its answer in
                    // `eax`, and it is the answer that gives the type.
                    last_here = Some(register.width);
                }
                _ => {}
            }
        }
        if let Some(width) = last_here {
            last_in_any_block = Some(
                last_in_any_block.map_or(width, |widest: Width| widest.max(width)),
            );
            if leaves {
                on_the_way_out = Some(width);
            }
        }
    }
    on_the_way_out.or(last_in_any_block)
}

/// Takes the prologue and the epilogue back out.
///
/// Every compiled function begins by moving the stack pointer and saving the
/// registers it is not allowed to change, and ends by putting them back. None
/// of that was in the source, and all of it survives every later pass: the
/// stack pointer is live because the convention says so, and the saved
/// registers are live because they are restored. Left in, a four-line function
/// decompiles to eleven, seven of which say nothing.
///
/// Three shapes go, and each is safe for a stated reason:
///
/// - **Anything written to the stack pointer.** It has no place in C at all —
///   the frame is addressed by the slots [`recognise`] named, not by an
///   offset from a register — so no output can want it.
/// - **The frame pointer being set from the stack pointer**, which is the
///   second half of every prologue that keeps one.
/// - **A preserved register written to the frame, and read back from it.**
///   The read is removed only where the write was seen first, so a genuine
///   load from a structure into `rbx` is not mistaken for the end of a
///   prologue.
///
/// Run before [`recognise`], so the slots the prologue used never become
/// locals with names in the output.
pub fn strip_frame(blocks: &mut [Vec<Statement>], convention: Convention) {
    let preserved = convention.preserved();
    let mut saved: Vec<&'static str> = Vec::new();
    for statements in blocks.iter_mut() {
        statements.retain(|statement| {
            let Stmt::Assign { place, value } = &statement.effect else {
                return true;
            };
            match (place, value) {
                (Place::Register(register), _) if is_stack_pointer(register.root) => false,
                (Place::Register(frame), Expr::Read(source))
                    if is_frame_pointer(frame.root)
                        && matches!(source.as_ref(), Place::Register(register)
                            if is_stack_pointer(register.root)) =>
                {
                    false
                }
                (Place::Memory { .. }, Expr::Read(source)) => {
                    let Place::Register(register) = source.as_ref() else {
                        return true;
                    };
                    if !preserved.contains(&register.root) || frame_offset(place).is_none() {
                        return true;
                    }
                    saved.push(register.root);
                    false
                }
                (Place::Register(register), Expr::Read(source)) => {
                    if !saved.contains(&register.root) || frame_offset(source).is_none() {
                        return true;
                    }
                    false
                }
                _ => true,
            }
        });
    }
}

fn is_stack_pointer(root: &str) -> bool {
    matches!(root, "rsp" | "esp" | "sp")
}

fn is_frame_pointer(root: &str) -> bool {
    matches!(root, "rbp" | "ebp" | "x29")
}

/// Gives every call the arguments the convention says it takes.
///
/// Counted as the longest run of argument registers written since the last
/// call — which is exactly the sequence a compiler emits to set a call up, and
/// exactly what stops the dataflow pass from deleting those assignments as
/// dead.
///
/// Followed across the whole function in address order, not one block at a
/// time. An optimiser hoists the loads out of the branch: `rdi` is filled in
/// one block and the call is made in the next, and reading each block on its
/// own reported that call as taking no arguments at all — which is not a
/// cautious answer but a wrong one. Address order is the executed order while
/// nothing jumps into the middle of it, the same limit [`crate::operand`] and
/// [`crate::analysis::stack`] draw and for the same reason.
fn arguments_of_calls(blocks: &mut [Vec<Statement>], convention: Convention) {
    let registers = convention.argument_registers();
    if registers.is_empty() {
        return;
    }
    let mut loaded: Vec<Option<Width>> = vec![None; registers.len()];
    for statements in blocks.iter_mut() {
        // By index: the statement at `index` is rewritten in place after the
        // registers before it have been read, which an iterator cannot do.
        #[expect(
            clippy::needless_range_loop,
            reason = "the call is rewritten in place after the loads before it are read"
        )]
        for index in 0..statements.len() {
            match &statements[index].effect {
                Stmt::Assign {
                    place: Place::Register(register),
                    ..
                } => {
                    if let Some(position) =
                        registers.iter().position(|name| *name == register.root)
                    {
                        loaded[position] = Some(register.width);
                    }
                }
                Stmt::Call { .. } => {
                    let count = loaded
                        .iter()
                        .position(Option::is_none)
                        .unwrap_or(registers.len());
                    let arguments: Vec<Expr> = (0..count)
                        .map(|position| {
                            Expr::register(Register::new(
                                registers[position],
                                loaded[position].unwrap_or(Width::Qword),
                            ))
                        })
                        .collect();
                    if let Stmt::Call {
                        arguments: slot, ..
                    } = &mut statements[index].effect
                    {
                        *slot = arguments;
                    }
                    // The convention lets a called function keep none of them,
                    // so what was loaded for this call says nothing about the
                    // next.
                    loaded = vec![None; registers.len()];
                }
                _ => {}
            }
        }
    }
}

/// The registers a statement reads, with the width it reads them at.
fn registers_read(effect: &Stmt) -> Vec<(&'static str, Width)> {
    let mut found = Vec::new();
    let mut collect = |expression: &Expr| gather_registers(expression, &mut found);
    match effect {
        Stmt::Assign { place, value } => {
            collect(value);
            if let Place::Memory { address, .. } = place {
                gather_registers(address, &mut found);
            }
        }
        Stmt::Call { arguments, .. } => {
            for argument in arguments {
                gather_registers(argument, &mut found);
            }
        }
        Stmt::Return(Some(value)) | Stmt::IndirectBranch(value) => collect(value),
        Stmt::Branch {
            condition: Some(condition),
            ..
        } => collect(condition),
        _ => {}
    }
    found
}

fn gather_registers(expression: &Expr, into: &mut Vec<(&'static str, Width)>) {
    match expression {
        Expr::Read(place) | Expr::AddressOf(place) => match place.as_ref() {
            Place::Register(register) => into.push((register.root, register.width)),
            Place::Memory { address, .. } => gather_registers(address, into),
            Place::Condition(_) | Place::Local { .. } => {}
        },
        Expr::Unary { operand, .. } => gather_registers(operand, into),
        Expr::Binary { left, right, .. } => {
            gather_registers(left, into);
            gather_registers(right, into);
        }
        Expr::Cast { value, .. } => gather_registers(value, into),
        Expr::Call { arguments, .. } => {
            for argument in arguments {
                gather_registers(argument, into);
            }
        }
        Expr::Select {
            condition,
            when_true,
            when_false,
        } => {
            gather_registers(condition, into);
            gather_registers(when_true, into);
            gather_registers(when_false, into);
        }
        Expr::Const { .. } | Expr::Symbol { .. } | Expr::Unknown(_) => {}
    }
}

const fn written_place(effect: &Stmt) -> Option<&Place> {
    match effect {
        Stmt::Assign { place, .. }
        | Stmt::Call {
            result: Some(place),
            ..
        } => Some(place),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decompiler::native::ir::{Binary, Callee};

    fn register(root: &'static str, width: Width) -> Place {
        Place::Register(Register::new(root, width))
    }

    fn assign(place: Place, value: Expr) -> Statement {
        Statement::new(0x10, Stmt::Assign { place, value })
    }

    fn frame(offset: i64) -> Place {
        Place::Memory {
            address: Box::new(Expr::binary(
                Binary::Subtract,
                Expr::register(Register::new("rbp", Width::Qword)),
                Expr::constant(offset.unsigned_abs(), Width::Qword),
            )),
            width: Width::Dword,
        }
    }

    /// A PE on x86-64 does not pass its first argument in `rdi`, and reading
    /// it as though it did would name every argument of every Windows binary
    /// wrongly.
    #[test]
    fn the_convention_follows_the_format_and_not_only_the_architecture() {
        assert_eq!(
            Convention::of(BinaryFormat::Pe, Architecture::X86_64),
            Convention::Microsoft
        );
        assert_eq!(
            Convention::of(
                BinaryFormat::Elf {
                    bits: 64,
                    endianness: crate::Endianness::Little,
                },
                Architecture::X86_64
            ),
            Convention::SystemV
        );
        assert_eq!(
            Convention::Microsoft.argument_registers()[0],
            "rcx",
            "the Microsoft convention starts at rcx"
        );
    }

    /// A register the function reads before writing held something the caller
    /// put there. There is no other way for it to have a value.
    #[test]
    fn a_register_read_before_it_is_written_is_a_parameter() {
        let blocks = vec![vec![assign(
            frame(-0x18),
            Expr::register(Register::new("rdi", Width::Qword)),
        )]];
        let naming = recognise(&blocks, Convention::SystemV);
        assert_eq!(naming.parameters.len(), 1);
        assert_eq!(naming.parameters[0].root, "rdi");
    }

    /// A convention fills its registers in order, so a function reading the
    /// first and the third was given three.
    #[test]
    fn the_count_is_the_run_and_not_the_set() {
        let blocks = vec![vec![
            assign(frame(-0x8), Expr::register(Register::new("rdi", Width::Qword))),
            assign(
                frame(-0x10),
                Expr::register(Register::new("rdx", Width::Qword)),
            ),
        ]];
        let naming = recognise(&blocks, Convention::SystemV);
        assert_eq!(naming.parameters.len(), 3, "rsi is passed and ignored");
        assert_eq!(naming.parameters[1].root, "rsi");
    }

    #[test]
    fn two_accesses_at_one_offset_are_one_slot() {
        let blocks = vec![vec![
            assign(frame(-0x18), Expr::constant(1, Width::Dword)),
            assign(
                register("rax", Width::Dword),
                Expr::read(frame(-0x18)),
            ),
        ]];
        let naming = recognise(&blocks, Convention::SystemV);
        assert_eq!(naming.locals.len(), 1);
        assert_eq!(naming.locals[0].name, "local_18");
        assert_eq!(naming.locals[0].label, "rbp-0x18");
    }

    #[test]
    fn a_frame_access_becomes_the_slot_it_names() {
        let mut blocks = vec![vec![assign(frame(-0x18), Expr::constant(1, Width::Dword))]];
        let naming = recognise(&blocks, Convention::SystemV);
        apply(&naming, &mut blocks);
        let Stmt::Assign { place, .. } = &blocks[0][0].effect else {
            panic!("the assignment survives");
        };
        assert!(matches!(place, Place::Local { .. }), "got {place:?}");
    }

    /// The reason this module runs before the dataflow pass: a call reads
    /// nothing in the lifted IR, so the loads that set it up look dead.
    #[test]
    fn a_call_is_given_the_arguments_the_convention_names() {
        let mut blocks = vec![vec![
            assign(
                register("rdi", Width::Qword),
                Expr::Symbol {
                    name: "message".to_owned(),
                    address: 0x2000,
                },
            ),
            assign(register("rsi", Width::Dword), Expr::constant(4, Width::Dword)),
            Statement::new(
                0x20,
                Stmt::Call {
                    result: Some(register("rax", Width::Qword)),
                    callee: Callee::Named("printf".to_owned()),
                    arguments: Vec::new(),
                },
            ),
        ]];
        let naming = recognise(&blocks, Convention::SystemV);
        apply(&naming, &mut blocks);
        let Stmt::Call { arguments, .. } = &blocks[0][2].effect else {
            panic!("the call survives");
        };
        assert_eq!(arguments.len(), 2, "two registers were loaded for it");
        assert_eq!(
            arguments[0],
            Expr::register(Register::new("rdi", Width::Qword))
        );
    }

    /// A function that never touches the return register returns `void`, and
    /// saying so is worth more than an `int64_t` nobody should read.
    #[test]
    fn a_function_that_leaves_nothing_behind_returns_void() {
        let blocks = vec![vec![Statement::new(0x10, Stmt::Return(None))]];
        let naming = recognise(&blocks, Convention::SystemV);
        assert_eq!(naming.returns, None);
    }

    #[test]
    fn a_function_that_writes_the_answer_register_returns_it() {
        let blocks = vec![vec![
            assign(register("rax", Width::Dword), Expr::constant(0, Width::Dword)),
            Statement::new(0x14, Stmt::Return(None)),
        ]];
        let mut blocks = blocks;
        let naming = recognise(&blocks, Convention::SystemV);
        assert_eq!(naming.returns, Some(Width::Dword));
        apply(&naming, &mut blocks);
        assert!(
            matches!(blocks[0][1].effect, Stmt::Return(Some(_))),
            "the return should carry the value"
        );
    }
}
