//! Turning a linker's spelling of a name back into the one it was written as.
//!
//! A Rust function reaches the symbol table as
//! `_ZN4core3fmt5write17h0e4b3a1c9f2d8a76E` or
//! `_RNvNtCs1234_4core3fmt5write`, and a table of two thousand of those is a
//! table nobody reads. This reads them back: `core::fmt::write`.
//!
//! Both of Rust's schemes are decoded — the legacy one, which is C++ mangling
//! with a hash bolted on, and `v0`, which is a grammar of its own. What is
//! *not* decoded is left alone: a name using a corner of the grammar this does
//! not understand comes back as `None`, and the caller shows the original.
//! That rule is the whole point. A half-decoded name that looks decoded is
//! worse than a mangled one, because the reader has no way to tell that the
//! part they are relying on was guessed at.
//!
//! Nothing here is lossy in the other direction either: the mangled spelling
//! is what the file holds, and every caller keeps it so a reader can check the
//! reading against it.

/// A readable spelling of `symbol`, or `None` when it is not a mangled name —
/// or is one this does not fully understand.
#[must_use]
pub fn readable(symbol: &str) -> Option<String> {
    // Mach-O prefixes every symbol with an underscore, so `__ZN…` and `__R…`
    // are `_ZN…` and `_R…` with one more of them. Only that one is the
    // platform's; the rest belongs to the name.
    let symbol = if symbol.starts_with("__Z") || symbol.starts_with("__R") {
        &symbol[1..]
    } else {
        symbol
    };
    v0(symbol).or_else(|| legacy(symbol))
}

/// Whether a name is one of the two Rust schemes at all.
///
/// Cheaper than decoding, for a caller counting how much of a symbol table is
/// worth offering to decode.
#[must_use]
pub fn is_mangled(symbol: &str) -> bool {
    let symbol = symbol.strip_prefix('_').unwrap_or(symbol);
    (symbol.starts_with("_R") || symbol.starts_with('R')) && symbol.len() > 3
        || symbol.starts_with("_ZN")
        || symbol.starts_with("ZN")
}

// ---------------------------------------------------------------------------
// The legacy scheme
// ---------------------------------------------------------------------------

/// Rust's first scheme: `_ZN`, the path as `<length><name>` components, a
/// sixteen-hex-digit hash, and `E`.
///
/// The hash is what tells this from a C++ symbol of the same shape, and it is
/// dropped from the reading: it identifies the crate a generic was
/// instantiated in, which is not part of the name anyone wrote.
fn legacy(symbol: &str) -> Option<String> {
    let rest = symbol
        .strip_prefix("_ZN")
        .or_else(|| symbol.strip_prefix("ZN"))?;
    let rest = rest.strip_suffix('E')?;

    let mut components = Vec::new();
    let mut rest = rest;
    while !rest.is_empty() {
        let digits = rest.find(|character: char| !character.is_ascii_digit())?;
        if digits == 0 {
            return None;
        }
        let length: usize = rest[..digits].parse().ok()?;
        let body = rest.get(digits..digits + length)?;
        components.push(body);
        rest = &rest[digits + length..];
    }
    if components.is_empty() {
        return None;
    }
    // The trailing hash is not part of the name.
    if components.last().is_some_and(|last| is_legacy_hash(last)) {
        components.pop();
    }
    if components.is_empty() {
        return None;
    }
    Some(
        components
            .into_iter()
            .map(undo_legacy_escapes)
            .collect::<Vec<String>>()
            .join("::"),
    )
}

/// The `17h` component: `h` followed by sixteen hexadecimal digits.
fn is_legacy_hash(component: &str) -> bool {
    let Some(digits) = component.strip_prefix('h') else {
        return false;
    };
    digits.len() == 16 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Puts back the characters the legacy scheme spells with `$…$`.
///
/// The scheme has no escape of its own for a run that merely looks like one,
/// so an unknown `$…$` is left exactly as it stands rather than guessed at.
fn undo_legacy_escapes(component: &str) -> String {
    const ESCAPES: &[(&str, &str)] = &[
        ("$SP$", "@"),
        ("$BP$", "*"),
        ("$RF$", "&"),
        ("$LT$", "<"),
        ("$GT$", ">"),
        ("$LP$", "("),
        ("$RP$", ")"),
        ("$C$", ","),
        ("$u20$", " "),
        ("$u22$", "\""),
        ("$u27$", "'"),
        ("$u2b$", "+"),
        ("$u3b$", ";"),
        ("$u5b$", "["),
        ("$u5d$", "]"),
        ("$u7b$", "{"),
        ("$u7d$", "}"),
        ("$u7e$", "~"),
    ];

    let mut out = String::with_capacity(component.len());
    // A component that begins with an escape carries a leading underscore the
    // compiler added so the component starts with a letter. It is not part of
    // the name: `_$LT$…` is `<…`.
    let mut rest = component
        .strip_prefix("_$")
        .map_or(component, |_| &component[1..]);
    'outer: while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix("..") {
            // `..` is how the scheme spells the path separator inside one
            // component, and a lone `.` is a hyphen in a crate name.
            out.push_str("::");
            rest = tail;
            continue;
        }
        if rest.starts_with('$') {
            for (escape, character) in ESCAPES {
                if let Some(tail) = rest.strip_prefix(escape) {
                    out.push_str(character);
                    rest = tail;
                    continue 'outer;
                }
            }
        }
        let mut characters = rest.chars();
        let first = characters.next().unwrap_or('\0');
        out.push(if first == '.' { '-' } else { first });
        rest = characters.as_str();
    }
    out
}

// ---------------------------------------------------------------------------
// The `v0` scheme
// ---------------------------------------------------------------------------

/// Rust's `v0` scheme: a grammar rather than a list of components, with back
/// references into the name itself.
///
/// The whole name has to parse. A parser that stopped where it stopped
/// understanding would produce exactly the half-decoded name this module
/// refuses to produce.
fn v0(symbol: &str) -> Option<String> {
    let rest = symbol
        .strip_prefix("_R")
        .or_else(|| symbol.strip_prefix('R'))?;
    let mut parser = Parser {
        input: rest.as_bytes(),
        at: 0,
        depth: 0,
    };
    let mut out = String::new();
    parser.path(&mut out)?;
    // What follows the path is the crate a generic was instantiated in, and a
    // vendor suffix beginning with `.`. Neither is part of the name.
    Some(out)
}

/// How deep the grammar may nest before this gives up.
///
/// A hostile or corrupt name can describe a type inside a type without end,
/// and back references let it point at itself. The limit is far past anything
/// a compiler emits and far short of a blown stack.
const MAXIMUM_DEPTH: usize = 64;

/// Longest name this will build, so a name whose back references multiply
/// cannot be expanded without bound.
const MAXIMUM_OUTPUT: usize = 4096;

struct Parser<'a> {
    input: &'a [u8],
    at: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.at).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.at += 1;
        Some(byte)
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    /// Runs `body` one level deeper, refusing to go past [`MAXIMUM_DEPTH`].
    fn nested<T>(&mut self, body: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        if self.depth >= MAXIMUM_DEPTH {
            return None;
        }
        self.depth += 1;
        let result = body(self);
        self.depth -= 1;
        result
    }

    /// A base-62 number: digits `0-9a-zA-Z` closed by `_`, where a bare `_` is
    /// zero and everything else is its value plus one.
    fn base62(&mut self) -> Option<u64> {
        if self.eat(b'_') {
            return Some(0);
        }
        let mut value: u64 = 0;
        loop {
            let byte = self.next()?;
            if byte == b'_' {
                return value.checked_add(1);
            }
            let digit = match byte {
                b'0'..=b'9' => u64::from(byte - b'0'),
                b'a'..=b'z' => u64::from(byte - b'a') + 10,
                b'A'..=b'Z' => u64::from(byte - b'A') + 36,
                _ => return None,
            };
            value = value.checked_mul(62)?.checked_add(digit)?;
        }
    }

    /// A decimal count, as an identifier's length is written.
    ///
    /// A leading zero *is* the number: the scheme spells zero as `0` and has
    /// no other number starting with one, so `00` is two lengths of zero and
    /// not one length of zero written twice. Reading it greedily swallowed
    /// both — which is how a closure inside a closure, spelled `…00`, came
    /// back undecoded while a closure on its own decoded fine.
    fn decimal(&mut self) -> Option<usize> {
        if self.eat(b'0') {
            return Some(0);
        }
        let start = self.at;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.at += 1;
        }
        if self.at == start {
            return None;
        }
        std::str::from_utf8(&self.input[start..self.at])
            .ok()?
            .parse()
            .ok()
    }

    /// The optional `s<base-62>` that tells two same-named items apart. It is
    /// read and dropped: it disambiguates for the linker, not for a reader.
    fn disambiguator(&mut self) -> Option<()> {
        if self.eat(b's') {
            self.base62()?;
        }
        Some(())
    }

    /// One `<decimal><bytes>` identifier, with the `_` that separates a count
    /// from a body starting with a digit.
    fn identifier(&mut self) -> Option<String> {
        self.disambiguator()?;
        // `u` marks a name that needs Punycode to spell. Decoding it is a
        // second algorithm for a case that reaches a symbol table roughly
        // never, so such a name is left mangled rather than half-read.
        if self.eat(b'u') {
            return None;
        }
        let length = self.decimal()?;
        self.eat(b'_');
        let body = self.input.get(self.at..self.at + length)?;
        self.at += length;
        std::str::from_utf8(body).ok().map(str::to_owned)
    }

    /// Follows a back reference, parsing at the position it names.
    ///
    /// The position is a byte offset into the name with `_R` already taken
    /// off, which is how the scheme counts and how this parser is indexed.
    ///
    /// The reference must point strictly backwards, at something read before
    /// the `B` itself: a name pointing at itself, or forwards, would parse for
    /// ever.
    fn backref<T>(&mut self, body: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        // `B` has already been taken, so the tag itself is one byte back.
        let tag = self.at.checked_sub(1)?;
        let target = usize::try_from(self.base62()?).ok()?;
        if target >= tag {
            return None;
        }
        let mut inner = Parser {
            input: self.input,
            at: target,
            depth: self.depth,
        };
        self.nested(|_| body(&mut inner))
    }

    /// A path: the sequence of names that leads to an item.
    fn path(&mut self, out: &mut String) -> Option<()> {
        if out.len() > MAXIMUM_OUTPUT {
            return None;
        }
        self.nested(|parser| match parser.next()? {
            // A crate root. Its disambiguator is the crate's identity, not
            // part of its name.
            b'C' => {
                let name = parser.identifier()?;
                out.push_str(&name);
                Some(())
            }
            // An item inside something else.
            b'N' => {
                let namespace = parser.next()?;
                parser.path(out)?;
                let name = parser.identifier()?;
                // An item the compiler generated carries no name of its own,
                // and the namespace letter says what it is. Saying `{closure}`
                // is the whole truth about such a component; leaving the row
                // blank would read as a name that failed to decode.
                if name.is_empty() {
                    out.push_str(match namespace {
                        b'C' => "::{closure}",
                        b'S' => "::{shim}",
                        _ => "::{compiler-generated}",
                    });
                } else {
                    out.push_str("::");
                    out.push_str(&name);
                }
                Some(())
            }
            // An inherent impl: `<Type>::…`.
            b'M' => {
                parser.impl_path()?;
                out.push('<');
                parser.type_name(out)?;
                out.push('>');
                Some(())
            }
            // A trait impl: `<Type as Trait>::…`.
            b'X' => {
                parser.impl_path()?;
                out.push('<');
                parser.type_name(out)?;
                out.push_str(" as ");
                parser.path(out)?;
                out.push('>');
                Some(())
            }
            // A trait definition.
            b'Y' => {
                out.push('<');
                parser.type_name(out)?;
                out.push_str(" as ");
                parser.path(out)?;
                out.push('>');
                Some(())
            }
            // A generic instance. The arguments are read so the rest of the
            // name parses, and shown: `Vec<u8>` and `Vec<String>` are two
            // functions, and a listing that spells both `Vec` is a listing
            // with two identical rows in it.
            b'I' => {
                parser.path(out)?;
                out.push_str("::<");
                let mut first = true;
                while !parser.eat(b'E') {
                    if !first {
                        out.push_str(", ");
                    }
                    first = false;
                    parser.generic_argument(out)?;
                }
                out.push('>');
                Some(())
            }
            b'B' => parser.backref(|inner| {
                let mut nested = String::new();
                inner.path(&mut nested)?;
                out.push_str(&nested);
                Some(())
            }),
            _ => None,
        })
    }

    /// The path in front of an impl, which names the crate the impl was
    /// written in. Read and dropped: it is not part of what anyone calls the
    /// function.
    fn impl_path(&mut self) -> Option<()> {
        self.disambiguator()?;
        let mut discarded = String::new();
        self.path(&mut discarded)
    }

    fn generic_argument(&mut self, out: &mut String) -> Option<()> {
        match self.peek()? {
            // A lifetime, which has no name a reader would recognise.
            b'L' => {
                self.at += 1;
                self.base62()?;
                out.push_str("'_");
                Some(())
            }
            // A const argument: the value an array length or a `const N` is.
            b'K' => {
                self.at += 1;
                self.constant(out)
            }
            _ => self.type_name(out),
        }
    }

    /// A type, spelled the way it would be written in source.
    fn type_name(&mut self, out: &mut String) -> Option<()> {
        if out.len() > MAXIMUM_OUTPUT {
            return None;
        }
        self.nested(|parser| {
            if let Some(basic) = parser.peek().and_then(basic_type) {
                parser.at += 1;
                out.push_str(basic);
                return Some(());
            }
            match parser.peek()? {
                b'A' => {
                    parser.at += 1;
                    out.push('[');
                    parser.type_name(out)?;
                    out.push_str("; ");
                    parser.constant(out)?;
                    out.push(']');
                    Some(())
                }
                b'S' => {
                    parser.at += 1;
                    out.push('[');
                    parser.type_name(out)?;
                    out.push(']');
                    Some(())
                }
                b'T' => {
                    parser.at += 1;
                    out.push('(');
                    let mut first = true;
                    while !parser.eat(b'E') {
                        if !first {
                            out.push_str(", ");
                        }
                        first = false;
                        parser.type_name(out)?;
                    }
                    out.push(')');
                    Some(())
                }
                reference @ (b'R' | b'Q') => {
                    parser.at += 1;
                    out.push('&');
                    if parser.eat(b'L') {
                        parser.base62()?;
                    }
                    if reference == b'Q' {
                        out.push_str("mut ");
                    }
                    parser.type_name(out)
                }
                pointer @ (b'P' | b'O') => {
                    parser.at += 1;
                    out.push_str(if pointer == b'P' { "*const " } else { "*mut " });
                    parser.type_name(out)
                }
                b'D' => {
                    parser.at += 1;
                    out.push_str("dyn ");
                    parser.dyn_bounds(out)?;
                    // The trailing lifetime of a trait object.
                    if parser.eat(b'L') {
                        parser.base62()?;
                    }
                    Some(())
                }
                // A function pointer, spelled as its signature.
                //
                // Not a rare corner: `Box<dyn FnOnce() -> T>` carries one, and
                // so does every closure passed by pointer, which is a sixth of
                // the encoded names in an ordinary Rust binary. Leaving it
                // undecoded left all of those mangled.
                b'F' => {
                    parser.at += 1;
                    parser.function_signature(out)
                }
                b'B' => {
                    // The tag itself has to be taken before the number is
                    // read. Left in place, it was read as the first digit of
                    // the offset — `BQ_` came out as 2347 instead of 53 — and
                    // every type reached through a back reference failed as a
                    // reference pointing forwards.
                    parser.at += 1;
                    parser.backref(|inner| {
                        let mut nested = String::new();
                        inner.type_name(&mut nested)?;
                        out.push_str(&nested);
                        Some(())
                    })
                }
                _ => parser.path(out),
            }
        })
    }

    /// The signature of a function pointer: `fn(u8, u8) -> bool`, with the
    /// `unsafe` and the `extern "C"` in front of it when the name says so.
    ///
    /// The `F` has already been taken.
    fn function_signature(&mut self, out: &mut String) -> Option<()> {
        // An optional binder, for a signature with lifetimes of its own.
        if self.eat(b'G') {
            self.base62()?;
        }
        if self.eat(b'U') {
            out.push_str("unsafe ");
        }
        if self.eat(b'K') {
            // `C` is the one ABI with a shorthand; the rest are spelled out,
            // with `_` standing for the `-` a name like `system-unwind` has.
            let abi = if self.eat(b'C') {
                "C".to_owned()
            } else {
                self.identifier()?.replace('_', "-")
            };
            out.push_str("extern \"");
            out.push_str(&abi);
            out.push_str("\" ");
        }
        out.push_str("fn(");
        let mut first = true;
        while !self.eat(b'E') {
            if !first {
                out.push_str(", ");
            }
            first = false;
            self.type_name(out)?;
        }
        out.push(')');
        // The return type. `()` is written by leaving it off, the way source
        // does, rather than as `-> ()`.
        let mut returned = String::new();
        self.type_name(&mut returned)?;
        if returned != "()" {
            out.push_str(" -> ");
            out.push_str(&returned);
        }
        Some(())
    }

    /// The traits of a `dyn Trait` type, and the associated types bound in it.
    fn dyn_bounds(&mut self, out: &mut String) -> Option<()> {
        // An optional binder, for a bound with its own lifetimes.
        if self.eat(b'G') {
            self.base62()?;
        }
        let mut first = true;
        while !self.eat(b'E') {
            if !first {
                out.push_str(" + ");
            }
            first = false;
            self.path(out)?;
            // Associated-type bindings: `p<identifier><type>`.
            while self.eat(b'p') {
                let name = self.identifier()?;
                out.push_str(", ");
                out.push_str(&name);
                out.push_str(" = ");
                self.type_name(out)?;
            }
        }
        Some(())
    }

    /// A const generic argument: its type, then its value.
    fn constant(&mut self, out: &mut String) -> Option<()> {
        self.nested(|parser| {
            if parser.eat(b'B') {
                return parser.backref(|inner| {
                    let mut nested = String::new();
                    inner.constant(&mut nested)?;
                    out.push_str(&nested);
                    Some(())
                });
            }
            let kind = parser.next()?;
            match kind {
                // A placeholder, where the value is not part of the name.
                b'p' => {
                    out.push('_');
                    Some(())
                }
                b'b' => {
                    let value = parser.constant_digits()?;
                    out.push_str(if value == "0" { "false" } else { "true" });
                    Some(())
                }
                b'c' => {
                    let value = parser.constant_digits()?;
                    out.push('\'');
                    out.push_str("\\u{");
                    out.push_str(&value);
                    out.push_str("}'");
                    Some(())
                }
                _ if basic_type(kind).is_some() => {
                    // A negative value is written with a leading `n`.
                    let negative = parser.eat(b'n');
                    let digits = parser.constant_digits()?;
                    let value = u128::from_str_radix(&digits, 16).ok()?;
                    if negative {
                        out.push('-');
                    }
                    out.push_str(&value.to_string());
                    Some(())
                }
                _ => None,
            }
        })
    }

    /// The hexadecimal body of a constant, closed by `_`.
    fn constant_digits(&mut self) -> Option<String> {
        let start = self.at;
        while self.peek().is_some_and(|byte| byte.is_ascii_hexdigit()) {
            self.at += 1;
        }
        let digits = std::str::from_utf8(&self.input[start..self.at]).ok()?;
        if !self.eat(b'_') || digits.is_empty() {
            return None;
        }
        Some(digits.to_owned())
    }
}

/// The one-letter spelling of each primitive type.
const fn basic_type(byte: u8) -> Option<&'static str> {
    Some(match byte {
        b'a' => "i8",
        b'b' => "bool",
        b'c' => "char",
        b'd' => "f64",
        b'e' => "str",
        b'f' => "f32",
        b'h' => "u8",
        b'i' => "isize",
        b'j' => "usize",
        b'l' => "i32",
        b'm' => "u32",
        b'n' => "i128",
        b'o' => "u128",
        b's' => "i16",
        b't' => "u16",
        b'u' => "()",
        b'v' => "...",
        b'x' => "i64",
        b'y' => "u64",
        b'z' => "!",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_legacy_path_reads_back_without_its_hash() {
        assert_eq!(
            readable("_ZN4core3fmt5write17h0e4b3a1c9f2d8a76E").as_deref(),
            Some("core::fmt::write")
        );
    }

    /// Mach-O writes an extra underscore in front of every symbol.
    #[test]
    fn the_platform_s_leading_underscore_is_not_part_of_the_name() {
        assert_eq!(
            readable("__ZN3std2io5Write9write_all17h1111111111111111E").as_deref(),
            Some("std::io::Write::write_all")
        );
    }

    /// The legacy scheme spells the characters a linker will not take as
    /// `$…$`, and a path separator inside one component as `..`.
    #[test]
    fn the_legacy_escapes_are_put_back() {
        assert_eq!(
            readable("_ZN59_$LT$core..option..Option$LT$T$GT$$u20$as$u20$core..fmt$GT$3fmt17h0000000000000000E")
                .as_deref(),
            Some("<core::option::Option<T> as core::fmt>::fmt")
        );
    }

    /// A crate spelled with a hyphen reaches the symbol table with a dot.
    #[test]
    fn a_dot_in_a_component_is_a_hyphen() {
        assert_eq!(
            readable("_ZN10serde.json5parse17h0000000000000000E").as_deref(),
            Some("serde-json::parse")
        );
    }

    /// A C++ symbol has the same shape and no hash. Reading it as a Rust path
    /// would be wrong about the language, but the components are the same
    /// sequence — so what matters is that nothing is invented: the last
    /// component stays, because it is not a hash.
    #[test]
    fn a_name_without_a_hash_keeps_all_of_its_components() {
        assert_eq!(
            readable("_ZN3foo3barE").as_deref(),
            Some("foo::bar"),
            "no hash to drop means no component dropped"
        );
    }

    #[test]
    fn a_v0_path_reads_back() {
        assert_eq!(
            readable("_RNvNtCs1234_4core3fmt5write").as_deref(),
            Some("core::fmt::write")
        );
    }

    #[test]
    fn a_v0_generic_instance_keeps_its_arguments() {
        // `alloc::vec::Vec::<u8>::new`
        let name = readable("_RINvNtNtCs1234_5alloc3vec3Vec3newhE");
        assert_eq!(name.as_deref(), Some("alloc::vec::Vec::new::<u8>"));
    }

    #[test]
    fn a_v0_trait_impl_reads_as_the_source_spelled_it() {
        // `<u8 as core::fmt::Debug>::fmt`
        let name = readable("_RNvXCs1234_4testhNtNtCs1234_4core3fmt5Debug3fmt");
        assert_eq!(name.as_deref(), Some("<u8 as core::fmt::Debug>::fmt"));
    }

    #[test]
    fn v0_reference_and_slice_types_are_spelled_out() {
        // `<&[u8] as test::Trait>::run`
        let name = readable("_RNvXCs1234_4testRShNtCs1234_4test5Trait3run");
        assert_eq!(name.as_deref(), Some("<&[u8] as test::Trait>::run"));
    }

    /// A back reference points at a name already read. One that points at
    /// itself, or forwards, would loop for ever.
    #[test]
    fn a_back_reference_that_does_not_go_backwards_is_refused() {
        // `B_` at the very start points at offset 0, which is the `B` itself.
        assert_eq!(readable("_RB_"), None);
        // And one that reaches back into a construct still being read is
        // stopped by the depth limit rather than by the stack.
        assert_eq!(readable("_RNvB_4test"), None);
    }

    /// The reference every real Rust binary is full of: a path read once and
    /// pointed at afterwards. Getting the offset wrong by the two bytes of the
    /// `_R` prefix leaves every such name undecoded, which is most of them.
    #[test]
    fn a_back_reference_names_the_path_it_points_at() {
        // `<alloc::raw_vec::RawVecInner>::finish_grow`, with `B5_` pointing at
        // the `alloc::raw_vec` path read a moment earlier.
        let name = readable("_RNvMs5_NtCs1234_5alloc7raw_vecNtB5_11RawVecInner11finish_grow");

        assert_eq!(
            name.as_deref(),
            Some("<alloc::raw_vec::RawVecInner>::finish_grow")
        );
    }

    /// A name using a corner of the grammar this does not decode comes back
    /// whole rather than half-read.
    #[test]
    fn what_is_not_understood_is_left_alone() {
        // An identifier that needs Punycode to spell. Decoding it is a second
        // algorithm for a case that reaches a symbol table roughly never, so
        // the name comes back whole rather than half-read.
        assert_eq!(readable("_RNvNtCs1234_4core3fmtu8fooXYZab"), None);
        assert_eq!(readable("not a mangled name at all"), None);
        assert_eq!(readable("main"), None);
    }

    /// A function pointer is not a corner of the grammar: `Box<dyn FnOnce()>`
    /// carries one, and so does every closure passed by pointer.
    #[test]
    fn a_function_pointer_type_is_spelled_as_a_signature() {
        // `<fn(u8) -> bool as test::Trait>::run`
        let name = readable("_RNvXCs1234_4testFhEbNtCs1234_4test5Trait3run");

        assert_eq!(
            name.as_deref(),
            Some("<fn(u8) -> bool as test::Trait>::run")
        );
    }

    #[test]
    fn nothing_panics_and_nothing_loops_on_any_input() {
        let candidates = [
            "_R",
            "_RB",
            "_RN",
            "_ZN",
            "_ZNE",
            "_ZN999999999999999999999aE",
            "_RINvNtCs_",
            "_R\u{0}\u{0}\u{0}",
            "_ZN4core",
        ];
        for candidate in candidates {
            let _ = readable(candidate);
        }
        // A type nested inside itself four thousand deep: the depth limit is
        // what stops this, not the stack.
        let deep = format!("_RNvXCs1_1t{}h", "R".repeat(4096));
        assert_eq!(readable(&deep), None);
    }

    /// The end-to-end check, on the one Rust binary certainly present wherever
    /// these tests run: their own executable.
    ///
    /// A grammar this size fails quietly — one rule wrong and a whole family
    /// of names comes back `None`, which looks exactly like "not a mangled
    /// name" and shows up as a table still full of `_RNvMs5_…`. That is how
    /// the back-reference offset was wrong by the two bytes of the `_R`
    /// prefix: every unit test passed, and nearly nothing in a real binary
    /// decoded. A share is the only assertion that catches it.
    #[test]
    fn most_of_a_real_binary_s_encoded_names_are_read_back() {
        let path = std::env::current_exe().expect("the test binary has a path");
        let analysis = crate::analyse_path(&path).expect("analysable");

        let encoded: Vec<&str> = analysis
            .symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .filter(|name| is_mangled(name))
            .collect();
        if encoded.len() < 100 {
            // A stripped test binary — some platforms build one — has nothing
            // to measure, and an assertion over four names says nothing.
            return;
        }
        let read = encoded
            .iter()
            .filter(|name| readable(name).is_some())
            .count();
        #[expect(
            clippy::cast_precision_loss,
            reason = "two counts of names in one symbol table"
        )]
        let share = read as f64 / encoded.len() as f64;
        assert!(
            share > 0.95,
            "only {read} of {} encoded names read back ({:.0}%)",
            encoded.len(),
            share * 100.0
        );
    }

    #[test]
    fn a_name_that_is_not_mangled_is_recognised_as_such() {
        assert!(is_mangled("_ZN4core3fmt5writeE"));
        assert!(is_mangled("_RNvNtCs1234_4core3fmt5write"));
        assert!(!is_mangled("main"));
        assert!(!is_mangled("_start"));
    }
}
