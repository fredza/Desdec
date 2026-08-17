//! Vector icons drawn by the application itself: no icon font, no external
//! asset, and identical rendering on every platform.
//!
//! Every glyph is described in a unit square and scaled to whatever rectangle
//! it is given, so the same icon serves a 16-pixel rail and a 34-pixel toolbar
//! button without a second drawing. Stroke width scales with it: a glyph drawn
//! with a fixed 1.8-pixel line looks spindly when enlarged and turns into a
//! blot when shrunk.

use eframe::egui;

/// One drawn symbol. Each workspace view has its own, and no two views share
/// one — an icon that stands for two things stands for neither.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Icon {
    Overview,
    Segments,
    Functions,
    Strings,
    Disassembly,
    Decompile,
    Patches,
    Yara,
    Open,
    Palette,
    Preferences,
    About,
    Menu,
    Close,
    /// Points at the edge the panel would move towards.
    CollapseLeft,
    ExpandRight,
}

impl Icon {
    /// Every icon there is, for the tests that hold the whole set to the same
    /// promises.
    #[cfg(test)]
    pub const ALL: &'static [Self] = &[
        Self::Overview,
        Self::Segments,
        Self::Functions,
        Self::Strings,
        Self::Disassembly,
        Self::Decompile,
        Self::Patches,
        Self::Yara,
        Self::Open,
        Self::Palette,
        Self::Preferences,
        Self::About,
        Self::Menu,
        Self::Close,
        Self::CollapseLeft,
        Self::ExpandRight,
    ];
}

const BUTTON_SIZE: egui::Vec2 = egui::vec2(34.0, 30.0);
/// Fraction of the button taken by the glyph; the rest is breathing room.
const GLYPH_SCALE: f32 = 0.62;
const CORNER_RADIUS: f32 = 5.0;
/// Stroke as a fraction of the glyph's size, and the range it stays inside so
/// a very small icon keeps a visible line and a large one does not go heavy.
const STROKE_RATIO: f32 = 0.1;
const STROKE_RANGE: std::ops::RangeInclusive<f32> = 0.9..=2.6;
/// Opacity of the accent colour behind the selected action.
const SELECTED_FILL: f32 = 0.42;

/// A square icon button of the toolbar's usual size.
pub fn button(
    ui: &mut egui::Ui,
    icon: Icon,
    tooltip: Option<String>,
    selected: bool,
    accent: egui::Color32,
) -> egui::Response {
    sized_button(ui, icon, tooltip, selected, accent, BUTTON_SIZE)
}

/// The same, at a chosen size — the navigation rail draws narrower ones.
pub fn sized_button(
    ui: &mut egui::Ui,
    icon: Icon,
    tooltip: Option<String>,
    selected: bool,
    accent: egui::Color32,
    size: egui::Vec2,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let visuals = ui.style().interact(&response);
    let fill = if selected {
        accent.gamma_multiply(SELECTED_FILL)
    } else if response.hovered() {
        visuals.bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    if fill != egui::Color32::TRANSPARENT {
        ui.painter()
            .rect_filled(rect.shrink(1.0), CORNER_RADIUS, fill);
    }
    draw(ui.painter(), rect, icon, visuals.fg_stroke.color);

    match tooltip {
        Some(tooltip) => response.on_hover_text(tooltip),
        None => response,
    }
}

/// Draws `icon` centred in `rect`, as large as its shortest side allows.
///
/// Public so a widget that already owns its layout — a menu row with a label
/// beside the glyph — can place one itself.
pub fn draw(painter: &egui::Painter, rect: egui::Rect, icon: Icon, color: egui::Color32) {
    let side = rect.width().min(rect.height()) * GLYPH_SCALE;
    let square = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(side));
    let pen = Pen {
        painter,
        rect: square,
        stroke: egui::Stroke::new(
            (side * STROKE_RATIO).clamp(*STROKE_RANGE.start(), *STROKE_RANGE.end()),
            color,
        ),
        color,
    };
    match icon {
        Icon::Overview => overview(&pen),
        Icon::Segments => segments(&pen),
        Icon::Functions => functions(&pen),
        Icon::Strings => strings(&pen),
        Icon::Disassembly => disassembly(&pen),
        Icon::Decompile => decompile(&pen),
        Icon::Patches => patches(&pen),
        Icon::Yara => yara(&pen),
        Icon::Open => open(&pen),
        Icon::Palette => palette(&pen),
        Icon::Preferences => preferences(&pen),
        Icon::About => about(&pen),
        Icon::Menu => menu(&pen),
        Icon::Close => close(&pen),
        Icon::CollapseLeft => chevron(&pen, -1.0),
        Icon::ExpandRight => chevron(&pen, 1.0),
    }
}

/// Drawing in a unit square: `(0, 0)` is the glyph's top left, `(1, 1)` its
/// bottom right, whatever size it is being drawn at.
struct Pen<'a> {
    painter: &'a egui::Painter,
    rect: egui::Rect,
    stroke: egui::Stroke,
    color: egui::Color32,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> egui::Pos2 {
        egui::pos2(
            self.rect.left() + self.rect.width() * x,
            self.rect.top() + self.rect.height() * y,
        )
    }

    fn line(&self, from: (f32, f32), to: (f32, f32)) {
        self.painter
            .line_segment([self.at(from.0, from.1), self.at(to.0, to.1)], self.stroke);
    }

    fn path(&self, points: &[(f32, f32)]) {
        let points: Vec<egui::Pos2> = points.iter().map(|(x, y)| self.at(*x, *y)).collect();
        self.painter.add(egui::Shape::line(points, self.stroke));
    }

    /// A rectangle outline. The stroke straddles the edge rather than sitting
    /// inside it: a small glyph's box is thinner than its own line, and an
    /// inside stroke would leave nothing at all to draw.
    fn boxed(&self, min: (f32, f32), max: (f32, f32), rounding: f32) {
        self.painter.rect_stroke(
            egui::Rect::from_min_max(self.at(min.0, min.1), self.at(max.0, max.1)),
            rounding,
            self.stroke,
            egui::StrokeKind::Middle,
        );
    }

    fn filled(&self, min: (f32, f32), max: (f32, f32), rounding: f32) {
        self.painter.rect_filled(
            egui::Rect::from_min_max(self.at(min.0, min.1), self.at(max.0, max.1)),
            rounding,
            self.color,
        );
    }

    fn dot(&self, at: (f32, f32)) {
        let radius = (self.rect.width() * 0.09).max(1.0);
        self.painter
            .circle_filled(self.at(at.0, at.1), radius, self.color);
    }

    fn circle(&self, center: (f32, f32), radius: f32) {
        self.painter.circle_stroke(
            self.at(center.0, center.1),
            self.rect.width() * radius,
            self.stroke,
        );
    }
}

/// Four panes: the whole file seen at once.
fn overview(pen: &Pen) {
    for (x, y) in [(0.0, 0.0), (0.56, 0.0), (0.0, 0.56), (0.56, 0.56)] {
        pen.boxed((x, y), (x + 0.44, y + 0.44), 1.0);
    }
}

/// Bands of unequal width, stacked: the section table as a map of the image.
fn segments(pen: &Pen) {
    for (y, width) in [(0.06, 1.0), (0.4, 0.62), (0.74, 0.84)] {
        pen.filled((0.0, y), (width, y + 0.2), 1.0);
    }
}

/// A call splitting into two branches: control flow.
fn functions(pen: &Pen) {
    pen.line((0.0, 0.5), (0.45, 0.5));
    pen.line((0.45, 0.5), (1.0, 0.08));
    pen.line((0.45, 0.5), (1.0, 0.92));
    for point in [(0.0, 0.5), (1.0, 0.08), (1.0, 0.92)] {
        pen.dot(point);
    }
}

/// Opening and closing quotation marks.
fn strings(pen: &Pen) {
    for (outer, inner) in [(0.3, 0.06), (0.7, 0.94)] {
        pen.path(&[(outer, 0.16), (inner, 0.44), (outer, 0.72)]);
    }
}

/// Stacked instruction lines of uneven length, with their address column.
fn disassembly(pen: &Pen) {
    for y in [0.1, 0.42, 0.74] {
        pen.line((0.0, y), (0.22, y));
    }
    for (y, end) in [(0.1, 1.0), (0.42, 0.72), (0.74, 0.92)] {
        pen.line((0.36, y), (end, y));
    }
}

/// A pair of braces: the pseudo-code rebuilt from those instructions.
fn decompile(pen: &Pen) {
    for (edge, inward) in [(0.0_f32, 1.0_f32), (1.0, -1.0)] {
        let spine = edge + inward * 0.22;
        pen.path(&[
            (spine + inward * 0.22, 0.0),
            (spine, 0.14),
            (spine, 0.4),
            (edge, 0.5),
            (spine, 0.6),
            (spine, 0.86),
            (spine + inward * 0.22, 1.0),
        ]);
    }
}

/// Bytes replaced: a run of cells with one written over.
fn patches(pen: &Pen) {
    pen.boxed((0.0, 0.28), (1.0, 0.72), 1.0);
    pen.line((0.34, 0.28), (0.34, 0.72));
    pen.line((0.66, 0.28), (0.66, 0.72));
    pen.filled((0.36, 0.3), (0.64, 0.7), 0.0);
}

/// A magnifier over a rule: scanning the file for patterns.
fn yara(pen: &Pen) {
    pen.circle((0.42, 0.42), 0.3);
    pen.line((0.64, 0.64), (1.0, 1.0));
    pen.line((0.3, 0.42), (0.54, 0.42));
}

/// An open folder.
fn open(pen: &Pen) {
    pen.path(&[
        (0.0, 0.86),
        (0.0, 0.14),
        (0.36, 0.14),
        (0.46, 0.32),
        (1.0, 0.32),
        (1.0, 0.86),
        (0.0, 0.86),
    ]);
}

/// A command input with its prompt.
fn palette(pen: &Pen) {
    pen.boxed((0.0, 0.1), (1.0, 0.9), 2.0);
    pen.path(&[(0.2, 0.36), (0.38, 0.5), (0.2, 0.64)]);
    pen.line((0.5, 0.64), (0.8, 0.64));
}

/// Two sliders: settings that are chosen rather than toggled.
fn preferences(pen: &Pen) {
    for (y, knob) in [(0.28, 0.66), (0.72, 0.34)] {
        pen.line((0.0, y), (1.0, y));
        pen.dot((knob, y));
    }
}

/// The usual circled letter, drawn rather than typed so it needs no font.
fn about(pen: &Pen) {
    pen.circle((0.5, 0.5), 0.5);
    pen.dot((0.5, 0.24));
    pen.line((0.5, 0.44), (0.5, 0.78));
}

/// Three bars: the menu.
fn menu(pen: &Pen) {
    for y in [0.12, 0.5, 0.88] {
        pen.line((0.0, y), (1.0, y));
    }
}

fn close(pen: &Pen) {
    pen.line((0.08, 0.08), (0.92, 0.92));
    pen.line((0.92, 0.08), (0.08, 0.92));
}

/// A chevron pointing left (`-1.0`) or right (`1.0`).
fn chevron(pen: &Pen, direction: f32) {
    let (near, far) = if direction < 0.0 {
        (0.72, 0.28)
    } else {
        (0.28, 0.72)
    };
    pen.path(&[(near, 0.1), (far, 0.5), (near, 0.9)]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon, at sizes from a cramped rail to a large button: the glyphs
    /// are scaled rather than drawn once, so each size is a fresh chance to
    /// divide by a zero-width rectangle or paint nothing at all.
    #[test]
    fn every_icon_draws_at_every_size() {
        let ctx = egui::Context::default();
        for icon in Icon::ALL {
            for side in [10.0_f32, 16.0, 24.0, 34.0, 64.0] {
                let output = ctx.run(crate::testing::window_input(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::hover());
                        draw(ui.painter(), rect, *icon, egui::Color32::WHITE);
                    });
                });
                let painted = output
                    .shapes
                    .iter()
                    .filter(|clipped| !matches!(clipped.shape, egui::Shape::Noop))
                    .count();
                assert!(painted > 0, "{icon:?} drew nothing at {side} pixels");
            }
        }
    }
}

/// Renders the whole icon set to an SVG, for a human to look at.
///
/// A drawn icon can only be judged by eye, and a test that asserts "something
/// was painted" says nothing about whether the glyph reads as what it stands
/// for. Run with the destination in the environment to get a sheet out:
///
/// ```text
/// DESDEC_ICON_SHEET=/tmp/icons.svg cargo test -p desdec-app icon_sheet
/// ```
///
/// It writes nothing when that variable is unset, which is every other run.
#[cfg(test)]
mod sheet {
    use super::*;

    use std::fmt::Write as _;

    const CELL: f32 = 64.0;
    const COLUMNS: usize = 8;

    /// A count as a coordinate. Sheets are a handful of cells wide.
    #[expect(
        clippy::cast_precision_loss,
        reason = "the sheet is a few cells across"
    )]
    fn coordinate(count: usize) -> f32 {
        count as f32
    }

    #[test]
    fn icon_sheet() {
        let Ok(path) = std::env::var("DESDEC_ICON_SHEET") else {
            return;
        };
        let rows = Icon::ALL.len().div_ceil(COLUMNS);
        let ctx = egui::Context::default();
        let mut body = String::new();

        for (index, icon) in Icon::ALL.iter().enumerate() {
            let column = index % COLUMNS;
            let row = index / COLUMNS;
            let origin = egui::pos2(coordinate(column) * CELL, coordinate(row) * CELL);
            let mut cell = egui::Rect::NOTHING;
            let output = ctx.run(crate::testing::window_input(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::Vec2::splat(CELL), egui::Sense::hover());
                    cell = rect;
                    draw(ui.painter(), rect, *icon, egui::Color32::WHITE);
                });
            });
            let offset = origin - cell.min;
            for clipped in &output.shapes {
                emit(&clipped.shape, offset, &mut body);
            }
            let _ = writeln!(
                body,
                "<text x=\"{}\" y=\"{}\" fill=\"#8a93a6\" font-size=\"7\" font-family=\"sans-serif\" text-anchor=\"middle\">{icon:?}</text>",
                origin.x + CELL / 2.0,
                origin.y + CELL - 4.0
            );
        }

        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><rect width=\"100%\" height=\"100%\" fill=\"#1b1f27\"/>\n{body}</svg>",
            coordinate(COLUMNS) * CELL,
            coordinate(rows) * CELL,
            coordinate(COLUMNS) * CELL,
            coordinate(rows) * CELL,
        );
        std::fs::write(path, svg).expect("the sheet is writable");
    }
    /// Turns one painted shape into its SVG equivalent.
    fn emit(shape: &egui::Shape, offset: egui::Vec2, out: &mut String) {
        const INK: &str = "#e6e9ef";
        let point = |p: egui::Pos2| (p.x + offset.x, p.y + offset.y);
        let colour = |present: bool| if present { INK } else { "none" };
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    emit(shape, offset, out);
                }
            }
            egui::Shape::LineSegment { points, stroke } => {
                let (a, b) = (point(points[0]), point(points[1]));
                let _ = writeln!(
                    out,
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{INK}\" stroke-width=\"{}\" stroke-linecap=\"round\"/>",
                    a.0, a.1, b.0, b.1, stroke.width
                );
            }
            egui::Shape::Path(path) => {
                let points: Vec<String> = path
                    .points
                    .iter()
                    .map(|p| {
                        let p = point(*p);
                        format!("{},{}", p.0, p.1)
                    })
                    .collect();
                let _ = writeln!(
                    out,
                    "<polyline points=\"{}\" fill=\"none\" stroke=\"{INK}\" stroke-width=\"{}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>",
                    points.join(" "),
                    path.stroke.width
                );
            }
            egui::Shape::Rect(rect) => {
                if rect.rect.width() > CELL {
                    return; // The panel's own background, not a glyph.
                }
                let min = point(rect.rect.min);
                let _ = writeln!(
                    out,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>",
                    min.0,
                    min.1,
                    rect.rect.width(),
                    rect.rect.height(),
                    rect.corner_radius.nw,
                    colour(rect.fill != egui::Color32::TRANSPARENT),
                    colour(rect.stroke.width > 0.0),
                    rect.stroke.width
                );
            }
            egui::Shape::Circle(circle) => {
                let center = point(circle.center);
                let _ = writeln!(
                    out,
                    "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>",
                    center.0,
                    center.1,
                    circle.radius,
                    colour(circle.fill != egui::Color32::TRANSPARENT),
                    colour(circle.stroke.width > 0.0),
                    circle.stroke.width
                );
            }
            _ => {}
        }
    }
}
