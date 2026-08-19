//! Byte-level patches: the editor, the pending list, and the export.
//!
//! Editing is deliberately at the byte level. The decoder shows what the typed
//! bytes became, so a patch is judged on what the processor will read rather
//! than on what the editor meant.

use desdec_core::Architecture;
use eframe::egui;

use crate::{
    app::DesdecApp,
    i18n::{Language, Text, text},
    patches::{Assembled, Editor, Preview},
    ui::{ERROR, MUTED, card, syntax},
};

pub fn show(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let Some(architecture) = app
        .analysis
        .as_ref()
        .map(|analysis| analysis.summary.architecture)
    else {
        return;
    };

    if app.patch_editor.is_some() {
        editor(app, ui, architecture);
        ui.add_space(12.0);
    }
    pending(app, ui);
}

/// The instruction being edited, and what its bytes currently mean.
fn editor(app: &mut DesdecApp, ui: &mut egui::Ui, architecture: Architecture) {
    let language = app.preferences.language;
    let mut close = false;
    let mut record = None;

    card(ui, app.t(Text::EditingInstruction), |ui| {
        let Some(editor) = app.patch_editor.as_mut() else {
            return;
        };
        ui.label(
            egui::RichText::new(format!("{:#018x}", editor.address))
                .monospace()
                .color(MUTED),
        );
        ui.add_space(4.0);
        ui.small(text(language, Text::PatchLengthRule));
        ui.add_space(8.0);

        // The assembler first: it is what a reader reaches for, and what it
        // produces lands in the byte field underneath, where it can still be
        // corrected by hand.
        ui.horizontal(|ui| {
            ui.label(text(language, Text::PatchAssembly));
            ui.add(
                egui::TextEdit::singleline(&mut editor.assembly)
                    .font(egui::TextStyle::Monospace)
                    .hint_text(text(language, Text::PatchAssemblyHint))
                    .desired_width(240.0),
            );
        });
        let assembled = editor.take_assembled(architecture);
        assembled_row(ui, &assembled, language);
        ui.small(egui::RichText::new(text(language, Text::PatchAssemblyHelp)).color(MUTED));
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label(text(language, Text::PatchBytes));
            ui.add(
                egui::TextEdit::singleline(&mut editor.input)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(240.0),
            );
            ui.label(
                egui::RichText::new(format!(
                    "{} {}",
                    text(language, Text::PatchLengthMismatch),
                    editor.original.len()
                ))
                .color(MUTED),
            );
        });

        ui.add_space(8.0);
        let preview = editor.preview(architecture);
        preview_row(ui, &preview, language);

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let valid = matches!(preview, Preview::Decoded(_) | Preview::NotAnInstruction);
            if ui
                .add_enabled(valid, egui::Button::new(text(language, Text::ApplyPatch)))
                .clicked()
            {
                record = editor.to_patch().ok();
                close = true;
            }
            if ui.button(text(language, Text::RevertPatch)).clicked() {
                editor.reset();
            }
            if ui.button(text(language, Text::CancelEdit)).clicked() {
                close = true;
            }
        });
    });

    if let Some(patch) = record {
        app.note(
            crate::journal::Level::Note,
            format!(
                "{} {:#x} · {} {}",
                text(language, Text::JournalPatchRecorded),
                patch.address,
                patch.replacement.len(),
                text(language, Text::JournalBytes)
            ),
        );
        app.patches.set(patch);
    }
    if close {
        app.patch_editor = None;
    }
}

/// What the typed assembly encoded to, or why it could not be used.
///
/// Said in full: the bytes it produced, the `nop`s that filled the room it did
/// not need, or the reason the line was refused. A patch is written to a file
/// someone else will run, and none of that is a detail.
fn assembled_row(ui: &mut egui::Ui, assembled: &Assembled, language: Language) {
    match assembled {
        Assembled::Fits { bytes, padding } => {
            ui.horizontal(|ui| {
                ui.label(syntax::dim(ui, &to_hex(bytes), egui::Color32::TRANSPARENT));
                if *padding > 0 {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {padding} {}",
                            text(language, Text::PatchPadded),
                            text(language, Text::PatchNops)
                        ))
                        .small()
                        .color(MUTED),
                    );
                }
            });
        }
        Assembled::TooLong { encoded, room } => {
            ui.colored_label(
                ERROR,
                format!(
                    "{} {encoded} {} {room}",
                    text(language, Text::PatchTooLong),
                    text(language, Text::PatchRoom)
                ),
            );
        }
        Assembled::Refused(error) => {
            ui.colored_label(ERROR, crate::patches::refusal(error, language));
        }
        // Nothing typed is not a failure; the byte field is the way in.
        Assembled::Empty => {
            ui.label("");
        }
    }
}

/// A run of bytes as the editor writes them.
fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the typed bytes decode to, or why they cannot be used.
fn preview_row(ui: &mut egui::Ui, preview: &Preview, language: Language) {
    match preview {
        Preview::Decoded(instruction) => {
            ui.horizontal(|ui| {
                ui.label(text(language, Text::PatchBecomes));
                ui.label(syntax::assembly(
                    ui,
                    &instruction.text,
                    egui::Color32::TRANSPARENT,
                ));
            });
        }
        // Not an error: data can be patched too, but it must not look like a
        // decoded instruction.
        Preview::NotAnInstruction => {
            ui.label(egui::RichText::new(text(language, Text::PatchNotAnInstruction)).color(MUTED));
        }
        Preview::LengthChanged { expected, found } => {
            ui.colored_label(
                ERROR,
                format!(
                    "{} {expected} ({found})",
                    text(language, Text::PatchLengthMismatch)
                ),
            );
        }
        Preview::Invalid(reason) => {
            ui.colored_label(ERROR, reason);
        }
    }
}

/// The patches waiting to be exported.
fn pending(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    let mut remove = None;
    let mut discard = false;
    let mut export = false;

    let title = if app.patches.is_empty() {
        app.t(Text::PendingPatches).to_owned()
    } else {
        format!("{} ({})", app.t(Text::PendingPatches), app.patches.len())
    };
    card(ui, &title, |ui| {
        ui.small(text(language, Text::ExportInfo));
        ui.add_space(8.0);

        if app.patches.is_empty() {
            ui.label(egui::RichText::new(text(language, Text::NoPatches)).color(MUTED));
            return;
        }

        egui::Grid::new("pending_patches")
            .num_columns(4)
            .striped(true)
            .spacing([18.0, 6.0])
            .show(ui, |ui| {
                for header in [Text::Address, Text::OriginalBytes, Text::PatchedColumn] {
                    ui.strong(text(language, header));
                }
                ui.end_row();

                for patch in app.patches.entries() {
                    ui.label(syntax::dim(
                        ui,
                        &format!("{:#018x}", patch.address),
                        egui::Color32::TRANSPARENT,
                    ));
                    ui.label(syntax::dim(
                        ui,
                        &hex(&patch.original),
                        egui::Color32::TRANSPARENT,
                    ));
                    ui.monospace(hex(&patch.replacement));
                    if ui.button(text(language, Text::RevertPatch)).clicked() {
                        remove = Some(patch.address);
                    }
                    ui.end_row();
                }
            });

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button(text(language, Text::ExportPatched)).clicked() {
                export = true;
            }
            if ui.button(text(language, Text::DiscardPatches)).clicked() {
                discard = true;
            }
        });

        if let Some(report) = &app.export_report {
            ui.add_space(8.0);
            match report {
                Ok(path) => {
                    let path_text = path.display().to_string();
                    ui.horizontal(|ui| {
                        ui.label(text(language, Text::ExportSucceeded));
                        let path_label = ui
                            .add(
                                egui::Label::new(egui::RichText::new(&path_text).monospace())
                                    .sense(egui::Sense::click()),
                            )
                            .on_hover_text(&path_text)
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        if path_label.clicked()
                            || ui.button(text(language, Text::CopyPath)).clicked()
                        {
                            ui.ctx().copy_text(path_text.clone());
                        }
                    });
                }
                Err(error) => {
                    ui.colored_label(
                        ERROR,
                        format!("{} {error}", text(language, Text::ExportFailed)),
                    );
                }
            }
        }
    });

    if let Some(address) = remove {
        app.patches.remove(address);
    }
    if discard {
        app.patches.clear();
        app.export_report = None;
    }
    if export {
        app.export_patched_copy(ui.ctx());
    }
}

/// Opens the editor on an instruction, when its bytes exist in the file.
///
/// Returns `false` for an instruction that occupies no file bytes, which is
/// nothing to write a patch into.
pub fn open_editor(app: &mut DesdecApp, address: u64) -> bool {
    let Some(analysis) = app.analysis.as_ref() else {
        return false;
    };
    let Some(instruction) = analysis
        .instructions
        .iter()
        .find(|instruction| instruction.address == address)
    else {
        return false;
    };
    let Some(file_offset) = crate::patches::file_offset_of(analysis, address) else {
        return false;
    };

    // Reopening on an already patched instruction shows the edit in progress,
    // not the original bytes: the user would otherwise silently lose it.
    let mut editor = Editor::new(instruction, file_offset);
    if let Some(patch) = app.patches.patch_at(address) {
        editor.input = hex(&patch.replacement);
    }
    app.patch_editor = Some(editor);
    true
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::WorkspaceView;

    /// A real analysis of a real binary: the test executable itself.
    fn opened() -> DesdecApp {
        crate::testing::opened_app(WorkspaceView::Patches)
    }

    /// First instruction whose bytes actually exist in the file.
    ///
    /// Every format the tests run on reaches one: the section tables are read
    /// for ELF, PE and Mach-O alike, so this is demanded rather than skipped.
    fn first_patchable(app: &DesdecApp) -> u64 {
        let analysis = app.analysis.as_ref().expect("a binary is open");
        analysis
            .instructions
            .iter()
            .map(|instruction| instruction.address)
            .find(|address| crate::patches::file_offset_of(analysis, *address).is_some())
            .expect("the host binary must decode a patchable instruction")
    }

    /// An instruction's address becomes a place in the file differently in
    /// each format — sections are padded differently on disk, and a Mach-O
    /// segment is not a PE section. Patching the wrong offset would write
    /// plausible bytes into the wrong part of the file, so the mapping is held
    /// to the bytes it lands on, for every format.
    #[test]
    fn an_instruction_maps_back_to_its_own_bytes_in_every_format() {
        for sample in crate::testing::samples() {
            let label = sample.fixture.label;
            let analysis = &sample.analysis;
            let file = &sample.fixture.bytes;
            let mut checked = 0;

            for instruction in &analysis.instructions {
                let Some(offset) = crate::patches::file_offset_of(analysis, instruction.address)
                else {
                    continue;
                };
                let at = usize::try_from(offset).expect("fixtures stay small");
                let found = file
                    .get(at..at + instruction.bytes.len())
                    .unwrap_or_else(|| panic!("{label}: {offset:#x} is outside the file"));
                assert_eq!(
                    found,
                    instruction.bytes.as_slice(),
                    "{label}: {:#x} maps to {offset:#x}, which holds other bytes",
                    instruction.address
                );
                checked += 1;
            }
            assert!(checked > 0, "{label}: no instruction was patchable at all");
        }
    }

    /// The whole path: open an editor, type new bytes, record the patch.
    #[test]
    fn editing_an_instruction_records_a_patch_of_the_same_length() {
        let mut app = opened();
        let address = first_patchable(&app);

        assert!(open_editor(&mut app, address));
        let editor = app.patch_editor.as_mut().expect("the editor is open");
        let length = editor.original.len();
        editor.input = "90 ".repeat(length);

        let patch = editor.to_patch().expect("same-length bytes are patchable");
        app.patches.set(patch);

        assert_eq!(app.patches.len(), 1);
        assert_eq!(app.patches.entries()[0].replacement, vec![0x90; length]);
        assert_eq!(app.patches.entries()[0].address, address);
    }

    /// The rule that protects every offset in the file.
    #[test]
    fn a_longer_replacement_is_refused_by_the_editor() {
        let mut app = opened();
        let address = first_patchable(&app);
        assert!(open_editor(&mut app, address));

        let editor = app.patch_editor.as_mut().expect("the editor is open");
        editor.input = "90 ".repeat(editor.original.len() + 1);

        assert!(matches!(
            editor.to_patch(),
            Err(Preview::LengthChanged { .. })
        ));
    }

    /// Reopening must show the edit in progress, not the file's bytes, or the
    /// pending patch would be silently overwritten.
    #[test]
    fn reopening_a_patched_instruction_shows_the_pending_bytes() {
        let mut app = opened();
        let address = first_patchable(&app);

        assert!(open_editor(&mut app, address));
        let editor = app.patch_editor.as_mut().expect("the editor is open");
        let length = editor.original.len();
        editor.input = "90 ".repeat(length);
        let patch = editor.to_patch().expect("same-length bytes are patchable");
        app.patches.set(patch);
        app.patch_editor = None;

        assert!(open_editor(&mut app, address));
        let reopened = app.patch_editor.as_ref().expect("the editor is open again");
        assert_eq!(reopened.bytes(), Ok(vec![0x90; length]));
    }

    /// Patches describe one file at one set of offsets; carrying them to the
    /// next binary would write bytes at positions that mean something else.
    #[test]
    fn closing_the_binary_discards_its_patches() {
        let mut app = opened();
        let address = first_patchable(&app);
        assert!(open_editor(&mut app, address));
        let editor = app.patch_editor.as_mut().expect("the editor is open");
        editor.input = "90 ".repeat(editor.original.len());
        let patch = editor.to_patch().expect("same-length bytes are patchable");
        app.patches.set(patch);

        app.close_binary();

        assert!(app.patches.is_empty());
        assert!(app.patch_editor.is_none());
    }

    /// The export writes a copy that really differs by exactly the patched
    /// bytes, and leaves the analysed file untouched.
    #[test]
    fn the_exported_copy_differs_only_where_it_was_patched() {
        let source = crate::testing::reference_path();
        let original = crate::testing::reference_bytes().to_vec();
        let mut app = opened();
        let address = first_patchable(&app);
        assert!(open_editor(&mut app, address));
        let editor = app.patch_editor.as_mut().expect("the editor is open");
        let length = editor.original.len();
        editor.input = "90 ".repeat(length);
        let patch = editor.to_patch().expect("same-length bytes are patchable");
        let offset = usize::try_from(patch.file_offset).expect("the offset fits in memory");
        app.patches.set(patch);

        let destination = std::env::temp_dir().join("desdec-export-roundtrip.bin");
        let written =
            desdec_core::patch::write_patched_copy(source, &destination, app.patches.entries())
                .expect("the copy is writable");
        let copy = std::fs::read(&destination).expect("the copy is readable");

        assert_eq!(written, original.len() as u64, "the size never changes");
        assert_eq!(copy[offset..offset + length], vec![0x90; length]);
        let differing: Vec<usize> = (0..original.len())
            .filter(|index| original[*index] != copy[*index])
            .collect();
        assert!(
            differing
                .iter()
                .all(|index| (offset..offset + length).contains(index)),
            "only the patched bytes may differ"
        );
        assert_eq!(
            std::fs::read(source).expect("the source is readable"),
            original,
            "the analysed file must be untouched"
        );
        let _ = std::fs::remove_file(&destination);
    }

    /// The view must lay out with an editor open, which the whole-application
    /// layout test cannot reach on its own.
    #[test]
    fn the_view_lays_out_with_an_open_editor_and_a_pending_patch() {
        let ctx = egui::Context::default();
        let mut app = opened();
        let address = first_patchable(&app);
        assert!(open_editor(&mut app, address));
        let editor = app.patch_editor.as_mut().expect("the editor is open");
        editor.input = "90 ".repeat(editor.original.len());
        let patch = editor.to_patch().expect("same-length bytes are patchable");
        app.patches.set(patch);

        for language in crate::i18n::Language::ALL {
            app.preferences.language = *language;
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                crate::ui::views::show_central_panel(&mut app, ctx);
            });
            assert!(!output.shapes.is_empty(), "the patches view drew nothing");
        }
    }
}
