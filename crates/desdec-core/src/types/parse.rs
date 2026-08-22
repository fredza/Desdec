//! Reading type definitions written in C.
//!
//! The reader already has the declarations: they are in the headers of the
//! program, or in the documentation of the library it calls, and they are
//! written in C. Asking them to re-enter that through a form, one member at a
//! time, is asking them to translate something they can already paste.
//!
//! So this reads C — the declaration subset of it. Structures, unions,
//! enumerations and `typedef`s; pointers, arrays and bit-fields; comments and
//! `#` lines dropped, so a header can be pasted in as it stands and the parts
//! that are not declarations do not stop it.
//!
//! What it deliberately does not do is compute. `char name[MAX];` is refused
//! by name rather than guessed at, because a structure laid out from a guessed
//! array length would place every member after it wrongly, and quietly.

use std::collections::HashMap;

use super::{Constant, Definition, Member, Primitive, Type};

/// Why a definition could not be read, and where.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseError {
    /// What was wrong, in the reader's terms.
    pub message: String,
    /// The line it was on, counting from one.
    pub line: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Reads every definition in `source`.
///
/// Definitions may name each other in any order; nothing is laid out here, so
/// a name that is never defined is only found when the type is used.
///
/// # Errors
///
/// On the first thing that is not a declaration this understands.
pub fn definitions(source: &str) -> Result<Vec<Definition>, ParseError> {
    let tokens = lex(source)?;
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
        aliases: HashMap::new(),
    };
    parser.unit()
}

/// One piece of C source.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Token {
    Word(String),
    Number(i64),
    Symbol(char),
}

#[derive(Clone, Debug)]
struct Spanned {
    token: Token,
    line: usize,
}

/// Cuts the source into words, numbers and punctuation.
///
/// Comments and preprocessor lines are dropped here rather than parsed, which
/// is what lets a header be pasted in whole.
fn lex(source: &str) -> Result<Vec<Spanned>, ParseError> {
    let mut tokens = Vec::new();
    let bytes: Vec<char> = source.chars().collect();
    let mut at = 0;
    let mut line = 1;

    while at < bytes.len() {
        let character = bytes[at];
        match character {
            '\n' => {
                line += 1;
                at += 1;
            }
            character if character.is_whitespace() => at += 1,
            '#' => {
                // A preprocessor line. Skipped whole, continuation lines and
                // all, so `#include` and `#ifdef` do not stop a pasted header.
                while at < bytes.len() && bytes[at] != '\n' {
                    let escaped = bytes[at] == '\\';
                    at += 1;
                    if escaped && at < bytes.len() && bytes[at] == '\n' {
                        line += 1;
                        at += 1;
                    }
                }
            }
            '/' if bytes.get(at + 1) == Some(&'/') => {
                while at < bytes.len() && bytes[at] != '\n' {
                    at += 1;
                }
            }
            '/' if bytes.get(at + 1) == Some(&'*') => {
                at += 2;
                loop {
                    match bytes.get(at) {
                        None => {
                            return Err(ParseError {
                                message: "a comment was opened and never closed".to_owned(),
                                line,
                            });
                        }
                        Some('\n') => {
                            line += 1;
                            at += 1;
                        }
                        Some('*') if bytes.get(at + 1) == Some(&'/') => {
                            at += 2;
                            break;
                        }
                        Some(_) => at += 1,
                    }
                }
            }
            '\'' => {
                // A character constant, which is how an enumeration often
                // names its tags: `Png = 'P'`.
                let (value, width) = character_constant(&bytes[at..], line)?;
                tokens.push(Spanned {
                    token: Token::Number(value),
                    line,
                });
                at += width;
            }
            character if character.is_ascii_digit() => {
                let (value, width) = number(&bytes[at..], line)?;
                tokens.push(Spanned {
                    token: Token::Number(value),
                    line,
                });
                at += width;
            }
            character if character.is_alphabetic() || character == '_' => {
                let start = at;
                while at < bytes.len() && (bytes[at].is_alphanumeric() || bytes[at] == '_') {
                    at += 1;
                }
                tokens.push(Spanned {
                    token: Token::Word(bytes[start..at].iter().collect()),
                    line,
                });
            }
            symbol => {
                tokens.push(Spanned {
                    token: Token::Symbol(symbol),
                    line,
                });
                at += 1;
            }
        }
    }
    Ok(tokens)
}

/// Reads a decimal, hexadecimal, octal or binary literal, and its width in
/// characters. Integer suffixes (`u`, `L`, `ULL`) are read and dropped.
fn number(input: &[char], line: usize) -> Result<(i64, usize), ParseError> {
    let (radix, start) = match (input.first(), input.get(1)) {
        (Some('0'), Some('x' | 'X')) => (16, 2),
        (Some('0'), Some('b' | 'B')) => (2, 2),
        (Some('0'), Some(digit)) if digit.is_ascii_digit() => (8, 1),
        _ => (10, 0),
    };
    let mut at = start;
    let mut digits = String::new();
    while let Some(character) = input.get(at) {
        if character.is_digit(radix) {
            digits.push(*character);
            at += 1;
        } else if *character == '\'' {
            // A digit separator, as C++14 and Rust both write it.
            at += 1;
        } else {
            break;
        }
    }
    while let Some(character) = input.get(at) {
        if matches!(character, 'u' | 'U' | 'l' | 'L') {
            at += 1;
        } else {
            break;
        }
    }
    if digits.is_empty() {
        // `0` on its own, whose leading zero was taken for an octal marker.
        return Ok((0, at.max(1)));
    }
    i64::from_str_radix(&digits, radix)
        .map(|value| (value, at))
        .map_err(|_| ParseError {
            message: format!("{digits} is too large to be a value here"),
            line,
        })
}

/// Reads a character constant and its width in characters.
fn character_constant(input: &[char], line: usize) -> Result<(i64, usize), ParseError> {
    let refuse = |message: &str| ParseError {
        message: message.to_owned(),
        line,
    };
    let (value, width) = match (input.get(1), input.get(2)) {
        (Some('\\'), Some(escaped)) => {
            let value = match escaped {
                'n' => 10,
                't' => 9,
                'r' => 13,
                '0' => 0,
                '\\' => 92,
                '\'' => 39,
                other => i64::from(u32::from(*other)),
            };
            (value, 4)
        }
        (Some(character), _) => (i64::from(u32::from(*character)), 3),
        (None, _) => return Err(refuse("a character constant was opened and never closed")),
    };
    if input.get(width - 1) != Some(&'\'') {
        return Err(refuse("a character constant holds one character"));
    }
    Ok((value, width))
}

struct Parser<'a> {
    tokens: &'a [Spanned],
    at: usize,
    /// What each `typedef` name stands for, so later declarations can use it.
    aliases: HashMap<String, Type>,
}

impl Parser<'_> {
    fn unit(&mut self) -> Result<Vec<Definition>, ParseError> {
        let mut definitions = Vec::new();
        while self.at < self.tokens.len() {
            // A stray `;` between declarations is allowed and means nothing.
            if self.take_symbol(';') {
                continue;
            }
            if let Some(definition) = self.definition()? {
                definitions.push(definition);
            }
        }
        Ok(definitions)
    }

    /// One top-level declaration. `None` for a `typedef`, which defines a name
    /// rather than a type of its own.
    fn definition(&mut self) -> Result<Option<Definition>, ParseError> {
        if self.take_word("typedef") {
            self.type_definition()?;
            return Ok(None);
        }
        let line = self.line();
        let keyword = match self.peek_word() {
            Some(word @ ("struct" | "union" | "enum")) => word.to_owned(),
            _ => {
                return Err(refuse(
                    "a definition starts with struct, union, enum or typedef",
                    line,
                ));
            }
        };
        self.at += 1;
        let name = self.name("a name for what is being defined")?;
        if keyword == "enum" {
            return self.enumeration(name).map(Some);
        }
        self.expect_symbol('{')?;
        let members = self.members()?;
        self.expect_symbol('}')?;
        self.expect_symbol(';')?;
        Ok(Some(if keyword == "struct" {
            Definition::Struct { name, members }
        } else {
            Definition::Union { name, members }
        }))
    }

    /// `typedef <type> <name>;`, which adds a name for a type that already
    /// exists rather than a definition of its own.
    fn type_definition(&mut self) -> Result<(), ParseError> {
        let base = self.specifier()?;
        let (name, kind) = self.declarator(base)?;
        self.expect_symbol(';')?;
        self.aliases.insert(name, kind);
        Ok(())
    }

    fn enumeration(&mut self, name: String) -> Result<Definition, ParseError> {
        // `enum Colour : unsigned char { ... }`, which C++ and every compiler
        // worth reading accept, and which decides how wide the value is.
        let base = if self.take_symbol(':') {
            match self.specifier()? {
                Type::Primitive(primitive) => primitive,
                other => {
                    let line = self.line();
                    return Err(refuse(
                        &format!(
                            "an enumeration is stored as an integer, not as {}",
                            other.label()
                        ),
                        line,
                    ));
                }
            }
        } else {
            Primitive::Int
        };
        self.expect_symbol('{')?;
        let mut constants = Vec::new();
        let mut next = 0i64;
        while !self.take_symbol('}') {
            let constant = self.name("a name for the constant")?;
            let value = if self.take_symbol('=') {
                self.number()?
            } else {
                next
            };
            next = value.saturating_add(1);
            constants.push(Constant {
                name: constant,
                value,
            });
            if !self.take_symbol(',') {
                self.expect_symbol('}')?;
                break;
            }
        }
        self.expect_symbol(';')?;
        Ok(Definition::Enumeration {
            name,
            base,
            constants,
        })
    }

    fn members(&mut self) -> Result<Vec<Member>, ParseError> {
        let mut members = Vec::new();
        while !matches!(self.peek(), Some(Token::Symbol('}')) | None) {
            let base = self.specifier()?;
            loop {
                // `unsigned int : 0;` — a bit-field with no name, which is
                // padding written out by hand rather than a member.
                let (name, kind) = if self.peek() == Some(&Token::Symbol(':')) {
                    (String::new(), base.clone())
                } else {
                    self.declarator(base.clone())?
                };
                let bits = if self.take_symbol(':') {
                    let width = self.number()?;
                    Some(u32::try_from(width).map_err(|_| ParseError {
                        message: format!("{width} is not a width in bits"),
                        line: self.line(),
                    })?)
                } else {
                    None
                };
                members.push(Member { name, kind, bits });
                if !self.take_symbol(',') {
                    break;
                }
            }
            self.expect_symbol(';')?;
        }
        Ok(members)
    }

    /// The type a declaration starts with, before any stars or brackets.
    fn specifier(&mut self) -> Result<Type, ParseError> {
        let line = self.line();
        // `struct Node *next;` — the tag says which namespace, and Desdec
        // keeps one, so the tag is read and the name is what matters.
        if matches!(self.peek_word(), Some("struct" | "union" | "enum")) {
            self.at += 1;
            let name = self.name("a name after struct, union or enum")?;
            return Ok(Type::Named(name));
        }

        let mut words: Vec<String> = Vec::new();
        while let Some(word) = self.peek_word() {
            // Qualifiers say nothing about the layout, so they are read and
            // dropped rather than refused.
            if matches!(word, "const" | "volatile" | "restrict" | "_Atomic") {
                self.at += 1;
                continue;
            }
            let names_a_type =
                is_type_word(word) || (words.is_empty() && self.aliases.contains_key(word));
            if !names_a_type {
                break;
            }
            words.push(word.to_owned());
            self.at += 1;
            // A name the reader gave a type stands alone: `Node *next;`.
            if words.len() == 1 && self.aliases.contains_key(&words[0]) {
                break;
            }
        }
        if words.is_empty() {
            let seen = self.describe_here();
            return Err(refuse(
                &format!("a type was expected, and {seen} was written"),
                line,
            ));
        }
        if let Some(alias) = self.aliases.get(&words[0]) {
            if words.len() == 1 {
                return Ok(alias.clone());
            }
        }
        combine(&words).ok_or_else(|| {
            let written = words.join(" ");
            ParseError {
                message: format!("{written} is not a type this reads"),
                line,
            }
        })
    }

    /// The stars, the name and the brackets: `*name[4]`.
    fn declarator(&mut self, base: Type) -> Result<(String, Type), ParseError> {
        let mut kind = base;
        while self.take_symbol('*') {
            // A qualifier on the pointer itself: `char *const p`.
            while matches!(self.peek_word(), Some("const" | "volatile" | "restrict")) {
                self.at += 1;
            }
            kind = kind.pointer_to();
        }
        let name = self.name("a name for the member")?;
        // Brackets are read left to right and applied right to left, so
        // `a[2][3]` is two arrays of three, as C says it is.
        let mut counts = Vec::new();
        while self.take_symbol('[') {
            let count = self.number()?;
            let count = u64::try_from(count).map_err(|_| ParseError {
                message: format!("{count} is not a number of elements"),
                line: self.line(),
            })?;
            self.expect_symbol(']')?;
            counts.push(count);
        }
        for count in counts.into_iter().rev() {
            kind = Type::Array(Box::new(kind), count);
        }
        Ok((name, kind))
    }

    fn number(&mut self) -> Result<i64, ParseError> {
        let line = self.line();
        let negative = self.take_symbol('-');
        if !negative {
            // `+3`, which nobody writes and every parser should still read.
            let _ = self.take_symbol('+');
        }
        if let Some(Token::Number(value)) = self.peek() {
            let value = *value;
            self.at += 1;
            return Ok(if negative { -value } else { value });
        }
        let seen = self.describe_here();
        Err(refuse(
            &format!("a number was expected, and {seen} was written"),
            line,
        ))
    }

    fn name(&mut self, expected: &str) -> Result<String, ParseError> {
        let line = self.line();
        if let Some(Token::Word(word)) = self.peek() {
            let word = word.clone();
            self.at += 1;
            return Ok(word);
        }
        let seen = self.describe_here();
        Err(refuse(
            &format!("{expected} was expected, and {seen} was written"),
            line,
        ))
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at).map(|spanned| &spanned.token)
    }

    fn peek_word(&self) -> Option<&str> {
        match self.peek() {
            Some(Token::Word(word)) => Some(word),
            _ => None,
        }
    }

    fn take_word(&mut self, word: &str) -> bool {
        if self.peek_word() == Some(word) {
            self.at += 1;
            return true;
        }
        false
    }

    fn take_symbol(&mut self, symbol: char) -> bool {
        if self.peek() == Some(&Token::Symbol(symbol)) {
            self.at += 1;
            return true;
        }
        false
    }

    fn expect_symbol(&mut self, symbol: char) -> Result<(), ParseError> {
        let line = self.line();
        if self.take_symbol(symbol) {
            return Ok(());
        }
        let seen = self.describe_here();
        Err(refuse(
            &format!("{symbol} was expected, and {seen} was written"),
            line,
        ))
    }

    /// The line the parser is on, or the last line of the source once it has
    /// run off the end.
    fn line(&self) -> usize {
        self.tokens
            .get(self.at)
            .or_else(|| self.tokens.last())
            .map_or(1, |spanned| spanned.line)
    }

    /// What is written where the parser is, for a message that quotes it back.
    fn describe_here(&self) -> String {
        match self.peek() {
            None => "the definition ended".to_owned(),
            Some(Token::Word(word)) => format!("`{word}`"),
            Some(Token::Number(value)) => format!("`{value}`"),
            Some(Token::Symbol(symbol)) => format!("`{symbol}`"),
        }
    }
}

/// The refusal, spelled once.
fn refuse(message: &str, line: usize) -> ParseError {
    ParseError {
        message: message.to_owned(),
        line,
    }
}

/// Whether a word can take part in a built-in type.
fn is_type_word(word: &str) -> bool {
    matches!(
        word,
        "void"
            | "char"
            | "short"
            | "int"
            | "long"
            | "signed"
            | "unsigned"
            | "float"
            | "double"
            | "bool"
            | "_Bool"
    ) || fixed_width(word).is_some()
}

/// The fixed-width and platform spellings, resolved to the C type of the same
/// width.
///
/// The Windows ones are here because x64dbg is a Windows debugger and its
/// users' headers are full of them: a reader pasting a `WinAPI` structure
/// should not have to translate `DWORD` first.
fn fixed_width(word: &str) -> Option<Primitive> {
    Some(match word {
        "int8_t" | "CHAR" => Primitive::SignedChar,
        "uint8_t" | "BYTE" | "UCHAR" | "UINT8" | "BOOLEAN" => Primitive::UnsignedChar,
        "int16_t" | "SHORT" => Primitive::Short,
        "uint16_t" | "WORD" | "USHORT" | "WCHAR" => Primitive::UnsignedShort,
        // `BOOL` is an `int` in the Windows headers, not a one-byte boolean:
        // a structure laid out with it as one byte has every member after it
        // in the wrong place.
        "int32_t" | "INT" | "LONG32" | "BOOL" => Primitive::Int,
        "uint32_t" | "DWORD" | "UINT" | "ULONG32" => Primitive::UnsignedInt,
        "int64_t" | "LONGLONG" | "INT64" => Primitive::LongLong,
        "uint64_t" | "QWORD" | "ULONGLONG" | "UINT64" | "DWORD64" => Primitive::UnsignedLongLong,
        "size_t" | "uintptr_t" | "ULONG_PTR" | "SIZE_T" | "DWORD_PTR" => {
            Primitive::UnsignedPointerSized
        }
        "intptr_t" | "ptrdiff_t" | "ssize_t" | "LONG_PTR" => Primitive::SignedPointerSized,
        _ => return None,
    })
}

/// Turns the words of a built-in type into the type itself.
///
/// `unsigned`, `long long int` and `signed char` are all several words that
/// name one type; the order they are written in does not matter to C, and does
/// not matter here.
fn combine(words: &[String]) -> Option<Type> {
    if words.len() == 1 {
        if let Some(primitive) = fixed_width(&words[0]) {
            return Some(Type::Primitive(primitive));
        }
    }

    let mut signed = None;
    let mut longs = 0;
    let mut shorts = 0;
    let mut base = None;
    for word in words {
        match word.as_str() {
            "signed" => signed = Some(true),
            "unsigned" => signed = Some(false),
            "long" => longs += 1,
            "short" => shorts += 1,
            "void" | "char" | "int" | "float" | "double" | "bool" | "_Bool" => {
                if base.is_some() {
                    return None;
                }
                base = Some(word.as_str());
            }
            // A fixed-width name mixed in with keywords, as in `unsigned
            // DWORD`, which is not C.
            _ => return None,
        }
    }
    if longs > 2 || shorts > 1 || (longs > 0 && shorts > 0) {
        return None;
    }

    let unsigned = signed == Some(false);
    Some(Type::Primitive(match base {
        Some("void") => Primitive::Void,
        Some("float") => Primitive::Float,
        // `long double` is eighty bits padded to sixteen bytes on System V,
        // eight bytes on Windows, and eight again on ARM64 macOS. Reading it
        // as one of those would lay out every member after it wrongly on the
        // other two, so it is refused by name instead.
        Some("double") if longs == 0 => Primitive::Double,
        Some("bool" | "_Bool") => Primitive::Bool,
        Some("char") => match signed {
            None => Primitive::Char,
            Some(true) => Primitive::SignedChar,
            Some(false) => Primitive::UnsignedChar,
        },
        // `int`, or nothing at all: `unsigned` on its own is `unsigned int`.
        Some("int") | None => match (shorts, longs, unsigned) {
            (1, _, false) => Primitive::Short,
            (1, _, true) => Primitive::UnsignedShort,
            (_, 1, false) => Primitive::Long,
            (_, 1, true) => Primitive::UnsignedLong,
            (_, 2, false) => Primitive::LongLong,
            (_, 2, true) => Primitive::UnsignedLongLong,
            (_, _, false) => {
                if signed.is_none() && base.is_none() {
                    // No keyword at all is not a type.
                    return None;
                }
                Primitive::Int
            }
            (_, _, true) => Primitive::UnsignedInt,
        },
        // `long double`, whose width is one thing on System V, another on
        // Windows and another on ARM64 macOS, and anything else that got this
        // far without being a type.
        _ => return None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(source: &str) -> Vec<Definition> {
        definitions(source).expect("the definitions read")
    }

    fn members(source: &str) -> Vec<Member> {
        read(source)
            .first()
            .expect("one definition")
            .members()
            .to_vec()
    }

    fn refusal(source: &str) -> ParseError {
        definitions(source).expect_err("the definition is refused")
    }

    #[test]
    fn a_declaration_is_read_as_c_writes_it() {
        let members = members("struct S { unsigned int count; char *name; int grid[2][3]; };");
        assert_eq!(members[0].kind, Type::primitive(Primitive::UnsignedInt));
        assert_eq!(
            members[1].kind,
            Type::primitive(Primitive::Char).pointer_to()
        );
        assert_eq!(
            members[2].kind,
            Type::Array(
                Box::new(Type::Array(Box::new(Type::primitive(Primitive::Int)), 3)),
                2
            ),
            "two of three, which is what C means by [2][3]"
        );
    }

    #[test]
    fn several_members_may_share_one_type() {
        let members = members("struct S { int a, *b, c[4]; };");
        assert_eq!(members.len(), 3);
        assert_eq!(members[0].kind, Type::primitive(Primitive::Int));
        assert_eq!(
            members[1].kind,
            Type::primitive(Primitive::Int).pointer_to()
        );
        assert_eq!(
            members[2].kind,
            Type::Array(Box::new(Type::primitive(Primitive::Int)), 4)
        );
    }

    /// The order of the keywords does not matter to C and must not matter
    /// here.
    #[test]
    fn the_words_of_a_built_in_type_are_read_in_any_order() {
        for source in [
            "struct S { unsigned long long int a; };",
            "struct S { long unsigned long a; };",
            "struct S { unsigned long long a; };",
        ] {
            assert_eq!(
                members(source)[0].kind,
                Type::primitive(Primitive::UnsignedLongLong),
                "{source}"
            );
        }
        assert_eq!(
            members("struct S { unsigned a; };")[0].kind,
            Type::primitive(Primitive::UnsignedInt),
            "unsigned on its own is unsigned int"
        );
    }

    #[test]
    fn a_fixed_width_name_is_the_type_of_that_width() {
        let members = members("struct S { uint32_t a; DWORD b; uint8_t c; size_t d; };");
        assert_eq!(members[0].kind, members[1].kind);
        assert_eq!(members[2].kind, Type::primitive(Primitive::UnsignedChar));
        assert_eq!(
            members[3].kind,
            Type::primitive(Primitive::UnsignedPointerSized)
        );
    }

    /// `BOOL` is an `int` in the Windows headers. Reading it as one byte would
    /// put every member after it at the wrong offset.
    #[test]
    fn a_windows_bool_is_four_bytes_and_a_windows_boolean_is_one() {
        let members = members("struct S { BOOL wide; BOOLEAN narrow; };");
        assert_eq!(members[0].kind, Type::primitive(Primitive::Int));
        assert_eq!(members[1].kind, Type::primitive(Primitive::UnsignedChar));
    }

    #[test]
    fn a_typedef_names_a_type_that_later_declarations_may_use() {
        let definitions = read(
            "typedef unsigned int handle_t;
             struct S { handle_t which; handle_t *out; };",
        );
        assert_eq!(definitions.len(), 1, "a typedef is not a definition");
        let members = definitions[0].members();
        assert_eq!(members[0].kind, Type::primitive(Primitive::UnsignedInt));
        assert_eq!(
            members[1].kind,
            Type::primitive(Primitive::UnsignedInt).pointer_to()
        );
    }

    #[test]
    fn an_enumeration_counts_on_from_the_last_value_it_was_given() {
        let definitions = read("enum Colour { Red, Green = 5, Blue, Black = -1 };");
        let Definition::Enumeration {
            base, constants, ..
        } = &definitions[0]
        else {
            panic!("an enumeration");
        };
        assert_eq!(*base, Primitive::Int);
        assert_eq!(
            constants
                .iter()
                .map(|constant| (constant.name.as_str(), constant.value))
                .collect::<Vec<_>>(),
            vec![("Red", 0), ("Green", 5), ("Blue", 6), ("Black", -1)]
        );
    }

    #[test]
    fn a_value_may_be_written_in_any_base_or_as_a_character() {
        let definitions = read("enum E { A = 0x10, B = 0b101, C = 'P', D = 017 };");
        let Definition::Enumeration { constants, .. } = &definitions[0] else {
            panic!("an enumeration");
        };
        let values: Vec<i64> = constants.iter().map(|constant| constant.value).collect();
        assert_eq!(values, vec![16, 5, 80, 15]);
    }

    /// A header is pasted in as it stands, and the parts that are not
    /// declarations must not stop it.
    #[test]
    fn comments_and_preprocessor_lines_are_dropped_rather_than_refused() {
        let members = members(
            "#include <stdint.h>
             #define MAX 8
             /* what the file holds
                over several lines */
             struct Header {
                 uint32_t magic;   // the four bytes at the front
                 uint16_t version; /* and what follows */
             };",
        );
        assert_eq!(members.len(), 2);
        assert_eq!(members[1].name, "version");
    }

    #[test]
    fn a_qualifier_is_read_and_says_nothing_about_the_layout() {
        let members = members("struct S { const volatile int a; char *const name; };");
        assert_eq!(members[0].kind, Type::primitive(Primitive::Int));
        assert_eq!(
            members[1].kind,
            Type::primitive(Primitive::Char).pointer_to()
        );
    }

    #[test]
    fn a_bit_field_carries_its_width_and_an_unnamed_one_carries_no_name() {
        let members = members(
            "struct S { unsigned int flag : 1; unsigned int : 0; unsigned int rest : 7; };",
        );
        assert_eq!(members[0].bits, Some(1));
        assert_eq!(members[1].name, "");
        assert_eq!(members[1].bits, Some(0));
        assert_eq!(members[2].bits, Some(7));
    }

    /// An array length that is a name is refused rather than guessed at: a
    /// guessed length puts every member after it at the wrong offset, and does
    /// it quietly.
    #[test]
    fn an_array_length_that_is_not_a_number_is_refused_by_name() {
        let error = refusal("struct S { char name[MAX]; };");
        assert!(
            error.message.contains("MAX"),
            "the refusal quotes what stopped it: {}",
            error.message
        );
        assert_eq!(error.line, 1);
    }

    #[test]
    fn the_line_a_refusal_happened_on_is_the_line_it_reports() {
        let error = refusal(
            "struct S {
                 int fine;
                 nonsense broken;
             };",
        );
        assert_eq!(error.line, 3);
    }

    /// `long double` is eighty bits on System V, sixty-four on Windows and
    /// sixty-four again on ARM64 macOS. Any one of those is wrong somewhere.
    #[test]
    fn long_double_is_refused_rather_than_laid_out_at_one_of_its_widths() {
        let error = refusal("struct S { long double wide; };");
        assert!(
            error.message.contains("long double"),
            "the refusal names it: {}",
            error.message
        );
    }

    #[test]
    fn a_definition_that_never_closes_is_refused_rather_than_read_as_far_as_it_got() {
        assert!(definitions("struct S { int a;").is_err());
        assert!(definitions("struct S { int a; };").is_ok());
    }

    #[test]
    fn nothing_at_all_defines_nothing_at_all() {
        assert!(definitions("").expect("read").is_empty());
        assert!(
            definitions("   \n // just a comment \n")
                .expect("read")
                .is_empty()
        );
    }
}
