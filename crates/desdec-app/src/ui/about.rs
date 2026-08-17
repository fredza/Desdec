use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::Text,
    ui::opening_position,
};

/// Where the source and the licences live. Written once here and shown in the
/// About window, so a reader can always find the code that read their file.
pub const REPOSITORY: &str = "https://github.com/fredza/Desdec";

/// Size assumed the first time the window opens, before egui has measured it.
const ASSUMED_SIZE: egui::Vec2 = egui::vec2(400.0, 260.0);

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::About) {
        return;
    }

    // A stable id keeps the window in place when the title is translated.
    let id = egui::Id::new("desdec.about");
    let mut open = true;
    let mut window = egui::Window::new(app.t(Text::AboutTitle))
        .id(id)
        .open(&mut open)
        .collapsible(false);
    if let Some(step) = app.dialogs.opening_step(Dialog::About) {
        window = window.current_pos(opening_position(ctx, id, step, ASSUMED_SIZE));
    }
    window.show(ctx, |ui| {
        ui.heading("Desdec");
        ui.small(version_line());
        ui.add_space(6.0);
        ui.label(app.t(Text::AboutDescription));

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        // Where the source, the licences and the issue tracker are. An
        // application that reads other people's binaries should be easy to
        // read in turn.
        ui.horizontal(|ui| {
            ui.small(format!("{} :", app.t(Text::Repository)));
            ui.hyperlink_to(egui::RichText::new(REPOSITORY).small(), REPOSITORY);
        });
        ui.horizontal(|ui| {
            ui.small(app.t(Text::LicenceLine));
            ui.hyperlink_to(
                egui::RichText::new("Apache-2.0").small(),
                format!("{REPOSITORY}/blob/main/LICENSE-APACHE"),
            );
            ui.hyperlink_to(
                egui::RichText::new("MIT").small(),
                format!("{REPOSITORY}/blob/main/LICENSE-MIT"),
            );
        });

        ui.add_space(8.0);
        ui.small(app.t(Text::LegalNotice));
    });
    app.dialogs.set(Dialog::About, open);
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

    /// The window must offer the source and the licences, in every language:
    /// a tool that reads other people's binaries has to be readable itself.
    #[test]
    fn the_about_window_links_to_the_source_and_the_licences() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, crate::app::WorkspaceView::Overview);
        app.dialogs.open(Dialog::About);

        for language in crate::i18n::Language::ALL {
            app.preferences.language = *language;
            // A window is measured on the frame it appears and painted on the
            // next, so the first frame of a fresh context draws nothing.
            let _ = ctx.run(crate::testing::window_input(), |ctx| show(&mut app, ctx));
            let output = ctx.run(crate::testing::window_input(), |ctx| show(&mut app, ctx));
            let drawn = crate::testing::drawn_text(&output.shapes);
            assert!(drawn.contains(REPOSITORY), "{language:?}: no repository");
            assert!(drawn.contains("Apache-2.0"), "{language:?}: no Apache link");
            assert!(drawn.contains("MIT"), "{language:?}: no MIT link");
        }
    }

    #[test]
    fn version_line_carries_a_build_identifier() {
        let build = env!("DESDEC_BUILD");
        assert!(!build.is_empty(), "the build script left the build empty");
        assert!(version_line().ends_with(build));
    }
}
