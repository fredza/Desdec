//! Vector icons drawn by the application itself: no icon font, no external
//! asset, and identical rendering on every platform.

use eframe::egui;

#[derive(Clone, Copy)]
pub enum Icon {
    Overview,
    Disassembly,
    Decompile,
    Functions,
    Strings,
    Patches,
    Open,
    Palette,
}

const BUTTON_SIZE: egui::Vec2 = egui::vec2(34.0, 30.0);
/// Distance between the button edge and the glyph.
const GLYPH_PADDING: f32 = 7.0;
const CORNER_RADIUS: f32 = 5.0;
const STROKE_WIDTH: f32 = 1.8;
/// Heavier stroke, for the one line that carries an icon's meaning.
const EMPHASIS_STROKE_WIDTH: f32 = 3.0;
/// Opacity of the accent colour behind the selected action.
const SELECTED_FILL: f32 = 0.42;

pub fn button(
    ui: &mut egui::Ui,
    icon: Icon,
    tooltip: Option<String>,
    selected: bool,
    accent: egui::Color32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(BUTTON_SIZE, egui::Sense::click());
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
    draw(
        ui.painter(),
        rect.shrink(GLYPH_PADDING),
        icon,
        visuals.fg_stroke.color,
    );

    match tooltip {
        Some(tooltip) => response.on_hover_text(tooltip),
        None => response,
    }
}

fn draw(painter: &egui::Painter, rect: egui::Rect, icon: Icon, color: egui::Color32) {
    let stroke = egui::Stroke::new(STROKE_WIDTH, color);
    match icon {
        Icon::Overview => overview(painter, rect, stroke),
        Icon::Disassembly => disassembly(painter, rect, stroke),
        Icon::Decompile => decompile(painter, rect, stroke),
        Icon::Functions => functions(painter, rect, stroke, color),
        Icon::Strings => strings(painter, rect, stroke),
        Icon::Patches => patches(painter, rect, stroke, color),
        Icon::Open => open(painter, rect, stroke),
        Icon::Palette => palette(painter, rect, stroke, color),
    }
}

/// Four panes: the workspace seen from above.
fn overview(painter: &egui::Painter, rect: egui::Rect, stroke: egui::Stroke) {
    let center = rect.center();
    for x in [rect.left(), center.x + 1.0] {
        for y in [rect.top(), center.y + 1.0] {
            painter.rect_stroke(
                egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(5.0, 5.0)),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
    }
}

/// Stacked instruction lines of uneven length.
fn disassembly(painter: &egui::Painter, rect: egui::Rect, stroke: egui::Stroke) {
    for (offset, width) in [(0.0, 13.0), (5.0, 9.0), (10.0, 13.0)] {
        painter.line_segment(
            [
                egui::pos2(rect.left(), rect.top() + offset),
                egui::pos2(rect.left() + width, rect.top() + offset),
            ],
            stroke,
        );
    }
}

/// A pair of braces: the pseudo-code rebuilt from those instructions. The
/// disassembly keeps the stacked lines, so the two toolbar actions no longer
/// wear the same glyph.
fn decompile(painter: &egui::Painter, rect: egui::Rect, stroke: egui::Stroke) {
    let (top, bottom, middle) = (rect.top() + 1.0, rect.bottom() - 1.0, rect.center().y);
    for (edge, inward) in [(rect.left() + 1.0, 1.0_f32), (rect.right() - 1.0, -1.0)] {
        let spine = edge + inward * 3.0;
        painter.add(egui::Shape::line(
            vec![
                egui::pos2(spine + inward * 3.0, top),
                egui::pos2(spine, top + 3.0),
                egui::pos2(spine, middle - 2.0),
                egui::pos2(edge, middle),
                egui::pos2(spine, middle + 2.0),
                egui::pos2(spine, bottom - 3.0),
                egui::pos2(spine + inward * 3.0, bottom),
            ],
            stroke,
        ));
    }
}

/// A call splitting into two branches.
fn functions(
    painter: &egui::Painter,
    rect: egui::Rect,
    stroke: egui::Stroke,
    color: egui::Color32,
) {
    let center = rect.center();
    let entry = egui::pos2(rect.left() + 2.0, center.y);
    let upper = egui::pos2(rect.right() - 2.0, rect.top() + 2.0);
    let lower = egui::pos2(rect.right() - 2.0, rect.bottom() - 2.0);

    painter.line_segment([entry, center], stroke);
    painter.line_segment([center, upper], stroke);
    painter.line_segment([center, lower], stroke);
    for point in [entry, upper, lower] {
        painter.circle_filled(point, 2.0, color);
    }
}

/// Opening and closing quotation marks.
fn strings(painter: &egui::Painter, rect: egui::Rect, stroke: egui::Stroke) {
    for (outer, inner) in [
        (rect.left() + 3.0, rect.left()),
        (rect.right() - 3.0, rect.right()),
    ] {
        painter.line_segment(
            [
                egui::pos2(outer, rect.top() + 2.0),
                egui::pos2(inner, rect.top() + 6.0),
            ],
            stroke,
        );
        painter.line_segment(
            [
                egui::pos2(inner, rect.top() + 6.0),
                egui::pos2(outer, rect.top() + 10.0),
            ],
            stroke,
        );
    }
}

/// A patch applied with a thick diagonal stroke.
fn patches(painter: &egui::Painter, rect: egui::Rect, stroke: egui::Stroke, color: egui::Color32) {
    painter.line_segment(
        [
            egui::pos2(rect.left() + 2.0, rect.bottom() - 2.0),
            egui::pos2(rect.right() - 2.0, rect.top() + 2.0),
        ],
        egui::Stroke::new(EMPHASIS_STROKE_WIDTH, color),
    );
    painter.line_segment(
        [
            egui::pos2(rect.left() + 1.0, rect.bottom() - 1.0),
            egui::pos2(rect.left() + 5.0, rect.bottom() - 2.0),
        ],
        stroke,
    );
}

/// An open folder.
fn open(painter: &egui::Painter, rect: egui::Rect, stroke: egui::Stroke) {
    let flap = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.top() + 3.0),
        egui::pos2(rect.center().x, rect.top() + 7.0),
    );
    let folder = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.top() + 6.0),
        egui::pos2(rect.right(), rect.bottom() - 1.0),
    );
    painter.rect_stroke(flap, 1.0, stroke, egui::StrokeKind::Inside);
    painter.rect_stroke(folder, 1.5, stroke, egui::StrokeKind::Inside);
}

/// A command input with its prompt.
fn palette(painter: &egui::Painter, rect: egui::Rect, stroke: egui::Stroke, color: egui::Color32) {
    painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
    painter.line_segment(
        [
            egui::pos2(rect.left() + 3.0, rect.center().y),
            egui::pos2(rect.right() - 3.0, rect.center().y),
        ],
        stroke,
    );
    painter.circle_filled(egui::pos2(rect.left() + 4.0, rect.top() + 4.0), 1.5, color);
}
