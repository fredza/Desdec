//! Panels, dialogs and views. Every module here draws from `&mut DesdecApp`
//! and owns no state of its own.

use eframe::egui;

pub mod about;
pub mod action_bar;
pub mod annotation;
pub mod assistant;
pub mod classes;
pub mod decompile;
pub mod disassembly;
pub mod dump;
pub mod expert;
pub mod expression;
pub mod flags;
pub mod functions;
pub mod graph;
pub mod library_note;
pub mod machine;
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
pub mod symbols;
pub mod syntax;
pub mod trace_until;
pub mod types;
pub mod update_window;
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

/// Opens a window at the exact centre of the workspace.
///
/// Centring by pivot rather than by arithmetic means the window never has to
/// be measured first: a dialog lands centred on the very frame it appears,
/// instead of opening off-centre and jumping into place once egui has laid it
/// out. The reader can move it afterwards; only the opening is decided here.
///
/// Every dialog opens this way except the two that answer a question about one
/// particular line — the corresponding disassembly and the operand inspection.
/// Those open beside the pointer, where the eye already is.
///
/// The pivot is set on every frame, not only on the opening one: egui stores a
/// window's position as the pivot point it was last given, so a window placed
/// by its centre and then drawn without a pivot would have that centre read
/// back as its top-left corner and jump down and to the right.
#[must_use]
pub fn centred<'a>(
    window: egui::Window<'a>,
    ctx: &egui::Context,
    opening: bool,
) -> egui::Window<'a> {
    let window = window.pivot(egui::Align2::CENTER_CENTER);
    if opening {
        window.current_pos(ctx.screen_rect().center())
    } else {
        window
    }
}

/// Where an inspection started from the disassembly opens: just below the
/// pointer, while keeping the whole window on screen.
pub fn under_cursor(ctx: &egui::Context, size: egui::Vec2) -> egui::Pos2 {
    const GAP: f32 = 12.0;
    let screen = ctx.screen_rect();
    let pointer = ctx.pointer_latest_pos().unwrap_or(screen.center());
    egui::pos2(
        (pointer.x + GAP)
            .min(screen.right() - size.x)
            .max(screen.left()),
        (pointer.y + GAP)
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

/// A filled round pip, the size of the surrounding text.
///
/// Drawn rather than written. The obvious spelling is a black-circle
/// character, and none of the fonts egui ships carries one in the
/// proportional family: `⬤` came out as `◻`, the replacement glyph, which
/// turned the loudest mark on the overview into a sign that something was
/// broken. A circle is two lines of painting and needs no font at all.
pub fn pip(ui: &mut egui::Ui, colour: egui::Color32) -> egui::Response {
    let diameter = ui.text_style_height(&egui::TextStyle::Body) * 0.52;
    let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::splat(diameter), egui::Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), diameter / 2.0, colour);
    response
}

/// A monospace value that gives way when the space runs short.
///
/// A path, a digest or a library name cannot be wrapped, and egui will not
/// shorten one on its own: laid out plainly it takes whatever width it needs,
/// and the card holding it grows with it. Truncated, it takes the width it is
/// given and the whole of it stays one hover away.
pub fn monospace_value(ui: &mut egui::Ui, value: &str) -> egui::Response {
    ui.add(egui::Label::new(egui::RichText::new(value).monospace()).truncate())
        .on_hover_text(value)
}

/// A two-column row that collapses to one column on a narrow window.
///
/// Both closures always run: each column owns its own `Ui`, so the caller does
/// not have to know which layout was chosen.
///
/// The two columns are placed here rather than by `Ui::columns`, which puts
/// each one at a fixed offset, clips neither, and then reports the widest as
/// the width of the whole row. A card holding something that cannot be made
/// narrower — the path of the open file, a sixty-four character digest — grew
/// past the column it had been given and was painted straight over its
/// neighbour, while the panel itself grew wider than the window. Which cards
/// overlapped depended on how long the path of the file being read happened to
/// be, which is what made it look as though it happened at random.
pub fn columns(
    ui: &mut egui::Ui,
    left: impl FnOnce(&mut egui::Ui),
    right: impl FnOnce(&mut egui::Ui),
) {
    columns_over(ui, &mut (), |ui, ()| left(ui), |ui, ()| right(ui));
}

/// The same, with one thing both columns write to.
///
/// The columns are drawn one after the other, but two closures that each
/// borrowed the same state would both be alive at once, which the borrow
/// checker refuses however they are called. Handing the state to each column
/// in turn is what lets a view keep its own state — an editor on the left, a
/// reading of what it produced on the right — in one place.
pub fn columns_over<T>(
    ui: &mut egui::Ui,
    over: &mut T,
    left: impl FnOnce(&mut egui::Ui, &mut T),
    right: impl FnOnce(&mut egui::Ui, &mut T),
) {
    if ui.available_width() < TWO_COLUMN_WIDTH {
        left(ui, over);
        ui.add_space(12.0);
        right(ui, over);
        return;
    }

    let spacing = ui.spacing().item_spacing.x;
    let full_width = ui.available_width();
    let width = (full_width - spacing) / 2.0;
    let top_left = ui.cursor().min;

    let heights = [
        column(ui, top_left, width, |ui| left(ui, over)),
        column(
            ui,
            top_left + egui::vec2(width + spacing, 0.0),
            width,
            |ui| right(ui, over),
        ),
    ];

    // The row takes up the width it was given and the height of its taller
    // column — never the width its contents asked for.
    ui.allocate_rect(
        egui::Rect::from_min_size(top_left, egui::vec2(full_width, heights[0].max(heights[1]))),
        egui::Sense::hover(),
    );
}

/// One column of [`columns`], held to its width whatever it is asked to draw.
///
/// Returns how tall what it drew turned out to be.
fn column(
    ui: &mut egui::Ui,
    top_left: egui::Pos2,
    width: f32,
    draw: impl FnOnce(&mut egui::Ui),
) -> f32 {
    let rect = egui::Rect::from_min_max(
        top_left,
        egui::pos2(top_left.x + width, ui.max_rect().bottom()),
    );
    let mut column = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down_justified(egui::Align::LEFT)),
    );
    column.set_width(width);
    // Whatever still refuses to be narrowed is cut off at the column's own
    // edge rather than painted over the column beside it. Only the sides are
    // held: how tall the column will be is not known until it has drawn.
    let clip = column.clip_rect();
    column.shrink_clip_rect(egui::Rect::from_min_max(
        egui::pos2(rect.left(), clip.top()),
        egui::pos2(rect.right(), clip.bottom()),
    ));
    draw(&mut column);
    column.min_rect().height()
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

    /// The outlined rectangles the cards are drawn with.
    ///
    /// A card is a `ui.group`, which paints one stroked rectangle around what
    /// it holds; the small stroked shapes in the panel — gauges, buttons — are
    /// left out by size.
    fn card_frames(shapes: &[egui::epaint::ClippedShape]) -> Vec<egui::Rect> {
        fn walk(shape: &egui::Shape, found: &mut Vec<egui::Rect>) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, found);
                    }
                }
                egui::Shape::Rect(rect)
                    if rect.stroke.width > 0.0
                        && rect.rect.width() > 120.0
                        && rect.rect.height() > 40.0 =>
                {
                    found.push(rect.rect);
                }
                _ => {}
            }
        }

        let mut found = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, &mut found);
        }
        found
    }

    /// No card may be drawn over the card beside it, or past the window.
    ///
    /// `Ui::columns` puts each column at a fixed offset and clips neither, so
    /// a card holding something that cannot be narrowed — the path of the open
    /// file is the everyday one — grew past its column and was painted over
    /// its neighbour, and the panel itself grew wider than the window. Nothing
    /// about the text drawn says so: every string is on screen either way.
    /// Only where it landed does, which is what this reads.
    #[test]
    fn no_card_in_the_overview_is_drawn_over_another_or_past_the_window() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        // Measured in the interface's own type, not egui's. What decides
        // whether a card fits is the width of the text in it, and this test
        // used to lay out at egui's default sizes — a font the application
        // never draws a single word in.
        crate::fonts::install(&ctx);
        crate::preferences::apply_theme(&ctx, app.preferences.theme);
        // A path long enough to overflow every width tested below. Reading a
        // file from a deeply nested directory was all it ever took.
        if let Some(analysis) = app.analysis.as_mut() {
            analysis.summary.path = std::path::PathBuf::from(
                "/home/quelqu-un/projets/retro-ingenierie/echantillons/2026/aout/lot-17/desdec_app-ac00cd5b152a0c7b",
            );
        }

        for width in [700.0, 899.0, 901.0, 1200.0, 1500.0, 1920.0] {
            // Two frames: a panel is measured on the first and painted after.
            let mut draw = |ctx: &egui::Context| views::show_central_panel(&mut app, ctx);
            let _ = ctx.run(input_of_width(width), &mut draw);
            let output = ctx.run(input_of_width(width), &mut draw);

            let frames = card_frames(&output.shapes);
            assert!(
                frames.len() >= 2,
                "the overview drew {} cards at width {width}",
                frames.len()
            );
            for (index, frame) in frames.iter().enumerate() {
                assert!(
                    frame.right() <= width + 1.0,
                    "a card reaches to {} at width {width}",
                    frame.right()
                );
                for other in &frames[index + 1..] {
                    assert!(
                        !frame.intersects(*other),
                        "two cards overlap at width {width}: {frame:?} and {other:?}"
                    );
                }
            }
        }
    }

    /// A column holds its width even when what it is asked to draw will not.
    ///
    /// The card above is narrowed by shortening the path it holds, so the
    /// overview passes this whether or not the columns themselves keep to
    /// their width. This reads the guarantee directly: a column given
    /// something that refuses to be narrowed — anything laid out with
    /// `extend`, which is what a long unwrappable run of text amounts to —
    /// must cut it off at its own edge rather than let it be painted over the
    /// column beside it, and must not widen the row that holds both.
    #[test]
    fn a_column_never_lets_its_contents_reach_the_column_beside_it() {
        let width = 1200.0;
        let ctx = egui::Context::default();
        let output = ctx.run(input_of_width(width), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                columns(
                    ui,
                    |ui| {
                        ui.add(
                            egui::Label::new("A".repeat(400)).wrap_mode(egui::TextWrapMode::Extend),
                        );
                    },
                    |ui| {
                        ui.label("beside it");
                    },
                );
                // What the row as a whole took up, which is what the panel and
                // everything below it are laid out against.
                assert!(
                    ui.min_rect().right() <= width + 1.0,
                    "the row reaches to {} in a window {width} wide",
                    ui.min_rect().right()
                );
            });
        });

        for clipped in &output.shapes {
            let egui::Shape::Text(text) = &clipped.shape else {
                continue;
            };
            if !text.galley.text().starts_with("AAA") {
                continue;
            }
            // Where the run of text is allowed to show, which is the left
            // column and no further: it is laid out at whatever width it
            // wants, and the clip is what holds it.
            assert!(
                clipped.clip_rect.right() < width / 2.0 + 20.0,
                "the left column paints as far as {}, in a window {width} wide",
                clipped.clip_rect.right()
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
