//! Making the listing shorter without making it say anything else.
//!
//! Lifting one instruction at a time produces something correct and unreadable.
//! Four instructions —
//!
//! ```text
//! mov -0x18(%rbp),%rax
//! mov 0x8(%rax),%rdx
//! mov -0x20(%rbp),%rax
//! cmp %rax,%rdx
//! ```
//!
//! — become, statement for statement, four assignments and a comparison
//! between two registers, which tells a reader nothing they could not read in
//! the listing itself. What they are is one question: is `local_18->field_8`
//! above `local_20`? Getting from the first to the second is this module's
//! entire job, and it is done by two passes that are old, small and safe.
//!
//! **Substitution.** A value assigned to a place and then read back is
//! replaced by the value itself. Safe exactly when nothing between the write
//! and the read can have changed the answer — which is what
//! [`Expr::depends_on`] and [`Place::may_clobber`] decide, and they decide it
//! pessimistically: any write to memory is assumed to reach any other, and
//! nothing at all crosses an instruction the lifter did not model.
//!
//! **Dead-store elimination.** An assignment whose place is never read again
//! goes. This is what removes the flags — a `cmp` settles eight questions and
//! the branch below it asks one — and the intermediate registers left behind
//! once substitution has moved their values to where they were used.
//!
//! # Where this stops, and why it stops there
//!
//! Substitution runs **within a basic block** and does not cross a branch.
//! That is the same limit [`crate::analysis::stack`] and [`crate::operand`]
//! draw, for the same reason: a value carried across a branch is only the
//! value that arrives if the branch was the one taken, and a decompiler that
//! assumes otherwise writes confident C about a path that may never run. What
//! *does* cross a branch is liveness, which is computed over the whole
//! function — because deleting an assignment requires knowing that no *other*
//! block reads it, and being wrong in that direction deletes real code.
//!
//! The practical effect is that the C is at its cleanest inside a block and
//! keeps a named register where a value genuinely arrives from two directions.
//! A reader seeing `rax` in the output is being told something true: the value
//! there depends on how the function got there.

use std::collections::HashSet;

use crate::decompiler::native::ir::{
    Binary, Condition, Expr, Place, Statement, Stmt, Unary, Width,
};

/// A place liveness can follow exactly.
///
/// Memory is not here on purpose: deciding that two addresses are different is
/// the aliasing problem, so a store is never treated as dead. Registers and
/// conditions are exact, and they are where all the noise is.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Key {
    Register(&'static str),
    Condition(Condition),
}

impl Key {
    fn of(place: &Place) -> Option<Self> {
        match place {
            Place::Register(register) => Some(Self::Register(register.root)),
            Place::Condition(condition) => Some(Self::Condition(*condition)),
            Place::Memory { .. } | Place::Local { .. } => None,
        }
    }
}

/// How large a value may be and still be substituted into more than one place.
///
/// Past this, moving it means writing it twice, and two copies of a five-term
/// expression are harder to read than the temporary they came from — which is
/// the same reason a person writing C would have introduced the temporary.
const REPEATABLE: usize = 3;

/// Simplifies every block of a function in place.
///
/// `successors` gives, per block, the indices of the blocks the flow can reach
/// from it; `escaping` is what is still live when the function returns — the
/// return register and whatever the ABI says the caller may read.
pub fn simplify(blocks: &mut [Vec<Statement>], successors: &[Vec<usize>], escaping: &[Key]) {
    // Dead stores first, then substitution, then dead stores again — and the
    // first pass is not an optimisation but a correction. A `cmp` settles
    // eight questions, so every arithmetic instruction is followed by eight
    // statements reading the register it wrote. Substitution counts those
    // reads when it decides whether moving a value is worth it, concludes that
    // a register read nine times should stay where it is, and leaves the
    // scratch register in the output. Removing the questions nothing asks
    // before counting anything is what makes three lines into one.
    for round in 0..2 {
        let live_out = liveness(blocks, successors, escaping);
        for (index, statements) in blocks.iter_mut().enumerate() {
            if round == 1 {
                substitute_within(statements);
            }
            let live = live_out.get(index).cloned().unwrap_or_default();
            remove_dead_stores(statements, &live);
        }
    }
}

// ---------------------------------------------------------------------------
// Liveness
// ---------------------------------------------------------------------------

/// What is live on the way out of each block.
///
/// The ordinary backwards fixed point: a place is live at the end of a block
/// if some block it can reach reads it before writing it. Iterated until
/// nothing changes, which for a function-sized graph is a handful of rounds.
fn liveness(
    blocks: &[Vec<Statement>],
    successors: &[Vec<usize>],
    escaping: &[Key],
) -> Vec<HashSet<Key>> {
    let count = blocks.len();
    let mut reads = Vec::with_capacity(count);
    let mut writes = Vec::with_capacity(count);
    for statements in blocks {
        let (block_reads, block_writes) = read_before_written(statements);
        reads.push(block_reads);
        writes.push(block_writes);
    }

    let escaping: HashSet<Key> = escaping.iter().copied().collect();
    let mut live_out: Vec<HashSet<Key>> = vec![HashSet::new(); count];
    let mut changed = true;
    // A bound as well as a fixed point: an irreducible graph is rare but a
    // decompiler that can be made to spin on one is worse than one that stops
    // slightly early with a conservative answer.
    let mut rounds = 0;
    while changed && rounds < 64 {
        changed = false;
        rounds += 1;
        for index in (0..count).rev() {
            let mut out = HashSet::new();
            if successors[index].is_empty() {
                // Nowhere further inside this function: what the caller may
                // read is live, and nothing else is.
                out.extend(escaping.iter().copied());
            }
            for successor in &successors[index] {
                // Live into a successor is live out of here: what it reads
                // before writing, plus what is live past it.
                out.extend(reads[*successor].iter().copied());
                for key in &live_out[*successor] {
                    if !writes[*successor].contains(key) {
                        out.insert(*key);
                    }
                }
            }
            if out != live_out[index] {
                live_out[index] = out;
                changed = true;
            }
        }
    }
    live_out
}

/// What a block reads before writing it, and what it writes at all.
fn read_before_written(statements: &[Statement]) -> (HashSet<Key>, HashSet<Key>) {
    let mut reads = HashSet::new();
    let mut writes = HashSet::new();
    for statement in statements {
        for place in places_read(&statement.effect) {
            if let Some(key) = Key::of(&place)
                && !writes.contains(&key)
            {
                reads.insert(key);
            }
        }
        // Only a write that covers the whole register kills what was there. A
        // byte written into `%al` leaves the other seven alive, and treating
        // it as a kill would delete the assignment that filled them.
        if let Some(place) = place_written(&statement.effect)
            && let Some(key) = Key::of(place)
            && covers(place)
        {
            writes.insert(key);
        }
    }
    (reads, writes)
}

const fn covers(place: &Place) -> bool {
    match place {
        Place::Register(register) => register.covers_root(),
        Place::Condition(_) => true,
        Place::Memory { .. } | Place::Local { .. } => false,
    }
}

// ---------------------------------------------------------------------------
// Substitution
// ---------------------------------------------------------------------------

/// Moves each value to where it is read, inside one block.
///
/// Walked by index rather than by iterator so that, at each definition, the
/// statements *below* it can be counted: a value read once should be moved to
/// where it is read however large it is, and one read three times should be
/// moved only if writing it three times is an improvement. Counting reads
/// across the whole block instead — which is what this did first — refuses to
/// move a value past the second read of a register that was written again in
/// between, and leaves the scratch register in the output.
fn substitute_within(statements: &mut Vec<Statement>) {
    // What is currently known about each place, with how many times the block
    // still reads it, and — for the answer of a call — which statement the
    // value came from. A definition leaves the table the moment anything could
    // have changed it or what it was computed from.
    let mut known: Vec<Definition> = Vec::new();
    let mut consumed: Vec<usize> = Vec::new();
    for index in 0..statements.len() {
        // Reads first: an assignment's own value is read before its place is
        // written, and `rsp = rsp - 8` must see the old `rsp`.
        let mut used: Vec<usize> = Vec::new();
        for_each_read_expression(&mut statements[index].effect, &mut |expression| {
            for definition in &known {
                if (definition.remaining <= 1 || definition.value.complexity() <= REPEATABLE)
                    && replace_reads(expression, &definition.place, &definition.value)
                    && let Some(from) = definition.from
                {
                    used.push(from);
                }
            }
            fold(expression);
        });
        consumed.extend(used);

        // Then the write, which invalidates whatever it could have changed.
        match &statements[index].effect {
            Stmt::Assign { place, value } => {
                let (place, value) = (place.clone(), value.clone());
                known.retain(|definition| {
                    !place.may_clobber(&definition.place) && !definition.value.depends_on(&place)
                });
                // A local is not propagated. It has a name, and the name is
                // what the reader wants: `if (letter <= 90)` says what the
                // program is doing and `if (al <= 90)` says what the machine
                // is. Registers carry no such information and are moved
                // wherever they are read.
                if !matches!(place, Place::Local { .. }) {
                    let remaining = reads_of(&statements[index + 1..], &place);
                    // A definition that reads the place it defines —
                    // `r8 = r8 + 1`, which is every counter there is — cannot
                    // be moved into a read below it. The assignment *stays*:
                    // the register is live further on, nothing removes the
                    // line, and by the time the substituted text is reached
                    // `r8` already holds the new value. `r8 = r8 + 1;`
                    // followed by `if (r8 + 1 == 7)` tests the wrong number
                    // and reads exactly like code that tests the right one,
                    // which is the one kind of wrong output this decompiler
                    // must never produce. The value of a call is the
                    // exception below, because there the statement goes with
                    // it and nothing is left behind to have changed.
                    let reads_itself = value.depends_on(&place);
                    // A value that has effects of its own — one holding a call
                    // — moves only where it is read exactly once, and the
                    // statement it came from is removed with it. Written out
                    // twice it would happen twice, which is not a tidier
                    // program but a different one. This is what turns
                    // `ZF = strcmp(a, b) == 0;` followed by `if (!ZF)` into
                    // `if (strcmp(a, b) != 0)`.
                    let moves = if value.has_effects() {
                        remaining == 1
                    } else {
                        !reads_itself
                    };
                    if moves {
                        let from = value.has_effects().then_some(index);
                        known.push(Definition {
                            place,
                            value,
                            remaining,
                            from,
                        });
                    }
                }
            }
            // A call runs code this file does not state the effects of, and an
            // unmodelled instruction is unbounded by definition. Everything
            // known is forgotten at both, which is what stops a value being
            // carried across a `printf`.
            Stmt::Call {
                result: Some(place),
                callee,
                arguments,
            } => {
                let (place, callee, arguments) =
                    (place.clone(), callee.clone(), arguments.clone());
                known.clear();
                // The answer of a call moves to where it is read, and the call
                // moves with it: `rax = strlen(s); n = rax;` is two lines
                // saying one thing. Only ever where it is read **exactly
                // once** — a call written out twice would be made twice, which
                // is not a tidier program but a different one. Any write at
                // all clears the table, so nothing can be reordered past a
                // store either.
                let remaining = reads_of(&statements[index + 1..], &place);
                if remaining == 1 {
                    known.push(Definition {
                        place,
                        value: Expr::Call { callee, arguments },
                        remaining,
                        from: Some(index),
                    });
                }
            }
            Stmt::Call { .. } | Stmt::SystemCall { .. } | Stmt::Opaque(_) => known.clear(),
            Stmt::Branch { .. }
            | Stmt::IndirectBranch(_)
            | Stmt::Return(_)
            | Stmt::Trap
            | Stmt::Nothing => {}
        }
    }
    // The calls whose answer was moved into the line that reads it. Removed
    // last, so the indices gathered along the way still name what they named.
    consumed.sort_unstable();
    consumed.dedup();
    let mut index = 0;
    statements.retain(|_| {
        let kept = !consumed.contains(&index);
        index += 1;
        kept
    });
}

/// A value known to be in a place, as the block is walked forwards.
struct Definition {
    place: Place,
    value: Expr,
    /// How many times the rest of the block reads the place.
    remaining: usize,
    /// The statement the value came from, for a call — which is removed when
    /// its answer is moved into the line that reads it.
    from: Option<usize>,
}

/// How many times a place is read in what is left of a block.
///
/// A narrower window onto the same register counts as a read of it, because
/// [`replace_reads`] will substitute into one. Counting only exact matches
/// made `rax = strlen(s)` followed by `mov %eax,%edx` look like a call whose
/// answer nothing reads.
fn reads_of(statements: &[Statement], place: &Place) -> usize {
    let mut found = 0;
    for statement in statements {
        found += places_read(&statement.effect)
            .iter()
            .filter(|read| *read == place || narrowed(place, read).is_some())
            .count();
        // Past a write that covers it, the reads are of a different value and
        // are none of this definition's business. Counting to the end of the
        // block instead made a value read once look like a value read three
        // times, and refused to move it.
        if let Some(written) = place_written(&statement.effect)
            && written.may_clobber(place)
            && covers(written)
        {
            break;
        }
    }
    found
}

/// Replaces every read of `target` inside `expression` with `value`.
///
/// A narrower read of the same register counts. A compiler writes `%eax` and
/// reads `%al` constantly — `movzbl (%rax),%eax` then `mov %al,-0x9(%rbp)` is
/// one load of one byte written as two instructions — and refusing the match
/// leaves both of them in the output. Reading the low byte of what was just
/// written is a truncation and nothing else, so the value is substituted
/// inside a cast that says so.
fn replace_reads(expression: &mut Expr, target: &Place, value: &Expr) -> bool {
    if let Expr::Read(place) = expression {
        if place.as_ref() == target {
            *expression = value.clone();
            return true;
        }
        if let Some(width) = narrowed(target, place) {
            *expression = Expr::Cast {
                value: Box::new(value.clone()),
                width,
                signed: false,
            };
            return true;
        }
    }
    let mut replaced = false;
    for child in children_mut(expression) {
        replaced |= replace_reads(child, target, value);
    }
    replaced
}

/// The width at which `read` addresses part of what `target` wrote, when it
/// does. `%ah` is refused: it addresses the second byte, which no truncation
/// expresses.
fn narrowed(target: &Place, read: &Place) -> Option<Width> {
    let (Place::Register(written), Place::Register(read)) = (target, read) else {
        return None;
    };
    if written.root != read.root
        || written.high_byte
        || read.high_byte
        || read.width >= written.width
    {
        return None;
    }
    Some(read.width)
}

/// Every expression a statement reads, so a caller can rewrite them all.
fn for_each_read_expression(effect: &mut Stmt, act: &mut impl FnMut(&mut Expr)) {
    match effect {
        Stmt::Assign { place, value } => {
            act(value);
            // The address of a store is read even though the location is
            // written: `*(p + 8) = x` reads `p`.
            if let Place::Memory { address, .. } = place {
                act(address);
            }
        }
        Stmt::Call {
            result, arguments, ..
        } => {
            for argument in arguments.iter_mut() {
                act(argument);
            }
            if let Some(Place::Memory { address, .. }) = result {
                act(address);
            }
        }
        Stmt::Return(Some(value)) | Stmt::IndirectBranch(value) => act(value),
        Stmt::Branch {
            condition: Some(condition),
            ..
        } => act(condition),
        Stmt::Return(None)
        | Stmt::Branch {
            condition: None, ..
        }
        | Stmt::Opaque(_)
        | Stmt::SystemCall { .. }
        | Stmt::Trap
        | Stmt::Nothing => {}
    }
}

/// The sub-expressions of an expression, for a rewrite to descend into.
fn children_mut(expression: &mut Expr) -> Vec<&mut Expr> {
    match expression {
        Expr::Const { .. } | Expr::Symbol { .. } | Expr::Unknown(_) => Vec::new(),
        Expr::Read(place) | Expr::AddressOf(place) => match place.as_mut() {
            Place::Memory { address, .. } => vec![address.as_mut()],
            Place::Register(_) | Place::Condition(_) | Place::Local { .. } => Vec::new(),
        },
        Expr::Unary { operand, .. } => vec![operand.as_mut()],
        Expr::Binary { left, right, .. } => vec![left.as_mut(), right.as_mut()],
        Expr::Cast { value, .. } => vec![value.as_mut()],
        Expr::Call { arguments, .. } => arguments.iter_mut().collect(),
        Expr::Select {
            condition,
            when_true,
            when_false,
        } => vec![
            condition.as_mut(),
            when_true.as_mut(),
            when_false.as_mut(),
        ],
    }
}

// ---------------------------------------------------------------------------
// Folding
// ---------------------------------------------------------------------------

/// The arithmetic a substitution leaves behind.
///
/// Substituting `rsp = rsp - 8` into `*(rsp)` gives `*(rsp - 8)`, and a second
/// one gives `*(rsp - 8 + 8)`. None of that was in the program; it is an
/// artefact of writing one instruction as two statements, and folding it away
/// is what turns a stack access back into the slot it names.
fn fold(expression: &mut Expr) {
    for child in children_mut(expression) {
        fold(child);
    }
    match expression {
        // A conversion to the width the value already has says nothing, and
        // two stacked conversions say only the narrower one. Substituting a
        // narrow read of a wide write produces both.
        Expr::Cast { value, width, .. } => {
            if value.width() == Some(*width) {
                *expression = (**value).clone();
                return;
            }
            if let Expr::Cast {
                value: inner,
                width: inner_width,
                ..
            } = value.as_ref()
                && *inner_width >= *width
            {
                let (inner, width) = (inner.clone(), *width);
                *expression = Expr::Cast {
                    value: inner,
                    width,
                    signed: false,
                };
                // What is left may itself say nothing — the inner value often
                // already has the width the outer conversion asks for.
                fold(expression);
            }
        }
        // `!(a == b)` is `a != b`, which is what a reader wants to see where a
        // compiler emitted `jne` over the body it meant to skip.
        Expr::Unary {
            operator: Unary::LogicalNot,
            operand,
        } => {
            if let Expr::Binary {
                operator,
                left,
                right,
            } = operand.as_ref()
                && let Some(opposite) = operator.negated()
            {
                *expression = Expr::binary(opposite, (**left).clone(), (**right).clone());
            }
        }
        Expr::Binary {
            operator,
            left,
            right,
        } => {
            if let (
                Expr::Const {
                    value: first,
                    width,
                },
                Expr::Const { value: second, .. },
            ) = (left.as_ref(), right.as_ref())
                && let Some(folded) = evaluate(*operator, *first, *second)
            {
                *expression = Expr::constant(folded, *width);
                return;
            }
            // `x + 0`, `x - 0`, `x * 1`: nothing, written down.
            if let Expr::Const { value: 0, .. } = right.as_ref()
                && matches!(
                    operator,
                    Binary::Add
                        | Binary::Subtract
                        | Binary::Or
                        | Binary::Xor
                        | Binary::ShiftLeft
                        | Binary::ShiftRight
                )
            {
                *expression = (**left).clone();
                return;
            }
            // `(a + 8) - 8`, which is what the stack pointer moving out and
            // back leaves behind.
            if let Expr::Binary {
                operator: inner,
                left: base,
                right: first,
            } = left.as_ref()
                && let (Expr::Const { value: first, .. }, Expr::Const { value: second, .. }) =
                    (first.as_ref(), right.as_ref())
                && let Some(combined) = combine(*inner, *first, *operator, *second)
            {
                *expression = match combined {
                    0 => (**base).clone(),
                    #[expect(
                        clippy::cast_sign_loss,
                        reason = "the displacement is rebuilt as the two's complement it came from"
                    )]
                    difference if difference > 0 => Expr::binary(
                        Binary::Add,
                        (**base).clone(),
                        Expr::constant(difference as u64, Width::Qword),
                    ),
                    difference => Expr::binary(
                        Binary::Subtract,
                        (**base).clone(),
                        Expr::constant(difference.unsigned_abs(), Width::Qword),
                    ),
                };
            }
        }
        _ => {}
    }
}

/// Two constants under an operator, when the answer is exact.
fn evaluate(operator: Binary, left: u64, right: u64) -> Option<u64> {
    Some(match operator {
        Binary::Add => left.wrapping_add(right),
        Binary::Subtract => left.wrapping_sub(right),
        Binary::Multiply => left.wrapping_mul(right),
        Binary::And => left & right,
        Binary::Or => left | right,
        Binary::Xor => left ^ right,
        Binary::ShiftLeft if right < 64 => left << right,
        Binary::ShiftRight if right < 64 => left >> right,
        _ => return None,
    })
}

/// The single displacement two chained additions or subtractions come to.
fn combine(first: Binary, left: u64, second: Binary, right: u64) -> Option<i64> {
    let signed = |operator: Binary, value: u64| -> Option<i64> {
        let value = i64::try_from(value).ok()?;
        match operator {
            Binary::Add => Some(value),
            Binary::Subtract => Some(-value),
            _ => None,
        }
    };
    Some(signed(first, left)? + signed(second, right)?)
}

// ---------------------------------------------------------------------------
// Dead stores
// ---------------------------------------------------------------------------

/// Removes assignments nothing reads.
///
/// Walked backwards, so what is live is known before each statement is judged.
/// A store to memory is never removed, and neither is anything whose value has
/// effects of its own — deleting a call because its result is unused would
/// delete the call.
fn remove_dead_stores(statements: &mut Vec<Statement>, live_at_exit: &HashSet<Key>) {
    let mut live = live_at_exit.clone();
    let mut keep = vec![true; statements.len()];
    for (index, statement) in statements.iter().enumerate().rev() {
        if let Stmt::Assign { place, value } = &statement.effect
            && let Some(key) = Key::of(place)
            // A condition is a flag of the processor and writing one has no
            // effect a program can observe, so a dead one goes even when what
            // settled it was not modelled — which is the case for `OF` and
            // `PF` after most arithmetic, and is where `OF = OF;` came from.
            && (!value.has_effects() || matches!(place, Place::Condition(_)))
        {
            let read_later = live.contains(&key);
            if !read_later && covers(place) {
                keep[index] = false;
                continue;
            }
            if covers(place) {
                live.remove(&key);
            }
        }
        for place in places_read(&statement.effect) {
            if let Some(key) = Key::of(&place) {
                live.insert(key);
            }
        }
    }
    let mut index = 0;
    statements.retain(|_| {
        let kept = keep[index];
        index += 1;
        kept
    });
    // An instruction whose every effect was removed still has an address, and
    // the view maps lines to addresses. Nothing is put back for it: the row is
    // simply not there, which is the honest thing — the instruction did
    // nothing this function's result depends on.
}

// ---------------------------------------------------------------------------
// Walking
// ---------------------------------------------------------------------------

/// Every place a statement reads.
fn places_read(effect: &Stmt) -> Vec<Place> {
    let mut found = Vec::new();
    let mut collect = |expression: &Expr| gather_reads(expression, &mut found);
    match effect {
        Stmt::Assign { place, value } => {
            collect(value);
            if let Place::Memory { address, .. } = place {
                gather_reads(address, &mut found);
            }
            // A narrow write reads the register it does not cover: `%al = 1`
            // keeps the other fifty-six bits, so the old value is still needed.
            if !covers(place)
                && let Place::Register(_) = place
            {
                found.push(place.clone());
            }
        }
        Stmt::Call {
            result, arguments, ..
        } => {
            for argument in arguments {
                gather_reads(argument, &mut found);
            }
            if let Some(Place::Memory { address, .. }) = result {
                gather_reads(address, &mut found);
            }
        }
        Stmt::Return(Some(value)) | Stmt::IndirectBranch(value) => collect(value),
        Stmt::Branch {
            condition: Some(condition),
            ..
        } => collect(condition),
        Stmt::Return(None)
        | Stmt::Branch {
            condition: None, ..
        }
        | Stmt::Opaque(_)
        | Stmt::SystemCall { .. }
        | Stmt::Trap
        | Stmt::Nothing => {}
    }
    found
}

fn gather_reads(expression: &Expr, into: &mut Vec<Place>) {
    if let Expr::Read(place) = expression {
        into.push((**place).clone());
    }
    match expression {
        Expr::Read(place) | Expr::AddressOf(place) => {
            if let Place::Memory { address, .. } = place.as_ref() {
                gather_reads(address, into);
            }
        }
        Expr::Unary { operand, .. } => gather_reads(operand, into),
        Expr::Binary { left, right, .. } => {
            gather_reads(left, into);
            gather_reads(right, into);
        }
        Expr::Cast { value, .. } => gather_reads(value, into),
        Expr::Call { arguments, .. } => {
            for argument in arguments {
                gather_reads(argument, into);
            }
        }
        Expr::Select {
            condition,
            when_true,
            when_false,
        } => {
            gather_reads(condition, into);
            gather_reads(when_true, into);
            gather_reads(when_false, into);
        }
        Expr::Const { .. } | Expr::Symbol { .. } | Expr::Unknown(_) => {}
    }
}

/// The place a statement writes, when it writes one.
const fn place_written(effect: &Stmt) -> Option<&Place> {
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
    use crate::decompiler::native::ir::Register;

    fn register(root: &'static str, width: Width) -> Place {
        Place::Register(Register::new(root, width))
    }

    fn assign(address: u64, place: Place, value: Expr) -> Statement {
        Statement::new(address, Stmt::Assign { place, value })
    }

    /// The whole point: a value written to a register and read back once
    /// should end up where it was read, and the register should go.
    #[test]
    fn a_value_moves_to_where_it_is_read() {
        let mut blocks = vec![vec![
            assign(
                0x10,
                register("rax", Width::Qword),
                Expr::constant(4, Width::Qword),
            ),
            assign(
                0x14,
                register("rbx", Width::Qword),
                Expr::read(register("rax", Width::Qword)),
            ),
        ]];
        simplify(&mut blocks, &[vec![]], &[Key::Register("rbx")]);
        assert_eq!(blocks[0].len(), 1, "the temporary should be gone");
        let Stmt::Assign { place, value } = &blocks[0][0].effect else {
            panic!("what is left is the assignment that matters");
        };
        assert_eq!(*place, register("rbx", Width::Qword));
        assert_eq!(*value, Expr::constant(4, Width::Qword));
    }

    /// A `cmp` settles eight questions and the branch below it asks one. The
    /// other seven are the single largest source of noise in a naive lifting.
    #[test]
    fn the_questions_nothing_asks_are_removed() {
        let mut statements = vec![
            assign(
                0x10,
                Place::Condition(Condition::Zero),
                Expr::binary(
                    Binary::Equal,
                    Expr::read(register("rax", Width::Qword)),
                    Expr::constant(0, Width::Qword),
                ),
            ),
            assign(
                0x10,
                Place::Condition(Condition::Carry),
                Expr::binary(
                    Binary::Below,
                    Expr::read(register("rax", Width::Qword)),
                    Expr::constant(0, Width::Qword),
                ),
            ),
            Statement::new(
                0x14,
                Stmt::Branch {
                    condition: Some(Expr::read(Place::Condition(Condition::Zero))),
                    target: 0x40,
                },
            ),
        ];
        let mut blocks = vec![std::mem::take(&mut statements)];
        simplify(&mut blocks, &[vec![]], &[]);
        assert_eq!(blocks[0].len(), 1, "only the branch should remain");
        let Stmt::Branch {
            condition: Some(condition),
            ..
        } = &blocks[0][0].effect
        else {
            panic!("the branch survives");
        };
        // And it now asks the question in the program's own terms.
        assert_eq!(
            *condition,
            Expr::binary(
                Binary::Equal,
                Expr::read(register("rax", Width::Qword)),
                Expr::constant(0, Width::Qword)
            )
        );
    }

    /// A value must not be carried across a call: the called function may
    /// write the register, and this file does not state that it does not.
    #[test]
    fn nothing_is_carried_across_a_call() {
        let mut blocks = vec![vec![
            assign(
                0x10,
                register("rax", Width::Qword),
                Expr::constant(4, Width::Qword),
            ),
            Statement::new(
                0x14,
                Stmt::Call {
                    result: None,
                    callee: crate::decompiler::native::ir::Callee::Address(0x2000),
                    arguments: Vec::new(),
                },
            ),
            assign(
                0x19,
                register("rbx", Width::Qword),
                Expr::read(register("rax", Width::Qword)),
            ),
        ]];
        simplify(&mut blocks, &[vec![]], &[Key::Register("rbx")]);
        let Stmt::Assign { value, .. } = &blocks[0][2].effect else {
            panic!("the last assignment survives");
        };
        assert_eq!(
            *value,
            Expr::read(register("rax", Width::Qword)),
            "the constant must not have crossed the call"
        );
    }

    /// A register another block reads is not dead, however unread it looks
    /// from inside this one.
    #[test]
    fn liveness_crosses_a_branch_even_though_substitution_does_not() {
        let mut blocks = vec![
            vec![assign(
                0x10,
                register("rax", Width::Qword),
                Expr::constant(7, Width::Qword),
            )],
            vec![assign(
                0x20,
                register("rbx", Width::Qword),
                Expr::read(register("rax", Width::Qword)),
            )],
        ];
        simplify(&mut blocks, &[vec![1], vec![]], &[Key::Register("rbx")]);
        assert_eq!(
            blocks[0].len(),
            1,
            "the assignment feeds the block below and must survive"
        );
    }

    /// What the stack pointer moving out and back leaves behind is arithmetic
    /// that was never in the program.
    #[test]
    fn the_arithmetic_substitution_leaves_behind_is_folded_away() {
        let mut expression = Expr::binary(
            Binary::Add,
            Expr::binary(
                Binary::Subtract,
                Expr::read(register("rsp", Width::Qword)),
                Expr::constant(8, Width::Qword),
            ),
            Expr::constant(8, Width::Qword),
        );
        fold(&mut expression);
        assert_eq!(expression, Expr::read(register("rsp", Width::Qword)));
    }

    #[test]
    fn a_negated_comparison_becomes_the_opposite_comparison() {
        let mut expression = Expr::unary(
            Unary::LogicalNot,
            Expr::binary(
                Binary::LessOrEqual,
                Expr::read(register("rax", Width::Qword)),
                Expr::constant(4, Width::Qword),
            ),
        );
        fold(&mut expression);
        assert_eq!(
            expression,
            Expr::binary(
                Binary::Greater,
                Expr::read(register("rax", Width::Qword)),
                Expr::constant(4, Width::Qword)
            )
        );
    }

    /// Deleting a call because nothing reads its result would delete the call.
    #[test]
    fn a_call_whose_result_is_unused_is_still_a_call() {
        let mut blocks = vec![vec![Statement::new(
            0x10,
            Stmt::Call {
                result: Some(register("rax", Width::Qword)),
                callee: crate::decompiler::native::ir::Callee::Named("puts".to_owned()),
                arguments: Vec::new(),
            },
        )]];
        simplify(&mut blocks, &[vec![]], &[]);
        assert_eq!(blocks[0].len(), 1);
    }
}
