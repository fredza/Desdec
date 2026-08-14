//! Printable string extraction.
//!
//! Strings are often the fastest way into an unknown binary: they name files,
//! URLs, error messages and licence checks. Both encodings that matter in
//! practice are recognised — plain ASCII, and the UTF-16 little-endian used
//! throughout Windows binaries.

/// Shortest run of printable characters worth reporting. Below four, the noise
/// from ordinary code and data drowns out anything useful.
pub const MINIMUM_LENGTH: usize = 4;

/// Longest string kept whole. Anything longer is truncated, so a single
/// megabyte-sized blob cannot dominate the list.
pub const MAXIMUM_LENGTH: usize = 400;

/// Upper bound on how many strings are collected, keeping the analysis bounded
/// on binaries that are essentially one big string table.
pub const MAXIMUM_COUNT: usize = 20_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StringEncoding {
    Ascii,
    Utf16Le,
}

impl StringEncoding {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ascii => "ASCII",
            Self::Utf16Le => "UTF-16LE",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractedString {
    /// Where the string starts in the file, so it can be located in a hex view.
    pub file_offset: u64,
    pub encoding: StringEncoding,
    pub value: String,
    /// Set when the run was longer than [`MAXIMUM_LENGTH`] and was cut short.
    pub truncated: bool,
}

/// Collects printable strings, ordered by file offset.
///
/// Runs are reported once: a UTF-16 string is not also reported as the shorter
/// ASCII fragments its interleaved zero bytes would otherwise produce.
#[must_use]
pub fn extract(bytes: &[u8]) -> Vec<ExtractedString> {
    let mut found = Vec::new();
    let mut index = 0;

    while index < bytes.len() && found.len() < MAXIMUM_COUNT {
        if let Some(string) = read_utf16le(bytes, index) {
            index += string.consumed;
            found.push(string.string);
            continue;
        }
        if let Some(string) = read_ascii(bytes, index) {
            index += string.consumed;
            found.push(string.string);
            continue;
        }
        index += 1;
    }

    found
}

/// A recognised run, plus how many bytes of input it covered.
struct Run {
    string: ExtractedString,
    consumed: usize,
}

const fn is_printable(byte: u8) -> bool {
    byte.is_ascii_graphic() || byte == b' ' || byte == b'\t'
}

fn read_ascii(bytes: &[u8], start: usize) -> Option<Run> {
    let length = bytes[start..]
        .iter()
        .take_while(|byte| is_printable(**byte))
        .count();
    if length < MINIMUM_LENGTH {
        return None;
    }

    let kept = length.min(MAXIMUM_LENGTH);
    let value = String::from_utf8_lossy(&bytes[start..start + kept]).into_owned();
    Some(Run {
        string: ExtractedString {
            file_offset: start as u64,
            encoding: StringEncoding::Ascii,
            value,
            truncated: length > kept,
        },
        consumed: length,
    })
}

/// Recognises the `c\0h\0a\0r\0` pattern of UTF-16 little-endian text.
fn read_utf16le(bytes: &[u8], start: usize) -> Option<Run> {
    let length = bytes[start..]
        .chunks_exact(2)
        .take_while(|pair| is_printable(pair[0]) && pair[1] == 0)
        .count();
    if length < MINIMUM_LENGTH {
        return None;
    }

    let kept = length.min(MAXIMUM_LENGTH);
    let value = bytes[start..start + kept * 2]
        .chunks_exact(2)
        .map(|pair| char::from(pair[0]))
        .collect();
    Some(Run {
        string: ExtractedString {
            file_offset: start as u64,
            encoding: StringEncoding::Utf16Le,
            value,
            truncated: length > kept,
        },
        consumed: length * 2,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_runs_are_ignored() {
        assert!(extract(b"\x00abc\x00").is_empty());
    }

    #[test]
    fn ascii_runs_are_reported_with_their_offset() {
        let found = extract(b"\x00\x00/usr/bin/env\x00\x01");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].value, "/usr/bin/env");
        assert_eq!(found[0].file_offset, 2);
        assert_eq!(found[0].encoding, StringEncoding::Ascii);
        assert!(!found[0].truncated);
    }

    #[test]
    fn utf16_runs_are_not_split_into_ascii_fragments() {
        let mut bytes = vec![0_u8];
        for character in "C:\\Windows".bytes() {
            bytes.push(character);
            bytes.push(0);
        }
        bytes.push(0xff);

        let found = extract(&bytes);
        assert_eq!(found.len(), 1, "got {found:?}");
        assert_eq!(found[0].value, "C:\\Windows");
        assert_eq!(found[0].encoding, StringEncoding::Utf16Le);
        assert_eq!(found[0].file_offset, 1);
    }

    #[test]
    fn several_strings_keep_their_file_order() {
        let found = extract(b"first\x00\x00second\x00third!");
        let values: Vec<&str> = found.iter().map(|string| string.value.as_str()).collect();
        assert_eq!(values, ["first", "second", "third!"]);
        assert!(
            found
                .windows(2)
                .all(|pair| pair[0].file_offset < pair[1].file_offset)
        );
    }

    #[test]
    fn overlong_runs_are_truncated_but_still_advance() {
        let bytes = vec![b'A'; MAXIMUM_LENGTH * 2];
        let found = extract(&bytes);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].value.len(), MAXIMUM_LENGTH);
        assert!(found[0].truncated);
    }

    #[test]
    fn extraction_stays_bounded_on_pathological_input() {
        // One qualifying string every five bytes, far past the cap.
        let bytes = b"abcd\x00".repeat(MAXIMUM_COUNT * 2);
        assert_eq!(extract(&bytes).len(), MAXIMUM_COUNT);
    }

    #[test]
    fn control_bytes_never_reach_the_output() {
        let found = extract(b"ok\x07fine\x00");
        assert!(
            found
                .iter()
                .all(|string| string.value.bytes().all(is_printable)),
            "got {found:?}"
        );
    }
}
