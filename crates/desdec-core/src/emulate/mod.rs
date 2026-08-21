//! Running a binary on a processor Desdec builds, rather than on yours.
//!
//! Desdec has never executed a file it opens, and this does not change that.
//! What runs here is an interpreter: a register file, an address space built
//! from the file's own section table, and a loop that reads the instructions
//! the listing already decoded and works out what each one would leave behind.
//! No byte of the file reaches the machine's processor, no page of it is ever
//! made executable, and nothing the program asks the operating system for is
//! passed on — there is no operating system behind this one.
//!
//! That is the whole trade. A debugger that attaches to a process answers
//! every question and runs the program on your machine; this answers the
//! questions that can be answered without one, and stops — by name — at each
//! point where it would have to invent an answer:
//!
//! - an instruction the interpreter does not carry out,
//! - a `syscall`, an interrupt, a `cpuid`: a question for a system,
//! - a read or a write outside anything the file maps,
//! - a call into a library, whose code is in another file that is not here.
//!
//! Every one of those stops the run and is reported. None of them is stepped
//! over, and none is answered with a plausible value: a register that holds a
//! made-up number is worse than a run that ended early, because the reader
//! cannot tell which is which afterwards.
//!
//! What that buys, and what a static reading could never give: real register
//! values, a real stack, memory as it stands at this instruction, an indirect
//! call that goes where it actually goes, a loop whose trip count is a fact,
//! and breakpoints that are reached rather than reasoned about.

/// Conditions a breakpoint carries; see the module's own documentation.
pub mod condition;
/// The emulated address space; see the module's own documentation.
pub mod memory;
/// The register file; see the module's own documentation.
pub mod registers;
/// System-call ABI decoding, without a host operating system; see the
/// module's own documentation.
pub mod system;
mod x86;

use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use iced_x86::{Decoder, DecoderOptions, Formatter, GasFormatter};

use crate::{
    Analysis, Architecture, BinaryFormat,
    emulate::{
        memory::{Access, Fault, Memory, Region},
        registers::Registers,
        x86::{Cpu, Outcome, Refusal},
    },
};

/// How many instructions the run can be taken back through.
///
/// Each step back costs one register file and whatever bytes that instruction
/// overwrote — a hundred and forty-odd bytes plus a handful — so a few
/// thousand of them is under a megabyte. Bounded for the same reason the trace
/// is: a run of a hundred million instructions must not become a hundred
/// million saved states.
pub const REWIND_LENGTH: usize = 4096;

/// How many executed instructions the run keeps a record of.
///
/// A ring, not a log: a run of a hundred million instructions must not turn
/// into a hundred million lines of memory, and what a reader wants after a
/// breakpoint is the recent past, not the whole of it.
pub const TRACE_LENGTH: usize = 4096;

/// How many instructions one press of "run" carries out before handing
/// control back, so the interface never stops answering.
pub const RUN_BUDGET: u64 = 2_000_000;

/// The value put on the stack as the return address of the first frame.
///
/// It is not a valid address, and it is not meant to be reached by executing
/// anything: a `ret` that lands on it is the outermost function returning, and
/// that is the run finishing normally rather than a fault.
pub const RETURN_SENTINEL: u64 = 0x0000_dead_0000_0000;

/// Why the emulation is not running.
///
/// Every variant is a fact about the program or about the emulator's own
/// limits, and each one is worded for the reader rather than for a log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Stop {
    /// A breakpoint the reader set was reached, before it ran.
    Breakpoint { address: u64 },
    /// A watched address was read or written.
    Watchpoint {
        address: u64,
        at: u64,
        access: Access,
    },
    /// The outermost function returned: the program, as started here, is over.
    Finished,
    /// The step or run the reader asked for is done.
    Paused,
    /// The interpreter does not carry out this instruction.
    Unsupported { at: u64, instruction: String },
    /// The program asked the operating system something, and there is none.
    SystemCall {
        at: u64,
        instruction: String,
        /// The request as its ABI registers describe it. It was observed, not
        /// executed and not answered.
        call: system::SystemCall,
    },
    /// Execution reached a place the file maps no bytes to — the usual sign of
    /// a call into a library, whose code lives in a file that is not open.
    LeftTheImage { at: u64 },
    /// Execution reached the first page, which nothing is ever mapped at.
    ///
    /// Distinguished from [`Self::LeftTheImage`] because its cause is always
    /// the same one and is worth naming: the call went through a table that
    /// only a loader fills in, and nothing has.
    UnresolvedCall { at: u64 },
    /// A read or a write could not be carried out.
    Fault { at: u64, fault: Fault },
    /// A division with no answer, or one too large to store.
    DivideError { at: u64 },
    /// The program stopped itself: `hlt`, `ud2`, `int3`.
    Halted { at: u64, instruction: String },
    /// The bytes at the instruction pointer decode to no instruction.
    Undecodable { at: u64 },
    /// The architecture is one the interpreter does not have.
    UnsupportedArchitecture { architecture: Architecture },
    /// The budget for one press ran out; pressing again carries on.
    OutOfBudget,
}

impl Stop {
    /// Whether the run may be carried on from here.
    ///
    /// A budget that ran out is resumable and a fault is not: the difference
    /// decides whether the interface offers to carry on or only to restart.
    #[must_use]
    pub const fn is_resumable(&self) -> bool {
        matches!(
            self,
            Self::Breakpoint { .. } | Self::Watchpoint { .. } | Self::Paused | Self::OutOfBudget
        )
    }

    /// The address the run stopped at, when the stop has one.
    #[must_use]
    pub const fn address(&self) -> Option<u64> {
        match self {
            Self::Breakpoint { address } => Some(*address),
            Self::Watchpoint { at, .. }
            | Self::Unsupported { at, .. }
            | Self::SystemCall { at, .. }
            | Self::LeftTheImage { at }
            | Self::UnresolvedCall { at }
            | Self::Fault { at, .. }
            | Self::DivideError { at }
            | Self::Halted { at, .. }
            | Self::Undecodable { at } => Some(*at),
            Self::Finished
            | Self::Paused
            | Self::UnsupportedArchitecture { .. }
            | Self::OutOfBudget => None,
        }
    }
}

/// One instruction the run carried out, kept for the trace.
#[derive(Clone, Debug)]
pub struct Executed {
    pub address: u64,
    pub text: String,
    /// How many instructions had run before this one.
    pub ordinal: u64,
}

/// Everything one instruction changed, kept so it can be put back.
///
/// This is what makes a run reversible, and it is a thing only an emulator can
/// offer: a debugger attached to a real process cannot un-write a byte. What
/// is stored is not a reading or a reconstruction — it is the state as it was,
/// so stepping back is exact rather than inferred.
#[derive(Clone, Debug)]
struct Undo {
    /// The registers as they stood before the instruction ran.
    registers: Registers,
    /// The bytes it overwrote, and what they held.
    memory: Vec<(u64, u8)>,
    /// The call depth before it.
    depth: i64,
    /// What it did to the call stack, so that can be put back too.
    frames: FrameChange,
}

/// What one instruction did to the call stack.
#[derive(Clone, Debug)]
enum FrameChange {
    /// Nothing.
    None,
    /// It made a call, so undoing it takes that frame off again.
    Pushed,
    /// It returned, so undoing it puts the frame back.
    Popped(Box<Frame>),
}

/// One call the run has made and not yet come back from.
///
/// Recorded as the call happens rather than reconstructed afterwards by
/// walking frame pointers: a function compiled without one, or stopped in the
/// middle of its prologue, has no chain to walk, and a call stack that is
/// sometimes a guess is worse than one that is always a fact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Frame {
    /// The `call` instruction that made it.
    pub called_from: u64,
    /// Where it went.
    pub entered: u64,
    /// Where it will come back to.
    pub returns_to: u64,
    /// The stack pointer just after the call was made.
    pub stack_pointer: u64,
}

/// A breakpoint, and what it takes for it to stop the run.
///
/// Bare, it stops every time. With a condition it stops only where that holds,
/// and with a pass count it lets that many qualifying passes go by first —
/// which together are what make a breakpoint inside a loop of ten thousand
/// turns worth setting at all.
#[derive(Clone, Debug, Default)]
pub struct Breakpoint {
    /// What the reader wrote, kept so it can be shown and edited again.
    pub condition: String,
    /// The same, read. `None` when there is no condition; a condition that
    /// does not parse is refused when it is set, so this is never a silent
    /// failure.
    parsed: Option<condition::Expression>,
    /// How many qualifying passes to let by before stopping. `0` stops at the
    /// first.
    pub skip: u64,
    /// How many times the run has been here with the condition holding, since
    /// the last restart. Shown, because "it never stopped" and "it stopped
    /// after nine hundred passes" are different things to be told.
    pub passes: u64,
    /// Whether it stops at all. A breakpoint turned off keeps its condition
    /// and its count, which is what makes turning it off useful rather than
    /// the same as deleting it.
    pub enabled: bool,
}

impl Breakpoint {
    /// A breakpoint that stops every time.
    #[must_use]
    pub fn always() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }

    /// Reads a condition into this breakpoint, or says why it cannot.
    ///
    /// # Errors
    ///
    /// [`condition::ParseError`], with the position in the text. The
    /// breakpoint is left exactly as it was: a condition that does not parse
    /// never replaces one that does.
    pub fn set_condition(&mut self, source: &str) -> Result<(), condition::ParseError> {
        if source.trim().is_empty() {
            self.condition.clear();
            self.parsed = None;
            return Ok(());
        }
        let parsed = condition::Expression::parse(source)?;
        source.clone_into(&mut self.condition);
        self.parsed = Some(parsed);
        Ok(())
    }

    /// Whether this breakpoint stops the run right now.
    ///
    /// Counts a pass whenever the condition holds, whether or not it stops on
    /// it: the count is of qualifying passes, so `skip` means what a reader
    /// expects it to mean.
    fn stops(&mut self, registers: &Registers, memory: &Memory) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(condition) = self.parsed.as_ref()
            && !condition.holds(registers, memory)
        {
            return false;
        }
        self.passes = self.passes.saturating_add(1);
        self.passes > self.skip
    }
}

/// An address the reader is watching, and what they are watching for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Watchpoint {
    pub address: u64,
    pub size: u64,
    pub on_read: bool,
    pub on_write: bool,
}

impl Watchpoint {
    /// Whether an access of `length` bytes at `address` touches this watch.
    const fn touched_by(&self, address: u64, length: u64) -> bool {
        address < self.address.wrapping_add(self.size)
            && self.address < address.wrapping_add(length)
    }
}

/// How far one press of a step button goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    /// One instruction, into whatever it calls.
    Into,
    /// One instruction, and the whole of a call it makes.
    Over,
    /// Until the function being run returns to the one that called it.
    Out,
    /// One instruction the other way: the state as it was before it ran.
    Back,
}

/// The emulated processor, its memory, and everything one run has done.
pub struct Machine {
    pub registers: Registers,
    pub memory: Memory,
    architecture: Architecture,
    bitness: u32,
    /// The container tells which OS ABI the system-request registers use.
    format: BinaryFormat,
    /// Where the run would start again from.
    entry_point: u64,
    /// The top of the allocated stack, which is where `rsp` starts.
    stack_top: u64,
    executed: u64,
    stop: Option<Stop>,
    breakpoints: BTreeMap<u64, Breakpoint>,
    watchpoints: Vec<Watchpoint>,
    trace: VecDeque<Executed>,
    /// How many calls deep the run is, relative to where it started. What
    /// "step out" counts, and what the interface shows as a call depth.
    depth: i64,
    /// The calls made and not yet returned from, outermost first.
    frames: Vec<Frame>,
    /// How to put back each of the last few instructions, newest last.
    rewind: VecDeque<Undo>,
}

impl Machine {
    /// Builds a machine for an analysed binary, ready to run from its entry
    /// point.
    ///
    /// The file's bytes are shared, not copied; the caller keeps its own copy
    /// for every other view.
    #[must_use]
    pub fn new(analysis: &Analysis, file: Arc<[u8]>) -> Self {
        let memory = Memory::load(file, analysis);
        Self::over_format(
            memory,
            analysis.summary.architecture,
            analysis.summary.format,
            analysis.entry_point.unwrap_or_default(),
        )
    }

    /// Builds a machine over an address space the caller has already laid
    /// out, starting at an address of their choosing.
    ///
    /// What [`Self::new`] uses, and what running one function rather than a
    /// whole file needs: the caller decides what is mapped and where the run
    /// begins, and gets the same stack, the same sentinel and the same stops.
    #[must_use]
    pub fn over(memory: Memory, architecture: Architecture, entry_point: u64) -> Self {
        Self::over_format(memory, architecture, BinaryFormat::Unknown, entry_point)
    }

    fn over_format(
        mut memory: Memory,
        architecture: Architecture,
        format: BinaryFormat,
        entry_point: u64,
    ) -> Self {
        let bitness = match architecture {
            Architecture::X86_64 => 64,
            Architecture::X86 => 32,
            _ => 0,
        };
        let stack_top = Self::place_stack(&mut memory);
        let mut machine = Self {
            registers: Registers::new(),
            memory,
            architecture,
            bitness,
            format,
            entry_point,
            stack_top,
            executed: 0,
            stop: None,
            breakpoints: BTreeMap::new(),
            watchpoints: Vec::new(),
            trace: VecDeque::new(),
            depth: 0,
            frames: Vec::new(),
            rewind: VecDeque::new(),
        };
        machine.restart();
        machine
    }

    /// Finds room for the stack, below the default top, and reserves it.
    ///
    /// Returns the address `rsp` starts at. A file that already maps the
    /// default range gets its stack elsewhere rather than on top of it.
    fn place_stack(memory: &mut Memory) -> u64 {
        let size = memory::DEFAULT_STACK_SIZE;
        let mut top = memory::DEFAULT_STACK_TOP;
        for _ in 0..64 {
            let start = top.saturating_sub(size);
            if memory.allocate("stack", start, size) {
                return top;
            }
            top = start.saturating_sub(memory::PAGE);
            if top <= size {
                break;
            }
        }
        top
    }

    /// Puts the machine back where it started: registers cleared, memory as
    /// the file has it, trace emptied. Breakpoints and watches are kept,
    /// because they are the reader's, not the run's.
    pub fn restart(&mut self) {
        self.registers = Registers::new();
        self.memory.forget_writes();
        self.executed = 0;
        self.trace.clear();
        self.depth = 0;
        self.frames.clear();
        self.rewind.clear();
        // The pass counts are of this run, not of the reader's session: a
        // breakpoint that had gone by nine hundred times has gone by none.
        for breakpoint in self.breakpoints.values_mut() {
            breakpoint.passes = 0;
        }
        self.stop = if self.bitness == 0 {
            Some(Stop::UnsupportedArchitecture {
                architecture: self.architecture,
            })
        } else {
            None
        };
        if self.bitness == 0 {
            return;
        }
        self.registers.instruction_pointer = self.entry_point;
        // Sixteen bytes of headroom below the top, and an alignment the
        // calling convention expects at the moment a function starts.
        let start = (self.stack_top - 64) & !0xf_u64;
        self.registers.set_stack_pointer(start);
        let mut cpu = self.cpu();
        // The first frame is given somewhere to return to, so that the
        // outermost `ret` is a program ending rather than a fault.
        let _ = cpu.push(RETURN_SENTINEL);
    }

    fn cpu(&mut self) -> Cpu<'_> {
        Cpu {
            registers: &mut self.registers,
            memory: &mut self.memory,
            bitness: self.bitness,
        }
    }

    /// Where the next instruction to run begins.
    #[must_use]
    pub const fn instruction_pointer(&self) -> u64 {
        self.registers.instruction_pointer
    }

    /// How many instructions this run has carried out.
    #[must_use]
    pub const fn executed(&self) -> u64 {
        self.executed
    }

    /// How many calls deep the run is.
    #[must_use]
    pub const fn depth(&self) -> i64 {
        self.depth
    }

    /// Why the run is not running, if it is not.
    #[must_use]
    pub const fn stop(&self) -> Option<&Stop> {
        self.stop.as_ref()
    }

    /// Whether the run can be carried on, or only restarted.
    #[must_use]
    pub fn can_continue(&self) -> bool {
        self.bitness != 0 && self.stop.as_ref().is_none_or(Stop::is_resumable)
    }

    /// How many instructions the run can be taken back through.
    ///
    /// Fewer than have been executed once the record is full: what is kept is
    /// the recent past, and the interface says so rather than offering a
    /// button that would stop working part way.
    #[must_use]
    pub fn rewindable(&self) -> usize {
        self.rewind.len()
    }

    /// Takes the run back one instruction, exactly.
    ///
    /// Not a re-run from the start and not a reading of what the instruction
    /// probably did: the registers and the overwritten bytes are put back as
    /// they were. Returns whether there was anything to go back through.
    pub fn step_back(&mut self) -> bool {
        let Some(undo) = self.rewind.pop_back() else {
            return false;
        };
        self.registers = undo.registers;
        self.memory.undo(&undo.memory);
        self.depth = undo.depth;
        match undo.frames {
            FrameChange::None => {}
            FrameChange::Pushed => {
                self.frames.pop();
            }
            FrameChange::Popped(frame) => self.frames.push(*frame),
        }
        self.executed = self.executed.saturating_sub(1);
        self.trace.pop_back();
        // Whatever stopped the run is undone with it: the instruction that
        // faulted has not run now, so the fault has not happened.
        self.stop = None;
        true
    }

    /// The calls the run is inside, outermost first.
    #[must_use]
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// The instructions most recently carried out, oldest first.
    #[must_use]
    pub fn trace(&self) -> impl DoubleEndedIterator<Item = &Executed> {
        self.trace.iter()
    }

    /// Every mapped region of the emulated address space.
    #[must_use]
    pub fn regions(&self) -> &[Region] {
        self.memory.regions()
    }

    // ----- what the reader sets ---------------------------------------------

    /// Puts a breakpoint at an address or takes it away, and says which it now
    /// is.
    ///
    /// Taking one away takes its condition with it. That is what the reader
    /// asked for; turning one off without losing what was written on it is
    /// [`Breakpoint::enabled`].
    pub fn toggle_breakpoint(&mut self, address: u64) -> bool {
        if self.breakpoints.remove(&address).is_some() {
            false
        } else {
            self.breakpoints.insert(address, Breakpoint::always());
            true
        }
    }

    #[must_use]
    pub fn has_breakpoint(&self, address: u64) -> bool {
        self.breakpoints.contains_key(&address)
    }

    /// The breakpoint at an address, to read what it carries.
    #[must_use]
    pub fn breakpoint(&self, address: u64) -> Option<&Breakpoint> {
        self.breakpoints.get(&address)
    }

    /// The same, to change it — a condition, a skip count, whether it is on.
    pub fn breakpoint_mut(&mut self, address: u64) -> Option<&mut Breakpoint> {
        self.breakpoints.get_mut(&address)
    }

    /// Every breakpoint, in address order.
    pub fn breakpoints(&self) -> impl Iterator<Item = (u64, &Breakpoint)> + '_ {
        self.breakpoints
            .iter()
            .map(|(address, breakpoint)| (*address, breakpoint))
    }

    pub fn clear_breakpoints(&mut self) {
        self.breakpoints.clear();
    }

    /// Watches an address, replacing any watch already on it.
    pub fn watch(&mut self, watchpoint: Watchpoint) {
        self.watchpoints
            .retain(|existing| existing.address != watchpoint.address);
        self.watchpoints.push(watchpoint);
        self.watchpoints.sort_by_key(|watch| watch.address);
    }

    pub fn unwatch(&mut self, address: u64) {
        self.watchpoints.retain(|watch| watch.address != address);
    }

    #[must_use]
    pub fn watchpoints(&self) -> &[Watchpoint] {
        &self.watchpoints
    }

    // ----- running ----------------------------------------------------------

    /// Carries out one instruction, whatever it is.
    ///
    /// A breakpoint on the instruction about to run does not stop a single
    /// step: the reader who pressed the button is standing on it, and refusing
    /// to move would be a trap they could not get out of.
    pub fn step_one(&mut self) -> Option<&Stop> {
        if !self.can_continue() {
            return self.stop.as_ref();
        }
        self.stop = None;
        self.execute_one();
        self.stop.as_ref()
    }

    /// Carries out one step of the shape the reader asked for.
    pub fn step(&mut self, step: Step) -> Option<&Stop> {
        match step {
            Step::Into => self.step_one(),
            Step::Over => self.step_over(),
            Step::Out => self.run_until_depth(self.depth - 1),
            Step::Back => {
                self.step_back();
                self.stop.as_ref()
            }
        }
    }

    /// One instruction, and the whole of any call it makes.
    ///
    /// Not a breakpoint on the next address, which a recursive function would
    /// hit at the wrong depth, but the call depth itself: the run carries on
    /// until it is back where it was.
    fn step_over(&mut self) -> Option<&Stop> {
        let before = self.depth;
        self.step_one();
        if self.depth > before && self.can_continue() {
            return self.run_until_depth(before);
        }
        self.stop.as_ref()
    }

    /// Runs until the call depth is back down to `target`, or something stops
    /// the run first.
    fn run_until_depth(&mut self, target: i64) -> Option<&Stop> {
        if !self.can_continue() {
            return self.stop.as_ref();
        }
        self.stop = None;
        for _ in 0..RUN_BUDGET {
            if self.paused_by_breakpoint() {
                return self.stop.as_ref();
            }
            self.execute_one();
            if self.stop.is_some() {
                return self.stop.as_ref();
            }
            if self.depth <= target {
                self.stop = Some(Stop::Paused);
                return self.stop.as_ref();
            }
        }
        self.stop = Some(Stop::OutOfBudget);
        self.stop.as_ref()
    }

    /// Runs until a breakpoint, a stop, or the budget for one press.
    pub fn run(&mut self) -> Option<&Stop> {
        if !self.can_continue() {
            return self.stop.as_ref();
        }
        self.stop = None;
        // The first instruction is carried out whatever is on it, so that
        // pressing "run" while stopped on a breakpoint moves.
        self.execute_one();
        for _ in 1..RUN_BUDGET {
            if self.stop.is_some() {
                return self.stop.as_ref();
            }
            if self.paused_by_breakpoint() {
                return self.stop.as_ref();
            }
            self.execute_one();
        }
        if self.stop.is_none() {
            self.stop = Some(Stop::OutOfBudget);
        }
        self.stop.as_ref()
    }

    /// Runs until the instruction pointer reaches an address, a breakpoint, or
    /// something that stops the run. What "run to cursor" is.
    pub fn run_to(&mut self, address: u64) -> Option<&Stop> {
        if !self.can_continue() {
            return self.stop.as_ref();
        }
        self.stop = None;
        for turn in 0..RUN_BUDGET {
            if turn > 0 {
                if self.registers.instruction_pointer == address {
                    self.stop = Some(Stop::Paused);
                    return self.stop.as_ref();
                }
                if self.paused_by_breakpoint() {
                    return self.stop.as_ref();
                }
            }
            self.execute_one();
            if self.stop.is_some() {
                return self.stop.as_ref();
            }
        }
        self.stop = Some(Stop::OutOfBudget);
        self.stop.as_ref()
    }

    /// Whether a breakpoint on the instruction about to run stops it.
    ///
    /// The condition is asked here, of the state as it stands before the
    /// instruction runs — which is the state the reader would be looking at if
    /// it stopped, and so the one the condition should be about.
    fn paused_by_breakpoint(&mut self) -> bool {
        let address = self.registers.instruction_pointer;
        // Split apart so the condition can read the registers and the memory
        // while the breakpoint counts its own passes.
        let Some(mut breakpoint) = self.breakpoints.remove(&address) else {
            return false;
        };
        let stops = breakpoint.stops(&self.registers, &self.memory);
        self.breakpoints.insert(address, breakpoint);
        if stops {
            self.stop = Some(Stop::Breakpoint { address });
        }
        stops
    }

    /// Decodes and carries out the instruction at the instruction pointer,
    /// recording it and setting [`Self::stop`] if anything ended the run.
    fn execute_one(&mut self) {
        let at = self.registers.instruction_pointer;
        if at == RETURN_SENTINEL {
            self.stop = Some(Stop::Finished);
            return;
        }
        let Some((instruction, text)) = self.decode_at(at) else {
            return;
        };
        let touched = self.watch_touched(&instruction, at);
        // The state as it stands, before anything changes it. Kept whatever
        // the instruction turns out to do — including faulting, which is a
        // thing a reader most wants to be able to step back out of.
        let before = self.registers.clone();
        let depth_before = self.depth;
        self.memory.start_recording();
        let mut cpu = Cpu {
            registers: &mut self.registers,
            memory: &mut self.memory,
            bitness: self.bitness,
        };
        let outcome = cpu.execute(&instruction, &text);
        let overwritten = self.memory.take_recording();
        self.executed = self.executed.saturating_add(1);
        self.record(at, text.clone());
        let mut frames = FrameChange::None;
        match outcome {
            Ok(Outcome::Continued) => {}
            Ok(Outcome::Called { returns_to }) => {
                self.depth += 1;
                frames = FrameChange::Pushed;
                // Bounded, like the trace: a runaway recursion must not turn
                // into a list as long as the run itself.
                if self.frames.len() < TRACE_LENGTH {
                    self.frames.push(Frame {
                        called_from: at,
                        entered: self.registers.instruction_pointer,
                        returns_to,
                        stack_pointer: self.registers.stack_pointer(),
                    });
                }
            }
            Ok(Outcome::Returned) => {
                self.depth -= 1;
                if let Some(frame) = self.frames.pop() {
                    frames = FrameChange::Popped(Box::new(frame));
                }
            }
            Err(refusal) => {
                self.stop = Some(match refusal {
                    Refusal::Unsupported { text } => Stop::Unsupported {
                        at,
                        instruction: text,
                    },
                    Refusal::SystemCall { text } => Stop::SystemCall {
                        at,
                        instruction: text,
                        call: system::SystemCall::capture(
                            self.format,
                            self.bitness,
                            &self.registers,
                        ),
                    },
                    Refusal::Fault(fault) => Stop::Fault { at, fault },
                    Refusal::DivideError => Stop::DivideError { at },
                    Refusal::Halted { text } => Stop::Halted {
                        at,
                        instruction: text,
                    },
                });
                // The instruction did not finish, so the pointer goes back to
                // it: the reader is stopped *on* what failed, not after it.
                self.registers.instruction_pointer = at;
                self.remember_how_to_undo(before, overwritten, depth_before, FrameChange::None);
                return;
            }
        }
        self.remember_how_to_undo(before, overwritten, depth_before, frames);
        if let Some(stop) = touched {
            self.stop = Some(stop);
        }
    }

    /// Files away what one instruction changed, dropping the oldest when the
    /// record is full.
    fn remember_how_to_undo(
        &mut self,
        registers: Registers,
        memory: Vec<(u64, u8)>,
        depth: i64,
        frames: FrameChange,
    ) {
        if self.rewind.len() >= REWIND_LENGTH {
            self.rewind.pop_front();
        }
        self.rewind.push_back(Undo {
            registers,
            memory,
            depth,
            frames,
        });
    }

    /// Reads and decodes the instruction at an address, reporting why not.
    fn decode_at(&mut self, at: u64) -> Option<(iced_x86::Instruction, String)> {
        // As many bytes as an instruction can be, and as few as the mapping
        // allows: an instruction at the very end of a section is still
        // decodable, and asking for fifteen bytes there would fault.
        let mut window = [0_u8; 16];
        let mut length = 0_usize;
        for slot in 0..15_u64 {
            let mut byte = [0_u8; 1];
            if self.memory.fetch(at.wrapping_add(slot), &mut byte).is_err() {
                break;
            }
            window[length] = byte[0];
            length += 1;
        }
        if length == 0 {
            // Nothing at all could be fetched. Landing in a page the file does
            // not map is what a call into a library looks like from here, and
            // saying so is more use than "read fault at 0x…".
            self.stop = Some(if at < memory::PAGE {
                Stop::UnresolvedCall { at }
            } else if self.memory.region_at(at).is_none() {
                Stop::LeftTheImage { at }
            } else {
                Stop::Fault {
                    at,
                    fault: Fault::Protection {
                        address: at,
                        needed: Access::Execute,
                        granted: self
                            .memory
                            .region_at(at)
                            .map(|region| region.permissions)
                            .unwrap_or_default(),
                    },
                }
            });
            return None;
        }
        let mut decoder =
            Decoder::with_ip(self.bitness, &window[..length], at, DecoderOptions::NONE);
        let instruction = decoder.decode();
        if instruction.is_invalid() {
            self.stop = Some(Stop::Undecodable { at });
            return None;
        }
        let mut text = String::new();
        GasFormatter::new().format(&instruction, &mut text);
        Some((instruction, text))
    }

    /// Whether the instruction about to run touches a watched address, and
    /// what to report if it does.
    ///
    /// Worked out before the instruction runs, from the operands it names, so
    /// that a write is reported with the run stopped just after it — the point
    /// at which the new value can be looked at.
    fn watch_touched(&self, instruction: &iced_x86::Instruction, at: u64) -> Option<Stop> {
        if self.watchpoints.is_empty() {
            return None;
        }
        let size = instruction.memory_size().size() as u64;
        if size == 0 {
            return None;
        }
        let address = instruction
            .virtual_address(0, 0, |register, _element, _size| {
                Some(self.registers.get(register))
            })
            .or_else(|| {
                instruction.virtual_address(1, 0, |register, _element, _size| {
                    Some(self.registers.get(register))
                })
            })?;
        let writes = (0..instruction.op_count())
            .any(|operand| instruction.op_kind(operand) == iced_x86::OpKind::Memory)
            && instruction_writes_memory(instruction);
        self.watchpoints.iter().find_map(|watch| {
            if !watch.touched_by(address, size) {
                return None;
            }
            let access = if writes { Access::Write } else { Access::Read };
            let wanted = if writes {
                watch.on_write
            } else {
                watch.on_read
            };
            wanted.then_some(Stop::Watchpoint {
                address: watch.address,
                at,
                access,
            })
        })
    }

    /// Adds one instruction to the ring, dropping the oldest when it is full.
    fn record(&mut self, address: u64, text: String) {
        if self.trace.len() >= TRACE_LENGTH {
            self.trace.pop_front();
        }
        self.trace.push_back(Executed {
            address,
            text,
            ordinal: self.executed,
        });
    }

    /// Sets the registers a function's arguments go in, so a reader can run
    /// one function rather than the whole program.
    ///
    /// The convention is the platform's own, and naming it is the caller's
    /// job: the same registers mean different arguments on Linux and Windows,
    /// and guessing which is a good way to be confidently wrong.
    pub fn call_function(&mut self, address: u64, arguments: &[u64], convention: Convention) {
        self.restart();
        self.registers.instruction_pointer = address;
        for (slot, value) in convention.argument_registers().iter().zip(arguments) {
            self.registers.set(*slot, *value);
        }
        self.depth = 0;
        self.frames.clear();
        self.rewind.clear();
    }
}

/// Which platform's rule says where a function's arguments are.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Convention {
    /// The System V rule, used by Linux and macOS: `rdi`, `rsi`, `rdx`, `rcx`,
    /// `r8`, `r9`.
    #[default]
    SystemV,
    /// The Microsoft rule: `rcx`, `rdx`, `r8`, `r9`.
    Microsoft,
}

impl Convention {
    /// The registers arguments arrive in, in order.
    #[must_use]
    pub const fn argument_registers(self) -> &'static [iced_x86::Register] {
        use iced_x86::Register::{R8, R9, RCX, RDI, RDX, RSI};
        match self {
            Self::SystemV => &[RDI, RSI, RDX, RCX, R8, R9],
            Self::Microsoft => &[RCX, RDX, R8, R9],
        }
    }

    /// What to call it on screen.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::SystemV => "System V",
            Self::Microsoft => "Microsoft",
        }
    }
}

/// Whether an instruction's memory operand is one it writes to.
///
/// Read from the instruction's own operand kinds rather than from a list of
/// mnemonics: iced-x86 knows which operand a mnemonic writes, and a list
/// maintained here would drift from it.
fn instruction_writes_memory(instruction: &iced_x86::Instruction) -> bool {
    use iced_x86::OpAccess;
    let info = iced_x86::InstructionInfoFactory::new()
        .info(instruction)
        .clone();
    info.used_memory().iter().any(|used| {
        matches!(
            used.access(),
            OpAccess::Write | OpAccess::ReadWrite | OpAccess::CondWrite | OpAccess::ReadCondWrite
        )
    })
}

pub use memory::{Access as MemoryAccess, Fault as MemoryFault};
pub use registers::Flag as CpuFlag;

#[cfg(test)]
// The tests assemble code, and the assembler takes an address as a signed
// immediate: `mov rdi, DATA as i64` is how one says "put this address in this
// register". Writing each of them through `i64::try_from(…).unwrap()` would
// bury what the test is about under the conversion.
#[expect(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    reason = "addresses are given to the assembler as signed immediates"
)]
mod tests {
    use iced_x86::{
        Register,
        code_asm::{
            CodeAssembler, byte_ptr, dword_ptr, ptr, qword_ptr, rax, rbx, rcx, rdi, rdx, rsi, xmm0,
            xmm1, xmmword_ptr,
        },
    };

    use super::{
        Machine, Stop, Watchpoint,
        memory::{Access, Fault, Memory, PAGE},
        registers::Flag,
    };
    use crate::{Architecture, Permissions, Section};

    /// Where the code the tests assemble is mapped.
    const CODE: u64 = 0x40_0000;
    /// Where their writable page is.
    const DATA: u64 = 0x50_0000;

    /// Builds a machine holding one page of code and one of writable data.
    ///
    /// The code comes from the same assembler the patch editor uses, so a test
    /// says what it means — `mov rax, 1` — rather than a row of opcode bytes
    /// that the next reader would have to decode by hand.
    fn machine(assemble: impl FnOnce(&mut CodeAssembler)) -> Machine {
        let mut assembler = CodeAssembler::new(64).expect("64-bit assembler");
        assemble(&mut assembler);
        let code = assembler.assemble(CODE).expect("assembles");
        let mut file = vec![0_u8; PAGE as usize];
        file[..code.len()].copy_from_slice(&code);
        let sections = vec![
            Section {
                name: String::from(".text"),
                virtual_address: CODE,
                file_offset: 0,
                virtual_size: PAGE,
                file_size: code.len() as u64,
                permissions: Permissions {
                    read: true,
                    write: false,
                    execute: true,
                },
                entropy: None,
            },
            Section {
                name: String::from(".data"),
                virtual_address: DATA,
                file_offset: 0,
                virtual_size: PAGE,
                file_size: 0,
                permissions: Permissions {
                    read: true,
                    write: true,
                    execute: false,
                },
                entropy: None,
            },
        ];
        let memory = Memory::from_sections(file.into(), &sections);
        Machine::over(memory, Architecture::X86_64, CODE)
    }

    /// The address of each assembled instruction, so a test can name one
    /// without counting opcode bytes by hand.
    fn addresses(assemble: impl FnOnce(&mut CodeAssembler)) -> Vec<u64> {
        let mut assembler = CodeAssembler::new(64).expect("64-bit assembler");
        assemble(&mut assembler);
        let code = assembler.assemble(CODE).expect("assembles");
        let mut decoder =
            iced_x86::Decoder::with_ip(64, &code, CODE, iced_x86::DecoderOptions::NONE);
        let mut found = Vec::new();
        while decoder.can_decode() {
            found.push(decoder.ip());
            let _ = decoder.decode();
        }
        found
    }

    /// Runs a whole program and returns the machine it left behind.
    fn run(assemble: impl FnOnce(&mut CodeAssembler)) -> Machine {
        let mut machine = machine(assemble);
        machine.run();
        machine
    }

    #[test]
    fn a_program_that_returns_finishes() {
        let machine = run(|code| {
            code.mov(rax, 7_i64).unwrap();
            code.ret().unwrap();
        });
        assert_eq!(machine.registers.get(Register::RAX), 7);
        assert_eq!(machine.stop(), Some(&Stop::Finished));
    }

    #[test]
    fn xmm_moves_copy_sixteen_bytes_and_rewind_exactly() {
        let mut machine = machine(|code| {
            code.mov(rsi, DATA as i64).unwrap();
            code.mov(rdi, (DATA + 32) as i64).unwrap();
            code.movdqu(xmm0, xmmword_ptr(rsi)).unwrap();
            code.pxor(xmm0, xmm1).unwrap();
            code.movups(xmmword_ptr(rdi), xmm0).unwrap();
            code.ret().unwrap();
        });
        let source = *b"0123456789abcdef";
        for (offset, byte) in source.into_iter().enumerate() {
            assert!(machine.memory.poke(DATA + offset as u64, byte));
        }

        // Get to the SIMD move, then take it back: vector state belongs to
        // the same exact rewind record as general registers and memory.
        machine.step_one();
        machine.step_one();
        machine.step_one();
        assert_eq!(
            machine.registers.xmm(Register::XMM0),
            Some(u128::from_le_bytes(source))
        );
        assert!(machine.step_back());
        assert_eq!(machine.registers.xmm(Register::XMM0), Some(0));

        machine.run();
        assert_eq!(machine.stop(), Some(&Stop::Finished));
        assert_eq!(
            machine.registers.xmm(Register::XMM0),
            Some(u128::from_le_bytes(source))
        );
        let copied: Vec<u8> = (0..16)
            .map(|offset| machine.memory.peek(DATA + 32 + offset).unwrap())
            .collect();
        assert_eq!(copied, source);
    }

    #[test]
    fn a_thirty_two_bit_write_clears_the_top_half() {
        // The rule that catches everyone: `mov eax, …` is not a partial write.
        let machine = run(|code| {
            code.mov(rax, -1_i64).unwrap();
            code.mov(iced_x86::code_asm::eax, 1_i32).unwrap();
            code.ret().unwrap();
        });
        assert_eq!(machine.registers.get(Register::RAX), 1);
    }

    #[test]
    fn an_eight_bit_write_leaves_the_rest_alone() {
        let machine = run(|code| {
            code.mov(rax, -1_i64).unwrap();
            code.mov(iced_x86::code_asm::al, 0_i32).unwrap();
            code.ret().unwrap();
        });
        assert_eq!(machine.registers.get(Register::RAX), 0xffff_ffff_ffff_ff00);
    }

    #[test]
    fn addition_settles_the_flags_it_should() {
        let machine = run(|code| {
            code.mov(rax, -1_i64).unwrap();
            code.add(rax, 1_i32).unwrap();
            code.ret().unwrap();
        });
        assert_eq!(machine.registers.get(Register::RAX), 0);
        assert!(machine.registers.flag(Flag::Zero), "the result is zero");
        assert!(machine.registers.flag(Flag::Carry), "it wrapped");
        assert!(
            !machine.registers.flag(Flag::Overflow),
            "unsigned wrap only"
        );
    }

    #[test]
    fn a_signed_overflow_is_not_a_carry() {
        let machine = run(|code| {
            code.mov(rax, i64::from(i32::MAX)).unwrap();
            code.add(iced_x86::code_asm::eax, 1_i32).unwrap();
            code.ret().unwrap();
        });
        assert!(machine.registers.flag(Flag::Overflow), "signed overflow");
        assert!(!machine.registers.flag(Flag::Carry), "no unsigned carry");
        assert!(
            machine.registers.flag(Flag::Sign),
            "the result reads negative"
        );
    }

    #[test]
    fn increment_leaves_the_carry_alone() {
        let machine = run(|code| {
            code.stc().unwrap();
            code.mov(rax, 1_i64).unwrap();
            code.inc(rax).unwrap();
            code.ret().unwrap();
        });
        assert_eq!(machine.registers.get(Register::RAX), 2);
        assert!(machine.registers.flag(Flag::Carry), "inc does not touch it");
    }

    #[test]
    fn a_conditional_branch_is_taken_on_the_flags() {
        let machine = run(|code| {
            let taken = code.create_label();
            code.mov(rax, 1_i64).unwrap();
            code.cmp(rax, 1_i32).unwrap();
            code.je(taken).unwrap();
            code.mov(rax, 0_i64).unwrap();
            code.set_label(&mut { taken }).unwrap();
            code.ret().unwrap();
        });
        assert_eq!(
            machine.registers.get(Register::RAX),
            1,
            "the branch was taken"
        );
    }

    #[test]
    fn a_loop_runs_the_number_of_times_it_says() {
        let machine = run(|code| {
            let top = code.create_label();
            code.mov(rcx, 10_i64).unwrap();
            code.xor(rax, rax).unwrap();
            code.set_label(&mut { top }).unwrap();
            code.add(rax, 3_i32).unwrap();
            code.dec(rcx).unwrap();
            code.jne(top).unwrap();
            code.ret().unwrap();
        });
        assert_eq!(machine.registers.get(Register::RAX), 30);
        assert_eq!(machine.registers.get(Register::RCX), 0);
    }

    #[test]
    fn a_call_and_its_return_balance_the_stack() {
        let mut machine = machine(|code| {
            let callee = code.create_label();
            code.call(callee).unwrap();
            code.ret().unwrap();
            code.set_label(&mut { callee }).unwrap();
            code.mov(rax, 5_i64).unwrap();
            code.ret().unwrap();
        });
        let before = machine.registers.stack_pointer();
        machine.step_one();
        assert_eq!(machine.depth(), 1, "inside the call");
        assert_eq!(
            machine.registers.stack_pointer(),
            before - 8,
            "the return address is on the stack"
        );
        machine.run();
        assert_eq!(machine.registers.get(Register::RAX), 5);
        assert_eq!(machine.stop(), Some(&Stop::Finished));
        // Eight bytes above where it started, because the outermost `ret` took
        // the sentinel off as well: that is what finishing looks like.
        assert_eq!(machine.registers.stack_pointer(), before + 8);
    }

    #[test]
    fn stepping_over_a_call_runs_the_whole_of_it() {
        let mut machine = machine(|code| {
            let callee = code.create_label();
            code.call(callee).unwrap();
            code.mov(rbx, 1_i64).unwrap();
            code.ret().unwrap();
            code.set_label(&mut { callee }).unwrap();
            code.mov(rax, 5_i64).unwrap();
            code.ret().unwrap();
        });
        machine.step(super::Step::Over);
        assert_eq!(machine.registers.get(Register::RAX), 5, "the call ran");
        assert_eq!(machine.registers.get(Register::RBX), 0, "and only the call");
        assert_eq!(machine.depth(), 0, "back where it started");
    }

    #[test]
    fn stepping_out_leaves_the_function() {
        let mut machine = machine(|code| {
            let callee = code.create_label();
            code.call(callee).unwrap();
            code.mov(rbx, 1_i64).unwrap();
            code.ret().unwrap();
            code.set_label(&mut { callee }).unwrap();
            code.mov(rax, 5_i64).unwrap();
            code.mov(rcx, 6_i64).unwrap();
            code.ret().unwrap();
        });
        machine.step_one();
        machine.step_one();
        assert_eq!(machine.depth(), 1);
        machine.step(super::Step::Out);
        assert_eq!(machine.depth(), 0);
        assert_eq!(
            machine.registers.get(Register::RCX),
            6,
            "the rest of it ran"
        );
        assert_eq!(machine.registers.get(Register::RBX), 0, "and stopped there");
    }

    #[test]
    fn an_indirect_call_goes_where_the_register_points() {
        // The question a static reading cannot answer, and the reason the
        // emulator exists.
        let mut machine = machine(|code| {
            let callee = code.create_label();
            code.lea(rax, ptr(callee)).unwrap();
            code.call(rax).unwrap();
            code.ret().unwrap();
            code.set_label(&mut { callee }).unwrap();
            code.mov(rbx, 99_i64).unwrap();
            code.ret().unwrap();
        });
        machine.run();
        assert_eq!(machine.registers.get(Register::RBX), 99);
        assert_eq!(machine.stop(), Some(&Stop::Finished));
    }

    #[test]
    fn a_breakpoint_stops_the_run_before_the_instruction() {
        let program = |code: &mut CodeAssembler| {
            code.mov(rax, 1_i64).unwrap();
            code.mov(rax, 2_i64).unwrap();
            code.mov(rax, 3_i64).unwrap();
            code.ret().unwrap();
        };
        let mut machine = machine(program);
        let second = addresses(program)[1];
        assert!(machine.toggle_breakpoint(second));
        machine.run();
        assert_eq!(machine.stop(), Some(&Stop::Breakpoint { address: second }));
        assert_eq!(
            machine.registers.get(Register::RAX),
            1,
            "stopped before the instruction ran, not after"
        );
    }

    #[test]
    fn a_run_carries_on_from_the_breakpoint_it_stopped_on() {
        let program = |code: &mut CodeAssembler| {
            code.mov(rax, 1_i64).unwrap();
            code.mov(rax, 2_i64).unwrap();
            code.ret().unwrap();
        };
        let mut machine = machine(program);
        machine.toggle_breakpoint(addresses(program)[1]);
        machine.run();
        machine.run();
        assert_eq!(machine.registers.get(Register::RAX), 2);
        assert_eq!(machine.stop(), Some(&Stop::Finished));
    }

    #[test]
    fn memory_written_by_the_run_reads_back() {
        let machine = run(|code| {
            code.mov(rax, 0x1234_5678_i64).unwrap();
            code.mov(rdi, DATA as i64).unwrap();
            code.mov(qword_ptr(rdi), rax).unwrap();
            code.mov(rbx, qword_ptr(rdi)).unwrap();
            code.ret().unwrap();
        });
        assert_eq!(machine.registers.get(Register::RBX), 0x1234_5678);
        assert_eq!(machine.memory.peek(DATA), Some(0x78));
    }

    #[test]
    fn reading_an_unmapped_address_stops_the_run() {
        let machine = run(|code| {
            code.xor(rdi, rdi).unwrap();
            code.mov(rax, qword_ptr(rdi)).unwrap();
            code.ret().unwrap();
        });
        assert!(matches!(
            machine.stop(),
            Some(Stop::Fault {
                fault: Fault::Unmapped { address: 0 },
                ..
            })
        ));
        assert!(!machine.can_continue(), "a fault is not resumable");
    }

    #[test]
    fn writing_to_the_code_stops_the_run() {
        let machine = run(|code| {
            code.mov(rdi, CODE as i64).unwrap();
            code.mov(byte_ptr(rdi), 0x90_i32).unwrap();
            code.ret().unwrap();
        });
        assert!(
            matches!(
                machine.stop(),
                Some(Stop::Fault {
                    fault: Fault::Protection {
                        needed: Access::Write,
                        ..
                    },
                    ..
                })
            ),
            "got {:?}",
            machine.stop()
        );
    }

    #[test]
    fn the_instruction_pointer_stays_on_what_faulted() {
        let mut machine = machine(|code| {
            code.xor(rdi, rdi).unwrap();
            code.mov(rax, qword_ptr(rdi)).unwrap();
            code.ret().unwrap();
        });
        machine.run();
        assert_eq!(
            machine.instruction_pointer(),
            addresses(|code: &mut CodeAssembler| {
                code.xor(rdi, rdi).unwrap();
                code.mov(rax, qword_ptr(rdi)).unwrap();
                code.ret().unwrap();
            })[1],
            "stopped on the load, not after it"
        );
    }

    #[test]
    fn a_system_call_stops_the_run_by_name() {
        let machine = run(|code| {
            code.mov(rax, 60_i64).unwrap();
            code.syscall().unwrap();
        });
        assert!(
            matches!(machine.stop(), Some(Stop::SystemCall { .. })),
            "got {:?}",
            machine.stop()
        );
    }

    #[test]
    fn an_unimplemented_instruction_stops_rather_than_being_skipped() {
        let machine = run(|code| {
            code.rdrand(rax).unwrap();
            code.ret().unwrap();
        });
        assert!(
            matches!(machine.stop(), Some(Stop::Unsupported { .. })),
            "got {:?}",
            machine.stop()
        );
    }

    #[test]
    fn dividing_by_zero_stops_the_run() {
        let machine = run(|code| {
            code.mov(rax, 10_i64).unwrap();
            code.xor(rdx, rdx).unwrap();
            code.xor(rcx, rcx).unwrap();
            code.div(rcx).unwrap();
            code.ret().unwrap();
        });
        assert!(matches!(machine.stop(), Some(Stop::DivideError { .. })));
    }

    #[test]
    fn signed_and_unsigned_division_disagree_as_they_should() {
        let signed = run(|code| {
            code.mov(rax, -7_i64).unwrap();
            code.cqo().unwrap();
            code.mov(rcx, 2_i64).unwrap();
            code.idiv(rcx).unwrap();
            code.ret().unwrap();
        });
        assert_eq!(signed.registers.get(Register::RAX) as i64, -3);
        assert_eq!(signed.registers.get(Register::RDX) as i64, -1);
    }

    #[test]
    fn multiplication_fills_both_halves() {
        let machine = run(|code| {
            code.mov(rax, -1_i64).unwrap();
            code.mov(rcx, 2_i64).unwrap();
            code.mul(rcx).unwrap();
            code.ret().unwrap();
        });
        assert_eq!(machine.registers.get(Register::RAX), u64::MAX - 1);
        assert_eq!(machine.registers.get(Register::RDX), 1, "the high half");
        assert!(machine.registers.flag(Flag::Carry), "it did not fit in one");
    }

    #[test]
    fn a_shift_by_zero_changes_no_flag() {
        let machine = run(|code| {
            code.stc().unwrap();
            code.mov(rax, 1_i64).unwrap();
            code.mov(rcx, 0_i64).unwrap();
            code.shl(rax, iced_x86::code_asm::cl).unwrap();
            code.ret().unwrap();
        });
        assert_eq!(machine.registers.get(Register::RAX), 1);
        assert!(machine.registers.flag(Flag::Carry), "left exactly as found");
    }

    #[test]
    fn a_repeated_store_fills_the_range() {
        let machine = run(|code| {
            code.mov(rdi, DATA as i64).unwrap();
            code.mov(rcx, 8_i64).unwrap();
            code.mov(iced_x86::code_asm::al, 0x41_i32).unwrap();
            code.rep().stosb().unwrap();
            code.ret().unwrap();
        });
        assert_eq!(machine.stop(), Some(&Stop::Finished));
        for step in 0..8 {
            assert_eq!(machine.memory.peek(DATA + step), Some(0x41), "byte {step}");
        }
        assert_eq!(machine.memory.peek(DATA + 8), Some(0), "and no further");
        assert_eq!(machine.registers.get(Register::RCX), 0);
    }

    #[test]
    fn a_repeated_move_is_stepped_one_iteration_at_a_time() {
        // What a debugger's single step does on a real processor, and what a
        // reader watching `rcx` expects to see.
        let mut machine = machine(|code| {
            code.mov(rsi, CODE as i64).unwrap();
            code.mov(rdi, DATA as i64).unwrap();
            code.mov(rcx, 4_i64).unwrap();
            code.rep().movsb().unwrap();
            code.ret().unwrap();
        });
        for _ in 0..3 {
            machine.step_one();
        }
        let at = machine.instruction_pointer();
        machine.step_one();
        assert_eq!(machine.registers.get(Register::RCX), 3, "one iteration");
        assert_eq!(machine.instruction_pointer(), at, "still on the same one");
    }

    #[test]
    fn a_watchpoint_reports_the_write_that_touched_it() {
        let mut machine = machine(|code| {
            code.mov(rdi, DATA as i64).unwrap();
            code.mov(rax, 1_i64).unwrap();
            code.mov(dword_ptr(rdi), iced_x86::code_asm::eax).unwrap();
            code.ret().unwrap();
        });
        machine.watch(Watchpoint {
            address: DATA + 2,
            size: 1,
            on_read: false,
            on_write: true,
        });
        machine.run();
        assert!(
            matches!(
                machine.stop(),
                Some(Stop::Watchpoint {
                    access: Access::Write,
                    ..
                })
            ),
            "got {:?}",
            machine.stop()
        );
        assert_eq!(machine.memory.peek(DATA), Some(1), "the write happened");
    }

    #[test]
    fn restarting_forgets_what_the_last_run_wrote() {
        let mut machine = machine(|code| {
            code.mov(rdi, DATA as i64).unwrap();
            code.mov(byte_ptr(rdi), 0x41_i32).unwrap();
            code.ret().unwrap();
        });
        machine.run();
        assert_eq!(machine.memory.peek(DATA), Some(0x41));
        machine.restart();
        assert_eq!(
            machine.memory.peek(DATA),
            Some(0),
            "back to the file's bytes"
        );
        assert_eq!(machine.executed(), 0);
        assert_eq!(machine.instruction_pointer(), CODE);
    }

    #[test]
    fn a_breakpoint_survives_a_restart_and_the_trace_does_not() {
        let mut machine = machine(|code| {
            code.mov(rax, 1_i64).unwrap();
            code.ret().unwrap();
        });
        machine.toggle_breakpoint(CODE);
        machine.run();
        assert!(machine.trace().count() > 0);
        machine.restart();
        assert!(machine.has_breakpoint(CODE), "the reader's, not the run's");
        assert_eq!(machine.trace().count(), 0);
    }

    #[test]
    fn the_trace_keeps_what_ran_in_order() {
        let machine = run(|code| {
            code.mov(rax, 1_i64).unwrap();
            code.mov(rbx, 2_i64).unwrap();
            code.ret().unwrap();
        });
        let executed: Vec<u64> = machine.trace().map(|entry| entry.address).collect();
        assert_eq!(executed.first(), Some(&CODE));
        assert_eq!(executed.len(), 3, "two moves and the return");
        assert_eq!(machine.executed(), 3);
    }

    #[test]
    fn the_call_stack_holds_the_calls_not_yet_returned_from() {
        let mut machine = machine(|code| {
            let outer = code.create_label();
            let inner = code.create_label();
            code.call(outer).unwrap();
            code.ret().unwrap();
            code.set_label(&mut { outer }).unwrap();
            code.call(inner).unwrap();
            code.ret().unwrap();
            code.set_label(&mut { inner }).unwrap();
            code.mov(rax, 1_i64).unwrap();
            code.ret().unwrap();
        });
        // Into the outer call, then into the inner one.
        machine.step_one();
        machine.step_one();
        let frames = machine.frames();
        assert_eq!(frames.len(), 2, "two calls deep");
        assert_eq!(frames[0].called_from, CODE, "the outermost call");
        assert_eq!(frames[1].entered, frames[1].entered, "and the one it made");
        assert!(
            frames[1].stack_pointer < frames[0].stack_pointer,
            "the stack grows downwards as calls are made"
        );
        machine.run();
        assert!(machine.frames().is_empty(), "all of them returned");
    }

    /// A condition is what makes a breakpoint in a loop worth setting.
    #[test]
    fn a_conditional_breakpoint_stops_on_the_turn_it_names() {
        let program = |code: &mut CodeAssembler| {
            let top = code.create_label();
            code.mov(rcx, 10_i64).unwrap();
            code.set_label(&mut { top }).unwrap();
            code.dec(rcx).unwrap();
            code.jne(top).unwrap();
            code.ret().unwrap();
        };
        let mut machine = machine(program);
        let top = addresses(program)[1];
        machine.toggle_breakpoint(top);
        machine
            .breakpoint_mut(top)
            .expect("just set")
            .set_condition("rcx == 4")
            .expect("a condition that parses");

        machine.run();
        assert_eq!(machine.stop(), Some(&Stop::Breakpoint { address: top }));
        assert_eq!(
            machine.registers.get(Register::RCX),
            4,
            "stopped on the turn the condition names, not on the first"
        );
    }

    /// A condition that never holds never stops, and the run finishes.
    #[test]
    fn a_condition_that_never_holds_never_stops_the_run() {
        let program = |code: &mut CodeAssembler| {
            let top = code.create_label();
            code.mov(rcx, 5_i64).unwrap();
            code.set_label(&mut { top }).unwrap();
            code.dec(rcx).unwrap();
            code.jne(top).unwrap();
            code.ret().unwrap();
        };
        let mut machine = machine(program);
        let top = addresses(program)[1];
        machine.toggle_breakpoint(top);
        machine
            .breakpoint_mut(top)
            .expect("just set")
            .set_condition("rcx == 99")
            .expect("parses");

        machine.run();
        assert_eq!(machine.stop(), Some(&Stop::Finished));
    }

    /// A pass count lets that many qualifying passes by first.
    #[test]
    fn a_pass_count_lets_that_many_go_by_before_stopping() {
        let program = |code: &mut CodeAssembler| {
            let top = code.create_label();
            code.mov(rcx, 10_i64).unwrap();
            code.set_label(&mut { top }).unwrap();
            code.dec(rcx).unwrap();
            code.jne(top).unwrap();
            code.ret().unwrap();
        };
        let mut machine = machine(program);
        let top = addresses(program)[1];
        machine.toggle_breakpoint(top);
        machine.breakpoint_mut(top).expect("just set").skip = 3;

        machine.run();
        assert_eq!(machine.stop(), Some(&Stop::Breakpoint { address: top }));
        assert_eq!(
            machine.registers.get(Register::RCX),
            7,
            "three turns went by, and it stopped on the fourth"
        );
        assert_eq!(machine.breakpoint(top).expect("still there").passes, 4);
    }

    /// A breakpoint turned off keeps what was written on it.
    #[test]
    fn a_breakpoint_turned_off_stops_nothing_and_forgets_nothing() {
        let mut machine = machine(|code| {
            code.mov(rax, 1_i64).unwrap();
            code.ret().unwrap();
        });
        machine.toggle_breakpoint(CODE);
        let breakpoint = machine.breakpoint_mut(CODE).expect("just set");
        breakpoint.set_condition("rax == 0").expect("parses");
        breakpoint.enabled = false;

        machine.run();
        assert_eq!(machine.stop(), Some(&Stop::Finished), "it stopped nothing");
        assert_eq!(
            machine.breakpoint(CODE).expect("still there").condition,
            "rax == 0",
            "and it is still what the reader wrote"
        );
    }

    /// A condition that does not parse never replaces one that does.
    #[test]
    fn a_condition_that_does_not_parse_is_refused_and_changes_nothing() {
        let mut machine = machine(|code| {
            code.ret().unwrap();
        });
        machine.toggle_breakpoint(CODE);
        let breakpoint = machine.breakpoint_mut(CODE).expect("just set");
        breakpoint.set_condition("rax == 1").expect("parses");
        assert!(breakpoint.set_condition("rax == ").is_err());
        assert_eq!(
            breakpoint.condition, "rax == 1",
            "what was there is still there"
        );
    }

    /// Restarting starts the counting again.
    #[test]
    fn restarting_starts_the_pass_counts_again() {
        let program = |code: &mut CodeAssembler| {
            let top = code.create_label();
            code.mov(rcx, 4_i64).unwrap();
            code.set_label(&mut { top }).unwrap();
            code.dec(rcx).unwrap();
            code.jne(top).unwrap();
            code.ret().unwrap();
        };
        let mut machine = machine(program);
        let top = addresses(program)[1];
        machine.toggle_breakpoint(top);
        machine.breakpoint_mut(top).expect("just set").skip = 100;
        machine.run();
        assert!(machine.breakpoint(top).expect("there").passes > 0);
        machine.restart();
        assert_eq!(machine.breakpoint(top).expect("there").passes, 0);
    }

    /// Stepping back puts every register where it was.
    #[test]
    fn a_step_back_is_the_state_as_it_was_and_not_a_reading_of_it() {
        let mut machine = machine(|code| {
            code.mov(rax, 1_i64).unwrap();
            code.mov(rbx, 2_i64).unwrap();
            code.add(rax, rbx).unwrap();
            code.ret().unwrap();
        });
        machine.step_one();
        machine.step_one();
        let before = machine.registers.clone();
        let at = machine.instruction_pointer();
        let executed = machine.executed();

        machine.step_one();
        assert_eq!(machine.registers.get(Register::RAX), 3, "the add ran");

        assert!(machine.step_back(), "there is something to go back through");
        assert_eq!(machine.instruction_pointer(), at, "back on the add");
        assert_eq!(machine.executed(), executed, "and it has not run");
        for (name, value) in before.general() {
            assert_eq!(
                machine.registers.get(match name {
                    "rax" => Register::RAX,
                    "rbx" => Register::RBX,
                    "rcx" => Register::RCX,
                    _ => continue,
                }),
                value,
                "{name} is back where it was"
            );
        }
        for flag in Flag::ALL {
            assert_eq!(
                machine.registers.flag(flag),
                before.flag(flag),
                "{} is back where it was",
                flag.short_name()
            );
        }
    }

    /// And puts back every byte it wrote.
    #[test]
    fn a_step_back_un_writes_what_the_instruction_wrote() {
        let mut machine = machine(|code| {
            code.mov(rdi, DATA as i64).unwrap();
            code.mov(qword_ptr(rdi), rax).unwrap();
            code.ret().unwrap();
        });
        // Something already there, so the test is about restoring rather than
        // about clearing.
        machine.memory.poke(DATA, 0x5a);
        machine.registers.set(Register::RAX, 0x1122_3344_5566_7788);
        machine.step_one();
        machine.step_one();
        assert_eq!(machine.memory.peek(DATA), Some(0x88), "the store happened");

        assert!(machine.step_back());
        assert_eq!(
            machine.memory.peek(DATA),
            Some(0x5a),
            "and the byte that was there is there again"
        );
    }

    /// A call and its return are undone as calls and returns.
    #[test]
    fn a_step_back_puts_the_call_stack_back() {
        let mut machine = machine(|code| {
            let callee = code.create_label();
            code.call(callee).unwrap();
            code.ret().unwrap();
            code.set_label(&mut { callee }).unwrap();
            code.mov(rax, 5_i64).unwrap();
            code.ret().unwrap();
        });
        machine.step_one();
        assert_eq!(machine.depth(), 1);
        assert_eq!(machine.frames().len(), 1);
        let stack_pointer = machine.registers.stack_pointer();

        assert!(machine.step_back());
        assert_eq!(machine.depth(), 0, "not in the call any more");
        assert!(machine.frames().is_empty(), "and the frame is gone with it");
        assert_eq!(
            machine.registers.stack_pointer(),
            stack_pointer + 8,
            "the return address is off the stack again"
        );

        // Forward again, to the same place.
        machine.step_one();
        assert_eq!(machine.depth(), 1);
        assert_eq!(machine.registers.stack_pointer(), stack_pointer);
    }

    /// Stepping back out of a fault undoes the fault with the instruction.
    #[test]
    fn a_step_back_out_of_a_fault_is_a_run_that_can_carry_on() {
        let mut machine = machine(|code| {
            code.mov(rax, 7_i64).unwrap();
            code.xor(rdi, rdi).unwrap();
            code.mov(rbx, qword_ptr(rdi)).unwrap();
            code.ret().unwrap();
        });
        machine.run();
        assert!(matches!(machine.stop(), Some(Stop::Fault { .. })));
        assert!(!machine.can_continue(), "a fault is the end of that run");

        assert!(machine.step_back(), "but not the end of the session");
        assert_eq!(machine.stop(), None, "the fault has not happened now");
        assert!(machine.can_continue());
        assert_eq!(
            machine.registers.get(Register::RAX),
            7,
            "the rest still ran"
        );
    }

    /// Going back further than the record reaches says so rather than
    /// pretending, and going back to the start leaves the run at the start.
    #[test]
    fn there_is_nothing_before_the_first_instruction() {
        let mut machine = machine(|code| {
            code.mov(rax, 1_i64).unwrap();
            code.ret().unwrap();
        });
        assert!(!machine.step_back(), "nothing has run yet");
        machine.step_one();
        assert_eq!(machine.rewindable(), 1);
        assert!(machine.step_back());
        assert_eq!(machine.instruction_pointer(), CODE);
        assert_eq!(machine.executed(), 0);
        assert!(!machine.step_back(), "and no further");
    }

    /// A long run keeps only the recent past, and says how much.
    #[test]
    fn the_record_of_how_to_go_back_is_bounded() {
        let mut machine = machine(|code| {
            let top = code.create_label();
            code.mov(rcx, 20_000_i64).unwrap();
            code.set_label(&mut { top }).unwrap();
            code.dec(rcx).unwrap();
            code.jne(top).unwrap();
            code.ret().unwrap();
        });
        machine.run();
        assert_eq!(machine.stop(), Some(&Stop::Finished));
        assert!(machine.executed() > u64::try_from(super::REWIND_LENGTH).unwrap_or(0));
        assert_eq!(
            machine.rewindable(),
            super::REWIND_LENGTH,
            "the recent past, and no more of it than that"
        );
    }

    /// The strongest thing that can be said about going back: going back and
    /// forward again lands on exactly the state that was left.
    ///
    /// Over a loop that computes, branches and writes to memory, so registers,
    /// flags, the stack pointer and stored bytes all have to come back.
    #[test]
    fn going_back_and_forward_again_lands_on_the_same_state() {
        let mut machine = machine(|code| {
            let top = code.create_label();
            code.mov(rcx, 200_i64).unwrap();
            code.mov(rdi, DATA as i64).unwrap();
            code.xor(rax, rax).unwrap();
            code.set_label(&mut { top }).unwrap();
            code.add(rax, rcx).unwrap();
            code.mov(qword_ptr(rdi), rax).unwrap();
            code.add(rdi, 8_i32).unwrap();
            code.dec(rcx).unwrap();
            code.jne(top).unwrap();
            code.ret().unwrap();
        });
        for _ in 0..600 {
            machine.step_one();
        }
        let registers: Vec<(&str, u64)> = machine.registers.general().collect();
        let flags: Vec<bool> = Flag::ALL.map(|flag| machine.registers.flag(flag)).into();
        let pointer = machine.instruction_pointer();
        let executed = machine.executed();
        let memory: Vec<Option<u8>> = (0..256).map(|at| machine.memory.peek(DATA + at)).collect();

        // Not a multiple of the loop's length, so the walk back really lands
        // somewhere else rather than on the same row of the same iteration.
        for _ in 0..203 {
            assert!(machine.step_back(), "the record reaches this far back");
        }
        assert_ne!(machine.executed(), executed, "it really moved");
        assert_ne!(machine.instruction_pointer(), pointer, "and to another row");
        for _ in 0..203 {
            machine.step_one();
        }

        assert_eq!(machine.instruction_pointer(), pointer);
        assert_eq!(machine.executed(), executed);
        assert_eq!(
            machine.registers.general().collect::<Vec<_>>(),
            registers,
            "every register is what it was"
        );
        assert_eq!(
            Flag::ALL.map(|flag| machine.registers.flag(flag)).to_vec(),
            flags,
            "and every flag"
        );
        assert_eq!(
            (0..256)
                .map(|at| machine.memory.peek(DATA + at))
                .collect::<Vec<_>>(),
            memory,
            "and every byte it had written"
        );
    }

    /// Restarting throws the record away with everything else.
    #[test]
    fn restarting_leaves_nothing_to_step_back_through() {
        let mut machine = machine(|code| {
            code.mov(rax, 1_i64).unwrap();
            code.ret().unwrap();
        });
        machine.step_one();
        assert_eq!(machine.rewindable(), 1);
        machine.restart();
        assert_eq!(machine.rewindable(), 0);
        assert!(!machine.step_back());
    }

    #[test]
    fn an_architecture_without_an_interpreter_says_so() {
        let memory = Memory::new(Vec::new().into());
        let machine = Machine::over(memory, Architecture::Arm64, 0);
        assert!(matches!(
            machine.stop(),
            Some(Stop::UnsupportedArchitecture { .. })
        ));
        assert!(!machine.can_continue());
    }

    #[test]
    fn running_off_the_mapped_image_is_reported_as_leaving_it() {
        // What a call into a library looks like from here: the code is in
        // another file, and that file is not open.
        let machine = run(|code| {
            code.mov(rax, 0x9000_0000_i64).unwrap();
            code.jmp(rax).unwrap();
        });
        assert!(
            matches!(machine.stop(), Some(Stop::LeftTheImage { at: 0x9000_0000 })),
            "got {:?}",
            machine.stop()
        );
    }
}
