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

/// The reference binary with its symbol table taken away.
///
/// What most files worth reading actually look like, and the state the
/// Functions view used to be empty in.
#[must_use]
pub fn stripped_app(view: WorkspaceView) -> DesdecApp {
    let mut analysis = reference_analysis().clone();
    analysis.symbols.clear();
    let mut app = DesdecApp::for_test(Some(analysis), view);
    app.file_bytes = reference_bytes().to_vec();
    app
}

/// The first sample whose architecture the emulator has an interpreter for.
///
/// A test that runs anything must not use the host's own binary: on an Apple
/// Silicon runner that binary is `AArch64`, which is decoded and read like any
/// other but has no processor here, so every such test failed there and only
/// there. The fixtures carry a real `x86-64` one whatever the host is.
#[must_use]
pub fn emulatable_sample() -> Sample {
    samples()
        .into_iter()
        .find(|sample| {
            matches!(
                sample.analysis.summary.architecture,
                desdec_core::Architecture::X86_64 | desdec_core::Architecture::X86
            )
        })
        .expect("a fixture the emulator can run")
}

/// A window large enough for a view to lay out as it would on screen.
pub fn window_input() -> eframe::egui::RawInput {
    // A real window is whatever the reader's screen is, and a view that only
    // ever laid out at one width is one whose columns were never asked to
    // share a wide one. `DESDEC_WIDTH` is for looking at that.
    let size = window_size();
    eframe::egui::RawInput {
        screen_rect: Some(eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            size,
        )),
        ..Default::default()
    }
}

/// How big the window a test lays out in is.
///
/// `DESDEC_WIDTH` and `DESDEC_HEIGHT` override it, for looking at a view at
/// the size a real screen gives it: the default is deliberately small, and a
/// view that only ever laid out at one size is one whose panes were never
/// asked to share a large one.
#[must_use]
pub fn window_size() -> eframe::egui::Vec2 {
    let read = |name: &str, fallback: f32| {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(fallback)
    };
    eframe::egui::vec2(read("DESDEC_WIDTH", 1200.0), read("DESDEC_HEIGHT", 800.0))
}

/// A window wide enough for the listing to show all five of its columns.
///
/// The listing holds each column to the widest thing the file can put in it —
/// a fifteen-byte instruction is forty-six characters of bytes alone — so that
/// nothing moves as the reader scrolls. That costs width, and the listing
/// shares the workspace with the pseudo-code beside it, so in the ordinary
/// test window the last columns are past the right edge and have to be
/// scrolled to. A test about what the listing *says* should not be a test of
/// what fits in six hundred pixels wide.
#[must_use]
pub fn listing_window_input() -> eframe::egui::RawInput {
    let mut input = window_input();
    if let Some(rect) = input.screen_rect.as_mut() {
        rect.max.x = rect.min.x + 1800.0;
    }
    input
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
///
/// Two more say what to draw. `DESDEC_VIEW` names the view, by the English
/// label of its own command — every view is worth looking at, and this file
/// used to have to be edited to see any but the first. `DESDEC_RUN` starts the
/// emulation and steps it a few instructions, with a breakpoint on the entry
/// point and the listing scrolled to where the run stands: the marks a run
/// leaves are the whole of what there is to look at, and a listing showing its
/// first page carries none of them.
///
/// ```text
/// DESDEC_FRAME=/tmp/f.svg DESDEC_VIEW=Machine cargo test -p desdec-app frame_sheet
/// DESDEC_FRAME=/tmp/f.svg DESDEC_VIEW=Disassembly DESDEC_RUN=1 cargo test -p desdec-app frame_sheet
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
        // The update windows, on demand: they are the two a reader meets
        // without having gone looking for them, so they are the two most worth
        // looking at. `DESDEC_UPDATE=consent` or `=offer`.
        match std::env::var("DESDEC_UPDATE").as_deref() {
            Ok("consent") => {
                app.dialogs.close(Dialog::Plugins);
                app.dialogs.close(Dialog::Console);
                app.preferences.check_for_updates = None;
                app.dialogs.open(Dialog::UpdateConsent);
            }
            Ok("offer") => {
                app.dialogs.close(Dialog::Plugins);
                app.dialogs.close(Dialog::Console);
                app.preferences.check_for_updates = Some(true);
                app.update = crate::app::UpdateState::Offered(Box::new(
                    desdec_core::update::Release {
                        version: desdec_core::update::Version::parse("0.4.0").expect("a version"),
                        tag: String::from("v0.4.0"),
                        notes: String::from(
                            "## Ce qui change\n\n* Une machine émulée : registres, mémoire, pile, points d'arrêt.\n* Les colonnes du listing ne bougent plus au défilement.\n* Trois traductions à jour.",
                        ),
                        page: String::from("https://github.com/fredza/Desdec/releases/tag/v0.4.0"),
                        published: String::from("2026-08-20T00:00:00Z"),
                        archive: desdec_core::update::Asset {
                            name: String::from("desdec-linux-x86_64-release.tar.gz"),
                            url: String::new(),
                            size: 9_199_178,
                        },
                        checksum: None,
                    },
                ));
                app.dialogs.open(Dialog::Update);
            }
            _ => {}
        }

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
    let size = window_size();
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\"><rect width=\"100%\" height=\"100%\" fill=\"#0d1017\"/>\n{body}</svg>",
        width = size.x,
        height = size.y
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
        // Which view to draw. Every view is worth looking at, and a sheet that
        // could only ever show the first one meant editing this file to look
        // at any other.
        let view = view_named(std::env::var("DESDEC_VIEW").unwrap_or_default().as_str());
        // `DESDEC_STRIP=1` takes the symbol table away, which is the state
        // most files worth reading are in.
        let mut app = if std::env::var("DESDEC_STRIP").is_ok() {
            super::stripped_app(view)
        } else {
            opened_app(view)
        };
        app.navigation_open = true;
        // A sheet of the listing is worth little without a run on it: the
        // marks a run leaves are exactly what a reader has to look at.
        if std::env::var("DESDEC_RUN").is_ok() {
            let entry = app.analysis.as_ref().and_then(|a| a.entry_point);
            app.selected_instruction = entry;
            if let (Some(entry), Some(machine)) = (entry, app.machine()) {
                machine.toggle_breakpoint(entry);
                for _ in 0..3 {
                    machine.step_one();
                }
            }
            // And scrolled to where the run stands: a listing showing its
            // first page says nothing about a run a hundred thousand rows down.
            app.follow_the_run();
        }
        // A breakpoint carrying a condition, so the pane that edits them has
        // something in it to look at.
        if std::env::var("DESDEC_BREAKPOINT").is_ok() {
            let entry = app.analysis.as_ref().and_then(|a| a.entry_point);
            if let (Some(entry), Some(machine)) = (entry, app.machine()) {
                // Whatever `DESDEC_RUN` already put there keeps its place: the
                // two are meant to be usable together.
                if !machine.has_breakpoint(entry) {
                    machine.toggle_breakpoint(entry);
                }
                if let Some(breakpoint) = machine.breakpoint_mut(entry) {
                    let _ = breakpoint.set_condition("rcx == 0 && [rdi]:1 != 0");
                    breakpoint.skip = 3;
                }
                let second = entry.wrapping_add(4);
                if !machine.has_breakpoint(second) {
                    machine.toggle_breakpoint(second);
                }
                if let Some(breakpoint) = machine.breakpoint_mut(second) {
                    breakpoint.enabled = false;
                }
                for _ in 0..6 {
                    machine.step_one();
                }
            }
        }

        // Two frames: panels are measured on the first and painted after.
        let _ = ctx.run(window_input(), |ctx| app.run_frame(ctx));
        let output = ctx.run(window_input(), |ctx| app.run_frame(ctx));

        super::write_svg(&output.shapes, &path);
    }

    /// The view a sheet was asked for, by the name its command carries.
    ///
    /// Matched against the English label so the name does not change with the
    /// reader's language, and falling back to the overview.
    fn view_named(name: &str) -> WorkspaceView {
        WorkspaceView::ALL
            .iter()
            .copied()
            .find(|view| {
                crate::i18n::text(crate::i18n::Language::English, view.text())
                    .eq_ignore_ascii_case(name)
            })
            .unwrap_or(WorkspaceView::Overview)
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

    /// One run of text, and whatever is painted behind it.
    ///
    /// Its own function because it is the longest arm by far and the one that
    /// grows: the background alone took a dozen lines to find.
    fn emit_text(text: &egui::epaint::TextShape, out: &mut String) {
        // What is painted *behind* the text. It is not a shape of its
        // own and not in the row's mesh either: it is a property of
        // the layout job's sections, and the tessellator turns it into
        // rectangles only after the frame is handed over. A sheet that
        // did not read it here showed no selection, no patched byte
        // and no mark a run leaves — every one of which is a colour
        // behind text, and the only thing distinguishing those rows.
        //
        // Drawn as one rectangle over the whole galley rather than one
        // per section: the sections' own widths are not measured until
        // tessellation, and what a reader needs from a sheet is which
        // rows are marked, not by how many pixels.
        if let Some(behind) = text
            .galley
            .job
            .sections
            .iter()
            .map(|section| section.format.background)
            .find(|fill| *fill != egui::Color32::TRANSPARENT)
        {
            let size = text.galley.size();
            let _ = writeln!(
                out,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
                text.pos.x,
                text.pos.y,
                size.x,
                size.y,
                colour(behind)
            );
        }
        // Row by row rather than as one string: a galley that wrapped, or one
        // carrying newlines, is several lines on screen. Writing them as one
        // made a paragraph look like a single line running off the right edge
        // — a rendering fault that was not there, which hides the ones that
        // are.
        let shade = colour(if text.fallback_color == egui::Color32::PLACEHOLDER {
            egui::Color32::WHITE
        } else {
            text.fallback_color
        });
        for row in &text.galley.rows {
            let content = row
                .text()
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            if content.trim().is_empty() {
                continue;
            }
            let at = text.pos + row.rect.min.to_vec2();
            let _ = writeln!(
                out,
                "<text x=\"{}\" y=\"{}\" fill=\"{shade}\" font-size=\"12\" font-family=\"sans-serif\">{content}</text>",
                at.x,
                at.y + 11.0,
            );
        }
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
            egui::Shape::Text(text) => emit_text(text, out),
            egui::Shape::Circle(circle) => {
                let _ = writeln!(
                    out,
                    "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{}\"/>",
                    circle.center.x,
                    circle.center.y,
                    circle.radius,
                    colour(circle.fill)
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
                // The fill as well as the stroke: a filled shape with no
                // stroke — which is what a solid glyph is — was drawn as
                // nothing at all, so the one icon painted that way was
                // invisible on the sheet while being perfectly visible in the
                // application. The same fault Circle had.
                let _ = writeln!(
                    out,
                    "<polyline points=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/>",
                    points.join(" "),
                    if path.fill == egui::Color32::TRANSPARENT {
                        "none".to_owned()
                    } else {
                        colour(path.fill)
                    },
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
