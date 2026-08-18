//! Stable, UI-independent primitives for inspecting executable files.

mod analysis;
/// Optional AI assistance; see the module's own documentation.
pub mod assistant;
mod binary;
mod bytes;
pub mod decompiler;
/// Synthetic binaries for tests; see the module's own documentation.
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
pub mod parallel;
pub mod patch;
pub mod yara;

pub use analysis::{
    ANALYSIS_BYTE_LIMIT, Analysis, BinaryDetails, Confidence, Decoded, ExtractedString, FileKind,
    Hardening, Instruction, InstructionBytes, LanguageEvidence, LastWrite, Permissions, Relro,
    Section, Segment, SourceLanguage, StackSlot, StackState, StringEncoding, Symbol, Target, Trace,
    analyse_bytes, analyse_path, analyse_path_cancellable, decode_one, entropy, hash, language,
    operand, stack, strings,
};
pub use binary::{Architecture, BinaryFormat, BinarySummary, Endianness, inspect_path};
pub use patch::{Patch, PatchError};
