//! Stable, UI-independent primitives for inspecting executable files.

mod analysis;
mod binary;
mod bytes;

pub use analysis::{
    ANALYSIS_BYTE_LIMIT, Analysis, BinaryDetails, ExtractedString, FileKind, Hardening,
    Permissions, Relro, Section, Segment, StringEncoding, analyse_bytes, analyse_path, entropy,
    hash, strings,
};
pub use binary::{Architecture, BinaryFormat, BinarySummary, Endianness, inspect_path};
