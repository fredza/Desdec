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
/// Writing a decoded listing back out as assembler source; see the module's
/// own documentation.
pub mod export;
/// Synthetic binaries for tests; see the module's own documentation.
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
pub mod parallel;
pub mod patch;
/// Types the reader names, and where their members sit in the bytes; see the
/// module's own documentation.
pub mod types;
/// Asking GitHub whether there is a newer release; see the module's own
/// documentation.
pub mod update;
pub mod yara;

pub use analysis::{
    ANALYSIS_BYTE_LIMIT, AnalysedFile, Analysis, BinaryDetails, Class, ClassMethod, ClassSource,
    Confidence, Decoded, ExtractedString, FileKind, Hardening, ImportSlot, ImportedLibrary,
    Instruction, InstructionBytes, LanguageEvidence, LastWrite, NetworkName, NetworkUse,
    Permissions, Protection, ProtectionKind, Reach, Relro, Section, Segment, SourceLanguage,
    StackSlot, StackState, StringEncoding, Symbol, SymbolKind, Target, Trace, analyse_bytes,
    analyse_path, analyse_path_cancellable, analyse_path_with_bytes_cancellable, blocks,
    decode_one, demangle, discover, entropy, flags, flow, hash, imports, language, network,
    operand, protection, stack, strings,
};
pub use binary::{Architecture, BinaryFormat, BinarySummary, Endianness, inspect_path};
pub use patch::{Patch, PatchError};
