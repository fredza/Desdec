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

    let mut counts = [0_u32; 256];
    for byte in bytes {
        counts[*byte as usize] += 1;
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "counts are compared as ratios; a bit of precision at extreme sizes is irrelevant"
    )]
    let total = bytes.len() as f32;
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
}
