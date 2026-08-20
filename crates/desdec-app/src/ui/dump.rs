//! The file's bytes, sixteen to a row.
//!
//! Every other view reads the bytes for the reader — as sections, as strings,
//! as instructions. This one shows them, because sometimes the reading is the
//! thing in question: a header field nothing decodes, the padding between two
//! functions, the table an operand points into.
//!
//! Both coordinates are on every row, and neither is invented. The offset is
//! where the byte is in the file; the address is where it is loaded, shown only
//! for the parts of the file that are mapped at all — a symbol table or a
//! debug section is in the file and nowhere in memory, and a made-up address
//! for it would be worse than none.

use eframe::egui;

use crate::{
    app::DesdecApp,
    i18n::{Language, Text, text},
    patches::Patches,
    ui::{MUTED, ROW_HEIGHT, decompile, syntax},
};

/// Bytes on one row. Sixteen is what a hexadecimal dump has had since the
/// first one, and what makes an offset's last digit the column number.
const WIDTH: usize = 16;

/// Bytes a pending patch would write, in the colour the listing marks them.
const PATCHED: egui::Color32 = egui::Color32::from_rgb(224, 164, 104);

/// What the reader is looking at, and where they asked to go.
#[derive(Default)]
pub struct State {
    /// The byte the view is centred on, as a file offset.
    pub offset: Option<u64>,
    /// The row to bring into view, once.
    pub pending_scroll: Option<u64>,
    /// What has been typed into the "go to" field.
    pub goto: String,
    /// Set when what was typed is neither an address nor an offset.
    pub goto_failed: bool,
}

pub fn show(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    let Some(analysis) = &app.analysis else {
        return;
    };
    if app.file_bytes.is_empty() {
        ui.label(egui::RichText::new(text(language, Text::NoDisassembly)).color(MUTED));
        return;
    }

    let mut go_to = None;
    ui.horizontal(|ui| {
        ui.label(text(language, Text::DumpGoTo));
        let field = ui.add(
            egui::TextEdit::singleline(&mut app.dump.goto)
                .font(egui::TextStyle::Monospace)
                .hint_text(text(language, Text::DumpGoToHint))
                .desired_width(200.0),
        );
        if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            go_to = Some(app.dump.goto.clone());
        }
        if let Some(offset) = app.dump.offset {
            ui.separator();
            ui.label(syntax::dim(
                ui,
                &format!("{offset:#010x}"),
                egui::Color32::TRANSPARENT,
            ));
            match analysis.address_at(offset) {
                Some((address, section)) => {
                    ui.label(syntax::dim(
                        ui,
                        &format!("{address:#018x}"),
                        egui::Color32::TRANSPARENT,
                    ));
                    ui.label(egui::RichText::new(&section.name).monospace().color(MUTED));
                }
                None => {
                    ui.label(
                        egui::RichText::new(text(language, Text::DumpUnmapped))
                            .small()
                            .color(MUTED),
                    );
                }
            }
        }
    });
    if app.dump.goto_failed {
        ui.colored_label(crate::ui::ERROR, text(language, Text::DumpNowhere));
    }
    ui.small(egui::RichText::new(text(language, Text::DumpHelp)).color(MUTED));
    ui.add_space(8.0);

    rows(app, ui);

    if let Some(typed) = go_to {
        let found = resolve(app, &typed);
        app.dump.goto_failed = !found;
    }
}

/// Reads what was typed as an address first, then as an offset.
///
/// An address first because that is what the rest of the interface deals in —
/// an operand's target, a symbol, a jump — and because a file offset that
/// happens to look like an address is the rarer accident of the two.
fn resolve(app: &mut DesdecApp, typed: &str) -> bool {
    let cleaned = typed
        .trim()
        .trim_start_matches("0x")
        .replace(['_', ' '], "");
    let Ok(value) = u64::from_str_radix(&cleaned, 16) else {
        return false;
    };
    let Some(analysis) = &app.analysis else {
        return false;
    };
    if let Some(offset) = analysis.file_offset_of(value) {
        app.show_bytes_at_offset(offset);
        return true;
    }
    if usize::try_from(value).is_ok_and(|offset| offset < app.file_bytes.len()) {
        app.show_bytes_at_offset(value);
        return true;
    }
    false
}

/// The rows themselves, virtualised: a large binary is a million of them.
fn rows(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    let Some(analysis) = &app.analysis else {
        return;
    };
    let file = &app.file_bytes;
    let total = file.len().div_ceil(WIDTH);
    let scroll_to = app
        .dump
        .pending_scroll
        .take()
        .map(|offset| usize::try_from(offset).unwrap_or(0) / WIDTH);
    let area =
        decompile::listing_area_at_row(egui::ScrollArea::both().id_salt("dump"), ui, scroll_to);
    let marked = app.dump.offset;
    // What a row asked for, gathered as the rows are drawn: the machine is the
    // application's, and cannot be reached while the file's bytes are borrowed.
    let mut watch = None;
    area.auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, total, |ui, visible| {
            egui::Grid::new("dump")
                .num_columns(4)
                .striped(true)
                .min_row_height(ROW_HEIGHT)
                .show(ui, |ui| {
                    for row in visible {
                        let offset = (row * WIDTH) as u64;
                        let end = ((row + 1) * WIDTH).min(file.len());
                        watch = watch.or(line(
                            ui,
                            analysis,
                            &app.patches,
                            &file[row * WIDTH..end],
                            offset,
                            marked,
                            language,
                        ));
                    }
                });
        });
    if let Some(address) = watch
        && let Some(machine) = app.machine()
    {
        machine.watch(desdec_core::emulate::Watchpoint {
            address,
            // One byte, which is enough: a watch is touched by any access that
            // overlaps it, and a wider read of the same word overlaps this one.
            size: 1,
            on_read: true,
            on_write: true,
        });
    }
}

/// One row: where it is in the file, where it is loaded, its bytes, its text.
///
/// Returns an address the reader asked to watch, if they asked.
fn line(
    ui: &mut egui::Ui,
    analysis: &desdec_core::Analysis,
    patches: &Patches,
    bytes: &[u8],
    offset: u64,
    marked: Option<u64>,
    language: Language,
) -> Option<u64> {
    ui.label(syntax::dim(
        ui,
        &format!("{offset:#010x}"),
        egui::Color32::TRANSPARENT,
    ));
    // Only a mapped row can be watched: a watchpoint is about an address the
    // run sees, and an unmapped part of the file has none.
    let mut watch = None;
    match analysis.address_at(offset) {
        Some((address, _)) => {
            ui.add(
                egui::Label::new(syntax::dim(
                    ui,
                    &format!("{address:#018x}"),
                    egui::Color32::TRANSPARENT,
                ))
                .sense(egui::Sense::click()),
            )
            .context_menu(|ui| {
                if ui.button(text(language, Text::WatchThisAddress)).clicked() {
                    watch = Some(address);
                    ui.close_menu();
                }
            });
        }
        None => {
            ui.label(
                egui::RichText::new(text(language, Text::DumpUnmapped))
                    .small()
                    .color(MUTED),
            );
        }
    }

    // The bytes, each with what a pending patch would put there instead.
    let mut hex = egui::text::LayoutJob::default();
    let mut ascii = String::with_capacity(bytes.len());
    for (index, byte) in bytes.iter().enumerate() {
        let at = offset.saturating_add(index as u64);
        let replacement = patches.byte_at_offset(at);
        let shown = replacement.unwrap_or(*byte);
        let colour = if replacement.is_some() {
            PATCHED
        } else if marked == Some(at) {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        hex.append(
            &format!("{shown:02x} "),
            0.0,
            egui::TextFormat {
                font_id: egui::TextStyle::Monospace.resolve(ui.style()),
                color: colour,
                background: if marked == Some(at) {
                    ui.visuals().selection.bg_fill
                } else {
                    egui::Color32::TRANSPARENT
                },
                ..egui::TextFormat::default()
            },
        );
        ascii.push(if shown.is_ascii_graphic() || shown == b' ' {
            char::from(shown)
        } else {
            '.'
        });
    }
    ui.label(hex);
    ui.label(egui::RichText::new(ascii).monospace().color(MUTED));
    ui.end_row();
    watch
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::WorkspaceView,
        commands::Command,
        testing::{drawn_text, opened_app, window_input},
    };
    use eframe::egui;

    /// The first rows of the file must be on screen, in both coordinates.
    #[test]
    fn the_dump_shows_the_bytes_it_has() {
        let mut app = opened_app(WorkspaceView::Dump);
        let bytes = crate::testing::reference_bytes();
        let ctx = egui::Context::default();

        let output = ctx.run(window_input(), |ctx| {
            crate::ui::views::show_central_panel(&mut app, ctx);
        });

        let drawn = drawn_text(&output.shapes);
        assert!(drawn.contains("0x00000000"), "the first offset");
        let mut first = String::new();
        for byte in bytes.iter().take(4) {
            use std::fmt::Write as _;
            let _ = write!(first, "{byte:02x} ");
        }
        assert!(drawn.contains(first.trim_end()), "and the bytes there");
    }

    /// Following an address is the whole point of the view: an operand points
    /// somewhere, and this is where the reader looks at what is there.
    #[test]
    fn an_address_is_followed_to_the_byte_it_names() {
        let analysis = crate::testing::reference_analysis();
        let Some(instruction) = analysis.instructions.first() else {
            return;
        };
        let address = instruction.address;
        let expected = analysis
            .file_offset_of(address)
            .expect("a decoded instruction is in the file");
        let mut app = opened_app(WorkspaceView::Disassembly);

        app.follow_in_dump(address);

        assert_eq!(app.dump.offset, Some(expected));
        assert_eq!(app.active_view, WorkspaceView::Dump);
    }

    /// What is typed is read as an address first and as an offset second, and
    /// anything else is refused rather than scrolled to somewhere arbitrary.
    #[test]
    fn the_go_to_field_reads_an_address_then_an_offset() {
        let analysis = crate::testing::reference_analysis();
        let Some(address) = analysis.instructions.first().map(|i| i.address) else {
            return;
        };
        let mut app = opened_app(WorkspaceView::Dump);

        assert!(resolve(&mut app, &format!("{address:#x}")));
        assert_eq!(app.dump.offset, analysis.file_offset_of(address));

        assert!(resolve(&mut app, "0x40"), "an offset into the file");
        assert_eq!(app.dump.offset, Some(0x40));

        assert!(!resolve(&mut app, "nonsense"));
        assert!(!resolve(&mut app, "0xffffffff0000"), "nowhere in the file");
    }

    /// The view is reachable the way every other one is.
    #[test]
    fn the_command_opens_the_view() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Overview);

        app.run_command(&ctx, Command::Dump);

        assert_eq!(app.active_view, WorkspaceView::Dump);
    }
}
