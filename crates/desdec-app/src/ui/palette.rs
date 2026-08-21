use std::path::PathBuf;

use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    commands::Command,
    i18n::Text,
    ui::MUTED,
};

/// Opening size, also the one assumed before egui has measured the window.
const SIZE: egui::Vec2 = egui::vec2(520.0, 420.0);

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::CommandPalette) {
        return;
    }

    // A stable id keeps the window in place when the title is translated.
    let id = egui::Id::new("desdec.command_palette");
    let mut open = true;
    let mut chosen = None;
    let mut window = egui::Window::new(app.t(Text::PaletteTitle))
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(SIZE)
        .min_width(360.0)
        .min_height(260.0);
    if let Some(step) = app.dialogs.opening_step(Dialog::CommandPalette) {
        window = window.current_pos(crate::ui::opening_position(ctx, id, step, SIZE));
    }
    window.show(ctx, |ui| {
        chosen = contents(app, ui);
    });

    if let Some(chosen) = chosen {
        match chosen {
            PaletteChoice::Command(command) => app.run_command(ctx, command),
            PaletteChoice::Recent(path) => app.open_recent_binary(ctx, path),
            PaletteChoice::ClearHistory => app.clear_recent_binaries(),
        }
        open = false;
    }
    app.dialogs.set(Dialog::CommandPalette, open);
}

enum PaletteChoice {
    Command(Command),
    Recent(PathBuf),
    ClearHistory,
}

fn contents(app: &mut DesdecApp, ui: &mut egui::Ui) -> Option<PaletteChoice> {
    ui.label(app.t(Text::SearchAction));
    // The keys that drive the list are claimed before the search field is
    // drawn, so they only ever do one thing: the focused `TextEdit` would
    // otherwise also act on them — the arrows moving the text cursor, Enter
    // dropping the focus — while the list moved underneath.
    let keys = NavigationKeys::take(ui);
    let search_hint = app.t(Text::SearchHint);
    let response =
        ui.add(egui::TextEdit::singleline(&mut app.palette.query).hint_text(search_hint));
    response.request_focus();
    if response.changed() {
        app.palette.selected = 0;
    }
    ui.add_space(8.0);

    let matches = matching_commands(app);
    let recent = matching_recent(app);
    let mut chosen = keyboard_selection(app, &matches, keys).map(PaletteChoice::Command);
    egui::ScrollArea::vertical()
        .max_height(ui.available_height().max(160.0))
        .show(ui, |ui| {
            if !recent.is_empty() {
                ui.strong(app.t(Text::RecentBinaries));
                for path in recent {
                    let label = path.display().to_string();
                    if ui.button(label).clicked() {
                        chosen = Some(PaletteChoice::Recent(path));
                    }
                }
                if ui.button(app.t(Text::ClearRecentBinaries)).clicked() {
                    chosen = Some(PaletteChoice::ClearHistory);
                }
                ui.separator();
            }
            for (index, command) in matches.iter().copied().enumerate() {
                ui.horizontal(|ui| {
                    let selected = index == app.palette.selected;
                    let label = command.label(app.preferences.language);
                    let entry = ui.add_enabled(
                        app.can_run(command),
                        egui::SelectableLabel::new(selected, label),
                    );
                    if entry.clicked() {
                        chosen = Some(PaletteChoice::Command(command));
                        app.palette.selected = index;
                    }
                    // Keep the highlight visible when the arrows walk past the
                    // bottom of the list.
                    if selected && keys.moved() {
                        entry.scroll_to_me(Some(egui::Align::Center));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(app.shortcut_label(command)).color(MUTED));
                    });
                });
            }
        });
    chosen
}

/// The keys the palette handles itself, taken out of the frame's events.
#[derive(Clone, Copy, Default)]
struct NavigationKeys {
    up: bool,
    down: bool,
    run: bool,
}

impl NavigationKeys {
    fn take(ui: &egui::Ui) -> Self {
        ui.input_mut(|input| Self {
            up: input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            down: input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            run: input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
        })
    }

    const fn moved(self) -> bool {
        self.up || self.down
    }
}

fn matching_recent(app: &DesdecApp) -> Vec<PathBuf> {
    let query = app.palette.query.to_lowercase();
    app.recent_binaries()
        .iter()
        .filter(|path| path.display().to_string().to_lowercase().contains(&query))
        .cloned()
        .collect()
}

fn matching_commands(app: &DesdecApp) -> Vec<Command> {
    let query = app.palette.query.to_lowercase();
    Command::ALL
        .iter()
        .copied()
        .filter(|command| {
            command
                .label(app.preferences.language)
                .to_lowercase()
                .contains(&query)
        })
        .collect()
}

/// Moves the highlight with the arrow keys and runs it with `Enter`.
///
/// The highlight only ever lands on a command that can actually run, so
/// `Enter` always answers something.
fn keyboard_selection(
    app: &mut DesdecApp,
    matches: &[Command],
    keys: NavigationKeys,
) -> Option<Command> {
    let runnable: Vec<usize> = matches
        .iter()
        .enumerate()
        .filter(|(_, command)| app.can_run(**command))
        .map(|(index, _)| index)
        .collect();
    let Some(&first) = runnable.first() else {
        app.palette.selected = 0;
        return None;
    };

    let position = runnable
        .iter()
        .position(|index| *index == app.palette.selected)
        .unwrap_or_else(|| {
            app.palette.selected = first;
            0
        });
    let last = runnable.len() - 1;
    if keys.down {
        app.palette.selected = runnable[if position == last { 0 } else { position + 1 }];
    } else if keys.up {
        app.palette.selected = runnable[position.checked_sub(1).unwrap_or(last)];
    }
    keys.run.then(|| matches[app.palette.selected])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::WorkspaceView, i18n::Language};

    /// One frame carrying a single key press.
    fn press(key: egui::Key) -> egui::RawInput {
        egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        }
    }

    /// One frame carrying a key held with modifiers.
    fn shortcut(key: egui::Key, modifiers: egui::Modifiers) -> egui::RawInput {
        egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            modifiers,
            ..Default::default()
        }
    }

    /// One frame carrying typed text, as the search field receives it.
    fn typed(text: &str) -> egui::RawInput {
        egui::RawInput {
            events: vec![egui::Event::Text(text.to_owned())],
            ..Default::default()
        }
    }

    /// Runs one whole application frame, not just this window: the palette
    /// shares its keyboard with the shortcut handling and every other panel,
    /// and a key one of them claims first never reaches the list.
    fn frame(ctx: &egui::Context, app: &mut DesdecApp, input: egui::RawInput) {
        let _ = ctx.run(input, |ctx| app.run_frame(ctx));
    }

    /// An open palette whose query was typed in, with the search field focused
    /// as it is in front of a user.
    fn searching(ctx: &egui::Context, query: &str) -> DesdecApp {
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.preferences.language = Language::English;
        app.dialogs.open(Dialog::CommandPalette);
        frame(ctx, &mut app, egui::RawInput::default());
        for character in query.chars() {
            frame(ctx, &mut app, typed(&character.to_string()));
        }
        assert_eq!(app.palette.query, query, "the query should have been typed");
        app
    }

    /// Enter must reach the list even though the search field holds the
    /// keyboard focus.
    #[test]
    fn enter_runs_the_highlighted_command() {
        let ctx = egui::Context::default();
        let mut app = searching(&ctx, "disassembly");

        frame(&ctx, &mut app, press(egui::Key::Enter));

        assert_eq!(app.active_view, WorkspaceView::Disassembly);
        assert!(
            !app.dialogs.is_open(Dialog::CommandPalette),
            "running a command closes it"
        );
    }

    #[test]
    fn the_arrows_walk_the_list_before_enter_runs_it() {
        let ctx = egui::Context::default();
        let mut app = searching(&ctx, "theme");

        frame(&ctx, &mut app, press(egui::Key::ArrowDown));
        assert_eq!(app.palette.selected, 1);

        frame(&ctx, &mut app, press(egui::Key::ArrowUp));
        assert_eq!(app.palette.selected, 0);
    }

    #[test]
    fn the_highlight_wraps_around_the_ends_of_the_list() {
        let ctx = egui::Context::default();
        let mut app = searching(&ctx, "e");
        let count = matching_commands(&app).len();

        frame(&ctx, &mut app, press(egui::Key::ArrowUp));
        assert_eq!(app.palette.selected, count - 1);

        frame(&ctx, &mut app, press(egui::Key::ArrowDown));
        assert_eq!(app.palette.selected, 0);
    }

    /// A command that cannot run right now must never be what `Enter` lands
    /// on: a highlight that does nothing reads as a broken palette.
    #[test]
    fn the_highlight_skips_commands_that_cannot_run() {
        let ctx = egui::Context::default();
        // Scanning needs a binary, and this application has none open.
        // Abandoning an opening is only possible while one is under way, and
        // this application is idle.
        let mut app = searching(&ctx, "give up the opening");
        let matches = matching_commands(&app);
        assert_eq!(matches, vec![Command::CancelAnalysis]);
        assert!(!app.can_run(Command::CancelAnalysis));

        frame(&ctx, &mut app, press(egui::Key::Enter));

        assert!(
            app.dialogs.is_open(Dialog::CommandPalette),
            "an unavailable entry leaves the palette open instead of pretending to act"
        );
    }

    /// The path a user actually takes: open the palette with its shortcut,
    /// type, then press Enter — all of it through the whole frame.
    #[test]
    fn the_shortcut_opens_a_palette_that_answers_the_keyboard() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.preferences.language = Language::English;

        frame(&ctx, &mut app, egui::RawInput::default());
        frame(
            &ctx,
            &mut app,
            shortcut(egui::Key::P, egui::Modifiers::CTRL | egui::Modifiers::SHIFT),
        );
        assert!(
            app.dialogs.is_open(Dialog::CommandPalette),
            "the shortcut opens the palette"
        );

        for character in "strings".chars() {
            frame(&ctx, &mut app, typed(&character.to_string()));
        }
        frame(&ctx, &mut app, press(egui::Key::Enter));

        assert_eq!(app.active_view, WorkspaceView::Strings);
        assert!(!app.dialogs.is_open(Dialog::CommandPalette));
    }

    /// A shortcut must keep working while the search field holds the focus:
    /// its own combination is how the palette is closed again.
    #[test]
    fn the_shortcut_still_closes_the_palette_it_opened() {
        let ctx = egui::Context::default();
        let mut app = searching(&ctx, "theme");
        let combination = shortcut(egui::Key::P, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);

        frame(&ctx, &mut app, combination);

        assert!(!app.dialogs.is_open(Dialog::CommandPalette));
    }

    /// The palette is the one place the whole application is visible, so an
    /// empty query must list every command there is — no hidden entries.
    #[test]
    fn recent_binaries_are_searchable_from_the_palette() {
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.preferences.recent_binaries = vec![
            std::path::PathBuf::from("/tmp/example.elf"),
            std::path::PathBuf::from("/tmp/tool.exe"),
        ];
        app.palette.query = "elf".to_owned();

        assert_eq!(
            matching_recent(&app),
            vec![std::path::PathBuf::from("/tmp/example.elf")]
        );
    }

    #[test]
    fn an_empty_query_lists_every_command() {
        let ctx = egui::Context::default();
        let app = searching(&ctx, "");

        assert_eq!(matching_commands(&app).len(), Command::ALL.len());
    }

    /// Every view reachable from the menu must also be reachable from the
    /// palette. `Segments` was missing, and so could only be opened one way.
    ///
    /// The mapping is read rather than exercised: running every command to see
    /// where each one led also ran the ones that open a file dialog, and put
    /// seven of them on the screen.
    #[test]
    fn every_view_has_a_command() {
        for view in WorkspaceView::ALL {
            assert!(
                Command::ALL
                    .iter()
                    .any(|command| command.opens_view() == Some(*view)),
                "{view:?} cannot be reached from the palette"
            );
        }
    }

    /// A command that declares a view must actually open it, so the
    /// declaration cannot drift from what running it does.
    #[test]
    fn a_command_opens_the_view_it_declares() {
        let ctx = egui::Context::default();
        for command in Command::ALL.iter().copied() {
            let Some(expected) = command.opens_view() else {
                continue;
            };
            // Only view-switching commands are run here; none of them touches
            // the file system or opens a dialog.
            let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
            app.run_command(&ctx, command);
            assert_eq!(app.active_view, expected, "{command:?}");
            assert!(
                !app.showing_a_native_dialog(),
                "{command:?} opened a dialog on the user's screen"
            );
        }
    }

    /// Actions offered elsewhere in the interface must be offered here too.
    #[test]
    fn the_actions_of_the_interface_are_all_in_the_palette() {
        let ctx = egui::Context::default();
        let app = searching(&ctx, "");
        let labels: Vec<String> = matching_commands(&app)
            .iter()
            .map(|command| command.label(Language::English).to_lowercase())
            .collect();

        for expected in [
            "open binary",
            "close binary",
            "segments",
            "export patched binary",
            "discard all",
            "keep decompiled functions on disk",
            "clear the cache",
            "rizin + rz-ghidra",
            "retdec",
            "run the selected decompiler again",
            "corresponding disassembly",
            "copy displayed pseudocode",
            "preferences",
        ] {
            assert!(
                labels.iter().any(|label| label.contains(expected)),
                "the palette offers no way to {expected}"
            );
        }
    }

    /// An action that cannot act right now is listed but not chosen: an export
    /// with nothing to export would swallow the keystroke.
    #[test]
    fn an_action_with_nothing_to_act_on_cannot_be_chosen() {
        let app = DesdecApp::for_test(None, WorkspaceView::Overview);

        assert!(!app.can_run(Command::ExportPatched), "no binary, no export");
        assert!(!app.can_run(Command::CloseBinary), "nothing to close");
        assert!(
            app.can_run(Command::Disassembly),
            "switching view says to open a binary, which is an answer"
        );
        assert!(app.can_run(Command::Preferences));
    }

    /// Every other command in the list is reachable and does something.
    #[test]
    fn walking_the_whole_list_never_stops_on_an_unavailable_command() {
        let ctx = egui::Context::default();
        let mut app = searching(&ctx, "");
        let matches = matching_commands(&app);

        for _ in 0..=matches.len() {
            assert!(
                app.can_run(matches[app.palette.selected]),
                "the highlight stopped on an unavailable command"
            );
            frame(&ctx, &mut app, press(egui::Key::ArrowDown));
        }
    }
}
