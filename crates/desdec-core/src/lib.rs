//! Stable, UI-independent primitives for inspecting executable files.

mod analysis;
mod binary;
mod bytes;
pub mod decompiler;
pub mod patch;

pub use analysis::{
    ANALYSIS_BYTE_LIMIT, Analysis, BinaryDetails, ExtractedString, FileKind, Hardening,
    Instruction, Permissions, Relro, Section, Segment, StringEncoding, Symbol, analyse_bytes,
    analyse_path, decode_one, entropy, hash, strings,
};
pub use binary::{Architecture, BinaryFormat, BinarySummary, Endianness, inspect_path};
pub use patch::{Patch, PatchError};
