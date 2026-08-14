//! Bounds-checked reads over an untrusted byte slice.
//!
//! Every accessor returns `None` instead of panicking: a truncated, padded or
//! deliberately malformed file must never crash the analysis.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Endianness {
    Little,
    Big,
    Unknown,
}

impl Endianness {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Little => "little-endian",
            Self::Big => "big-endian",
            Self::Unknown => "unknown endianness",
        }
    }
}

/// Declares a bounds-checked, endianness-aware integer reader.
macro_rules! read_int {
    ($name:ident, $integer:ty) => {
        #[must_use]
        pub fn $name(bytes: &[u8], offset: usize, endianness: Endianness) -> Option<$integer> {
            const WIDTH: usize = std::mem::size_of::<$integer>();

            let end = offset.checked_add(WIDTH)?;
            let input: [u8; WIDTH] = bytes.get(offset..end)?.try_into().ok()?;
            Some(match endianness {
                Endianness::Little => <$integer>::from_le_bytes(input),
                Endianness::Big => <$integer>::from_be_bytes(input),
                Endianness::Unknown => return None,
            })
        }
    };
}

read_int!(read_u16, u16);
read_int!(read_u32, u32);
read_int!(read_u64, u64);

/// Borrows `length` bytes at `offset`, or `None` if they are not all present.
#[must_use]
pub fn read_slice(bytes: &[u8], offset: usize, length: usize) -> Option<&[u8]> {
    bytes.get(offset..offset.checked_add(length)?)
}

/// Reads a fixed-width name field, dropping the zero padding that follows it.
///
/// Used by PE section headers and Mach-O segment names, which pad to 8 and 16
/// bytes respectively. Bytes that are not printable ASCII are replaced, so a
/// corrupted name can never inject control characters into the interface.
#[must_use]
pub fn read_padded_name(bytes: &[u8], offset: usize, width: usize) -> Option<String> {
    let field = read_slice(bytes, offset, width)?;
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(width);
    Some(sanitise(&field[..end]))
}

/// Reads a zero-terminated name starting at `offset`, bounded by `limit` bytes.
#[must_use]
pub fn read_c_string(bytes: &[u8], offset: usize, limit: usize) -> Option<String> {
    let available = bytes.get(offset..)?;
    let window = &available[..limit.min(available.len())];
    let end = window
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(window.len());
    Some(sanitise(&window[..end]))
}

/// Keeps printable ASCII, replaces anything else with `.`.
fn sanitise(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte)
            } else {
                '.'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_reads_are_bounds_checked() {
        assert_eq!(read_u32(&[1, 2, 3], 0, Endianness::Little), None);
        assert_eq!(read_u16(&[1, 2], usize::MAX, Endianness::Little), None);
        assert_eq!(read_u16(&[1, 2], 0, Endianness::Unknown), None);
        assert_eq!(read_u16(&[1, 2], 0, Endianness::Big), Some(0x0102));
        assert_eq!(
            read_u64(&[1, 0, 0, 0, 0, 0, 0, 0], 0, Endianness::Little),
            Some(1)
        );
    }

    #[test]
    fn slices_never_read_past_the_end() {
        assert_eq!(read_slice(&[1, 2, 3], 1, 2), Some(&[2, 3][..]));
        assert_eq!(read_slice(&[1, 2, 3], 1, 3), None);
        assert_eq!(read_slice(&[1, 2, 3], 0, usize::MAX), None);
    }

    #[test]
    fn padded_names_drop_their_zero_filling() {
        assert_eq!(
            read_padded_name(b".text\0\0\0", 0, 8).as_deref(),
            Some(".text")
        );
        assert_eq!(
            read_padded_name(b"12345678", 0, 8).as_deref(),
            Some("12345678"),
            "a name filling the whole field keeps every character"
        );
        assert_eq!(read_padded_name(b".text", 0, 8), None);
    }

    #[test]
    fn names_never_carry_control_characters() {
        assert_eq!(read_padded_name(b"a\x01b\0", 0, 4).as_deref(), Some("a.b"));
        assert_eq!(read_c_string(b"ok\x07\0", 0, 8).as_deref(), Some("ok."));
    }

    #[test]
    fn c_strings_stop_at_the_terminator_or_the_limit() {
        assert_eq!(read_c_string(b"abc\0def", 0, 16).as_deref(), Some("abc"));
        assert_eq!(read_c_string(b"abcdef", 0, 3).as_deref(), Some("abc"));
        assert_eq!(read_c_string(b"abc", 9, 3), None);
    }
}
