use eframe::egui;

use crate::{app::DesdecApp, commands::Command, i18n::Text, ui::MUTED};

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.command_palette {
        return;
    }

    let mut open = true;
    let mut chosen = None;
    egui::Window::new(app.t(Text::PaletteTitle))
        // A stable id keeps the window in place when the title is translated.
        .id(egui::Id::new("desdec.command_palette"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(egui::vec2(520.0, 420.0))
        .min_width(360.0)
        .min_height(260.0)
        .show(ctx, |ui| {
            chosen = contents(app, ctx, ui);
        });

    if let Some(command) = chosen {
        app.run_command(ctx, command);
        open = false;
    }
    app.dialogs.command_palette = open;
}

fn contents(app: &mut DesdecApp, ctx: &egui::Context, ui: &mut egui::Ui) -> Option<Command> {
    ui.label(app.t(Text::SearchAction));
    let search_hint = app.t(Text::SearchHint);
    let response =
        ui.add(egui::TextEdit::singleline(&mut app.palette.query).hint_text(search_hint));
    response.request_focus();
    if response.changed() {
        app.palette.selected = 0;
    }
    ui.add_space(8.0);

    let matches = matching_commands(app);
    let mut chosen = keyboard_selection(app, ctx, &matches);
    egui::ScrollArea::vertical()
        .max_height(ui.available_height().max(160.0))
        .show(ui, |ui| {
            for (index, command) in matches.iter().copied().enumerate() {
                ui.horizontal(|ui| {
                    let selected = index == app.palette.selected;
                    let label = command.label(app.preferences.language);
                    let enabled = !matches!(command, Command::AiAssistance);
                    if ui
                        .add_enabled(enabled, egui::SelectableLabel::new(selected, label))
                        .clicked()
                    {
                        chosen = Some(command);
                        app.palette.selected = index;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(app.shortcut_label(command)).color(MUTED));
                    });
                });
            }
        });
    chosen
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
fn keyboard_selection(
    app: &mut DesdecApp,
    ctx: &egui::Context,
    matches: &[Command],
) -> Option<Command> {
    let Some(last) = matches.len().checked_sub(1) else {
        app.palette.selected = 0;
        return None;
    };

    app.palette.selected = app.palette.selected.min(last);
    if ctx.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
        app.palette.selected = (app.palette.selected + 1) % matches.len();
    }
    if ctx.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
        app.palette.selected = app.palette.selected.checked_sub(1).unwrap_or(last);
    }
    ctx.input(|input| input.key_pressed(egui::Key::Enter))
        .then(|| matches[app.palette.selected])
}
