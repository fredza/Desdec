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
