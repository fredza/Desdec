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
    /// The names the file declares: imports and defined symbols.
    Symbols,
    /// C++ classes recovered from the symbol names.
    Classes,
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
    /// One value in every base at once, and the bit operations over it.
    Calculator,
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
    /// A run of bytes cut into the members of a structure.
    Structures,
    Run,
    Restart,
    Breakpoint,
    /// The road sign: a warning that changes how everything under it should
    /// be read.
    Warning,
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
        Self::Symbols,
        Self::Classes,
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
        Self::Calculator,
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
        Self::Graph,
        Self::Structures,
        Self::Run,
        Self::Restart,
        Self::Breakpoint,
        Self::Warning,
    ];
}

/// The size of a toolbar button.
///
/// Public because a row holding both buttons and text has to be given this
/// height before anything is put in it. egui centres each widget against the
/// height the row has reached so far, not the height it ends up with, so a
/// label written before the first button is centred against the label's own
/// height and then left sitting above the buttons that follow it.
pub const BUTTON_SIZE: egui::Vec2 = egui::vec2(34.0, 30.0);
/// Fraction of the button taken by the glyph; the rest is breathing room.
///
/// Public so a caller that wants the glyph at a size of its own — the
/// application's icon, which is a glyph on a tile and not a glyph in a button
/// — can work out the rectangle that produces it.
pub const GLYPH_SCALE: f32 = 0.62;
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
    let width = (side * STROKE_RATIO).clamp(*STROKE_RANGE.start(), *STROKE_RANGE.end());
    draw_with_stroke(painter, rect, icon, color, width);
}

/// The stroke [`draw`] would use at this size, without the clamp.
///
/// [`STROKE_RANGE`] is a button's range: it keeps a sixteen-pixel rail icon
/// from thinning to a hairline and a thirty-four-pixel one from going heavy.
/// Nothing outside that range wants it — the application's own icon is drawn
/// at five hundred pixels for a dock, and `draw` would put a two-and-a-half
/// pixel line across it.
#[must_use]
pub fn stroke_for(side: f32) -> f32 {
    side * GLYPH_SCALE * STROKE_RATIO
}

/// The same glyph, at a stroke the caller chose.
pub fn draw_with_stroke(
    painter: &egui::Painter,
    rect: egui::Rect,
    icon: Icon,
    color: egui::Color32,
    stroke_width: f32,
) {
    let side = rect.width().min(rect.height()) * GLYPH_SCALE;
    let square = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(side));
    let pen = Pen {
        painter,
        rect: square,
        stroke: egui::Stroke::new(stroke_width, color),
        color,
    };
    match icon {
        Icon::Overview => overview(&pen),
        Icon::Segments => segments(&pen),
        Icon::Functions => functions(&pen),
        Icon::Strings => strings(&pen),
        Icon::Symbols => symbols(&pen),
        Icon::Classes => classes(&pen),
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
        Icon::Calculator => calculator(&pen),
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
        Icon::Structures => structures(&pen),
        Icon::Run => run(&pen),
        Icon::Restart => restart(&pen),
        Icon::Breakpoint => breakpoint(&pen),
        Icon::Warning => warning(&pen),
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
///
/// Outlined rather than filled. Three solid slabs were the heaviest mark on
/// the rail and pulled the eye to the section table over everything beside
/// it; at button size a stroked band of this height fills in anyway.
fn segments(pen: &Pen) {
    for (y, width) in [(0.04_f32, 1.0_f32), (0.38, 0.56), (0.72, 0.82)] {
        pen.boxed((0.0, y), (width, y + 0.24), 1.0);
    }
}

/// A call arriving at a routine: the arrow is the call, the rounded block the
/// body it enters.
///
/// It used to be a line splitting into two branches, which is the share icon
/// every phone has drawn for a decade — and, worse, it is control flow, which
/// is what the graph view is for. Nothing else in the set is a block an arrow
/// enters.
fn functions(pen: &Pen) {
    pen.boxed((0.44, 0.18), (1.0, 0.82), 3.0);
    pen.line((0.0, 0.5), (0.44, 0.5));
    pen.path(&[(0.22, 0.3), (0.44, 0.5), (0.22, 0.7)]);
}

/// Quotation marks over the line of text they open and close.
///
/// It was written as two mirrored chevrons, which came out as `<` and `>` —
/// the code icon, worn by the view that lists a file's *text*. The marks are
/// upright and straight rather than curled: a comma's tail is two pixels
/// across at button size and becomes a smudge.
fn strings(pen: &Pen) {
    // Barely a lean. Slanted any further the four strokes read as `// //` —
    // two code comments, which is the opposite of what this view lists.
    for x in [0.08_f32, 0.26, 0.62, 0.8] {
        pen.line((x + 0.03, 0.08), (x, 0.42));
    }
    pen.line((0.02, 0.8), (0.98, 0.8));
}

/// A list of named entries: a marker for each declared name, then the name
/// beside it. Distinct from the disassembly glyph, whose column is addresses
/// rather than the pip that stands for a symbol here.
fn symbols(pen: &Pen) {
    for (y, end) in [(0.14_f32, 0.86), (0.5, 0.98), (0.86, 0.7)] {
        pen.filled((0.02, y - 0.09), (0.2, y + 0.09), 0.4);
        pen.line((0.34, y), (end, y));
    }
}

/// A class: a header box over the members it groups, like a small class
/// diagram seen from far away.
fn classes(pen: &Pen) {
    pen.filled((0.14, 0.06), (0.86, 0.3), 0.4);
    for y in [0.52_f32, 0.78] {
        pen.line((0.24, y), (0.76, y));
    }
    pen.line((0.5, 0.3), (0.5, 0.9));
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
///
/// Two rows of three rather than three: nine cells at button size are a solid
/// block with a line beside it, and what makes this a dump — an offset column
/// against a field of bytes — is lost in the ink.
fn dump(pen: &Pen) {
    pen.line((0.0, 0.06), (0.0, 0.94));
    for y in [0.18_f32, 0.58] {
        for x in [0.3_f32, 0.6, 0.9] {
            pen.filled((x - 0.11, y), (x + 0.11, y + 0.24), 0.5);
        }
    }
}

/// Braces around uneven source lines: pseudo-C rather than a generic code
/// block, visibly distinct from the address-and-opcode listing icon.
///
/// The braces are kept to a sixth of the width at each edge. Drawn wider they
/// reached the middle of the glyph, met the lines of code and each other, and
/// the whole thing closed up into a lozenge — a shape that said nothing at
/// all, least of all "source".
fn decompile(pen: &Pen) {
    /// How far into the glyph a brace reaches. The lines of code start well
    /// clear of it.
    const REACH: f32 = 0.16;
    for (edge, inward) in [(0.0_f32, 1.0_f32), (1.0, -1.0)] {
        let spine = edge + inward * REACH;
        pen.path(&[
            (spine + inward * 0.1, 0.02),
            (spine, 0.16),
            (spine, 0.42),
            (edge, 0.5),
            (spine, 0.58),
            (spine, 0.84),
            (spine + inward * 0.1, 0.98),
        ]);
    }
    for (y, end) in [(0.26_f32, 0.7_f32), (0.5, 0.64), (0.74, 0.58)] {
        pen.line((0.36, y), (end, y));
    }
}

/// A byte lifted out of the run and put back changed.
///
/// Drawn flush in the row — a strip of cells with one of them filled — this
/// was the same picture as a structure's members, and at button size the two
/// views wore one icon between them. Lifted clear of the row, it is an edit
/// and nothing else.
fn patches(pen: &Pen) {
    pen.boxed((0.0, 0.5), (1.0, 0.94), 1.0);
    for x in [0.34_f32, 0.66] {
        pen.line((x, 0.5), (x, 0.94));
    }
    pen.filled((0.36, 0.02), (0.64, 0.34), 1.0);
    pen.line((0.5, 0.34), (0.5, 0.5));
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

/// A command field with the matches it offers under it.
///
/// It used to be a framed prompt, which is exactly what the script console
/// is: at button size the palette and the console were one glyph drawn twice.
/// What tells them apart is that the palette answers — so the answers are in
/// the picture.
fn palette(pen: &Pen) {
    pen.boxed((0.0, 0.02), (1.0, 0.44), 2.0);
    // The caret, upright, ahead of what has been typed: without it the frame
    // was just a wide band, and the whole glyph was the section table.
    pen.line((0.14, 0.13), (0.14, 0.33));
    pen.line((0.28, 0.23), (0.66, 0.23));
    for (y, end) in [(0.68_f32, 0.9_f32), (0.94, 0.64)] {
        pen.line((0.06, y), (end, y));
    }
}

/// A calculator: the body, the display band across the top, and the keys.
///
/// The outer edge is what tells it from the dump's field of bytes, which is
/// otherwise the same picture — small cells in rows. A calculator is an object
/// with a rim, and the filled band it reads out on is a mark no other glyph
/// here carries.
fn calculator(pen: &Pen) {
    pen.boxed((0.16, 0.02), (0.84, 0.98), 2.0);
    pen.filled((0.28, 0.14), (0.72, 0.32), 1.0);
    for y in [0.56_f32, 0.82] {
        for x in [0.32_f32, 0.5, 0.68] {
            pen.dot((x, y));
        }
    }
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

/// A plug: the body, and the two pins under it.
///
/// Stood upright. Lying on its side it was a block with two leads entering
/// from the left, and so was the functions icon — a block an arrow enters.
/// Turned ninety degrees it is a plug, and it is the only vertical body in
/// the set.
fn plugins(pen: &Pen) {
    pen.boxed((0.18, 0.04), (0.82, 0.6), 2.0);
    pen.line((0.36, 0.6), (0.36, 0.94));
    pen.line((0.64, 0.6), (0.64, 0.94));
}

/// Two sliders: settings that are chosen rather than toggled.
///
/// The handles are upright bars, not round pips. A line with a pip on it is
/// what the output log draws for a stamped entry, and a rail with a pip is
/// the same mark rotated: the two glyphs sat four rows apart in the same menu.
fn preferences(pen: &Pen) {
    for (y, knob) in [(0.3_f32, 0.66_f32), (0.7, 0.34)] {
        pen.line((0.0, y), (1.0, y));
        pen.filled((knob - 0.07, y - 0.18), (knob + 0.07, y + 0.18), 1.0);
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

/// Back to the first instruction: an arrow driven up against the wall the
/// program starts at.
///
/// It used to be a rifle sight — a ringed cross with a pip in it — which is
/// the mark every tool uses for "aim here" and says nothing about a beginning.
/// A bar with an arrow into it is the one every transport has used for "back
/// to the start" since tape decks had buttons.
fn walk_to_entry(pen: &Pen) {
    pen.line((0.1, 0.14), (0.1, 0.86));
    pen.line((0.24, 0.5), (0.94, 0.5));
    pen.path(&[(0.5, 0.2), (0.22, 0.5), (0.5, 0.8)]);
}

/// Undo the last step: the arrow that turns back on itself.
///
/// The same mark every editor draws for undo, which is exactly what taking a
/// step back is. What was drawn before — a rectangular loop with a head on it
/// — read as a flow chart at button size.
fn walk_back(pen: &Pen) {
    let arc: Vec<(f32, f32)> = (0..=20_u8)
        .map(|step| {
            // From the left, over the top, round to the right: an open circle
            // with its gap where the head goes.
            let angle = std::f32::consts::TAU * (0.5 + 0.62 * f32::from(step) / 20.0);
            (0.5 + 0.36 * angle.cos(), 0.56 + 0.36 * angle.sin())
        })
        .collect();
    pen.path(&arc);
    pen.path(&[(0.0, 0.32), (0.14, 0.56), (0.3, 0.36)]);
}

/// Into the call: an arrow coming down onto the instruction it lands on.
///
/// The instruction is a filled disc, not a line: what a step lands on is one
/// place, and the disc is what a debugger has drawn there for thirty years.
fn walk_into(pen: &Pen) {
    pen.line((0.5, 0.04), (0.5, 0.46));
    pen.path(&[(0.28, 0.28), (0.5, 0.5), (0.72, 0.28)]);
    pen.disc((0.5, 0.8), 0.15);
}

/// Over the call: an arc that leaves the line and comes back down to it,
/// clearing the instruction under it.
fn walk_over(pen: &Pen) {
    let arc: Vec<(f32, f32)> = (0..=20_u8)
        .map(|step| {
            let angle = std::f32::consts::PI * (1.0 + f32::from(step) / 20.0);
            (0.5 + 0.42 * angle.cos(), 0.56 + 0.4 * angle.sin())
        })
        .collect();
    pen.path(&arc);
    pen.path(&[(0.72, 0.32), (0.92, 0.56), (0.7, 0.66)]);
    pen.disc((0.5, 0.86), 0.13);
}

/// Out of the call: an arrow leaving the instruction it stands on, upwards.
///
/// The instruction stays where it is in the step-in glyph — at the bottom,
/// under the arrow — and only the arrow turns round. The pair then reads as
/// one idea seen twice, which is what they are.
fn walk_out(pen: &Pen) {
    pen.line((0.5, 0.54), (0.5, 0.06));
    pen.path(&[(0.28, 0.28), (0.5, 0.06), (0.72, 0.28)]);
    pen.disc((0.5, 0.8), 0.15);
}

/// Forget the path: the bin.
///
/// The one glyph in the set that is a household object rather than a shape,
/// and deliberately: every application on the reader's machine draws this for
/// "throw away", and a route with a small cross beside it — what was here
/// before — had to be read twice.
fn walk_clear(pen: &Pen) {
    pen.line((0.06, 0.24), (0.94, 0.24));
    pen.path(&[(0.36, 0.24), (0.36, 0.1), (0.64, 0.1), (0.64, 0.24)]);
    pen.path(&[(0.16, 0.24), (0.24, 0.96), (0.76, 0.96), (0.84, 0.24)]);
    for x in [0.4_f32, 0.6] {
        pen.line((x, 0.38), (x, 0.84));
    }
}

/// The road sign: a triangle standing on its base, with the mark inside it.
///
/// Drawn as the sign a driver knows rather than as a coloured pip, because it
/// is asked to do the same job — say *read what follows differently* before
/// anything under it has been read. A filled dot said "here is a thing"; a
/// triangle says "careful".
fn warning(pen: &Pen) {
    /// How far the corners are cut, as a fraction of the glyph. A triangle
    /// drawn to its points has three hairline spikes at button size; cutting
    /// them leaves the shape and loses the spikes.
    const CUT: f32 = 0.14;
    pen.path(&[
        (0.5 - CUT / 2.0, 0.06),
        (0.5 + CUT / 2.0, 0.06),
        (0.98, 0.88),
        (0.98 - CUT, 0.94),
        (0.02 + CUT, 0.94),
        (0.02, 0.88),
        (0.5 - CUT / 2.0, 0.06),
    ]);
    pen.line((0.5, 0.36), (0.5, 0.64));
    pen.dot((0.5, 0.79));
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

/// A run of bytes with the cuts a structure makes in it: one band, divided
/// into members of unequal width, which is what a layout is.
fn structures(pen: &Pen) {
    pen.boxed((0.04, 0.28), (0.96, 0.72), 1.0);
    pen.line((0.32, 0.28), (0.32, 0.72));
    pen.line((0.52, 0.28), (0.52, 0.72));
    pen.line((0.78, 0.28), (0.78, 0.72));
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
/// for. Every icon appears twice: once large enough to see the shape, and once
/// at the size a toolbar button gives it, which is the size that decides
/// whether it reads at all. Run with the destination in the environment to get
/// a sheet out:
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

    /// Width and height of one cell of the sheet.
    const CELL: egui::Vec2 = egui::vec2(120.0, 78.0);
    const COLUMNS: usize = 5;
    /// The glyph drawn large, to judge the shape.
    const LARGE: f32 = 56.0;
    /// And the same glyph at the size the toolbar actually draws it, which is
    /// the only size that decides whether it reads. A shape that is obvious at
    /// fifty pixels and a smudge at eighteen is a failed icon, and the sheet
    /// used to show only the flattering half of that.
    const SMALL: f32 = 30.0;

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
            let origin = egui::pos2(coordinate(column) * CELL.x, coordinate(row) * CELL.y);
            // The large one on the left, the button-sized one beside it, so
            // the eye compares them without moving.
            let places = [
                (LARGE, egui::vec2(8.0, 6.0)),
                (SMALL, egui::vec2(LARGE + 22.0, 6.0 + (LARGE - SMALL) / 2.0)),
            ];
            for (side, inset) in places {
                let mut cell = egui::Rect::NOTHING;
                let output = ctx.run(crate::testing::window_input(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::hover());
                        cell = rect;
                        draw(ui.painter(), rect, *icon, egui::Color32::WHITE);
                    });
                });
                let offset = (origin + inset) - cell.min;
                for clipped in &output.shapes {
                    emit(&clipped.shape, offset, &mut body);
                }
            }
            let _ = writeln!(
                body,
                "<text x=\"{}\" y=\"{}\" fill=\"#8a93a6\" font-size=\"8\" font-family=\"sans-serif\" text-anchor=\"middle\">{icon:?}</text>",
                origin.x + CELL.x / 2.0,
                origin.y + CELL.y - 5.0
            );
        }

        let width = coordinate(COLUMNS) * CELL.x;
        let height = coordinate(rows) * CELL.y;
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\"><rect width=\"100%\" height=\"100%\" fill=\"#1b1f27\"/>\n{body}</svg>",
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
                if rect.rect.width() > LARGE {
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
