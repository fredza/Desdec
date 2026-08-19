//! Fixtures shared by the tests.
//!
//! Nearly every test here wants a real analysis of a real binary, and the test
//! executable is the one binary certainly present, in the host's own format.
//! Analysing it costs seconds, and the suite used to pay that price once per
//! test: it is analysed once here and lent to everyone who asks.
//!
//! The reference binary is the host's, so what these fixtures exercise is the
//! host's format and architecture. Tests that must hold for a format the host
//! does not use build their own bytes instead.

use crate::app::{DesdecApp, WorkspaceView};
use desdec_core::Analysis;
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// Path of the test executable.
pub fn reference_path() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| std::env::current_exe().expect("the test binary has a path"))
}

/// Its bytes, read once.
pub fn reference_bytes() -> &'static [u8] {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| std::fs::read(reference_path()).expect("the test binary is readable"))
}

/// One analysis of it, shared by the whole suite.
///
/// Borrowed rather than handed over: a test that needs to alter it clones what
/// it needs, so no test can leave a modified analysis behind for the next.
pub fn reference_analysis() -> &'static Analysis {
    static ANALYSIS: OnceLock<Analysis> = OnceLock::new();
    ANALYSIS.get_or_init(|| {
        desdec_core::analyse_path(reference_path()).expect("the test binary is analysable")
    })
}

/// An application with the reference binary open, as opening one really leaves
/// it: analysis, derived function index and file bytes all in place.
pub fn opened_app(view: WorkspaceView) -> DesdecApp {
    let mut app = DesdecApp::for_test(Some(reference_analysis().clone()), view);
    app.file_bytes = reference_bytes().to_vec();
    app
}

/// A synthetic binary, analysed, with what it was built to contain.
///
/// The reference binary above is whatever the host is; these are one ELF, one
/// PE and one Mach-O, so a reader that only ever runs on Linux still has its
/// Mach-O and PE paths exercised — and a view that shows nothing for a format
/// is caught here rather than by whoever opens such a file.
pub struct Sample {
    pub fixture: desdec_core::fixtures::Fixture,
    pub analysis: Analysis,
}

impl Sample {
    /// An application with this binary open, as opening one really leaves it.
    pub fn opened(&self, view: WorkspaceView) -> DesdecApp {
        let mut app = DesdecApp::for_test(Some(self.analysis.clone()), view);
        app.file_bytes.clone_from(&self.fixture.bytes);
        app
    }
}

/// One sample per format the analysis reads.
pub fn samples() -> Vec<Sample> {
    desdec_core::fixtures::all()
        .into_iter()
        .map(|fixture| {
            let analysis = desdec_core::analyse_bytes(
                Path::new("fixture.bin"),
                fixture.bytes.len() as u64,
                &fixture.bytes,
            );
            Sample { fixture, analysis }
        })
        .collect()
}

/// A window large enough for a view to lay out as it would on screen.
pub fn window_input() -> eframe::egui::RawInput {
    eframe::egui::RawInput {
        screen_rect: Some(eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(1200.0, 800.0),
        )),
        ..Default::default()
    }
}

/// A frame of that window in which the primary button is pressed somewhere.
///
/// The press, not the release: that is what the interface reacts to, and it is
/// what lets a window be dragged by its title bar and let go anywhere.
pub fn press_at(position: eframe::egui::Pos2) -> eframe::egui::RawInput {
    let mut input = window_input();
    input.events = vec![
        eframe::egui::Event::PointerMoved(position),
        eframe::egui::Event::PointerButton {
            pos: position,
            button: eframe::egui::PointerButton::Primary,
            pressed: true,
            modifiers: eframe::egui::Modifiers::NONE,
        },
    ];
    input
}

/// A frame in which the primary button is pressed and let go at a position.
///
/// A whole click, unlike [`press_at`]: a widget only reports one once the
/// button has come back up over it.
pub fn click_at(position: eframe::egui::Pos2) -> eframe::egui::RawInput {
    let mut input = press_at(position);
    input.events.push(eframe::egui::Event::PointerButton {
        pos: position,
        button: eframe::egui::PointerButton::Primary,
        pressed: false,
        modifiers: eframe::egui::Modifiers::NONE,
    });
    input
}

/// Every string a frame actually drew, with where it was drawn.
///
/// What a virtualised view leaves out is as much of its behaviour as what it
/// shows, and only the shapes say which is which. The position is what lets a
/// test click on what it can see.
pub fn drawn(shapes: &[eframe::egui::epaint::ClippedShape]) -> Vec<(String, eframe::egui::Pos2)> {
    fn walk(shape: &eframe::egui::Shape, out: &mut Vec<(String, eframe::egui::Pos2)>) {
        match shape {
            eframe::egui::Shape::Text(text) => {
                out.push((text.galley.text().to_owned(), text.pos));
            }
            eframe::egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// The text of everything a frame drew, joined.
pub fn drawn_text(shapes: &[eframe::egui::epaint::ClippedShape]) -> String {
    drawn(shapes).into_iter().map(|(text, _)| text).collect()
}

/// Renders one whole frame to an SVG, for a human to look at.
///
/// A test can say that shapes were drawn; it cannot say whether what came out
/// looks like a menu. Run with a destination in the environment:
///
/// ```text
/// DESDEC_FRAME=/tmp/frame.svg cargo test -p desdec-app frame_sheet
/// ```
/// Renders the windows to an SVG, for a human to look at.
///
/// The companion of [`frame_sheet`], for what that one cannot show: a window
/// is drawn over the workspace and only when it is open. Two consecutive small
/// labels overlapping exactly is the kind of thing every assertion here passes
/// and no reader would accept.
///
/// ```text
/// DESDEC_WINDOWS=/tmp/windows.svg cargo test -p desdec-app window_sheet
/// ```
#[cfg(test)]
mod window_sheet {
    use super::*;
    use crate::app::Dialog;
    use eframe::egui;

    #[test]
    fn window_sheet() {
        let Ok(path) = std::env::var("DESDEC_WINDOWS") else {
            return;
        };
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.script.source =
            "for f in functions() {\n  if f.size > 512 { bookmark(f.address); }\n}".to_owned();
        app.dialogs.open(Dialog::Console);
        app.run_command(&ctx, crate::commands::Command::RunScript);
        app.script.vocabulary_open = true;
        app.plugins = crate::plugins::read(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins"),
        );
        app.dialogs.open(Dialog::Plugins);

        let _ = ctx.run(window_input(), |ctx| app.run_frame(ctx));
        let output = ctx.run(window_input(), |ctx| app.run_frame(ctx));
        write_svg(&output.shapes, &path);
    }
}

/// Writes one frame's shapes to an SVG file.
#[cfg(test)]
pub fn write_svg(shapes: &[eframe::egui::epaint::ClippedShape], path: &str) {
    let mut body = String::new();
    for clipped in shapes {
        frame_sheet::emit(&clipped.shape, &mut body);
    }
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1200\" height=\"800\" viewBox=\"0 0 1200 800\"><rect width=\"100%\" height=\"100%\" fill=\"#0d1017\"/>\n{body}</svg>"
    );
    std::fs::write(path, svg).expect("writable");
}

#[cfg(test)]
mod frame_sheet {
    use super::*;
    use eframe::egui;
    use std::fmt::Write as _;

    #[test]
    fn frame_sheet() {
        let Ok(path) = std::env::var("DESDEC_FRAME") else {
            return;
        };
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Overview);
        app.navigation_open = true;

        // Two frames: panels are measured on the first and painted after.
        let _ = ctx.run(window_input(), |ctx| app.run_frame(ctx));
        let output = ctx.run(window_input(), |ctx| app.run_frame(ctx));

        super::write_svg(&output.shapes, &path);
    }

    pub fn colour(c: egui::Color32) -> String {
        format!(
            "rgba({},{},{},{})",
            c.r(),
            c.g(),
            c.b(),
            f32::from(c.a()) / 255.0
        )
    }

    pub fn emit(shape: &egui::Shape, out: &mut String) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    emit(shape, out);
                }
            }
            egui::Shape::Rect(rect) => {
                let _ = writeln!(
                    out,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>",
                    rect.rect.left(),
                    rect.rect.top(),
                    rect.rect.width().max(0.0),
                    rect.rect.height().max(0.0),
                    colour(rect.fill),
                    colour(rect.stroke.color),
                    rect.stroke.width
                );
            }
            egui::Shape::Text(text) => {
                let content = text
                    .galley
                    .text()
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                let _ = writeln!(
                    out,
                    "<text x=\"{}\" y=\"{}\" fill=\"{}\" font-size=\"12\" font-family=\"sans-serif\">{content}</text>",
                    text.pos.x,
                    text.pos.y + 11.0,
                    colour(if text.fallback_color == egui::Color32::PLACEHOLDER {
                        egui::Color32::WHITE
                    } else {
                        text.fallback_color
                    })
                );
            }
            egui::Shape::LineSegment { points, stroke } => {
                let _ = writeln!(
                    out,
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>",
                    points[0].x,
                    points[0].y,
                    points[1].x,
                    points[1].y,
                    colour(stroke.color),
                    stroke.width
                );
            }
            egui::Shape::Path(path) => {
                let points: Vec<String> = path
                    .points
                    .iter()
                    .map(|p| format!("{},{}", p.x, p.y))
                    .collect();
                let _ = writeln!(
                    out,
                    "<polyline points=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"/>",
                    points.join(" "),
                    match path.stroke.color {
                        egui::epaint::ColorMode::Solid(solid) => colour(solid),
                        egui::epaint::ColorMode::UV(_) => "#ffffff".to_owned(),
                    },
                    path.stroke.width
                );
            }
            _ => {}
        }
    }
}
