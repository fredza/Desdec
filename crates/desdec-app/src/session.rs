//! The reader's work on one binary, in a file of their own beside it.
//!
//! Desdec already keeps notes on its own: under the platform's data directory,
//! named by the binary's digest, written as the reader types. That is the
//! right default — it needs no decision and survives a binary being renamed or
//! moved — but it is invisible, it lives on one machine, and it cannot be
//! handed to anybody.
//!
//! This is the other half: `program` and `program.dcl` beside each other, one
//! file a reader can copy onto a USB stick, commit next to the binary, or send
//! to whoever asked them what the thing does. Saving is deliberate, and so is
//! opening — nothing here happens on its own.
//!
//! # Which binary it belongs to
//!
//! The digest of the binary is written in the file, and checked when it is
//! read back. A `.dcl` describes addresses, and an address means nothing
//! without the bytes it points into: the same file opened beside a *different*
//! program would name its functions with somebody else's names, confidently
//! and wrongly. So a mismatch is reported, and what happens next is the
//! reader's decision rather than this module's.
//!
//! The name is not the check. `program.dcl` beside a rebuilt `program` is the
//! ordinary case — a reader recompiles what they are studying, and their notes
//! about `parse_header` are still about `parse_header`. It is also exactly the
//! case where the addresses may all have moved. Only the reader can say which
//! it is, so they are told, in those terms.

use std::{
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::annotations::Annotations;

/// The extension a saved session carries.
pub const EXTENSION: &str = "dcl";

/// What a `.dcl` holds.
///
/// The work, and enough about the binary to say whether it is the one this
/// work was done on. Nothing else: this is not a copy of the analysis, which
/// Desdec can redo in seconds, and a file that carried one would go stale
/// against the very binary it sits beside.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    /// The format's own version, so a file written by a later Desdec is
    /// refused with a sentence rather than read as something it is not.
    pub version: u32,
    /// The binary this was written about, as hexadecimal SHA-256.
    ///
    /// A string rather than the thirty-two bytes: a `.dcl` is a text file a
    /// reader may open, and a digest they can compare by eye against
    /// `sha256sum` is worth more than four lines of numbers.
    pub digest: String,
    /// What the binary was called when this was saved. Shown when the digest
    /// does not match, because "this was written about `crackme-v1`" is what
    /// tells a reader which file they are actually holding.
    pub binary: String,
    /// Everything the reader wrote: the names, the comments, the bookmarks,
    /// the types, and which register holds what.
    pub notes: Annotations,
}

/// The version this Desdec writes, and the only one it reads.
pub const VERSION: u32 = 1;

/// What reading a `.dcl` could not do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    /// The file could not be read or written.
    Io(String),
    /// It is not a session file, or not one this build understands.
    Unreadable(String),
    /// It was written by a later Desdec, whose format this one does not know.
    FromTheFuture { version: u32 },
}

impl std::fmt::Display for Error {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(why) | Self::Unreadable(why) => write!(out, "{why}"),
            Self::FromTheFuture { version } => write!(out, "v{version}"),
        }
    }
}

/// Whether the work in a file was done on the binary it was opened beside.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Belongs {
    /// The digests agree: this is the binary these notes were written about.
    ToThisBinary,
    /// They do not. The notes are still readable, and the addresses in them
    /// may point anywhere.
    ToAnother,
    /// One side has no digest to compare — a binary read only in part has
    /// none. Nothing can be said either way, and saying nothing is the honest
    /// answer.
    Unknown,
}

/// The `.dcl` that belongs beside a binary: its path with the extension
/// added, not replaced.
///
/// `program.exe` gets `program.exe.dcl` rather than `program.dcl`, so a
/// directory holding `tool.exe` and `tool.dll` does not end up with one file
/// for the two of them. A binary with no extension gets `program.dcl`, which
/// is the same rule.
#[must_use]
pub fn beside(binary: &Path) -> PathBuf {
    let mut name = binary.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(EXTENSION);
    binary.with_file_name(name)
}

impl Session {
    /// The work on a binary, ready to be written.
    #[must_use]
    pub fn of(binary: &Path, digest: Option<[u8; 32]>, notes: &Annotations) -> Self {
        Self {
            version: VERSION,
            digest: digest
                .map(|digest| desdec_core::hash::to_hex(&digest))
                .unwrap_or_default(),
            binary: binary
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            notes: notes.clone(),
        }
    }

    /// Whether this was written about the binary now open.
    #[must_use]
    pub fn belongs_to(&self, digest: Option<[u8; 32]>) -> Belongs {
        let Some(digest) = digest else {
            return Belongs::Unknown;
        };
        if self.digest.is_empty() {
            return Belongs::Unknown;
        }
        if self
            .digest
            .eq_ignore_ascii_case(&desdec_core::hash::to_hex(&digest))
        {
            Belongs::ToThisBinary
        } else {
            Belongs::ToAnother
        }
    }
}

/// Writes the work to a file.
///
/// # Errors
///
/// [`Error::Io`] when the file cannot be written — a read-only directory is
/// the ordinary case, since a reader may well be studying something they
/// cannot write beside.
pub fn write(path: &Path, session: &Session) -> Result<(), Error> {
    let text = ron::ser::to_string_pretty(session, ron::ser::PrettyConfig::default())
        .map_err(|error| Error::Unreadable(error.to_string()))?;
    std::fs::write(path, text).map_err(|error| Error::Io(describe(path, &error)))
}

/// Reads work back from a file.
///
/// # Errors
///
/// [`Error::Io`] when it cannot be read, [`Error::Unreadable`] when it is not
/// a session file, and [`Error::FromTheFuture`] when it was written by a
/// Desdec whose format this one does not know.
pub fn read(path: &Path) -> Result<Session, Error> {
    let text = std::fs::read_to_string(path).map_err(|error| Error::Io(describe(path, &error)))?;
    let session: Session =
        ron::from_str(&text).map_err(|error| Error::Unreadable(error.to_string()))?;
    // Checked after parsing rather than before: a later format may still parse
    // as this one and mean something else, and the version is what says so.
    if session.version > VERSION {
        return Err(Error::FromTheFuture {
            version: session.version,
        });
    }
    Ok(session)
}

/// An I/O failure said with the file it happened to.
fn describe(path: &Path, error: &io::Error) -> String {
    format!("{}: {error}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotations::Annotation;

    fn notes_naming(address: u64, label: &str) -> Annotations {
        let mut notes = Annotations::default();
        notes.set(
            address,
            Annotation {
                label: label.to_owned(),
                ..Annotation::default()
            },
        );
        notes
    }

    #[test]
    fn the_file_sits_beside_the_binary_and_keeps_its_extension() {
        assert_eq!(
            beside(Path::new("/tmp/program.exe")),
            Path::new("/tmp/program.exe.dcl")
        );
        // Otherwise `tool.exe` and `tool.dll` in one directory would share a
        // single `tool.dcl`, and whichever was saved last would win.
        assert_ne!(
            beside(Path::new("/tmp/tool.exe")),
            beside(Path::new("/tmp/tool.dll"))
        );
        assert_eq!(
            beside(Path::new("/tmp/program")),
            Path::new("/tmp/program.dcl")
        );
    }

    #[test]
    fn work_written_out_reads_back_the_same() {
        let directory = std::env::temp_dir().join("desdec-session-round-trip");
        let _ = std::fs::create_dir_all(&directory);
        let path = directory.join("program.dcl");

        let notes = notes_naming(0x0040_1000, "parse_header");
        let session = Session::of(Path::new("/tmp/program"), Some([7; 32]), &notes);
        write(&path, &session).expect("writable");
        let read_back = read(&path).expect("readable");

        assert_eq!(read_back, session);
        assert_eq!(read_back.notes.label(0x0040_1000), Some("parse_header"));
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// An address means nothing without the bytes it points into, so a file
    /// opened beside another binary has to be recognised as such.
    #[test]
    fn a_file_knows_which_binary_it_was_written_about() {
        let session = Session::of(
            Path::new("/tmp/program"),
            Some([7; 32]),
            &Annotations::default(),
        );

        assert_eq!(session.belongs_to(Some([7; 32])), Belongs::ToThisBinary);
        assert_eq!(session.belongs_to(Some([9; 32])), Belongs::ToAnother);
        // A binary read only in part has no digest, and nothing can be said.
        assert_eq!(session.belongs_to(None), Belongs::Unknown);
    }

    /// And a file that carries no digest — written about a binary too large to
    /// hash whole — says so rather than claiming to match.
    #[test]
    fn a_file_without_a_digest_claims_nothing() {
        let session = Session::of(Path::new("/tmp/program"), None, &Annotations::default());

        assert!(session.digest.is_empty());
        assert_eq!(session.belongs_to(Some([7; 32])), Belongs::Unknown);
    }

    /// A file from a later Desdec is refused with a sentence rather than read
    /// as something it is not.
    #[test]
    fn a_file_from_a_later_build_is_refused_by_name() {
        let directory = std::env::temp_dir().join("desdec-session-future");
        let _ = std::fs::create_dir_all(&directory);
        let path = directory.join("later.dcl");
        let later = Session {
            version: VERSION + 1,
            ..Session::default()
        };
        write(&path, &later).expect("writable");

        assert_eq!(
            read(&path),
            Err(Error::FromTheFuture {
                version: VERSION + 1
            })
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn something_that_is_not_a_session_is_not_read_as_one() {
        let directory = std::env::temp_dir().join("desdec-session-rubbish");
        let _ = std::fs::create_dir_all(&directory);
        let path = directory.join("rubbish.dcl");
        std::fs::write(&path, "this is not a session file").expect("writable");

        assert!(matches!(read(&path), Err(Error::Unreadable(_))));
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The whole cycle through the application: name something, save, lose the
    /// note, read it back. Each half passing on its own says nothing about the
    /// pair working — that is what a round trip is for.
    #[test]
    fn the_work_survives_a_save_and_an_open_through_the_application() {
        use crate::app::WorkspaceView;

        let mut app = crate::testing::opened_app(WorkspaceView::Functions);
        let Some(binary) = app
            .analysis
            .as_ref()
            .map(|analysis| analysis.summary.path.clone())
        else {
            return; // Nothing open on this host.
        };
        // Beside the binary is beside the *test* binary here, which is in
        // `target/`. That is where it belongs and it is removed on the way out.
        let path = beside(&binary);
        let _ = std::fs::remove_file(&path);

        let address = 0x0040_1000;
        app.annotations.set(
            address,
            Annotation {
                label: "written_by_the_reader".to_owned(),
                comment: "and a sentence about it".to_owned(),
                bookmarked: true,
                ..Annotation::default()
            },
        );
        app.save_session();
        assert!(path.is_file(), "nothing was written to {}", path.display());

        // Everything forgotten, the way a later session starts.
        app.annotations = Annotations::default();
        assert_eq!(app.annotations.label(address), None);

        app.open_session();
        assert_eq!(
            app.annotations.label(address),
            Some("written_by_the_reader"),
            "the name did not come back"
        );
        let note = app.annotations.at(address).expect("the note came back");
        assert_eq!(note.comment, "and a sentence about it");
        assert!(note.bookmarked, "the mark came back too");

        let _ = std::fs::remove_file(&path);
    }

    /// Saving with nothing open says so rather than writing a file named after
    /// nothing.
    #[test]
    fn saving_without_a_binary_writes_nothing() {
        use crate::app::WorkspaceView;

        let mut app = crate::app::DesdecApp::for_test(None, WorkspaceView::Functions);
        app.save_session();

        assert!(
            app.journal
                .last()
                .is_some_and(|entry| entry.level == crate::journal::Level::Warning),
            "the reader is told, rather than nothing happening"
        );
    }

    #[test]
    fn a_file_that_is_not_there_is_an_io_error_naming_it() {
        let path = std::env::temp_dir().join("desdec-session-absent/nothing.dcl");

        match read(&path) {
            Err(Error::Io(why)) => assert!(why.contains("nothing.dcl"), "{why}"),
            other => panic!("expected an I/O error naming the file: {other:?}"),
        }
    }
}
