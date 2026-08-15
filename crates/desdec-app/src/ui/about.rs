use eframe::egui;

use crate::{app::DesdecApp, i18n::Text};

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.about {
        return;
    }

    let mut open = true;
    egui::Window::new(app.t(Text::AboutTitle))
        // A stable id keeps the window in place when the title is translated.
        .id(egui::Id::new("desdec.about"))
        .open(&mut open)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.heading("Desdec");
            ui.small(version_line());
            ui.label(app.t(Text::AboutDescription));
            ui.small(app.t(Text::LegalNotice));
        });
    app.dialogs.about = open;
}

/// The released version, next to the build it was produced from.
///
/// Both are baked in at compile time: the version by Cargo from `Cargo.toml`,
/// the build identifier by the build script. Neither can drift from the crate
/// they describe. The line holds no word, so it needs no translation.
fn version_line() -> String {
    format!("v{} · {}", env!("CARGO_PKG_VERSION"), env!("DESDEC_BUILD"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_reports_the_crate_version() {
        let line = version_line();
        assert!(
            line.starts_with(concat!("v", env!("CARGO_PKG_VERSION"), " · ")),
            "unexpected version line: {line}"
        );
    }

    #[test]
    fn version_line_carries_a_build_identifier() {
        let build = env!("DESDEC_BUILD");
        assert!(!build.is_empty(), "the build script left the build empty");
        assert!(version_line().ends_with(build));
    }
}
