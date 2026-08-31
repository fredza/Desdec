//! Comparing two binaries: what is the same, what moved, and what changed.
//!
//! The question a reader arrives with when a fix ships is not "what is in this
//! file" but "what is different about it". Neither a listing nor a symbol table
//! answers that: the whole image has moved by a few bytes, every address is
//! new, and a byte-for-byte comparison reports that nothing survived. What the
//! reader wanted was the three functions somebody edited.
//!
//! # What is compared, and what that is worth
//!
//! Functions are paired first, and each pairing says *how* it was arrived at,
//! because the ways are not equally good:
//!
//! - **By name** — both files name a function the same, and the name is unique
//!   on each side. The file says this outright.
//! - **At the same address, with the same bytes** — the function nobody
//!   touched in a build that did not move. A fact, and the one that answers
//!   most of a comparison of two neighbouring builds. It is its own pairing
//!   rather than a case of the one below because an address is unique on each
//!   side by construction: a program holds a thousand identical two-byte
//!   stubs, and the rule below refuses every one of them, while each of them
//!   sits at exactly one address.
//! - **By its bytes** — the same body, byte for byte, at whatever address. A
//!   function that only moved. Also a fact.
//! - **By its shape** — the same instructions once the numbers they carry are
//!   set aside, which is what recompiling at another address produces. A
//!   reading: two functions that differ only in their constants have the same
//!   shape, and so do two copies of the same short stub.
//! - **By a neighbour** — the one function this pair's left side calls that is
//!   still unpaired, against the one its right side calls that is still
//!   unpaired. A reading built on another answer, and the weakest here.
//!
//! Every pairing is refused unless the key is unique on *both* sides. Three
//! identical stubs on the left and three on the right can be paired six ways
//! and there is no reason to prefer one, so none is chosen and all six are
//! reported as unpaired. That is the whole rule this module is built on: an
//! arbitrary pairing is not a weaker answer than no pairing, it is a wrong one,
//! and it would be read as a finding.
//!
//! # The verdict and the measure are two different questions
//!
//! Whether a paired function *changed* is settled by its bytes, which is exact.
//! How much it changed is measured by aligning the two bodies, and there the
//! numbers an instruction carries are deliberately ignored: a file recompiled
//! at another base address moves every constant in it, and counting those would
//! report every function as rewritten. So a change that falls entirely on a
//! constant is a pair that the bytes call **changed** and the alignment counts
//! **no lines** in — which is the honest reading of `mov $0x1,%eax` becoming
//! `mov $0x2,%eax`, and is said in those terms rather than smoothed over.
//!
//! Nothing here reads a file. It works over two analyses already made, and the
//! bodies the caller hands it.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    Analysis, Instruction,
    analysis::{flow, hash, operand},
};

/// One function offered for comparison.
///
/// `name` is what the *file* calls it, and `None` when the file calls it
/// nothing. A name a tool made up out of an address — `sub_401000` — must not
/// be passed here: it would pair two unrelated functions that happen to sit at
/// the same address in two different programs, and it would report that as the
/// files having agreed on a name.
#[derive(Clone, Copy, Debug)]
pub struct Body<'a> {
    /// Address the function starts at, which is what a caller names it by.
    pub start: u64,
    /// The file's own name for it, when the file gives one.
    pub name: Option<&'a str>,
    /// Its decoded instructions, in address order.
    pub instructions: &'a [Instruction],
}

/// How a pair was arrived at, strongest first.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Pairing {
    /// Both files spell the name the same way, and only once each.
    Name,
    /// The same address and the same body: a function a build left alone.
    Address,
    /// The same body, byte for byte, at another address.
    Bytes,
    /// The same instructions once their numbers are set aside.
    Shape,
    /// The only unpaired function called from each side of a pair already made.
    Neighbour,
}

impl Pairing {
    /// Whether the files state this, as against it being a reading of them.
    ///
    /// The interface shows the difference, for the same reason
    /// [`crate::discover::Evidence`] does: a reader has to be able to tell a
    /// name both files carry from a shape that looked alike.
    #[must_use]
    pub const fn is_certain(self) -> bool {
        matches!(self, Self::Name | Self::Address | Self::Bytes)
    }
}

/// What a pair turned out to be worth, settled by the bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verdict {
    /// The same bytes on both sides. It may still have moved.
    Identical,
    /// The bytes differ.
    Changed,
}

/// How far apart two bodies are, in instructions.
///
/// Counted over the alignment described in the module documentation, so the
/// numbers an instruction carries are not what is counted. Both being zero on a
/// changed pair is a real answer and not a failure: the two bodies hold the
/// same instructions and differ only in what those instructions carry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Difference {
    /// Instructions on the left that the alignment found no match for.
    pub removed: usize,
    /// Instructions on the right that it found no match for.
    pub added: usize,
}

/// Two functions taken to be the same one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pair {
    /// Position in the bodies given for the left file.
    pub left: usize,
    /// Position in the bodies given for the right file.
    pub right: usize,
    pub pairing: Pairing,
    pub verdict: Verdict,
    /// How far apart the bodies are, and `None` when they were too large to
    /// align — see [`MAXIMUM_ALIGNMENT`]. `None` is not zero, and the interface
    /// must not show it as such.
    pub difference: Option<Difference>,
    /// Whether the function sits at a different address in the two files.
    pub moved: bool,
}

/// What comparing two sets of function bodies came to.
#[derive(Clone, Debug, Default)]
pub struct Functions {
    /// The pairs, in the order of the left file's bodies.
    pub pairs: Vec<Pair>,
    /// Positions in the left file's bodies that nothing was paired with.
    pub only_left: Vec<usize>,
    /// Positions in the right file's bodies that nothing was paired with.
    pub only_right: Vec<usize>,
}

impl Functions {
    /// How many pairs hold the same bytes on both sides.
    #[must_use]
    pub fn identical(&self) -> usize {
        self.pairs
            .iter()
            .filter(|pair| pair.verdict == Verdict::Identical)
            .count()
    }

    /// How many pairs hold different bytes.
    #[must_use]
    pub fn changed(&self) -> usize {
        self.pairs
            .iter()
            .filter(|pair| pair.verdict == Verdict::Changed)
            .count()
    }
}

/// Largest alignment table that will be filled, in cells.
///
/// Aligning two bodies costs one cell per pair of instructions, and a pair of
/// ten-thousand-instruction functions is a hundred million of them — seconds of
/// work for a count beside a row. Beyond this the pair still has its verdict,
/// which is what the reader is actually being told; only the measure is left
/// unanswered, and it is left unanswered rather than estimated.
pub const MAXIMUM_ALIGNMENT: usize = 4_000_000;

/// Most names either side of a list comparison will report.
///
/// A large image holds tens of thousands of strings, and two unrelated files
/// compared against each other differ in all of them. A list that long is not
/// an answer anybody reads, and building it costs time nobody asked to spend.
pub const MAXIMUM_LISTED: usize = 5_000;

/// What one function's body reduces to, for the purpose of finding it again.
struct Fingerprint {
    /// The body's bytes, exactly. Equality here is equality of the function.
    bytes: [u8; 32],
    /// The body's instructions with their numbers set aside; see the module
    /// documentation.
    shape: [u8; 32],
    /// One key per instruction, over the same reading `shape` is built from, so
    /// the alignment and the pairing never disagree about what two instructions
    /// being alike means.
    lines: Vec<u64>,
    /// Addresses this body calls, where the text states one, in body order.
    calls: Vec<u64>,
}

fn fingerprint(body: &Body) -> Fingerprint {
    let mut bytes = Vec::with_capacity(body.instructions.len() * 4);
    let mut lines = Vec::with_capacity(body.instructions.len());
    let mut calls = Vec::new();
    for instruction in body.instructions {
        bytes.extend_from_slice(instruction.bytes.as_slice());
        lines.push(line_key(&instruction.text));
        if flow::kind(operand::mnemonic(&instruction.text)) == flow::Kind::Call
            && let Some(target) = operand::branch_target(instruction)
        {
            calls.push(target);
        }
    }
    let mut shape_source = Vec::with_capacity(lines.len() * 8);
    for key in &lines {
        shape_source.extend_from_slice(&key.to_le_bytes());
    }
    Fingerprint {
        bytes: hash::sha256(&bytes),
        shape: hash::sha256(&shape_source),
        lines,
        calls,
    }
}

/// One instruction reduced to what it does, without the numbers it does it
/// with.
///
/// A number is a run of alphanumeric characters that *begins* with a digit, so
/// `0x1f`, `8` and `#12` go and `cvtsi2sd`, `x0` and `movd` stay — the digit
/// inside a mnemonic or a register name is part of its spelling, not a value.
/// Hashed as it is read rather than built into a string first: this runs once
/// per instruction of two whole programs.
fn line_key(text: &str) -> u64 {
    // FNV-1a, which is enough for a key that only ever has to tell two
    // instructions apart, and needs no dependency to say so.
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = OFFSET;
    let mut previous_was_alphanumeric = false;
    let mut skipping_number = false;
    for byte in text.bytes() {
        let alphanumeric = byte.is_ascii_alphanumeric();
        if skipping_number {
            if alphanumeric {
                continue;
            }
            skipping_number = false;
        } else if byte.is_ascii_digit() && !previous_was_alphanumeric {
            // One marker for the whole number, whatever its digits were.
            hash = (hash ^ u64::from(b'N')).wrapping_mul(PRIME);
            skipping_number = true;
            previous_was_alphanumeric = true;
            continue;
        }
        previous_was_alphanumeric = alphanumeric;
        hash = (hash ^ u64::from(byte)).wrapping_mul(PRIME);
    }
    hash
}

/// Pairs the functions of two files.
///
/// The bodies are given in the caller's own order, and every position in the
/// answer is a position in them.
#[must_use]
pub fn functions(left: &[Body], right: &[Body]) -> Functions {
    let left_prints: Vec<Fingerprint> = left.iter().map(fingerprint).collect();
    let right_prints: Vec<Fingerprint> = right.iter().map(fingerprint).collect();

    let mut left_taken = vec![false; left.len()];
    let mut right_taken = vec![false; right.len()];
    let mut pairs: Vec<Pair> = Vec::new();

    // The file's own names, then what stayed where it was, then the bodies
    // themselves, then their shape. Each pass sees only what the ones before it
    // left, so a name settles a function that a dozen identical stubs would
    // otherwise have made ambiguous.
    pair_on(
        &mut pairs,
        &mut left_taken,
        &mut right_taken,
        Pairing::Name,
        |index| left[index].name,
        |index| right[index].name,
    );
    pair_on(
        &mut pairs,
        &mut left_taken,
        &mut right_taken,
        Pairing::Address,
        |index| Some((left[index].start, left_prints[index].bytes)),
        |index| Some((right[index].start, right_prints[index].bytes)),
    );
    pair_on(
        &mut pairs,
        &mut left_taken,
        &mut right_taken,
        Pairing::Bytes,
        |index| Some(left_prints[index].bytes),
        |index| Some(right_prints[index].bytes),
    );
    pair_on(
        &mut pairs,
        &mut left_taken,
        &mut right_taken,
        Pairing::Shape,
        |index| Some(left_prints[index].shape),
        |index| Some(right_prints[index].shape),
    );
    propagate(
        left,
        right,
        &left_prints,
        &right_prints,
        &mut pairs,
        &mut left_taken,
        &mut right_taken,
    );

    for pair in &mut pairs {
        let (from, to) = (&left_prints[pair.left], &right_prints[pair.right]);
        pair.verdict = if from.bytes == to.bytes {
            Verdict::Identical
        } else {
            Verdict::Changed
        };
        pair.moved = left[pair.left].start != right[pair.right].start;
        pair.difference = match pair.verdict {
            Verdict::Identical => Some(Difference::default()),
            Verdict::Changed => align(&from.lines, &to.lines),
        };
    }

    pairs.sort_by_key(|pair| pair.left);
    Functions {
        only_left: (0..left.len())
            .filter(|index| !left_taken[*index])
            .collect(),
        only_right: (0..right.len())
            .filter(|index| !right_taken[*index])
            .collect(),
        pairs,
    }
}

/// Pairs whatever is still unpaired on a key that is unique on both sides.
///
/// The uniqueness is worked out over the *remaining* functions rather than over
/// all of them: two identical bodies of which one has already been paired by
/// name leave one candidate each, and refusing that pair because the key was
/// ambiguous before the earlier pass ran would throw away an answer.
fn pair_on<K: Ord>(
    pairs: &mut Vec<Pair>,
    left_taken: &mut [bool],
    right_taken: &mut [bool],
    pairing: Pairing,
    left_key: impl Fn(usize) -> Option<K>,
    right_key: impl Fn(usize) -> Option<K>,
) {
    let left_unique = unique_keys(left_taken, left_key);
    let right_unique = unique_keys(right_taken, right_key);
    for (key, left_index) in left_unique {
        let Some(right_index) = right_unique.get(&key) else {
            continue;
        };
        left_taken[left_index] = true;
        right_taken[*right_index] = true;
        pairs.push(Pair {
            left: left_index,
            right: *right_index,
            pairing,
            verdict: Verdict::Changed,
            difference: None,
            moved: false,
        });
    }
}

/// The keys held by exactly one unpaired function, and which one holds each.
fn unique_keys<K: Ord>(taken: &[bool], key: impl Fn(usize) -> Option<K>) -> BTreeMap<K, usize> {
    let mut once: BTreeMap<K, usize> = BTreeMap::new();
    let mut repeated: BTreeSet<K> = BTreeSet::new();
    for (index, already) in taken.iter().enumerate() {
        if *already {
            continue;
        }
        let Some(key) = key(index) else { continue };
        if repeated.contains(&key) {
            continue;
        }
        if once.remove(&key).is_some() {
            repeated.insert(key);
            continue;
        }
        once.insert(key, index);
    }
    once
}

/// Spreads the pairs already made along the calls the bodies make.
///
/// If two functions are the same one, and each calls exactly one function that
/// is still unpaired, those two are the same one as well. That is the only step
/// here, and it is applied until it says nothing new.
///
/// It terminates, and in linear work: a pair is looked at again only after a
/// new pair was made from it, and there can be no more new pairs than there are
/// functions on the smaller side.
fn propagate(
    left: &[Body],
    right: &[Body],
    left_prints: &[Fingerprint],
    right_prints: &[Fingerprint],
    pairs: &mut Vec<Pair>,
    left_taken: &mut [bool],
    right_taken: &mut [bool],
) {
    let left_at = starts(left);
    let right_at = starts(right);
    let mut queue: Vec<usize> = (0..pairs.len()).collect();

    while let Some(position) = queue.pop() {
        let pair = pairs[position];
        let callees = |print: &Fingerprint, at: &BTreeMap<u64, usize>, taken: &[bool]| {
            let mut found: Vec<usize> = print
                .calls
                .iter()
                .filter_map(|target| at.get(target).copied())
                .filter(|index| !taken[*index])
                .collect();
            found.sort_unstable();
            found.dedup();
            found
        };
        let from = callees(&left_prints[pair.left], &left_at, left_taken);
        let to = callees(&right_prints[pair.right], &right_at, right_taken);
        let ([left_index], [right_index]) = (from.as_slice(), to.as_slice()) else {
            continue;
        };
        left_taken[*left_index] = true;
        right_taken[*right_index] = true;
        pairs.push(Pair {
            left: *left_index,
            right: *right_index,
            pairing: Pairing::Neighbour,
            verdict: Verdict::Changed,
            difference: None,
            moved: false,
        });
        queue.push(pairs.len() - 1);
        // This one may have had more than one candidate a moment ago and have
        // exactly one now, so it is worth another look — and only because a
        // pair was just made, which is what bounds the whole loop.
        queue.push(position);
    }
}

/// Where each body starts, for turning a call's target into the function it
/// enters.
///
/// A target that is not exactly a function's first address is left out: it is
/// a call into the middle of something, or into a part of the image these
/// bodies do not cover, and neither is a function this can name.
fn starts(bodies: &[Body]) -> BTreeMap<u64, usize> {
    bodies
        .iter()
        .enumerate()
        .map(|(index, body)| (body.start, index))
        .collect()
}

/// How many instructions each side holds that the other does not.
///
/// The longest common subsequence of the two bodies, which is the alignment
/// that reads a body as a sequence rather than as a set: three instructions
/// inserted in the middle are three added lines, not a body where everything
/// after the insertion moved. Only the length is wanted, so one row of the
/// table is kept rather than all of it.
///
/// `None` when the table would be larger than [`MAXIMUM_ALIGNMENT`].
fn align(left: &[u64], right: &[u64]) -> Option<Difference> {
    let cells = left.len().checked_mul(right.len())?;
    if cells > MAXIMUM_ALIGNMENT {
        return None;
    }
    // The shorter side across the row, so the row is as small as it can be.
    let (short, long) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    let mut row = vec![0_u32; short.len() + 1];
    for item in long {
        let mut diagonal = 0;
        for (index, other) in short.iter().enumerate() {
            let above = row[index + 1];
            row[index + 1] = if item == other {
                diagonal + 1
            } else {
                above.max(row[index])
            };
            diagonal = above;
        }
    }
    let common = *row.last().unwrap_or(&0) as usize;
    Some(Difference {
        removed: left.len() - common.min(left.len()),
        added: right.len() - common.min(right.len()),
    })
}

/// Names one file holds and the other does not.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Changes {
    /// In the right file only, in the order that file gives them.
    pub added: Vec<String>,
    /// In the left file only.
    pub removed: Vec<String>,
    /// Set when either list was cut at [`MAXIMUM_LISTED`].
    pub truncated: bool,
}

impl Changes {
    /// Whether either file holds something the other does not.
    #[must_use]
    pub fn any(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty()
    }

    /// What is in `right` and not in `left`, and the other way about.
    ///
    /// Order is the one each file gave, so a reader comparing this against the
    /// view a file's own list is shown reads them the same way round.
    fn of(left: &[&str], right: &[&str]) -> Self {
        let left_set: BTreeSet<&str> = left.iter().copied().collect();
        let right_set: BTreeSet<&str> = right.iter().copied().collect();
        let mut changes = Self::default();
        for name in right {
            if !left_set.contains(name) && changes.added.len() < MAXIMUM_LISTED {
                changes.added.push((*name).to_owned());
            }
        }
        for name in left {
            if !right_set.contains(name) && changes.removed.len() < MAXIMUM_LISTED {
                changes.removed.push((*name).to_owned());
            }
        }
        changes.truncated =
            changes.added.len() >= MAXIMUM_LISTED || changes.removed.len() >= MAXIMUM_LISTED;
        changes
    }
}

/// The libraries one file links and the other does not.
#[must_use]
pub fn libraries(left: &Analysis, right: &Analysis) -> Changes {
    let held: Vec<&str> = left
        .details
        .linked_libraries
        .iter()
        .map(String::as_str)
        .collect();
    let theirs: Vec<&str> = right
        .details
        .linked_libraries
        .iter()
        .map(String::as_str)
        .collect();
    Changes::of(&held, &theirs)
}

/// The strings one file holds and the other does not.
///
/// By their text alone: a string that only moved is not a change, and its
/// offset moves in every recompilation.
#[must_use]
pub fn strings(left: &Analysis, right: &Analysis) -> Changes {
    let held: Vec<&str> = left
        .strings
        .iter()
        .map(|string| string.value.as_str())
        .collect();
    let theirs: Vec<&str> = right
        .strings
        .iter()
        .map(|string| string.value.as_str())
        .collect();
    Changes::of(&held, &theirs)
}

/// What one file's section holds, for setting beside the other's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SectionFacts {
    pub file_size: u64,
    pub virtual_size: u64,
    pub entropy: Option<f32>,
}

/// A section name, and what each file says under it.
///
/// `None` on a side means that file has no section of that name at all, which
/// is a different statement from having an empty one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SectionPair<'a> {
    pub name: &'a str,
    pub left: Option<SectionFacts>,
    pub right: Option<SectionFacts>,
}

impl SectionPair<'_> {
    /// Whether the two files say the same thing about this section.
    #[must_use]
    pub fn changed(&self) -> bool {
        match (self.left, self.right) {
            (Some(left), Some(right)) => {
                left.file_size != right.file_size || left.virtual_size != right.virtual_size
            }
            _ => true,
        }
    }
}

/// Every section name either file holds, with what each says about it.
///
/// In the left file's own order, with what only the right file holds after it:
/// a reader comparing this against the segment view of the file they opened
/// finds the rows in the order that view puts them in.
#[must_use]
pub fn sections<'a>(left: &'a Analysis, right: &'a Analysis) -> Vec<SectionPair<'a>> {
    let facts = |section: &crate::Section| SectionFacts {
        file_size: section.file_size,
        virtual_size: section.virtual_size,
        entropy: section.entropy,
    };
    let mut pairs: Vec<SectionPair<'a>> = left
        .sections
        .iter()
        .map(|section| SectionPair {
            name: section.name.as_str(),
            left: Some(facts(section)),
            right: right
                .sections
                .iter()
                .find(|other| other.name == section.name)
                .map(facts),
        })
        .collect();
    pairs.extend(
        right
            .sections
            .iter()
            .filter(|section| !left.sections.iter().any(|other| other.name == section.name))
            .map(|section| SectionPair {
                name: section.name.as_str(),
                left: None,
                right: Some(facts(section)),
            }),
    );
    pairs
}

/// Everything comparing two analysed files came to.
#[derive(Clone, Debug)]
pub struct Comparison<'a> {
    /// Whether the two files are the same bytes. `None` when either digest is
    /// missing, which is what a file too large to be read whole leaves behind.
    pub same_file: Option<bool>,
    pub sections: Vec<SectionPair<'a>>,
    pub libraries: Changes,
    pub strings: Changes,
    pub functions: Functions,
}

/// Compares two analysed files and the function bodies found in them.
#[must_use]
pub fn compare<'a>(
    left: &'a Analysis,
    left_bodies: &[Body],
    right: &'a Analysis,
    right_bodies: &[Body],
) -> Comparison<'a> {
    Comparison {
        same_file: match (left.sha256, right.sha256) {
            (Some(left), Some(right)) => Some(left == right),
            _ => None,
        },
        sections: sections(left, right),
        libraries: libraries(left, right),
        strings: strings(left, right),
        functions: functions(left_bodies, right_bodies),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Body, MAXIMUM_LISTED, Pairing, Verdict, align, compare, functions, line_key, strings,
    };
    use crate::{Instruction, InstructionBytes, analyse_bytes, fixtures};
    use std::path::Path;

    /// Builds a body out of `(bytes, text)` pairs laid end to end from `start`.
    fn body(start: u64, lines: &[(&[u8], &str)]) -> Vec<Instruction> {
        let mut address = start;
        lines
            .iter()
            .map(|(bytes, text)| {
                let instruction = Instruction {
                    address,
                    bytes: InstructionBytes::new(bytes).expect("a test instruction"),
                    text: (*text).to_owned(),
                    section: std::sync::Arc::from(".text"),
                };
                address += bytes.len() as u64;
                instruction
            })
            .collect()
    }

    const PROLOGUE: (&[u8], &str) = (&[0x55], "push %rbp");
    const FRAME: (&[u8], &str) = (&[0x48, 0x89, 0xe5], "mov %rsp,%rbp");
    const LEAVE: (&[u8], &str) = (&[0xc9], "leave");
    const RETURN: (&[u8], &str) = (&[0xc3], "ret");

    #[test]
    fn a_name_both_files_carry_pairs_them_whatever_the_bytes_say() {
        let left = body(0x1000, &[PROLOGUE, FRAME, RETURN]);
        let right = body(0x2000, &[PROLOGUE, FRAME, LEAVE, RETURN]);
        let compared = functions(
            &[Body {
                start: 0x1000,
                name: Some("parse"),
                instructions: &left,
            }],
            &[Body {
                start: 0x2000,
                name: Some("parse"),
                instructions: &right,
            }],
        );

        assert_eq!(compared.pairs.len(), 1);
        assert_eq!(compared.pairs[0].pairing, Pairing::Name);
        assert_eq!(compared.pairs[0].verdict, Verdict::Changed);
        assert!(compared.pairs[0].moved);
        assert_eq!(compared.pairs[0].difference.expect("aligned").added, 1);
        assert_eq!(compared.pairs[0].difference.expect("aligned").removed, 0);
    }

    /// The pairing that answers most of a comparison of two neighbouring
    /// builds, and the one that a thousand identical stubs cannot make
    /// ambiguous.
    #[test]
    fn a_body_that_stayed_where_it_was_is_paired_by_its_address() {
        let mine = body(0x1000, &[RETURN]);
        let other = body(0x1100, &[RETURN]);
        let their_mine = body(0x1000, &[RETURN]);
        let their_other = body(0x1100, &[RETURN]);
        let compared = functions(
            &[
                Body {
                    start: 0x1000,
                    name: None,
                    instructions: &mine,
                },
                Body {
                    start: 0x1100,
                    name: None,
                    instructions: &other,
                },
            ],
            &[
                Body {
                    start: 0x1000,
                    name: None,
                    instructions: &their_mine,
                },
                Body {
                    start: 0x1100,
                    name: None,
                    instructions: &their_other,
                },
            ],
        );

        assert_eq!(compared.pairs.len(), 2);
        assert!(
            compared
                .pairs
                .iter()
                .all(|pair| pair.pairing == Pairing::Address),
            "two bodies nothing else can tell apart, each still at its own address"
        );
        assert!(compared.pairs.iter().all(|pair| !pair.moved));
        assert!(compared.only_left.is_empty());
    }

    #[test]
    fn the_same_body_at_another_address_is_paired_and_called_identical() {
        let left = body(0x1000, &[PROLOGUE, FRAME, RETURN]);
        let right = body(0x8000, &[PROLOGUE, FRAME, RETURN]);
        let compared = functions(
            &[Body {
                start: 0x1000,
                name: None,
                instructions: &left,
            }],
            &[Body {
                start: 0x8000,
                name: None,
                instructions: &right,
            }],
        );

        assert_eq!(compared.pairs.len(), 1);
        assert_eq!(compared.pairs[0].pairing, Pairing::Bytes);
        assert_eq!(compared.pairs[0].verdict, Verdict::Identical);
        assert!(compared.pairs[0].moved);
        assert_eq!(
            compared.pairs[0].difference,
            Some(super::Difference::default())
        );
    }

    /// The case the whole module exists for: a recompilation moves every
    /// address, so the bytes of a function that nobody edited are not the same
    /// bytes. Its shape is.
    #[test]
    fn a_body_that_only_moved_is_paired_by_its_shape() {
        let left = body(
            0x1000,
            &[
                PROLOGUE,
                (&[0xe8, 0x10, 0x00, 0x00, 0x00], "call 0x1015"),
                RETURN,
            ],
        );
        let right = body(
            0x2000,
            &[
                PROLOGUE,
                (&[0xe8, 0x40, 0x00, 0x00, 0x00], "call 0x2045"),
                RETURN,
            ],
        );
        let compared = functions(
            &[Body {
                start: 0x1000,
                name: None,
                instructions: &left,
            }],
            &[Body {
                start: 0x2000,
                name: None,
                instructions: &right,
            }],
        );

        assert_eq!(compared.pairs.len(), 1);
        assert_eq!(compared.pairs[0].pairing, Pairing::Shape);
        assert_eq!(compared.pairs[0].verdict, Verdict::Changed);
        // The bytes differ and not one instruction does: exactly the reading
        // the module documentation promises rather than smooths over.
        assert_eq!(compared.pairs[0].difference.expect("aligned").added, 0);
        assert_eq!(compared.pairs[0].difference.expect("aligned").removed, 0);
    }

    /// The rule the module is built on. Two identical stubs each side can be
    /// paired two ways round and nothing prefers either, so none is offered.
    #[test]
    fn an_ambiguous_key_pairs_nothing_rather_than_picking_one() {
        let stub = body(0x1000, &[RETURN]);
        let other = body(0x1100, &[RETURN]);
        let their_stub = body(0x2000, &[RETURN]);
        let their_other = body(0x2100, &[RETURN]);
        let compared = functions(
            &[
                Body {
                    start: 0x1000,
                    name: None,
                    instructions: &stub,
                },
                Body {
                    start: 0x1100,
                    name: None,
                    instructions: &other,
                },
            ],
            &[
                Body {
                    start: 0x2000,
                    name: None,
                    instructions: &their_stub,
                },
                Body {
                    start: 0x2100,
                    name: None,
                    instructions: &their_other,
                },
            ],
        );

        assert!(compared.pairs.is_empty());
        assert_eq!(compared.only_left, vec![0, 1]);
        assert_eq!(compared.only_right, vec![0, 1]);
    }

    /// And the other half of that rule: a name settles one of the two, and the
    /// one it leaves behind is then the only candidate there is.
    #[test]
    fn a_name_leaves_one_candidate_and_the_bytes_then_pair_it() {
        let named = body(0x1000, &[RETURN]);
        let unnamed = body(0x1100, &[RETURN]);
        let their_named = body(0x2000, &[RETURN]);
        let their_unnamed = body(0x2100, &[RETURN]);
        let compared = functions(
            &[
                Body {
                    start: 0x1000,
                    name: Some("stub"),
                    instructions: &named,
                },
                Body {
                    start: 0x1100,
                    name: None,
                    instructions: &unnamed,
                },
            ],
            &[
                Body {
                    start: 0x2000,
                    name: Some("stub"),
                    instructions: &their_named,
                },
                Body {
                    start: 0x2100,
                    name: None,
                    instructions: &their_unnamed,
                },
            ],
        );

        assert_eq!(compared.pairs.len(), 2);
        assert_eq!(compared.pairs[0].pairing, Pairing::Name);
        assert_eq!(compared.pairs[1].pairing, Pairing::Bytes);
        assert!(compared.only_left.is_empty());
        assert!(compared.only_right.is_empty());
    }

    /// Two functions nothing else can tell apart, each called from a pair that
    /// is already settled.
    #[test]
    fn the_only_unpaired_callee_of_a_pair_is_paired_with_the_other_s() {
        // The callers are told apart by their names; what they call is not
        // told apart by anything but who calls it.
        let entry = body(
            0x1000,
            &[(&[0xe8, 0xfb, 0x00, 0x00, 0x00], "call 0x1100"), RETURN],
        );
        let helper = body(0x1100, &[PROLOGUE, FRAME, LEAVE, RETURN]);
        let their_entry = body(
            0x2000,
            &[(&[0xe8, 0xfb, 0x00, 0x00, 0x00], "call 0x2100"), RETURN],
        );
        // Different bytes and a different shape, so nothing but the call
        // reaches it.
        let their_helper = body(0x2100, &[PROLOGUE, FRAME, PROLOGUE, LEAVE, RETURN]);
        let compared = functions(
            &[
                Body {
                    start: 0x1000,
                    name: Some("main"),
                    instructions: &entry,
                },
                Body {
                    start: 0x1100,
                    name: None,
                    instructions: &helper,
                },
            ],
            &[
                Body {
                    start: 0x2000,
                    name: Some("main"),
                    instructions: &their_entry,
                },
                Body {
                    start: 0x2100,
                    name: None,
                    instructions: &their_helper,
                },
            ],
        );

        assert_eq!(compared.pairs.len(), 2);
        assert_eq!(compared.pairs[1].pairing, Pairing::Neighbour);
        assert_eq!(compared.pairs[1].verdict, Verdict::Changed);
        assert_eq!(compared.pairs[1].difference.expect("aligned").added, 1);
    }

    #[test]
    fn a_function_only_one_file_holds_is_reported_on_that_side() {
        let left = body(0x1000, &[PROLOGUE, RETURN]);
        let kept = body(0x2000, &[PROLOGUE, RETURN]);
        let added = body(0x2100, &[FRAME, LEAVE, RETURN]);
        let compared = functions(
            &[Body {
                start: 0x1000,
                name: None,
                instructions: &left,
            }],
            &[
                Body {
                    start: 0x2000,
                    name: None,
                    instructions: &kept,
                },
                Body {
                    start: 0x2100,
                    name: None,
                    instructions: &added,
                },
            ],
        );

        assert_eq!(compared.pairs.len(), 1);
        assert!(compared.only_left.is_empty());
        assert_eq!(compared.only_right, vec![1]);
    }

    #[test]
    fn the_numbers_an_instruction_carries_are_not_part_of_its_key() {
        assert_eq!(line_key("call 0x1015"), line_key("call 0x2045"));
        assert_eq!(line_key("mov $0x1,%eax"), line_key("mov $0x99,%eax"));
        assert_eq!(line_key("b.eq #0x100000180"), line_key("b.eq #0x4001f0"));
    }

    /// The digit inside a mnemonic or a register name is part of its spelling.
    #[test]
    fn a_digit_that_belongs_to_a_name_is_kept() {
        assert_ne!(line_key("mov %rax,%rbx"), line_key("mov %r8,%rbx"));
        assert_ne!(
            line_key("cvtsi2sd %eax,%xmm0"),
            line_key("cvtsi2ss %eax,%xmm0")
        );
        assert_ne!(line_key("ldr x0,[x1]"), line_key("ldr x0,[x2]"));
    }

    #[test]
    fn the_alignment_counts_an_insertion_as_an_insertion() {
        let before = [1, 2, 3, 4];
        let after = [1, 2, 9, 3, 4];
        let difference = align(&before, &after).expect("small enough");
        assert_eq!(difference.removed, 0);
        assert_eq!(difference.added, 1);
    }

    #[test]
    fn the_alignment_says_nothing_rather_than_zero_for_a_table_too_large() {
        let long: Vec<u64> = (0..3000).collect();
        assert!(align(&long, &long).is_none());
    }

    #[test]
    fn a_string_only_one_file_holds_is_the_only_one_reported() {
        let mut left = fixtures::elf_x86_64();
        let mut right = fixtures::elf_x86_64();
        left.bytes
            .extend_from_slice(b"\0shared text\0only on the left\0");
        right
            .bytes
            .extend_from_slice(b"\0shared text\0only on the right\0");
        let left = analyse_bytes(Path::new("left"), left.bytes.len() as u64, &left.bytes);
        let right = analyse_bytes(Path::new("right"), right.bytes.len() as u64, &right.bytes);

        let changes = strings(&left, &right);
        assert!(
            changes
                .added
                .iter()
                .any(|value| value == "only on the right")
        );
        assert!(
            changes
                .removed
                .iter()
                .any(|value| value == "only on the left")
        );
        assert!(!changes.added.iter().any(|value| value == "shared text"));
        assert!(!changes.removed.iter().any(|value| value == "shared text"));
        assert!(!changes.truncated);
        assert!(changes.any());
    }

    #[test]
    fn a_file_compared_against_itself_says_so_and_finds_nothing_changed() {
        let fixture = fixtures::elf_x86_64();
        let analysis = analyse_bytes(
            Path::new("same"),
            fixture.bytes.len() as u64,
            &fixture.bytes,
        );
        let compared = compare(&analysis, &[], &analysis, &[]);

        assert_eq!(compared.same_file, Some(true));
        assert!(!compared.strings.any());
        assert!(!compared.libraries.any());
        assert!(
            compared.sections.iter().all(|section| !section.changed()),
            "a file's own sections cannot differ from themselves"
        );
    }

    #[test]
    fn two_formats_compared_report_the_sections_only_one_of_them_has() {
        let elf = fixtures::elf_x86_64();
        let pe = fixtures::pe_x86_64();
        let elf = analyse_bytes(Path::new("elf"), elf.bytes.len() as u64, &elf.bytes);
        let pe = analyse_bytes(Path::new("pe"), pe.bytes.len() as u64, &pe.bytes);
        let compared = compare(&elf, &[], &pe, &[]);

        assert_eq!(compared.same_file, Some(false));
        assert!(
            compared
                .sections
                .iter()
                .any(|section| section.left.is_some() && section.right.is_none()),
            "the ELF holds sections the PE does not"
        );
        assert!(
            compared
                .sections
                .iter()
                .any(|section| section.left.is_none() && section.right.is_some()),
            "and the other way about"
        );
    }

    /// Neither list is unbounded, whatever it is handed.
    #[test]
    fn a_list_longer_than_the_bound_is_cut_and_says_so() {
        let held: Vec<String> = (0..MAXIMUM_LISTED + 10)
            .map(|n| format!("mine {n}"))
            .collect();
        let theirs: Vec<String> = (0..MAXIMUM_LISTED + 10)
            .map(|n| format!("theirs {n}"))
            .collect();
        let held: Vec<&str> = held.iter().map(String::as_str).collect();
        let theirs: Vec<&str> = theirs.iter().map(String::as_str).collect();

        let changes = super::Changes::of(&held, &theirs);
        assert_eq!(changes.added.len(), MAXIMUM_LISTED);
        assert_eq!(changes.removed.len(), MAXIMUM_LISTED);
        assert!(changes.truncated);
    }

    /// Comparing a file with nothing is the state the view opens in, and it
    /// must be an answer rather than a panic.
    #[test]
    fn comparing_against_an_empty_list_of_bodies_answers() {
        let left = body(0x1000, &[PROLOGUE, RETURN]);
        let compared = functions(
            &[Body {
                start: 0x1000,
                name: Some("only"),
                instructions: &left,
            }],
            &[],
        );
        assert!(compared.pairs.is_empty());
        assert_eq!(compared.only_left, vec![0]);
        assert!(compared.only_right.is_empty());

        let nothing = functions(&[], &[]);
        assert!(nothing.pairs.is_empty());
    }

    /// An empty body is a real thing — a function symbol of size zero at the
    /// end of a section — and two of them are not evidence of anything.
    #[test]
    fn empty_bodies_do_not_pair_each_other_on_being_empty() {
        let compared = functions(
            &[
                Body {
                    start: 0x1000,
                    name: None,
                    instructions: &[],
                },
                Body {
                    start: 0x1100,
                    name: None,
                    instructions: &[],
                },
            ],
            &[
                Body {
                    start: 0x2000,
                    name: None,
                    instructions: &[],
                },
                Body {
                    start: 0x2100,
                    name: None,
                    instructions: &[],
                },
            ],
        );
        assert!(compared.pairs.is_empty());
    }

    /// The counts the view puts at the top of the comparison.
    #[test]
    fn the_tallies_count_what_the_pairs_say() {
        let kept = body(0x1000, &[PROLOGUE, RETURN]);
        let edited = body(0x1100, &[FRAME, RETURN]);
        let their_kept = body(0x1000, &[PROLOGUE, RETURN]);
        let their_edited = body(0x1100, &[FRAME, LEAVE, RETURN]);
        let compared = functions(
            &[
                Body {
                    start: 0x1000,
                    name: Some("kept"),
                    instructions: &kept,
                },
                Body {
                    start: 0x1100,
                    name: Some("edited"),
                    instructions: &edited,
                },
            ],
            &[
                Body {
                    start: 0x1000,
                    name: Some("kept"),
                    instructions: &their_kept,
                },
                Body {
                    start: 0x1100,
                    name: Some("edited"),
                    instructions: &their_edited,
                },
            ],
        );

        assert_eq!(compared.identical(), 1);
        assert_eq!(compared.changed(), 1);
        assert!(compared.pairs.iter().all(|pair| !pair.moved));
    }
}
