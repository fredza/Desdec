//! Pending patches, and the editing state behind the Patches view.
//!
//! The list is the single source of truth: the disassembly reads it to show
//! which instructions were edited, and the export writes exactly what it holds.
//! Nothing here touches the disk — [`desdec_core::patch`] does that, to a copy.

use desdec_core::{Analysis, Architecture, Instruction, Patch, PatchError, assemble, decode_one};

use crate::i18n::{Language, Text, text};

/// An instruction being edited, with what its bytes currently decode to.
pub struct Editor {
    /// Instruction the editor was opened on.
    pub address: u64,
    pub file_offset: u64,
    pub original: Vec<u8>,
    /// Hex the user is typing. Kept as text so a half-typed byte is not lost.
    pub input: String,
    /// The instruction the reader is typing, when they would rather write one
    /// than count bytes.
    pub assembly: String,
    /// The assembly last encoded into `input`, so typing bytes by hand is not
    /// undone on the next frame by an assembly line that has not changed.
    assembled_from: String,
}

/// What the typed assembly became, or why it cannot be used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Assembled {
    /// Bytes that fit where they are going, and how many `nop`s were added to
    /// fill the room left over.
    Fits { bytes: Vec<u8>, padding: usize },
    /// It encodes longer than the instruction it would replace. Writing it
    /// would move every byte after it, which a patch must never do.
    TooLong { encoded: usize, room: usize },
    /// The assembler could not read the line.
    Refused(assemble::Error),
    /// Nothing has been typed.
    Empty,
}

/// What the bytes currently in the editor mean.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Preview {
    /// They decode to one whole instruction.
    Decoded(Instruction),
    /// They are valid bytes, but not one whole instruction for this
    /// architecture. Still patchable — data can be patched too — but the user
    /// should know it is not an instruction.
    NotAnInstruction,
    /// The text is not readable as hex bytes yet.
    Invalid(String),
    /// The length no longer matches, which would move every following byte.
    LengthChanged { expected: usize, found: usize },
}

impl Editor {
    #[must_use]
    pub fn new(instruction: &Instruction, file_offset: u64) -> Self {
        Self {
            address: instruction.address,
            file_offset,
            original: instruction.bytes.to_vec(),
            input: to_hex(&instruction.bytes),
            assembly: String::new(),
            assembled_from: String::new(),
        }
    }

    /// Restores the bytes the file actually contains.
    pub fn reset(&mut self) {
        self.input = to_hex(&self.original);
        self.assembly.clear();
        self.assembled_from.clear();
    }

    /// What the typed assembly would write here.
    ///
    /// A patch keeps the length of what it replaces — anything else moves
    /// every byte after it — so a shorter encoding is filled out with `nop`s,
    /// exactly as a reader would do by hand, and a longer one is refused with
    /// the room it would have needed.
    #[must_use]
    pub fn assembled(&self, architecture: Architecture) -> Assembled {
        if self.assembly.trim().is_empty() {
            return Assembled::Empty;
        }
        let mut bytes = match assemble::assemble(&self.assembly, architecture, self.address) {
            Ok(bytes) => bytes,
            Err(error) => return Assembled::Refused(error),
        };
        let room = self.original.len();
        if bytes.len() > room {
            return Assembled::TooLong {
                encoded: bytes.len(),
                room,
            };
        }
        let padding = room - bytes.len();
        bytes.resize(room, assemble::PADDING);
        Assembled::Fits { bytes, padding }
    }

    /// Puts the assembled bytes in the byte field, once, when the line they
    /// came from has changed.
    ///
    /// Once and not every frame: the byte field stays the thing that gets
    /// written, and a reader who edits it by hand afterwards must not have
    /// their edit overwritten by an assembly line they have stopped touching.
    pub fn take_assembled(&mut self, architecture: Architecture) -> Assembled {
        let outcome = self.assembled(architecture);
        if self.assembled_from == self.assembly {
            return outcome;
        }
        self.assembled_from.clone_from(&self.assembly);
        if let Assembled::Fits { bytes, .. } = &outcome {
            self.input = to_hex(bytes);
        }
        outcome
    }

    /// Reads the typed hex, refusing anything that changes the length.
    ///
    /// # Errors
    ///
    /// Returns the reason the text cannot become a patch.
    pub fn bytes(&self) -> Result<Vec<u8>, Preview> {
        let bytes = parse_hex(&self.input).map_err(Preview::Invalid)?;
        if bytes.len() != self.original.len() {
            return Err(Preview::LengthChanged {
                expected: self.original.len(),
                found: bytes.len(),
            });
        }
        Ok(bytes)
    }

    /// What the typed bytes decode to right now.
    #[must_use]
    pub fn preview(&self, architecture: Architecture) -> Preview {
        match self.bytes() {
            Ok(bytes) => decode_one(&bytes, architecture, self.address)
                .map_or(Preview::NotAnInstruction, Preview::Decoded),
            Err(reason) => reason,
        }
    }

    /// Builds the patch, or explains why the bytes cannot become one.
    ///
    /// # Errors
    ///
    /// Returns the reason the current text is not a usable patch.
    pub fn to_patch(&self) -> Result<Patch, Preview> {
        let replacement = self.bytes()?;
        Patch::new(
            self.file_offset,
            self.address,
            self.original.clone(),
            replacement,
        )
        .map_err(|error| match error {
            PatchError::LengthChanged { expected, found } => {
                Preview::LengthChanged { expected, found }
            }
            other => Preview::Invalid(other.to_string()),
        })
    }
}

/// Why the assembler could not read a line, in its own words.
///
/// Here rather than in the view that shows it: a script is refused by the same
/// assembler, and a reader who typed the same line in the patch editor and in
/// a script must not be told two different things about it.
#[must_use]
pub fn refusal(error: &assemble::Error, language: Language) -> String {
    match error {
        assemble::Error::UnsupportedArchitecture => {
            text(language, Text::AssemblerUnsupported).to_owned()
        }
        assemble::Error::Empty => String::new(),
        assemble::Error::UnknownMnemonic(mnemonic) => {
            format!(
                "{} {mnemonic}",
                text(language, Text::AssemblerUnknownMnemonic)
            )
        }
        assemble::Error::UnknownOperand(operand) => {
            format!(
                "{} {operand}",
                text(language, Text::AssemblerUnknownOperand)
            )
        }
        assemble::Error::WrongOperands(mnemonic) => {
            format!(
                "{} {mnemonic}",
                text(language, Text::AssemblerWrongOperands)
            )
        }
        assemble::Error::Refused(reason) => {
            format!("{} {reason}", text(language, Text::AssemblerRefused))
        }
    }
}

/// Every pending patch, newest last.
#[derive(Default)]
pub struct Patches {
    entries: Vec<Patch>,
}

impl Patches {
    #[must_use]
    pub fn entries(&self) -> &[Patch] {
        &self.entries
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The byte a pending patch would write at a file offset, if one would.
    ///
    /// By offset rather than by address, because that is what a dump of the
    /// file is laid out in — and what the export will actually write to.
    #[must_use]
    pub fn byte_at_offset(&self, offset: u64) -> Option<u8> {
        self.entries.iter().find_map(|patch| {
            let within = offset.checked_sub(patch.file_offset)?;
            patch
                .replacement
                .get(usize::try_from(within).ok()?)
                .copied()
        })
    }

    /// Whether an instruction already carries a patch.
    #[must_use]
    pub fn patch_at(&self, address: u64) -> Option<&Patch> {
        self.entries.iter().find(|patch| patch.address == address)
    }

    /// Records a patch, replacing any earlier one on the same instruction.
    ///
    /// A patch that rewrites the same bytes is dropped instead of stored: it
    /// would show in the list as a change nobody made.
    pub fn set(&mut self, patch: Patch) {
        self.remove(patch.address);
        if patch.changes_anything() {
            self.entries.push(patch);
        }
    }

    pub fn remove(&mut self, address: u64) {
        self.entries.retain(|patch| patch.address != address);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Locates an instruction's bytes in the file.
///
/// A patch is written at a file offset, while the disassembly works in virtual
/// addresses; an instruction in a section that occupies no file space — `.bss`
/// and friends — has no offset to write to, and cannot be patched.
#[must_use]
pub fn file_offset_of(analysis: &Analysis, address: u64) -> Option<u64> {
    analysis.file_offset_of(address)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Reads hex bytes, accepting the spacing the disassembly itself prints.
fn parse_hex(input: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = input
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    if cleaned.is_empty() {
        return Err("no bytes".to_owned());
    }
    if let Some(character) = cleaned.chars().find(|c| !c.is_ascii_hexdigit()) {
        return Err(format!("'{character}' is not a hex digit"));
    }
    if cleaned.len() % 2 != 0 {
        return Err("an odd number of hex digits".to_owned());
    }
    cleaned
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|_| "unreadable".to_owned())?;
            u8::from_str_radix(text, 16).map_err(|error| error.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instruction(address: u64, bytes: &[u8]) -> Instruction {
        Instruction {
            address,
            bytes: desdec_core::InstructionBytes::new(bytes).expect("test instructions are short"),
            text: "push %rbp".to_owned(),
            section: std::sync::Arc::from(".text"),
        }
    }

    #[test]
    fn an_editor_opens_on_the_bytes_the_file_holds() {
        let editor = Editor::new(&instruction(0x40_1136, &[0x55]), 0x1136);
        assert_eq!(editor.input, "55");
        assert_eq!(editor.bytes(), Ok(vec![0x55]));
    }

    #[test]
    fn hex_is_read_however_it_is_spaced() {
        assert_eq!(parse_hex("48 89 e5"), Ok(vec![0x48, 0x89, 0xe5]));
        assert_eq!(parse_hex("4889e5"), Ok(vec![0x48, 0x89, 0xe5]));
        assert_eq!(parse_hex(" 48\t89 E5 "), Ok(vec![0x48, 0x89, 0xe5]));
    }

    #[test]
    fn text_that_is_not_hex_is_refused_with_a_reason() {
        assert!(parse_hex("zz").is_err());
        assert!(parse_hex("4").is_err(), "an odd digit count is incomplete");
        assert!(parse_hex("").is_err());
    }

    /// The whole point of the length rule: everything after the patch keeps
    /// its offset.
    #[test]
    fn a_replacement_of_a_different_length_never_becomes_a_patch() {
        let mut editor = Editor::new(&instruction(0x40_1136, &[0x55]), 0x1136);
        editor.input = "48 89".to_owned();

        assert!(matches!(
            editor.to_patch(),
            Err(Preview::LengthChanged {
                expected: 1,
                found: 2
            })
        ));
    }

    #[test]
    fn the_preview_shows_what_the_edited_bytes_decode_to() {
        let mut editor = Editor::new(&instruction(0x40_1136, &[0x55]), 0x1136);
        editor.input = "90".to_owned();

        match editor.preview(Architecture::X86_64) {
            Preview::Decoded(instruction) => assert_eq!(instruction.text, "nop"),
            _ => panic!("0x90 should decode to a nop"),
        }
    }

    #[test]
    fn resetting_restores_the_original_bytes() {
        let mut editor = Editor::new(&instruction(0x40_1136, &[0x55]), 0x1136);
        editor.input = "90".to_owned();
        editor.reset();
        assert_eq!(editor.bytes(), Ok(vec![0x55]));
    }

    #[test]
    fn a_patch_replaces_an_earlier_one_on_the_same_instruction() {
        let mut patches = Patches::default();
        patches.set(Patch::new(0x1136, 0x40_1136, vec![0x55], vec![0x90]).expect("same length"));
        patches.set(Patch::new(0x1136, 0x40_1136, vec![0x55], vec![0xcc]).expect("same length"));

        assert_eq!(patches.len(), 1);
        assert_eq!(patches.entries()[0].replacement, [0xcc]);
    }

    /// Re-typing the original bytes is not a change, and must not show as one.
    #[test]
    fn a_patch_that_changes_nothing_is_not_recorded() {
        let mut patches = Patches::default();
        patches.set(Patch::new(0x1136, 0x40_1136, vec![0x55], vec![0x90]).expect("same length"));
        patches.set(Patch::new(0x1136, 0x40_1136, vec![0x55], vec![0x55]).expect("same length"));

        assert!(patches.is_empty());
    }

    /// The point of the assembler: a reader types an instruction, and the
    /// bytes that will be written appear underneath it.
    #[test]
    fn typing_an_instruction_fills_the_byte_field() {
        let mut editor = Editor::new(&instruction(0x40_1000, &[0x90; 3]), 0x1000);
        editor.assembly = "mov rbp, rsp".to_owned();

        let outcome = editor.take_assembled(Architecture::X86_64);

        assert_eq!(
            outcome,
            Assembled::Fits {
                bytes: vec![0x48, 0x89, 0xe5],
                padding: 0
            }
        );
        assert_eq!(editor.input, "48 89 e5");
    }

    /// A patch keeps the length of what it replaces, so the room left over is
    /// filled with `nop`s rather than left to move everything after it.
    #[test]
    fn a_short_encoding_is_filled_out_with_nops() {
        let mut editor = Editor::new(&instruction(0x40_1000, &[0x90; 7]), 0x1000);
        editor.assembly = "ret".to_owned();

        let outcome = editor.take_assembled(Architecture::X86_64);

        assert_eq!(
            outcome,
            Assembled::Fits {
                bytes: vec![0xc3, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90],
                padding: 6
            }
        );
        assert_eq!(editor.input, "c3 90 90 90 90 90 90");
    }

    /// Longer than the room it has is refused: writing it would move every
    /// byte after it.
    #[test]
    fn an_encoding_that_does_not_fit_is_refused_with_its_measurements() {
        let mut editor = Editor::new(&instruction(0x40_1000, &[0x90; 2]), 0x1000);
        editor.assembly = "mov rax, 0x1122334455667788".to_owned();

        let outcome = editor.take_assembled(Architecture::X86_64);

        assert_eq!(
            outcome,
            Assembled::TooLong {
                encoded: 10,
                room: 2
            }
        );
        assert_eq!(editor.input, "90 90", "the bytes are left as they were");
    }

    /// The byte field is what gets written, so an edit made in it by hand must
    /// survive the frames that follow.
    #[test]
    fn bytes_edited_by_hand_are_not_overwritten_by_a_settled_assembly_line() {
        let mut editor = Editor::new(&instruction(0x40_1000, &[0x90]), 0x1000);
        editor.assembly = "ret".to_owned();
        let _ = editor.take_assembled(Architecture::X86_64);
        assert_eq!(editor.input, "c3");

        editor.input = "cc".to_owned();
        let _ = editor.take_assembled(Architecture::X86_64);

        assert_eq!(editor.input, "cc", "the hand edit stands");
    }

    #[test]
    fn a_patched_instruction_is_findable_by_address() {
        let mut patches = Patches::default();
        patches.set(Patch::new(0x1136, 0x40_1136, vec![0x55], vec![0x90]).expect("same length"));

        assert!(patches.patch_at(0x40_1136).is_some());
        assert!(patches.patch_at(0x40_1137).is_none());
    }
}
