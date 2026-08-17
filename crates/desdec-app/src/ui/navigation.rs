//! The navigation menu: where a binary is opened, and where the workspace
//! views and the tools are reached.
//!
//! The menu is resizable, and what it shows follows the width it was given.
//! Dragged narrow it becomes a rail of icons — the views are still one click
//! away, each naming itself on hover; given more room it puts the labels back
//! beside the icons; wider still it shows the sections a first-time reader
//! needs. The width itself is a preference, so a menu left as a rail reopens
//! as a rail.

use eframe::egui;

use crate::{
    app::{DesdecApp, WorkspaceView},
    commands::Command,
    i18n::Text,
    icons::{self, Icon},
    preferences::accent,
    ui::{ERROR, MUTED, section_title},
};

/// The panel's own identifier, needed to read back the width a drag left.
const PANEL_ID: &str = "navigation";

/// Width the menu opens at until it is dragged somewhere else.
pub const DEFAULT_WIDTH: u16 = 276;
/// Narrowest the menu goes: one icon button and its padding.
pub const MINIMUM_WIDTH: f32 = 56.0;
const MAXIMUM_WIDTH: f32 = 420.0;

/// Below this the menu is a rail of icons; above it, labels fit beside them.
const LABEL_WIDTH: f32 = 132.0;
/// Below this the sections that explain themselves are left out: their titles
/// and the recent files would take the room the views need.
const SECTION_WIDTH: f32 = 218.0;

const ICON_BUTTON: egui::Vec2 = egui::vec2(30.0, 28.0);
const ROW_HEIGHT: f32 = 32.0;
const PRIMARY_BUTTON_HEIGHT: f32 = 34.0;
const SECONDARY_BUTTON_HEIGHT: f32 = 30.0;

/// How much of itself the menu can show at its current width.
///
/// One decision, taken once per frame from the width alone, so every part of
/// the menu agrees on what it is: nothing here reads the width a second time
/// and reaches a different conclusion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Density {
    /// Icons only, each naming itself on hover.
    Rail,
    /// Icons with their labels.
    Compact,
    /// Labels, section titles, recent files, and the binary's own actions.
    Full,
}

impl Density {
    #[must_use]
    pub fn of_width(width: f32) -> Self {
        if width < LABEL_WIDTH {
            Self::Rail
        } else if width < SECTION_WIDTH {
            Self::Compact
        } else {
            Self::Full
        }
    }

    const fn shows_labels(self) -> bool {
        !matches!(self, Self::Rail)
    }

    const fn shows_sections(self) -> bool {
        matches!(self, Self::Full)
    }
}

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.navigation_open {
        return;
    }

    let width = f32::from(app.preferences.navigation_width).clamp(MINIMUM_WIDTH, MAXIMUM_WIDTH);
    // Two things can move the menu: a drag, which egui records in the panel's
    // own memory, and the preference, which anything else sets. They are left
    // agreeing at the end of every frame, and that agreed width is kept, so
    // whichever of the two no longer matches it is the one that just changed
    // and the one to follow.
    if agreed_width(ctx).is_some_and(|agreed| (width - agreed).abs() > 0.5) {
        store_panel_width(ctx, width);
    }
    let panel = egui::SidePanel::left(PANEL_ID)
        .resizable(true)
        .width_range(MINIMUM_WIDTH..=MAXIMUM_WIDTH)
        .default_width(width)
        .show(ctx, |ui| {
            // The width egui hands the contents is what the reader dragged to,
            // so the layout answers to the drag on the very same frame.
            let density = Density::of_width(ui.available_width());
            header(app, ui, density);
            binary_actions(app, ctx, ui, density);
            ui.add_space(if density.shows_sections() { 14.0 } else { 8.0 });
            views_section(app, ui, density);
            ui.add_space(8.0);
            tools_section(app, ctx, ui, density);

            // One line of help, and only the part a reader cannot discover by
            // looking: that the edge is draggable and the width is kept. The
            // second line explained how to reopen a menu that is, at that
            // moment, open.
            if density.shows_sections() {
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.small(egui::RichText::new(app.t(Text::DragToResizeMenu)).color(MUTED));
                    ui.separator();
                });
            }
        });

    remember_width(app, ctx, panel.response.rect.width());
}

/// Keeps the width the reader dragged to, so the menu reopens as they left it.
fn remember_width(app: &mut DesdecApp, ctx: &egui::Context, width: f32) {
    let width = width.clamp(MINIMUM_WIDTH, MAXIMUM_WIDTH).round();
    // The clamp above keeps this inside `u16`; the cast cannot lose anything a
    // menu width could hold.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to a range far inside u16"
    )]
    let rounded = width as u16;
    if rounded != app.preferences.navigation_width {
        app.preferences.navigation_width = rounded;
    }
    set_agreed_width(ctx, width);
}

/// The width the panel and the preference were last left agreeing on.
fn agreed_width(ctx: &egui::Context) -> Option<f32> {
    ctx.data(|data| data.get_temp(agreed_id()))
}

fn set_agreed_width(ctx: &egui::Context, width: f32) {
    ctx.data_mut(|data| data.insert_temp(agreed_id(), width));
}

fn agreed_id() -> egui::Id {
    egui::Id::new("desdec.navigation.agreed_width")
}

/// Sets the width directly, for the buttons that fold the menu to its rail or
/// unfold it again.
///
/// The preference alone is not enough: a panel remembers the width it was last
/// dragged to, and that memory wins over `default_width` on the next frame, so
/// the choice has to be written where the drag would have written it.
fn set_width(app: &mut DesdecApp, ctx: &egui::Context, width: f32) {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "every caller passes one of this module's own widths"
    )]
    let rounded = width.round() as u16;
    app.preferences.navigation_width = rounded;
    store_panel_width(ctx, width);
    set_agreed_width(ctx, width);
}

fn store_panel_width(ctx: &egui::Context, width: f32) {
    let id = egui::Id::new(PANEL_ID);
    let Some(state) = egui::containers::panel::PanelState::load(ctx, id) else {
        return; // Never shown yet: `default_width` will do it.
    };
    let rect = egui::Rect::from_min_size(state.rect.min, egui::vec2(width, state.rect.height()));
    ctx.data_mut(|data| {
        data.insert_persisted(id, egui::containers::panel::PanelState { rect });
    });
}

/// The controls that size and close the menu, and nothing else.
///
/// The mark and the name used to sit here too, an arm's length below the same
/// mark and name in the action bar. Repeating them cost the menu a row of
/// height at every width and told the reader nothing they were not already
/// looking at, so the header is now only what the action bar cannot offer:
/// the width of this panel.
fn header(app: &mut DesdecApp, ui: &mut egui::Ui, density: Density) {
    let accent = accent(app.preferences.theme);
    ui.add_space(4.0);

    if !density.shows_labels() {
        ui.vertical_centered(|ui| {
            let widen = app.tooltip(
                icons::sized_button(ui, Icon::ExpandRight, None, false, accent, ICON_BUTTON),
                app.t(Text::WidenMenu),
            );
            if widen.clicked() {
                set_width(app, ui.ctx(), f32::from(DEFAULT_WIDTH));
            }
        });
        ui.add_space(4.0);
        ui.separator();
        return;
    }

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let close = app.tooltip(
            icons::sized_button(ui, Icon::Close, None, false, accent, ICON_BUTTON),
            app.t(Text::CollapseMenu),
        );
        if close.clicked() {
            app.navigation_open = false;
        }
        let narrow = app.tooltip(
            icons::sized_button(ui, Icon::CollapseLeft, None, false, accent, ICON_BUTTON),
            app.t(Text::NarrowMenu),
        );
        if narrow.clicked() {
            set_width(app, ui.ctx(), MINIMUM_WIDTH);
        }
    });
    ui.add_space(4.0);
    ui.separator();
}

fn binary_actions(app: &mut DesdecApp, ctx: &egui::Context, ui: &mut egui::Ui, density: Density) {
    let accent = accent(app.preferences.theme);
    ui.add_space(8.0);

    if !density.shows_labels() {
        ui.vertical_centered(|ui| {
            let open = app.tooltip(
                icons::sized_button(ui, Icon::Open, None, false, accent, ICON_BUTTON),
                &app.command_tooltip(Command::OpenBinary),
            );
            if open.clicked() {
                app.choose_binary(ctx);
            }
            if app.is_analysing() {
                ui.add_space(4.0);
                ui.spinner();
            }
        });
        return;
    }

    let open = ui.add_sized(
        [ui.available_width(), PRIMARY_BUTTON_HEIGHT],
        egui::Button::new(egui::RichText::new(app.t(Text::OpenBinary)).color(egui::Color32::WHITE))
            .fill(accent.gamma_multiply(0.72))
            .truncate(),
    );
    if open.clicked() {
        app.choose_binary(ctx);
    }

    if app.is_analysing() {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.small(egui::RichText::new(app.t(Text::StatusWorking)).color(MUTED));
        });
        let cancel = ui.add_sized(
            [ui.available_width(), SECONDARY_BUTTON_HEIGHT],
            egui::Button::new(
                egui::RichText::new(app.t(Text::CancelAnalysis)).color(egui::Color32::WHITE),
            )
            .fill(ERROR.gamma_multiply(0.78))
            .truncate(),
        );
        if cancel.clicked() {
            app.cancel_analysis();
        }
    }

    if app.analysis.is_some() {
        ui.add_space(6.0);
        let close = ui.add_sized(
            [ui.available_width(), SECONDARY_BUTTON_HEIGHT],
            egui::Button::new(app.t(Text::CloseBinary)).truncate(),
        );
        if close.clicked() {
            app.close_binary();
        }
    }

    if density.shows_sections() {
        recent_binaries(app, ctx, ui);
    }
}

fn recent_binaries(app: &mut DesdecApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let recent = app.recent_binaries().to_vec();
    if recent.is_empty() {
        return;
    }

    ui.add_space(6.0);
    egui::CollapsingHeader::new(section_title(app.t(Text::RecentBinaries)))
        .id_salt("navigation.recent_binaries")
        .default_open(true)
        .show(ui, |ui| {
            let mut selected = None;
            for path in recent {
                let label = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                let response = ui
                    .add_sized(
                        [ui.available_width(), SECONDARY_BUTTON_HEIGHT],
                        egui::Button::new(label).truncate(),
                    )
                    .on_hover_text(path.display().to_string());
                if response.clicked() {
                    selected = Some(path);
                }
            }
            if ui.button(app.t(Text::ClearRecentBinaries)).clicked() {
                app.clear_recent_binaries();
            }
            if let Some(path) = selected {
                app.open_recent_binary(ctx, path);
            }
        });
}

/// The workspace views: the part of the menu that survives at every width.
/// Views the toolbar keeps to itself.
///
/// Each is one click away up there, and repeating it in the menu made the two
/// into copies of one another — the menu's copy being the one you have to open
/// something to reach. The overview is deliberately not among them: it is
/// where a reader starts, and a menu with no way back to it is a menu you have
/// to leave in order to use.
const TOOLBAR_ONLY: &[WorkspaceView] = &[
    WorkspaceView::Disassembly,
    WorkspaceView::Decompile,
    WorkspaceView::Functions,
    WorkspaceView::Strings,
    WorkspaceView::Patches,
];

/// The views the menu offers.
///
/// With the toolbar hidden it offers all of them: it is then the only place
/// left that shows them, and the rest would be reachable by shortcut or the
/// command palette alone.
fn views_to_offer(app: &DesdecApp) -> Vec<WorkspaceView> {
    WorkspaceView::ALL
        .iter()
        .copied()
        .filter(|view| !app.preferences.show_toolbar || !TOOLBAR_ONLY.contains(view))
        .collect()
}

fn views_section(app: &mut DesdecApp, ui: &mut egui::Ui, density: Density) {
    let views = views_to_offer(app);
    if views.is_empty() {
        return;
    }
    if density.shows_sections() {
        ui.label(section_title(app.t(Text::Exploration)));
        ui.add_space(4.0);
    } else {
        ui.separator();
    }

    for view in views {
        let selected = app.active_view == view;
        if entry(app, ui, view.glyph(), app.t(view.text()), selected, density)
            .on_hover_text(app.command_tooltip(view.command()))
            .clicked()
        {
            app.select_view(view);
        }
    }
}

fn tools_section(app: &mut DesdecApp, ctx: &egui::Context, ui: &mut egui::Ui, density: Density) {
    if density.shows_sections() {
        ui.add_space(6.0);
        ui.label(section_title(app.t(Text::Tools)));
        ui.add_space(4.0);
    } else {
        ui.separator();
    }

    for (icon, text, command) in [
        (Icon::Palette, Text::CommandPalette, Command::CommandPalette),
        (Icon::Preferences, Text::Preferences, Command::Preferences),
        (Icon::About, Text::About, Command::About),
    ] {
        if entry(app, ui, icon, app.t(text), false, density)
            .on_hover_text(app.command_tooltip(command))
            .clicked()
        {
            app.run_command(ctx, command);
        }
    }
}

/// One menu entry: an icon at every width, with its label when there is room.
///
/// The icon keeps the same place in the row whether or not the label is drawn,
/// so widening the menu adds text beside what the reader was already aiming
/// at rather than moving it.
fn entry(
    app: &DesdecApp,
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    selected: bool,
    density: Density,
) -> egui::Response {
    let accent = accent(app.preferences.theme);
    if !density.shows_labels() {
        return ui
            .vertical_centered(|ui| {
                icons::sized_button(ui, icon, None, selected, accent, ICON_BUTTON)
            })
            .inner;
    }

    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::click());
    let visuals = ui.style().interact_selectable(&response, selected);
    if selected || response.hovered() {
        let fill = if selected {
            accent.gamma_multiply(0.42)
        } else {
            visuals.bg_fill
        };
        ui.painter().rect_filled(rect.shrink(1.0), 5.0, fill);
    }

    let glyph = egui::Rect::from_min_size(
        rect.left_top() + egui::vec2(4.0, 0.0),
        egui::vec2(ICON_BUTTON.x, rect.height()),
    );
    icons::draw(ui.painter(), glyph, icon, visuals.text_color());
    ui.painter().text(
        egui::pos2(glyph.right() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Button.resolve(ui.style()),
        visuals.text_color(),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::WorkspaceView,
        testing::{drawn_text, opened_app, window_input},
    };

    /// A width as the preference holds it.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "test widths are this module's own constants"
    )]
    fn points(width: f32) -> u16 {
        width.round() as u16
    }

    fn frame(app: &mut DesdecApp, ctx: &egui::Context) -> String {
        let output = ctx.run(window_input(), |ctx| show(app, ctx));
        drawn_text(&output.shapes)
    }

    /// Widths on either side of each threshold: the menu must lay out at all
    /// of them, and the views must be reachable at every single one — that is
    /// what the rail exists for.
    #[test]
    fn the_menu_lays_out_at_every_width() {
        let ctx = egui::Context::default();
        for width in [
            MINIMUM_WIDTH,
            LABEL_WIDTH - 1.0,
            LABEL_WIDTH,
            SECTION_WIDTH - 1.0,
            SECTION_WIDTH,
            MAXIMUM_WIDTH,
        ] {
            let mut app = opened_app(WorkspaceView::Overview);
            app.navigation_open = true;
            app.preferences.navigation_width = points(width);

            for language in crate::i18n::Language::ALL {
                app.preferences.language = *language;
                let output = ctx.run(window_input(), |ctx| show(&mut app, ctx));
                assert!(
                    !output.shapes.is_empty(),
                    "the menu drew nothing at {width} in {language:?}"
                );
            }
        }
    }

    /// The rail shows no labels — that is the whole point of it — and the wide
    /// menu names every view it offers.
    #[test]
    fn labels_appear_only_once_there_is_room_for_them() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Overview);
        app.navigation_open = true;

        app.preferences.navigation_width = points(MINIMUM_WIDTH);
        let rail = frame(&mut app, &ctx);
        for view in WorkspaceView::ALL {
            assert!(
                !rail.contains(app.t(view.text())),
                "the rail should not have room for {view:?}"
            );
        }

        app.preferences.navigation_width = DEFAULT_WIDTH;
        let wide = frame(&mut app, &ctx);
        for view in views_to_offer(&app) {
            assert!(
                wide.contains(app.t(view.text())),
                "the wide menu must name {view:?}"
            );
        }
    }

    /// The menu leaves the toolbar's own views to the toolbar, and keeps the
    /// rest — including the overview, which is where a reader starts.
    #[test]
    fn the_menu_leaves_out_what_the_toolbar_already_offers() {
        let mut app = opened_app(WorkspaceView::Overview);
        app.preferences.show_toolbar = true;

        let offered = views_to_offer(&app);
        for view in TOOLBAR_ONLY {
            assert!(
                !offered.contains(view),
                "{view:?} is in the toolbar and in the menu"
            );
        }
        assert!(
            offered.contains(&WorkspaceView::Overview),
            "the way back to the overview must stay in the menu"
        );
        assert_eq!(
            offered.len(),
            WorkspaceView::ALL.len() - TOOLBAR_ONLY.len(),
            "every other view belongs to the menu"
        );
    }

    /// What the menu hands to the toolbar, the toolbar must actually show, or
    /// a view would be offered in neither.
    #[test]
    fn everything_the_menu_defers_is_in_the_toolbar() {
        for view in TOOLBAR_ONLY {
            assert!(
                crate::ui::action_bar::VIEW_ACTIONS.contains(view),
                "{view:?} is in neither the toolbar nor the menu"
            );
        }
    }

    /// With the toolbar hidden, the menu is the only place left, so it carries
    /// every view rather than the handful the toolbar was not showing.
    #[test]
    fn hiding_the_toolbar_puts_every_view_back_in_the_menu() {
        let mut app = opened_app(WorkspaceView::Overview);
        app.preferences.show_toolbar = false;

        assert_eq!(views_to_offer(&app), WorkspaceView::ALL.to_vec());
    }

    /// A dragged width is a choice, and choices are kept.
    #[test]
    fn the_dragged_width_is_remembered() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Overview);
        app.navigation_open = true;
        app.preferences.navigation_width = DEFAULT_WIDTH;
        let _ = frame(&mut app, &ctx);

        // What a drag leaves behind: egui's own width for the panel.
        store_panel_width(&ctx, 160.0);
        let _ = frame(&mut app, &ctx);

        assert_eq!(app.preferences.navigation_width, 160);
    }

    /// Each threshold does what it says, so a change to one of them is a
    /// deliberate change to the menu rather than an accident of arithmetic.
    #[test]
    fn width_decides_what_the_menu_shows() {
        assert_eq!(Density::of_width(MINIMUM_WIDTH), Density::Rail);
        assert_eq!(Density::of_width(LABEL_WIDTH - 1.0), Density::Rail);
        assert_eq!(Density::of_width(LABEL_WIDTH), Density::Compact);
        assert_eq!(Density::of_width(SECTION_WIDTH - 1.0), Density::Compact);
        assert_eq!(Density::of_width(SECTION_WIDTH), Density::Full);
        assert_eq!(Density::of_width(MAXIMUM_WIDTH), Density::Full);

        assert!(!Density::Rail.shows_labels());
        assert!(Density::Compact.shows_labels());
        assert!(!Density::Compact.shows_sections());
        assert!(Density::Full.shows_sections());
    }

    /// A whole frame — toolbar, menu, workspace — at each width the menu
    /// changes shape at. The panels share the window between them, so a menu
    /// that lays out on its own can still leave the rest with nothing.
    #[test]
    fn a_whole_frame_lays_out_at_every_menu_width() {
        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.navigation_open = true;

        for width in [MINIMUM_WIDTH, LABEL_WIDTH, SECTION_WIDTH, MAXIMUM_WIDTH] {
            app.preferences.navigation_width = points(width);
            let output = ctx.run(window_input(), |ctx| app.run_frame(ctx));
            assert!(
                !output.shapes.is_empty(),
                "the frame drew nothing with a menu of {width}"
            );
            assert_eq!(
                app.preferences.navigation_width,
                points(width),
                "the menu did not settle at the width it was given"
            );
        }
    }

    /// No two views may wear the same icon: in the rail the icon is all the
    /// reader has to tell one entry from another.
    #[test]
    fn every_view_has_its_own_icon() {
        let mut seen: Vec<Icon> = Vec::new();
        for view in WorkspaceView::ALL {
            let glyph = view.glyph();
            assert!(
                !seen.contains(&glyph),
                "{view:?} shares its icon with another view"
            );
            seen.push(glyph);
        }
    }
}
