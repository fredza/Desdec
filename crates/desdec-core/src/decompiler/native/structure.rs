//! Giving the graph back the shape the source had.
//!
//! A compiler is handed `while (i < n) { … }` and emits a comparison, a branch
//! forward, a body, and a branch back. The four are what the listing shows and
//! what [`crate::analysis::blocks`] cuts into basic blocks; the loop is what
//! the person who wrote it saw. Recovering it is what separates a decompiler
//! from a listing with C punctuation, and it is the difference between reading
//! forty `goto`s and reading a loop.
//!
//! The method is the one that has been used since Cifuentes' thesis and that
//! Ghidra, Hex-Rays and Binary Ninja all still use in some form: find the
//! dominators, find the back edges, and match the graph against a small set of
//! shapes.
//!
//! - **Dominance.** Block `d` dominates `b` when every path from the entry to
//!   `b` goes through `d`. Computed by the ordinary iterative algorithm, which
//!   is a few passes over the graph in reverse post-order.
//! - **A back edge** is an edge `u → v` where `v` dominates `u`: the flow
//!   returning to somewhere it has already been, which is a loop and cannot be
//!   anything else.
//! - **Post-dominance**, the same thing computed on the reversed graph, gives
//!   the point where the two arms of a test come back together — which is
//!   exactly where the `if` ends and is not otherwise visible.
//!
//! # What comes out, and what does not
//!
//! `while`, `do`/`while`, an unconditional loop with `break`s, `if`, `if`/`else`,
//! sequences, `break` and `continue`. Where the graph does not match — and some
//! graphs genuinely do not, because `goto` exists and because optimisers
//! produce shapes no source ever had — a `goto` and a label come out instead.
//! That is not a failure to be hidden: a function whose flow really is
//! irreducible is one a reader needs to know is irreducible, and inventing a
//! loop for it would be a lie about the program.
//!
//! Nothing here reads an instruction. It works on the edges the block cutter
//! found, so it applies to any architecture whose branches that module can
//! read — which is both of them.

use std::collections::{HashMap, HashSet};

use crate::{
    analysis::blocks::{BasicBlock, Edge},
    decompiler::native::ir::Expr,
};

/// One piece of recovered structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Structured {
    /// The statements of one basic block.
    ///
    /// `with_terminator` is false when the block's own branch has been taken
    /// up into an `if` or a loop above it: the branch is then said once, as
    /// the condition, rather than twice.
    Block {
        index: usize,
        with_terminator: bool,
    },
    Sequence(Vec<Structured>),
    If {
        condition: Expr,
        /// The address of the branch this came from, so the line knows where
        /// it is from.
        at: u64,
        then_branch: Box<Structured>,
        else_branch: Option<Box<Structured>>,
    },
    /// A loop whose test is at the top.
    While {
        condition: Expr,
        at: u64,
        body: Box<Structured>,
    },
    /// A loop whose test is at the bottom, which is what a compiler emits for
    /// a `for` it knows runs at least once.
    DoWhile {
        body: Box<Structured>,
        condition: Expr,
        at: u64,
    },
    /// A loop with no test of its own: left by a shape that matched neither of
    /// the above, and left by `while (1)`.
    Loop { body: Box<Structured> },
    Break,
    Continue,
    /// The flow goes somewhere structure could not account for.
    Goto { target: u64 },
    /// A block something jumps to, which therefore needs a name.
    Label {
        target: u64,
        body: Box<Structured>,
    },
    Nothing,
}

/// What structuring produced, and what it could not account for.
#[derive(Clone, Debug)]
pub struct Structure {
    pub root: Structured,
    /// Addresses a `goto` names, so the emitter knows which blocks to label.
    pub labelled: HashSet<u64>,
}

/// Recovers the structure of one function's blocks.
///
/// `conditions` gives, per block, the condition its terminating branch tests
/// when it has one — read from the IR rather than from the instruction, so it
/// is the simplified condition the dataflow pass produced.
#[must_use]
#[expect(
    clippy::implicit_hasher,
    reason = "the conditions are built by this crate's own pipeline, never by a caller"
)]
pub fn recover(blocks: &[BasicBlock], conditions: &HashMap<usize, (Expr, u64)>) -> Structure {
    if blocks.is_empty() {
        return Structure {
            root: Structured::Nothing,
            labelled: HashSet::new(),
        };
    }
    let index_of: HashMap<u64, usize> = blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.start, index))
        .collect();
    let successors: Vec<Vec<(usize, Edge)>> = blocks
        .iter()
        .map(|block| {
            block
                .successors
                .iter()
                .filter_map(|successor| {
                    index_of
                        .get(&successor.address)
                        .map(|index| (*index, successor.edge))
                })
                .collect()
        })
        .collect();

    let mut structurer = Structurer::new(blocks, &successors, conditions);
    let root = structurer.region(0, None, &mut Vec::new());
    Structure {
        root,
        labelled: structurer.labelled,
    }
}

struct Structurer<'a> {
    blocks: &'a [BasicBlock],
    successors: &'a [Vec<(usize, Edge)>],
    conditions: &'a HashMap<usize, (Expr, u64)>,
    post_dominators: Vec<Option<usize>>,
    /// Header → the blocks its natural loop contains.
    loops: HashMap<usize, HashSet<usize>>,
    /// Header → where the flow goes when the loop ends, when there is one such
    /// place.
    exits: HashMap<usize, Option<usize>>,
    emitted: HashSet<usize>,
    labelled: HashSet<u64>,
}

/// One loop being structured, so a jump inside it can be recognised as a
/// `break` or a `continue` rather than written as a `goto`.
struct Frame {
    header: usize,
    exit: Option<usize>,
}

impl<'a> Structurer<'a> {
    fn new(
        blocks: &'a [BasicBlock],
        successors: &'a [Vec<(usize, Edge)>],
        conditions: &'a HashMap<usize, (Expr, u64)>,
    ) -> Self {
        let dominators = dominators(successors, blocks.len());
        let post_dominators = post_dominators(successors, blocks.len());
        let mut loops = HashMap::new();
        let mut exits = HashMap::new();
        for (tail, edges) in successors.iter().enumerate() {
            for (head, _) in edges {
                // A back edge: the flow returning to somewhere that dominates
                // it, which is a loop and cannot be anything else.
                if dominates(&dominators, *head, tail) {
                    let body = natural_loop(successors, *head, tail);
                    let exit = single_exit(successors, &body);
                    loops
                        .entry(*head)
                        .and_modify(|existing: &mut HashSet<usize>| existing.extend(body.iter()))
                        .or_insert(body);
                    exits.entry(*head).or_insert(exit);
                }
            }
        }
        Self {
            blocks,
            successors,
            conditions,
            post_dominators,
            loops,
            exits,
            emitted: HashSet::new(),
            labelled: HashSet::new(),
        }
    }

    /// Structures the flow from `start` until `stop`, which is where the
    /// caller will carry on from.
    fn region(&mut self, start: usize, stop: Option<usize>, frames: &mut Vec<Frame>) -> Structured {
        let mut items: Vec<Structured> = Vec::new();
        let mut current = Some(start);
        while let Some(index) = current {
            if Some(index) == stop {
                break;
            }
            // Somewhere the flow has already been. Inside a loop that is a
            // `continue` or a `break`; anywhere else it is the one thing
            // structure could not account for.
            if self.emitted.contains(&index) {
                items.push(self.jump_to(index, frames));
                break;
            }
            if self.loops.contains_key(&index) && !frames.iter().any(|frame| frame.header == index) {
                let (loop_item, after) = self.build_loop(index, frames);
                items.push(loop_item);
                current = after.filter(|next| Some(*next) != stop);
                if current.is_none() {
                    break;
                }
                continue;
            }
            self.emitted.insert(index);
            let edges = &self.successors[index];
            match edges.len() {
                0 => {
                    items.push(Self::block(index, true));
                    current = None;
                }
                1 => {
                    // Without its terminator: where the flow goes from a block
                    // with one way out is what comes next in the output, and
                    // an unconditional `goto` saying the same thing is exactly
                    // what structuring is for removing.
                    items.push(Self::block(index, false));
                    current = Some(edges[0].0);
                }
                _ => {
                    let (item, after) = self.build_if(index, stop, frames);
                    items.push(item);
                    current = after;
                }
            }
        }
        match items.len() {
            0 => Structured::Nothing,
            1 => items.pop().unwrap_or(Structured::Nothing),
            _ => Structured::Sequence(items),
        }
    }

    /// A block, labelled when something jumps to it.
    const fn block(index: usize, with_terminator: bool) -> Structured {
        Structured::Block {
            index,
            with_terminator,
        }
    }

    /// Where the flow goes when it reaches somewhere it has already been.
    fn jump_to(&mut self, index: usize, frames: &[Frame]) -> Structured {
        if let Some(frame) = frames.last() {
            if frame.header == index {
                return Structured::Continue;
            }
            if frame.exit == Some(index) {
                return Structured::Break;
            }
        }
        let target = self.blocks[index].start;
        self.labelled.insert(target);
        Structured::Goto { target }
    }

    /// A test with two arms.
    ///
    /// The arms meet again at the immediate post-dominator, which is where the
    /// `if` ends. Where they do not meet at all — both arms return, or one
    /// leaves the function — there is nothing after the `if` and the whole of
    /// the rest goes inside it.
    fn build_if(
        &mut self,
        index: usize,
        _stop: Option<usize>,
        frames: &mut Vec<Frame>,
    ) -> (Structured, Option<usize>) {
        let Some((condition, at)) = self.conditions.get(&index).cloned() else {
            // Two ways out and no condition read from the IR: an indirect
            // branch, or an instruction the lifter did not model. Neither can
            // be turned into an `if` honestly.
            return (Self::block(index, true), None);
        };
        let taken = self.successors[index]
            .iter()
            .find(|(_, edge)| *edge == Edge::Taken)
            .map(|(target, _)| *target);
        let not_taken = self.successors[index]
            .iter()
            .find(|(_, edge)| matches!(edge, Edge::NotTaken | Edge::FallThrough))
            .map(|(target, _)| *target);
        let (Some(taken), Some(not_taken)) = (taken, not_taken) else {
            return (Self::block(index, true), None);
        };

        // A join that *is* the caller's stop is the best possible one: both
        // arms end there and the caller carries on from it. Excluding it left
        // the inner `if` of a nested pair with no join at all, so its first
        // arm ran on and swallowed the code after the outer one.
        let join = self.post_dominators[index]
            .filter(|join| *join != index && !self.emitted.contains(join));
        let leading = Self::block(index, false);

        // A test whose taken arm goes straight to where both arms meet is a
        // test *guarding* the other arm: written as it stands it would be an
        // `if` with an empty body and an `else`, which is not how anyone
        // writes C.
        let (condition, then_start, else_start) = if Some(taken) == join {
            (negate(&condition), not_taken, None)
        } else if Some(not_taken) == join {
            (condition, taken, None)
        } else {
            (condition, taken, Some(not_taken))
        };

        let then_branch = self.region(then_start, join, frames);
        let else_branch = else_start.map(|start| Box::new(self.region(start, join, frames)));
        let item = Structured::Sequence(vec![
            leading,
            Structured::If {
                condition,
                at,
                then_branch: Box::new(then_branch),
                else_branch,
            },
        ]);
        (item, join)
    }

    /// A loop, in whichever of the three shapes it turns out to have.
    fn build_loop(&mut self, header: usize, frames: &mut Vec<Frame>) -> (Structured, Option<usize>) {
        let body = self.loops.get(&header).cloned().unwrap_or_default();
        let exit = self.exits.get(&header).copied().flatten();
        frames.push(Frame { header, exit });

        // A test at the top: the header's only business is the condition, and
        // one of its arms leaves the loop. That is a `while`, and it is what a
        // reader is hoping to see.
        let header_tests_at_the_top = self.blocks[header].instruction_count() > 0
            && self.conditions.contains_key(&header)
            && self.successors[header]
                .iter()
                .any(|(target, _)| !body.contains(target));

        let item = if header_tests_at_the_top {
            self.emitted.insert(header);
            let (condition, at) = self.conditions[&header].clone();
            let taken = self.successors[header]
                .iter()
                .find(|(_, edge)| *edge == Edge::Taken)
                .map(|(target, _)| *target);
            let inside = self.successors[header]
                .iter()
                .find(|(target, _)| body.contains(target) && *target != header)
                .map(|(target, _)| *target);
            // The condition as written is the one that branches; the loop
            // continues while the *other* one holds when the taken arm leaves.
            let continues_on_taken = taken.is_some_and(|target| body.contains(&target));
            let condition = if continues_on_taken {
                condition
            } else {
                negate(&condition)
            };
            let body_item = inside.map_or(Structured::Nothing, |start| {
                self.region(start, Some(header), frames)
            });
            Structured::Sequence(vec![
                // Whatever the header does besides testing — a compiler often
                // reloads a value there — belongs inside the loop, before the
                // test. Saying it once, as the block, keeps it truthful.
                Self::block(header, false),
                Structured::While {
                    condition,
                    at,
                    body: Box::new(body_item),
                },
            ])
        } else {
            // Everything else becomes an unconditional loop whose exits are
            // `break`s. That is always correct, and where the shape really was
            // a `do`/`while` the tail's own test turns into `if (…) break;`,
            // which reads nearly as well and claims nothing untrue.
            let body_item = self.region(header, None, frames);
            Structured::Loop {
                body: Box::new(body_item),
            }
        };
        frames.pop();
        (item, exit)
    }
}

/// The condition that holds exactly when this one does not.
fn negate(condition: &Expr) -> Expr {
    if let Expr::Binary {
        operator,
        left,
        right,
    } = condition
        && let Some(opposite) = operator.negated()
    {
        return Expr::binary(opposite, (**left).clone(), (**right).clone());
    }
    if let Expr::Unary {
        operator: crate::decompiler::native::ir::Unary::LogicalNot,
        operand,
    } = condition
    {
        return (**operand).clone();
    }
    Expr::unary(crate::decompiler::native::ir::Unary::LogicalNot, condition.clone())
}

// ---------------------------------------------------------------------------
// Dominance
// ---------------------------------------------------------------------------

/// The immediate dominator of each block, by the iterative algorithm.
///
/// Cooper, Harvey and Kennedy's formulation: walk the blocks in reverse
/// post-order, intersect the dominators of each block's predecessors, repeat
/// until nothing moves. Simpler than the near-linear algorithm and fast enough
/// on a graph the size of a function.
fn dominators(successors: &[Vec<(usize, Edge)>], count: usize) -> Vec<Option<usize>> {
    if count == 0 {
        return Vec::new();
    }
    let order = reverse_post_order(successors, count, 0);
    let mut position = vec![usize::MAX; count];
    for (rank, node) in order.iter().enumerate() {
        position[*node] = rank;
    }
    let predecessors = reverse(successors, count);

    let mut idom: Vec<Option<usize>> = vec![None; count];
    idom[0] = Some(0);
    let mut changed = true;
    while changed {
        changed = false;
        for node in order.iter().copied().filter(|node| *node != 0) {
            let mut new: Option<usize> = None;
            for predecessor in &predecessors[node] {
                if idom[*predecessor].is_none() {
                    continue;
                }
                new = Some(match new {
                    None => *predecessor,
                    Some(current) => intersect(&idom, &position, current, *predecessor),
                });
            }
            if new.is_some() && idom[node] != new {
                idom[node] = new;
                changed = true;
            }
        }
    }
    idom
}

/// The same thing on the reversed graph, which gives where two arms meet.
///
/// A graph can have several exits — a function with three `return`s — and post
/// dominance needs one. The exits are joined to a virtual node, which is the
/// extra entry in the vectors below and is dropped from the answer.
fn post_dominators(successors: &[Vec<(usize, Edge)>], count: usize) -> Vec<Option<usize>> {
    if count == 0 {
        return Vec::new();
    }
    let virtual_exit = count;
    // The reversed graph, with every block that leaves the function pointing
    // at the virtual exit so that all paths end in one place.
    let mut reversed: Vec<Vec<(usize, Edge)>> = vec![Vec::new(); count + 1];
    for (node, edges) in successors.iter().enumerate() {
        if edges.is_empty() {
            reversed[virtual_exit].push((node, Edge::FallThrough));
        }
        for (target, edge) in edges {
            reversed[*target].push((node, *edge));
        }
    }
    // Rooted at the virtual exit, which is where the reversed walk starts.
    let mut rotated: Vec<Vec<(usize, Edge)>> = vec![Vec::new(); count + 1];
    rotated[..=count].clone_from_slice(&reversed[..=count]);
    let idom = dominators_rooted(&rotated, count + 1, virtual_exit);
    idom.into_iter()
        .take(count)
        .map(|dominator| dominator.filter(|node| *node != virtual_exit))
        .collect()
}

fn dominators_rooted(
    successors: &[Vec<(usize, Edge)>],
    count: usize,
    root: usize,
) -> Vec<Option<usize>> {
    let order = reverse_post_order(successors, count, root);
    let mut position = vec![usize::MAX; count];
    for (rank, node) in order.iter().enumerate() {
        position[*node] = rank;
    }
    let predecessors = reverse(successors, count);
    let mut idom: Vec<Option<usize>> = vec![None; count];
    idom[root] = Some(root);
    let mut changed = true;
    while changed {
        changed = false;
        for node in order.iter().copied().filter(|node| *node != root) {
            let mut new: Option<usize> = None;
            for predecessor in &predecessors[node] {
                if idom[*predecessor].is_none() {
                    continue;
                }
                new = Some(match new {
                    None => *predecessor,
                    Some(current) => intersect(&idom, &position, current, *predecessor),
                });
            }
            if new.is_some() && idom[node] != new {
                idom[node] = new;
                changed = true;
            }
        }
    }
    idom
}

/// Walks up both dominator chains until they meet.
fn intersect(
    idom: &[Option<usize>],
    position: &[usize],
    mut first: usize,
    mut second: usize,
) -> usize {
    // Bounded: a malformed chain would otherwise be an endless walk, and a
    // decompiler that hangs on one function is worse than one that gives a
    // conservative answer for it.
    let mut steps = 0;
    while first != second && steps < 4096 {
        steps += 1;
        while position[first] > position[second] {
            match idom[first] {
                Some(next) if next != first => first = next,
                _ => return second,
            }
        }
        while position[second] > position[first] {
            match idom[second] {
                Some(next) if next != second => second = next,
                _ => return first,
            }
        }
    }
    first
}

fn dominates(idom: &[Option<usize>], dominator: usize, node: usize) -> bool {
    let mut current = node;
    let mut steps = 0;
    loop {
        if current == dominator {
            return true;
        }
        match idom[current] {
            Some(next) if next != current && steps < 4096 => {
                current = next;
                steps += 1;
            }
            _ => return false,
        }
    }
}

fn reverse(successors: &[Vec<(usize, Edge)>], count: usize) -> Vec<Vec<usize>> {
    let mut predecessors = vec![Vec::new(); count];
    for (node, edges) in successors.iter().enumerate() {
        for (target, _) in edges {
            if *target < count {
                predecessors[*target].push(node);
            }
        }
    }
    predecessors
}

/// Depth-first post-order, reversed — the order the dominator algorithm wants.
fn reverse_post_order(
    successors: &[Vec<(usize, Edge)>],
    count: usize,
    root: usize,
) -> Vec<usize> {
    let mut order = Vec::with_capacity(count);
    let mut seen = vec![false; count];
    // An explicit stack rather than recursion: a function of ten thousand
    // blocks is unusual but real, and it must not be a stack overflow.
    let mut stack = vec![(root, 0_usize)];
    seen[root] = true;
    while let Some((node, next)) = stack.pop() {
        if let Some((target, _)) = successors[node].get(next) {
            stack.push((node, next + 1));
            if *target < count && !seen[*target] {
                seen[*target] = true;
                stack.push((*target, 0));
            }
        } else {
            order.push(node);
        }
    }
    order.reverse();
    order
}

/// Everything that can reach the tail of a back edge without leaving through
/// its head: the loop's body.
fn natural_loop(
    successors: &[Vec<(usize, Edge)>],
    header: usize,
    tail: usize,
) -> HashSet<usize> {
    let predecessors = reverse(successors, successors.len());
    let mut body = HashSet::from([header]);
    let mut stack = vec![tail];
    while let Some(node) = stack.pop() {
        if body.insert(node) {
            stack.extend(predecessors[node].iter().copied());
        }
    }
    body
}

/// Where a loop goes when it ends, when every way out goes to the same place.
///
/// A loop with two exits — a `break` in the middle and a condition at the top
/// — has no single one, and saying so is what makes the structurer fall back
/// to an unconditional loop with `break`s rather than claim a `while` that
/// leaves out one of the ways out.
fn single_exit(successors: &[Vec<(usize, Edge)>], body: &HashSet<usize>) -> Option<usize> {
    let mut outside: Option<usize> = None;
    for node in body {
        for (target, _) in &successors[*node] {
            if body.contains(target) {
                continue;
            }
            match outside {
                None => outside = Some(*target),
                Some(existing) if existing == *target => {}
                Some(_) => return None,
            }
        }
    }
    outside
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::blocks::{Exit, Successor};
    use crate::decompiler::native::ir::{Binary, Register, Width};

    fn block(start: u64, successors: Vec<(u64, Edge)>) -> BasicBlock {
        BasicBlock {
            start,
            instructions: 0..1,
            exit: if successors.is_empty() {
                Exit::Returns
            } else {
                Exit::Onwards
            },
            successors: successors
                .into_iter()
                .map(|(address, edge)| Successor { address, edge })
                .collect(),
        }
    }

    fn condition() -> Expr {
        Expr::binary(
            Binary::Less,
            Expr::register(Register::new("rax", Width::Dword)),
            Expr::constant(10, Width::Dword),
        )
    }

    /// A test, a body, and a place both arms meet: the shape of every `if`
    /// ever compiled.
    #[test]
    fn a_test_whose_arms_meet_again_is_an_if() {
        let blocks = vec![
            block(0x10, vec![(0x30, Edge::Taken), (0x20, Edge::NotTaken)]),
            block(0x20, vec![(0x30, Edge::FallThrough)]),
            block(0x30, vec![]),
        ];
        let conditions = HashMap::from([(0, (condition(), 0x18))]);
        let structure = recover(&blocks, &conditions);
        let Structured::Sequence(items) = &structure.root else {
            panic!("a function is a sequence, got {:?}", structure.root);
        };
        // The guard, the body it guards, and what follows.
        let found = items.iter().any(|item| {
            matches!(item, Structured::Sequence(inner)
                if inner.iter().any(|it| matches!(it, Structured::If { else_branch: None, .. })))
        });
        assert!(found, "expected a guard, got {items:?}");
        assert!(
            structure.labelled.is_empty(),
            "a shape this ordinary needs no goto"
        );
    }

    /// The taken arm going straight to where both arms meet is a guard around
    /// the other one, and writing it as `if (c) {} else { … }` is not how
    /// anyone writes C.
    #[test]
    fn a_guard_is_turned_around_rather_than_given_an_empty_body() {
        let blocks = vec![
            block(0x10, vec![(0x30, Edge::Taken), (0x20, Edge::NotTaken)]),
            block(0x20, vec![(0x30, Edge::FallThrough)]),
            block(0x30, vec![]),
        ];
        let conditions = HashMap::from([(0, (condition(), 0x18))]);
        let structure = recover(&blocks, &conditions);
        let mut found = None;
        collect_ifs(&structure.root, &mut found);
        let Some(Structured::If {
            condition,
            else_branch,
            ..
        }) = found
        else {
            panic!("expected an if");
        };
        assert!(else_branch.is_none());
        let Expr::Binary { operator, .. } = condition else {
            panic!("the condition is a comparison");
        };
        assert_eq!(
            operator,
            Binary::GreaterOrEqual,
            "the guard should test the opposite of the branch"
        );
    }

    fn collect_ifs(item: &Structured, found: &mut Option<Structured>) {
        match item {
            Structured::If { .. } if found.is_none() => *found = Some(item.clone()),
            Structured::Sequence(items) => {
                for inner in items {
                    collect_ifs(inner, found);
                }
            }
            Structured::While { body, .. }
            | Structured::Loop { body }
            | Structured::Label { body, .. } => collect_ifs(body, found),
            _ => {}
        }
    }

    /// The flow returning to somewhere that dominates it is a loop and cannot
    /// be anything else.
    #[test]
    fn a_back_edge_becomes_a_loop() {
        let blocks = vec![
            block(0x10, vec![(0x20, Edge::FallThrough)]),
            block(0x20, vec![(0x30, Edge::Taken), (0x40, Edge::NotTaken)]),
            block(0x30, vec![(0x20, Edge::Jump)]),
            block(0x40, vec![]),
        ];
        let conditions = HashMap::from([(1, (condition(), 0x28))]);
        let structure = recover(&blocks, &conditions);
        let mut loops = 0;
        count_loops(&structure.root, &mut loops);
        assert_eq!(loops, 1, "got {:?}", structure.root);
    }

    fn count_loops(item: &Structured, found: &mut usize) {
        match item {
            Structured::While { body, .. }
            | Structured::DoWhile { body, .. }
            | Structured::Loop { body } => {
                *found += 1;
                count_loops(body, found);
            }
            Structured::Sequence(items) => {
                for inner in items {
                    count_loops(inner, found);
                }
            }
            Structured::If {
                then_branch,
                else_branch,
                ..
            } => {
                count_loops(then_branch, found);
                if let Some(other) = else_branch {
                    count_loops(other, found);
                }
            }
            Structured::Label { body, .. } => count_loops(body, found),
            _ => {}
        }
    }

    /// A loop whose header only tests, and whose taken arm stays inside, is a
    /// `while` — and the condition must be the one that keeps it turning, not
    /// the one that leaves.
    #[test]
    fn a_loop_that_tests_at_the_top_is_a_while() {
        let blocks = vec![
            block(0x10, vec![(0x20, Edge::FallThrough)]),
            block(0x20, vec![(0x30, Edge::Taken), (0x40, Edge::NotTaken)]),
            block(0x30, vec![(0x20, Edge::Jump)]),
            block(0x40, vec![]),
        ];
        let conditions = HashMap::from([(1, (condition(), 0x28))]);
        let structure = recover(&blocks, &conditions);
        let mut found = None;
        find_while(&structure.root, &mut found);
        let Some(Structured::While { condition, .. }) = found else {
            panic!("expected a while, got {:?}", structure.root);
        };
        let Expr::Binary { operator, .. } = condition else {
            panic!("the condition is a comparison");
        };
        assert_eq!(
            operator,
            Binary::Less,
            "the taken arm stays inside, so the loop turns while the branch is taken"
        );
    }

    fn find_while(item: &Structured, found: &mut Option<Structured>) {
        match item {
            Structured::While { .. } if found.is_none() => *found = Some(item.clone()),
            Structured::Sequence(items) => {
                for inner in items {
                    find_while(inner, found);
                }
            }
            Structured::Loop { body } | Structured::Label { body, .. } => find_while(body, found),
            Structured::If {
                then_branch,
                else_branch,
                ..
            } => {
                find_while(then_branch, found);
                if let Some(other) = else_branch {
                    find_while(other, found);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn a_straight_line_needs_no_structure_at_all() {
        let blocks = vec![block(0x10, vec![(0x20, Edge::FallThrough)]), block(0x20, vec![])];
        let structure = recover(&blocks, &HashMap::new());
        assert!(structure.labelled.is_empty());
        let Structured::Sequence(items) = &structure.root else {
            panic!("two blocks in a row are a sequence");
        };
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn nothing_at_all_structures_to_nothing() {
        let structure = recover(&[], &HashMap::new());
        assert_eq!(structure.root, Structured::Nothing);
    }
}
