//! Who names an address.
//!
//! "What is here?" the listing answers by itself. "Who gets here?" it cannot:
//! the call that reaches a function may be a hundred thousand rows away, and
//! the pointer that holds its address is not code at all. This index answers
//! the second question for every address in the file, built once when the
//! binary is opened.
//!
//! Three strengths of answer, and they are kept apart. An instruction that
//! computes an address is arithmetic on decoded bytes and is exact — including
//! the ones that take two instructions to say it, which is every address an
//! `AArch64` file names. A branch that reaches its target through something
//! the file states — a table of cases, the stub standing in front of an
//! imported function — is exact too, but the instruction does not name the
//! address, so it is reported as what it is. And a word in a data section that
//! happens to hold a value inside the image is a *likely* pointer — a vtable
//! entry, a relocation — which may also be a number that looks like one, so it
//! is never merged with the calls.

use desdec_core::{Analysis, Architecture, Instruction, operand};

/// How an address is named.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Kind {
    /// A call: the flow goes there and is expected back.
    Call,
    /// A branch: the flow goes there.
    Jump,
    /// An instruction that computes the address without going to it — a `lea`
    /// taking the address of a string, a load from a table.
    Reads,
    /// A branch that arrives here by reading a table of targets: one arm of
    /// the `switch` the compiler turned into `jmp *%rax`.
    Table,
    /// A call that arrives here through the stub the linker put in front of an
    /// imported function.
    Stub,
    /// A word in a data section holding this address.
    Pointer,
}

/// One reference, and what kind it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reference {
    /// Where the reference is: an instruction, or the word holding it.
    pub from: u64,
    pub kind: Kind,
}

/// Every reference in the file, by the address it names.
#[derive(Clone, Debug, Default)]
pub struct Index {
    /// `(target, from, kind)`, sorted by target so one address's references
    /// are a contiguous run the search can bisect to.
    entries: Vec<(u64, u64, Kind)>,
}

impl Index {
    /// Builds the index for one binary. Done once when it is opened: the scan
    /// walks every decoded instruction and every data word, and follows the
    /// branches that name no address — through the table of cases they read,
    /// and through the stub standing in front of an imported function.
    #[must_use]
    pub fn of(analysis: &Analysis, file: &[u8]) -> Self {
        let mut entries: Vec<(u64, u64, Kind)> = Vec::new();
        let mut stubs: Vec<(u64, u64)> = Vec::new();
        for (index, instruction) in analysis.instructions.iter().enumerate() {
            // An address `AArch64` builds out of two instructions. Nothing
            // else in this loop sees it: the page is an immediate and so is
            // the offset, and a file for that architecture had no data
            // references whatever until the pair was put back together.
            for (from, target) in operand::page_formed(&analysis.instructions, index) {
                if analysis.section_at(target).is_some() {
                    entries.push((target, from, Kind::Reads));
                }
            }
            stub(analysis, index, &mut stubs);
            // Only a value that lands somewhere in the image is an address.
            // Without this the index listed every mask and every large
            // constant — `and $0xffffffff0000,%rax` is not a reference to
            // anything, and a reader asking who reaches an address should not
            // have to sort arithmetic out of the answer.
            // A branch's target as well as an ordinary operand's: AArch64
            // writes every branch target as an immediate, which the general
            // reader refuses on purpose, so the index held no branches at all
            // on those files.
            let Some(target) = operand::branch_target(instruction)
                .or_else(|| operand::target_address(instruction))
                .filter(|at| analysis.section_at(*at).is_some())
            else {
                continue;
            };
            entries.push((target, instruction.address, kind_of(instruction)));
        }
        pointers(analysis, file, &mut entries);
        tables(analysis, file, &mut entries);
        through_stubs(&mut stubs, &mut entries);
        entries.sort_unstable();
        entries.dedup();
        entries.shrink_to_fit();
        Self { entries }
    }

    /// The references naming `target`, as one contiguous run of the table.
    fn span(&self, target: u64) -> &[(u64, u64, Kind)] {
        let start = self.entries.partition_point(|(at, _, _)| *at < target);
        let end = self.entries.partition_point(|(at, _, _)| *at <= target);
        &self.entries[start..end]
    }

    /// Everything that names `target`, in address order.
    pub fn to(&self, target: u64) -> impl Iterator<Item = Reference> + '_ {
        self.span(target).iter().map(|(_, from, kind)| Reference {
            from: *from,
            kind: *kind,
        })
    }

    #[must_use]
    pub fn count(&self, target: u64) -> usize {
        self.span(target).len()
    }
}

/// What an instruction does with the address it names.
fn kind_of(instruction: &Instruction) -> Kind {
    // Past the prefixes: `bnd jmpq *0x3f61` is the jump every procedure table
    // entry is written as, and reading `bnd` as its mnemonic filed all of them
    // under "computes an address".
    let mnemonic = operand::mnemonic(&instruction.text);
    if mnemonic.starts_with("call") || matches!(mnemonic, "bl" | "blr") {
        return Kind::Call;
    }
    if mnemonic.starts_with('j')
        || mnemonic.starts_with("b.")
        || matches!(mnemonic, "b" | "br" | "cbz" | "cbnz" | "tbz" | "tbnz")
    {
        return Kind::Jump;
    }
    Kind::Reads
}

/// How far a stub or a table is looked for around the branch that uses it.
const REACH: usize = 12;

/// The most cases one table of targets is read for, however many the code
/// claims it has.
///
/// A `switch` with a thousand arms is already unusual; a "table" longer than
/// this is a stretch of data being read as one.
const TABLE_CASES: usize = 1024;

/// The stub standing in front of an imported function, when the instruction at
/// `index` is the branch that ends one.
///
/// Nothing in a file calls an imported function. It calls a stub of two or
/// three instructions, and the stub branches through the slot the loader is to
/// write the real address into — so "who calls `malloc`?", which is the
/// question a reader asks of an import, answered with a single reference: the
/// stub's own jump. The callers it stands for were nowhere.
///
/// What is recorded here is the address a call has to name to reach the stub,
/// against the slot the stub reads. The signal is the file's own word rather
/// than a shape: the branch must read a slot the file declares as an import.
fn stub(analysis: &Analysis, index: usize, stubs: &mut Vec<(u64, u64)>) {
    let instructions = &analysis.instructions;
    let Some(instruction) = instructions.get(index) else {
        return;
    };
    let mnemonic = operand::mnemonic(&instruction.text);

    // What an `AArch64` linker writes: the page of the slot, the load from it,
    // and the branch through what was loaded.
    if mnemonic == "adrp" {
        let Some(slot) = operand::page_formed(instructions, index)
            .into_iter()
            .map(|(_, target)| target)
            .find(|target| analysis.import_at(*target).is_some())
        else {
            return;
        };
        let branches = instructions
            .iter()
            .skip(index + 1)
            .take(REACH)
            .any(|next| matches!(operand::mnemonic(&next.text), "br" | "braa" | "braaz"));
        if branches {
            stubs.push((instruction.address, slot));
        }
        return;
    }

    // And what an x86 one writes: one jump through the slot. A jump and never
    // a call — a stub leaves for the import and does not expect to come back,
    // and a `call *slot` is an ordinary call site rather than a stub.
    if !mnemonic.starts_with("jmp") || !instruction.text.contains('*') {
        return;
    }
    let Some(slot) = operand::target_address(instruction) else {
        return;
    };
    if analysis.import_at(slot).is_none() {
        return;
    }
    stubs.push((instruction.address, slot));

    // A call names the first byte of the entry, and that is the marker the
    // compiler opens it with rather than the branch: `endbr64; bnd jmp *…`.
    let mut expected = instruction.address;
    for previous in instructions[..index].iter().rev().take(REACH) {
        if previous.address.saturating_add(previous.bytes.len() as u64) != expected {
            break;
        }
        let mnemonic = operand::mnemonic(&previous.text);
        if !mnemonic.starts_with("nop") && !matches!(mnemonic, "endbr64" | "endbr32" | "int3") {
            break;
        }
        expected = previous.address;
        stubs.push((previous.address, slot));
    }
}

/// Everything that reaches an import, by way of the stub in front of it.
///
/// The calls are already indexed against the stub, which is where they go; put
/// against the slot as well, they answer the question that was asked.
fn through_stubs(stubs: &mut Vec<(u64, u64)>, entries: &mut Vec<(u64, u64, Kind)>) {
    if stubs.is_empty() {
        return;
    }
    stubs.sort_unstable();
    stubs.dedup();

    // The length is taken first: what is appended here is a reference to a
    // slot, and a slot is not a stub, so nothing added can be forwarded again.
    let named = entries.len();
    for position in 0..named {
        let (target, from, kind) = entries[position];
        if !matches!(kind, Kind::Call | Kind::Jump) {
            continue;
        }
        let first = stubs.partition_point(|(entry, _)| *entry < target);
        for (_, slot) in stubs[first..]
            .iter()
            .take_while(|(entry, _)| *entry == target)
        {
            entries.push((*slot, from, Kind::Stub));
        }
    }
}

/// Where an indirect branch goes, when the file states a table of answers.
///
/// A `switch` compiles to `jmp *%rax`, and the listing stops dead there: the
/// reader asking where a case is handled gets nothing, and the arms of the
/// switch are code that nothing appears to reach. The table is in the file
/// though, and the instruction that named it stands a few lines above the
/// branch, so the answers are all there in the order the compiler wrote them.
///
/// What is claimed is bounded on purpose. The table must be in data, the code
/// must say how many cases it has, each of them must land in executable code,
/// and two must do so before any is believed — a string that happens to be
/// read a few instructions before an indirect branch reads as no table at all.
fn tables(analysis: &Analysis, file: &[u8], entries: &mut Vec<(u64, u64, Kind)>) {
    for (index, instruction) in analysis.instructions.iter().enumerate() {
        let mnemonic = operand::mnemonic(&instruction.text);
        let indirect =
            mnemonic == "br" || (mnemonic.starts_with("jmp") && instruction.text.contains('*'));
        if !indirect {
            continue;
        }
        let Some(base) = table_base(analysis, index) else {
            continue;
        };
        // Where the table ends is the compiler's word, never a guess: see
        // [`stated_bound`] for what reading past the end produced.
        let Some(bound) = stated_bound(&analysis.instructions, index) else {
            continue;
        };
        for target in cases(analysis, file, base, bound) {
            entries.push((target, instruction.address, Kind::Table));
        }
    }
}

/// The address an indirect branch reads its cases from.
fn table_base(analysis: &Analysis, index: usize) -> Option<u64> {
    let instructions = &analysis.instructions;
    let instruction = instructions.get(index)?;
    // The branch may name the table itself: `jmpq *0x402000(,%rax,8)`.
    if let Some(base) = indexed_base(&instruction.text) {
        return in_data(analysis, base);
    }
    // `jmpq *0x3f61` reads one word, and one word is a slot, not a table.
    if operand::target_address(instruction).is_some() {
        return None;
    }
    // Otherwise the instruction that put the table in a register stands close
    // above, in the same unbroken run of code.
    let mut expected = instruction.address;
    for (back, previous) in instructions[..index].iter().enumerate().rev().take(REACH) {
        if previous.address.saturating_add(previous.bytes.len() as u64) != expected {
            break;
        }
        expected = previous.address;
        if let Some(base) = operand::target_address(previous).and_then(|at| in_data(analysis, at)) {
            return Some(base);
        }
        if let Some(base) = operand::page_formed(instructions, back)
            .into_iter()
            .find_map(|(_, at)| in_data(analysis, at))
        {
            return Some(base);
        }
    }
    None
}

/// How many cases the code says the table has, and nothing when it does not
/// say.
///
/// Tables are packed one after another in the same section and nothing marks
/// where one ends, so a read that stops only when a word fails to land in code
/// runs straight on into the next table — whose offsets, read against the
/// wrong base, land in code perfectly well and point at arms of a switch that
/// has nothing to do with this one. Four fifths of what an unbounded read
/// found in a large binary was that.
///
/// The compiler does state the length, in the bound it checks before it
/// dispatches: `cmp $0x67,%sil; ja default` says there are 0x68 cases. Where
/// it is not there to be read, no table is claimed: the alternative is a
/// confident answer that sends the reader into another function.
fn stated_bound(instructions: &[Instruction], index: usize) -> Option<usize> {
    let mut expected = instructions.get(index)?.address;
    for previous in instructions[..index].iter().rev().take(REACH) {
        if previous.address.saturating_add(previous.bytes.len() as u64) != expected {
            break;
        }
        expected = previous.address;
        if !operand::mnemonic(&previous.text).starts_with("cmp") {
            continue;
        }
        // The bound is the immediate, in either syntax: `$0x67` and `#0x67`.
        let highest = previous
            .text
            .split(|c: char| c == ',' || c.is_whitespace())
            .find_map(|word| {
                let digits = word.strip_prefix('$').or_else(|| word.strip_prefix('#'))?;
                digits
                    .strip_prefix("0x")
                    .and_then(|hexadecimal| u64::from_str_radix(hexadecimal, 16).ok())
                    .or_else(|| digits.parse().ok())
            })?;
        // The check is on the highest index that has a case, so the count is
        // one more than it.
        return usize::try_from(highest.saturating_add(1))
            .ok()
            .map(|cases| cases.min(TABLE_CASES));
    }
    None
}

/// The address, when it is in a mapped section that holds data rather than
/// code.
fn in_data(analysis: &Analysis, address: u64) -> Option<u64> {
    analysis
        .section_at(address)
        .filter(|section| !section.permissions.execute)
        .map(|_| address)
}

/// The targets a table holds, in the two shapes a compiler writes them.
///
/// Position-independent code stores each case as a signed offset from the
/// table itself, and everything else stores the address. Both are read, and
/// the one that answers for more cases is the one the table is in.
fn cases(analysis: &Analysis, file: &[u8], base: u64, bound: usize) -> Vec<u64> {
    let relative = read_cases(analysis, file, base, 4, bound, |base, word| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "a four-byte case is a signed offset, and is read as one"
        )]
        let offset = i64::from(word as u32 as i32);
        base.checked_add_signed(offset)
    });
    let absolute = read_cases(
        analysis,
        file,
        base,
        word_width(analysis),
        bound,
        |_, word| Some(word),
    );
    if absolute.len() > relative.len() {
        absolute
    } else {
        relative
    }
}

/// Reads a table one way, and stops the moment it stops reading as one.
fn read_cases(
    analysis: &Analysis,
    file: &[u8],
    base: u64,
    width: usize,
    bound: usize,
    resolve: impl Fn(u64, u64) -> Option<u64>,
) -> Vec<u64> {
    let mut found = Vec::new();
    for step in 0..bound {
        let Ok(within) = u64::try_from(step * width) else {
            break;
        };
        let Some(at) = base.checked_add(within) else {
            break;
        };
        let Some(offset) = analysis
            .file_offset_of(at)
            .and_then(|offset| usize::try_from(offset).ok())
        else {
            break;
        };
        let Some(word) = file
            .get(offset..offset.saturating_add(width))
            .filter(|word| word.len() == width)
        else {
            break;
        };
        let value = word
            .iter()
            .rev()
            .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
        let lands = resolve(base, value)
            .and_then(|target| analysis.section_at(target).map(|section| (target, section)))
            .filter(|(_, section)| section.permissions.execute);
        let Some((target, _)) = lands else {
            break;
        };
        found.push(target);
    }
    // One case is not a table. Two in a row, each landing in code, is a shape
    // that does not happen by accident.
    if found.len() < 2 {
        found.clear();
    }
    found
}

/// The table an indexed operand reads: the `0x402000` of `*0x402000(,%rax,8)`.
///
/// Written out because the general reader refuses it, and rightly: a number in
/// front of a bracket is a displacement from a register everywhere else.
fn indexed_base(text: &str) -> Option<u64> {
    let after = text.split_once("*0x")?.1;
    let digits: String = after.chars().take_while(char::is_ascii_hexdigit).collect();
    if !after[digits.len()..].starts_with('(') {
        return None;
    }
    u64::from_str_radix(&digits, 16).ok()
}

/// How wide an address is in this file.
fn word_width(analysis: &Analysis) -> usize {
    match analysis.summary.architecture {
        Architecture::X86 | Architecture::Arm => 4,
        Architecture::X86_64 | Architecture::Arm64 | Architecture::Unknown => 8,
    }
}

/// Words in data that hold an address inside the image.
///
/// Only sections that are mapped and not executable — code is read as code,
/// and a byte sequence in the middle of an instruction stream that happens to
/// look like an address is noise. Only aligned words, for the same reason:
/// an unaligned scan finds a "pointer" every few bytes in any dense data.
fn pointers(analysis: &Analysis, file: &[u8], entries: &mut Vec<(u64, u64, Kind)>) {
    let width = word_width(analysis);
    for section in &analysis.sections {
        if section.permissions.execute || !section.is_mapped() {
            continue;
        }
        let Some(bytes) = section.bytes_in(file) else {
            continue;
        };
        for (index, word) in bytes.chunks_exact(width).enumerate() {
            let value = word
                .iter()
                .rev()
                .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
            if value == 0 || analysis.section_at(value).is_none() {
                continue;
            }
            let Ok(within) = u64::try_from(index * width) else {
                continue;
            };
            let at = section.virtual_address.saturating_add(within);
            entries.push((value, at, Kind::Pointer));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One fixture's analysis, and the bytes it was read from.
    ///
    /// A fixture rather than the host's own binary: what these tests are about
    /// is a listing of their own making, and the fixture supplies the one
    /// thing they cannot make up — a real section table to place it in.
    fn fixture(label: &str) -> (Analysis, Vec<u8>) {
        let sample = crate::testing::samples()
            .into_iter()
            .find(|sample| sample.fixture.label == label)
            .expect("every fixture is named");
        (sample.analysis, sample.fixture.bytes)
    }

    /// A listing written out as text, at the addresses the test wants it at.
    fn listing(lines: &[(u64, &str, usize)]) -> Vec<Instruction> {
        lines
            .iter()
            .map(|(address, text, length)| Instruction {
                address: *address,
                bytes: desdec_core::InstructionBytes::new(&vec![0x90; *length])
                    .expect("test instructions are short"),
                text: (*text).to_owned(),
                section: std::sync::Arc::from(".text"),
            })
            .collect()
    }

    /// `AArch64` has no instruction wide enough to hold an address, so before
    /// the pair was read as one thing, a file for it had no data references at
    /// all — not one string, not one global.
    #[test]
    fn an_aarch64_address_is_indexed_although_it_takes_two_instructions_to_say() {
        let (mut analysis, bytes) = fixture("Mach-O arm64");
        analysis.instructions = listing(&[
            (0x1_0000_0170, "adrp x0, #0x100000000", 4),
            (0x1_0000_0174, "add x0, x0, #0x190", 4),
        ]);
        let index = Index::of(&analysis, &bytes);

        assert_eq!(
            index.to(0x1_0000_0190).collect::<Vec<_>>(),
            vec![Reference {
                from: 0x1_0000_0174,
                kind: Kind::Reads,
            }],
            "the pair states one address, against the instruction that finishes it"
        );
    }

    /// The question a reader asks of an import is who calls it, and the answer
    /// used to be its own stub and nothing else.
    #[test]
    fn a_call_to_an_imported_function_is_indexed_against_the_slot_it_goes_through() {
        let (mut analysis, bytes) = fixture("PE x86-64");
        let slot = analysis
            .import_slots
            .first()
            .expect("the PE fixture imports by name")
            .address;
        analysis.instructions = listing(&[
            (0x1_4000_1000, "call 0x140001010", 5),
            (0x1_4000_1010, "endbr64", 4),
            (0x1_4000_1014, &format!("bnd jmpq *{slot:#x}"), 7),
        ]);
        let index = Index::of(&analysis, &bytes);
        let references: Vec<Reference> = index.to(slot).collect();

        assert!(
            references.contains(&Reference {
                from: 0x1_4000_1000,
                kind: Kind::Stub,
            }),
            "the caller reaches the import through the stub: {references:?}"
        );
        assert!(
            references.contains(&Reference {
                from: 0x1_4000_1014,
                kind: Kind::Jump,
            }),
            "and the stub's own jump is a jump, prefix and all: {references:?}"
        );
    }

    /// Where the arms of a `switch` are handled. The branch names a register,
    /// so the listing stopped there and the arms were code nothing reached.
    #[test]
    fn the_arms_of_a_switch_are_indexed_against_the_branch_that_dispatches_them() {
        let (mut analysis, mut bytes) = fixture("ELF x86-64");
        let base = 0x0040_02cc_u64;
        let arms = [0x0040_02b0_u64, 0x0040_02b4];
        let at = usize::try_from(
            analysis
                .file_offset_of(base)
                .expect("the table is in a section the file stores"),
        )
        .expect("a fixture is small");
        for (step, arm) in arms.iter().enumerate() {
            let distance = i64::try_from(*arm).expect("a fixture address is small")
                - i64::try_from(base).expect("a fixture address is small");
            let offset = i32::try_from(distance).expect("a short jump");
            bytes[at + step * 4..at + step * 4 + 4].copy_from_slice(&offset.to_le_bytes());
        }
        // With the bound the compiler checks before it dispatches: without it
        // nothing says where the table ends, and nothing is claimed.
        analysis.instructions = listing(&[
            (0x0040_02b0, "cmp $0x1,%eax", 3),
            (0x0040_02b3, "ja 0x00000000004002c0", 5),
            (0x0040_02b8, &format!("jmpq *{base:#x}(,%rax,8)"), 7),
        ]);

        let index = Index::of(&analysis, &bytes);
        for arm in arms {
            assert!(
                index.to(arm).any(|reference| reference
                    == Reference {
                        from: 0x0040_02b8,
                        kind: Kind::Table,
                    }),
                "{arm:#x} is one of the cases the table holds"
            );
        }
    }

    /// And what is not a table is not read as one: two cases landing in code,
    /// or nothing is claimed.
    #[test]
    fn a_stretch_of_data_read_a_moment_before_a_branch_is_not_a_table() {
        let (mut analysis, bytes) = fixture("ELF x86-64");
        analysis.instructions = listing(&[
            (0x0040_02b0, "lea 0x4002c9,%rdx", 7),
            (0x0040_02b7, "jmpq *%rax", 2),
        ]);
        let index = Index::of(&analysis, &bytes);

        assert!(
            analysis
                .instructions
                .iter()
                .all(|instruction| index.count(instruction.address) == 0),
            "a string is not a list of cases"
        );
    }

    /// A call, a jump and a load are three different answers to "who gets
    /// here?", and a reader following a flow needs to know which they have.
    #[test]
    fn an_instruction_is_read_for_what_it_does_with_the_address() {
        let instruction = |text: &str| Instruction {
            address: 0x1000,
            bytes: desdec_core::InstructionBytes::new(&[0x90]).expect("one byte"),
            text: text.to_owned(),
            section: std::sync::Arc::from(".text"),
        };

        assert_eq!(kind_of(&instruction("callq 0x401230")), Kind::Call);
        assert_eq!(kind_of(&instruction("bl 0x401230")), Kind::Call);
        assert_eq!(kind_of(&instruction("jne 0x401230")), Kind::Jump);
        assert_eq!(kind_of(&instruction("b.eq 0x401230")), Kind::Jump);
        assert_eq!(kind_of(&instruction("lea 0x2f61(%rip),%rdi")), Kind::Reads);
    }

    /// The whole point of the index: an address is reached from somewhere, and
    /// the somewhere may be anywhere in the file.
    #[test]
    fn the_index_finds_who_calls_a_real_function() {
        let analysis = crate::testing::reference_analysis();
        let index = Index::of(analysis, crate::testing::reference_bytes());
        let Some((target, from)) = analysis.instructions.iter().find_map(|instruction| {
            let mnemonic = instruction.text.split_whitespace().next()?;
            if !mnemonic.starts_with("call") && mnemonic != "bl" {
                return None;
            }
            let target = operand::branch_target(instruction)?;
            Some((target, instruction.address))
        }) else {
            return; // Nothing on this host calls a fixed address.
        };

        let references: Vec<Reference> = index.to(target).collect();
        assert!(
            references
                .iter()
                .any(|reference| reference.from == from && reference.kind == Kind::Call),
            "the call must be indexed against the address it calls"
        );
    }

    /// An address nothing names has no references, rather than the nearest
    /// ones: an answer rounded to the neighbouring run would send a reader to
    /// a function that has nothing to do with what they asked about.
    #[test]
    fn a_lookup_answers_for_one_address_and_never_its_neighbours() {
        let index = Index {
            entries: vec![
                (0x1000, 0x400, Kind::Call),
                (0x1000, 0x500, Kind::Jump),
                (0x2000, 0x600, Kind::Call),
            ],
        };

        assert_eq!(index.count(0x1000), 2);
        assert_eq!(
            index.count(0x1800),
            0,
            "between two runs is neither of them"
        );
        assert_eq!(index.count(0x2000), 1);
        assert_eq!(
            index.to(0x2000).next(),
            Some(Reference {
                from: 0x600,
                kind: Kind::Call
            })
        );
    }

    /// A mask is not a reference: the index only claims values that land
    /// somewhere in the image.
    #[test]
    fn a_constant_that_is_not_an_address_is_not_a_reference() {
        let analysis = crate::testing::reference_analysis();
        let index = Index::of(analysis, crate::testing::reference_bytes());

        for target in [0xffff_ffff_0000_u64, 0xffff_ffff_ffff_f000] {
            if analysis.section_at(target).is_some() {
                continue; // Improbable, but this host would make it a real one.
            }
            assert_eq!(index.count(target), 0, "{target:#x} is arithmetic");
        }
    }
}
