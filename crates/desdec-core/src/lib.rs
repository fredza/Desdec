//! Stable, UI-independent primitives for inspecting executable files.

mod analysis;
mod binary;
mod bytes;
pub mod decompiler;
pub mod parallel;
pub mod patch;
pub mod yara;

pub use analysis::{
    ANALYSIS_BYTE_LIMIT, Analysis, BinaryDetails, Confidence, ExtractedString, FileKind, Hardening,
    Instruction, LanguageEvidence, LastWrite, Permissions, Relro, Section, Segment, SourceLanguage,
    StringEncoding, Symbol, Target, analyse_bytes, analyse_path, analyse_path_cancellable,
    decode_one, entropy, hash, language, operand, strings,
};
pub use binary::{Architecture, BinaryFormat, BinarySummary, Endianness, inspect_path};
pub use patch::{Patch, PatchError};
