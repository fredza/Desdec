//! SHA-256, implemented here so the core keeps no dependencies.
//!
//! A file hash is how an analyst names a sample: it identifies the exact bytes
//! examined, survives renaming, and can be compared against a public report.

/// Round constants: the first 32 bits of the fractional parts of the cube roots
/// of the first 64 primes (FIPS 180-4, §4.2.2).
const ROUND_CONSTANTS: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// Initial state: the first 32 bits of the fractional parts of the square roots
/// of the first 8 primes (FIPS 180-4, §5.3.3).
const INITIAL_STATE: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

const BLOCK_BYTES: usize = 64;

/// SHA-256 digest of `bytes`.
#[must_use]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut state = INITIAL_STATE;

    let mut blocks = bytes.chunks_exact(BLOCK_BYTES);
    for block in &mut blocks {
        compress(&mut state, block);
    }

    // Final block(s): the remainder, a 0x80 marker, zero padding, and the
    // message length in bits as a big-endian u64.
    let remainder = blocks.remainder();
    let mut tail = [0_u8; BLOCK_BYTES * 2];
    tail[..remainder.len()].copy_from_slice(remainder);
    tail[remainder.len()] = 0x80;

    let bit_length = (bytes.len() as u64).wrapping_mul(8);
    let padded = if remainder.len() + 1 + 8 > BLOCK_BYTES {
        BLOCK_BYTES * 2
    } else {
        BLOCK_BYTES
    };
    tail[padded - 8..padded].copy_from_slice(&bit_length.to_be_bytes());
    for block in tail[..padded].chunks_exact(BLOCK_BYTES) {
        compress(&mut state, block);
    }

    let mut digest = [0_u8; 32];
    for (target, word) in digest.chunks_exact_mut(4).zip(state) {
        target.copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// Lowercase hexadecimal form, as every tool that prints hashes uses.
#[must_use]
pub fn to_hex(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;

    digest
        .iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

/// Processes one 64-byte block (FIPS 180-4, §6.2.2).
#[expect(
    clippy::many_single_char_names,
    reason = "a..h are the working variable names used by the specification"
)]
fn compress(state: &mut [u32; 8], block: &[u8]) {
    debug_assert_eq!(block.len(), BLOCK_BYTES);

    let mut schedule = [0_u32; 64];
    for (word, chunk) in schedule.iter_mut().zip(block.chunks_exact(4)) {
        *word = u32::from_be_bytes(chunk.try_into().unwrap_or([0; 4]));
    }
    for index in 16..64 {
        let previous = schedule[index - 15];
        let recent = schedule[index - 2];
        let s0 = previous.rotate_right(7) ^ previous.rotate_right(18) ^ (previous >> 3);
        let s1 = recent.rotate_right(17) ^ recent.rotate_right(19) ^ (recent >> 10);
        schedule[index] = schedule[index - 16]
            .wrapping_add(s0)
            .wrapping_add(schedule[index - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for (word, constant) in schedule.into_iter().zip(ROUND_CONSTANTS) {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choose = (e & f) ^ (!e & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(choose)
            .wrapping_add(constant)
            .wrapping_add(word);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(majority);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    for (current, addition) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *current = current.wrapping_add(addition);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_of(input: &[u8]) -> String {
        to_hex(&sha256(input))
    }

    /// Vectors published with FIPS 180-4 and its test suite.
    #[test]
    fn matches_the_published_vectors() {
        assert_eq!(
            hex_of(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex_of(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex_of(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    /// Exercises every padding case around the 64-byte block boundary: a block
    /// that still fits its length field, one that does not, and an exact block.
    #[test]
    fn padding_is_correct_around_a_block_boundary() {
        assert_eq!(
            hex_of(&[b'a'; 55]),
            "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
        );
        assert_eq!(
            hex_of(&[b'a'; 56]),
            "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
        );
        assert_eq!(
            hex_of(&[b'a'; 64]),
            "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
        );
    }

    #[test]
    fn a_million_repeated_letters_match_the_reference_digest() {
        assert_eq!(
            hex_of(&vec![b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn hex_is_lowercase_and_fixed_width() {
        let hex = hex_of(b"desdec");
        assert_eq!(hex.len(), 64);
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        );
    }
}
