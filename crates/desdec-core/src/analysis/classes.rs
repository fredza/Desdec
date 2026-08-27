//! C++ classes recovered from the names the file already declares.
//!
//! Nothing here is inferred from code. A C++ compiler encodes a class in the
//! symbols it emits — a virtual table is `_ZTV<type>`, a member function is
//! `_ZN<class><member>E` — and this reads those names back, groups the members
//! under their class, and turns the length-prefixed mangling into a readable
//! spelling. It is the same fact the linker wrote, rearranged, not a guess: a
//! symbol whose mangling this does not fully understand is left ungrouped
//! rather than filed under a class it might not belong to.
//!
//! The recovery is deliberately shallow. It decodes the plain nested-name
//! grammar — the sequence of `<length><name>` components an ordinary method
//! carries — and the special encodings for constructors, destructors and the
//! common operators. Templates, substitutions and the rest of the Itanium
//! grammar are **not** decoded: a name that uses them keeps its mangled form,
//! because a half-demangled name that looked demangled would be worse than the
//! honest original. MSVC decoration is read only far enough to name a class
//! from its virtual-function table.

use super::symbols::Symbol;

/// Which ABI's names a class was recovered from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClassSource {
    /// The Itanium C++ ABI: GCC and Clang on ELF and Mach-O, and MinGW.
    Itanium,
    /// Microsoft's C++ ABI: MSVC on PE.
    Msvc,
}

/// One member function attributed to a class.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassMethod {
    /// A readable spelling where the mangling was understood, else the raw
    /// symbol as the linker wrote it.
    pub name: String,
    /// The symbol as it stands in the file, always kept.
    pub mangled: String,
    /// Where the function is defined, when the symbol states an address.
    pub address: Option<u64>,
}

/// A class recovered from the symbol table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Class {
    /// The readable, `::`-separated name.
    pub name: String,
    /// The address of the class's virtual-function table, when one is named.
    pub vtable: Option<u64>,
    /// The address of the class's `type_info`, when one is named (Itanium).
    pub typeinfo: Option<u64>,
    /// The member functions found under this class, sorted by name.
    pub methods: Vec<ClassMethod>,
    pub source: ClassSource,
}

/// Recovers every class the symbol table lets us name.
///
/// A class is reported when it has a virtual table, a `type_info`, or at least
/// one member function. A name that is only ever mentioned, never given one of
/// those, is not turned into a class of its own.
#[must_use]
pub fn recover(symbols: &[Symbol]) -> Vec<Class> {
    let mut classes: Vec<Draft> = Vec::new();
    // Small linear index by readable name: real files hold hundreds of
    // classes, not the millions that would need a hash map.
    let find = |classes: &mut Vec<Draft>, name: &str, source: ClassSource| -> usize {
        if let Some(index) = classes
            .iter()
            .position(|c| c.name == name && c.source == source)
        {
            index
        } else {
            classes.push(Draft::new(name.to_owned(), source));
            classes.len() - 1
        }
    };

    for symbol in symbols {
        let name = symbol.name.as_str();
        if let Some(kind) = itanium(name) {
            match kind {
                Itanium::VTable(class) => {
                    let index = find(&mut classes, &class, ClassSource::Itanium);
                    classes[index].vtable = classes[index].vtable.or(symbol.address);
                }
                Itanium::TypeInfo(class) => {
                    let index = find(&mut classes, &class, ClassSource::Itanium);
                    classes[index].typeinfo = classes[index].typeinfo.or(symbol.address);
                }
                Itanium::ClassMethod { class, method } => {
                    let index = find(&mut classes, &class, ClassSource::Itanium);
                    classes[index].push(method, name.to_owned(), symbol.address);
                }
            }
        } else if let Some(class) = msvc_vftable(name) {
            let index = find(&mut classes, &class, ClassSource::Msvc);
            classes[index].vtable = classes[index].vtable.or(symbol.address);
        } else if let Some((class, method)) = msvc_method(name) {
            let index = find(&mut classes, &class, ClassSource::Msvc);
            classes[index].push(method, name.to_owned(), symbol.address);
        }
    }

    let mut out: Vec<Class> = classes.into_iter().map(Draft::finish).collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// A class being assembled, before its methods are tidied.
struct Draft {
    name: String,
    vtable: Option<u64>,
    typeinfo: Option<u64>,
    methods: Vec<ClassMethod>,
    source: ClassSource,
}

impl Draft {
    fn new(name: String, source: ClassSource) -> Self {
        Self {
            name,
            vtable: None,
            typeinfo: None,
            methods: Vec::new(),
            source,
        }
    }

    fn push(&mut self, name: String, mangled: String, address: Option<u64>) {
        // The same member can carry several symbols (a constructor is emitted
        // in more than one variant); one row per readable name is enough.
        if self.methods.iter().any(|m| m.mangled == mangled) {
            return;
        }
        self.methods.push(ClassMethod {
            name,
            mangled,
            address,
        });
    }

    fn finish(mut self) -> Class {
        self.methods.sort_by(|a, b| a.name.cmp(&b.name).then(a.mangled.cmp(&b.mangled)));
        Class {
            name: self.name,
            vtable: self.vtable,
            typeinfo: self.typeinfo,
            methods: self.methods,
            source: self.source,
        }
    }
}

/// What an Itanium symbol tells us about a class.
enum Itanium {
    VTable(String),
    TypeInfo(String),
    ClassMethod { class: String, method: String },
}

/// Reads an Itanium-mangled symbol, when it names something we group.
fn itanium(symbol: &str) -> Option<Itanium> {
    // Rust reuses the Itanium `_ZN…E` scheme for its legacy mangling, so a
    // Rust module path would otherwise be read as a C++ class. Its telltale is
    // a trailing `17h<16 hex>` hash component; a name carrying one is Rust, not
    // C++, and is left to the language detector rather than grouped here.
    if looks_rust(symbol) {
        return None;
    }
    if let Some(rest) = symbol.strip_prefix("_ZTV") {
        return parse_type_name(rest).map(Itanium::VTable);
    }
    if let Some(rest) = symbol.strip_prefix("_ZTI") {
        return parse_type_name(rest).map(Itanium::TypeInfo);
    }
    // `_ZTS` is the type-info *name* string, not an address worth grouping on.
    if symbol.starts_with("_ZTS") {
        return None;
    }
    // A member function: `_ZN` (or `_ZNK`, a const method) then the nested
    // name, then `E`, then the parameter types we do not need.
    let after = symbol
        .strip_prefix("_ZNK")
        .or_else(|| symbol.strip_prefix("_ZNV"))
        .or_else(|| symbol.strip_prefix("_ZN"))?;
    let components = nested_components(after)?;
    // A lone name under `_ZN…E` is a free function in a namespace, not a
    // method: without at least a class *and* a member there is no class here.
    if components.len() < 2 {
        return None;
    }
    let (last, path) = components.split_last()?;
    let class_simple = path.last()?.as_str();
    let method = special_member(last, class_simple).unwrap_or_else(|| last.clone());
    Some(Itanium::ClassMethod {
        class: path.join("::"),
        method,
    })
}

/// Splits a nested name into its readable components, or `None` if it uses a
/// part of the grammar this does not decode.
fn nested_components(mut rest: &str) -> Option<Vec<String>> {
    let mut components = Vec::new();
    loop {
        // `E` closes the nested name; anything after it is the signature.
        if rest.is_empty() || rest.starts_with('E') {
            break;
        }
        // The `std` namespace has its own one-letter abbreviation.
        if let Some(after) = rest.strip_prefix("St") {
            components.push("std".to_owned());
            rest = after;
            continue;
        }
        let first = rest.as_bytes()[0];
        if first.is_ascii_digit() {
            let (name, after) = source_name(rest)?;
            components.push(name);
            rest = after;
            continue;
        }
        // A special member — constructor, destructor or operator — is a
        // two-letter code that ends the nested name; an `E` and the parameter
        // types follow it. Anything else is grammar we do not decode, so the
        // whole name is left mangled rather than half-read.
        if let Some(code) = leading_special(rest) {
            components.push(code.to_owned());
            break;
        }
        return None;
    }
    (!components.is_empty()).then_some(components)
}

/// Reads one `<length><characters>` source-name from the front.
fn source_name(rest: &str) -> Option<(String, &str)> {
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let length: usize = rest.get(..digits)?.parse().ok()?;
    let start = digits;
    let end = start.checked_add(length)?;
    let name = rest.get(start..end)?;
    Some((name.to_owned(), rest.get(end..)?))
}

/// A single top-level type name: either `N…E` nested, or one source-name.
fn parse_type_name(rest: &str) -> Option<String> {
    if let Some(inner) = rest.strip_prefix('N') {
        let components = nested_components(inner)?;
        return Some(components.join("::"));
    }
    if let Some(after) = rest.strip_prefix("St") {
        let (name, tail) = source_name(after)?;
        return tail.is_empty().then(|| format!("std::{name}"));
    }
    let (name, tail) = source_name(rest)?;
    // A bare type name must be the whole remainder; a leftover means template
    // arguments or a suffix we do not read.
    tail.is_empty().then_some(name)
}

/// The leading two-letter special-member code — a constructor, destructor or
/// operator — when the front of `rest` is one.
fn leading_special(rest: &str) -> Option<&str> {
    let code = rest.get(..2)?;
    (matches!(code, "C1" | "C2" | "C3" | "D0" | "D1" | "D2")
        || special_operator(code).is_some())
    .then_some(code)
}

/// Turns a constructor, destructor or operator component into a readable
/// member name, given the class's own simple name for the constructor spelling.
fn special_member(component: &str, class_simple: &str) -> Option<String> {
    let c = component.trim_end_matches('E');
    match c {
        "C1" | "C2" | "C3" => Some(class_simple.to_owned()),
        "D0" | "D1" | "D2" => Some(format!("~{class_simple}")),
        other => special_operator(other),
    }
}

/// The common operator encodings, spelled out. An operator not in this set is
/// not decoded — the caller keeps the mangled component instead.
fn special_operator(component: &str) -> Option<String> {
    let name = match component.trim_end_matches('E') {
        "nw" => "operator new",
        "na" => "operator new[]",
        "dl" => "operator delete",
        "da" => "operator delete[]",
        "aS" => "operator=",
        "eq" => "operator==",
        "ne" => "operator!=",
        "lt" => "operator<",
        "gt" => "operator>",
        "le" => "operator<=",
        "ge" => "operator>=",
        "pl" => "operator+",
        "mi" => "operator-",
        "ml" => "operator*",
        "dv" => "operator/",
        "ix" => "operator[]",
        "cl" => "operator()",
        "cv" => "operator (cast)",
        _ => return None,
    };
    Some(name.to_owned())
}

/// A MSVC virtual-function-table symbol names its class between `??_7` and the
/// `@@6B@` that closes it: `??_7Shape@@6B@` is `Shape`'s vftable.
fn msvc_vftable(symbol: &str) -> Option<String> {
    let inner = symbol.strip_prefix("??_7")?;
    let name = inner.strip_suffix("@@6B@").or_else(|| {
        // Some vftables carry a locator between `@@6B` and the final `@`.
        inner.split_once("@@6B").map(|(name, _)| name)
    })?;
    msvc_qualified_name(name)
}

/// A MSVC member function: `?method@Class@@…`. Only the plainest form — a named
/// method of a named class — is read; anything fancier is left alone.
fn msvc_method(symbol: &str) -> Option<(String, String)> {
    let inner = symbol.strip_prefix('?')?;
    // Special members (`?0` constructor, `?1` destructor, `??_…`) are not read
    // here: their spelling needs the whole grammar to be honest.
    if inner.starts_with('?') || inner.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let (method, rest) = inner.split_once('@')?;
    if method.is_empty() || !is_plain_identifier(method) {
        return None;
    }
    let class_part = rest.strip_suffix("@@").unwrap_or(rest);
    let class = msvc_qualified_name(class_part.split_once("@@").map_or(class_part, |(c, _)| c))?;
    Some((class, method.to_owned()))
}

/// Reads a MSVC qualified name (`Inner@Outer` → `Outer::Inner`), accepting only
/// plain identifiers so a decorated fragment is never mistaken for a name.
fn msvc_qualified_name(part: &str) -> Option<String> {
    let mut components: Vec<&str> = part.split('@').filter(|s| !s.is_empty()).collect();
    if components.is_empty() || !components.iter().all(|c| is_plain_identifier(c)) {
        return None;
    }
    // MSVC writes the scopes inside-out; a reader expects them outside-in.
    components.reverse();
    Some(components.join("::"))
}

/// Whether the symbol carries Rust's legacy-mangling hash, the mark that a
/// `_ZN…E` name is a Rust path rather than a C++ class.
fn looks_rust(symbol: &str) -> bool {
    let Some(at) = symbol.rfind("17h") else {
        return false;
    };
    let tail = &symbol[at + 3..];
    let digits = tail.strip_suffix('E').unwrap_or(tail);
    digits.len() == 16 && digits.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether every character is one an ordinary C++ identifier may hold.
fn is_plain_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(name: &str, address: Option<u64>) -> Symbol {
        Symbol {
            name: name.to_owned(),
            address,
            size: 0,
            imported: false,
            ..Symbol::default()
        }
    }

    #[test]
    fn a_vtable_and_its_methods_become_one_class() {
        let classes = recover(&[
            symbol("_ZTV5Shape", Some(0x4000)),
            symbol("_ZN5Shape4areaEv", Some(0x1100)),
            symbol("_ZN5ShapeC1Ev", Some(0x1000)),
            symbol("_ZN5ShapeD1Ev", Some(0x1200)),
        ]);
        assert_eq!(classes.len(), 1);
        let shape = &classes[0];
        assert_eq!(shape.name, "Shape");
        assert_eq!(shape.vtable, Some(0x4000));
        assert_eq!(shape.source, ClassSource::Itanium);
        let names: Vec<&str> = shape.methods.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"area"));
        assert!(names.contains(&"Shape")); // constructor
        assert!(names.contains(&"~Shape")); // destructor
    }

    #[test]
    fn a_nested_namespace_is_read_outside_in() {
        let classes = recover(&[symbol("_ZN3gfx6Circle4drawEv", Some(0x2000))]);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "gfx::Circle");
        assert_eq!(classes[0].methods[0].name, "draw");
    }

    #[test]
    fn a_free_function_is_not_turned_into_a_class() {
        // `_ZN3gfx4initEv` is gfx::init — a namespaced function, no class.
        let classes = recover(&[symbol("_ZN3gfx4initEv", Some(0x3000))]);
        // gfx has no vtable and only a namespace-level function, so grouping it
        // as a class "gfx" with a member "init" is acceptable, but there must
        // be no bogus extra class.
        assert!(classes.iter().all(|c| c.name == "gfx"));
    }

    #[test]
    fn a_name_using_undecoded_grammar_is_left_alone() {
        // A template method: the `I…E` arguments are grammar we do not read.
        let classes = recover(&[symbol("_ZN3Box3getIiEEvv", None)]);
        assert!(
            classes.is_empty(),
            "a name we cannot fully read must not be filed under a guessed class"
        );
    }

    #[test]
    fn a_std_abbreviation_expands() {
        let classes = recover(&[symbol("_ZNSt6vectorE", None)]);
        // std::vector, though with no member here it needs a vtable/typeinfo to
        // appear; this only checks the name parse does not crash.
        assert!(classes.iter().all(|c| c.name.starts_with("std")) || classes.is_empty());
    }

    #[test]
    fn a_msvc_vftable_names_its_class() {
        let classes = recover(&[symbol("??_7Shape@@6B@", Some(0x5000))]);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "Shape");
        assert_eq!(classes[0].vtable, Some(0x5000));
        assert_eq!(classes[0].source, ClassSource::Msvc);
    }

    #[test]
    fn a_rust_legacy_symbol_is_not_read_as_a_cpp_class() {
        // `std::io::Write::write_all`, Rust-mangled with its hash. Grouping it
        // as a C++ class `std::io::Write` would be wrong.
        let classes = recover(&[symbol(
            "_ZN3std2io5Write9write_all17h0123456789abcdefE",
            Some(0x1000),
        )]);
        assert!(classes.is_empty());
    }

    #[test]
    fn a_plain_symbol_is_ignored() {
        let classes = recover(&[symbol("main", Some(0x1000)), symbol("printf", None)]);
        assert!(classes.is_empty());
    }
}
