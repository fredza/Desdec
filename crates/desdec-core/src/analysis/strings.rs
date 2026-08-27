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

/// Whether a run of printable bytes is an x86-64 register-save prologue
/// rather than text.
///
/// The pushes that open a function encode as bytes that are all printable
/// ASCII, so the extractor reports them as strings and a reader opening the
/// Strings view of any optimised binary meets dozens of them:
///
/// ```text
/// AUATUSH      41 55 41 54 55 53 48   push %r13; push %r12; push %rbp; push %rbx; …
/// AVAUATUSH9   41 56 41 55 41 54 …    push %r14; push %r13; push %r12; …
/// UAWAVSPH     55 41 57 41 56 53 50   push %rbp; push %r15; push %r14; …
/// ```
///
/// The grammar is the encoding itself: `0x50`–`0x57` is `push` of one of the
/// first eight registers, `0x41` is the `REX.B` prefix that names the other
/// eight, and `0x58`–`0x5F` are the matching `pop`s. What may follow is the
/// start of the next instruction — a `REX` prefix and an opcode or two, which
/// is why so many of these end in `H` (`0x48`, `REX.W`) or `I` (`0x49`).
///
/// **This is not a decision on its own.** `STATUS` is `push %rbx; push %rsp;
/// push %r12; push %rbp; push %rbx` read as code, and `TRUST` and `PURSUIT`
/// are pushes too — every one of them a word a program really contains. What
/// separates them is not their spelling but where they live: a prologue is in
/// an executable section and a message is not. So this answers only the
/// spelling question, and the caller is expected to have answered the other
/// one; see `ui::strings::Scope` for the pairing.
#[must_use]
pub fn is_register_save_prologue(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    let mut pushes = 0;

    while index < bytes.len() {
        // `REX.B` and then the push it applies to: `%r8` through `%r15`.
        if bytes[index] == REX_B && matches!(bytes.get(index + 1), Some(&next) if is_stack_op(next))
        {
            index += 2;
            pushes += 1;
            continue;
        }
        if is_stack_op(bytes[index]) {
            index += 1;
            pushes += 1;
            continue;
        }
        break;
    }

    // Two is the fewest that can be told apart from a coincidence: one push is
    // a single character, and the extractor reports nothing shorter than four.
    if pushes < 2 {
        return false;
    }

    // Whatever is left is the instruction the prologue was cut off in the
    // middle of. It has to *look* like the start of one — a `REX` prefix and
    // at most one byte behind it — or this is a string that merely began with
    // something push-shaped.
    //
    // One byte and not three, which is the whole difference between this and
    // a filter that hides words: `UPSTREAM` is five pushes and then `EAM`,
    // and `SUPPRESS` is five and then `ESS`. The cost is a prologue whose
    // next instruction happens to show three printable bytes, which stays in
    // the list — a miss, and the direction to miss in.
    match bytes.get(index) {
        None => true,
        Some(&byte) => is_rex(byte) && bytes.len() - index <= 2,
    }
}

/// `REX.B`, the prefix that turns a push of one of the first eight registers
/// into a push of one of the other eight.
const REX_B: u8 = 0x41;

/// `push` or `pop` of a whole register, which is one byte and its own opcode.
const fn is_stack_op(byte: u8) -> bool {
    byte >= 0x50 && byte <= 0x5F
}

/// Any of the sixteen `REX` prefixes, which is what an instruction operating
/// on 64-bit registers begins with.
const fn is_rex(byte: u8) -> bool {
    byte >= 0x40 && byte <= 0x4F
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The eight a reader actually met in the Strings view, which is where
    /// this came from.
    #[test]
    fn the_prologues_that_flood_the_strings_view_are_recognised() {
        for value in [
            "AUATI",
            "UAWAVSPH",
            "UAWAVAUATSH",
            "AWAVATSH",
            "AUATUSH",
            "AVAUATUSH9",
        ] {
            assert!(
                is_register_save_prologue(value),
                "{value} is a run of pushes"
            );
        }
    }

    /// The whole reason this answers only half the question. Every one of
    /// these is a word a program really contains, and every one of them reads
    /// as a valid sequence of pushes — `STATUS` is `push %rbx; push %rsp;
    /// push %r12; push %rbp; push %rbx`. Only the section they live in tells
    /// them apart, which is the caller's business.
    #[test]
    fn ordinary_words_that_happen_to_spell_pushes_are_not_settled_by_spelling() {
        for value in ["STATUS", "TRUST", "PURSUIT"] {
            assert!(
                is_register_save_prologue(value),
                "{value} really does read as pushes — the section is what separates it"
            );
        }
    }

    /// What the grammar does rule out, on spelling alone: a string that opens
    /// with something push-shaped and then goes on being text.
    #[test]
    fn text_that_merely_starts_with_a_push_is_not_a_prologue() {
        for value in [
            // Five pushes, a `REX` prefix, and then three more bytes: too
            // much to be the instruction a prologue was cut off in.
            "UPSTREAM", // Ends in a letter that is no `REX` prefix.
            "SUPPRESS",
            // A `REX` prefix, and then far too much to be one instruction.
            "USHERING", // One push is a coincidence, not a prologue.
            "Path",
        ] {
            assert!(
                !is_register_save_prologue(value),
                "{value} is not a prologue"
            );
        }
    }

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
