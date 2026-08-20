//! Stable, UI-independent primitives for inspecting executable files.

mod analysis;
/// Turning a typed instruction back into bytes; see the module's own
/// documentation.
pub mod assemble;
/// Optional AI assistance; see the module's own documentation.
pub mod assistant;
mod binary;
mod bytes;
pub mod decompiler;
/// Running a binary on a processor Desdec builds; see the module's own
/// documentation.
pub mod emulate;
/// Synthetic binaries for tests; see the module's own documentation.
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
pub mod parallel;
pub mod patch;
/// Asking GitHub whether there is a newer release; see the module's own
/// documentation.
pub mod update;
pub mod yara;

pub use analysis::{
    ANALYSIS_BYTE_LIMIT, AnalysedFile, Analysis, BinaryDetails, Confidence, Decoded,
    ExtractedString, FileKind, Hardening, ImportedLibrary, Instruction, InstructionBytes,
    LanguageEvidence, LastWrite, Permissions, Relro, Section, Segment, SourceLanguage, StackSlot,
    StackState, StringEncoding, Symbol, Target, Trace, analyse_bytes, analyse_path,
    analyse_path_cancellable, analyse_path_with_bytes_cancellable, decode_one, discover, entropy,
    flags, hash, language, operand, stack, strings,
};
pub use binary::{Architecture, BinaryFormat, BinarySummary, Endianness, inspect_path};
pub use patch::{Patch, PatchError};
