//! Reading an address through a type.
//!
//! Given a [`Registry`], a [`Type`] and an address, this answers with the
//! members of that type and what each one holds — a tree, because a structure
//! holds structures.
//!
//! Two decisions are worth stating, because both are about not inventing
//! anything:
//!
//! - **A pointer is never followed on its own.** A linked list read to the
//!   bottom is an infinite tree, and a list whose tail is a value that never
//!   was a pointer is an infinite tree of nonsense. What a pointer holds is
//!   shown; where it leads is followed when the reader asks, one step at a
//!   time.
//! - **Bytes that are not mapped have no value.** [`Value::Unreadable`], not
//!   zero. A structure read at an address the file never had is then plainly
//!   empty rather than plausibly full of zeroes.

use crate::Endianness;

use super::{Definition, Layout, LayoutError, Model, Placed, Primitive, Registry, Type};

/// Somewhere bytes can be read from at an address.
///
/// Implemented for the emulated machine's memory, so a structure can be read
/// out of a running state, and for [`Flat`], so one can be read out of a byte
/// slice that has an address.
pub trait Source {
    /// Fills `into` from `address`. Answers `false` — leaving `into` in any
    /// state — when the whole range is not readable.
    fn read(&self, address: u64, into: &mut [u8]) -> bool;
}

/// A byte slice that starts at an address.
#[derive(Clone, Copy, Debug)]
pub struct Flat<'a> {
    pub base: u64,
    pub bytes: &'a [u8],
}

impl Source for Flat<'_> {
    fn read(&self, address: u64, into: &mut [u8]) -> bool {
        let Some(offset) = address.checked_sub(self.base) else {
            return false;
        };
        let Ok(offset) = usize::try_from(offset) else {
            return false;
        };
        let Some(end) = offset.checked_add(into.len()) else {
            return false;
        };
        match self.bytes.get(offset..end) {
            Some(slice) => {
                into.copy_from_slice(slice);
                true
            }
            None => false,
        }
    }
}

impl Source for crate::emulate::memory::Memory {
    fn read(&self, address: u64, into: &mut [u8]) -> bool {
        // The plain read, not the fetch: reading a structure is a data access,
        // and asking for execute permission would refuse every ordinary one.
        Self::read(self, address, into).is_ok()
    }
}

/// What one member holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    Bool(bool),
    /// One byte meant as a character.
    Character(u8),
    /// An address. Not followed; see the module documentation.
    Address(u64),
    /// An integer with names for some of its values, and the name of this one
    /// when it has one.
    Enumerated {
        value: i64,
        name: Option<String>,
    },
    /// An array of characters, as the text it spells, up to its first zero.
    Text(String),
    /// A structure, union or array. What it holds is in `members`.
    Aggregate,
    /// The bytes are not there to read.
    Unreadable,
    /// The type could not be laid out, so nothing was read.
    Undefined(LayoutError),
}

impl Value {
    /// The value as an unsigned number, for the readers that want one: the
    /// expression window, a jump to an address, a copy.
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Unsigned(value) | Self::Address(value) => Some(*value),
            #[expect(
                clippy::cast_sign_loss,
                reason = "the caller wants the bits, to go to an address or copy them"
            )]
            Self::Signed(value) | Self::Enumerated { value, .. } => Some(*value as u64),
            Self::Character(byte) => Some(u64::from(*byte)),
            Self::Bool(state) => Some(u64::from(*state)),
            Self::Float(_)
            | Self::Text(_)
            | Self::Aggregate
            | Self::Unreadable
            | Self::Undefined(_) => None,
        }
    }

    /// Whether it stands for something read rather than something missing.
    #[must_use]
    pub const fn is_known(&self) -> bool {
        !matches!(self, Self::Unreadable | Self::Undefined(_))
    }
}

/// One member of a type, at an address, with what it holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    /// How the member was named where it was declared. The root of a tree
    /// carries the type's own name.
    pub name: String,
    /// What it was declared as, written as C writes it.
    pub type_label: String,
    /// The type itself, so a caller can read further through it.
    pub kind: Type,
    pub address: u64,
    /// Bytes the member occupies. For a bit-field, its storage unit.
    pub size: u64,
    /// Which bits of the storage unit it is, when it is a bit-field.
    pub bits: Option<(u32, u32)>,
    pub value: Value,
    /// Something worth saying beside the value: the text a `char *` leads to.
    pub note: Option<String>,
    /// What it holds, for a structure, union or array.
    pub members: Vec<Reading>,
}

impl Reading {
    /// How many members there are below this one, at every depth.
    #[must_use]
    pub fn count(&self) -> usize {
        1 + self.members.iter().map(Self::count).sum::<usize>()
    }

    /// Whether anything at all was read: an entirely unreadable structure is
    /// worth saying so about once, rather than member by member.
    ///
    /// A structure is never itself a value, so it does not count as read; only
    /// something with bytes behind it does.
    #[must_use]
    pub fn any_known(&self) -> bool {
        let read_here = self.value.is_known() && self.value != Value::Aggregate;
        read_here || self.members.iter().any(Self::any_known)
    }
}

/// How far a reading goes before it stops expanding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Depth {
    /// How many levels of structure to expand.
    pub levels: usize,
    /// How many elements of one array to read.
    pub array: u64,
}

impl Default for Depth {
    /// Deep enough for the structures a reader writes by hand, and short
    /// enough that a buffer of a million bytes does not become a million rows
    /// before anyone has looked at it.
    fn default() -> Self {
        Self {
            levels: 8,
            array: 64,
        }
    }
}

/// How long a string a `char *` is followed for.
const TEXT_PREVIEW: usize = 64;

/// Reads `address` through `kind`.
///
/// The result is a tree: a structure holds its members, an array its elements.
/// Nothing is followed through a pointer.
#[must_use]
pub fn read(
    registry: &Registry,
    kind: &Type,
    address: u64,
    source: &dyn Source,
    depth: Depth,
) -> Reading {
    let name = kind.label();
    read_named(registry, &name, kind, address, None, source, depth)
}

fn read_named(
    registry: &Registry,
    name: &str,
    kind: &Type,
    address: u64,
    bits: Option<(u32, u32)>,
    source: &dyn Source,
    depth: Depth,
) -> Reading {
    let layout = match registry.layout(kind) {
        Ok(layout) => layout,
        Err(error) => {
            return Reading {
                name: name.to_owned(),
                type_label: kind.label(),
                kind: kind.clone(),
                address,
                size: 0,
                bits,
                value: Value::Undefined(error),
                note: None,
                members: Vec::new(),
            };
        }
    };

    let model = registry.model();
    let (value, note) = value_of(registry, kind, address, bits, layout, source, model);
    let members = if depth.levels == 0 {
        Vec::new()
    } else {
        members_of(registry, kind, address, source, depth)
    };

    Reading {
        name: name.to_owned(),
        type_label: kind.label(),
        kind: kind.clone(),
        address,
        size: layout.size,
        bits,
        value,
        note,
        members,
    }
}

/// What the bytes of one member say, and anything worth adding beside it.
fn value_of(
    registry: &Registry,
    kind: &Type,
    address: u64,
    bits: Option<(u32, u32)>,
    layout: Layout,
    source: &dyn Source,
    model: &Model,
) -> (Value, Option<String>) {
    match kind {
        Type::Primitive(primitive) => {
            let Some(raw) = integer(address, layout.size, source, model.endianness) else {
                return (Value::Unreadable, None);
            };
            let raw = match bits {
                Some((start, width)) => extract(raw, start, width),
                None => raw,
            };
            (
                primitive_value(*primitive, raw, layout.size, bits.map(|(_, width)| width)),
                None,
            )
        }
        Type::Pointer(inner) => {
            let Some(raw) = integer(address, model.pointer, source, model.endianness) else {
                return (Value::Unreadable, None);
            };
            // A pointer to characters is the one pointer worth following a
            // little way without being asked: the whole reason to declare a
            // `char *` is to read what it says.
            let note = if raw != 0 && is_character(inner) {
                text_at(raw, source).filter(|text| !text.is_empty())
            } else {
                None
            };
            (Value::Address(raw), note)
        }
        Type::Array(element, count) => {
            if is_character(element) {
                let length = usize::try_from(*count)
                    .unwrap_or(usize::MAX)
                    .min(TEXT_PREVIEW);
                let mut bytes = vec![0u8; length];
                if source.read(address, &mut bytes) {
                    return (Value::Text(text_of(&bytes)), None);
                }
                return (Value::Unreadable, None);
            }
            (Value::Aggregate, None)
        }
        Type::Named(name) => match registry.get(name) {
            Some(Definition::Enumeration { base, .. }) => {
                let Some(raw) = integer(address, layout.size, source, model.endianness) else {
                    return (Value::Unreadable, None);
                };
                let raw = match bits {
                    Some((start, width)) => extract(raw, start, width),
                    None => raw,
                };
                let value = if base.is_signed() {
                    sign_extend(raw, layout.size * 8)
                } else {
                    // An unsigned enumeration wider than `i63` would wrap, and
                    // no enumeration is stored in one.
                    #[expect(
                        clippy::cast_possible_wrap,
                        reason = "an enumeration is stored in at most eight bytes of value"
                    )]
                    {
                        raw as i64
                    }
                };
                (
                    Value::Enumerated {
                        value,
                        name: registry.constant(name, value).map(ToOwned::to_owned),
                    },
                    None,
                )
            }
            // A structure or union: what it holds is below it. Whether its
            // bytes are there at all is answered by its members, one of which
            // will say so.
            _ => (Value::Aggregate, None),
        },
    }
}

/// The members of a structure, union or array, read in turn.
fn members_of(
    registry: &Registry,
    kind: &Type,
    address: u64,
    source: &dyn Source,
    depth: Depth,
) -> Vec<Reading> {
    // A character array says what it spells; listing its bytes underneath as
    // well turns a name into sixteen rows of numbers.
    if let Type::Array(element, _) = kind {
        if is_character(element) {
            return Vec::new();
        }
    }
    let Ok(placed) = registry.members_of(kind, depth.array) else {
        return Vec::new();
    };
    let deeper = Depth {
        levels: depth.levels - 1,
        array: depth.array,
    };
    placed
        .into_iter()
        .map(
            |Placed {
                 name,
                 kind,
                 offset,
                 bits,
                 ..
             }| {
                read_named(
                    registry,
                    &name,
                    &kind,
                    address.wrapping_add(offset),
                    bits,
                    source,
                    deeper,
                )
            },
        )
        .collect()
}

/// A primitive, read from the bytes it was stored in.
#[expect(
    clippy::cast_possible_truncation,
    reason = "each cast takes exactly the bytes the type was read from"
)]
fn primitive_value(primitive: Primitive, raw: u64, size: u64, bits: Option<u32>) -> Value {
    let width = bits.map_or(size.saturating_mul(8), u64::from);
    match primitive {
        Primitive::Bool => Value::Bool(raw != 0),
        Primitive::Char | Primitive::SignedChar | Primitive::UnsignedChar if bits.is_none() => {
            Value::Character(raw as u8)
        }
        Primitive::Float => Value::Float(f64::from(f32::from_bits(raw as u32))),
        Primitive::Double => Value::Float(f64::from_bits(raw)),
        other if other.is_signed() => Value::Signed(sign_extend(raw, width)),
        Primitive::Void => Value::Unreadable,
        _ => Value::Unsigned(raw),
    }
}

/// Whether a type is one byte meant as a character, which decides whether an
/// array of it is shown as text.
fn is_character(kind: &Type) -> bool {
    matches!(
        kind,
        Type::Primitive(Primitive::Char | Primitive::SignedChar | Primitive::UnsignedChar)
    )
}

/// Reads up to eight bytes as one number.
fn integer(address: u64, size: u64, source: &dyn Source, endianness: Endianness) -> Option<u64> {
    let size = usize::try_from(size).ok()?;
    if size == 0 || size > 8 {
        return None;
    }
    let mut bytes = [0u8; 8];
    if !source.read(address, &mut bytes[..size]) {
        return None;
    }
    let mut value = 0u64;
    match endianness {
        Endianness::Big => {
            for byte in &bytes[..size] {
                value = (value << 8) | u64::from(*byte);
            }
        }
        // Unknown is read little-endian, as `Model::of` already decided.
        _ => {
            for byte in bytes[..size].iter().rev() {
                value = (value << 8) | u64::from(*byte);
            }
        }
    }
    Some(value)
}

/// The `width` bits of `raw` that start at `start`.
const fn extract(raw: u64, start: u32, width: u32) -> u64 {
    let start = if start > 63 { 63 } else { start };
    if width == 0 || width >= 64 {
        return raw >> start;
    }
    (raw >> start) & ((1u64 << width) - 1)
}

/// Reads `value` as a two's-complement number `width` bits wide.
#[expect(
    clippy::cast_possible_wrap,
    reason = "reading the same bits as signed is what the function is for"
)]
const fn sign_extend(value: u64, width: u64) -> i64 {
    if width == 0 || width >= 64 {
        return value as i64;
    }
    let shift = 64 - width;
    ((value << shift) as i64) >> shift
}

/// The zero-terminated text at an address, for a `char *`.
fn text_at(address: u64, source: &dyn Source) -> Option<String> {
    let mut bytes = Vec::with_capacity(TEXT_PREVIEW);
    for step in 0..TEXT_PREVIEW as u64 {
        let mut byte = [0u8; 1];
        if !source.read(address.wrapping_add(step), &mut byte) {
            // Nothing there at all is a pointer that leads nowhere readable;
            // stopping part way through is a string that runs off the end of
            // what is mapped, and what was read is still worth showing.
            return (step > 0).then(|| text_of(&bytes));
        }
        if byte[0] == 0 {
            break;
        }
        bytes.push(byte[0]);
    }
    Some(text_of(&bytes))
}

/// Bytes as the text they spell, up to the first zero.
///
/// Anything that is not printable becomes a dot, so a structure read at the
/// wrong address cannot put control characters into the interface.
fn text_of(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| match byte {
            0x20..=0x7e => char::from(*byte),
            _ => '.',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::parse;

    const BASE: u64 = 0x1000;

    fn registry(source: &str) -> Registry {
        let mut registry = Registry::new(Model {
            pointer: 8,
            long: 8,
            endianness: Endianness::Little,
        });
        for definition in parse::definitions(source).expect("the definitions read") {
            registry.define(definition);
        }
        registry
    }

    fn named(name: &str) -> Type {
        Type::Named(name.to_owned())
    }

    /// Reads `name` at [`BASE`] out of `bytes`.
    fn reading(registry: &Registry, name: &str, bytes: &[u8]) -> Reading {
        let source = Flat { base: BASE, bytes };
        read(registry, &named(name), BASE, &source, Depth::default())
    }

    /// The member of a reading by name, at any depth of one level.
    fn member<'a>(reading: &'a Reading, name: &str) -> &'a Reading {
        reading
            .members
            .iter()
            .find(|member| member.name == name)
            .unwrap_or_else(|| panic!("a member named {name}"))
    }

    #[test]
    fn every_member_is_read_from_its_own_offset() {
        let registry = registry("struct S { char tag; int count; unsigned short flags; };");
        let mut bytes = vec![0u8; 12];
        bytes[0] = b'A';
        bytes[4..8].copy_from_slice(&1_000_000u32.to_le_bytes());
        bytes[8..10].copy_from_slice(&0xbeefu16.to_le_bytes());

        let reading = reading(&registry, "S", &bytes);
        assert_eq!(member(&reading, "tag").value, Value::Character(b'A'));
        assert_eq!(member(&reading, "count").value, Value::Signed(1_000_000));
        assert_eq!(member(&reading, "flags").value, Value::Unsigned(0xbeef));
        assert_eq!(member(&reading, "count").address, BASE + 4);
    }

    #[test]
    fn a_negative_value_is_read_as_the_negative_it_is() {
        let registry = registry("struct S { int down; unsigned int up; };");
        let mut bytes = vec![0u8; 8];
        bytes[0..4].copy_from_slice(&(-3i32).to_le_bytes());
        bytes[4..8].copy_from_slice(&(-3i32).to_le_bytes());

        let reading = reading(&registry, "S", &bytes);
        assert_eq!(member(&reading, "down").value, Value::Signed(-3));
        assert_eq!(
            member(&reading, "up").value,
            Value::Unsigned(0xffff_fffd),
            "the same bytes, read as the type says"
        );
    }

    #[test]
    fn the_byte_order_of_the_file_is_the_byte_order_it_is_read_in() {
        let mut registry = registry("struct S { unsigned int number; };");
        let bytes = [0x11, 0x22, 0x33, 0x44];
        assert_eq!(
            member(&reading(&registry, "S", &bytes), "number").value,
            Value::Unsigned(0x4433_2211)
        );

        registry.set_model(Model {
            pointer: 8,
            long: 8,
            endianness: Endianness::Big,
        });
        assert_eq!(
            member(&reading(&registry, "S", &bytes), "number").value,
            Value::Unsigned(0x1122_3344)
        );
    }

    #[test]
    fn a_character_array_says_what_it_spells() {
        let registry = registry("struct S { char name[8]; };");
        let mut bytes = vec![0u8; 8];
        bytes[..5].copy_from_slice(b"hello");

        let reading = reading(&registry, "S", &bytes);
        let name = member(&reading, "name");
        assert_eq!(name.value, Value::Text("hello".to_owned()));
        assert!(
            name.members.is_empty(),
            "a name is one row, not eight rows of numbers"
        );
    }

    /// The whole reason to declare a `char *` is to read what it says.
    #[test]
    fn a_pointer_to_characters_carries_what_it_leads_to() {
        let registry = registry("struct S { char *label; };");
        let mut bytes = vec![0u8; 32];
        bytes[0..8].copy_from_slice(&(BASE + 16).to_le_bytes());
        bytes[16..21].copy_from_slice(b"words");

        let reading = reading(&registry, "S", &bytes);
        let label = member(&reading, "label");
        assert_eq!(label.value, Value::Address(BASE + 16));
        assert_eq!(label.note.as_deref(), Some("words"));
    }

    /// A list read to the bottom would be an infinite tree, and a list whose
    /// tail was never a pointer an infinite tree of nonsense.
    #[test]
    fn a_pointer_to_a_structure_is_shown_and_not_followed() {
        let registry = registry("struct Node { struct Node *next; int value; };");
        let mut bytes = vec![0u8; 16];
        bytes[0..8].copy_from_slice(&BASE.to_le_bytes());

        let reading = reading(&registry, "Node", &bytes);
        let next = member(&reading, "next");
        assert_eq!(next.value, Value::Address(BASE), "it points at itself");
        assert!(next.members.is_empty(), "and is not followed there");
    }

    #[test]
    fn a_bit_field_is_read_from_its_own_bits() {
        let registry = registry(
            "struct Flags {
                 unsigned int first : 1;
                 unsigned int second : 2;
                 unsigned int rest : 5;
             };",
        );
        // Counting from the least significant bit: 1, then 10, then 11010.
        let bytes = [0b1101_0101, 0, 0, 0];
        let reading = reading(&registry, "Flags", &bytes);
        assert_eq!(member(&reading, "first").value, Value::Unsigned(1));
        assert_eq!(member(&reading, "second").value, Value::Unsigned(2));
        assert_eq!(member(&reading, "rest").value, Value::Unsigned(0b11010));
    }

    #[test]
    fn a_signed_bit_field_is_read_as_a_negative_when_its_top_bit_is_set() {
        let registry = registry("struct S { int small : 4; };");
        let bytes = [0b0000_1111, 0, 0, 0];
        assert_eq!(
            member(&reading(&registry, "S", &bytes), "small").value,
            Value::Signed(-1),
            "four bits all set is minus one, not fifteen"
        );
    }

    #[test]
    fn an_enumeration_carries_the_name_of_the_value_it_holds() {
        let registry = registry(
            "enum Colour { Red = 0, Green = 5 };
             struct S { enum Colour shade; };",
        );
        let bytes = 5u32.to_le_bytes();
        assert_eq!(
            member(&reading(&registry, "S", &bytes), "shade").value,
            Value::Enumerated {
                value: 5,
                name: Some("Green".to_owned())
            }
        );

        let bytes = 9u32.to_le_bytes();
        assert_eq!(
            member(&reading(&registry, "S", &bytes), "shade").value,
            Value::Enumerated {
                value: 9,
                name: None
            },
            "a value the enumeration never named is shown as the number it is"
        );
    }

    /// Not zero. A structure read at an address the file never had must be
    /// plainly empty rather than plausibly full of zeroes.
    #[test]
    fn bytes_that_are_not_there_have_no_value_rather_than_a_value_of_zero() {
        let registry = registry("struct S { int a; int b; };");
        // Only the first member's bytes are mapped.
        let bytes = vec![0u8; 4];
        let reading = reading(&registry, "S", &bytes);
        assert_eq!(member(&reading, "a").value, Value::Signed(0));
        assert_eq!(member(&reading, "b").value, Value::Unreadable);
        assert!(reading.any_known(), "part of it was read");

        let nothing = read(
            &registry,
            &named("S"),
            0x9999,
            &Flat {
                base: BASE,
                bytes: &bytes,
            },
            Depth::default(),
        );
        assert!(!nothing.any_known(), "none of it was");
    }

    #[test]
    fn a_type_that_could_not_be_laid_out_says_so_instead_of_reading_anything() {
        let registry = registry("struct S { struct Missing thing; };");
        let reading = reading(&registry, "S", &[0u8; 16]);
        assert_eq!(
            reading.value,
            Value::Undefined(LayoutError::Unknown("Missing".to_owned()))
        );
    }

    #[test]
    fn a_structure_inside_a_structure_is_read_where_it_sits() {
        let registry = registry(
            "struct Inner { unsigned short a; unsigned short b; };
             struct Outer { char tag; struct Inner inner; };",
        );
        let mut bytes = vec![0u8; 8];
        bytes[0] = b'x';
        bytes[2..4].copy_from_slice(&7u16.to_le_bytes());
        bytes[4..6].copy_from_slice(&9u16.to_le_bytes());

        let reading = reading(&registry, "Outer", &bytes);
        let inner = member(&reading, "inner");
        assert_eq!(inner.address, BASE + 2);
        assert_eq!(member(inner, "a").value, Value::Unsigned(7));
        assert_eq!(member(inner, "b").value, Value::Unsigned(9));
    }

    #[test]
    fn an_array_is_read_element_by_element_up_to_the_depth_asked_for() {
        let registry = registry("struct S { unsigned int values[4]; };");
        let mut bytes = vec![0u8; 16];
        for (index, chunk) in bytes.chunks_exact_mut(4).enumerate() {
            let index = u32::try_from(index).expect("four elements");
            chunk.copy_from_slice(&(index * 10).to_le_bytes());
        }
        let reading = reading(&registry, "S", &bytes);
        let values = member(&reading, "values");
        assert_eq!(values.members.len(), 4);
        assert_eq!(values.members[2].value, Value::Unsigned(20));
        assert_eq!(values.members[2].name, "[2]");
    }

    #[test]
    fn a_float_is_read_as_the_number_it_stands_for() {
        let registry = registry("struct S { float single; double wide; };");
        let mut bytes = vec![0u8; 16];
        bytes[0..4].copy_from_slice(&1.5f32.to_le_bytes());
        bytes[8..16].copy_from_slice(&(-2.25f64).to_le_bytes());

        let reading = reading(&registry, "S", &bytes);
        assert_eq!(member(&reading, "single").value, Value::Float(1.5));
        assert_eq!(member(&reading, "wide").value, Value::Float(-2.25));
    }

    /// Text read at the wrong address must never put control characters into
    /// the interface.
    #[test]
    fn text_shows_only_what_can_be_printed() {
        let registry = registry("struct S { char name[6]; };");
        let bytes = [b'a', 0x1b, b'[', b'2', b'J', b'z'];
        assert_eq!(
            member(&reading(&registry, "S", &bytes), "name").value,
            Value::Text("a.[2Jz".to_owned())
        );
    }
}
