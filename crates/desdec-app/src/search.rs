//! Finding things in a binary: a run of bytes, an instruction, a note.
//!
//! Three questions a reader asks that no view answers by scrolling. Each is a
//! scan of what has already been read — nothing here goes back to the disk —
//! and each stops at a bound: a pattern of two bytes matches tens of thousands
//! of times in a large image, and a list that long is not an answer.

use desdec_core::Analysis;

use crate::annotations::Annotations;

/// What is being looked for.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Mode {
    /// A run of bytes, with `??` standing for any byte.
    #[default]
    Bytes,
    /// Text in the decoded instructions.
    Instructions,
    /// Text in the reader's own labels and comments.
    Notes,
}

impl Mode {
    pub const ALL: &[Self] = &[Self::Bytes, Self::Instructions, Self::Notes];
}

/// One thing found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hit {
    /// Where it is in memory, when the bytes it was found in are mapped.
    pub address: Option<u64>,
    /// Where it is in the file, for a hit found by scanning bytes.
    pub file_offset: Option<u64>,
    /// The section it falls in, when it falls in one.
    pub section: Option<String>,
    /// The line to show for it.
    pub text: String,
}

/// What a search found, and whether it stopped early.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Results {
    pub hits: Vec<Hit>,
    /// Set when the bound was reached and the file holds more.
    pub truncated: bool,
}

/// How many hits are collected before a search gives up.
///
/// A short byte pattern matches everywhere; a reader given fifty thousand rows
/// has been answered with the file itself. Better to say the list is cut and
/// let them narrow the pattern.
pub const LIMIT: usize = 500;

/// A run of bytes to look for, some of which may be anything.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Pattern {
    bytes: Vec<Option<u8>>,
}

impl Pattern {
    /// Reads `48 8b ?? 05`, or `488b??05`, or any mix of the two.
    ///
    /// `None` when the text is not a pattern at all — a half-written one is
    /// not an error to complain about, it is a reader still typing.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut bytes = Vec::new();
        for word in text.split_whitespace() {
            let digits: Vec<char> = word.chars().collect();
            if digits.len() % 2 != 0 {
                return None;
            }
            for pair in digits.chunks_exact(2) {
                if pair.iter().all(|digit| *digit == '?') {
                    bytes.push(None);
                    continue;
                }
                let pair: String = pair.iter().collect();
                bytes.push(Some(u8::from_str_radix(&pair, 16).ok()?));
            }
        }
        (!bytes.is_empty()).then_some(Self { bytes })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Every offset in `haystack` where the pattern matches, up to `limit`.
    #[must_use]
    pub fn find(&self, haystack: &[u8], limit: usize) -> (Vec<usize>, bool) {
        let mut found = Vec::new();
        if self.bytes.is_empty() || haystack.len() < self.bytes.len() {
            return (found, false);
        }
        for offset in 0..=(haystack.len() - self.bytes.len()) {
            let matches = self
                .bytes
                .iter()
                .zip(&haystack[offset..])
                .all(|(wanted, byte)| wanted.is_none_or(|wanted| wanted == *byte));
            if matches {
                if found.len() == limit {
                    return (found, true);
                }
                found.push(offset);
            }
        }
        (found, false)
    }
}

/// Where a file offset is mapped, and in which section.
fn mapped(analysis: &Analysis, offset: u64) -> (Option<u64>, Option<String>) {
    match analysis.address_at(offset) {
        Some((address, section)) => (Some(address), Some(section.name.clone())),
        None => (None, None),
    }
}

/// Runs of bytes matching a pattern, at most [`LIMIT`] of them.
#[must_use]
pub fn bytes(analysis: &Analysis, file: &[u8], pattern: &Pattern) -> Results {
    bytes_within(analysis, file, pattern, LIMIT)
}

/// The same, bounded by the caller.
///
/// A reader is answered on screen, where a few hundred rows is already more
/// than anyone reads; a script is answered in a loop it wrote itself, and
/// cutting that at the same place would quietly rename four hundred of the
/// nine hundred functions it found.
#[must_use]
pub fn bytes_within(analysis: &Analysis, file: &[u8], pattern: &Pattern, limit: usize) -> Results {
    let (offsets, truncated) = pattern.find(file, limit);
    let hits = offsets
        .into_iter()
        .map(|offset| {
            let end = offset.saturating_add(pattern.len()).min(file.len());
            let text = file[offset..end]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            let offset = u64::try_from(offset).unwrap_or(u64::MAX);
            let (address, section) = mapped(analysis, offset);
            Hit {
                address,
                file_offset: Some(offset),
                section,
                text,
            }
        })
        .collect();
    Results { hits, truncated }
}

/// Decoded instructions whose text contains `needle`, ignoring case.
#[must_use]
pub fn instructions(analysis: &Analysis, needle: &str) -> Results {
    instructions_within(analysis, needle, LIMIT)
}

/// The same, bounded by the caller.
#[must_use]
pub fn instructions_within(analysis: &Analysis, needle: &str, limit: usize) -> Results {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return Results::default();
    }
    let mut results = Results::default();
    for instruction in &analysis.instructions {
        if !instruction.text.to_lowercase().contains(&needle) {
            continue;
        }
        if results.hits.len() == limit {
            results.truncated = true;
            break;
        }
        results.hits.push(Hit {
            address: Some(instruction.address),
            file_offset: None,
            section: Some(instruction.section.to_string()),
            text: instruction.text.clone(),
        });
    }
    results
}

/// The reader's own notes, whole or narrowed by `needle`.
///
/// An empty needle lists every note there is, which is what makes this the
/// bookmark list as well as a search.
#[must_use]
pub fn notes(analysis: &Analysis, annotations: &Annotations, needle: &str) -> Results {
    notes_within(analysis, annotations, needle, LIMIT)
}

/// The same, bounded by the caller.
#[must_use]
pub fn notes_within(
    analysis: &Analysis,
    annotations: &Annotations,
    needle: &str,
    limit: usize,
) -> Results {
    let needle = needle.trim().to_lowercase();
    let mut results = Results::default();
    for (address, annotation) in annotations.iter() {
        let haystack = format!("{} {}", annotation.label, annotation.comment).to_lowercase();
        if !needle.is_empty() && !haystack.contains(&needle) {
            continue;
        }
        if results.hits.len() == limit {
            results.truncated = true;
            break;
        }
        let mut text = String::new();
        if annotation.bookmarked {
            text.push_str("\u{2605} ");
        }
        if !annotation.label.trim().is_empty() {
            text.push_str(annotation.label.trim());
            text.push_str(": ");
        }
        text.push_str(annotation.comment.trim());
        results.hits.push(Hit {
            address: Some(address),
            file_offset: None,
            section: analysis
                .section_at(address)
                .map(|section| section.name.clone()),
            text: text.trim_end_matches(": ").to_owned(),
        });
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotations::Annotation;

    #[test]
    fn a_pattern_reads_both_spellings_and_its_wildcards() {
        assert_eq!(
            Pattern::parse("48 8b ?? 05"),
            Pattern::parse("488b??05"),
            "spacing is not part of a pattern"
        );
        let pattern = Pattern::parse("48 ?? 05").expect("a pattern");
        assert_eq!(pattern.len(), 3);

        // Not patterns: a lone digit, something that is not hexadecimal, and
        // nothing at all.
        assert_eq!(Pattern::parse("4"), None);
        assert_eq!(Pattern::parse("zz"), None);
        assert_eq!(Pattern::parse("   "), None);
    }

    #[test]
    fn a_wildcard_matches_any_byte_and_the_rest_must_match_exactly() {
        let haystack = [0x48, 0x8b, 0x05, 0x48, 0x8b, 0x99, 0x48, 0x00, 0x05];
        let pattern = Pattern::parse("48 8b ??").expect("a pattern");

        let (found, truncated) = pattern.find(&haystack, LIMIT);

        assert_eq!(found, vec![0, 3]);
        assert!(!truncated);
    }

    /// A short pattern matches everywhere; the answer says so rather than
    /// listing the file back at the reader.
    #[test]
    fn a_search_stops_at_its_bound_and_says_it_did() {
        let haystack = vec![0x90; 32];
        let pattern = Pattern::parse("90").expect("a pattern");

        let (found, truncated) = pattern.find(&haystack, 10);

        assert_eq!(found.len(), 10);
        assert!(truncated, "the file holds more than was listed");
    }

    #[test]
    fn instructions_are_searched_without_case() {
        let analysis = crate::testing::reference_analysis();
        let Some(first) = analysis.instructions.first() else {
            return;
        };
        let mnemonic = first
            .text
            .split_whitespace()
            .next()
            .expect("an instruction has a mnemonic")
            .to_uppercase();

        let results = instructions(analysis, &mnemonic);

        assert!(
            results
                .hits
                .iter()
                .any(|hit| hit.address == Some(first.address)),
            "{mnemonic} must find the instruction it was taken from"
        );
    }

    /// An empty needle lists every note, which is what makes this the list of
    /// bookmarks as well as a search.
    #[test]
    fn an_empty_search_lists_every_note() {
        let analysis = crate::testing::reference_analysis();
        let mut annotations = Annotations::default();
        annotations.set(
            0x0040_1000,
            Annotation {
                label: "parse_header".to_owned(),
                comment: "reads the magic".to_owned(),
                bookmarked: true,
            },
        );
        annotations.toggle_bookmark(0x0040_2000);

        assert_eq!(notes(analysis, &annotations, "").hits.len(), 2);
        let narrowed = notes(analysis, &annotations, "magic");
        assert_eq!(narrowed.hits.len(), 1);
        assert_eq!(narrowed.hits[0].address, Some(0x0040_1000));
        assert!(narrowed.hits[0].text.contains("parse_header"));
    }
}
