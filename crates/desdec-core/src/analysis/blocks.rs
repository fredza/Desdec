//! Cutting a decoded function into basic blocks.
//!
//! A basic block is a run of instructions with one way in and one way out: the
//! flow enters at the top and leaves at the bottom, and nothing branches in
//! between. Cut that way, a function stops being a column of hundreds of lines
//! and becomes a shape — a test with two arms, a loop with a body, a chain of
//! guards that all leave early — which is what the graph view draws.
//!
//! What is read here is only what the listing states. A branch whose target is
//! in a register leaves a block with no successor rather than a guessed one,
//! and a target outside the function's own body is not made up into a block:
//! both are recorded as the block simply having nowhere further to go, because
//! inventing an edge would draw a shape the code does not have.

use std::{
    collections::{BTreeSet, HashMap},
    ops::Range,
};

use crate::{
    Instruction,
    analysis::{
        flow::{self, Kind},
        operand,
    },
};

/// Why the flow goes from one block to another.
///
/// Kept apart from the address because the two arms of a test are what a
/// reader is looking for, and an edge list that only carried addresses made
/// them indistinguishable: the graph drew two identical arrows out of every
/// comparison and left the reader to work out which was which.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edge {
    /// The next instruction in the listing, nothing having branched.
    FallThrough,
    /// Where an unconditional jump goes.
    Jump,
    /// Where a conditional branch goes when its condition holds.
    Taken,
    /// Where it carries on when the condition does not hold.
    NotTaken,
}

/// One way out of a block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Successor {
    pub address: u64,
    pub edge: Edge,
}

/// How the flow leaves a block, for the two cases where it leaves without an
/// arrow to draw.
///
/// The two are not the same thing and must not read as one. A function that
/// returns goes somewhere perfectly well known — back to whoever called it —
/// and drawing no arrow for it says nothing is unknown. A branch through a
/// register goes somewhere only a running program knows, and that *is* a limit
/// of what has been read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Exit {
    /// On to the blocks the successors name.
    Onwards,
    /// Back to whoever called this function.
    Returns,
    /// Through a register, or out of this function's own body: where it goes
    /// is not something the listing states.
    Unstated,
}

/// A run of instructions the flow enters at the top and leaves at the bottom.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BasicBlock {
    /// Address of the first instruction.
    pub start: u64,
    /// Where the block's instructions sit in the slice it was cut from.
    pub instructions: Range<usize>,
    /// Where the flow goes from here, empty when it goes nowhere inside this
    /// body — a return, or a branch through a register.
    pub successors: Vec<Successor>,
    /// How the flow leaves, for when there is no arrow to draw.
    pub exit: Exit,
}

impl BasicBlock {
    #[must_use]
    pub const fn instruction_count(&self) -> usize {
        self.instructions.end - self.instructions.start
    }

    /// Whether the flow leaves this block for somewhere no arrow can be drawn
    /// to, which is what [`Self::exit`] then says the nature of.
    #[must_use]
    pub fn leaves(&self) -> bool {
        self.successors.is_empty()
    }
}

/// Cuts a decoded function body into basic blocks, in address order.
///
/// `instructions` is one function's body and nothing else: the addresses in
/// the successors are addresses inside it, so a caller drawing the result
/// never has to ask whether an edge leads out of the function.
#[must_use]
pub fn of(instructions: &[Instruction]) -> Vec<BasicBlock> {
    if instructions.is_empty() {
        return Vec::new();
    }

    let index_of: HashMap<u64, usize> = instructions
        .iter()
        .enumerate()
        .map(|(index, instruction)| (instruction.address, index))
        .collect();

    // Where a block may begin: the first instruction, anything branched to
    // from within this body, and whatever follows an instruction the flow
    // leaves from.
    let mut leaders = BTreeSet::from([0]);
    for (index, instruction) in instructions.iter().enumerate() {
        let kind = flow::kind_of(&instruction.text);
        // A call's target is not a leader: it is the beginning of another
        // function, and where it happens to be this one — a recursive call —
        // the flow comes back, so cutting there would say something untrue
        // about this block.
        if kind != Kind::Call
            && let Some(target) = branch_target(instruction).and_then(|at| index_of.get(&at))
        {
            leaders.insert(*target);
        }
        if kind.ends_a_block() && index + 1 < instructions.len() {
            leaders.insert(index + 1);
        }
    }

    let leaders: Vec<usize> = leaders.into_iter().collect();
    let starts: BTreeSet<u64> = leaders
        .iter()
        .map(|index| instructions[*index].address)
        .collect();

    leaders
        .iter()
        .enumerate()
        .map(|(position, start)| {
            let end = leaders
                .get(position + 1)
                .copied()
                .unwrap_or(instructions.len());
            let last = &instructions[end - 1];
            let fall_through = leaders
                .get(position + 1)
                .map(|index| instructions[*index].address);
            let successors = successors_of(last, fall_through, &starts);
            BasicBlock {
                start: instructions[*start].address,
                instructions: *start..end,
                exit: exit_of(last, &successors),
                successors,
            }
        })
        .collect()
}

/// Where the flow goes from the last instruction of a block.
fn successors_of(
    last: &Instruction,
    fall_through: Option<u64>,
    starts: &BTreeSet<u64>,
) -> Vec<Successor> {
    let branch = branch_target(last).filter(|address| starts.contains(address));
    let onwards = |edge: Edge| {
        fall_through
            .filter(|address| starts.contains(address))
            .map(|address| Successor { address, edge })
    };

    match flow::kind_of(&last.text) {
        // Both arms, taken first: a reader looking at a test wants the branch
        // before the way past it, which is the order the code reads in.
        Kind::Conditional => branch
            .map(|address| Successor {
                address,
                edge: Edge::Taken,
            })
            .into_iter()
            .chain(onwards(Edge::NotTaken))
            .collect(),
        // A jump through a register states no target, and none is invented.
        Kind::Jump => branch
            .map(|address| Successor {
                address,
                edge: Edge::Jump,
            })
            .into_iter()
            .collect(),
        Kind::Return => Vec::new(),
        Kind::Call | Kind::Ordinary => onwards(Edge::FallThrough).into_iter().collect(),
    }
}

/// How the flow leaves a block, once it is known where it goes.
fn exit_of(last: &Instruction, successors: &[Successor]) -> Exit {
    if !successors.is_empty() {
        return Exit::Onwards;
    }
    match flow::kind_of(&last.text) {
        Kind::Return => Exit::Returns,
        // A conditional branch always has the way past it, so a conditional
        // with no successors at all is one whose next instruction is outside
        // this body — the end of what was decoded.
        Kind::Jump | Kind::Conditional | Kind::Call | Kind::Ordinary => Exit::Unstated,
    }
}

/// Where the last instruction of a block branches to, when it says.
fn branch_target(instruction: &Instruction) -> Option<u64> {
    operand::branch_target(instruction)
}

#[cfg(test)]
mod tests {
    use super::{Edge, Exit, of};
    use crate::{Instruction, InstructionBytes};

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

    /// A straight run of instructions is one block, however long it is.
    #[test]
    fn code_that_never_branches_is_a_single_block() {
        let blocks = of(&body(&[
            (0x10, "mov rax, 1"),
            (0x14, "add rax, 2"),
            (0x18, "ret"),
        ]));
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].start, 0x10);
        assert_eq!(blocks[0].instruction_count(), 3);
        assert!(blocks[0].leaves(), "a return draws no arrow in this body");
        assert_eq!(blocks[0].exit, Exit::Returns);
    }

    /// The two arms of a test are told apart, and the taken one comes first.
    #[test]
    fn a_conditional_branch_leaves_two_named_arms() {
        let blocks = of(&body(&[
            (0x10, "cmp rax, 0"),
            (0x14, "jne 0x20"),
            (0x18, "mov rax, 1"),
            (0x1c, "ret"),
            (0x20, "mov rax, 2"),
            (0x24, "ret"),
        ]));
        assert_eq!(blocks.len(), 3, "the test, and one block per arm");
        let out = &blocks[0].successors;
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].edge, Edge::Taken);
        assert_eq!(out[0].address, 0x20);
        assert_eq!(out[1].edge, Edge::NotTaken);
        assert_eq!(out[1].address, 0x18);
    }

    /// A loop's back edge lands on a leader, which is what makes the shape a
    /// loop rather than a chain.
    #[test]
    fn a_backward_branch_closes_a_loop() {
        let blocks = of(&body(&[
            (0x10, "mov rcx, 10"),
            (0x14, "dec rcx"),
            (0x18, "jne 0x14"),
            (0x1c, "ret"),
        ]));
        assert_eq!(blocks.len(), 3);
        let looping = blocks
            .iter()
            .find(|block| block.start == 0x14)
            .expect("the body of the loop");
        assert!(
            looping
                .successors
                .iter()
                .any(|out| out.address == 0x14 && out.edge == Edge::Taken),
            "the block branches back to its own start"
        );
    }

    /// A call comes back, so it neither ends a block nor starts one.
    #[test]
    fn a_call_stays_inside_the_block_it_sits_in() {
        let blocks = of(&body(&[
            (0x10, "mov rdi, 1"),
            (0x14, "call 0x900"),
            (0x18, "mov rax, 0"),
            (0x1c, "ret"),
        ]));
        assert_eq!(blocks.len(), 1, "one block, not one per call");
    }

    /// A branch through a register states no target, and none is invented.
    #[test]
    fn an_indirect_jump_leaves_the_block_with_nowhere_stated() {
        let blocks = of(&body(&[(0x10, "mov rax, 1"), (0x14, "jmp rax")]));
        assert_eq!(blocks.len(), 1);
        assert!(
            blocks[0].leaves(),
            "nothing is guessed for an indirect jump"
        );
        assert_eq!(
            blocks[0].exit,
            Exit::Unstated,
            "and it is not reported as a return, which goes somewhere known"
        );
    }

    /// A branch out of this body is not made into a block of it.
    #[test]
    fn a_branch_out_of_the_body_is_not_drawn_as_an_edge_inside_it() {
        let blocks = of(&body(&[(0x10, "je 0x5000"), (0x14, "ret")]));
        assert_eq!(blocks.len(), 2);
        let out = &blocks[0].successors;
        assert_eq!(out.len(), 1, "only the arm that stays in the body");
        assert_eq!(out[0].edge, Edge::NotTaken);
        assert_eq!(out[0].address, 0x14);
    }

    #[test]
    fn an_empty_body_has_no_blocks() {
        assert!(of(&[]).is_empty());
    }
}
