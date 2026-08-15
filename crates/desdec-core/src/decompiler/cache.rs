//! On-disk cache of decompiled functions.
//!
//! Running `rizin` takes seconds even for one small function, most of it spent
//! starting the engine and loading its Sleigh grammars. Switching back to a
//! function already seen paid that cost again, so the answers are kept.
//!
//! What makes an entry reusable is decided by the key, never by the file name
//! alone:
//!
//! - **The binary's SHA-256**, so a rebuilt or edited file never reads the
//!   previous one's answers. A file whose digest is unknown — a truncated
//!   analysis — is not cached at all rather than keyed on something weaker.
//! - **The engine**, since two decompilers disagree about the same bytes.
//! - **The function address**, which is what was asked for.
//! - **A format version**, so a change to what is stored retires the old
//!   entries instead of misreading them.
//!
//! Entries are written to a temporary file and renamed into place, so a run
//! interrupted half-way leaves no half-written answer to be read back as if it
//! were the engine's.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use super::Engine;
use crate::analysis::hash;

/// Bumped when the stored format changes, retiring every earlier entry.
const FORMAT: u32 = 1;

/// What identifies one decompiled function.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Key<'a> {
    /// SHA-256 of the binary the function lives in.
    pub digest: &'a [u8; 32],
    pub engine: Engine,
    /// `None` is the entry point, which is what the engines default to.
    pub address: Option<u64>,
}

impl Key<'_> {
    /// File name for this entry: a digest of everything that identifies it, so
    /// the name is fixed-length and safe on every file system, and two
    /// different keys cannot collide into one name.
    fn file_name(&self) -> String {
        let mut material = Vec::with_capacity(64);
        material.extend_from_slice(&FORMAT.to_be_bytes());
        material.extend_from_slice(self.digest);
        material.extend_from_slice(self.engine.program().as_bytes());
        // The address is part of the material, and `None` is distinct from any
        // address rather than standing in for zero.
        match self.address {
            Some(address) => {
                material.push(1);
                material.extend_from_slice(&address.to_be_bytes());
            }
            None => material.push(0),
        }
        format!("{}.c", hash::to_hex(&hash::sha256(&material)))
    }

    fn path_in(&self, directory: &Path) -> PathBuf {
        directory.join(self.file_name())
    }
}

/// Reads a cached answer, or `None` when there is none to read.
///
/// A cache miss is never an error: an unreadable entry simply means the
/// engine has to run, which is exactly what would have happened anyway.
#[must_use]
pub fn read(directory: &Path, key: Key<'_>) -> Option<String> {
    let text = fs::read_to_string(key.path_in(directory)).ok()?;
    (!text.trim().is_empty()).then_some(text)
}

/// Stores an answer for next time.
///
/// # Errors
///
/// Returns an error when the cache directory cannot be created or written.
pub fn write(directory: &Path, key: Key<'_>, decompiled: &str) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let destination = key.path_in(directory);

    // A neighbouring temporary file, then a rename: a reader either sees the
    // previous entry or the complete new one, never a partial write. The name
    // carries the process id so two Desdec instances cannot share it.
    let temporary = directory.join(format!("{}.{}.tmp", key.file_name(), std::process::id()));
    let mut file = fs::File::create(&temporary)?;
    let written = file
        .write_all(decompiled.as_bytes())
        .and_then(|()| file.sync_all());
    drop(file);

    if let Err(error) = written {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    fs::rename(&temporary, &destination).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

/// Removes every cached answer, reporting how many entries went.
///
/// # Errors
///
/// Returns an error when the directory exists but cannot be read.
pub fn clear(directory: &Path) -> io::Result<usize> {
    if !directory.exists() {
        return Ok(0);
    }
    let mut removed = 0;
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        // Only what this module writes, so a directory the user pointed at by
        // mistake does not lose anything else.
        let ours = path
            .extension()
            .is_some_and(|extension| extension == "c" || extension == "tmp");
        if ours && fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Bytes currently held, for showing what clearing would free.
#[must_use]
pub fn size(directory: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .filter(std::fs::Metadata::is_file)
        .map(|metadata| metadata.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of its own per test, so they cannot disturb each other.
    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("desdec-cache-test-{name}"));
        let _ = fs::remove_dir_all(&directory);
        directory
    }

    const DIGEST: [u8; 32] = [7; 32];
    const OTHER_DIGEST: [u8; 32] = [9; 32];

    fn key(digest: &[u8; 32], address: Option<u64>) -> Key<'_> {
        Key {
            digest,
            engine: Engine::RzGhidra,
            address,
        }
    }

    #[test]
    fn an_answer_comes_back_exactly_as_it_was_stored() {
        let directory = scratch("roundtrip");
        let decompiled = "int main(void)\n{\n    return 42;\n}\n";

        write(&directory, key(&DIGEST, Some(0x1234)), decompiled).expect("the cache is writable");

        assert_eq!(
            read(&directory, key(&DIGEST, Some(0x1234))),
            Some(decompiled.to_owned())
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn nothing_is_read_before_anything_is_written() {
        let directory = scratch("empty");
        assert_eq!(read(&directory, key(&DIGEST, Some(0x1234))), None);
    }

    /// The point of keying on the digest: a rebuilt binary must not be shown
    /// the previous build's decompilation.
    #[test]
    fn another_binary_never_reads_this_ones_answers() {
        let directory = scratch("digest");
        write(&directory, key(&DIGEST, Some(0x1234)), "old build").expect("writable");

        assert_eq!(read(&directory, key(&OTHER_DIGEST, Some(0x1234))), None);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn each_function_has_its_own_entry() {
        let directory = scratch("functions");
        write(&directory, key(&DIGEST, Some(0x1000)), "first").expect("writable");
        write(&directory, key(&DIGEST, Some(0x2000)), "second").expect("writable");

        assert_eq!(
            read(&directory, key(&DIGEST, Some(0x1000))),
            Some("first".to_owned())
        );
        assert_eq!(
            read(&directory, key(&DIGEST, Some(0x2000))),
            Some("second".to_owned())
        );
        let _ = fs::remove_dir_all(&directory);
    }

    /// The entry point is asked for as `None`, which must not collide with the
    /// function that happens to live at address zero.
    #[test]
    fn the_entry_point_is_distinct_from_address_zero() {
        let directory = scratch("entry");
        write(&directory, key(&DIGEST, None), "entry point").expect("writable");
        write(&directory, key(&DIGEST, Some(0)), "address zero").expect("writable");

        assert_eq!(
            read(&directory, key(&DIGEST, None)),
            Some("entry point".to_owned())
        );
        assert_eq!(
            read(&directory, key(&DIGEST, Some(0))),
            Some("address zero".to_owned())
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn two_engines_do_not_share_an_entry() {
        let directory = scratch("engines");
        let rizin = Key {
            digest: &DIGEST,
            engine: Engine::RzGhidra,
            address: Some(0x1234),
        };
        let retdec = Key {
            engine: Engine::RetDec,
            ..rizin
        };
        write(&directory, rizin, "from rizin").expect("writable");

        assert_eq!(read(&directory, retdec), None);
        assert_eq!(read(&directory, rizin), Some("from rizin".to_owned()));
        let _ = fs::remove_dir_all(&directory);
    }

    /// An empty file is a failed write, not an answer: showing it would look
    /// like a decompiler that produced nothing.
    #[test]
    fn an_empty_entry_is_treated_as_a_miss() {
        let directory = scratch("blank");
        fs::create_dir_all(&directory).expect("creatable");
        fs::write(key(&DIGEST, Some(1)).path_in(&directory), "   \n").expect("writable");

        assert_eq!(read(&directory, key(&DIGEST, Some(1))), None);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn writing_again_replaces_the_previous_answer() {
        let directory = scratch("replace");
        write(&directory, key(&DIGEST, Some(5)), "first").expect("writable");
        write(&directory, key(&DIGEST, Some(5)), "second").expect("writable");

        assert_eq!(
            read(&directory, key(&DIGEST, Some(5))),
            Some("second".to_owned())
        );
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn clearing_removes_the_entries_and_reports_how_many() {
        let directory = scratch("clear");
        write(&directory, key(&DIGEST, Some(1)), "one").expect("writable");
        write(&directory, key(&DIGEST, Some(2)), "two").expect("writable");
        assert!(size(&directory) > 0);

        assert_eq!(clear(&directory).expect("readable"), 2);

        assert_eq!(read(&directory, key(&DIGEST, Some(1))), None);
        assert_eq!(size(&directory), 0);
        let _ = fs::remove_dir_all(&directory);
    }

    /// Clearing a directory that was never used is not an error.
    #[test]
    fn clearing_nothing_is_not_a_failure() {
        assert_eq!(clear(&scratch("absent")).expect("no directory is fine"), 0);
    }

    /// Whatever else lives in the directory the caller named stays there.
    #[test]
    fn clearing_leaves_files_this_module_did_not_write() {
        let directory = scratch("foreign");
        fs::create_dir_all(&directory).expect("creatable");
        let precious = directory.join("notes.txt");
        fs::write(&precious, "not ours").expect("writable");
        write(&directory, key(&DIGEST, Some(1)), "ours").expect("writable");

        assert_eq!(clear(&directory).expect("readable"), 1);

        assert!(precious.exists(), "an unrelated file was deleted");
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn no_temporary_file_is_left_behind() {
        let directory = scratch("temporaries");
        write(&directory, key(&DIGEST, Some(1)), "answer").expect("writable");

        let leftovers: Vec<_> = fs::read_dir(&directory)
            .expect("readable")
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|end| end == "tmp"))
            .collect();

        assert!(leftovers.is_empty(), "{leftovers:?}");
        let _ = fs::remove_dir_all(&directory);
    }
}
