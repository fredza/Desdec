//! Stable, process-isolated binary analysis report.
//!
//! This crate deliberately talks JSON rather than exposing a Rust ABI.  A
//! caller can be a Rust application, a C/C++ program, a script, or a program
//! on another machine: it only needs to spawn `desdec-analyzer` and parse
//! stdout.  No third-party code is loaded into the caller's process.

use std::path::Path;

use desdec_core::{Analysis, BinaryFormat};
use serde_json::{Value, json};

/// Version of the JSON contract.  Consumers must reject a newer major version
/// rather than silently treating a changed field as the old one.
pub const PROTOCOL_VERSION: u32 = 1;

/// Shapes which potentially large collections are included in the report.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReportOptions {
    /// Include the full decoded instruction listing.  It is deliberately
    /// opt-in: binaries can contain millions of instructions, while the rest
    /// of the report stays compact enough for an interactive caller.
    pub instructions: bool,
}

/// Build a JSON report from an already analysed binary.
///
/// All values come from `desdec-core`, which bounds file reads and malformed
/// tables.  Arrays which are absent are represented by `[]`, never `null`;
/// a field whose format cannot answer the question is `null`.
#[must_use]
pub fn report(path: &Path, analysis: &Analysis, options: ReportOptions) -> Value {
    let format = match analysis.summary.format {
        BinaryFormat::Elf { bits, endianness } => json!({
            "name": "ELF", "bits": bits, "endianness": endianness.label(),
        }),
        BinaryFormat::Pe => json!({"name": "PE"}),
        BinaryFormat::MachO { bits, endianness } => json!({
            "name": "Mach-O", "bits": bits, "endianness": endianness.label(),
        }),
        BinaryFormat::Unknown => json!({"name": "unknown"}),
    };
    let hardening = &analysis.details.hardening;
    let instructions = options.instructions.then(|| {
        analysis
            .instructions
            .iter()
            .map(|instruction| {
                json!({
                    "address": hex(instruction.address),
                    "bytes": hex_bytes(instruction.bytes.as_slice()),
                    "text": instruction.text,
                    "section": instruction.section.as_ref(),
                })
            })
            .collect::<Vec<_>>()
    });

    json!({
        "protocol_version": PROTOCOL_VERSION,
        "tool": {"name": "desdec-analyzer", "version": env!("CARGO_PKG_VERSION")},
        "input": {
            "path": path,
            "size": analysis.summary.size,
            "format": format,
            "architecture": analysis.summary.architecture.label(),
            "sha256": analysis.sha256.map(hex_bytes),
            "analysed_bytes": analysis.analysed_bytes,
            "truncated": analysis.truncated,
        },
        "overview": {
            "file_kind": analysis.details.file_kind.label(),
            "bits": analysis.details.bits,
            "endianness": analysis.details.endianness.label(),
            "entry_point": analysis.entry_point.map(hex),
            "entropy": analysis.entropy,
            "suggests_packing": analysis.suggests_packing(),
            "instruction_count": analysis.instructions.len(),
            "instruction_listing_included": options.instructions,
            "instruction_listing_truncated": analysis.code_truncated,
        },
        "hardening": {
            "position_independent": hardening.position_independent,
            "non_executable_stack": hardening.non_executable_stack,
            "relro": hardening.relro.map(|value| value.label()),
            "stack_canary": hardening.stack_canary,
            "address_space_randomisation": hardening.address_space_randomisation,
            "data_execution_prevention": hardening.data_execution_prevention,
            "control_flow_guard": hardening.control_flow_guard,
            "signed": hardening.signed,
        },
        "mapping": {
            "sections": analysis.sections.iter().map(|section| json!({
                "name": section.name,
                "virtual_address": hex(section.virtual_address),
                "virtual_size": section.virtual_size,
                "file_offset": section.file_offset,
                "file_size": section.file_size,
                "permissions": section.permissions.label(),
                "entropy": section.entropy,
            })).collect::<Vec<_>>(),
            "segments": analysis.details.segments.iter().map(|segment| json!({
                "kind": segment.kind,
                "virtual_address": hex(segment.virtual_address),
                "virtual_size": segment.virtual_size,
                "file_offset": segment.file_offset,
                "file_size": segment.file_size,
                "permissions": segment.permissions.label(),
            })).collect::<Vec<_>>(),
        },
        "loader": {
            "interpreter": analysis.details.interpreter,
            "subsystem": analysis.details.subsystem,
            "timestamp": analysis.details.timestamp,
            "linked_libraries": analysis.details.linked_libraries,
            "imports": analysis.details.imports.iter().map(|library| json!({
                "library": library.library,
                "functions": library.functions,
                "truncated": library.truncated,
            })).collect::<Vec<_>>(),
            "import_slots": analysis.import_slots.iter().map(|slot| json!({
                "address": hex(slot.address), "name": slot.name,
            })).collect::<Vec<_>>(),
        },
        "symbols": analysis.symbols.iter().map(|symbol| json!({
            "name": symbol.name,
            "address": symbol.address.map(hex),
            "size": symbol.size,
            "imported": symbol.imported,
        })).collect::<Vec<_>>(),
        "classes": analysis.classes.iter().map(|class| json!({
            "name": class.name,
            "source": format!("{:?}", class.source),
            "vtable": class.vtable.map(hex),
            "typeinfo": class.typeinfo.map(hex),
            "methods": class.methods.iter().map(|method| json!({
                "name": method.name,
                "mangled": method.mangled,
                "address": method.address.map(hex),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "strings": analysis.strings.iter().map(|string| json!({
            "file_offset": string.file_offset,
            "encoding": string.encoding.label(),
            "value": string.value,
            "truncated": string.truncated,
        })).collect::<Vec<_>>(),
        "source_languages": analysis.languages.iter().map(|evidence| json!({
            "language": evidence.language.label(),
            "confidence": format!("{:?}", evidence.confidence).to_lowercase(),
            "evidence": evidence.evidence,
            "toolchain": evidence.toolchain,
        })).collect::<Vec<_>>(),
        "network": {
            "names": analysis.network.names.iter().map(|name| json!({
                "name": name.name,
                "reach": format!("{:?}", name.reach).to_lowercase(),
            })).collect::<Vec<_>>(),
            "libraries": analysis.network.libraries,
            "can_send": analysis.network.sends(),
            "can_receive": analysis.network.receives(),
        },
        "instructions": instructions,
    })
}

fn hex(value: u64) -> String {
    format!("0x{value:X}")
}

fn hex_bytes(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_has_a_versioned_contract() {
        let analysis = desdec_core::analyse_path(std::env::current_exe().expect("test executable"))
            .expect("test executable is analysable");
        let report = report(Path::new("program"), &analysis, ReportOptions::default());
        assert_eq!(report["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(report["input"]["path"], "program");
        assert!(report["mapping"]["sections"].is_array());
        assert!(report["instructions"].is_null());
    }
}
