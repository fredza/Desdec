use eframe::egui;

#[derive(Clone, Copy)]
pub enum Icon {
    Overview,
    Disassembly,
    Functions,
    Strings,
    Patches,
    Open,
    Palette,
}

pub fn button(
    ui: &mut egui::Ui,
    icon: Icon,
    tooltip: Option<String>,
    selected: bool,
    accent: egui::Color32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(34.0, 30.0), egui::Sense::click());
    let visuals = ui.style().interact(&response);
    let fill = if selected {
        accent.gamma_multiply(0.42)
    } else if response.hovered() {
        visuals.bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    if fill != egui::Color32::TRANSPARENT {
        ui.painter().rect_filled(rect.shrink(1.0), 5.0, fill);
    }
    draw(
        ui.painter(),
        rect.shrink(7.0),
        icon,
        visuals.fg_stroke.color,
    );
    if let Some(tooltip) = tooltip {
        response.on_hover_text(tooltip)
    } else {
        response
    }
}

fn draw(painter: &egui::Painter, rect: egui::Rect, icon: Icon, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.8_f32, color);
    let center = rect.center();
    match icon {
        Icon::Overview => {
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
        Icon::Disassembly => {
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
        Icon::Functions => {
            painter.line_segment(
                [
                    egui::pos2(rect.left() + 2.0, center.y),
                    egui::pos2(center.x, center.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y),
                    egui::pos2(rect.right() - 2.0, rect.top() + 2.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y),
                    egui::pos2(rect.right() - 2.0, rect.bottom() - 2.0),
                ],
                stroke,
            );
            for point in [
                egui::pos2(rect.left() + 2.0, center.y),
                egui::pos2(rect.right() - 2.0, rect.top() + 2.0),
                egui::pos2(rect.right() - 2.0, rect.bottom() - 2.0),
            ] {
                painter.circle_filled(point, 2.0, color);
            }
        }
        Icon::Strings => {
            painter.line_segment(
                [
                    egui::pos2(rect.left() + 3.0, rect.top() + 2.0),
                    egui::pos2(rect.left(), rect.top() + 6.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(rect.left(), rect.top() + 6.0),
                    egui::pos2(rect.left() + 3.0, rect.top() + 10.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(rect.right() - 3.0, rect.top() + 2.0),
                    egui::pos2(rect.right(), rect.top() + 6.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(rect.right(), rect.top() + 6.0),
                    egui::pos2(rect.right() - 3.0, rect.top() + 10.0),
                ],
                stroke,
            );
        }
        Icon::Patches => {
            painter.line_segment(
                [
                    egui::pos2(rect.left() + 2.0, rect.bottom() - 2.0),
                    egui::pos2(rect.right() - 2.0, rect.top() + 2.0),
                ],
                egui::Stroke::new(3.0_f32, color),
            );
            painter.line_segment(
                [
                    egui::pos2(rect.left() + 1.0, rect.bottom() - 1.0),
                    egui::pos2(rect.left() + 5.0, rect.bottom() - 2.0),
                ],
                stroke,
            );
        }
        Icon::Open => {
            let flap = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.top() + 3.0),
                egui::pos2(center.x, rect.top() + 7.0),
            );
            let folder = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.top() + 6.0),
                egui::pos2(rect.right(), rect.bottom() - 1.0),
            );
            painter.rect_stroke(flap, 1.0, stroke, egui::StrokeKind::Inside);
            painter.rect_stroke(folder, 1.5, stroke, egui::StrokeKind::Inside);
        }
        Icon::Palette => {
            painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [
                    egui::pos2(rect.left() + 3.0, center.y),
                    egui::pos2(rect.right() - 3.0, center.y),
                ],
                stroke,
            );
            painter.circle_filled(egui::pos2(rect.left() + 4.0, rect.top() + 4.0), 1.5, color);
        }
    }
}
