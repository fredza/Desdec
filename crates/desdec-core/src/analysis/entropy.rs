//! Shannon entropy over raw bytes.
//!
//! Entropy answers one practical question: does this region look like code and
//! data written by a compiler, or like compressed or encrypted content? It is
//! an indicator, never a verdict — a section can legitimately hold compressed
//! resources.

/// Entropy in bits per byte, from `0.0` (one repeated byte) to [`MAXIMUM`]
/// (every byte value equally likely).
pub const MAXIMUM: f32 = 8.0;

/// Above this, a region is dense enough that compression or encryption is the
/// most likely explanation. Ordinary machine code sits well below.
pub const PACKED_THRESHOLD: f32 = 7.2;

/// Returns `None` for an empty slice, where entropy is undefined.
#[must_use]
pub fn shannon(bytes: &[u8]) -> Option<f32> {
    if bytes.is_empty() {
        return None;
    }
    from_counts(&histogram(bytes), bytes.len())
}

/// Counts byte values, using every core once the input is large enough.
///
/// Counting is a sum, and a sum does not care how it was split: the totals are
/// identical whatever the number of threads, so the entropy of a file never
/// depends on the machine that measured it.
///
/// `u32` counters cannot overflow here: [`super::ANALYSIS_BYTE_LIMIT`] caps a
/// region at 256 MB, far below the 4.29 billion a `u32` holds.
fn histogram(bytes: &[u8]) -> [u32; 256] {
    let workers = crate::parallel::worker_count(bytes.len());
    if workers <= 1 {
        return count(bytes);
    }
    crate::parallel::map_chunks(bytes, workers, count)
        .into_iter()
        .fold([0_u32; 256], |mut total, partial| {
            for (slot, value) in total.iter_mut().zip(partial) {
                *slot += value;
            }
            total
        })
}

fn count(bytes: &[u8]) -> [u32; 256] {
    let mut counts = [0_u32; 256];
    for byte in bytes {
        counts[*byte as usize] += 1;
    }
    counts
}

fn from_counts(counts: &[u32; 256], length: usize) -> Option<f32> {
    if length == 0 {
        return None;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "counts are compared as ratios; a bit of precision at extreme sizes is irrelevant"
    )]
    let total = length as f32;
    let entropy = counts
        .iter()
        .filter(|count| **count > 0)
        .map(|count| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "see above: this is a ratio, not an exact count"
            )]
            let probability = *count as f32 / total;
            -probability * probability.log2()
        })
        .sum::<f32>();
    // A region of one repeated byte sums to `-0.0`, which would be displayed as
    // "-0.00". Zero has no sign here.
    Some(entropy + 0.0)
}

/// Whether a region is dense enough to suggest packing or encryption.
#[must_use]
pub fn suggests_packing(entropy: f32) -> bool {
    entropy >= PACKED_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undefined_for_an_empty_region() {
        assert_eq!(shannon(&[]), None);
    }

    #[test]
    fn a_single_repeated_byte_carries_no_information() {
        let entropy = shannon(&[0x41; 64]).expect("a non-empty region has entropy");
        assert!(entropy.abs() < f32::EPSILON, "got {entropy}");
        assert!(
            entropy.is_sign_positive(),
            "zero entropy must not be displayed as -0.00"
        );
    }

    #[test]
    fn two_equally_likely_bytes_carry_one_bit() {
        let bytes: Vec<u8> = (0..64).map(|index| index % 2).collect();
        assert_eq!(shannon(&bytes), Some(1.0));
    }

    #[test]
    fn every_byte_value_reaches_the_maximum() {
        let bytes: Vec<u8> = (0..=255).collect();
        let entropy = shannon(&bytes).expect("a full byte range has entropy");
        assert!((entropy - MAXIMUM).abs() < f32::EPSILON, "got {entropy}");
        assert!(suggests_packing(entropy));
    }

    #[test]
    fn plain_text_stays_far_below_the_packing_threshold() {
        let text = b"The quick brown fox jumps over the lazy dog. ".repeat(8);
        let entropy = shannon(&text).expect("text has entropy");
        assert!(entropy < 5.0, "got {entropy}");
        assert!(!suggests_packing(entropy));
    }

    /// Entropy is reported as a measurement, so it must not depend on how many
    /// cores happened to count the bytes.
    #[test]
    fn the_answer_is_the_same_however_many_threads_counted() {
        // Large enough to be split, and deliberately uneven so the chunk
        // boundaries do not line up with the repeating pattern.
        let bytes: Vec<u8> = (0..5_000_003_u32)
            .map(|index| (index % 251) as u8)
            .collect();

        let reference = count(&bytes);
        for workers in [1, 2, 3, 8, 64] {
            let counts = crate::parallel::map_chunks(&bytes, workers, count)
                .into_iter()
                .fold([0_u32; 256], |mut total, partial| {
                    for (slot, value) in total.iter_mut().zip(partial) {
                        *slot += value;
                    }
                    total
                });
            assert_eq!(counts, reference, "{workers} threads changed the counts");
        }
        assert_eq!(shannon(&bytes), from_counts(&reference, bytes.len()));
    }

    /// Counts of a large file must not wrap, which `u32` would risk on inputs
    /// of a few gigabytes made mostly of one byte value.
    #[test]
    fn counts_have_room_for_a_large_file() {
        let bytes = vec![0_u8; 4096];
        assert_eq!(histogram(&bytes)[0], 4096);
    }
}
