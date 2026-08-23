//! Two windows about versions: whether to ask, and what was found.
//!
//! The consent one comes first and comes once. Asking a server whether there
//! is a newer release tells that server this copy was started, and that is a
//! thing to agree to rather than a thing to discover — so the question is put
//! plainly, in the reader's language, with what it costs written down and both
//! answers given the same weight. "No" is remembered for good.
//!
//! The second is the one this was asked for: a version to offer, what it
//! changes, and a download that ends in a checksum. It is drawn with more care
//! than the rest of the interface on purpose — a version banner, room for the
//! release notes, and a progress bar — because it is the one window a reader
//! meets without having gone looking for it, and the austere table that suits
//! a listing reads as an error box when it arrives unannounced.
//!
//! Nothing here installs anything. The archive lands in a folder and the
//! reader opens it when they choose to.

use desdec_core::update;
use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog, UpdateState},
    i18n::{Language, Text, text},
    preferences::accent,
    ui::{ERROR, MUTED, format_size},
};

/// How wide the windows open. Wide enough for a paragraph to be a paragraph
/// rather than a column of two-word lines.
const SIZE: egui::Vec2 = egui::vec2(560.0, 420.0);
/// The banner's height, and the room the notes get before they scroll.
const BANNER: f32 = 76.0;
const NOTES_HEIGHT: f32 = 180.0;

/// The question asked once: may Desdec look?
pub fn consent(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::UpdateConsent) {
        return;
    }
    let language = app.preferences.language;
    let tint = accent(app.preferences.theme);
    let mut answer: Option<bool> = None;
    let mut open = true;
    let id = egui::Id::new("desdec.update_consent");
    let mut window = egui::Window::new(text(language, Text::UpdateConsentTitle))
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(SIZE.x);
    window = crate::ui::centred(
        window,
        ctx,
        app.dialogs.opening_step(Dialog::UpdateConsent).is_some(),
    );
    window.show(ctx, |ui| {
        consent_banner(ui, language, tint);
        ui.add_space(10.0);
        ui.label(text(language, Text::UpdateConsentExplained));
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            // The two answers are the same size and sit side by side: a
            // question whose second answer is a small grey word below the
            // button is not a question. Neither of them is "never" — that
            // lives in the preferences, where a decision for good belongs.
            if accented_button(ui, text(language, Text::UpdateConsentYes), tint).clicked() {
                answer = Some(true);
            }
            if ui
                .add(egui::Button::new(text(language, Text::UpdateConsentNotNow)).min_size(BUTTON))
                .clicked()
            {
                answer = Some(false);
            }
        });
    });
    if !open {
        // Shutting the window is "not this time" by another route: it settles
        // nothing, and the question comes back next time the application does.
        app.postpone_update_consent();
    }
    match answer {
        Some(true) => app.allow_update_checks(ctx),
        Some(false) => app.postpone_update_consent(),
        None => {}
    }
}

/// The size every button in these windows is, so none of them is the small one.
const BUTTON: egui::Vec2 = egui::vec2(148.0, 30.0);

/// What was found, and what to do about it.
pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::Update) {
        return;
    }
    let language = app.preferences.language;
    let tint = accent(app.preferences.theme);
    let mut act = None;
    let mut open = true;
    let id = egui::Id::new("desdec.update");
    let mut window = egui::Window::new(text(language, Text::Updates))
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(SIZE.x);
    window = crate::ui::centred(
        window,
        ctx,
        app.dialogs.opening_step(Dialog::Update).is_some(),
    );
    window.show(ctx, |ui| {
        ui.set_min_width(SIZE.x);
        match &app.update {
            UpdateState::Idle if app.preferences.check_for_updates != Some(true) => {
                status_card(
                    ui,
                    text(language, Text::Updates),
                    text(language, Text::UpdateCheckingDisabled),
                    tint,
                );
            }
            UpdateState::Idle | UpdateState::Checking => {
                status_card(ui, text(language, Text::Updates), "", tint);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(text(language, Text::UpdateChecking));
                });
            }
            UpdateState::UpToDate => {
                status_card(
                    ui,
                    text(language, Text::UpdateUpToDate),
                    &format!(
                        "{} {}",
                        text(language, Text::UpdateCurrentVersion),
                        running_version()
                    ),
                    egui::Color32::from_rgb(120, 196, 132),
                );
            }
            UpdateState::Offered(release) => {
                offer(ui, release, language, tint, &mut act);
            }
            UpdateState::Downloading { release, received } => {
                banner(ui, release, language, tint);
                ui.add_space(8.0);
                let share = share_of(*received);
                ui.add(
                    egui::ProgressBar::new(share.clamp(0.0, 1.0))
                        .desired_width(SIZE.x - 32.0)
                        .fill(tint)
                        .text(format!(
                            "{} {} / {}",
                            text(language, Text::UpdateDownloading),
                            format_size(received.received),
                            format_size(received.total)
                        )),
                );
            }
            UpdateState::Downloaded { release, file } => {
                downloaded(ui, release, file, language, tint, &mut act);
            }
            UpdateState::Failed(error) => {
                status_card(
                    ui,
                    text(language, Text::Updates),
                    &explain(error, language),
                    ERROR,
                );
                ui.add_space(8.0);
                if ui.button(text(language, Text::UpdateOpenPage)).clicked() {
                    act = Some(Act::OpenPage);
                }
            }
        }
    });

    if !open {
        app.dialogs.close(Dialog::Update);
    }
    match act {
        Some(Act::Download) => app.start_update_download(ctx),
        Some(Act::Later) => app.dialogs.close(Dialog::Update),
        Some(Act::Skip) => app.skip_offered_update(),
        Some(Act::OpenPage) => open_page(app, ctx),
        Some(Act::ShowFile) => show_file(app, ctx),
        None => {}
    }
}

/// A small coloured card for the states that would otherwise be one line of
/// text adrift in a large window. It gives progress, success and errors the
/// same deliberate visual weight as an offered release.
fn status_card(ui: &mut egui::Ui, title: &str, detail: &str, colour: egui::Color32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 64.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 8.0, colour.gamma_multiply(0.13));
    let inner = rect.shrink2(egui::vec2(16.0, 12.0));
    ui.painter().circle_filled(
        inner.left_center() + egui::vec2(5.0, 0.0),
        5.0,
        colour.gamma_multiply(0.9),
    );
    ui.painter().text(
        inner.left_top() + egui::vec2(18.0, 0.0),
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::proportional(17.0),
        colour,
    );
    if !detail.is_empty() {
        ui.painter().text(
            inner.left_bottom() + egui::vec2(18.0, 0.0),
            egui::Align2::LEFT_BOTTOM,
            detail,
            egui::FontId::proportional(12.0),
            MUTED,
        );
    }
    ui.add_space(10.0);
}

/// The consent question gets a quiet header too: it is a choice about a
/// network request, not an error prompt.
fn consent_banner(ui: &mut egui::Ui, language: Language, tint: egui::Color32) {
    status_card(
        ui,
        text(language, Text::Updates),
        text(language, Text::UpdateConsentTitle),
        tint,
    );
}

/// What a press asked for, acted on once the window's borrow has ended.
enum Act {
    Download,
    Later,
    Skip,
    OpenPage,
    ShowFile,
}

/// How much of the archive has arrived, as a fraction a bar can be drawn from.
///
/// Worked out in whole thousandths and only then made a float: a byte count
/// does not fit exactly in one, a thousandth of a download does, and a bar is
/// drawn to a pixel rather than to a byte.
fn share_of(progress: update::Progress) -> f32 {
    if progress.total == 0 {
        return 0.0;
    }
    let thousandths = progress.received.saturating_mul(1000) / progress.total.max(1);
    f32::from(u16::try_from(thousandths).unwrap_or(1000).min(1000)) / 1000.0
}

/// The release on offer, and the three answers to it.
fn offer(
    ui: &mut egui::Ui,
    release: &update::Release,
    language: Language,
    tint: egui::Color32,
    act: &mut Option<Act>,
) {
    banner(ui, release, language, tint);
    notes(ui, release, language);
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if accented_button(ui, text(language, Text::UpdateDownload), tint).clicked() {
            *act = Some(Act::Download);
        }
        if ui
            .add(egui::Button::new(text(language, Text::UpdateLater)).min_size(BUTTON))
            .clicked()
        {
            *act = Some(Act::Later);
        }
        if ui.button(text(language, Text::UpdateSkipThis)).clicked() {
            *act = Some(Act::Skip);
        }
    });
    ui.add_space(4.0);
    if ui.link(text(language, Text::UpdateOpenPage)).clicked() {
        *act = Some(Act::OpenPage);
    }
}

/// What is shown once the archive is here and its hash has been checked.
///
/// Its own function because it is the longest of the states and the one that
/// has the most to say: where the file is, what the checksum does and does not
/// prove, and that installing it is the reader's own move.
fn downloaded(
    ui: &mut egui::Ui,
    release: &update::Release,
    file: &std::path::Path,
    language: Language,
    tint: egui::Color32,
    act: &mut Option<Act>,
) {
    banner(ui, release, language, tint);
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(text(language, Text::UpdateVerified))
            .color(egui::Color32::from_rgb(120, 196, 132)),
    );
    ui.add_space(4.0);
    ui.monospace(file.display().to_string());
    ui.add_space(8.0);
    ui.label(text(language, Text::UpdateInstallYourself));
    ui.add_space(4.0);
    ui.small(egui::RichText::new(text(language, Text::UpdateChecksumNote)).color(MUTED));
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if accented_button(ui, text(language, Text::UpdateShowFile), tint).clicked() {
            *act = Some(Act::ShowFile);
        }
        if ui.button(text(language, Text::CopyPath)).clicked() {
            ui.ctx().copy_text(file.display().to_string());
        }
        if ui.button(text(language, Text::UpdateOpenPage)).clicked() {
            *act = Some(Act::OpenPage);
        }
    });
}

/// The head of the window: the version being offered, in the accent, over what
/// is running now.
///
/// The one piece of the interface drawn for effect rather than for density,
/// and deliberately: this window arrives unasked, and a reader should be able
/// to tell what it is about without reading a word of it.
fn banner(ui: &mut egui::Ui, release: &update::Release, language: Language, tint: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), BANNER),
        egui::Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, 8.0, tint.gamma_multiply(0.16));
    let inner = rect.shrink2(egui::vec2(16.0, 10.0));
    ui.painter().text(
        inner.left_top(),
        egui::Align2::LEFT_TOP,
        text(language, Text::UpdateAvailable),
        egui::FontId::proportional(13.0),
        MUTED,
    );
    ui.painter().text(
        inner.left_top() + egui::vec2(0.0, 22.0),
        egui::Align2::LEFT_TOP,
        format!(
            "{} {}",
            text(language, Text::UpdateNewVersion),
            release.version
        ),
        egui::FontId::proportional(22.0),
        tint,
    );
    ui.painter().text(
        inner.right_bottom(),
        egui::Align2::RIGHT_BOTTOM,
        format!(
            "{} {} · {}",
            text(language, Text::UpdateCurrentVersion),
            running_version(),
            format_size(release.archive.size)
        ),
        egui::FontId::proportional(12.0),
        MUTED,
    );
    ui.add_space(10.0);
}

/// The release notes, given room and then a scrollbar.
fn notes(ui: &mut egui::Ui, release: &update::Release, language: Language) {
    if release.notes.trim().is_empty() {
        return;
    }
    ui.label(egui::RichText::new(text(language, Text::UpdateWhatChanged)).strong());
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .max_height(NOTES_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.label(readable(&release.notes));
        });
}

/// The release notes, made readable without being rendered.
///
/// GitHub holds them as markdown. Rendering it properly is a browser's job and
/// this window is not one; showing it raw puts a `## What changed` directly
/// under the heading that already says so, and leaves every list item starting
/// with an asterisk. So: the heading marks come off, and a bullet is drawn as
/// a bullet. Nothing else is touched — a line this does not recognise is shown
/// exactly as it was written.
fn readable(notes: &str) -> String {
    notes
        .trim()
        .lines()
        .map(|line| {
            let trimmed = line.trim_end();
            let bare = trimmed.trim_start();
            if let Some(heading) = bare.strip_prefix("### ") {
                return heading.to_owned();
            }
            if let Some(heading) = bare.strip_prefix("## ") {
                return heading.to_owned();
            }
            if let Some(heading) = bare.strip_prefix("# ") {
                return heading.to_owned();
            }
            for mark in ["* ", "- "] {
                if let Some(item) = bare.strip_prefix(mark) {
                    return format!("• {item}");
                }
            }
            trimmed.to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A button in the accent colour, for the one action a window is about.
fn accented_button(ui: &mut egui::Ui, label: &str, tint: egui::Color32) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).strong())
            .min_size(BUTTON)
            .fill(tint.gamma_multiply(0.85)),
    )
}

/// The version this build is, as the about window words it.
fn running_version() -> String {
    update::Version::running().map_or_else(|| String::from("—"), |version| version.to_string())
}

/// What went wrong, in the reader's language, with the detail after it.
///
/// The session's account uses this too: a line reading `Updates :
/// x86_64-unknown-linux-gnu` — which is all the bare `Display` of a
/// [`update::Error::NoArchiveForThisPlatform`] amounts to — told the reader
/// nothing, in a language that was not theirs.
pub fn explain(error: &update::Error, language: Language) -> String {
    let say = |item: Text| text(language, item).to_owned();
    match error {
        update::Error::Unreachable(why) | update::Error::Unreadable(why) => {
            format!("{} ({why})", say(Text::UpdateUnreachable))
        }
        update::Error::NoArchiveForThisPlatform { platform } => {
            format!("{} {platform}", say(Text::UpdateNoArchiveHere))
        }
        update::Error::ChecksumMismatch { expected, found } => {
            format!("{}\n\n{expected}\n{found}", say(Text::UpdateRefused))
        }
        update::Error::NoChecksum => say(Text::UpdateNoChecksum),
        update::Error::Storage(why) => format!("{} ({why})", say(Text::UpdateUnreachable)),
        update::Error::TooLarge { size } => {
            format!("{} ({})", say(Text::UpdateUnreachable), format_size(*size))
        }
    }
}

/// Sends the reader to the release's own page.
///
/// Through egui's own url opener, which is what every other link in the
/// application uses: Desdec does not start processes of its own, and a browser
/// launched from here would be one.
fn open_page(app: &DesdecApp, ctx: &egui::Context) {
    if let Some(release) = app.update.release() {
        ctx.open_url(egui::OpenUrl::new_tab(&release.page));
    }
}

/// Opens the folder the archive was written to, the same way.
fn show_file(app: &DesdecApp, ctx: &egui::Context) {
    let UpdateState::Downloaded { file, .. } = &app.update else {
        return;
    };
    let Some(directory) = file.parent() else {
        return;
    };
    // A `file:` url, which is what the platform's own handler expects; the
    // path is one this application wrote, so it is a path and not input.
    ctx.open_url(egui::OpenUrl::new_tab(format!(
        "file://{}",
        directory.display()
    )));
}

#[cfg(test)]
mod tests {
    use desdec_core::update;
    use eframe::egui;

    use crate::{
        app::{DesdecApp, Dialog, UpdateState, WorkspaceView},
        commands::Command,
        i18n::{Language, Text, text},
        testing::{drawn_text, window_input},
    };

    /// An application that has not been asked about updates yet.
    fn never_asked() -> DesdecApp {
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.preferences.check_for_updates = None;
        app.preferences.language = Language::English;
        app
    }

    /// Draws one frame of the whole interface and returns what it said.
    fn frame(app: &mut DesdecApp) -> String {
        let ctx = egui::Context::default();
        let _ = ctx.run(window_input(), |ctx| app.run_frame(ctx));
        let output = ctx.run(window_input(), |ctx| app.run_frame(ctx));
        drawn_text(&output.shapes)
    }

    /// The notes are made readable, not rendered, and not left raw.
    #[test]
    fn release_notes_lose_their_marks_without_being_rewritten() {
        let notes = "## What changed\n\n* One thing\n- Another\n\nA plain line, `code` and all.";
        let shown = super::readable(notes);
        assert!(shown.starts_with("What changed"), "the hashes come off");
        assert!(shown.contains("• One thing"), "a bullet is a bullet");
        assert!(shown.contains("• Another"), "whichever mark was used");
        assert!(
            shown.contains("A plain line, `code` and all."),
            "and a line this does not recognise is left exactly as written"
        );
    }

    #[test]
    fn the_question_is_asked_before_anything_is_ever_sent() {
        let mut app = never_asked();
        let said = frame(&mut app);
        assert!(
            app.dialogs.is_open(Dialog::UpdateConsent),
            "the question is put before the first check"
        );
        assert!(
            said.contains(text(Language::English, Text::UpdateConsentNotNow)),
            "and putting it off is offered as plainly as agreeing: {said}"
        );
    }

    /// Putting the question off settles nothing and asks nothing further.
    ///
    /// The preference is left unanswered on purpose: the reader said "not
    /// now", not "no", and writing down a refusal they did not give would be
    /// deciding for them.
    #[test]
    fn putting_the_question_off_settles_nothing_and_does_not_ask_again_today() {
        let mut app = never_asked();
        app.postpone_update_consent();
        assert_eq!(
            app.preferences.check_for_updates, None,
            "nothing was decided"
        );
        assert!(!app.dialogs.is_open(Dialog::UpdateConsent));

        let _ = frame(&mut app);
        assert!(
            !app.dialogs.is_open(Dialog::UpdateConsent),
            "and it is not asked twice in one sitting"
        );
    }

    /// Agreeing is what writes something down.
    #[test]
    fn agreeing_is_remembered_and_the_question_does_not_come_back() {
        let ctx = egui::Context::default();
        let mut app = never_asked();
        app.allow_update_checks(&ctx);
        assert_eq!(app.preferences.check_for_updates, Some(true));
        assert!(!app.dialogs.is_open(Dialog::UpdateConsent));

        let _ = frame(&mut app);
        assert!(!app.dialogs.is_open(Dialog::UpdateConsent));
    }

    /// Turning them off for good is the preferences' job, and it sticks.
    #[test]
    fn turning_the_checks_off_in_the_preferences_stops_the_question() {
        let mut app = never_asked();
        app.preferences.check_for_updates = Some(false);
        let _ = frame(&mut app);
        assert!(
            !app.dialogs.is_open(Dialog::UpdateConsent),
            "a decision taken there is not a question asked here"
        );
    }

    #[test]
    fn the_question_never_opens_over_another_window() {
        let mut app = never_asked();
        app.dialogs.open(Dialog::About);
        let _ = frame(&mut app);
        assert!(
            !app.dialogs.is_open(Dialog::UpdateConsent),
            "it waits until the reader is not in the middle of something"
        );
    }

    /// A release built as GitHub would describe it, with this machine's own
    /// archive so the window has something to offer whatever the runner is.
    fn offered(version: &str) -> update::Release {
        let archive = update::platform_archive().unwrap_or("desdec-archive");
        update::Release {
            version: update::Version::parse(version).expect("a version"),
            tag: format!("v{version}"),
            notes: String::from("Something changed."),
            page: String::from("https://example.invalid/releases"),
            published: String::from("2026-08-20T00:00:00Z"),
            archive: update::Asset {
                name: archive.to_owned(),
                url: String::from("https://example.invalid/archive"),
                size: 9_199_178,
            },
            checksum: Some(update::Asset {
                name: format!("{archive}.sha256"),
                url: String::from("https://example.invalid/archive.sha256"),
                size: 101,
            }),
        }
    }

    #[test]
    fn an_offered_release_says_what_it_is_and_offers_three_answers() {
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.preferences.language = Language::English;
        app.preferences.check_for_updates = Some(true);
        app.update = UpdateState::Offered(Box::new(offered("9.9.9")));
        app.dialogs.open(Dialog::Update);

        let said = frame(&mut app);
        for expected in [
            Text::UpdateDownload,
            Text::UpdateLater,
            Text::UpdateSkipThis,
            Text::UpdateWhatChanged,
        ] {
            let wanted = text(Language::English, expected);
            assert!(said.contains(wanted), "{wanted:?} is offered: {said}");
        }
        assert!(said.contains("9.9.9"), "the version being offered: {said}");
        assert!(said.contains("Something changed."), "the notes: {said}");
    }

    #[test]
    fn a_skipped_version_is_not_offered_again_until_something_newer_appears() {
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.preferences.check_for_updates = Some(true);
        app.update = UpdateState::Offered(Box::new(offered("9.9.9")));
        app.dialogs.open(Dialog::Update);
        app.skip_offered_update();

        assert_eq!(app.preferences.skipped_release.as_deref(), Some("9.9.9"));
        assert!(!app.dialogs.is_open(Dialog::Update));
        assert!(
            !app.would_offer(&offered("9.9.9")),
            "the one turned down stays turned down"
        );
        assert!(
            app.would_offer(&offered("9.9.10")),
            "and something newer than it is offered"
        );
    }

    #[test]
    fn a_failed_check_says_so_rather_than_saying_nothing() {
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.preferences.language = Language::English;
        app.preferences.check_for_updates = Some(true);
        app.update = UpdateState::Failed(update::Error::Unreachable(String::from("no route")));
        app.dialogs.open(Dialog::Update);

        let said = frame(&mut app);
        assert!(
            said.contains(text(Language::English, Text::UpdateUnreachable)),
            "an unreachable server is reported: {said}"
        );
    }

    #[test]
    fn a_mismatched_checksum_says_the_file_was_deleted() {
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.preferences.language = Language::English;
        app.preferences.check_for_updates = Some(true);
        app.update = UpdateState::Failed(update::Error::ChecksumMismatch {
            expected: "a".repeat(64),
            found: "b".repeat(64),
        });
        app.dialogs.open(Dialog::Update);

        let said = frame(&mut app);
        assert!(
            said.contains(text(Language::English, Text::UpdateRefused)),
            "the reader is told what happened to it: {said}"
        );
    }

    #[test]
    fn asking_deliberately_with_checking_off_says_it_is_off() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.preferences.language = Language::English;
        app.preferences.check_for_updates = Some(false);
        app.run_command(&ctx, Command::CheckForUpdates);

        let said = frame(&mut app);
        assert!(
            said.contains(text(Language::English, Text::UpdateCheckingDisabled)),
            "nothing is asked of GitHub, and the window says why: {said}"
        );
    }

    #[test]
    fn nothing_in_the_window_is_drawn_on_top_of_anything_else() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.preferences.check_for_updates = Some(true);
        app.update = UpdateState::Offered(Box::new(offered("9.9.9")));
        app.dialogs.open(Dialog::Update);

        let mut draw = |ctx: &egui::Context| super::show(&mut app, ctx);
        let _ = ctx.run(window_input(), &mut draw);
        let output = ctx.run(window_input(), &mut draw);

        let mut seen: Vec<egui::Pos2> = Vec::new();
        for (said, at) in crate::testing::drawn(&output.shapes) {
            assert!(
                !seen.contains(&at),
                "{said:?} is drawn on top of something else, at {at:?}"
            );
            seen.push(at);
        }
    }
}
