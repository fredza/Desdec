//! Desdec's own decompiler: from a decoded function to C.
//!
//! Until now this tool had two ways of showing a function as C, and neither
//! was a decompiler. One was a line-by-line translation — `rax = rbx;` under
//! `mov`, `if (/* jle condition from flags */) goto label_…;` under a branch —
//! which is the listing with C punctuation and hides nothing and reveals
//! nothing. The other was an external engine, `RetDec` or rz-ghidra: real
//! decompilers, and the right answer when one is installed, but they are
//! another program on the machine, they take a process and a deadline, they
//! publish no map from their output back to the listing, and they are not
//! there on the machine where the reader actually is.
//!
//! This is the third way, and it is the one that belongs in the tool: a
//! decompiler built out of the analysis Desdec already does. Six passes, each
//! its own module, each with its limits written down where it is implemented:
//!
//! 1. [`lift`] — one decoded instruction to the effects it has.
//! 2. [`naming`] — the calling convention: what the function was given, what
//!    its frame holds, what it answers, and what each call is passed. Before
//!    the dataflow pass, for the reason that module's documentation gives.
//! 3. [`dataflow`] — substitution and dead-store elimination, which is what
//!    makes eight statements into one line.
//! 4. [`structure`] — dominators and back edges, which is what makes forty
//!    `goto`s into a loop.
//! 5. [`emit`] — the C itself, one line at a time, each carrying the address
//!    it came from.
//!
//! [`ir`] holds the representation the five of them share.
//!
//! # What this is, and is not
//!
//! It is exact about what it states and silent about what it does not. Every
//! instruction the lifter does not model appears in the output as a comment
//! holding its own assembly, and [`emit::Decompiled::coverage`] says what
//! fraction of the body was understood, so the reader can weigh what they are
//! looking at rather than trust it evenly.
//!
//! It is not a replacement for Ghidra's decompiler, and it does not claim to
//! be. It does not recover types beyond the width of an access, and it lifts
//! x86-64 only — `AArch64` functions come out as their own listing inside a C
//! frame, which is what the pipeline does with anything it cannot read. Where
//! an external engine is installed and chosen it will do better on both
//! counts, and the preference that selects one is still there.
//!
//! Of SSE it reads the *moves*, which is most of what is there: sixteen,
//! thirty-two and sixty-four bytes copied about is what `memcpy`, every string
//! comparison and every vectorised loop are made of, and on an optimised
//! binary they were two thirds of everything this used to leave unread. The
//! packed *arithmetic* — `pand`, `pshufd`, `paddq` — is not lifted, and
//! neither are the partial moves such as `movss`, which write a quarter of a
//! register and leave the rest standing: this IR has no way to name the low
//! quarter of `%xmm0`, and claiming a whole-register assignment would be a
//! statement about twelve bytes nothing touched. The x87 stack is not modelled
//! at all.
//!
//! What it does have that no external engine does: it is always available, it
//! answers in microseconds rather than in a process and a deadline, it never
//! runs anything, and every line of its output knows which instruction it came
//! from.

pub mod dataflow;
pub mod emit;
pub mod ir;
pub mod lift;
pub mod naming;
pub mod structure;

use std::collections::HashMap;

use crate::{
    Analysis, Instruction,
    analysis::blocks::{self, Edge},
    decompiler::native::{
        emit::{Decompiled, Source},
        ir::{Expr, Statement, Stmt},
        naming::Convention,
    },
};

/// One function to decompile.
///
/// The body is passed in already bounded rather than worked out here: where a
/// function ends is a question the application answers once, for the Functions
/// view and the graph and this, and answering it twice is how two views come
/// to disagree about the same function.
pub struct Request<'a> {
    pub analysis: &'a Analysis,
    /// What to call it in the output.
    pub name: &'a str,
    /// Its first address.
    pub start: u64,
    /// Its decoded instructions, in address order.
    pub body: &'a [Instruction],
    /// The file's bytes, when they are to hand. Only used to read the strings
    /// an address points at, so that a call comes out as `puts("usage: …")`
    /// rather than as `puts(0x2004)`.
    pub file: Option<&'a [u8]>,
}

/// Decompiles one function.
///
/// Deterministic and self-contained: the same bytes give the same text, no
/// process is started, nothing is executed, and nothing is read from the
/// network or the disk beyond what the caller passed in.
#[must_use]
pub fn decompile(request: &Request<'_>) -> Decompiled {
    let architecture = request.analysis.summary.architecture;
    let convention = Convention::of(request.analysis.summary.format, architecture);
    let blocks = blocks::of(request.body);
    if blocks.is_empty() {
        return Decompiled {
            name: request.name.to_owned(),
            address: request.start,
            lines: Vec::new(),
            unmodelled: 0,
            instructions: 0,
        };
    }

    // One name lookup, shared by every instruction: a call's target, and the
    // address a `lea` computes.
    let names = Names {
        analysis: request.analysis,
        file: request.file,
    };
    let context = lift::Context {
        architecture,
        name_of: &|address| names.of(address),
    };

    let mut statements: Vec<Vec<Statement>> = blocks
        .iter()
        .map(|block| {
            request
                .body
                .get(block.instructions.clone())
                .unwrap_or_default()
                .iter()
                .flat_map(|instruction| lift::lift(instruction, &context))
                .collect()
        })
        .collect();

    // The prologue and the epilogue first, so the slots they use never become
    // locals with names of their own in the output.
    naming::strip_frame(&mut statements, convention);
    // Then the jumps that leave the function, which are calls and must be read
    // as calls before anything counts what a call is passed.
    tail_calls(&mut statements, request.body, &names, convention);
    // Then the arguments, because a call reading `rdi` is the evidence that
    // `rdi` is a parameter — so this has to have happened before the interface
    // is read off the body.
    naming::arguments_of_calls(&mut statements, convention);
    let naming = naming::recognise(&statements, convention);
    naming::apply(&naming, &mut statements);

    let index_of: HashMap<u64, usize> = blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.start, index))
        .collect();
    let successors: Vec<Vec<usize>> = blocks
        .iter()
        .map(|block| {
            block
                .successors
                .iter()
                .filter_map(|successor| index_of.get(&successor.address).copied())
                .collect()
        })
        .collect();
    dataflow::simplify(&mut statements, &successors, &naming.escaping());

    // The condition each block's own branch tests, read off the simplified IR
    // rather than off the instruction: by now `jle` has become the comparison
    // it belongs to, and that is what the `if` should say.
    let conditions: HashMap<usize, (Expr, u64)> = statements
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            block
                .iter()
                .rev()
                .find_map(|statement| match &statement.effect {
                    Stmt::Branch {
                        condition: Some(condition),
                        ..
                    } => Some((index, (condition.clone(), statement.address))),
                    _ => None,
                })
        })
        .collect();
    let structure = structure::recover(&blocks, &conditions);

    emit::write(&Source {
        name: request.name,
        address: request.start,
        blocks: &blocks,
        statements: &statements,
        naming: &naming,
        structure: &structure,
    })
}

/// Turns a jump that leaves the function into the call it is.
///
/// A compiler ends a function that finishes with a call by jumping to it
/// rather than calling it: the frame is already gone, and the called function
/// will return straight to this one's caller. `ls` does it —
/// `jmp 1000 <fflush_unlocked@plt>` — and read as an ordinary branch it came
/// out as `goto label_1000;`, naming a label that is nowhere in the function
/// and cannot be. That is C which does not compile, and worse, it says nothing
/// about what actually happens: the flow leaves for another function and does
/// not come back here.
///
/// So a branch whose target is outside this body becomes the call it is,
/// followed by the return that the jump performs on the callee's behalf.
/// [`naming::apply`] then gives the return its value like any other.
///
/// Only unconditional branches. A conditional one that leaves the function is
/// rare — a compiler tests locally and jumps away afterwards — and turning one
/// into a call would need a statement this IR does not have, so it keeps its
/// branch and [`emit`] gives it a label at the address it names.
fn tail_calls(
    statements: &mut [Vec<Statement>],
    body: &[Instruction],
    names: &Names<'_>,
    convention: naming::Convention,
) {
    let first = body.first().map(|instruction| instruction.address);
    let last = body.last().map(|instruction| instruction.address);
    let (Some(first), Some(last)) = (first, last) else {
        return;
    };
    let inside = |address: u64| {
        (first..=last).contains(&address)
            && body
                .binary_search_by(|instruction| instruction.address.cmp(&address))
                .is_ok()
    };
    let answer = convention
        .return_register()
        .map(|root| ir::Place::Register(ir::Register::new(root, ir::Width::Qword)));

    for block in statements.iter_mut() {
        let mut rewritten: Vec<Statement> = Vec::with_capacity(block.len() + 1);
        for statement in block.drain(..) {
            let callee = match &statement.effect {
                Stmt::Branch {
                    condition: None,
                    target,
                } if !inside(*target) => names
                    .of(*target)
                    .map_or(ir::Callee::Address(*target), ir::Callee::Named),
                // `jmp *0x24c20` — the stub the linker put in front of an
                // imported function, which is a tail call through the slot the
                // loader fills in. Only where the file names that slot: a jump
                // through a register is a switch table as often as it is a
                // call, and this must not guess between the two.
                Stmt::IndirectBranch(through) => {
                    let Some(name) = slot_named(through, names) else {
                        rewritten.push(statement);
                        continue;
                    };
                    ir::Callee::Named(name)
                }
                _ => {
                    rewritten.push(statement);
                    continue;
                }
            };
            let at = statement.address;
            rewritten.push(Statement::new(
                at,
                Stmt::Call {
                    result: answer.clone(),
                    callee,
                    arguments: Vec::new(),
                },
            ));
            rewritten.push(Statement::new(at, Stmt::Return(None)));
        }
        *block = rewritten;
    }
}

/// The imported name a jump goes through, when the file names it.
fn slot_named(through: &Expr, names: &Names<'_>) -> Option<String> {
    let ir::Expr::Read(place) = through else {
        return None;
    };
    let ir::Place::Memory { address, .. } = place.as_ref() else {
        return None;
    };
    let ir::Expr::Const { value, .. } = address.as_ref() else {
        return None;
    };
    names.analysis.import_at(*value).map(ToOwned::to_owned)
}

/// What the file calls an address.
struct Names<'a> {
    analysis: &'a Analysis,
    file: Option<&'a [u8]>,
}

impl Names<'_> {
    /// The strongest name the file gives an address.
    ///
    /// The imported name first — a call through a slot is a call to whatever
    /// the loader will write there, and that is the only thing that names it —
    /// then a symbol standing exactly at the address, then the string that
    /// lives there. Nothing is invented: an address the file says nothing
    /// about comes back as `None` and is printed as the number it is.
    fn of(&self, address: u64) -> Option<String> {
        if let Some(name) = self.analysis.import_at(address) {
            return Some(name.to_owned());
        }
        if let Some(symbol) = self
            .analysis
            .symbols
            .iter()
            .find(|symbol| symbol.address == Some(address) && !symbol.imported)
        {
            return Some(symbol.name.clone());
        }
        if let Some(name) = self.behind_a_stub(address) {
            return Some(name);
        }
        self.string_at(address).map(|text| format!("\"{text}\""))
    }

    /// The imported function a stub stands in front of.
    ///
    /// A call to a library function does not go to the library: it goes to a
    /// few bytes the linker put in this file, which jump through a slot the
    /// loader fills in. The call names the stub, the stub names the slot, and
    /// only the slot names the function — so without this step every call to
    /// `printf` in an ordinary dynamically linked program comes out as
    /// `function_400380`, which is the name of nothing.
    fn behind_a_stub(&self, address: u64) -> Option<String> {
        let index = self
            .analysis
            .instructions
            .binary_search_by(|instruction| instruction.address.cmp(&address))
            .ok()?;
        // A stub is a jump through a fixed address, sometimes preceded by the
        // `endbr64` a hardened build begins every reachable block with.
        let instruction = self
            .analysis
            .instructions
            .get(index..index + 2)?
            .iter()
            .find(|instruction| instruction.text.starts_with("jmp"))?;
        let target = instruction
            .text
            .split_once('*')?
            .1
            .trim()
            .strip_prefix("0x")?;
        let slot = u64::from_str_radix(target, 16).ok()?;
        self.analysis.import_at(slot).map(ToOwned::to_owned)
    }

    /// Printable text at an address, when the bytes there read as text.
    ///
    /// What turns `puts(0x2004)` into `puts("usage: …")`, which is often the
    /// single most informative thing in a decompiled function.
    fn string_at(&self, address: u64) -> Option<String> {
        let file = self.file?;
        let section = self
            .analysis
            .sections
            .iter()
            .filter(|section| section.is_mapped() && section.file_size > 0)
            .find(|section| {
                address >= section.virtual_address
                    && address < section.virtual_address.saturating_add(section.virtual_size)
            })?;
        let offset = usize::try_from(
            section
                .file_offset
                .saturating_add(address - section.virtual_address),
        )
        .ok()?;
        let bytes = file.get(offset..file.len().min(offset + STRING_LIMIT))?;
        let end = bytes.iter().position(|byte| *byte == 0)?;
        let text = bytes.get(..end)?;
        // Long enough to be text rather than a coincidence, and printable
        // throughout: two bytes that happen to be letters are not a string.
        if text.len() < 3 || !text.iter().all(|byte| is_printable(*byte)) {
            return None;
        }
        Some(escape(std::str::from_utf8(text).ok()?))
    }
}

/// How far a string is read before it is taken not to be one.
const STRING_LIMIT: usize = 96;

const fn is_printable(byte: u8) -> bool {
    matches!(byte, 0x20..=0x7E | b'\n' | b'\r' | b'\t')
}

/// The string as C would have to write it, so a newline in the data does not
/// become a newline in the output.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out
}

/// Where each line of a decompilation came from, for a view that wants to take
/// a click to the listing.
///
/// The whole reason the emitter carries an address on every line rather than
/// producing a string.
#[must_use]
pub fn line_addresses(decompiled: &Decompiled) -> Vec<Option<u64>> {
    decompiled.lines.iter().map(|line| line.address).collect()
}

/// Whether a block's edges name both arms of a test, which is what an `if`
/// needs and an indirect branch does not have.
#[must_use]
pub fn is_two_armed(block: &blocks::BasicBlock) -> bool {
    let taken = block
        .successors
        .iter()
        .any(|successor| successor.edge == Edge::Taken);
    let other = block
        .successors
        .iter()
        .any(|successor| matches!(successor.edge, Edge::NotTaken | Edge::FallThrough));
    taken && other
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Architecture, BinaryFormat};
    use std::sync::Arc;

    /// A function assembled by hand, so the test states what went in as
    /// clearly as what should come out.
    fn function(lines: &[(u64, &str)]) -> Vec<Instruction> {
        lines
            .iter()
            .map(|(address, text)| Instruction {
                address: *address,
                bytes: crate::InstructionBytes::new(&[0x90]).expect("one byte is an instruction"),
                text: (*text).to_owned(),
                section: Arc::from(".text"),
            })
            .collect()
    }

    /// A real ELF, so the summary these tests read is one the analysis
    /// produced rather than one written here — the body under test is
    /// assembled by hand, but the file it is said to come from is genuine.
    fn analysis() -> Analysis {
        let fixture = crate::fixtures::elf_x86_64();
        let analysis = crate::analyse_bytes(
            std::path::Path::new("fixture"),
            fixture.bytes.len() as u64,
            &fixture.bytes,
        );
        assert_eq!(analysis.summary.architecture, Architecture::X86_64);
        assert!(matches!(analysis.summary.format, BinaryFormat::Elf { .. }));
        analysis
    }

    fn decompiled(body: &[Instruction]) -> Decompiled {
        let analysis = analysis();
        decompile(&Request {
            analysis: &analysis,
            name: "example",
            start: body.first().map_or(0, |first| first.address),
            body,
            file: None,
        })
    }

    /// The function the old line-by-line translation could not read: a
    /// prologue, a comparison, a branch, two arms. What comes out should be an
    /// `if`, and the condition should be in the program's own terms.
    #[test]
    fn a_comparison_and_a_branch_become_an_if() {
        let body = function(&[
            (0x1000, "push %rbp"),
            (0x1001, "mov %rsp,%rbp"),
            (0x1004, "mov %edi,-0x4(%rbp)"),
            (0x1007, "cmpl $0xA,-0x4(%rbp)"),
            (0x100b, "jle 0x0000000000001016"),
            (0x100d, "mov $1,%eax"),
            (0x1012, "jmp 0x000000000000101b"),
            (0x1016, "mov $0,%eax"),
            (0x101b, "pop %rbp"),
            (0x101c, "ret"),
        ]);
        let text = decompiled(&body).text();
        // `local_4` and not `argument_1`: the argument was written to the
        // frame and it is the slot the comparison reads, which is what the
        // program does. A local is not substituted away — see
        // [`super::dataflow`] — precisely so that this line names it.
        assert!(
            text.contains("if (local_4 <= 10)"),
            "expected the comparison in the program's own terms, got:\n{text}"
        );
        assert!(
            !text.contains("goto"),
            "a test with two arms needs no goto:\n{text}"
        );
        assert!(
            !text.contains("condition from flags"),
            "the flags should not reach the output:\n{text}"
        );
    }

    /// The output used to name `eax` and declare nothing. Three quarters of
    /// the functions in an optimised binary referred to a variable that was
    /// nowhere: C that does not compile, and — worse for a reader — C in which
    /// `eax` and `rax` look like two things when the machine has one.
    #[test]
    fn a_register_that_survives_every_pass_is_declared_rather_than_named_from_nowhere() {
        let body = function(&[
            (0x1000, "push %rbp"),
            (0x1001, "mov %rsp,%rbp"),
            (0x1004, "mov %edi,-0x4(%rbp)"),
            (0x1007, "cmpl $0xA,-0x4(%rbp)"),
            (0x100b, "jle 0x0000000000001016"),
            (0x100d, "mov $1,%eax"),
            (0x1012, "jmp 0x000000000000101b"),
            (0x1016, "mov $0,%eax"),
            (0x101b, "pop %rbp"),
            (0x101c, "ret"),
        ]);
        let text = decompiled(&body).text();
        assert!(
            text.contains("eax"),
            "the register is what the two arms write:\n{text}"
        );
        assert!(
            text.contains("uint32_t eax;"),
            "and it must be declared where the frame is:\n{text}"
        );
    }

    /// A narrow write is not a narrow assignment. Writing `%al` clears none of
    /// the other fifty-six bits, and an output that says `al = 1` beside a
    /// `rax` it also names has quietly claimed otherwise.
    #[test]
    fn a_write_to_part_of_a_register_is_the_merge_the_machine_performs() {
        let body = function(&[
            (0x1000, "mov (%rdi),%rax"),
            (0x1003, "mov $1,%al"),
            (0x1005, "ret"),
        ]);
        let text = decompiled(&body).text();
        assert!(
            text.contains("uint64_t rax;"),
            "the widest window used decides the variable:\n{text}"
        );
        assert!(
            text.contains("rax = (rax & 0xffffffffffffff00) | (uint8_t)(1);"),
            "a byte write keeps the other seven:\n{text}"
        );
    }

    /// The other half of the same rule, and the one the architecture makes
    /// total: a 32-bit write *does* clear the top half, so the assignment
    /// alone is the whole truth and no mask belongs in front of it.
    #[test]
    fn a_thirty_two_bit_write_needs_no_mask_because_it_clears_the_rest() {
        let body = function(&[
            (0x1000, "mov (%rdi),%rax"),
            (0x1003, "mov $1,%eax"),
            (0x1005, "ret"),
        ]);
        let text = decompiled(&body).text();
        assert!(
            !text.contains("rax & 0x"),
            "nothing survives a 32-bit write to be merged with:\n{text}"
        );
    }

    /// A definition that reads the place it defines cannot be moved into a
    /// read below it. Found on a crackme whose key counter wrapped at seven:
    /// the output said `r8 = r8 + 1;` and then `if (r8 + 1 == 7)`, which tests
    /// the wrong number — the assignment stays, so by that line `r8` already
    /// holds the new value. Wrong C that reads exactly like right C is the one
    /// kind of output this decompiler must never produce, and a reader
    /// following those two lines derives the wrong answer.
    #[test]
    fn a_register_that_increments_itself_is_not_substituted_into_the_test_below_it() {
        // The counter has to come from *outside* the block for the bug to
        // exist at all: where the previous value is known here, the increment
        // is substituted with that value and says something true. It is the
        // loop that makes `r8` an unknown the block adds one to.
        let body = function(&[
            (0x1000, "xor %r8,%r8"),
            (0x1003, "cmp $0xA,%rsi"),
            (0x1007, "je 0x0000000000001020"),
            (0x1009, "inc %rsi"),
            (0x100c, "inc %r8"),
            (0x100f, "cmp $7,%r8"),
            (0x1013, "jne 0x0000000000001003"),
            (0x1015, "xor %r8,%r8"),
            (0x1018, "jmp 0x0000000000001003"),
            (0x1020, "ret"),
        ]);
        let text = decompiled(&body).text();
        // The increment itself is expected — `r8 = r8 + 1;` is the line the
        // instruction is. What must not appear is that expression *inside the
        // test*, where the register has already been incremented.
        assert!(
            !text.contains("if (r8 + 1"),
            "the increment must not be substituted into the test that follows it:\n{text}"
        );
        assert!(
            text.contains("if (r8 != 7)") || text.contains("if (r8 == 7)"),
            "the test is on the register as it stands by then:\n{text}"
        );
    }

    /// Every line that came from an instruction knows which one — the thing no
    /// external engine gives, and what lets a click go somewhere.
    #[test]
    fn every_line_from_an_instruction_carries_its_address() {
        let body = function(&[(0x1000, "mov $1,%eax"), (0x1005, "ret")]);
        let decompiled = decompiled(&body);
        let addresses: Vec<u64> = decompiled
            .lines
            .iter()
            .filter_map(|line| line.address)
            .collect();
        assert!(
            addresses.contains(&0x1005),
            "the return should map to the ret, got {addresses:x?}"
        );
        for address in &addresses {
            assert!(
                (0x1000..=0x1005).contains(address),
                "{address:#x} is not an address of this function"
            );
        }
    }

    /// Eight statements from a `cmp` and none of them in the output.
    #[test]
    fn the_registers_a_compiler_uses_as_scratch_do_not_reach_the_output() {
        // `rcx` rather than `rax`: the return register is live when a
        // function returns — nothing in the file says otherwise — so a value
        // left in it is not scratch and must not be removed. `rcx` is scratch
        // under this convention, and is what a compiler uses when it is not
        // computing a result.
        let body = function(&[
            (0x1000, "mov -0x8(%rbp),%rcx"),
            (0x1004, "add $1,%rcx"),
            (0x1008, "mov %rcx,-0x8(%rbp)"),
            (0x100c, "ret"),
        ]);
        let text = decompiled(&body).text();
        assert!(
            !text.contains("rcx ="),
            "the scratch register should have been substituted away:\n{text}"
        );
        assert!(
            text.contains("local_8 = local_8 + 1;"),
            "expected the increment as one statement, got:\n{text}"
        );
    }

    /// What is not modelled must be visible as not modelled, and countable.
    #[test]
    fn an_unmodelled_instruction_is_reported_rather_than_dropped() {
        let body = function(&[(0x1000, "fldt 0x1FDE0"), (0x1006, "ret")]);
        let decompiled = decompiled(&body);
        assert_eq!(decompiled.unmodelled, 1);
        assert!(decompiled.text().contains("fldt 0x1FDE0"));
        assert_eq!(decompiled.coverage(), Some(0.5));
    }

    /// A function taking nothing and answering nothing should say so, rather
    /// than declaring six arguments and an `int64_t` nobody should read.
    #[test]
    fn a_function_that_takes_and_answers_nothing_says_so() {
        let body = function(&[(0x1000, "ret")]);
        let text = decompiled(&body).text();
        assert!(text.starts_with("void example(void)"), "got:\n{text}");
    }

    #[test]
    fn a_body_with_nothing_in_it_decompiles_to_nothing_rather_than_panicking() {
        let decompiled = decompiled(&[]);
        assert!(decompiled.lines.is_empty());
        assert_eq!(decompiled.coverage(), None);
    }

    /// Every format and both architectures, decompiled end to end.
    ///
    /// A decompiler is handed hostile input by definition, and the one thing
    /// it may never do is stop the program that is reading it. This walks
    /// every function of every fixture — an ELF, a PE and an `AArch64` Mach-O
    /// — and asks only that each one comes back.
    #[test]
    fn every_fixture_decompiles_without_stopping() {
        for fixture in crate::fixtures::all() {
            let analysis = crate::analyse_bytes(
                std::path::Path::new(fixture.label),
                fixture.bytes.len() as u64,
                &fixture.bytes,
            );
            let mut starts: Vec<u64> = analysis
                .symbols
                .iter()
                .filter(|symbol| !symbol.imported)
                .filter_map(|symbol| symbol.address)
                .collect();
            starts.sort_unstable();
            starts.dedup();
            for (index, start) in starts.iter().enumerate() {
                let end = starts.get(index + 1).copied().unwrap_or(u64::MAX);
                let span = analysis.instruction_span(*start..end);
                let body = &analysis.instructions[span];
                let decompiled = decompile(&Request {
                    analysis: &analysis,
                    name: "function",
                    start: *start,
                    body,
                    file: Some(&fixture.bytes),
                });
                assert!(
                    body.is_empty() || !decompiled.lines.is_empty(),
                    "{} at {start:#x} decompiled to nothing from {} instructions",
                    fixture.label,
                    body.len()
                );
            }
        }
    }

    /// `AArch64` is not lifted yet, and what that must look like is a function
    /// whose body is its own listing in comments — not an empty one, and not a
    /// plausible-looking translation of instructions nobody read.
    #[test]
    fn an_unlifted_architecture_comes_out_as_its_own_listing_and_says_so() {
        let fixture = crate::fixtures::mach_o_arm64();
        let analysis = crate::analyse_bytes(
            std::path::Path::new("aarch64"),
            fixture.bytes.len() as u64,
            &fixture.bytes,
        );
        if analysis.instructions.is_empty() {
            return;
        }
        let start = analysis.instructions[0].address;
        let decompiled = decompile(&Request {
            analysis: &analysis,
            name: "function",
            start,
            body: &analysis.instructions,
            file: Some(&fixture.bytes),
        });
        assert_eq!(
            decompiled.unmodelled, decompiled.instructions,
            "no AArch64 instruction is lifted yet, so every one of them should say so"
        );
        assert_eq!(
            decompiled.coverage(),
            Some(0.0),
            "and the view must be told that none of it was understood"
        );
        assert!(
            decompiled.text().contains("/* not modelled:"),
            "the instructions themselves must still be there"
        );
    }

    /// A compiler ends a function that finishes with a call by jumping to it.
    /// Read as an ordinary branch it came out as `goto label_1000;`, naming a
    /// label that is nowhere in the function — C that does not compile, and
    /// that says nothing about the flow leaving for good.
    #[test]
    fn a_jump_that_leaves_the_function_is_the_call_it_is() {
        let body = function(&[
            (0x1000, "mov 0x24f80,%rax"),
            (0x1007, "mov (%rax),%rdi"),
            (0x100a, "jmp 0x0000000000000fe0"),
        ]);
        let text = decompiled(&body).text();
        assert!(
            !text.contains("goto"),
            "a tail call is a call, not a jump to nowhere:\n{text}"
        );
        assert!(
            text.contains("function_fe0("),
            "and it must name where it goes:\n{text}"
        );
        assert!(
            text.contains("return "),
            "the jump returns on the callee's behalf:\n{text}"
        );
    }

    /// The distinction that makes the rule safe: a jump *inside* the function
    /// is flow this decompiler structures, and must not be turned into a call.
    #[test]
    fn a_jump_inside_the_function_stays_flow() {
        let body = function(&[
            (0x1000, "mov $1,%eax"),
            (0x1005, "jmp 0x000000000000100c"),
            (0x100a, "mov $2,%eax"),
            (0x100c, "ret"),
        ]);
        let text = decompiled(&body).text();
        assert!(
            !text.contains("function_100c"),
            "a local jump is not a call:\n{text}"
        );
    }

    /// Every `goto` the output prints must name a label the output has.
    /// Anything else is C that does not compile, and a reader cannot tell
    /// which of the two kinds of mistake they are looking at.
    #[test]
    fn no_goto_names_a_label_that_is_not_there() {
        for fixture in crate::fixtures::all() {
            let analysis = crate::analyse_bytes(
                std::path::Path::new(fixture.label),
                fixture.bytes.len() as u64,
                &fixture.bytes,
            );
            let mut starts: Vec<u64> = analysis
                .symbols
                .iter()
                .filter(|symbol| !symbol.imported)
                .filter_map(|symbol| symbol.address)
                .collect();
            starts.sort_unstable();
            starts.dedup();
            for (index, start) in starts.iter().enumerate() {
                let end = starts.get(index + 1).copied().unwrap_or(u64::MAX);
                let span = analysis.instruction_span(*start..end);
                let decompiled = decompile(&Request {
                    analysis: &analysis,
                    name: "function",
                    start: *start,
                    body: &analysis.instructions[span],
                    file: Some(&fixture.bytes),
                });
                let text = decompiled.text();
                for line in text.lines() {
                    let Some(target) = line
                        .trim()
                        .strip_prefix("goto ")
                        .or_else(|| line.split_once(") goto ").map(|(_, rest)| rest))
                    else {
                        continue;
                    };
                    // `goto *…` is a jump through a value, not a jump to a
                    // label: it names no label and needs none.
                    if target.starts_with('*') {
                        continue;
                    }
                    let target = target.trim_end_matches(';');
                    assert!(
                        text.contains(&format!("{target}:")),
                        "{} at {start:#x} jumps to {target}, which is nowhere in:\n{text}",
                        fixture.label
                    );
                }
            }
        }
    }

    /// The loop is the shape a reader is looking for, and the whole reason the
    /// structurer exists.
    #[test]
    fn a_back_edge_comes_out_as_a_loop() {
        let body = function(&[
            (0x1000, "mov $0,%eax"),
            (0x1005, "cmp $0xA,%eax"),
            (0x1008, "jge 0x0000000000001012"),
            (0x100a, "add $1,%eax"),
            (0x100d, "jmp 0x0000000000001005"),
            (0x1012, "ret"),
        ]);
        let text = decompiled(&body).text();
        assert!(text.contains("while ("), "expected a loop, got:\n{text}");
    }
}
