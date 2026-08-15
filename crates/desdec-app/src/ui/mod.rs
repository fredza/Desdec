//! Panels, dialogs and views. Every module here draws from `&mut DesdecApp`
//! and owns no state of its own.

use eframe::egui;

pub mod about;
pub mod action_bar;
pub mod decompile;
pub mod disassembly;
pub mod expert;
pub mod functions;
pub mod navigation;
pub mod palette;
pub mod patches_view;
pub mod preferences_window;
pub mod segments;
pub mod status_bar;
pub mod strings;
pub mod syntax;
pub mod views;

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
        let path = std::env::current_exe().expect("the test binary has a path");
        let analysis = desdec_core::analyse_path(&path).expect("the test binary is analysable");

        let ctx = egui::Context::default();
        for view in WorkspaceView::ALL {
            for language in crate::i18n::Language::ALL {
                let mut app = DesdecApp::for_test(Some(analysis.clone()), *view);
                app.preferences.language = *language;

                for width in layout_widths() {
                    let output = ctx.run(input_of_width(width), |ctx| {
                        views::show_central_panel(&mut app, ctx);
                        status_bar::show(&mut app, ctx);
                    });
                    assert!(
                        !output.shapes.is_empty(),
                        "{} drew nothing at width {width}",
                        view.icon()
                    );
                }
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
            assert!(!output.shapes.is_empty(), "{} drew nothing", view.icon());
        }
    }

    /// Filtering must never index outside the rows handed to the virtualiser.
    #[test]
    fn filtering_the_strings_view_stays_in_bounds() {
        let path = std::env::current_exe().expect("the test binary has a path");
        let analysis = desdec_core::analyse_path(&path).expect("the test binary is analysable");

        let ctx = egui::Context::default();
        for filter in ["", "/", "zzzzz-no-such-string", "LIB"] {
            let mut app = DesdecApp::for_test(Some(analysis.clone()), WorkspaceView::Strings);
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
