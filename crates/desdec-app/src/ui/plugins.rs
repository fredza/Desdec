//! What is installed, what each one asks for, and what it was given.
//!
//! A plugin is code from somewhere else, so this window is written to be read
//! before anything is enabled rather than after something has happened: the
//! permissions are listed next to the switch that grants them, in the reader's
//! own language, and a plugin that has not been enabled has never run.
//!
//! Enabling one grants exactly what its manifest asks for at that moment. A
//! plugin whose manifest later asks for more does not quietly get it — it
//! stops, and says the list has changed, until the reader has looked at the
//! new one.

use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::{Language, Text, text},
    plugins::{Broken, Consent, Hook, Plugin},
    script::Context,
    ui::{ERROR, MUTED, card},
};

const ASSUMED_SIZE: egui::Vec2 = egui::vec2(600.0, 480.0);

/// What the reader pressed, carried out once the window is done drawing.
enum Action {
    Enable(String),
    Disable(String),
    Run(String),
    Reload,
    CopyDirectory(String),
}

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::Plugins) {
        return;
    }
    let id = egui::Id::new("desdec.plugins");
    let mut open = true;
    let mut window = egui::Window::new(app.t(Text::PluginsTitle))
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(ASSUMED_SIZE)
        .min_width(420.0);
    if let Some(step) = app.dialogs.opening_step(Dialog::Plugins) {
        window = window.current_pos(crate::ui::opening_position(ctx, id, step, ASSUMED_SIZE));
    }
    let mut action = None;
    window.show(ctx, |ui| {
        action = contents(app, ui);
    });
    app.dialogs.set(Dialog::Plugins, open);
    if let Some(action) = action {
        carry_out(app, ctx, action);
    }
}

fn carry_out(app: &mut DesdecApp, ctx: &egui::Context, action: Action) {
    match action {
        Action::Reload => app.reload_plugins(),
        Action::CopyDirectory(path) => app.copy_to_clipboard(ctx, &path, Text::PathCopied),
        Action::Enable(id) => {
            // Granted from the manifest as it reads now, which is the list the
            // reader had in front of them when they pressed the switch.
            let wanted = app.plugins.get(&id).map(Plugin::wanted).unwrap_or_default();
            app.preferences.plugins.insert(
                id,
                Consent {
                    enabled: true,
                    granted: wanted,
                },
            );
        }
        Action::Disable(id) => {
            if let Some(consent) = app.preferences.plugins.get_mut(&id) {
                consent.enabled = false;
            }
        }
        Action::Run(id) => run_one(app, ctx, &id),
    }
}

/// Runs one plugin now, on the reader's say-so.
fn run_one(app: &mut DesdecApp, ctx: &egui::Context, id: &str) {
    let Some(plugin) = app.plugins.get(id) else {
        return;
    };
    let title = plugin.title().to_owned();
    let source = plugin.source.clone();
    let wanted = plugin.wanted();
    let granted = app
        .preferences
        .plugins
        .get(id)
        .filter(|consent| consent.enabled && consent.covers(&wanted))
        .map(|consent| consent.granted.clone())
        .unwrap_or_default();
    let context = Context {
        granted,
        limits: crate::script::Limits::default(),
        language: app.preferences.language,
    };
    let outcome = app.run_script(ctx, &source, &context);
    app.record_script_run(&title, &outcome);
    // The console is where a script's output is read, whichever script it
    // came from: a plugin that printed twelve lines has nowhere else to put
    // them, and a reader who ran it deliberately is owed the answer.
    app.script.took(Some(title), outcome);
    app.dialogs.open(Dialog::Console);
}

fn contents(app: &mut DesdecApp, ui: &mut egui::Ui) -> Option<Action> {
    let language = app.preferences.language;
    let mut action = None;

    match crate::plugins::directory() {
        Some(directory) => {
            let shown = directory.display().to_string();
            ui.horizontal_wrapped(|ui| {
                ui.small(egui::RichText::new(&shown).monospace().color(MUTED));
                if ui.small_button(text(language, Text::CopyPath)).clicked() {
                    action = Some(Action::CopyDirectory(shown.clone()));
                }
                if ui
                    .small_button(text(language, Text::ReloadPlugins))
                    .clicked()
                {
                    action = Some(Action::Reload);
                }
            });
        }
        None => {
            ui.colored_label(ERROR, text(language, Text::PluginDirectoryUnknown));
        }
    }
    ui.add_space(6.0);

    if app.plugins.is_empty() {
        ui.label(text(language, Text::NoPlugins));
        ui.add_space(2.0);
        ui.small(egui::RichText::new(text(language, Text::PluginsHint)).color(MUTED));
        return action;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for plugin in &app.plugins.plugins {
                if let Some(pressed) = one(
                    ui,
                    plugin,
                    app.preferences.plugins.get(&plugin.id),
                    language,
                ) {
                    action = Some(pressed);
                }
                ui.add_space(6.0);
            }
            for broken in &app.plugins.broken {
                unreadable(ui, broken, language);
                ui.add_space(6.0);
            }
        });
    action
}

/// One plugin: what it says it is, what it wants, and the switch.
fn one(
    ui: &mut egui::Ui,
    plugin: &Plugin,
    consent: Option<&Consent>,
    language: Language,
) -> Option<Action> {
    let mut action = None;
    let wanted = plugin.wanted();
    let enabled = consent.is_some_and(|consent| consent.enabled);
    let covered = consent.is_some_and(|consent| consent.covers(&wanted));

    card(ui, plugin.title(), |ui| {
        // One line rather than two labels side by side: consecutive small
        // labels in a wrapped row land on top of each other, and a version
        // printed over an author's name is worse than either alone.
        let said = provenance(plugin, language);
        if !said.is_empty() {
            ui.small(egui::RichText::new(said).color(MUTED));
        }

        let description = plugin.manifest.description.trim();
        if !description.is_empty() {
            ui.add_space(2.0);
            ui.label(description);
        }

        ui.add_space(4.0);
        ui.small(
            egui::RichText::new(format!(
                "{} : {}",
                text(language, Text::PluginRuns),
                hooks(plugin, language)
            ))
            .color(MUTED),
        );

        ui.add_space(2.0);
        if wanted.is_empty() {
            ui.small(egui::RichText::new(text(language, Text::PluginAsksNothing)).color(MUTED));
        } else {
            ui.small(
                egui::RichText::new(format!("{} :", text(language, Text::PluginWants)))
                    .color(MUTED),
            );
            for permission in &wanted {
                ui.small(
                    egui::RichText::new(format!("· {}", text(language, permission.label())))
                        .color(MUTED),
                );
            }
        }

        if enabled && !covered {
            ui.add_space(4.0);
            ui.colored_label(ERROR, text(language, Text::PluginAsksForMore));
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let switch = if enabled {
                text(language, Text::DisablePlugin)
            } else {
                text(language, Text::EnablePlugin)
            };
            if ui.button(switch).clicked() {
                action = Some(if enabled {
                    Action::Disable(plugin.id.clone())
                } else {
                    Action::Enable(plugin.id.clone())
                });
            }
            let can_run = enabled && covered;
            if ui
                .add_enabled(can_run, egui::Button::new(text(language, Text::RunPlugin)))
                .clicked()
            {
                action = Some(Action::Run(plugin.id.clone()));
            }
        });
    });
    action
}

/// A directory that could not be read as a plugin, and why.
fn unreadable(ui: &mut egui::Ui, broken: &Broken, language: Language) {
    card(ui, &broken.id, |ui| {
        ui.colored_label(ERROR, text(language, Text::PluginUnreadable));
        ui.small(egui::RichText::new(&broken.reason).monospace().color(MUTED));
    });
}

/// Where a plugin says it came from: its version, its author, or both.
fn provenance(plugin: &Plugin, language: Language) -> String {
    let mut said = Vec::new();
    let version = plugin.manifest.version.trim();
    if !version.is_empty() {
        said.push(version.to_owned());
    }
    let author = plugin.manifest.author.trim();
    if !author.is_empty() {
        said.push(format!("{} {author}", text(language, Text::PluginBy)));
    }
    said.join(" · ")
}

/// When a plugin runs, in words rather than as a list of variants.
fn hooks(plugin: &Plugin, language: Language) -> String {
    let said: Vec<&str> = Hook::ALL
        .iter()
        .filter(|hook| plugin.runs_on(**hook))
        .map(|hook| text(language, hook.label()))
        .collect();
    if said.is_empty() {
        return text(language, Text::HookOnDemand).to_owned();
    }
    said.join(", ")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use eframe::egui;

    use crate::{
        app::{DesdecApp, WorkspaceView},
        i18n::Text,
        plugins::{self, Consent},
        script::Permission,
        testing::{opened_app, window_input},
    };

    /// A plugin directory of this test's own, with one plugin in it.
    ///
    /// The application is never pointed at the real plugin directory here: a
    /// test must not depend on what the machine running it happens to have
    /// installed, and must never write into it.
    fn installed(name: &str, manifest: &str, script: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("desdec-ui-plugins-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        let directory = root.join(name);
        std::fs::create_dir_all(&directory).expect("the test directory can be created");
        std::fs::write(directory.join(plugins::MANIFEST), manifest).expect("manifest written");
        std::fs::write(directory.join("plugin.rhai"), script).expect("script written");
        root
    }

    /// An application with the reference binary open and one plugin installed.
    fn app_with(root: &Path) -> DesdecApp {
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.plugins = plugins::read(root);
        app
    }

    fn first_address(app: &DesdecApp) -> u64 {
        app.analysis
            .as_ref()
            .and_then(|analysis| analysis.instructions.first())
            .map(|instruction| instruction.address)
            .expect("the reference binary decodes to something")
    }

    fn naming_plugin(name: &str, hook: &str, address: u64) -> PathBuf {
        installed(
            name,
            &format!(
                r#"(name: "Namer", version: "1.0", script: "plugin.rhai", hooks: [{hook}], permissions: [WriteNotes])"#
            ),
            &format!(r#"label({address}, "named_by_a_plugin");"#),
        )
    }

    #[test]
    fn an_enabled_plugin_names_what_it_came_for_when_a_binary_opens() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);
        let address = first_address(&app);
        let root = naming_plugin("on-open", "OnOpen", address);
        app.plugins = plugins::read(&root);
        app.preferences.plugins.insert(
            "on-open".to_owned(),
            Consent {
                enabled: true,
                granted: vec![Permission::WriteNotes],
            },
        );

        app.run_plugins_on_open(&ctx);

        assert_eq!(app.annotations.label(address), Some("named_by_a_plugin"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_plugin_nobody_enabled_never_runs() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);
        let address = first_address(&app);
        let root = naming_plugin("never", "OnOpen", address);
        app.plugins = plugins::read(&root);

        app.run_plugins_on_open(&ctx);

        assert_eq!(
            app.annotations.label(address),
            None,
            "installing is not enabling"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The case a manifest could otherwise use to widen itself quietly: a
    /// plugin enabled last month, edited since to ask for more.
    #[test]
    fn a_plugin_asking_for_more_than_was_granted_is_stopped_until_it_is_seen() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);
        let address = first_address(&app);
        let root = naming_plugin("widened", "OnOpen", address);
        app.plugins = plugins::read(&root);
        app.preferences.plugins.insert(
            "widened".to_owned(),
            Consent {
                enabled: true,
                // What the reader agreed to before the manifest changed.
                granted: vec![Permission::Navigate],
            },
        );

        app.run_plugins_on_open(&ctx);

        assert_eq!(
            app.annotations.label(address),
            None,
            "it does not run at all, rather than running with less"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_plugin_that_asked_to_run_on_request_does_not_run_on_opening() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);
        let address = first_address(&app);
        let root = naming_plugin("on-demand", "OnDemand", address);
        app.plugins = plugins::read(&root);
        app.preferences.plugins.insert(
            "on-demand".to_owned(),
            Consent {
                enabled: true,
                granted: vec![Permission::WriteNotes],
            },
        );

        app.run_plugins_on_open(&ctx);
        assert_eq!(app.annotations.label(address), None);

        super::run_one(&mut app, &ctx, "on-demand");
        assert_eq!(
            app.annotations.label(address),
            Some("named_by_a_plugin"),
            "asked for, it runs"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn enabling_grants_exactly_what_the_manifest_asks_for() {
        let ctx = egui::Context::default();
        let root = naming_plugin("granting", "OnOpen", 0x0040_1000);
        let mut app = app_with(&root);

        super::carry_out(&mut app, &ctx, super::Action::Enable("granting".to_owned()));

        let consent = app
            .preferences
            .plugins
            .get("granting")
            .expect("the decision is remembered");
        assert!(consent.enabled);
        assert_eq!(consent.granted, vec![Permission::WriteNotes]);

        super::carry_out(
            &mut app,
            &ctx,
            super::Action::Disable("granting".to_owned()),
        );
        assert!(
            !app.preferences.plugins["granting"].enabled,
            "and it can be taken back"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_plugin_run_is_written_into_the_session_s_account() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);
        let address = first_address(&app);
        let root = naming_plugin("recorded", "OnOpen", address);
        app.plugins = plugins::read(&root);
        app.preferences.plugins.insert(
            "recorded".to_owned(),
            Consent {
                enabled: true,
                granted: vec![Permission::WriteNotes],
            },
        );
        let before = app.journal.entries().len();

        app.run_plugins_on_open(&ctx);

        assert!(
            app.journal.entries().len() > before,
            "a plugin that changed something says so where it can be read back"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_window_lists_what_a_plugin_asks_for_before_it_is_enabled() {
        let ctx = egui::Context::default();
        let root = naming_plugin("listed", "OnOpen", 0x0040_1000);
        let mut app = app_with(&root);
        app.dialogs.open(crate::app::Dialog::Plugins);

        let _ = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));
        let output = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));
        let drawn = crate::testing::drawn_text(&output.shapes);

        assert!(drawn.contains("Namer"), "the plugin is listed: {drawn}");
        assert!(
            drawn.contains(crate::i18n::text(
                app.preferences.language,
                Permission::WriteNotes.label()
            )),
            "what it asks for is on screen before the switch is pressed"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_directory_that_is_not_a_plugin_is_shown_rather_than_hidden() {
        let ctx = egui::Context::default();
        let root = installed("unreadable", "(name: \"Half\"", "");
        let mut app = app_with(&root);
        app.dialogs.open(crate::app::Dialog::Plugins);

        let _ = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));
        let output = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));
        let drawn = crate::testing::drawn_text(&output.shapes);

        assert!(
            drawn.contains(crate::i18n::text(
                app.preferences.language,
                Text::PluginUnreadable
            )),
            "a plugin that could not be read is said so: {drawn}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The plugins shipped in the repository must work.
    ///
    /// An example that does not run is worse than no example: it is the first
    /// thing a reader copies into their own plugin directory, and the first
    /// thing they would conclude is broken about the application rather than
    /// about the example.
    #[test]
    fn every_example_plugin_in_the_repository_reads_and_runs() {
        let ctx = egui::Context::default();
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins");
        let installed = plugins::read(&examples);
        assert!(
            installed.broken.is_empty(),
            "an example does not even parse: {:?}",
            installed.broken
        );
        assert!(
            !installed.plugins.is_empty(),
            "the repository ships at least one example"
        );

        let mut app = opened_app(WorkspaceView::Disassembly);
        app.plugins = installed;
        let ids: Vec<String> = app
            .plugins
            .plugins
            .iter()
            .map(|plugin| plugin.id.clone())
            .collect();
        for id in ids {
            let wanted = app
                .plugins
                .get(&id)
                .map(plugins::Plugin::wanted)
                .unwrap_or_default();
            app.preferences.plugins.insert(
                id.clone(),
                Consent {
                    enabled: true,
                    granted: wanted,
                },
            );
            super::run_one(&mut app, &ctx, &id);
            let outcome = app.script.last.as_ref().expect("it ran");
            assert_eq!(outcome.failure, None, "{id} failed: {outcome:?}");
        }
    }

    /// Two small labels drawn side by side in a wrapped row land on top of one
    /// another, which reads as a corrupted line rather than as a version and
    /// an author. They are one line now, and this is what says so.
    #[test]
    fn a_version_and_an_author_are_one_line_rather_than_two_on_top_of_each_other() {
        let ctx = egui::Context::default();
        let root = installed(
            "provenance",
            r#"(name: "Namer", version: "1.0", author: "Desdec", script: "plugin.rhai")"#,
            "",
        );
        let mut app = app_with(&root);
        app.dialogs.open(crate::app::Dialog::Plugins);

        let _ = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));
        let output = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));
        let drawn = crate::testing::drawn(&output.shapes);

        let mut seen: Vec<&eframe::egui::Pos2> = Vec::new();
        for (_, at) in &drawn {
            assert!(
                !seen.contains(&at),
                "two labels are drawn at {at:?}: {drawn:?}"
            );
            seen.push(at);
        }
        let line = drawn
            .iter()
            .find(|(said, _)| said.contains("1.0"))
            .map(|(said, _)| said.clone())
            .expect("the version is on screen");
        assert!(line.contains("Desdec"), "and the author is on it: {line}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
