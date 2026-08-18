//! The short-lived message shown over the workspace.
//!
//! Some actions have their whole effect outside the window — writing to the
//! clipboard is the only one so far — and without a word on screen they look
//! exactly like an action that did nothing. This says what happened, then
//! takes itself away rather than becoming one more thing to dismiss.

use eframe::egui;

use crate::{app::DesdecApp, preferences::accent};

/// How far above the bottom of the window the notice sits, clear of the
/// status bar.
const BOTTOM_MARGIN: f32 = 52.0;

/// The last part of its life is spent fading, so it leaves rather than blinks
/// out.
const FADE: f64 = 0.6;

/// How often the notice asks to be redrawn while it is on screen: the
/// interface is otherwise still, and a fade needs frames.
const FRAME: std::time::Duration = std::time::Duration::from_millis(33);

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    let now = ctx.input(|input| input.time);
    let Some(notice) = &app.notice else {
        return;
    };
    if now >= notice.until {
        app.notice = None;
        return;
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "an opacity between zero and one loses nothing that matters"
    )]
    let opacity = ((notice.until - now) / FADE).clamp(0.0, 1.0) as f32;
    let text = notice.text.clone();
    let accent = accent(app.preferences.theme);
    let screen = ctx.screen_rect();

    egui::Area::new(egui::Id::new("notice"))
        .order(egui::Order::Foreground)
        // Never interactive: a message that answers nothing must not be able
        // to swallow a click meant for what is underneath it.
        .interactable(false)
        .fixed_pos(egui::pos2(
            screen.center().x,
            screen.bottom() - BOTTOM_MARGIN,
        ))
        .pivot(egui::Align2::CENTER_BOTTOM)
        .show(ctx, |ui| {
            ui.set_opacity(opacity);
            egui::Frame::popup(ui.style())
                .stroke(egui::Stroke::new(1.0_f32, accent))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(text).monospace());
                });
        });

    // The interface redraws on input alone, so the fade needs asking for.
    ctx.request_repaint_after(FRAME);
}

#[cfg(test)]
mod tests {
    use crate::{
        app::{DesdecApp, WorkspaceView},
        i18n::Text,
        testing::window_input,
    };
    use eframe::egui;

    /// Copying writes to the clipboard, which is invisible; the notice is the
    /// only thing telling the reader it happened.
    #[test]
    fn copying_leaves_a_notice_that_names_what_was_copied() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);

        app.copy_to_clipboard(&ctx, "/tmp/binary", Text::PathCopied);

        let notice = app.notice.as_ref().expect("copying says so");
        assert!(notice.text.contains("/tmp/binary"));

        // Two frames: an area anchored by its own centre has to be measured
        // before it can be placed, so the first frame is what egui learns its
        // size from. The application redraws for the fade anyway.
        let _ = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));
        let output = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));
        assert!(
            crate::testing::drawn_text(&output.shapes).contains("/tmp/binary"),
            "the notice must reach the screen"
        );
    }

    /// It has to go on its own: a message that stayed would be one more thing
    /// to dismiss.
    #[test]
    fn a_notice_expires() {
        let ctx = egui::Context::default();
        let mut app = DesdecApp::for_test(None, WorkspaceView::Overview);
        app.copy_to_clipboard(&ctx, "/tmp/binary", Text::PathCopied);
        app.notice.as_mut().expect("just set").until = 0.0;

        let _ = ctx.run(window_input(), |ctx| super::show(&mut app, ctx));

        assert!(app.notice.is_none(), "the notice must take itself away");
    }
}
