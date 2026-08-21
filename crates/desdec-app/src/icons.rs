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
    Dump,
    Assistant,
    Patches,
    Yara,
    Script,
    Plugins,
    Open,
    Palette,
    Preferences,
    Output,
    About,
    Menu,
    Close,
    /// Points at the edge the panel would move towards.
    CollapseLeft,
    ExpandRight,
    /// The transport of the static walk, in the order the buttons sit.
    WalkToEntry,
    WalkBack,
    WalkInto,
    WalkOver,
    WalkOut,
    WalkClear,
    /// The emulated processor, and the transport of a run on it.
    Machine,
    /// One function drawn as its control flow.
    Graph,
    Run,
    Restart,
    Breakpoint,
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
        Self::Dump,
        Self::Assistant,
        Self::Patches,
        Self::Yara,
        Self::Script,
        Self::Plugins,
        Self::Open,
        Self::Palette,
        Self::Preferences,
        Self::Output,
        Self::About,
        Self::Menu,
        Self::Close,
        Self::CollapseLeft,
        Self::ExpandRight,
        Self::WalkToEntry,
        Self::WalkBack,
        Self::WalkInto,
        Self::WalkOver,
        Self::WalkOut,
        Self::WalkClear,
        Self::Machine,
        Self::Run,
        Self::Restart,
        Self::Breakpoint,
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
        Icon::Dump => dump(&pen),
        Icon::Assistant => assistant(&pen),
        Icon::Patches => patches(&pen),
        Icon::Yara => yara(&pen),
        Icon::Script => script(&pen),
        Icon::Plugins => plugins(&pen),
        Icon::Open => open(&pen),
        Icon::Palette => palette(&pen),
        Icon::Preferences => preferences(&pen),
        Icon::Output => output(&pen),
        Icon::About => about(&pen),
        Icon::Menu => menu(&pen),
        Icon::Close => close(&pen),
        Icon::CollapseLeft => chevron(&pen, -1.0),
        Icon::ExpandRight => chevron(&pen, 1.0),
        Icon::WalkToEntry => walk_to_entry(&pen),
        Icon::WalkBack => walk_back(&pen),
        Icon::WalkInto => walk_into(&pen),
        Icon::WalkOver => walk_over(&pen),
        Icon::WalkOut => walk_out(&pen),
        Icon::WalkClear => walk_clear(&pen),
        Icon::Machine => machine(&pen),
        Icon::Graph => graph(&pen),
        Icon::Run => run(&pen),
        Icon::Restart => restart(&pen),
        Icon::Breakpoint => breakpoint(&pen),
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

    /// A closed shape filled rather than outlined, for a glyph whose weight is
    /// the point of it — a tape deck's play against a step's open arrow.
    fn solid(&self, points: &[(f32, f32)]) {
        let corners: Vec<egui::Pos2> = points.iter().map(|(x, y)| self.at(*x, *y)).collect();
        self.painter.add(egui::Shape::convex_polygon(
            corners,
            self.color,
            egui::Stroke::NONE,
        ));
    }

    /// A filled circle of a chosen size, unlike [`Self::dot`], whose size is
    /// fixed because it marks a point rather than being the glyph itself.
    fn disc(&self, center: (f32, f32), radius: f32) {
        self.painter.circle_filled(
            self.at(center.0, center.1),
            self.rect.width() * radius,
            self.color,
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

/// The offset column, then the bytes: a hexadecimal dump seen from far away.
fn dump(pen: &Pen) {
    pen.line((0.0, 0.0), (0.0, 1.0));
    for y in [0.08, 0.42, 0.76] {
        for x in [0.26, 0.56, 0.86] {
            pen.filled((x - 0.12, y), (x + 0.12, y + 0.16), 0.5);
        }
    }
}

/// Braces around uneven source lines: pseudo-C rather than a generic code
/// block, visibly distinct from the address-and-opcode listing icon.
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
    for (y, end) in [(0.26_f32, 0.68_f32), (0.5, 0.78), (0.74, 0.6)] {
        pen.line((0.35, y), (end, y));
    }
}

/// Bytes replaced: a run of cells with one written over.
fn patches(pen: &Pen) {
    // Four cells rather than three, and the changed one off-centre: three with
    // the middle one filled read as a progress bar half done.
    pen.boxed((0.0, 0.3), (1.0, 0.7), 1.0);
    for x in [0.25_f32, 0.5, 0.75] {
        pen.line((x, 0.3), (x, 0.7));
    }
    pen.filled((0.52, 0.32), (0.73, 0.68), 0.0);
}

/// A scan frame with its sweep: rules run over the whole file.
///
/// Deliberately not a magnifier. Every tool drew one for twenty years, and it
/// came to mean "search" in general — while YARA is not a search but a set of
/// rules run over everything. Four corners and a sweep is the mark a reader
/// now recognises as scanning, and it is nothing like any other glyph here.
fn yara(pen: &Pen) {
    const REACH: f32 = 0.3;
    for (x, y, horizontal, vertical) in [
        (0.0_f32, 0.0_f32, 1.0_f32, 1.0_f32),
        (1.0, 0.0, -1.0, 1.0),
        (0.0, 1.0, 1.0, -1.0),
        (1.0, 1.0, -1.0, -1.0),
    ] {
        pen.path(&[
            (x + horizontal * REACH, y),
            (x, y),
            (x, y + vertical * REACH),
        ]);
    }
    pen.line((0.06, 0.5), (0.94, 0.5));
}

/// Two sparks: a reading offered alongside the listing, not a measurement of
/// it. Deliberately unlike every other glyph here, which draws a part of the
/// file — this one draws something added to it.
fn assistant(pen: &Pen) {
    spark(pen, (0.38, 0.38), 0.34);
    spark(pen, (0.82, 0.78), 0.18);
}

/// A four-pointed star, drawn as two crossed strokes pinched at the middle.
fn spark(pen: &Pen, center: (f32, f32), size: f32) {
    let (x, y) = center;
    pen.path(&[
        (x, y - size),
        (x + size * 0.28, y - size * 0.28),
        (x + size, y),
        (x + size * 0.28, y + size * 0.28),
        (x, y + size),
        (x - size * 0.28, y + size * 0.28),
        (x - size, y),
        (x - size * 0.28, y - size * 0.28),
        (x, y - size),
    ]);
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

/// Stamped lines, each with the moment it happened: an account kept in order.
fn output(pen: &Pen) {
    for (y, end) in [(0.12, 1.0), (0.5, 0.7), (0.88, 0.9)] {
        pen.dot((0.06, y));
        pen.line((0.28, y), (end, y));
    }
}

/// A prompt: the chevron and the line something is typed on.
fn script(pen: &Pen) {
    pen.path(&[(0.06, 0.22), (0.44, 0.5), (0.06, 0.78)]);
    pen.line((0.56, 0.78), (0.98, 0.78));
}

/// A plug: a body outside, and the pins going into what it plugs into.
fn plugins(pen: &Pen) {
    pen.boxed((0.3, 0.14), (1.0, 0.86), 2.0);
    pen.line((0.0, 0.36), (0.3, 0.36));
    pen.line((0.0, 0.64), (0.3, 0.64));
}

/// Two sliders: settings that are chosen rather than toggled.
fn preferences(pen: &Pen) {
    for (y, knob) in [(0.28, 0.66), (0.72, 0.34)] {
        pen.line((0.0, y), (1.0, y));
        pen.dot((knob, y));
    }
}

/// The usual circled letter, drawn rather than typed so it needs no font.
///
/// The ring is drawn at the same weight as every other glyph rather than
/// filled: a solid disc with a hole in it was the heaviest mark on the bar,
/// and the eye went to "about" before it went to anything the reader came for.
fn about(pen: &Pen) {
    pen.circle((0.5, 0.5), 0.46);
    pen.line((0.5, 0.22), (0.5, 0.3));
    pen.line((0.5, 0.42), (0.5, 0.76));
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

/// The first instruction of the static walk: an address in the code, not the
/// beginning of a recording.
fn walk_to_entry(pen: &Pen) {
    pen.circle((0.5, 0.5), 0.34);
    pen.line((0.5, 0.04), (0.5, 0.24));
    pen.line((0.5, 0.76), (0.5, 0.96));
    pen.line((0.04, 0.5), (0.24, 0.5));
    pen.line((0.76, 0.5), (0.96, 0.5));
    pen.dot((0.5, 0.5));
}

/// Reverse along the path already read. The bent arrow says "go back through
/// code", instead of borrowing the rewind glyph from a media player.
fn walk_back(pen: &Pen) {
    pen.path(&[
        (0.86, 0.22),
        (0.36, 0.22),
        (0.2, 0.38),
        (0.2, 0.74),
        (0.82, 0.74),
    ]);
    pen.path(&[(0.5, 0.56), (0.2, 0.74), (0.5, 0.92)]);
}

/// Follow the line into the called code, marked by a downward arrow landing
/// on its first instruction.
fn walk_into(pen: &Pen) {
    pen.line((0.08, 0.82), (0.92, 0.82));
    pen.line((0.5, 0.1), (0.5, 0.62));
    pen.path(&[(0.25, 0.4), (0.5, 0.66), (0.75, 0.4)]);
}

/// An arc jumping over a point on the line: the call that is passed rather
/// than entered.
fn walk_over(pen: &Pen) {
    pen.path(&[
        (0.0, 0.76),
        (0.16, 0.76),
        (0.34, 0.24),
        (0.66, 0.24),
        (0.84, 0.76),
        (1.0, 0.76),
    ]);
    pen.dot((0.5, 0.76));
    pen.path(&[(0.72, 0.54), (1.0, 0.76), (0.72, 0.98)]);
}

/// An arrow leaving the line it stands on: back out of the call.
fn walk_out(pen: &Pen) {
    pen.line((0.06, 0.9), (0.94, 0.9));
    pen.line((0.5, 0.86), (0.5, 0.12));
    pen.path(&[(0.24, 0.38), (0.5, 0.1), (0.76, 0.38)]);
}

/// Forget the path, shown as a route struck through rather than a media-player
/// stop button.
fn walk_clear(pen: &Pen) {
    pen.path(&[(0.12, 0.74), (0.36, 0.5), (0.58, 0.68), (0.86, 0.3)]);
    pen.dot((0.12, 0.74));
    pen.dot((0.36, 0.5));
    pen.dot((0.58, 0.68));
    pen.line((0.16, 0.16), (0.84, 0.84));
}

/// A processor: a square die with legs on all four sides. Deliberately not a
/// bug or a play button — what this view holds is a machine, and the run is
/// only one of the things it offers.
fn machine(pen: &Pen) {
    pen.boxed((0.26, 0.26), (0.74, 0.74), 1.0);
    for offset in [0.38_f32, 0.5, 0.62] {
        pen.line((offset, 0.06), (offset, 0.26));
        pen.line((offset, 0.74), (offset, 0.94));
        pen.line((0.06, offset), (0.26, offset));
        pen.line((0.74, offset), (0.94, offset));
    }
}

/// A block above two, joined by the arrows between them: the smallest shape
/// this view is for, which is a test and its two arms.
fn graph(pen: &Pen) {
    pen.boxed((0.32, 0.06), (0.68, 0.3), 1.0);
    pen.boxed((0.04, 0.7), (0.4, 0.94), 1.0);
    pen.boxed((0.6, 0.7), (0.96, 0.94), 1.0);
    pen.line((0.42, 0.3), (0.22, 0.7));
    pen.line((0.58, 0.3), (0.78, 0.7));
}

/// The tape deck's play, for a run that carries on until something stops it.
///
/// Solid, where the step buttons beside it are outlines: the two are one
/// press apart and would otherwise be two triangles the reader has to read
/// the tooltip to tell apart.
fn run(pen: &Pen) {
    pen.solid(&[(0.18, 0.06), (0.92, 0.5), (0.18, 0.94)]);
}

/// An arrow coming back round to where it started: three quarters of a circle,
/// with a head on the end that closes it.
fn restart(pen: &Pen) {
    let turn: Vec<(f32, f32)> = (0..=24_u8)
        .map(|step| {
            let angle = std::f32::consts::TAU * (0.12 + 0.76 * f32::from(step) / 24.0);
            (0.5 + 0.36 * angle.cos(), 0.52 + 0.36 * angle.sin())
        })
        .collect();
    pen.path(&turn);
    // The head sits on the gap the arc leaves, pointing the way round.
    pen.path(&[(0.62, 0.02), (0.84, 0.2), (0.58, 0.3)]);
}

/// The filled disc a debugger has used for a breakpoint since there were
/// debuggers.
fn breakpoint(pen: &Pen) {
    pen.disc((0.5, 0.5), 0.34);
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
                // A closed shape is a `polygon` and an open one a `polyline`,
                // because SVG closes the first and not the second. Written as
                // a polyline with no fill, a filled glyph — the transport's
                // play triangle — came out as an empty cell; written as a
                // polygon, every open glyph came out with a line joining its
                // two ends, which is a pair of quotation marks turned into two
                // triangles. The shape itself says which it is.
                let (element, fill) = if path.closed {
                    ("polygon", colour(path.fill != egui::Color32::TRANSPARENT))
                } else {
                    ("polyline", "none")
                };
                let _ = writeln!(
                    out,
                    "<{element} points=\"{}\" fill=\"{fill}\" stroke=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>",
                    points.join(" "),
                    colour(path.stroke.width > 0.0),
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
