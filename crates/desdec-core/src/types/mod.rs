//! Types the reader names, and where their members sit in the bytes.
//!
//! A file states almost nothing about its own data. The listing says
//! `mov 0x18(%rbx),%rax`, and a reader who knows what `rbx` points at knows
//! that line reads a field; a reader who does not sees an offset. What is
//! missing is not in the file and cannot be recovered from it: it is the
//! reader's own knowledge, and the only useful thing a tool can do is let them
//! write it down once and then apply it everywhere.
//!
//! That is what this module holds. A [`Registry`] is a set of named
//! definitions — structures, unions, enumerations — written in the C the
//! headers of the program were written in (see [`parse`]), and laid out here
//! against a [`Model`] that says how wide a pointer is and which way round the
//! bytes go. [`read`] then reads an address through a type and answers with
//! the members and their values.
//!
//! Two rules, both of which follow from Desdec never running the file:
//!
//! - **A layout is computed, never measured.** The offsets come from the rules
//!   of the C the reader wrote, applied to the model of the file in front of
//!   them. A structure whose real layout was changed by a `#pragma pack` the
//!   reader did not write down will be laid out wrongly, and the honest thing
//!   is that the reader can see every offset and say so.
//! - **A member whose bytes are not there has no value.** Not zero. See
//!   [`read::Value::Unreadable`].

pub mod catalogue;
pub mod infer;
pub mod parse;
pub mod read;

use std::collections::{BTreeMap, BTreeSet};

use crate::{Architecture, BinaryFormat, Endianness};

/// How wide the types of one file are, and which way round its bytes go.
///
/// `long` is the reason this exists rather than a single constant. It is four
/// bytes on 32-bit targets, eight on 64-bit ELF and Mach-O, and **four** on
/// 64-bit Windows, which is LLP64. A structure with a `long` in it therefore
/// lays out differently in a PE than in an ELF built from the same header, and
/// a tool that assumes one silently reads every field after it at the wrong
/// offset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Model {
    /// Bytes in a pointer, and so in `size_t` and `uintptr_t`.
    pub pointer: u64,
    /// Bytes in a `long`.
    pub long: u64,
    /// Which way round a multi-byte value is stored.
    pub endianness: Endianness,
}

impl Default for Model {
    /// 64-bit little-endian with an eight-byte `long`: what a file whose
    /// format could not be read is most likely to be.
    fn default() -> Self {
        Self {
            pointer: 8,
            long: 8,
            endianness: Endianness::Little,
        }
    }
}

impl Model {
    /// The model of a file, from what its header stated.
    ///
    /// The byte order comes from the format rather than being asked for
    /// separately: a PE is little-endian, and an ELF or a Mach-O says which it
    /// is in the same header the architecture was read from.
    #[must_use]
    pub const fn of(architecture: Architecture, format: BinaryFormat) -> Self {
        let pointer = match architecture {
            Architecture::X86 | Architecture::Arm => 4,
            _ => 8,
        };
        // LLP64 on 64-bit Windows, LP64 everywhere else.
        let long = match (pointer, format) {
            (8, BinaryFormat::Pe) => 4,
            _ => pointer,
        };
        let endianness = match format {
            BinaryFormat::Elf { endianness, .. } | BinaryFormat::MachO { endianness, .. } => {
                endianness
            }
            BinaryFormat::Pe | BinaryFormat::Unknown => Endianness::Little,
        };
        let endianness = match endianness {
            // A file that did not state its byte order is read little-endian,
            // which every architecture Desdec disassembles uses in practice.
            Endianness::Unknown => Endianness::Little,
            stated => stated,
        };
        Self {
            pointer,
            long,
            endianness,
        }
    }
}

/// A type with no members of its own.
///
/// Spelled as C spells them, because that is what the reader will type. The
/// fixed-width names (`uint32_t` and its kin) are accepted by the parser and
/// resolved to the variant of the same width, so `uint32_t` and `unsigned int`
/// are the same type here — which they are on every target Desdec reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Primitive {
    /// Only ever useful behind a pointer. Has no size, and a member declared
    /// with it is refused.
    Void,
    Bool,
    /// One byte, shown as the character it stands for as well as a number.
    Char,
    SignedChar,
    UnsignedChar,
    Short,
    UnsignedShort,
    Int,
    UnsignedInt,
    /// Four bytes or eight, depending on the [`Model`].
    Long,
    UnsignedLong,
    LongLong,
    UnsignedLongLong,
    /// As wide as a pointer, signed: `intptr_t`, `ptrdiff_t`, `ssize_t`.
    SignedPointerSized,
    /// As wide as a pointer, unsigned: `size_t`, `uintptr_t`.
    UnsignedPointerSized,
    Float,
    Double,
}

impl Primitive {
    /// How it is written in C, and so how it is shown.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Void => "void",
            Self::Bool => "bool",
            Self::Char => "char",
            Self::SignedChar => "signed char",
            Self::UnsignedChar => "unsigned char",
            Self::Short => "short",
            Self::UnsignedShort => "unsigned short",
            Self::Int => "int",
            Self::UnsignedInt => "unsigned int",
            Self::Long => "long",
            Self::UnsignedLong => "unsigned long",
            Self::LongLong => "long long",
            Self::UnsignedLongLong => "unsigned long long",
            Self::SignedPointerSized => "intptr_t",
            Self::UnsignedPointerSized => "size_t",
            Self::Float => "float",
            Self::Double => "double",
        }
    }

    /// Bytes it occupies under `model`, or `None` for `void`.
    #[must_use]
    pub const fn size(self, model: &Model) -> Option<u64> {
        Some(match self {
            Self::Void => return None,
            Self::Bool | Self::Char | Self::SignedChar | Self::UnsignedChar => 1,
            Self::Short | Self::UnsignedShort => 2,
            Self::Int | Self::UnsignedInt | Self::Float => 4,
            Self::Long | Self::UnsignedLong => model.long,
            Self::LongLong | Self::UnsignedLongLong | Self::Double => 8,
            Self::SignedPointerSized | Self::UnsignedPointerSized => model.pointer,
        })
    }

    /// Whether values of it are read as two's-complement negatives.
    ///
    /// A plain `char` is signed on x86 and ARM64 Linux, macOS and Windows, and
    /// unsigned on ARM32 Linux. Signed is the reading that matches every
    /// target Desdec emulates.
    #[must_use]
    pub const fn is_signed(self) -> bool {
        matches!(
            self,
            Self::Char
                | Self::SignedChar
                | Self::Short
                | Self::Int
                | Self::Long
                | Self::LongLong
                | Self::SignedPointerSized
        )
    }

    /// Whether it holds a floating-point value rather than an integer.
    #[must_use]
    pub const fn is_float(self) -> bool {
        matches!(self, Self::Float | Self::Double)
    }
}

/// Anything a member can be declared as.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Type {
    Primitive(Primitive),
    /// An address at which something of the inner type sits. As wide as the
    /// model's pointer whatever it points at, which is what lets a structure
    /// hold a pointer to itself.
    Pointer(Box<Type>),
    /// `count` values of the inner type, one after another.
    Array(Box<Type>, u64),
    /// A structure, union or enumeration by the name it was defined under.
    Named(String),
}

impl Type {
    /// A shorthand for the common case.
    #[must_use]
    pub const fn primitive(primitive: Primitive) -> Self {
        Self::Primitive(primitive)
    }

    /// A pointer to this type.
    #[must_use]
    pub fn pointer_to(self) -> Self {
        Self::Pointer(Box::new(self))
    }

    /// How it is written in C, near enough to be pasted back into a header.
    ///
    /// Declarator syntax is not reproduced: a pointer to an array is written
    /// `int[4] *` rather than `int (*)[4]`, because the C spelling is read
    /// inside out and the point of showing a type in a table is to be read at
    /// a glance.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Primitive(primitive) => primitive.label().to_owned(),
            Self::Pointer(inner) => format!("{} *", inner.label()),
            Self::Array(inner, count) => format!("{}[{count}]", inner.label()),
            Self::Named(name) => name.clone(),
        }
    }

    /// The type reached by following every pointer and array to its element.
    #[must_use]
    pub fn innermost(&self) -> &Self {
        match self {
            Self::Pointer(inner) | Self::Array(inner, _) => inner.innermost(),
            other => other,
        }
    }
}

/// One declared member of a structure or union.
///
/// An empty name is an anonymous bit-field — `unsigned int : 3;` — which is
/// padding the reader wrote out by hand. It takes up its bits and is not shown
/// as a member, because it does not name anything to show.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Member {
    pub name: String,
    pub kind: Type,
    /// Width in bits when the member was declared as a bit-field, as in
    /// `unsigned int enabled : 1;`.
    pub bits: Option<u32>,
}

impl Member {
    /// An ordinary member, occupying whole bytes.
    #[must_use]
    pub fn new(name: impl Into<String>, kind: Type) -> Self {
        Self {
            name: name.into(),
            kind,
            bits: None,
        }
    }

    /// A member declared as a bit-field of `bits` bits.
    #[must_use]
    pub fn bitfield(name: impl Into<String>, kind: Type, bits: u32) -> Self {
        Self {
            name: name.into(),
            kind,
            bits: Some(bits),
        }
    }
}

/// One named constant of an enumeration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Constant {
    pub name: String,
    pub value: i64,
}

/// What a name in the registry stands for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Definition {
    /// Members one after another, each at its own offset.
    Struct { name: String, members: Vec<Member> },
    /// Members all at offset zero, sharing the same bytes.
    Union { name: String, members: Vec<Member> },
    /// An integer with names for some of its values.
    Enumeration {
        name: String,
        /// What it is stored as. `int` unless the reader said otherwise.
        base: Primitive,
        constants: Vec<Constant>,
    },
}

impl Definition {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Struct { name, .. }
            | Self::Union { name, .. }
            | Self::Enumeration { name, .. } => name,
        }
    }

    /// What a name that has never been defined is written as, so that a type
    /// referring to one still reads back as C.
    pub const STRUCT: &'static str = "struct";

    /// The word C introduces it with, for showing it back.
    #[must_use]
    pub const fn keyword(&self) -> &'static str {
        match self {
            Self::Struct { .. } => "struct",
            Self::Union { .. } => "union",
            Self::Enumeration { .. } => "enum",
        }
    }

    /// Its members, or an empty slice for an enumeration.
    #[must_use]
    pub fn members(&self) -> &[Member] {
        match self {
            Self::Struct { members, .. } | Self::Union { members, .. } => members,
            Self::Enumeration { .. } => &[],
        }
    }
}

/// How much room a type takes, and where it may start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Layout {
    /// Bytes from the start of the type to the start of the next one beside
    /// it, trailing padding included.
    pub size: u64,
    /// The multiple of which an address of this type is.
    pub alignment: u64,
}

/// Why a type could not be laid out.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutError {
    /// A name no definition in the registry answers to.
    Unknown(String),
    /// A structure that, following its members, contains itself. Through a
    /// pointer this is ordinary and allowed; by value it is a type of infinite
    /// size, and no compiler accepts one either.
    Recursive(String),
    /// `void`, or an array of it: a member with no size at all.
    Sizeless,
    /// An array whose element count makes it wider than the address space.
    TooLarge,
    /// A bit-field wider than the type it is stored in.
    Overwide { member: String, bits: u32 },
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(name) => write!(formatter, "no type named {name} has been defined"),
            Self::Recursive(name) => write!(formatter, "{name} contains itself"),
            Self::Sizeless => formatter.write_str("void has no size"),
            Self::TooLarge => formatter.write_str("the type is wider than the address space"),
            Self::Overwide { member, bits } => {
                write!(
                    formatter,
                    "{member} asks for {bits} bits, more than its type holds"
                )
            }
        }
    }
}

impl std::error::Error for LayoutError {}

/// One member, placed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Placed {
    pub name: String,
    pub kind: Type,
    /// Bytes from the start of the enclosing type.
    pub offset: u64,
    /// Where its bits sit when it is a bit-field: the first bit counted from
    /// the least significant end of the storage unit at `offset`, and how many
    /// of them. `None` for an ordinary member.
    pub bits: Option<(u32, u32)>,
    /// The room the member itself takes. For a bit-field, the storage unit it
    /// was allocated in.
    pub layout: Layout,
}

/// The types one reader has written down.
///
/// Definitions may name each other in any order, and may name themselves
/// through a pointer. Layout is computed on demand rather than stored, so a
/// definition can be replaced without anything else going stale.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    definitions: BTreeMap<String, Definition>,
    model: Model,
}

impl Registry {
    /// An empty registry for a file of this shape.
    #[must_use]
    pub fn new(model: Model) -> Self {
        Self {
            definitions: BTreeMap::new(),
            model,
        }
    }

    #[must_use]
    pub const fn model(&self) -> &Model {
        &self.model
    }

    /// Reads later definitions against a different file.
    ///
    /// The definitions themselves are untouched: they are what the reader
    /// wrote, and are not about any one file. Only their layout changes.
    pub const fn set_model(&mut self, model: Model) {
        self.model = model;
    }

    /// Adds a definition, replacing any of the same name.
    pub fn define(&mut self, definition: Definition) {
        self.definitions
            .insert(definition.name().to_owned(), definition);
    }

    /// Forgets one definition. Answers whether there was one.
    pub fn forget(&mut self, name: &str) -> bool {
        self.definitions.remove(name).is_some()
    }

    /// Forgets every definition.
    pub fn clear(&mut self) {
        self.definitions.clear();
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Definition> {
        self.definitions.get(name)
    }

    /// Every definition, by name.
    pub fn all(&self) -> impl Iterator<Item = &Definition> {
        self.definitions.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }

    /// How much room `kind` takes, and where it may start.
    ///
    /// # Errors
    ///
    /// When the type names something undefined, contains itself by value, or
    /// is `void`.
    pub fn layout(&self, kind: &Type) -> Result<Layout, LayoutError> {
        self.layout_within(kind, &mut BTreeSet::new())
    }

    /// Where each member of `kind` sits, in declaration order.
    ///
    /// A type with no members of its own — a primitive, a pointer, an
    /// enumeration — answers with an empty list rather than an error, so a
    /// caller walking a tree does not have to ask what kind it is first. An
    /// array answers with one entry per element, up to `limit`, which keeps a
    /// million-element array from being expanded before anyone looks at it.
    ///
    /// # Errors
    ///
    /// As [`Registry::layout`].
    pub fn members_of(&self, kind: &Type, limit: u64) -> Result<Vec<Placed>, LayoutError> {
        match kind {
            Type::Array(element, count) => {
                let layout = self.layout(element)?;
                let shown = (*count).min(limit);
                (0..shown)
                    .map(|index| {
                        Ok(Placed {
                            name: format!("[{index}]"),
                            kind: (**element).clone(),
                            offset: index
                                .checked_mul(layout.size)
                                .ok_or(LayoutError::TooLarge)?,
                            bits: None,
                            layout,
                        })
                    })
                    .collect()
            }
            Type::Named(name) => match self.definitions.get(name) {
                None => Err(LayoutError::Unknown(name.clone())),
                Some(Definition::Enumeration { .. }) => Ok(Vec::new()),
                Some(Definition::Struct { members, .. }) => self
                    .place(members, Packing::Sequential, &mut visiting(name))
                    .map(|(placed, _)| placed),
                Some(Definition::Union { members, .. }) => self
                    .place(members, Packing::Overlaid, &mut visiting(name))
                    .map(|(placed, _)| placed),
            },
            Type::Primitive(_) | Type::Pointer(_) => Ok(Vec::new()),
        }
    }

    /// The name for `value` in an enumeration, when it has one.
    #[must_use]
    pub fn constant(&self, name: &str, value: i64) -> Option<&str> {
        let Some(Definition::Enumeration { constants, .. }) = self.definitions.get(name) else {
            return None;
        };
        constants
            .iter()
            .find(|constant| constant.value == value)
            .map(|constant| constant.name.as_str())
    }

    /// The whole registry written back out as C, in name order.
    ///
    /// What is read by [`parse::definitions`], so a registry survives being
    /// saved and read again.
    #[must_use]
    pub fn to_source(&self) -> String {
        let mut out = String::new();
        for definition in self.definitions.values() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&write_definition(definition, self));
        }
        out
    }

    fn layout_within(
        &self,
        kind: &Type,
        open: &mut BTreeSet<String>,
    ) -> Result<Layout, LayoutError> {
        match kind {
            Type::Primitive(primitive) => {
                let size = primitive.size(&self.model).ok_or(LayoutError::Sizeless)?;
                Ok(Layout {
                    size,
                    alignment: size,
                })
            }
            // A pointer is as wide as the model's pointer whatever it points
            // at, and what it points at is deliberately not laid out here:
            // that is what lets `struct Node { struct Node *next; }` work.
            Type::Pointer(_) => Ok(Layout {
                size: self.model.pointer,
                alignment: self.model.pointer,
            }),
            Type::Array(element, count) => {
                let element = self.layout_within(element, open)?;
                let size = element
                    .size
                    .checked_mul(*count)
                    .ok_or(LayoutError::TooLarge)?;
                Ok(Layout {
                    size,
                    alignment: element.alignment,
                })
            }
            Type::Named(name) => {
                let definition = self
                    .definitions
                    .get(name)
                    .ok_or_else(|| LayoutError::Unknown(name.clone()))?;
                if !open.insert(name.clone()) {
                    return Err(LayoutError::Recursive(name.clone()));
                }
                let layout = match definition {
                    Definition::Enumeration { base, .. } => {
                        let size = base.size(&self.model).ok_or(LayoutError::Sizeless)?;
                        Ok(Layout {
                            size,
                            alignment: size,
                        })
                    }
                    Definition::Struct { members, .. } => self
                        .place(members, Packing::Sequential, open)
                        .and_then(|(_, end)| self.extent(members, end, open)),
                    Definition::Union { members, .. } => self
                        .place(members, Packing::Overlaid, open)
                        .and_then(|(_, end)| self.extent(members, end, open)),
                };
                open.remove(name);
                layout
            }
        }
    }

    /// Where every member of a composite sits.
    ///
    /// Bit-fields are allocated within a storage unit of their declared type,
    /// starting at the least significant bit — which is what both the System V
    /// ABI and MSVC do on the little-endian targets Desdec reads. A member
    /// that does not fit in the open unit opens a new one; an ordinary member,
    /// or a zero-width bit-field, closes it.
    fn place(
        &self,
        members: &[Member],
        packing: Packing,
        open: &mut BTreeSet<String>,
    ) -> Result<(Vec<Placed>, u64), LayoutError> {
        let mut run = Run {
            placed: Vec::with_capacity(members.len()),
            offset: 0,
            unit: None,
            packing,
        };
        for member in members {
            let layout = self.layout_within(&member.kind, open)?;
            match member.bits {
                Some(bits) => run.bitfield(member, layout, bits)?,
                None => run.ordinary(member, layout),
            }
        }
        let end = run.end();
        Ok((run.placed, end))
    }

    /// The room a composite takes, from where its members ended up.
    fn extent(
        &self,
        members: &[Member],
        end: u64,
        open: &mut BTreeSet<String>,
    ) -> Result<Layout, LayoutError> {
        let mut alignment = 1u64;
        for member in members {
            let layout = self.layout_within(&member.kind, open)?;
            alignment = alignment.max(layout.alignment);
        }
        // A structure with nothing in it is one byte, so that an array of them
        // still gives every element an address of its own.
        let size = round_up(end.max(u64::from(members.is_empty())), alignment);
        Ok(Layout { size, alignment })
    }
}

/// Whether members follow each other or share the same bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Packing {
    Sequential,
    Overlaid,
}

/// A bit-field storage unit still being filled.
#[derive(Clone, Copy, Debug)]
struct OpenUnit {
    offset: u64,
    layout: Layout,
    used: u32,
}

/// Members being placed one after another.
struct Run {
    placed: Vec<Placed>,
    /// Where the next member starts, once whatever is open has been closed.
    offset: u64,
    unit: Option<OpenUnit>,
    packing: Packing,
}

impl Run {
    /// An ordinary member, aligned and given its own bytes.
    fn ordinary(&mut self, member: &Member, layout: Layout) {
        self.close_unit();
        let at = self.next(layout.alignment);
        self.placed.push(Placed {
            name: member.name.clone(),
            kind: member.kind.clone(),
            offset: at,
            bits: None,
            layout,
        });
        if self.packing == Packing::Sequential {
            self.offset = at.saturating_add(layout.size);
        }
    }

    /// A bit-field, allocated in the open storage unit when it still fits.
    ///
    /// Bits are counted from the least significant end, which is what both the
    /// System V ABI and MSVC do on the little-endian targets Desdec reads.
    fn bitfield(&mut self, member: &Member, layout: Layout, bits: u32) -> Result<(), LayoutError> {
        let capacity = u32::try_from(layout.size.saturating_mul(8)).unwrap_or(u32::MAX);
        if bits > capacity {
            return Err(LayoutError::Overwide {
                member: member.name.clone(),
                bits,
            });
        }
        if bits == 0 {
            // `int : 0;` names nothing, and asks for the next unit to start
            // clean.
            self.close_unit();
            return Ok(());
        }

        let fits = self.unit.is_some_and(|unit| {
            unit.layout.size == layout.size
                && unit.used + bits <= capacity
                && self.packing == Packing::Sequential
        });
        let (at, start) = if fits {
            let unit = self.unit.as_mut().expect("a unit that was just checked");
            let start = unit.used;
            unit.used += bits;
            (unit.offset, start)
        } else {
            self.close_unit();
            let at = self.next(layout.alignment);
            self.unit = Some(OpenUnit {
                offset: at,
                layout,
                used: bits,
            });
            self.offset = at;
            (at, 0)
        };

        if !member.name.is_empty() {
            self.placed.push(Placed {
                name: member.name.clone(),
                kind: member.kind.clone(),
                offset: at,
                bits: Some((start, bits)),
                layout,
            });
        }
        Ok(())
    }

    /// Where the composite ends, the unit still being filled included.
    fn end(&self) -> u64 {
        let open = self
            .unit
            .map_or(0, |unit| unit.offset.saturating_add(unit.layout.size));
        self.placed
            .iter()
            .fold(self.offset.max(open), |end, placed| {
                end.max(placed.offset.saturating_add(placed.layout.size))
            })
    }

    /// Where a member of this alignment starts.
    const fn next(&self, alignment: u64) -> u64 {
        match self.packing {
            Packing::Sequential => round_up(self.offset, alignment),
            // Every member of a union starts where the union does.
            Packing::Overlaid => 0,
        }
    }

    /// Gives up on filling the open bit-field unit, if there is one.
    fn close_unit(&mut self) {
        if let Some(closed) = self.unit.take() {
            self.offset = closed.offset.saturating_add(closed.layout.size);
        }
    }
}

fn visiting(name: &str) -> BTreeSet<String> {
    let mut open = BTreeSet::new();
    open.insert(name.to_owned());
    open
}

/// The next multiple of `alignment` at or after `value`.
#[must_use]
pub const fn round_up(value: u64, alignment: u64) -> u64 {
    if alignment <= 1 {
        return value;
    }
    match value % alignment {
        0 => value,
        remainder => value.saturating_add(alignment - remainder),
    }
}

/// One definition written back out as C.
fn write_definition(definition: &Definition, registry: &Registry) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    match definition {
        Definition::Enumeration {
            name,
            base,
            constants,
        } => {
            let _ = writeln!(out, "enum {name} : {} {{", base.label());
            for constant in constants {
                let _ = writeln!(out, "    {} = {},", constant.name, constant.value);
            }
            out.push_str("};\n");
        }
        Definition::Struct { name, members } | Definition::Union { name, members } => {
            let _ = writeln!(out, "{} {name} {{", definition.keyword());
            for member in members {
                let _ = writeln!(out, "    {};", declaration(member, registry));
            }
            out.push_str("};\n");
        }
    }
    out
}

/// One member written as C declares it, arrays and pointers included.
///
/// A named type is written with the keyword it was defined under, because
/// `Node *next;` is only C once something has made `Node` a name for a type,
/// and `struct Node *next;` always is.
fn declaration(member: &Member, registry: &Registry) -> String {
    let mut kind = &member.kind;
    let mut stars = String::new();
    let mut suffix = String::new();
    // C reads a declarator inside out: the array brackets follow the name and
    // the stars precede it, so they are peeled off the type here rather than
    // printed by `Type::label`.
    loop {
        match kind {
            Type::Pointer(inner) if suffix.is_empty() => {
                stars.push('*');
                kind = inner;
            }
            Type::Array(inner, count) => {
                use std::fmt::Write as _;

                let _ = write!(suffix, "[{count}]");
                kind = inner;
            }
            _ => break,
        }
    }
    let base = match kind {
        Type::Named(name) => {
            let keyword = registry
                .get(name)
                .map_or(Definition::STRUCT, Definition::keyword);
            format!("{keyword} {name}")
        }
        other => other.label(),
    };
    let bits = member
        .bits
        .map(|bits| format!(" : {bits}"))
        .unwrap_or_default();
    format!("{base} {stars}{}{suffix}{bits}", member.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The model of a 64-bit ELF: eight-byte pointers and an eight-byte
    /// `long`.
    fn lp64() -> Registry {
        Registry::new(Model {
            pointer: 8,
            long: 8,
            endianness: Endianness::Little,
        })
    }

    /// The model of 64-bit Windows: eight-byte pointers and a four-byte
    /// `long`.
    fn llp64() -> Registry {
        Registry::new(Model {
            pointer: 8,
            long: 4,
            endianness: Endianness::Little,
        })
    }

    fn define(registry: &mut Registry, source: &str) {
        for definition in parse::definitions(source).expect("the definitions read") {
            registry.define(definition);
        }
    }

    fn named(name: &str) -> Type {
        Type::Named(name.to_owned())
    }

    /// Where each member of a definition ended up, by name.
    fn offsets(registry: &Registry, name: &str) -> Vec<(String, u64)> {
        registry
            .members_of(&named(name), 64)
            .expect("the type is laid out")
            .into_iter()
            .map(|placed| (placed.name, placed.offset))
            .collect()
    }

    #[test]
    fn a_member_starts_at_the_next_address_its_own_alignment_allows() {
        let mut registry = lp64();
        define(&mut registry, "struct S { char a; int b; char c; };");
        assert_eq!(
            offsets(&registry, "S"),
            vec![
                ("a".to_owned(), 0),
                ("b".to_owned(), 4),
                ("c".to_owned(), 8)
            ],
            "three bytes of padding go before the int, none before the char"
        );
        assert_eq!(
            registry.layout(&named("S")).expect("laid out"),
            Layout {
                size: 12,
                alignment: 4
            },
            "and the structure is rounded up to its own alignment"
        );
    }

    /// The one difference between a 64-bit Windows build and every other
    /// 64-bit build, and it moves every member after it.
    #[test]
    fn a_long_is_four_bytes_on_windows_and_eight_everywhere_else() {
        let source = "struct S { long a; long b; };";
        let mut wide = lp64();
        define(&mut wide, source);
        let mut narrow = llp64();
        define(&mut narrow, source);

        assert_eq!(offsets(&wide, "S")[1].1, 8);
        assert_eq!(offsets(&narrow, "S")[1].1, 4);
        assert_eq!(wide.layout(&named("S")).expect("laid out").size, 16);
        assert_eq!(narrow.layout(&named("S")).expect("laid out").size, 8);
    }

    #[test]
    fn every_member_of_a_union_starts_where_the_union_does() {
        let mut registry = lp64();
        define(
            &mut registry,
            "union U { unsigned int number; unsigned char bytes[4]; double wide; };",
        );
        for (name, offset) in offsets(&registry, "U") {
            assert_eq!(offset, 0, "{name} shares the union's own address");
        }
        assert_eq!(
            registry.layout(&named("U")).expect("laid out"),
            Layout {
                size: 8,
                alignment: 8
            },
            "a union is as wide and as aligned as its widest member"
        );
    }

    /// A structure that holds itself has no size, and no compiler accepts one
    /// either. Through a pointer it is the ordinary shape of a list.
    #[test]
    fn a_structure_that_holds_itself_is_refused_and_one_that_points_to_itself_is_not() {
        let mut registry = lp64();
        define(&mut registry, "struct Loop { struct Loop inner; };");
        assert_eq!(
            registry.layout(&named("Loop")),
            Err(LayoutError::Recursive("Loop".to_owned()))
        );

        define(
            &mut registry,
            "struct Node { struct Node *next; char name[16]; };",
        );
        assert_eq!(
            registry.layout(&named("Node")).expect("laid out"),
            Layout {
                size: 24,
                alignment: 8
            }
        );
    }

    #[test]
    fn a_name_no_definition_answers_to_is_named_in_the_refusal() {
        let mut registry = lp64();
        define(&mut registry, "struct S { struct Missing thing; };");
        assert_eq!(
            registry.layout(&named("S")),
            Err(LayoutError::Unknown("Missing".to_owned()))
        );
    }

    /// Bit-fields fill one storage unit at a time, and one that does not fit
    /// opens the next.
    #[test]
    fn bit_fields_fill_a_storage_unit_before_opening_another() {
        let mut registry = lp64();
        define(
            &mut registry,
            "struct Flags {
                 unsigned int first : 1;
                 unsigned int second : 2;
                 unsigned int rest : 29;
                 unsigned int overflowing : 1;
             };",
        );
        let placed = registry.members_of(&named("Flags"), 64).expect("laid out");
        assert_eq!(placed[0].bits, Some((0, 1)));
        assert_eq!(placed[1].bits, Some((1, 2)));
        assert_eq!(placed[2].bits, Some((3, 29)));
        assert_eq!(placed[0].offset, 0);
        assert_eq!(placed[2].offset, 0, "all three are in the same four bytes");
        assert_eq!(
            (placed[3].offset, placed[3].bits),
            (4, Some((0, 1))),
            "the fourth does not fit, so it starts the next unit"
        );
        assert_eq!(registry.layout(&named("Flags")).expect("laid out").size, 8);
    }

    #[test]
    fn a_zero_width_bit_field_closes_the_unit_it_follows() {
        let mut registry = lp64();
        define(
            &mut registry,
            "struct S {
                 unsigned int a : 1;
                 unsigned int : 0;
                 unsigned int b : 1;
             };",
        );
        let placed = registry.members_of(&named("S"), 64).expect("laid out");
        assert_eq!(placed.len(), 2, "the unnamed one is not a member");
        assert_eq!(placed[1].offset, 4, "b starts a unit of its own");
    }

    #[test]
    fn a_bit_field_wider_than_what_holds_it_is_refused() {
        let mut registry = lp64();
        define(&mut registry, "struct S { unsigned char wide : 9; };");
        assert_eq!(
            registry.layout(&named("S")),
            Err(LayoutError::Overwide {
                member: "wide".to_owned(),
                bits: 9
            })
        );
    }

    #[test]
    fn an_array_is_its_element_repeated_and_aligned_like_one() {
        let registry = lp64();
        let kind = Type::Array(Box::new(Type::primitive(Primitive::Int)), 5);
        assert_eq!(
            registry.layout(&kind).expect("laid out"),
            Layout {
                size: 20,
                alignment: 4
            }
        );
        let placed = registry.members_of(&kind, 64).expect("laid out");
        assert_eq!(placed.len(), 5);
        assert_eq!(placed[3].name, "[3]");
        assert_eq!(placed[3].offset, 12);
    }

    /// A buffer of a million bytes must not become a million rows before
    /// anyone has looked at it.
    #[test]
    fn a_long_array_is_expanded_no_further_than_it_was_asked_to_be() {
        let registry = lp64();
        let kind = Type::Array(
            Box::new(Type::primitive(Primitive::UnsignedChar)),
            1_000_000,
        );
        assert_eq!(registry.members_of(&kind, 16).expect("laid out").len(), 16);
        assert_eq!(
            registry.layout(&kind).expect("laid out").size,
            1_000_000,
            "which does not change how wide it is"
        );
    }

    #[test]
    fn a_pointer_is_as_wide_as_the_model_whatever_it_points_at() {
        let registry = lp64();
        let to_char = Type::primitive(Primitive::Char).pointer_to();
        let to_missing = named("NeverDefined").pointer_to();
        assert_eq!(registry.layout(&to_char).expect("laid out").size, 8);
        assert_eq!(
            registry.layout(&to_missing).expect("laid out").size,
            8,
            "what it points at is not laid out, which is what lets a list work"
        );
    }

    #[test]
    fn an_enumeration_is_as_wide_as_what_it_is_stored_in() {
        let mut registry = lp64();
        define(
            &mut registry,
            "enum Small : unsigned char { A, B };
             enum Plain { C, D };",
        );
        assert_eq!(registry.layout(&named("Small")).expect("laid out").size, 1);
        assert_eq!(registry.layout(&named("Plain")).expect("laid out").size, 4);
        assert_eq!(registry.constant("Plain", 1), Some("D"));
        assert_eq!(registry.constant("Plain", 7), None);
    }

    /// A registry saved and read again must be the same registry.
    #[test]
    fn a_registry_written_back_out_as_c_reads_the_same() {
        let mut registry = lp64();
        define(
            &mut registry,
            "struct Node { struct Node *next; char name[16][2]; unsigned int flag : 3; };
             union U { int i; float f; };
             enum Colour : unsigned short { Red = 1, Green = 2 };",
        );
        let source = registry.to_source();
        let mut again = lp64();
        define(&mut again, &source);
        assert_eq!(again.len(), registry.len());
        for definition in registry.all() {
            assert_eq!(
                again.get(definition.name()),
                Some(definition),
                "{} survived being written and read",
                definition.name()
            );
        }
    }

    /// A definition, where some of its members sit, and how much room it
    /// takes: name, members, size, alignment.
    type Expected = (&'static str, &'static [(&'static str, u64)], u64, u64);

    /// The whole point of the module, checked against the only authority
    /// there is.
    ///
    /// Every number below was printed by a program built with GCC on 64-bit
    /// Linux — `offsetof` and `sizeof` over the same declarations, compiled
    /// and run — rather than worked out by hand from the rules. A layout that
    /// agrees with the ABI in every case here is one a reader can trust
    /// against the binary in front of them.
    #[test]
    fn the_layout_is_the_one_a_c_compiler_produces() {
        let mut registry = lp64();
        define(
            &mut registry,
            "struct Mixed { char a; int b; char c; };
             struct Node { struct Node *next; char name[16]; unsigned long id; };
             struct Nested { char tag; struct Mixed inner; short trailing; };
             union Word { unsigned int number; unsigned char bytes[4]; double wide; };
             struct Flags {
                 unsigned int first : 1;
                 unsigned int second : 2;
                 unsigned int rest : 29;
                 unsigned int overflowing : 1;
             };
             struct Gap { unsigned int a : 1; unsigned int : 0; unsigned int b : 1; };
             struct Wide { char lead; long long value; short tail; };
             struct Arrays { short pairs[3][2]; char pad; };",
        );

        let expected: &[Expected] = &[
            ("Mixed", &[("a", 0), ("b", 4), ("c", 8)], 12, 4),
            ("Node", &[("next", 0), ("name", 8), ("id", 24)], 32, 8),
            (
                "Nested",
                &[("tag", 0), ("inner", 4), ("trailing", 16)],
                20,
                4,
            ),
            ("Word", &[("number", 0), ("bytes", 0), ("wide", 0)], 8, 8),
            ("Flags", &[], 8, 4),
            ("Gap", &[], 8, 4),
            ("Wide", &[("lead", 0), ("value", 8), ("tail", 16)], 24, 8),
            ("Arrays", &[("pairs", 0), ("pad", 12)], 14, 2),
        ];

        for (name, members, size, alignment) in expected {
            assert_eq!(
                registry.layout(&named(name)).expect("laid out"),
                Layout {
                    size: *size,
                    alignment: *alignment
                },
                "{name} is the size and alignment the compiler gave it"
            );
            let placed = offsets(&registry, name);
            for (member, offset) in *members {
                let found = placed
                    .iter()
                    .find(|(had, _)| had == member)
                    .unwrap_or_else(|| panic!("{name} has a member {member}"));
                assert_eq!(found.1, *offset, "{name}.{member}");
            }
        }
    }

    #[test]
    fn a_void_member_is_refused_and_a_pointer_to_void_is_not() {
        let registry = lp64();
        assert_eq!(
            registry.layout(&Type::primitive(Primitive::Void)),
            Err(LayoutError::Sizeless)
        );
        assert_eq!(
            registry
                .layout(&Type::primitive(Primitive::Void).pointer_to())
                .expect("laid out")
                .size,
            8
        );
    }
}
