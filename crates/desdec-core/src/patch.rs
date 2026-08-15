//! Byte-level patches, and how they reach a file.
//!
//! A patch replaces bytes in place, never inserting or removing any: every
//! offset in the file — relocations, tables, headers — stays where it was.
//! That is why [`Patch::new`] refuses a replacement of a different length
//! instead of silently shifting the rest of the image.
//!
//! Patches are written to a copy. The analysed file is opened read-only by the
//! rest of this crate, and nothing here changes that.

use std::{fs, io, path::Path};

/// One replacement, anchored by its position in the file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Patch {
    /// Byte offset in the file, which is what actually gets written.
    pub file_offset: u64,
    /// Virtual address the bytes are decoded at, for display and for
    /// re-decoding the instruction.
    pub address: u64,
    pub original: Vec<u8>,
    pub replacement: Vec<u8>,
}

/// Why a patch was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PatchError {
    /// Replacing a different number of bytes would move everything after it.
    LengthChanged { expected: usize, found: usize },
    /// A patch with no bytes changes nothing.
    Empty,
    /// The bytes do not lie inside the file.
    OutOfBounds,
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LengthChanged { expected, found } => write!(
                formatter,
                "a patch must keep the instruction length: {expected} bytes expected, {found} given"
            ),
            Self::Empty => write!(formatter, "a patch cannot be empty"),
            Self::OutOfBounds => write!(formatter, "the patch falls outside the file"),
        }
    }
}

impl std::error::Error for PatchError {}

impl Patch {
    /// Builds a patch, refusing anything that would move the bytes after it.
    ///
    /// # Errors
    ///
    /// Returns [`PatchError`] when the replacement is empty or of a different
    /// length than the bytes it replaces.
    pub fn new(
        file_offset: u64,
        address: u64,
        original: Vec<u8>,
        replacement: Vec<u8>,
    ) -> Result<Self, PatchError> {
        if original.is_empty() || replacement.is_empty() {
            return Err(PatchError::Empty);
        }
        if original.len() != replacement.len() {
            return Err(PatchError::LengthChanged {
                expected: original.len(),
                found: replacement.len(),
            });
        }
        Ok(Self {
            file_offset,
            address,
            original,
            replacement,
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.replacement.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.replacement.is_empty()
    }

    /// Whether the patch would actually change anything.
    #[must_use]
    pub fn changes_anything(&self) -> bool {
        self.original != self.replacement
    }

    /// Byte range covered in the file.
    #[must_use]
    pub fn range(&self) -> std::ops::Range<u64> {
        self.file_offset..self.file_offset.saturating_add(self.len() as u64)
    }

    /// Whether two patches cover overlapping bytes, which would make the
    /// written result depend on the order they were applied in.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        let (left, right) = (self.range(), other.range());
        left.start < right.end && right.start < left.end
    }
}

/// Applies every patch to a copy of `bytes`.
///
/// # Errors
///
/// Returns [`PatchError::OutOfBounds`] when a patch does not fit in the file.
pub fn apply(bytes: &[u8], patches: &[Patch]) -> Result<Vec<u8>, PatchError> {
    let mut result = bytes.to_vec();
    for patch in patches {
        let start = usize::try_from(patch.file_offset).map_err(|_| PatchError::OutOfBounds)?;
        let end = start
            .checked_add(patch.len())
            .ok_or(PatchError::OutOfBounds)?;
        let target = result.get_mut(start..end).ok_or(PatchError::OutOfBounds)?;
        target.copy_from_slice(&patch.replacement);
    }
    Ok(result)
}

/// Writes a patched copy of `source` to `destination`.
///
/// The whole file is read, patched in memory and written in one go, so a
/// failure part-way leaves no half-patched binary behind. `source` is never
/// opened for writing — refusing to write over it is the caller's decision to
/// make, and here it is made once and for all.
///
/// # Errors
///
/// Returns an error when the source cannot be read, a patch does not fit, or
/// the destination cannot be written.
pub fn write_patched_copy(source: &Path, destination: &Path, patches: &[Patch]) -> io::Result<u64> {
    if source == destination {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "patches are exported to a copy, never over the analysed file",
        ));
    }
    let contents = fs::read(source)?;
    let result = apply(&contents, patches).map_err(io::Error::other)?;
    fs::write(destination, &result)?;
    Ok(result.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(offset: u64, original: &[u8], replacement: &[u8]) -> Patch {
        Patch::new(
            offset,
            0x40_0000 + offset,
            original.to_vec(),
            replacement.to_vec(),
        )
        .expect("the test patches keep their length")
    }

    #[test]
    fn a_replacement_of_a_different_length_is_refused() {
        let refused = Patch::new(0, 0, vec![0x55], vec![0x48, 0x89]);
        assert_eq!(
            refused,
            Err(PatchError::LengthChanged {
                expected: 1,
                found: 2
            })
        );
    }

    #[test]
    fn an_empty_patch_is_refused() {
        assert_eq!(Patch::new(0, 0, vec![], vec![]), Err(PatchError::Empty));
        assert_eq!(Patch::new(0, 0, vec![0x90], vec![]), Err(PatchError::Empty));
    }

    #[test]
    fn patches_replace_bytes_where_they_are() {
        let file = [0x55, 0x48, 0x89, 0xe5, 0xc3];
        let patched = apply(&file, &[patch(0, &[0x55], &[0x90])]).expect("the patch fits");

        assert_eq!(patched, [0x90, 0x48, 0x89, 0xe5, 0xc3]);
        assert_eq!(patched.len(), file.len(), "the file size never changes");
    }

    #[test]
    fn several_patches_all_land() {
        let file = [0x55, 0x48, 0x89, 0xe5, 0xc3];
        let patched = apply(
            &file,
            &[patch(0, &[0x55], &[0x90]), patch(4, &[0xc3], &[0xcc])],
        )
        .expect("both patches fit");

        assert_eq!(patched, [0x90, 0x48, 0x89, 0xe5, 0xcc]);
    }

    #[test]
    fn a_patch_past_the_end_is_refused_rather_than_growing_the_file() {
        let file = [0x55, 0xc3];
        assert_eq!(
            apply(&file, &[patch(1, &[0xc3, 0x90], &[0x90, 0x90])]),
            Err(PatchError::OutOfBounds)
        );
    }

    #[test]
    fn overlapping_patches_are_recognised() {
        let first = patch(4, &[0x48, 0x89], &[0x90, 0x90]);
        assert!(first.overlaps(&patch(5, &[0x89], &[0x90])));
        assert!(!first.overlaps(&patch(6, &[0xe5], &[0x90])));
        assert!(!first.overlaps(&patch(3, &[0xe5], &[0x90])));
    }

    #[test]
    fn a_patch_that_rewrites_the_same_bytes_changes_nothing() {
        assert!(!patch(0, &[0x55], &[0x55]).changes_anything());
        assert!(patch(0, &[0x55], &[0x90]).changes_anything());
    }

    #[test]
    fn exporting_never_writes_over_the_analysed_file() {
        let source = std::env::temp_dir().join("desdec-patch-source-test.bin");
        fs::write(&source, [0x55, 0xc3]).expect("the test file is writable");

        let refused = write_patched_copy(&source, &source, &[patch(0, &[0x55], &[0x90])]);

        assert_eq!(
            refused
                .expect_err("writing over the source must fail")
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            fs::read(&source).expect("the source is still readable"),
            [0x55, 0xc3],
            "the analysed file must be untouched"
        );
        let _ = fs::remove_file(&source);
    }

    #[test]
    fn an_exported_copy_carries_the_patches() {
        let directory = std::env::temp_dir();
        let source = directory.join("desdec-patch-export-source.bin");
        let destination = directory.join("desdec-patch-export-copy.bin");
        fs::write(&source, [0x55, 0x48, 0xc3]).expect("the test file is writable");

        let written = write_patched_copy(&source, &destination, &[patch(0, &[0x55], &[0x90])])
            .expect("the copy is writable");

        assert_eq!(written, 3);
        assert_eq!(
            fs::read(&destination).expect("the copy is readable"),
            [0x90, 0x48, 0xc3]
        );
        assert_eq!(
            fs::read(&source).expect("the source is readable"),
            [0x55, 0x48, 0xc3],
            "the analysed file must be untouched"
        );
        let _ = fs::remove_file(&source);
        let _ = fs::remove_file(&destination);
    }
}
