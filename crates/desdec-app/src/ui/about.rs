use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::Text,
    ui::opening_position,
};

/// Where the source and the licences live. Written once here and shown in the
/// About window, so a reader can always find the code that read their file.
pub const REPOSITORY: &str = "https://github.com/fredza/Desdec";

/// Who stands behind a release, in the same words the signature carries.
///
/// The releases are signed, and a reader who checks one is told this. Printing
/// the same line in the window is what lets them compare the two without
/// taking either on trust.
pub const AUTHOR: &str = "Frédéric Zawalski @2026 bdom";

/// Fingerprint of the key the releases are signed with, in the grouping GPG
/// prints, so it can be read off the screen and compared character by
/// character.
pub const SIGNING_KEY: &str = "C9A3 1D07 46E0 65C4 E2EA  33F6 08FA 1D81 8A91 F329";

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
                egui::RichText::new("PolyForm Noncommercial 1.0.0").small(),
                format!("{REPOSITORY}/blob/main/LICENSE"),
            );
        });

        ui.horizontal(|ui| {
            ui.small(app.t(Text::SignedBy));
            ui.small(egui::RichText::new(AUTHOR).strong());
        });
        ui.small(egui::RichText::new(SIGNING_KEY).monospace().size(9.0))
            .on_hover_text(app.t(Text::SigningKeyHint));

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

    /// The window says who signs the releases and with which key. A reader who
    /// has just checked a signature compares what GPG told them with what the
    /// application says; the two have to be the same words, and the same key,
    /// or the check answers nothing.
    #[test]
    fn the_window_names_the_signature_the_releases_carry() {
        let ctx = egui::Context::default();
        let mut app = crate::app::DesdecApp::for_test(None, crate::app::WorkspaceView::Overview);
        app.dialogs.open(Dialog::About);

        // Two frames: the window is measured on the first and painted after.
        let _ = ctx.run(crate::testing::window_input(), |ctx| show(&mut app, ctx));
        let output = ctx.run(crate::testing::window_input(), |ctx| show(&mut app, ctx));

        let drawn = crate::testing::drawn_text(&output.shapes);
        assert!(drawn.contains(AUTHOR), "the window must name who signs");
        assert!(
            drawn.contains(SIGNING_KEY),
            "and the key, so it can be compared with what GPG printed"
        );
    }

    /// The fingerprint is quoted in three README files, a release note and a
    /// signing script. One of them drifting would send a reader to compare
    /// against the wrong key, which is worse than not quoting it at all.
    #[test]
    fn the_fingerprint_is_the_same_everywhere_it_is_quoted() {
        let bare: String = SIGNING_KEY.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(bare.len(), 40, "a fingerprint is forty hexadecimal digits");
        assert!(bare.chars().all(|c| c.is_ascii_hexdigit()));

        for (path, quoted) in [
            ("../../README.md", SIGNING_KEY),
            ("../../README.fr.md", SIGNING_KEY),
            ("../../README.es.md", SIGNING_KEY),
            ("../../scripts/sign-release.sh", bare.as_str()),
            ("../../.github/workflows/platform-binaries.yml", SIGNING_KEY),
        ] {
            let at = concat!(env!("CARGO_MANIFEST_DIR"), "/").to_owned() + path;
            let text = std::fs::read_to_string(&at).unwrap_or_else(|_| panic!("{at} is readable"));
            assert!(text.contains(quoted), "{path} quotes a different key");
        }
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
            assert!(
                drawn.contains("PolyForm Noncommercial 1.0.0"),
                "{language:?}: no PolyForm link"
            );
        }
    }

    #[test]
    fn version_line_carries_a_build_identifier() {
        let build = env!("DESDEC_BUILD");
        assert!(!build.is_empty(), "the build script left the build empty");
        assert!(version_line().ends_with(build));
    }
}
