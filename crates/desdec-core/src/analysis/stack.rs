//! What the stack holds at each point of the listing.
//!
//! Desdec never runs the file, so nothing here is a measurement: the stack is
//! followed by reading the instructions that move the stack pointer, one after
//! the other, from the start of the frame they belong to. That is exact for
//! the shapes a compiler emits — a prologue that saves registers and reserves
//! room, an epilogue that gives it back — and it stops being exact the moment
//! the program does something the text alone cannot settle.
//!
//! Three rules keep the answer honest:
//!
//! - **An unreadable move makes the depth unknown, and it stays unknown.** A
//!   `sub %rax,%rsp` moves the pointer by a value only a run would know, so
//!   everything after it in the frame is reported as unknown rather than as a
//!   number that would look measured.
//! - **The walk restarts at a frame boundary.** A named function, and the
//!   instruction after a `ret`, begin a new frame; carrying a depth across one
//!   would describe a stack no execution ever has.
//! - **Nothing is inferred from a branch.** The listing is read in address
//!   order, which is only the executed order while nothing jumps into the
//!   middle of it. The depth is therefore a local reading, exactly like the
//!   register history in [`crate::operand`].

use std::collections::BTreeSet;

use crate::{
    Architecture,
    analysis::{Analysis, Instruction},
};

/// One thing the stack holds, newest first in [`StackState::slots`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StackSlot {
    /// The address the frame will return to, put there by the `call` that
    /// reached it. Only on architectures whose call writes it to the stack.
    ReturnAddress,
    /// A register the prologue saved, by name.
    Saved(String),
    /// Anything else pushed, in the instruction's own words.
    Pushed(String),
    /// Room made in one move, without naming what will go in it.
    Reserved(u64),
}

impl StackSlot {
    /// How many bytes of stack this slot occupies.
    #[must_use]
    pub const fn size(&self, word: u64) -> u64 {
        match self {
            Self::Reserved(bytes) => *bytes,
            _ => word,
        }
    }
}

/// The stack as it stands just before one instruction executes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StackState {
    /// Bytes between the stack pointer and where the frame began, or `None`
    /// once something moved the pointer by an amount the text does not state.
    pub depth: Option<u64>,
    /// What is known to be on the stack, top first.
    pub slots: Vec<StackSlot>,
    /// Set when the frame this was read from could not be reached — the walk
    /// back hit its bound — so the slots below are missing rather than absent.
    pub truncated: bool,
}

impl StackState {
    /// Nothing known at all: the state before the first instruction of a
    /// listing whose frame could not be found.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            depth: None,
            slots: Vec::new(),
            truncated: true,
        }
    }
}

/// Most slots kept for one frame.
///
/// A frame that pushes more than this is not a frame a reader is reading slot
/// by slot; the oldest are dropped so a hostile listing cannot make one state
/// grow without bound.
const MAXIMUM_SLOTS: usize = 64;

/// How far back the walk looks for the start of the frame an address is in.
///
/// A function longer than this is followed from wherever the bound lands, and
/// the state says so through [`StackState::truncated`] rather than pretending
/// the frame started there.
const LOOK_BACK: usize = 4096;

/// What one instruction does to the stack pointer.
enum Effect {
    /// Nothing at all — the overwhelming majority.
    Neutral,
    Push(StackSlot, u64),
    Pop(u64),
    /// Room made or given back in one move.
    Reserve(u64),
    Release(u64),
    /// The stack pointer is set back to the frame pointer.
    RestoreFramePointer,
    /// The frame pointer is set from the stack pointer.
    SetFramePointer,
    /// `leave`: restore the frame pointer, then pop it.
    Leave,
    /// The frame ends here; what follows belongs to another one.
    Return,
    /// The pointer moved by an amount the text does not state.
    Unreadable,
}

/// Follows the stack pointer through a run of instructions.
struct Tracker {
    architecture: Architecture,
    word: u64,
    depth: Option<u64>,
    slots: Vec<StackSlot>,
    /// Depth recorded when the frame pointer was set, so restoring it restores
    /// a depth rather than an unknown.
    frame_pointer: Option<u64>,
    truncated: bool,
}

impl Tracker {
    fn new(architecture: Architecture) -> Self {
        let word = match architecture {
            Architecture::X86_64 | Architecture::Arm64 => 8,
            _ => 4,
        };
        let mut tracker = Self {
            architecture,
            word,
            depth: None,
            slots: Vec::new(),
            frame_pointer: None,
            truncated: false,
        };
        tracker.enter();
        tracker
    }

    /// Begins a new frame, as the first instruction of a function sees it.
    fn enter(&mut self) {
        self.slots.clear();
        self.frame_pointer = None;
        self.truncated = false;
        // x86 puts the return address on the stack; AArch64 leaves it in the
        // link register, so its frame starts empty.
        match self.architecture {
            Architecture::X86 | Architecture::X86_64 => {
                self.depth = Some(self.word);
                self.slots.push(StackSlot::ReturnAddress);
            }
            _ => self.depth = Some(0),
        }
    }

    /// The stack as it stands before the next instruction runs.
    fn state(&self) -> StackState {
        StackState {
            depth: self.depth,
            slots: self.slots.iter().rev().cloned().collect(),
            truncated: self.truncated,
        }
    }

    fn push(&mut self, slot: StackSlot, size: u64) {
        self.depth = self.depth.map(|depth| depth.saturating_add(size));
        self.slots.push(slot);
        if self.slots.len() > MAXIMUM_SLOTS {
            self.slots.remove(0);
            self.truncated = true;
        }
    }

    fn take(&mut self, size: u64) {
        self.depth = self.depth.and_then(|depth| depth.checked_sub(size));
        // Only a slot of exactly that size can be named; anything else leaves
        // the list unable to say what went, so it is emptied rather than kept
        // describing bytes that are no longer there.
        match self.slots.last() {
            Some(slot) if slot.size(self.word) == size => {
                self.slots.pop();
            }
            _ => self.slots.clear(),
        }
    }

    /// Applies one instruction. Returns `true` when the frame ended with it.
    fn apply(&mut self, instruction: &Instruction) -> bool {
        match effect(instruction, self.architecture, self.word) {
            Effect::Neutral => {}
            Effect::Push(slot, size) => self.push(slot, size),
            Effect::Pop(size) => self.take(size),
            Effect::Reserve(bytes) => self.push(StackSlot::Reserved(bytes), bytes),
            Effect::Release(bytes) => self.take(bytes),
            Effect::SetFramePointer => self.frame_pointer = self.depth,
            Effect::RestoreFramePointer => {
                self.depth = self.frame_pointer;
                self.slots.clear();
            }
            Effect::Leave => {
                self.depth = self.frame_pointer;
                self.slots.clear();
                self.take(self.word);
            }
            Effect::Return => return true,
            Effect::Unreadable => {
                self.depth = None;
                self.slots.clear();
            }
        }
        false
    }
}

/// The frames of one binary, and the depth of the stack at each instruction.
///
/// Built once when a binary is opened. Both answers the interface wants — the
/// depth against every row, and what the frame holds at the selected one —
/// need the set of addresses that begin a frame, and rebuilding that from a
/// symbol table of twenty thousand entries on every frame drawn was the whole
/// cost of showing it.
///
/// One pass over the program: 320 000 instructions take about sixteen
/// milliseconds, against seconds for the analysis that produced them.
#[derive(Clone, Debug, Default)]
pub struct Trace {
    /// Addresses that begin a frame.
    starts: BTreeSet<u64>,
    /// Depth before each instruction, in listing order.
    depths: Vec<Option<u64>>,
}

impl Trace {
    #[must_use]
    pub fn of(analysis: &Analysis) -> Self {
        let starts = frame_starts(analysis);
        let mut tracker = Tracker::new(analysis.summary.architecture);
        let mut depths = Vec::with_capacity(analysis.instructions.len());
        let mut restart = true;

        for instruction in &analysis.instructions {
            if restart || starts.contains(&instruction.address) {
                tracker.enter();
            }
            depths.push(tracker.depth);
            restart = tracker.apply(instruction);
        }
        Self { starts, depths }
    }

    /// Depth before the instruction at `index` in the listing.
    #[must_use]
    pub fn depth(&self, index: usize) -> Option<u64> {
        self.depths.get(index).copied().flatten()
    }

    /// How many instructions were followed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.depths.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.depths.is_empty()
    }

    /// Everything known about the stack just before the instruction at
    /// `address`.
    ///
    /// Walks the frame that address belongs to rather than the whole listing,
    /// so this can be asked of the selected instruction on every frame drawn.
    #[must_use]
    pub fn state_at(&self, analysis: &Analysis, address: u64) -> StackState {
        let Some(index) = analysis.instruction_index(address) else {
            return StackState::unknown();
        };
        let (start, complete) = self.frame_start_index(analysis, index);

        let mut tracker = Tracker::new(analysis.summary.architecture);
        tracker.truncated = !complete;
        if !complete {
            // The frame it belongs to was not reached, so nothing is known
            // about what sits below this point — a depth counted from here
            // would be counted from an arbitrary instruction.
            tracker.depth = None;
            tracker.slots.clear();
        }
        for instruction in &analysis.instructions[start..index] {
            if tracker.apply(instruction) {
                tracker.enter();
            }
        }
        tracker.state()
    }

    /// Where the frame containing `index` starts, and whether that start was
    /// actually found rather than the bound being hit.
    fn frame_start_index(&self, analysis: &Analysis, index: usize) -> (usize, bool) {
        let floor = index.saturating_sub(LOOK_BACK);
        let mut position = index;
        while position > floor {
            if self
                .starts
                .contains(&analysis.instructions[position].address)
            {
                return (position, true);
            }
            // The instruction after a return begins another frame.
            if ends_a_frame(
                &analysis.instructions[position - 1],
                analysis.summary.architecture,
            ) {
                return (position, true);
            }
            position -= 1;
        }
        (position, position == 0)
    }
}

/// Addresses that begin a frame.
///
/// The named functions, and every address a `call` names. The second matters
/// as much as the first: a stripped binary has no symbols at all, and the
/// stubs of a PLT carry none even in one that is not — so the walk ran
/// straight through a dozen of them and reported a stack growing by eight
/// bytes at each, when each is entered on its own.
fn frame_starts(analysis: &Analysis) -> BTreeSet<u64> {
    let architecture = analysis.summary.architecture;
    let named = analysis
        .symbols
        .iter()
        .filter(|symbol| !symbol.imported)
        .filter_map(|symbol| symbol.address);
    let called = analysis
        .instructions
        .iter()
        .filter_map(|instruction| call_target(instruction, architecture));
    named.chain(called).collect()
}

/// The address a call names, when it names one outright.
///
/// An indirect call — through a register or a table — is left out: what it
/// reaches is a run-time value, and this only reads what the text states.
fn call_target(instruction: &Instruction, architecture: Architecture) -> Option<u64> {
    let (mnemonic, operands) = parts(instruction);
    let calls = match architecture {
        Architecture::X86 | Architecture::X86_64 => mnemonic.starts_with("call"),
        Architecture::Arm64 => mnemonic == "bl",
        Architecture::Arm | Architecture::Unknown => false,
    };
    if !calls || operands.starts_with('*') {
        return None;
    }
    immediate(operands)
}

fn ends_a_frame(instruction: &Instruction, architecture: Architecture) -> bool {
    matches!(effect(instruction, architecture, 8), Effect::Return)
}

/// The mnemonic and the operand text of one instruction.
fn parts(instruction: &Instruction) -> (&str, &str) {
    match instruction.text.split_once(char::is_whitespace) {
        Some((mnemonic, rest)) => (mnemonic, rest.trim()),
        None => (instruction.text.as_str(), ""),
    }
}

fn effect(instruction: &Instruction, architecture: Architecture, word: u64) -> Effect {
    match architecture {
        Architecture::X86 | Architecture::X86_64 => x86(instruction, word),
        Architecture::Arm64 => arm64(instruction),
        Architecture::Arm | Architecture::Unknown => Effect::Neutral,
    }
}

/// AT&T syntax, as the x86 formatter produces it: `push %rbp`,
/// `sub $0x20,%rsp`, `mov %rsp,%rbp`.
fn x86(instruction: &Instruction, word: u64) -> Effect {
    const POINTER: [&str; 2] = ["%rsp", "%esp"];

    let (mnemonic, operands) = parts(instruction);

    if mnemonic.starts_with("ret") {
        return Effect::Return;
    }
    if mnemonic.starts_with("leave") {
        return Effect::Leave;
    }
    // A call puts a return address on the stack and the matching return takes
    // it off again, so the instruction after the call sees the stack it left.
    if mnemonic.starts_with("call") {
        return Effect::Neutral;
    }
    if sets_frame_pointer(instruction, Architecture::X86_64) {
        return Effect::SetFramePointer;
    }

    let (stem, size) = stem_and_size(mnemonic, word);
    if stem == "push" || stem == "pushf" {
        let slot = match operands.strip_prefix('%') {
            Some(register) => StackSlot::Saved(register.to_owned()),
            None if operands.is_empty() => StackSlot::Pushed(mnemonic.to_owned()),
            None => StackSlot::Pushed(operands.to_owned()),
        };
        return Effect::Push(slot, size);
    }
    if stem == "pop" || stem == "popf" {
        return Effect::Pop(size);
    }

    let mut fields = operands.split(',').map(str::trim);
    let (Some(source), Some(destination)) = (fields.next(), fields.next()) else {
        // A one-operand instruction never writes the stack pointer here.
        return Effect::Neutral;
    };
    if fields.next().is_some() || !POINTER.contains(&destination) {
        return Effect::Neutral;
    }

    // Everything below writes the stack pointer.
    if stem == "mov" {
        return if source == "%rbp" || source == "%ebp" {
            Effect::RestoreFramePointer
        } else {
            Effect::Unreadable
        };
    }
    let Some(amount) = immediate(source) else {
        return Effect::Unreadable;
    };
    match stem {
        "sub" => Effect::Reserve(amount),
        "add" => Effect::Release(amount),
        _ => Effect::Unreadable,
    }
}

/// Splits an AT&T mnemonic from its operand-size suffix.
///
/// The suffix is only stripped from the mnemonics this module acts on: `sub`
/// and `add` end in letters that are themselves suffixes, and trimming them
/// blindly turned `sub` into `su`.
fn stem_and_size(mnemonic: &str, word: u64) -> (&str, u64) {
    const STEMS: [&str; 7] = ["push", "pushf", "pop", "popf", "mov", "sub", "add"];

    for (suffix, size) in [("q", 8_u64), ("l", 4), ("w", 2), ("b", 1)] {
        if let Some(stem) = mnemonic.strip_suffix(suffix)
            && STEMS.contains(&stem)
        {
            return (stem, size);
        }
    }
    (mnemonic, word)
}

/// Whether this instruction sets the frame pointer from the stack pointer.
fn sets_frame_pointer(instruction: &Instruction, architecture: Architecture) -> bool {
    let (mnemonic, operands) = parts(instruction);
    match architecture {
        Architecture::X86 | Architecture::X86_64 => {
            mnemonic.trim_end_matches(['q', 'l', 'w']) == "mov"
                && matches!(
                    operands
                        .split(',')
                        .map(str::trim)
                        .collect::<Vec<_>>()
                        .as_slice(),
                    ["%rsp", "%rbp"] | ["%esp", "%ebp"]
                )
        }
        Architecture::Arm64 => {
            mnemonic == "mov"
                && matches!(
                    operands
                        .split(',')
                        .map(str::trim)
                        .collect::<Vec<_>>()
                        .as_slice(),
                    ["x29", "sp"]
                )
        }
        _ => false,
    }
}

/// Capstone's `AArch64` syntax: `stp x29, x30, [sp, #-0x20]!`,
/// `sub sp, sp, #0x20`, `ldp x29, x30, [sp], #0x20`.
fn arm64(instruction: &Instruction) -> Effect {
    let (mnemonic, operands) = parts(instruction);

    if mnemonic == "ret" {
        return Effect::Return;
    }
    if sets_frame_pointer(instruction, Architecture::Arm64) {
        return Effect::SetFramePointer;
    }
    let fields: Vec<&str> = operands.split(',').map(str::trim).collect();

    // `mov sp, x29` — the epilogue's way back to the frame pointer.
    if mnemonic == "mov" && fields.as_slice() == ["sp", "x29"] {
        return Effect::RestoreFramePointer;
    }
    if (mnemonic == "sub" || mnemonic == "add") && fields.first() == Some(&"sp") {
        if fields.get(1) != Some(&"sp") {
            return Effect::Unreadable;
        }
        let Some(amount) = fields.get(2).and_then(|field| immediate(field)) else {
            return Effect::Unreadable;
        };
        return if mnemonic == "sub" {
            Effect::Reserve(amount)
        } else {
            Effect::Release(amount)
        };
    }

    // A store or load through the stack pointer with write-back is the usual
    // prologue and epilogue: `[sp, #-0x20]!` makes room, `[sp], #0x20` gives
    // it back. Without write-back the pointer does not move at all.
    let touches_pointer = operands.contains("[sp");
    if !touches_pointer {
        return Effect::Neutral;
    }
    let pre_indexed = operands.ends_with('!');
    let post_indexed = operands.contains("],");
    if !pre_indexed && !post_indexed {
        return Effect::Neutral;
    }
    let Some(amount) = write_back_amount(operands) else {
        return Effect::Unreadable;
    };
    match mnemonic {
        "stp" | "str" | "stur" if pre_indexed => Effect::Reserve(amount),
        "ldp" | "ldr" | "ldur" if post_indexed => Effect::Release(amount),
        _ => Effect::Unreadable,
    }
}

/// The displacement of a write-back addressing mode, as a positive count of
/// bytes.
fn write_back_amount(operands: &str) -> Option<u64> {
    let displacement = operands
        .rsplit_once('#')
        .map(|(_, rest)| rest.trim_end_matches(['!', ']', ' ']))?;
    immediate(displacement)
}

/// Reads `$0x20`, `#-0x20` or a plain decimal as a count of bytes.
///
/// The sign is dropped: which way the pointer moves is stated by the mnemonic,
/// and what this answers is how far.
fn immediate(field: &str) -> Option<u64> {
    let text = field.trim();
    let digits = text
        .strip_prefix('$')
        .or_else(|| text.strip_prefix('#'))
        .unwrap_or(text)
        .trim_start_matches('-');
    match digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        Some(hexadecimal) => u64::from_str_radix(hexadecimal, 16).ok(),
        None => digits.parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::InstructionBytes;

    fn listing(lines: &[&str]) -> Vec<Instruction> {
        lines
            .iter()
            .enumerate()
            .map(|(index, text)| Instruction {
                address: 0x1000 + index as u64 * 4,
                bytes: InstructionBytes::new(&[0x90]).expect("one byte"),
                text: (*text).to_owned(),
                section: std::sync::Arc::from(".text"),
            })
            .collect()
    }

    fn walk(architecture: Architecture, lines: &[&str]) -> Vec<Option<u64>> {
        let instructions = listing(lines);
        let mut tracker = Tracker::new(architecture);
        let mut depths = Vec::new();
        for instruction in &instructions {
            depths.push(tracker.depth);
            if tracker.apply(instruction) {
                tracker.enter();
            }
        }
        depths.push(tracker.depth);
        depths
    }

    /// The shape every x86-64 compiler emits: the return address is already
    /// there, the prologue saves a register and reserves room, the epilogue
    /// gives both back.
    #[test]
    fn a_prologue_and_its_epilogue_balance() {
        let depths = walk(
            Architecture::X86_64,
            &[
                "push %rbp",
                "mov %rsp,%rbp",
                "sub $0x20,%rsp",
                "nop",
                "add $0x20,%rsp",
                "pop %rbp",
                "ret",
            ],
        );

        assert_eq!(
            depths,
            [
                Some(8),  // the return address
                Some(16), // %rbp saved
                Some(16),
                Some(48), // 0x20 reserved
                Some(48),
                Some(16),
                Some(8),
                Some(8), // after the return, a new frame begins
            ]
        );
    }

    /// `sub` with a register operand moves the pointer by an amount only a run
    /// would know. Reporting a number there would be an invention, so the
    /// depth becomes unknown and stays unknown for the rest of the frame.
    #[test]
    fn an_unreadable_move_makes_the_depth_unknown_and_keeps_it_so() {
        let depths = walk(
            Architecture::X86_64,
            &["push %rbp", "sub %rax,%rsp", "push %rbx", "nop"],
        );

        assert_eq!(depths, [Some(8), Some(16), None, None, None]);
    }

    /// `leave` undoes the whole frame in one instruction, which only means
    /// anything because the frame pointer was recorded when it was set.
    #[test]
    fn leave_restores_the_depth_the_frame_pointer_was_set_at() {
        let depths = walk(
            Architecture::X86_64,
            &[
                "push %rbp",
                "mov %rsp,%rbp",
                "sub $0x100,%rsp",
                "leave",
                "ret",
            ],
        );

        assert_eq!(
            depths,
            [Some(8), Some(16), Some(16), Some(272), Some(8), Some(8)]
        );
    }

    /// `AArch64` keeps the return address in the link register, so a frame
    /// starts with nothing on the stack, and the prologue's write-back is what
    /// makes room.
    #[test]
    fn aarch64_frames_start_empty_and_grow_by_their_write_back() {
        let depths = walk(
            Architecture::Arm64,
            &[
                "stp x29, x30, [sp, #-0x20]!",
                "mov x29, sp",
                "sub sp, sp, #0x10",
                "add sp, sp, #0x10",
                "ldp x29, x30, [sp], #0x20",
                "ret",
            ],
        );

        assert_eq!(
            depths,
            [
                Some(0),
                Some(32),
                Some(32),
                Some(48),
                Some(32),
                Some(0),
                Some(0)
            ]
        );
    }

    /// A call is balanced by the return of what it called, so the instruction
    /// after it sees the stack the call was made on.
    #[test]
    fn a_call_leaves_the_stack_where_it_found_it() {
        let depths = walk(Architecture::X86_64, &["push %rbx", "call 0x2000", "nop"]);

        assert_eq!(depths, [Some(8), Some(16), Some(16), Some(16)]);
    }

    /// A PLT is a row of stubs, each entered by its own call and none of them
    /// named. Walking straight through reported a stack that grew by eight
    /// bytes at every stub, which no execution ever sees.
    #[test]
    fn an_address_a_call_names_begins_a_frame() {
        use crate::{
            Architecture, BinaryFormat, BinarySummary, Endianness,
            analysis::{Analysis, Instruction, InstructionBytes},
        };

        let listing = [
            (0x1000, "call 0x2000"),
            (0x1005, "nop"),
            (0x2000, "push $0"),
            (0x2005, "jmp 0x3000"),
            (0x2010, "push $1"),
        ];
        let analysis = Analysis {
            summary: BinarySummary {
                path: std::path::PathBuf::from("test.bin"),
                size: 0,
                format: BinaryFormat::Elf {
                    bits: 64,
                    endianness: Endianness::Little,
                },
                architecture: Architecture::X86_64,
            },
            entry_point: None,
            sections: Vec::new(),
            strings: Vec::new(),
            symbols: Vec::new(),
            import_slots: Vec::new(),
            network: crate::analysis::NetworkUse::default(),
            instructions: listing
                .iter()
                .map(|(address, text)| Instruction {
                    address: *address,
                    bytes: InstructionBytes::new(&[0x90]).expect("one byte"),
                    text: (*text).to_owned(),
                    section: std::sync::Arc::from(".plt"),
                })
                .collect(),
            code_truncated: false,
            details: crate::BinaryDetails::default(),
            languages: Vec::new(),
            sha256: None,
            entropy: None,
            analysed_bytes: 0,
            truncated: false,
        };

        let trace = Trace::of(&analysis);

        assert_eq!(trace.depth(2), Some(8), "the called address starts a frame");
        assert_eq!(
            trace.depth(4),
            Some(16),
            "the stub carries on until something says otherwise"
        );
    }

    /// The suffix carries the operand size, and the mnemonics that end in one
    /// of those letters must not lose it: `sub` is not `su`.
    #[test]
    fn operand_size_suffixes_are_only_stripped_from_the_mnemonics_that_take_one() {
        assert_eq!(stem_and_size("pushq", 8), ("push", 8));
        assert_eq!(stem_and_size("popl", 8), ("pop", 4));
        assert_eq!(stem_and_size("sub", 8), ("sub", 8));
        assert_eq!(stem_and_size("nop", 8), ("nop", 8));
    }

    #[test]
    fn what_the_prologue_saved_is_named() {
        let instructions = listing(&["push %rbp", "push %rbx", "sub $0x10,%rsp", "nop"]);
        let mut tracker = Tracker::new(Architecture::X86_64);
        for instruction in &instructions[..3] {
            tracker.apply(instruction);
        }

        assert_eq!(
            tracker.state().slots,
            [
                StackSlot::Reserved(0x10),
                StackSlot::Saved("rbx".to_owned()),
                StackSlot::Saved("rbp".to_owned()),
                StackSlot::ReturnAddress,
            ]
        );
    }
}
