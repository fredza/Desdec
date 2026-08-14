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
            ui.label(app.t(Text::AboutDescription));
            ui.small(app.t(Text::LegalNotice));
        });
    app.dialogs.about = open;
}
