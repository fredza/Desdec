//! Panels, dialogs and views. Every module here draws from `&mut DesdecApp`
//! and owns no state of its own.

use eframe::egui;

pub mod about;
pub mod action_bar;
pub mod annotation;
pub mod assistant;
pub mod decompile;
pub mod disassembly;
pub mod dump;
pub mod expert;
pub mod functions;
pub mod library_note;
pub mod navigation;
pub mod notice;
pub mod operand_note;
pub mod output;
pub mod palette;
pub mod patches_view;
pub mod plugins;
pub mod preferences_window;
pub mod references;
pub mod script;
pub mod search;
pub mod segments;
pub mod status_bar;
pub mod strings;
pub mod syntax;
pub mod views;
pub mod yara;

/// Secondary text: readable, never competing with the content.
pub const MUTED: egui::Color32 = egui::Color32::from_rgb(145, 155, 178);

/// Reserved for messages the user must not miss.
pub const ERROR: egui::Color32 = egui::Color32::from_rgb(232, 119, 91);

/// Section heading of a side panel: small, muted and uppercase.
pub fn section_title(label: &str) -> egui::RichText {
    egui::RichText::new(label).color(MUTED).small().strong()
}

/// Below this width, side-by-side columns would be too narrow to read, so the
/// layout falls back to a single column.
pub const TWO_COLUMN_WIDTH: f32 = 900.0;

/// How far each window steps aside from the one under it: enough that the
/// title bar and the close button of the window underneath stay reachable.
const CASCADE_STEP: f32 = 34.0;

/// Where a window opens: in the middle, then stepped aside once for every
/// window already on screen.
///
/// Windows egui is not told where to put land in the same corner, so a second
/// one covers the first exactly and the reader is left with a stack they
/// cannot see into. `step` is how many were already open, from
/// [`crate::app::Dialogs::opening_step`].
///
/// The size comes from the last time the window was laid out; the first
/// opening has none, so `assumed` stands in and is corrected on the frame
/// after — a window has to be measured before it can be centred.
pub fn opening_position(
    ctx: &egui::Context,
    id: egui::Id,
    step: usize,
    assumed: egui::Vec2,
) -> egui::Pos2 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "the step counts open windows, of which there are six"
    )]
    let offset = CASCADE_STEP * step as f32;
    let size = ctx
        .memory(|memory| memory.area_rect(id))
        .map_or(assumed, |rect| rect.size());
    let screen = ctx.screen_rect();
    let centred = screen.center() - size / 2.0;
    // Stepping a window off the screen would be worse than covering another,
    // so the last of a long cascade is brought back inside.
    egui::pos2(
        (centred.x + offset)
            .min(screen.right() - size.x)
            .max(screen.left()),
        (centred.y + offset)
            .min(screen.bottom() - size.y)
            .max(screen.top()),
    )
}

/// Height of one row in the long monospace listings.
///
/// Every such listing is virtualised — a decoded binary reaches a hundred
/// thousand instructions, and laying out a widget for each one takes seconds
/// per frame — and a virtualiser needs to know a row's height before drawing
/// it, so the listings are given this one rather than measuring their own.
pub const ROW_HEIGHT: f32 = 18.0;

/// A titled frame that fills the width it is given, so panels line up instead
/// of each shrinking to its own content.
pub fn card(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui)) {
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.vertical(|ui| {
            ui.strong(title);
            ui.add_space(8.0);
            contents(ui);
        });
    });
}

/// A two-column row that collapses to one column on a narrow window.
///
/// Both closures always run: each column owns its own `Ui`, so the caller does
/// not have to know which layout was chosen.
pub fn columns(
    ui: &mut egui::Ui,
    left: impl FnOnce(&mut egui::Ui),
    right: impl FnOnce(&mut egui::Ui),
) {
    if ui.available_width() < TWO_COLUMN_WIDTH {
        left(ui);
        ui.add_space(12.0);
        right(ui);
        return;
    }

    ui.columns(2, |columns| {
        left(&mut columns[0]);
        right(&mut columns[1]);
    });
}

/// Human-readable file size, in the binary units used by the rest of the tool.
#[must_use]
pub fn format_size(size: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;

    #[expect(
        clippy::cast_precision_loss,
        reason = "sizes are displayed with two decimals, far below f64 precision limits"
    )]
    if size >= MIB {
        format!("{:.2} MiB", size as f64 / MIB as f64)
    } else if size >= KIB {
        format!("{:.2} KiB", size as f64 / KIB as f64)
    } else {
        format!("{size} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{DesdecApp, WorkspaceView};

    /// A frame of a given window width, so both the wide and the narrow
    /// layouts are exercised rather than whichever egui defaults to.
    fn input_of_width(width: f32) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 800.0),
            )),
            ..Default::default()
        }
    }

    /// Widths either side of the point where the expert layout switches
    /// between one and two columns.
    fn layout_widths() -> [f32; 2] {
        [TWO_COLUMN_WIDTH - 100.0, TWO_COLUMN_WIDTH + 400.0]
    }

    /// Lays out one full frame of every view, against a real analysis.
    ///
    /// This runs the actual layout code — grids, virtualised row ranges,
    /// formatting — without opening a window, so a panic in a view is caught
    /// here rather than by whoever opens a binary.
    #[test]
    fn every_view_lays_out_without_panicking() {
        let ctx = egui::Context::default();
        // One application drawn in every combination rather than one per
        // combination: building it analyses a binary and indexes its
        // functions, and no view here leaves state the next one would read.
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        for view in WorkspaceView::ALL {
            for language in crate::i18n::Language::ALL {
                app.select_view(*view);
                app.preferences.language = *language;

                for width in layout_widths() {
                    let output = ctx.run(input_of_width(width), |ctx| {
                        views::show_central_panel(&mut app, ctx);
                        status_bar::show(&mut app, ctx);
                    });
                    assert!(
                        !output.shapes.is_empty(),
                        "{view:?} drew nothing at width {width}"
                    );
                }
            }
        }
    }

    /// The same, for a binary of each format the reader supports.
    ///
    /// The test executable above is one format and one architecture — the
    /// host's. These are an ELF, a PE and an `AArch64` Mach-O, so a view that
    /// draws nothing for a format the host does not use is caught here.
    #[test]
    fn every_view_lays_out_for_every_format() {
        let ctx = egui::Context::default();
        for sample in crate::testing::samples() {
            let label = sample.fixture.label;
            let mut app = sample.opened(WorkspaceView::Overview);
            for view in WorkspaceView::ALL {
                app.select_view(*view);
                for width in layout_widths() {
                    let output = ctx.run(input_of_width(width), |ctx| {
                        views::show_central_panel(&mut app, ctx);
                        status_bar::show(&mut app, ctx);
                    });
                    assert!(
                        !output.shapes.is_empty(),
                        "{label}: {view:?} drew nothing at width {width}"
                    );
                }
            }
        }
    }

    /// What each fixture was built to contain has to reach the screen: the
    /// names of its functions, its strings, and the library it links against.
    /// A reader returning nothing for a format would otherwise be
    /// indistinguishable from a binary that says nothing.
    #[test]
    fn every_format_shows_its_functions_strings_and_libraries() {
        let ctx = egui::Context::default();
        for sample in crate::testing::samples() {
            let label = sample.fixture.label;
            let mut app = sample.opened(WorkspaceView::Overview);

            let shown = |app: &mut DesdecApp| {
                let output = ctx.run(input_of_width(1200.0), |ctx| {
                    views::show_central_panel(app, ctx);
                });
                crate::testing::drawn_text(&output.shapes)
            };

            app.select_view(WorkspaceView::Functions);
            let functions = shown(&mut app);
            for (name, _) in &sample.fixture.functions {
                assert!(
                    functions.contains(name),
                    "{label}: the Functions view never named {name}"
                );
            }

            app.select_view(WorkspaceView::Strings);
            let strings = shown(&mut app);
            for text in &sample.fixture.strings {
                assert!(
                    strings.contains(text),
                    "{label}: the Strings view never showed {text:?}"
                );
            }

            app.select_view(WorkspaceView::Overview);
            app.preferences.explain_libraries = true;
            let overview = shown(&mut app);
            for library in &sample.fixture.libraries {
                assert!(
                    overview.contains(library),
                    "{label}: the Overview never named {library}"
                );
            }
        }
    }

    /// The empty state has no analysis to read; every view must cope.
    #[test]
    fn every_view_lays_out_before_a_binary_is_opened() {
        let ctx = egui::Context::default();
        for view in WorkspaceView::ALL {
            let mut app = DesdecApp::for_test(None, *view);
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                views::show_central_panel(&mut app, ctx);
                status_bar::show(&mut app, ctx);
            });
            assert!(!output.shapes.is_empty(), "{view:?} drew nothing");
        }
    }

    /// Filtering must never index outside the rows handed to the virtualiser.
    #[test]
    fn filtering_the_strings_view_stays_in_bounds() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Strings);
        for filter in ["", "/", "zzzzz-no-such-string", "LIB"] {
            app.strings_filter = filter.to_owned();
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                views::show_central_panel(&mut app, ctx);
            });
            assert!(
                !output.shapes.is_empty(),
                "the strings view drew nothing for filter {filter:?}"
            );
        }
    }

    #[test]
    fn sizes_use_binary_units() {
        assert_eq!(format_size(512), "512 bytes");
        assert_eq!(format_size(1024), "1.00 KiB");
        assert_eq!(format_size(1024 * 1024), "1.00 MiB");
        assert_eq!(format_size(1536), "1.50 KiB");
    }
}
