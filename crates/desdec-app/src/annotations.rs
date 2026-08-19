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

/// Everything the reader has written about one binary.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Annotations {
    /// By address, so the listing and the annotation list read in the same
    /// order the file does.
    entries: BTreeMap<u64, Annotation>,
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

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Where notes are kept: the user's data, not a cache.
///
/// A cache directory is a place a system is entitled to empty; what the reader
/// wrote about a binary is not something they should lose to a disk cleanup.
#[must_use]
pub fn directory() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
    }?;
    Some(base.join("desdec").join("notes"))
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
