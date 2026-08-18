//! What the assistant is told, assembled from what Desdec decoded.
//!
//! Every line here comes from the analysis: instructions as they were
//! disassembled, symbol names as the file spells them, entropies as they were
//! measured. Nothing is guessed on the way out, so a wrong answer is the
//! model's and not something this code invented for it — and the whole text is
//! shown to the reader before it is sent, which is only meaningful because it
//! is assembled in one readable place.
//!
//! The binary itself never goes anywhere. What leaves is the listing a reader
//! is already looking at.

use std::fmt::Write as _;

use crate::analysis::{Analysis, entropy};

/// What the reader wants read for them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Question {
    /// Where to start on this binary, from what the analysis already knows.
    Binary,
    /// What this function does.
    Function { address: u64 },
    /// What this instruction and the ones around it are doing.
    Instruction { address: u64 },
}

/// The two halves of a request, as every provider here wants them.
#[derive(Clone, Debug)]
pub struct Prompt {
    pub system: String,
    pub user: String,
}

/// Instructions of a function beyond which the answer stops improving and the
/// request only gets more expensive.
const MAXIMUM_BODY: usize = 400;
/// Instructions either side of the one being asked about.
const CONTEXT_WINDOW: usize = 12;
/// Strings and libraries listed when describing the whole binary.
const MAXIMUM_FACTS: usize = 40;
/// Longest string quoted, so one enormous blob cannot fill the request.
const MAXIMUM_STRING: usize = 120;

/// Builds the request for a question about this binary.
///
/// `reply_in` is the language the answer should be written in, named in that
/// language — the interface passes its own, so a French reader is not handed
/// an English paragraph about their binary.
#[must_use]
pub fn build(analysis: &Analysis, question: Question, reply_in: &str) -> Prompt {
    Prompt {
        system: system(reply_in),
        user: match question {
            Question::Binary => binary(analysis),
            Question::Function { address } => function(analysis, address),
            Question::Instruction { address } => instruction(analysis, address),
        },
    }
}

/// The rules the answer has to hold to.
///
/// These are Desdec's own rules, restated for a reader that does not share
/// them: say what is exact, say what is a reading, and say when you do not
/// know. A model will not obey them the way the rest of this crate does — that
/// is why the interface labels its answers as readings — but stating them
/// plainly is what makes the difference between a useful paragraph and a
/// confident invention.
fn system(reply_in: &str) -> String {
    format!(
        "You are helping someone read a binary they are authorised to analyse. \
They are looking at Desdec, a static binary explorer: the file was disassembled, \
never executed, and nothing in your answer will be run either.\n\n\
Rules:\n\
- Work only from the facts given below. Do not invent addresses, symbol names, \
strings or library names that are not there.\n\
- Separate what is certain from what is a reading. Say \"this is\" for what the \
listing shows outright, and \"this looks like\" for anything a branch could \
invalidate. When you do not know, say so instead of guessing.\n\
- Be concrete: name the addresses, registers and symbols you are talking about.\n\
- Be brief. A few short paragraphs, or a short list. No preamble, no restating \
of the question, no summary of these rules.\n\
- Write the whole answer in {reply_in}. Keep code, symbol names and addresses \
as they are written."
    )
}

/// Where to start on a binary, from the loader-level facts.
fn binary(analysis: &Analysis) -> String {
    let summary = &analysis.summary;
    let details = &analysis.details;
    let mut text = String::from(
        "Here is what a static analysis found. Suggest where to start reading, \
what is worth a closer look, and what the evidence does not support. Say plainly \
if nothing here is unusual.\n\n",
    );

    let _ = writeln!(text, "Format: {}", summary.format.label());
    let _ = writeln!(text, "Architecture: {}", summary.architecture.label());
    let _ = writeln!(text, "Kind: {}", details.file_kind.label());
    let _ = writeln!(text, "Size: {} bytes", summary.size);
    if let Some(entry) = analysis.entry_point {
        let _ = writeln!(text, "Entry point: {entry:#018x}");
    }
    if let Some(subsystem) = details.subsystem {
        let _ = writeln!(text, "Subsystem: {subsystem}");
    }
    if let Some(interpreter) = &details.interpreter {
        let _ = writeln!(text, "Interpreter: {interpreter}");
    }
    if let Some(language) = analysis.languages.first() {
        let _ = write!(text, "Source language: {}", language.language.label());
        if let Some(toolchain) = &language.toolchain {
            let _ = write!(text, " ({toolchain})");
        }
        let _ = writeln!(text, " — evidence: {}", language.evidence);
    }
    let _ = writeln!(text, "Hardening: {}", hardening(analysis));
    let _ = writeln!(
        text,
        "Instructions decoded: {}{}",
        analysis.instructions.len(),
        if analysis.code_truncated {
            " (some executable bytes were not read)"
        } else {
            ""
        }
    );

    let _ = write!(text, "\nSections (name, size, entropy):\n");
    for section in analysis.sections.iter().take(MAXIMUM_FACTS) {
        let _ = write!(text, "- {} — {} bytes", section.name, section.file_size);
        let _ = write!(text, ", {}", section.permissions.label());
        if let Some(measured) = section.entropy {
            let _ = write!(text, ", entropy {measured:.2}");
            if entropy::suggests_packing(measured) {
                let _ = write!(text, " (dense)");
            }
        }
        text.push('\n');
    }

    if details.linked_libraries.is_empty() {
        text.push_str("\nLinked libraries: none recorded.\n");
    } else {
        text.push_str("\nLinked libraries:\n");
        for library in details.linked_libraries.iter().take(MAXIMUM_FACTS) {
            let _ = writeln!(text, "- {library}");
        }
    }

    let imports: Vec<&str> = analysis
        .symbols
        .iter()
        .filter(|symbol| symbol.imported)
        .map(|symbol| symbol.name.as_str())
        .take(MAXIMUM_FACTS)
        .collect();
    if !imports.is_empty() {
        let _ = writeln!(text, "\nImported symbols: {}", imports.join(", "));
    }

    let strings = notable_strings(analysis);
    if !strings.is_empty() {
        text.push_str("\nSome of the strings found:\n");
        for value in strings {
            let _ = writeln!(text, "- {value:?}");
        }
    }
    text
}

/// What a function does, from its own instructions.
fn function(analysis: &Analysis, address: u64) -> String {
    let named = analysis
        .symbols
        .iter()
        .find(|symbol| symbol.address == Some(address) && !symbol.imported);
    let name = named.map_or("(unnamed)", |symbol| symbol.name.as_str());

    let mut text = format!(
        "Read this function and say what it does: its purpose, its parameters as \
far as the code shows them, what it returns, and anything worth a second look. \
It is disassembled, not decompiled — the listing is exact, the reading is yours.\n\n\
Function: {name} at {address:#018x}\n\
Architecture: {}\n\n",
        analysis.summary.architecture.label()
    );

    let body = body_of(analysis, address, named.map_or(0, |symbol| symbol.size));
    if body.is_empty() {
        text.push_str("No instructions were decoded at this address.\n");
        return text;
    }
    for instruction in body {
        let _ = writeln!(text, "{:#018x}  {}", instruction.address, instruction.text);
    }
    let hidden = decoded_length(analysis, address, named.map_or(0, |symbol| symbol.size))
        .saturating_sub(MAXIMUM_BODY);
    if hidden > 0 {
        let _ = writeln!(text, "… {hidden} further instructions were left out.");
    }
    text
}

/// One instruction, with enough either side to be readable.
fn instruction(analysis: &Analysis, address: u64) -> String {
    let Some(index) = analysis.instruction_index(address) else {
        return format!("No instruction was decoded at {address:#018x}.");
    };
    let start = index.saturating_sub(CONTEXT_WINDOW);
    let end = (index + CONTEXT_WINDOW + 1).min(analysis.instructions.len());

    let mut text = format!(
        "Explain what the instruction marked with an arrow does, and what it is \
doing in the sequence around it — where its operands come from and what the \
result is used for.\n\n\
Architecture: {}\n\n",
        analysis.summary.architecture.label()
    );
    for (position, instruction) in analysis.instructions[start..end].iter().enumerate() {
        let marker = if start + position == index {
            "=>"
        } else {
            "  "
        };
        let _ = writeln!(
            text,
            "{marker} {:#018x}  {}",
            instruction.address, instruction.text
        );
    }
    text
}

/// The instructions belonging to a function, as far as they can be known.
///
/// A symbol's size is the honest answer where the file states one. Where it
/// does not, the listing is followed to the next named function — a guess
/// about the boundary, never about the instructions themselves.
fn body_of(analysis: &Analysis, address: u64, size: u64) -> &[crate::analysis::Instruction] {
    let range = analysis.instruction_span(address..end_of(analysis, address, size));
    let end = range.end.min(range.start + MAXIMUM_BODY);
    &analysis.instructions[range.start..end]
}

fn decoded_length(analysis: &Analysis, address: u64, size: u64) -> usize {
    analysis
        .instruction_span(address..end_of(analysis, address, size))
        .len()
}

fn end_of(analysis: &Analysis, address: u64, size: u64) -> u64 {
    if size > 0 {
        return address.saturating_add(size);
    }
    analysis
        .symbols
        .iter()
        .filter(|symbol| !symbol.imported)
        .filter_map(|symbol| symbol.address)
        .filter(|start| *start > address)
        .min()
        .unwrap_or(u64::MAX)
}

/// Strings worth showing: the longer ones, which are the ones that say
/// something. Short runs are mostly fragments of the file's own machinery.
fn notable_strings(analysis: &Analysis) -> Vec<String> {
    let mut chosen: Vec<&str> = analysis
        .strings
        .iter()
        .map(|string| string.value.as_str())
        .filter(|value| value.chars().count() >= 6)
        .collect();
    chosen.sort_unstable_by_key(|value| std::cmp::Reverse(value.len()));
    chosen.truncate(MAXIMUM_FACTS);
    chosen
        .into_iter()
        .map(|value| value.chars().take(MAXIMUM_STRING).collect())
        .collect()
}

/// The hardening flags as one line, saying nothing where the format says
/// nothing rather than reporting an absence it never established.
fn hardening(analysis: &Analysis) -> String {
    let flags = &analysis.details.hardening;
    let mut stated = Vec::new();
    let mut say = |label: &str, value: Option<bool>| {
        if let Some(value) = value {
            stated.push(format!("{label}={value}"));
        }
    };
    say("PIE", flags.position_independent);
    say("NX", flags.non_executable_stack);
    say("canary", flags.stack_canary);
    say("ASLR", flags.address_space_randomisation);
    say("DEP", flags.data_execution_prevention);
    say("CFG", flags.control_flow_guard);
    say("signed", flags.signed);
    if let Some(relro) = flags.relro {
        stated.push(format!("RELRO={}", relro.label()));
    }
    if stated.is_empty() {
        "the format states none".to_owned()
    } else {
        stated.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analysis() -> Analysis {
        let fixture = crate::fixtures::elf_x86_64();
        crate::analyse_bytes(
            std::path::Path::new("fixture.bin"),
            fixture.bytes.len() as u64,
            &fixture.bytes,
        )
    }

    /// The answer is written for the reader, so the request has to say in
    /// which language — and it must never suggest running anything.
    #[test]
    fn the_rules_name_the_language_and_forbid_execution() {
        let prompt = build(&analysis(), Question::Binary, "français");
        assert!(prompt.system.contains("français"));
        assert!(prompt.system.contains("never executed"));
    }

    /// What is sent must be the facts the analysis found, and nothing about
    /// the file's own bytes.
    #[test]
    fn the_binary_question_carries_the_facts_it_found() {
        let analysis = analysis();
        let prompt = build(&analysis, Question::Binary, "English");
        assert!(prompt.user.contains(analysis.summary.format.label()));
        assert!(prompt.user.contains("Sections"));
        for library in &analysis.details.linked_libraries {
            assert!(prompt.user.contains(library), "{library} was left out");
        }
    }

    #[test]
    fn a_function_question_carries_its_own_instructions() {
        let analysis = analysis();
        let symbol = analysis
            .symbols
            .iter()
            .find(|symbol| !symbol.imported && symbol.address.is_some())
            .expect("the fixture names a function");
        let address = symbol.address.expect("filtered above");

        let prompt = build(&analysis, Question::Function { address }, "English");
        assert!(prompt.user.contains(&symbol.name));
        assert!(prompt.user.contains(&format!("{address:#018x}")));
    }

    /// A function whose address decodes to nothing must say so rather than
    /// sending an empty listing for the model to fill in.
    #[test]
    fn a_function_with_no_instructions_says_so() {
        let prompt = build(&analysis(), Question::Function { address: 1 }, "English");
        assert!(prompt.user.contains("No instructions"));
    }

    #[test]
    fn an_instruction_question_marks_the_one_that_was_asked_about() {
        let analysis = analysis();
        let instruction = analysis
            .instructions
            .get(analysis.instructions.len() / 2)
            .expect("the fixture decodes instructions");

        let prompt = build(
            &analysis,
            Question::Instruction {
                address: instruction.address,
            },
            "English",
        );
        assert!(prompt.user.contains(&format!(
            "=> {:#018x}  {}",
            instruction.address, instruction.text
        )));
    }

    #[test]
    fn an_address_that_decodes_to_nothing_says_so() {
        let prompt = build(&analysis(), Question::Instruction { address: 7 }, "English");
        assert!(prompt.user.contains("No instruction"));
    }

    /// A function of a hundred thousand instructions must not become a
    /// hundred-thousand-instruction request.
    #[test]
    fn a_long_function_is_cut_and_says_it_was() {
        let mut analysis = analysis();
        let base = 0x40_0000;
        let sample = analysis
            .instructions
            .first()
            .cloned()
            .expect("the fixture decodes instructions");
        analysis.instructions = (0..MAXIMUM_BODY as u64 + 50)
            .map(|index| crate::analysis::Instruction {
                address: base + index * 4,
                ..sample.clone()
            })
            .collect();
        analysis.symbols.clear();

        let prompt = build(&analysis, Question::Function { address: base }, "English");
        let listed = prompt.user.matches("0x0000000000").count();
        assert!(
            listed <= MAXIMUM_BODY + 1,
            "{listed} instructions were sent"
        );
        assert!(prompt.user.contains("further instructions were left out"));
    }
}
