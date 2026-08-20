//! Finding the functions of a binary that names none.
//!
//! A stripped file has no symbol table, so the Functions view of it is empty —
//! and everything built on that list, from the control-flow graph to "run this
//! function", is empty with it. That is the state most files worth reading are
//! in.
//!
//! What is done about it here is not guesswork dressed up as a symbol table.
//! Each address comes with the reason it is here, and the reasons are not
//! equally good:
//!
//! - **The entry point** is where the file itself says execution begins.
//! - **An address something calls** is a fact the bytes state: `call 0x1234`
//!   is an instruction saying that 0x1234 is entered as a function. This is
//!   the one that finds most of them, and it is exact.
//! - **A prologue that follows an end** — `push %rbp; mov %rsp,%rbp` or
//!   `endbr64`, standing where the previous function stopped. A reading, not a
//!   fact, and it takes both halves: a prologue in the middle of a function is
//!   a register being saved, and an `endbr64` in the middle of one is a marker
//!   on the target of an indirect jump. Either on its own found fifteen
//!   thousand "functions" in `bash`, which is not a list anybody reads.
//!
//! Nothing here renames anything or claims a function's extent. It answers
//! "where does a function start", with its reason attached, and the interface
//! shows the reason: a reader must be able to tell a called address from a
//! shape that looked right.

use crate::analysis::{Analysis, Instruction, operand};

/// The most a file will be reported to have.
///
/// A corrupted or hostile file can be made to look like a million tiny
/// functions; a list that long is not a list anyone reads, and building it
/// costs time no reader asked to spend.
const MAXIMUM: usize = 20_000;

/// Why an address is taken to be the start of a function.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Evidence {
    /// It begins the way a compiler begins a function, and stands where the
    /// last one ended.
    Prologue,
    /// Something calls it, and this is how many instructions do.
    Called(usize),
    /// The file says execution starts here.
    EntryPoint,
}

impl Evidence {
    /// Whether the bytes state this outright, as against it being a reading of
    /// their shape.
    #[must_use]
    pub const fn is_certain(self) -> bool {
        matches!(self, Self::EntryPoint | Self::Called(_))
    }
}

/// One address a function is taken to start at, and why.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Discovered {
    pub address: u64,
    pub evidence: Evidence,
}

/// Every function start the file's own code points at, in address order.
///
/// Addresses a symbol already names are left out: they have a name, and this
/// exists for the ones that do not.
#[must_use]
pub fn functions(analysis: &Analysis) -> Vec<Discovered> {
    let named = named_addresses(analysis);
    let mut found: std::collections::BTreeMap<u64, Evidence> = std::collections::BTreeMap::new();

    let mut offer = |address: u64, evidence: Evidence| {
        if named.binary_search(&address).is_ok() || found.len() >= MAXIMUM {
            return;
        }
        // The best reason wins, and `Called` counts how many callers there
        // were rather than keeping only the first.
        let entry = found.entry(address).or_insert(evidence);
        *entry = match (*entry, evidence) {
            (Evidence::Called(a), Evidence::Called(b)) => Evidence::Called(a + b),
            (existing, offered) => existing.max(offered),
        };
    };

    if let Some(entry) = analysis.entry_point
        && analysis.instruction_at(entry).is_some()
    {
        offer(entry, Evidence::EntryPoint);
    }

    let instructions = &analysis.instructions;
    for (index, instruction) in instructions.iter().enumerate() {
        if is_call(&instruction.text)
            && let Some(target) = operand::branch_target(instruction)
            && analysis.instruction_at(target).is_some()
        {
            offer(target, Evidence::Called(1));
        }
        // Both halves, and neither on its own: see the module's own
        // documentation for what each of them finds alone.
        if begins_a_function(instructions, index) && follows_an_end(instructions, index) {
            offer(instruction.address, Evidence::Prologue);
        }
    }

    found
        .into_iter()
        .map(|(address, evidence)| Discovered { address, evidence })
        .collect()
}

/// The addresses the file already has names for, sorted for lookup.
fn named_addresses(analysis: &Analysis) -> Vec<u64> {
    let mut named: Vec<u64> = analysis
        .symbols
        .iter()
        .filter(|symbol| !symbol.imported)
        .filter_map(|symbol| symbol.address)
        .collect();
    named.sort_unstable();
    named.dedup();
    named
}

/// Whether a line is a call, in either of the two syntaxes the tool decodes
/// into.
fn is_call(text: &str) -> bool {
    let mnemonic = text.split_whitespace().next().unwrap_or_default();
    // `call`, `callq`; and AArch64's `bl`, which is the same instruction under
    // another name. `blr` is an indirect call and names no target.
    matches!(mnemonic, "call" | "callq" | "calll" | "callw" | "bl")
}

/// Whether a line ends the flow, so what follows it is not part of it.
fn ends_the_flow(text: &str) -> bool {
    let mnemonic = text.split_whitespace().next().unwrap_or_default();
    matches!(
        mnemonic,
        "ret" | "retq" | "retl" | "retw" | "jmp" | "jmpq" | "b" | "br" | "hlt" | "ud2"
    )
}

/// Whether a line is padding a compiler puts between functions.
fn is_padding(text: &str) -> bool {
    let mnemonic = text.split_whitespace().next().unwrap_or_default();
    // `int3` fills the gap on Windows, `nop` everywhere else, and `nopw`,
    // `nopl` and friends are the wide nops an assembler pads with.
    mnemonic.starts_with("nop") || matches!(mnemonic, "int3" | "xchg")
}

/// Whether the instruction at `index` begins the way a function begins.
///
/// Two shapes, and both are read from the listing rather than from a table of
/// byte patterns: what a compiler emits changes, and the listing is what the
/// reader is looking at anyway.
fn begins_a_function(instructions: &[Instruction], index: usize) -> bool {
    let Some(instruction) = instructions.get(index) else {
        return false;
    };
    let text = instruction.text.as_str();
    // The frame-pointer prologue, which needs its second half to be one:
    // `push %rbp` on its own is how any function saves a register.
    if starts_with_word(text, "push") && text.contains("bp") {
        return instructions
            .get(index + 1)
            .is_some_and(|next| starts_with_word(&next.text, "mov") && next.text.contains("bp"));
    }
    // The branch-target marker, which only ever appears where something can
    // be jumped or called to.
    if starts_with_word(text, "endbr64") || starts_with_word(text, "endbr32") {
        return true;
    }
    // AArch64 saves the frame pointer and the link register together, and
    // does it with a pre-decrement that no ordinary store uses.
    starts_with_word(text, "stp") && text.contains("x29") && text.contains("x30")
}

/// Whether the instruction at `index` is the first after something ended.
///
/// Padding between the two is stepped over, and the end must be in the same
/// section: the first instruction of a section follows whatever the last
/// instruction of the previous one happened to be, which says nothing.
///
/// The first instruction of a section counts, because there is nothing before
/// it that could have ended.
fn follows_an_end(instructions: &[Instruction], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    let Some(instruction) = instructions.get(index) else {
        return false;
    };
    if is_padding(&instruction.text) {
        return false;
    }
    let mut before = index - 1;
    loop {
        let Some(previous) = instructions.get(before) else {
            return false;
        };
        if previous.section != instruction.section {
            // A new section starts here, and nothing in the previous one runs
            // into it.
            return true;
        }
        if !is_padding(&previous.text) {
            return ends_the_flow(&previous.text);
        }
        if before == 0 {
            return false;
        }
        before -= 1;
    }
}

/// Whether a line's first word is exactly this mnemonic.
fn starts_with_word(text: &str, word: &str) -> bool {
    text.split_whitespace().next() == Some(word)
}

/// The name to show an unnamed function under.
///
/// The convention every tool that has ever done this uses, so a reader coming
/// from one of them recognises it on sight, and nobody mistakes it for a name
/// the file carried.
#[must_use]
pub fn placeholder_name(address: u64) -> String {
    format!("sub_{address:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The reference binary is this test executable, which is a real file with
    /// a real symbol table — and the only one certain to be present on every
    /// machine these tests run on.
    fn reference() -> Analysis {
        crate::analyse_path(std::env::current_exe().expect("this executable"))
            .expect("it can be read")
    }

    /// What the file's own symbol table says the functions are, kept to the
    /// ones the listing actually decoded.
    fn known(analysis: &Analysis) -> BTreeSet<u64> {
        analysis
            .symbols
            .iter()
            .filter(|symbol| !symbol.imported && symbol.size > 0)
            .filter_map(|symbol| symbol.address)
            .filter(|address| analysis.instruction_at(*address).is_some())
            .collect()
    }

    /// The measure that matters: take a real file's symbols away, and see how
    /// much of what they said is found again from the code alone.
    ///
    /// Held to a deliberately loose figure. The point of the test is that the
    /// heuristics keep working at all — a change that broke calls, or one that
    /// stopped stepping over padding, drops this to nearly nothing — and not
    /// to pin a percentage that will move with every compiler.
    #[test]
    fn most_of_a_real_binarys_functions_are_found_without_its_symbols() {
        let analysis = reference();
        let truth = known(&analysis);
        if truth.len() < 50 {
            // A host whose executable names almost nothing has nothing to
            // measure against.
            return;
        }
        let mut stripped = analysis.clone();
        stripped.symbols.clear();
        let found: BTreeSet<u64> = functions(&stripped)
            .into_iter()
            .map(|function| function.address)
            .collect();

        let hit = truth.intersection(&found).count();
        let share = hit * 100 / truth.len();
        assert!(
            share >= 60,
            "only {share}% of {} known functions were found again ({hit})",
            truth.len()
        );
    }

    /// An address something calls is a fact, and the count is of callers.
    #[test]
    fn a_called_address_is_certain_and_counts_its_callers() {
        let analysis = reference();
        let mut stripped = analysis.clone();
        stripped.symbols.clear();
        let found = functions(&stripped);
        let called: Vec<&Discovered> = found
            .iter()
            .filter(|function| matches!(function.evidence, Evidence::Called(_)))
            .collect();
        // Only where the host's own binary has a call whose target the text
        // states. A file of nothing but indirect calls is a real thing, and
        // this test is about what is done with a stated one.
        if called.is_empty() {
            return;
        }
        assert!(
            called.iter().all(|function| function.evidence.is_certain()),
            "a call is what the bytes say, not a reading of them"
        );
        assert!(
            called
                .iter()
                .any(|function| matches!(function.evidence, Evidence::Called(count) if count >= 1)),
            "the count of callers is kept"
        );
    }

    /// Every address offered is one the listing actually decoded, so opening
    /// one always lands somewhere.
    #[test]
    fn every_address_offered_is_one_the_listing_can_show() {
        let analysis = reference();
        let mut stripped = analysis.clone();
        stripped.symbols.clear();
        for function in functions(&stripped) {
            assert!(
                stripped.instruction_at(function.address).is_some(),
                "{:#x} was offered and is not in the listing",
                function.address
            );
        }
    }

    /// What the file already names is left alone: this exists for what it does
    /// not name.
    #[test]
    fn an_address_the_file_names_is_not_offered_again() {
        let analysis = reference();
        let truth = known(&analysis);
        if truth.is_empty() {
            return;
        }
        let found: BTreeSet<u64> = functions(&analysis)
            .into_iter()
            .map(|function| function.address)
            .collect();
        assert!(
            found.intersection(&truth).next().is_none(),
            "a named function was offered as an unnamed one"
        );
    }

    /// The list is bounded, whatever it is asked of.
    #[test]
    fn the_list_is_bounded() {
        let analysis = reference();
        assert!(functions(&analysis).len() <= MAXIMUM);
    }

    /// The three formats and the two architectures, whichever machine the
    /// tests run on.
    ///
    /// The host's own binary is one format and one architecture — whatever it
    /// happens to be — so a heuristic that only worked on x86 passed on Linux
    /// and failed on an Apple Silicon runner, which is how the `AArch64` branch
    /// reader came to be written.
    #[test]
    fn every_format_and_architecture_finds_the_functions_its_fixture_declares() {
        for fixture in crate::fixtures::all() {
            let analysis = crate::analyse_bytes(
                std::path::Path::new("fixture.bin"),
                fixture.bytes.len() as u64,
                &fixture.bytes,
            );
            let mut stripped = analysis.clone();
            stripped.symbols.clear();
            let found: BTreeSet<u64> = functions(&stripped)
                .into_iter()
                .map(|function| function.address)
                .collect();
            assert!(
                !found.is_empty(),
                "{}: its own code points at nothing",
                fixture.label
            );
            // The entry point, which every fixture declares and every format
            // states, is the one address that must always be found.
            if let Some(entry) = analysis.entry_point
                && analysis.instruction_at(entry).is_some()
            {
                assert!(
                    found.contains(&entry),
                    "{}: the entry point {entry:#x} is not among {found:#x?}",
                    fixture.label
                );
            }
        }
    }

    #[test]
    fn a_placeholder_name_says_what_it_is() {
        assert_eq!(placeholder_name(0x0040_01f0), "sub_4001f0");
        assert_eq!(placeholder_name(0), "sub_0");
    }

    #[test]
    fn the_reasons_are_ordered_from_weakest_to_strongest() {
        assert!(Evidence::EntryPoint > Evidence::Called(9));
        assert!(Evidence::Called(1) > Evidence::Prologue);
        assert!(!Evidence::Prologue.is_certain());
        assert!(Evidence::Called(1).is_certain());
        assert!(Evidence::EntryPoint.is_certain());
    }
}
