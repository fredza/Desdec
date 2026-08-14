//! Stable, UI-independent primitives for inspecting executable files.

mod binary;

pub use binary::{Architecture, BinaryFormat, BinarySummary, Endianness, inspect_path};
