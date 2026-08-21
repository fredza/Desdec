//! Following the flow of the code without running it.
//!
//! Desdec never executes the binary, and this does not change that: the walk
//! reads the same bytes the listing shows and moves the selection the way the
//! processor *would* move, one instruction at a time. Everything it can say is
//! arithmetic on decoded instructions — where a call goes, where it would come
//! back to, which instruction follows this one — and everything it cannot is
//! reported as such rather than guessed at.
//!
//! Two places are where a static walk and a running program part company, and
//! both are answered here rather than papered over:
//!
//! - **A conditional branch** has no answer without values. Stepping *into*
//!   one follows the branch, stepping *over* it carries on to the next
//!   instruction: the reader chooses the path, and the button they pressed
//!   says which one they chose.
//! - **An indirect call or jump** — `callq *%rax`, `br x8` — goes where a
//!   register points, and no register has a value here. The walk stops and
//!   says so.

use desdec_core::{Analysis, flow::{self, Kind}, operand};

/// How far a single press moves the walk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    /// Follow the flow, into calls and along branches.
    Into,
    /// Carry on past a call, and past a branch that may not be taken.
    Over,
    /// Leave the call the walk stepped into, back to where it would return.
    Out,
}

/// What the flow does at one instruction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Move {
    /// Carry on to the next instruction of the listing.
    Next(u64),
    /// Follow a branch to a fixed address.
    Branch(u64),
    /// Into a call: where it goes, and where it would come back to.
    Call { target: u64, returns_to: u64 },
    /// Back out of a call, to wherever the walk entered it from.
    Return,
    /// The flow goes where only a running program would know.
    Unresolved,
    /// Nothing follows: the decoded listing ends here.
    End,
}

/// Where the flow goes from one instruction, given what its operand resolves
/// to and which instruction follows it in the listing.
///
/// Kept free of the analysis so every rule can be read — and tested — as what
/// it is: a decision about one instruction. What the mnemonic *does* is
/// [`desdec_core::flow`]'s answer, shared with the code that cuts a function
/// into basic blocks: the walk and the graph must never disagree about
/// whether an instruction branches.
fn decide(mnemonic: &str, target: Option<u64>, next: Option<u64>, over: bool) -> Move {
    let fall_through = || next.map_or(Move::End, Move::Next);
    match flow::kind(mnemonic) {
        Kind::Return => Move::Return,
        // What stepping over is for: the body of a call is skipped, and a
        // condition this tool cannot evaluate is not guessed at. Both carry
        // on to the instruction after this one.
        Kind::Call | Kind::Conditional if over => fall_through(),
        Kind::Call => match (target, next) {
            (Some(target), Some(returns_to)) => Move::Call { target, returns_to },
            // A call whose target is a register goes somewhere only a running
            // program knows. Nothing is invented for it.
            _ => Move::Unresolved,
        },
        // A jump goes where it says, and a branch stepped *into* is a branch
        // the reader has chosen to see taken.
        Kind::Jump | Kind::Conditional => target.map_or(Move::Unresolved, Move::Branch),
        Kind::Ordinary => fall_through(),
    }
}

/// Where the flow goes from `address`.
#[must_use]
pub fn next_move(analysis: &Analysis, address: u64, over: bool) -> Move {
    let Some(index) = analysis.instruction_index(address) else {
        return Move::End;
    };
    let Some(instruction) = analysis.instructions.get(index) else {
        return Move::End;
    };
    // The instruction *after this one in the listing*, rather than the address
    // its bytes end at: the walk must land on something that was decoded, and
    // at the end of a section those are not the same place.
    let next = analysis
        .instructions
        .get(index + 1)
        .map(|instruction| instruction.address);
    let mnemonic = instruction
        .text
        .split_whitespace()
        .next()
        .unwrap_or_default();
    // Only an address that was decoded is somewhere to stand. A call into a
    // section beyond what could be read resolves arithmetically and leads
    // nowhere the listing can show, and a walk that landed there would leave
    // the reader on a blank selection with no way back.
    let target = operand::target_address(instruction)
        .filter(|address| analysis.instruction_index(*address).is_some());
    decide(mnemonic, target, next, over)
}

/// Where the walk stands, and how it got there.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Position {
    address: u64,
    /// Where each call stepped into would come back to, innermost last.
    returns: Vec<u64>,
}

/// The trail a reader has walked through the code.
///
/// The whole trail rather than just the current address: a reader who followed
/// a call four levels down needs the way back, and a step that turned out to
/// be the wrong branch has to be undoable. Each position keeps its own return
/// stack, so stepping back restores exactly the state that was left behind
/// rather than an approximation of it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Walk {
    trail: Vec<Position>,
}

/// How many steps are kept. A reader walking a loop can press a button for a
/// long time, and the trail is only ever read from its end.
const TRAIL_LIMIT: usize = 4096;

impl Walk {
    /// Where the walk stands, if it has started.
    #[must_use]
    pub fn current(&self) -> Option<u64> {
        self.trail.last().map(|position| position.address)
    }

    /// How many calls the walk has stepped into and not yet left.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.trail
            .last()
            .map_or(0, |position| position.returns.len())
    }

    /// How many steps have been taken since the walk began.
    #[must_use]
    pub fn steps(&self) -> usize {
        self.trail.len().saturating_sub(1)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.trail.is_empty()
    }

    pub fn clear(&mut self) {
        self.trail.clear();
    }

    /// Begins a walk at `address`, forgetting whatever came before.
    pub fn start(&mut self, address: u64) {
        self.trail.clear();
        self.trail.push(Position {
            address,
            returns: Vec::new(),
        });
    }

    /// Takes the walk to wherever the reader has moved the selection.
    ///
    /// Clicking a row, following a cross-reference or searching a string all
    /// move the selection without walking anywhere, and a trail that then
    /// claimed to have arrived there by stepping would be a false account of
    /// how the reader got there. So the walk restarts, rather than pretending.
    pub fn follow_selection(&mut self, selected: Option<u64>) {
        if self.current() == selected {
            return;
        }
        match selected {
            Some(address) => self.start(address),
            None => self.clear(),
        }
    }

    /// Where a step would land, without taking it, from wherever the reader
    /// has left the selection.
    ///
    /// The selection rather than the trail's own end, because that is where a
    /// step really would start: a reader who clicked a row and then pressed a
    /// transport button expects the walk to leave from the row they are
    /// looking at. A position reached by hand carries no call stack, so
    /// stepping out of it answers nothing — which is what a greyed-out button
    /// must be told.
    #[must_use]
    pub fn preview_from(
        &self,
        analysis: &Analysis,
        selected: Option<u64>,
        step: Step,
    ) -> Option<u64> {
        let here = if self.current() == selected {
            self.trail.last()?.clone()
        } else {
            Position {
                address: selected?,
                returns: Vec::new(),
            }
        };
        advance(&here, analysis, step).map(|position| position.address)
    }

    /// Takes one step, and says where it landed.
    pub fn step(&mut self, analysis: &Analysis, step: Step) -> Option<u64> {
        let position = self.outcome(analysis, step)?;
        let address = position.address;
        self.trail.push(position);
        if self.trail.len() > TRAIL_LIMIT {
            self.trail.remove(0);
        }
        Some(address)
    }

    /// Undoes the last step, and says where the walk stands afterwards.
    pub fn back(&mut self) -> Option<u64> {
        if self.trail.len() < 2 {
            return None;
        }
        self.trail.pop();
        self.current()
    }

    #[must_use]
    pub fn can_go_back(&self) -> bool {
        self.trail.len() > 1
    }

    /// The position one step would leave the walk in.
    fn outcome(&self, analysis: &Analysis, step: Step) -> Option<Position> {
        advance(self.trail.last()?, analysis, step)
    }
}

/// Where one step from `position` lands, with the call stack it leaves behind.
fn advance(position: &Position, analysis: &Analysis, step: Step) -> Option<Position> {
    let mut returns = position.returns.clone();
    let address = match step {
        Step::Out => returns.pop()?,
        Step::Into | Step::Over => {
            match next_move(analysis, position.address, step == Step::Over) {
                Move::Next(address) | Move::Branch(address) => address,
                Move::Call { target, returns_to } => {
                    returns.push(returns_to);
                    target
                }
                Move::Return => returns.pop()?,
                Move::Unresolved | Move::End => return None,
            }
        }
    };
    Some(Position { address, returns })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A call is the one instruction where stepping into and stepping over
    /// part company, and the difference is the whole point of two buttons.
    #[test]
    fn a_call_is_entered_by_one_button_and_skipped_by_the_other() {
        assert_eq!(
            decide("callq", Some(0x4020), Some(0x4010), false),
            Move::Call {
                target: 0x4020,
                returns_to: 0x4010
            }
        );
        assert_eq!(
            decide("callq", Some(0x4020), Some(0x4010), true),
            Move::Next(0x4010)
        );
    }

    /// The condition depends on values no static reading has, so the button
    /// decides the path — and neither button may invent one.
    #[test]
    fn a_conditional_branch_is_followed_or_stepped_past() {
        assert_eq!(
            decide("jne", Some(0x4100), Some(0x4008), false),
            Move::Branch(0x4100)
        );
        assert_eq!(
            decide("jne", Some(0x4100), Some(0x4008), true),
            Move::Next(0x4008)
        );
        assert_eq!(
            decide("b.eq", Some(0x4100), Some(0x4008), false),
            Move::Branch(0x4100)
        );
    }

    /// An unconditional jump goes where it says, whichever button is pressed:
    /// stepping over one and landing after it would be an account of the flow
    /// that never happens.
    #[test]
    fn an_unconditional_jump_is_followed_by_both_buttons() {
        for over in [false, true] {
            assert_eq!(
                decide("jmp", Some(0x4100), Some(0x4008), over),
                Move::Branch(0x4100)
            );
            assert_eq!(
                decide("b", Some(0x4100), Some(0x4008), over),
                Move::Branch(0x4100)
            );
        }
    }

    /// Where a register points is not knowable without running the program.
    #[test]
    fn an_indirect_call_or_jump_stops_the_walk() {
        assert_eq!(decide("callq", None, Some(0x4008), false), Move::Unresolved);
        assert_eq!(decide("jmpq", None, Some(0x4008), false), Move::Unresolved);
        assert_eq!(decide("br", None, Some(0x4008), true), Move::Unresolved);
        // Stepping over an indirect call still lands after it: where it went
        // does not have to be known to know where it comes back to.
        assert_eq!(
            decide("callq", None, Some(0x4008), true),
            Move::Next(0x4008)
        );
    }

    #[test]
    fn an_ordinary_instruction_falls_through_and_the_last_one_ends() {
        assert_eq!(decide("mov", None, Some(0x4008), false), Move::Next(0x4008));
        assert_eq!(decide("mov", None, None, false), Move::End);
        assert_eq!(decide("ret", None, Some(0x4008), false), Move::Return);
    }

    /// An address that was never decoded is nowhere to stand: the walk stops
    /// rather than selecting a row the listing does not have.
    #[test]
    fn an_address_outside_the_decoded_code_stops_the_walk() {
        let analysis = crate::testing::reference_analysis();
        // Nothing is decoded at the very top of the address space, so there is
        // neither an instruction here nor anything after one.
        assert_eq!(next_move(analysis, 0xffff_ffff_0000, false), Move::End);
        assert_eq!(next_move(analysis, 0xffff_ffff_0000, true), Move::End);
    }

    /// The trail is what makes a wrong turn undoable, and what a return needs
    /// to know where to go back to.
    #[test]
    fn stepping_back_restores_the_position_that_was_left() {
        let analysis = crate::testing::reference_analysis();
        let Some(first) = analysis.instructions.first().map(|i| i.address) else {
            return;
        };
        let mut walk = Walk::default();
        walk.start(first);
        assert_eq!(walk.steps(), 0);
        assert!(!walk.can_go_back());

        let Some(second) = walk.step(analysis, Step::Over) else {
            return; // A one-instruction listing: nothing to step to.
        };
        assert_eq!(walk.current(), Some(second));
        assert_eq!(walk.steps(), 1);

        assert_eq!(walk.back(), Some(first));
        assert_eq!(walk.steps(), 0);
        assert_eq!(walk.back(), None, "the first position is not a step");
    }

    /// A selection moved by hand — a click, a cross-reference — was not
    /// walked to, and the trail must not claim otherwise.
    #[test]
    fn moving_the_selection_by_hand_restarts_the_walk() {
        let mut walk = Walk::default();
        walk.start(0x1000);
        walk.trail.push(Position {
            address: 0x1004,
            returns: vec![0x1008],
        });
        assert_eq!(walk.depth(), 1);

        walk.follow_selection(Some(0x9000));

        assert_eq!(walk.current(), Some(0x9000));
        assert_eq!(walk.steps(), 0);
        assert_eq!(walk.depth(), 0, "the call stack belonged to the old trail");
    }

    /// Stepping into a call and back out of it lands after the call, which is
    /// where the program would come back to.
    #[test]
    fn stepping_out_returns_to_where_the_call_came_from() {
        let mut walk = Walk::default();
        walk.start(0x1000);
        walk.trail.push(Position {
            address: 0x2000,
            returns: vec![0x1005],
        });

        let ctx = crate::testing::reference_analysis();
        assert_eq!(walk.step(ctx, Step::Out), Some(0x1005));
        assert_eq!(walk.depth(), 0);
    }
}
