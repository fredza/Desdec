//! What the reader has worked out about an address, and wants back tomorrow.
//!
//! An analysis tool tells you what a file contains; it cannot tell you what
//! any of it is *for*. That is what the reader works out, one address at a
//! time, and it is the only thing here the file itself does not hold: a name
//! for a function the symbol table never had, a sentence about why a branch
//! matters, a mark on the row to come back to.
//!
//! Kept beside the binary rather than in it — the analysed file is never
//! modified — and keyed by the file's digest rather than by its path, so notes
//! follow the bytes they were written about: a binary renamed, moved or copied
//! keeps its notes, and a *different* binary with the same name never inherits
//! them.

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// What the reader has said about one address.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Annotation {
    /// A name for this address, standing where a symbol would.
    pub label: String,
    /// A sentence about it, drawn where an assembler puts a comment.
    pub comment: String,
    /// A mark on the row, for the addresses being come back to.
    pub bookmarked: bool,
}

impl Annotation {
    /// Whether there is nothing left in it, and it should not be kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.label.trim().is_empty() && self.comment.trim().is_empty() && !self.bookmarked
    }
}

/// A type the reader has said one register holds inside one function.
///
/// It is what turns `0x18(%rbx)` in the listing into `header.count`: the file
/// never said what `rbx` points at there, and this is the reader saying it.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InCode {
    /// Where the function starts, which is how the listing finds it again.
    pub function: u64,
    /// The register, as the listing writes it and without a `%`.
    pub register: String,
    /// The type it holds, by the name it was defined under.
    pub kind: String,
}

/// A name the reader gave a variable of one decompiled function.
///
/// Named by *where the variable is* rather than by what the decompiler called
/// it. `local_18` and `argument_1` are made up afresh on every pass — the
/// numbering follows the order slots are met, so inserting one instruction can
/// renumber the lot — and a name pinned to them would move to another variable
/// the next time the function was decompiled.
///
/// What does not move is the machine: a local is a fixed offset from a frame
/// register, and a parameter is the register the convention passes it in. That
/// is what `slot` holds — `rbp-0x18` for a local, `rdi` for a parameter — and
/// it is the same string the listing writes, so a reader can see in the
/// disassembly what they named in the pseudo-code.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Variable {
    /// Where the function starts, which is what makes `rbp-0x18` mean one
    /// place rather than one in every function of the program.
    pub function: u64,
    /// Where the variable lives: `rbp-0x18`, `rsp+0x8`, or a register root.
    pub slot: String,
    /// What the reader calls it.
    pub name: String,
}

/// Everything the reader has written about one binary.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Annotations {
    /// By address, so the listing and the annotation list read in the same
    /// order the file does.
    entries: BTreeMap<u64, Annotation>,
    /// The type definitions written about this binary's data, as the C they
    /// were typed in; see [`crate::ui::types`].
    ///
    /// Kept here rather than beside the preferences because it is about this
    /// file and no other: the structures of one program describe nothing in
    /// the next one opened, and a reader coming back to a binary wants what
    /// they worked out about it, not what they worked out about another.
    types: String,
    /// Which type each register holds, function by function.
    in_code: Vec<InCode>,
    /// What the reader calls the locals and parameters of each function.
    variables: Vec<Variable>,
}

impl Annotations {
    #[must_use]
    pub fn at(&self, address: u64) -> Option<&Annotation> {
        self.entries.get(&address)
    }

    #[must_use]
    pub fn label(&self, address: u64) -> Option<&str> {
        self.at(address)
            .map(|annotation| annotation.label.trim())
            .filter(|label| !label.is_empty())
    }

    #[must_use]
    pub fn comment(&self, address: u64) -> Option<&str> {
        self.at(address)
            .map(|annotation| annotation.comment.trim())
            .filter(|comment| !comment.is_empty())
    }

    /// Whether the reader has written anything about this address — a name
    /// or a sentence, as opposed to merely marking the row.
    ///
    /// Asked once per drawn row so the listing can put a mark in the margin
    /// where a note sits: the note itself rides at the end of the line, which
    /// is off the right edge of a listing scrolled to look at the bytes.
    #[must_use]
    pub fn has_note(&self, address: u64) -> bool {
        self.label(address).is_some() || self.comment(address).is_some()
    }

    #[must_use]
    pub fn is_bookmarked(&self, address: u64) -> bool {
        self.at(address)
            .is_some_and(|annotation| annotation.bookmarked)
    }

    /// Replaces what is said about an address, forgetting it when nothing is
    /// left: an empty note is not a note, and keeping it would fill the list
    /// with addresses the reader cleared on purpose.
    pub fn set(&mut self, address: u64, annotation: Annotation) {
        if annotation.is_empty() {
            self.entries.remove(&address);
        } else {
            self.entries.insert(address, annotation);
        }
    }

    pub fn toggle_bookmark(&mut self, address: u64) {
        let mut annotation = self.at(address).cloned().unwrap_or_default();
        annotation.bookmarked = !annotation.bookmarked;
        self.set(address, annotation);
    }

    /// Every annotated address, in file order.
    pub fn iter(&self) -> impl Iterator<Item = (u64, &Annotation)> {
        self.entries
            .iter()
            .map(|(address, annotation)| (*address, annotation))
    }

    /// The C the reader wrote about this binary's data.
    #[must_use]
    pub fn types(&self) -> &str {
        &self.types
    }

    /// Keeps what the reader wrote about this binary's data.
    pub fn set_types(&mut self, source: String) {
        self.types = source;
    }

    /// Which type each register holds, function by function.
    #[must_use]
    pub fn in_code(&self) -> &[InCode] {
        &self.in_code
    }

    /// Says that a register holds a type inside a function, replacing whatever
    /// was said about that same register in that same function.
    pub fn say_in_code(&mut self, saying: InCode) {
        self.in_code
            .retain(|had| had.function != saying.function || had.register != saying.register);
        self.in_code.push(saying);
    }

    /// Takes one of those sayings back.
    pub fn forget_in_code(&mut self, function: u64, register: &str) {
        self.in_code
            .retain(|had| had.function != function || had.register != register);
    }

    /// What the reader calls a variable of a function, if they named it.
    ///
    /// `slot` is where it lives — `rbp-0x18` for a local, `rdi` for a
    /// parameter — which is what survives a second decompilation.
    #[must_use]
    pub fn variable(&self, function: u64, slot: &str) -> Option<&str> {
        self.variables
            .iter()
            .find(|named| named.function == function && named.slot == slot)
            .map(|named| named.name.as_str())
    }

    /// Names one, replacing whatever was called that place before. An empty
    /// name takes the saying back rather than storing a blank.
    pub fn name_variable(&mut self, function: u64, slot: &str, name: &str) {
        self.variables
            .retain(|named| named.function != function || named.slot != slot);
        let name = name.trim();
        if !name.is_empty() {
            self.variables.push(Variable {
                function,
                slot: slot.to_owned(),
                name: name.to_owned(),
            });
        }
    }

    /// Every variable the reader named, for the code that carries the names
    /// into a decompilation.
    #[must_use]
    pub fn variables(&self) -> &[Variable] {
        &self.variables
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
            && self.types.trim().is_empty()
            && self.in_code.is_empty()
            && self.variables.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.types.clear();
        self.in_code.clear();
        self.variables.clear();
    }
}

/// Where notes are kept: the user's data, not a cache.
///
/// A cache directory is a place a system is entitled to empty; what the reader
/// wrote about a binary is not something they should lose to a disk cleanup.
#[must_use]
pub fn directory() -> Option<PathBuf> {
    Some(crate::storage::data_directory()?.join("notes"))
}

/// The file one binary's notes live in, named after its digest.
#[must_use]
pub fn file_for(directory: &Path, digest: &[u8; 32]) -> PathBuf {
    use std::fmt::Write as _;

    let mut name = String::with_capacity(digest.len() * 2 + 4);
    for byte in digest {
        // Writing into a string cannot fail; the result is only checked so the
        // name is never silently half-written.
        let _ = write!(name, "{byte:02x}");
    }
    name.push_str(".ron");
    directory.join(name)
}

/// Reads back what was written about this binary, or nothing at all.
///
/// A file that cannot be read or parsed is treated as no notes rather than as
/// an error to put on screen: the reader is opening a binary, and a complaint
/// about a notes file from an older build helps nobody.
#[must_use]
pub fn read(directory: &Path, digest: &[u8; 32]) -> Option<Annotations> {
    let text = std::fs::read_to_string(file_for(directory, digest)).ok()?;
    ron::from_str(&text).ok()
}

/// Writes them, creating the directory if it is not there yet.
///
/// Notes emptied to nothing take their file with them: an empty file left
/// behind for every binary ever opened is a list of what someone looked at.
pub fn write(directory: &Path, digest: &[u8; 32], annotations: &Annotations) -> io::Result<()> {
    let path = file_for(directory, digest);
    if annotations.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            outcome => outcome,
        };
    }
    std::fs::create_dir_all(directory)?;
    let text = ron::to_string(annotations)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    /// A name for a variable is kept against *where it lives*, so it survives
    /// a second decompilation: `local_18` and `argument_1` are made up afresh
    /// on every pass, `rbp-0x18` and `rdi` are facts about the machine.
    #[test]
    fn a_variable_is_named_by_where_it_lives() {
        let mut annotations = Annotations::default();
        annotations.name_variable(0x0040_1000, "rbp-0x18", "header");
        annotations.name_variable(0x0040_1000, "rdi", "count");

        assert_eq!(
            annotations.variable(0x0040_1000, "rbp-0x18"),
            Some("header")
        );
        assert_eq!(annotations.variable(0x0040_1000, "rdi"), Some("count"));
        // The same place in another function is another variable: a frame
        // offset means nothing without the function it belongs to.
        assert_eq!(annotations.variable(0x0040_2000, "rbp-0x18"), None);
    }

    /// Naming the same place twice replaces rather than accumulates, and
    /// emptying the name takes the saying back rather than storing a blank.
    #[test]
    fn naming_a_variable_again_replaces_what_was_said() {
        let mut annotations = Annotations::default();
        annotations.name_variable(0x0040_1000, "rdi", "count");
        annotations.name_variable(0x0040_1000, "rdi", "length");
        assert_eq!(annotations.variable(0x0040_1000, "rdi"), Some("length"));
        assert_eq!(annotations.variables().len(), 1);

        annotations.name_variable(0x0040_1000, "rdi", "   ");
        assert_eq!(annotations.variable(0x0040_1000, "rdi"), None);
        assert!(annotations.variables().is_empty());
    }

    /// And a file holding nothing but a named variable is still a file worth
    /// writing: notes emptied to nothing take their file with them, and a
    /// reader who has only renamed things has not emptied theirs.
    #[test]
    fn a_named_variable_alone_is_worth_keeping() {
        let mut annotations = Annotations::default();
        assert!(annotations.is_empty());
        annotations.name_variable(0x0040_1000, "rdi", "count");
        assert!(!annotations.is_empty());
    }

    #[test]
    fn an_emptied_note_is_forgotten_rather_than_kept_blank() {
        let mut annotations = Annotations::default();
        annotations.set(
            0x0040_1000,
            Annotation {
                label: "parse_header".to_owned(),
                ..Annotation::default()
            },
        );
        assert_eq!(annotations.label(0x0040_1000), Some("parse_header"));

        annotations.set(0x0040_1000, Annotation::default());

        assert!(annotations.is_empty(), "nothing was left to keep");
        assert_eq!(annotations.label(0x0040_1000), None);
    }

    /// Whitespace is not a note: a label of three spaces would draw an empty
    /// name over the address and read as a bug.
    #[test]
    fn blank_text_counts_as_nothing() {
        let mut annotations = Annotations::default();
        annotations.set(
            0x0040_1000,
            Annotation {
                label: "   ".to_owned(),
                comment: "\t".to_owned(),
                bookmarked: false,
            },
        );
        assert!(annotations.is_empty());
    }

    #[test]
    fn a_bookmark_is_its_own_reason_to_keep_an_address() {
        let mut annotations = Annotations::default();
        annotations.toggle_bookmark(0x0040_1000);
        assert!(annotations.is_bookmarked(0x0040_1000));
        assert_eq!(annotations.iter().count(), 1);

        annotations.toggle_bookmark(0x0040_1000);
        assert!(!annotations.is_bookmarked(0x0040_1000));
        assert!(annotations.is_empty(), "an unmarked address is not a note");
    }

    /// The margin mark stands for something written, not for a bookmark: a
    /// row marked to come back to already has its own star, and a dot beside
    /// it would promise a note that is not there.
    #[test]
    fn a_bookmark_alone_is_not_a_note() {
        let mut annotations = Annotations::default();
        annotations.toggle_bookmark(0x0040_1000);
        assert!(!annotations.has_note(0x0040_1000));

        annotations.set(
            0x0040_1000,
            Annotation {
                comment: "reads the magic".to_owned(),
                bookmarked: true,
                ..Annotation::default()
            },
        );
        assert!(annotations.has_note(0x0040_1000));
        assert!(!annotations.has_note(0x0040_2000), "an untouched address");
    }

    /// Notes are keyed by the bytes they were written about, so a renamed or
    /// copied binary keeps them and a different one never inherits them.
    #[test]
    fn notes_are_written_and_read_back_by_digest() {
        let directory = std::env::temp_dir().join(format!("desdec-notes-{}", std::process::id()));
        let mut annotations = Annotations::default();
        annotations.set(
            0x0040_1230,
            Annotation {
                label: "parse_header".to_owned(),
                comment: "reads the magic".to_owned(),
                bookmarked: true,
            },
        );

        write(&directory, &digest(1), &annotations).expect("the notes are writable");

        assert_eq!(read(&directory, &digest(1)), Some(annotations.clone()));
        assert_eq!(read(&directory, &digest(2)), None, "another binary's notes");

        // Emptied notes take their file with them.
        write(&directory, &digest(1), &Annotations::default()).expect("removable");
        assert_eq!(read(&directory, &digest(1)), None);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
