//! Detailed x86/x86-64 decoding, synchronised with the local pseudo-code.
use crate::{
    app::DesdecApp,
    commands::Command,
    i18n::{Language, Text, text},
    icons::{self, Icon},
    patches::Patches,
    preferences::accent,
    ui::ERROR,
    ui::MUTED,
    ui::ROW_HEIGHT,
    ui::decompile,
    ui::syntax,
    walk,
};
use desdec_core::{Analysis, Instruction, StackSlot, Trace, operand};
use eframe::egui;

/// Bytes a pending patch would write, marked so an edited row is never taken
/// for what the file currently holds.
const PATCHED: egui::Color32 = egui::Color32::from_rgb(224, 164, 104);

/// Where a jump lands. Green, and the only colour in the gutter: a branch is
/// the one thing in a listing the reader follows rather than reads.
const JUMP: egui::Color32 = egui::Color32::from_rgb(91, 201, 139);

/// A row the reader has written on. Blue, and quiet: the mark says "there is
/// something of yours here", and the note itself is at the end of the line.
const NOTE: egui::Color32 = egui::Color32::from_rgb(94, 158, 236);

/// Room kept to the left of every row: the note dot, the bookmark star, and
/// the arrows, in that order from the edge.
const GUTTER_WIDTH: f32 = 36.0;

/// Where the note dot sits in the gutter, from its left edge, and how big it
/// is. Outside the arrows' lanes on purpose: a mark that a jump can be drawn
/// through is one the reader has to look twice at.
const NOTE_DOT_CENTRE: f32 = 4.5;
const NOTE_DOT_RADIUS: f32 = 3.5;

/// Where the bookmark star sits, from the same edge — just right of the dot,
/// so a row carrying both reads as two marks rather than one smudge.
const BOOKMARK_OFFSET: f32 = 9.0;

/// Space between two arrows drawn over the same rows.
const LANE_SPACING: f32 = 5.0;

/// How many lanes the gutter holds before arrows start sharing one. Past four
/// the gutter reads as a hedge, and the jump being followed is lost in it.
const MAX_LANES: usize = 4;

/// Rows drawn before the first instruction: the column titles.
const LEADING: usize = 1;

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// What one frame of the listing needs besides the analysis itself.
///
/// Gathered into one value rather than passed one by one: the listing already
/// carries its selection, its scrolling and its attention mark, and every
/// further parameter made the signature harder to read than the code under it.
struct Listing<'a> {
    patches: &'a Patches,
    stack: &'a Trace,
    /// The open file, read to say what an operand points at.
    file: &'a [u8],
    /// Instruction indices where a section begins, from
    /// [`crate::app::DesdecApp::section_starts`].
    sections: &'a [usize],
    /// The theme's accent, for the heading of each section.
    accent: egui::Color32,
    /// What the reader has written about these addresses.
    notes: &'a crate::annotations::Annotations,
    /// Whether hovering a row says what its operand designates.
    hints: bool,
    language: Language,
}

/// Draws the disassembly, returning what the reader asked of it.
pub fn show(app: &mut DesdecApp, ui: &mut egui::Ui) -> Action {
    // Before anything is laid out: the arrow keys move the selection, and the
    // row they land on has to be brought into view on this frame — the scroll
    // offset is decided as the listing is prepared, further down.
    step_with_arrow_keys(app, ui.ctx());
    // The transport first, because it is what moves the selection everything
    // below is drawn around.
    transport(app, ui);

    let language = app.preferences.language;
    let listing = Listing {
        patches: &app.patches,
        stack: &app.stack,
        file: &app.file_bytes,
        sections: &app.section_starts,
        accent: accent(app.preferences.theme),
        notes: &app.annotations,
        // The general switch still governs: a reader who turned the tooltips
        // off asked for a listing and nothing else.
        hints: app.preferences.show_tooltips && app.preferences.show_operand_hints,
        language,
    };
    let Some(analysis) = &app.analysis else {
        return Action::default();
    };
    if analysis.instructions.is_empty() {
        ui.label(egui::RichText::new(text(language, Text::NoDisassembly)).color(MUTED));
        return Action::default();
    }
    let mut action = Action::default();
    let selected = app.selected_instruction;
    ui.horizontal(|ui| {
        let patchable = selected
            .is_some_and(|address| crate::patches::file_offset_of(analysis, address).is_some());
        let button = ui.add_enabled(
            selected.is_some() && patchable,
            egui::Button::new(text(language, Text::EditInstruction)),
        );
        if button.clicked() {
            action.edit = selected;
        }
        if selected.is_some() && !patchable {
            ui.label(egui::RichText::new(text(language, Text::NotPatchable)).color(MUTED));
        } else {
            ui.small(text(language, Text::LocalDecoders));
        }
    });
    // A listing that stopped at the decoder's limit looks exactly like a
    // program that ends there, so it says which one this is.
    if analysis.code_truncated {
        ui.colored_label(ERROR, text(language, Text::TruncatedDisassembly));
    }
    stack_summary(ui, analysis, listing.stack, selected, language);
    // What the keys and the arrows in the gutter do, where they are used:
    // neither announces itself, and a reader who never presses a key would
    // otherwise scroll a hundred thousand rows by hand.
    ui.small(egui::RichText::new(text(language, Text::DisassemblyHelp)).color(MUTED));
    ui.add_space(8.0);
    let scroll_target = app.pending_instruction_scroll;
    let attention = decompile::active_attention(ui.ctx(), &mut app.instruction_attention);
    let mut asked = Asked::default();
    let selected_instruction = &mut app.selected_instruction;
    let pending_scroll = &mut app.pending_instruction_scroll;
    ui.columns(2, |columns| {
        columns[1].strong(text(language, Text::PseudoCode));
        columns[1].small(text(language, Text::PseudoCodeHelp));
        // Clicking a pseudo-code line here only moves the selection: the
        // assembly it stands for is already in the left column, so the address
        // the panel reports has no window to open.
        let _selected_by_click = decompile::panel(
            &mut columns[1],
            analysis,
            selected_instruction,
            scroll_target,
            pending_scroll,
            attention,
        );
        asked = instructions(
            &mut columns[0],
            analysis,
            selected_instruction,
            scroll_target,
            pending_scroll,
            attention,
            &listing,
        );
    });
    if *pending_scroll == scroll_target {
        *pending_scroll = None;
    }
    action.inspect = asked.inspect;
    // The notes are the application's, not the listing's, so they are written
    // here — where the borrow the listing held on them has ended.
    if let Some(address) = asked.bookmark {
        app.annotations.toggle_bookmark(address);
    }
    if let Some(address) = asked.annotate {
        app.annotating_address = Some(address);
        app.dialogs.open(crate::app::Dialog::Annotation);
    }
    if let Some(address) = asked.references {
        app.references_address = Some(address);
        app.dialogs.open(crate::app::Dialog::References);
    }
    if let Some(address) = asked.follow {
        app.follow_in_dump(address);
    }
    action
}

/// What a click in the listing asked for, gathered as the rows are drawn.
#[derive(Default)]
struct Asked {
    /// An instruction whose operand is to be explained.
    inspect: Option<u64>,
    /// An address whose note is to be opened.
    annotate: Option<u64>,
    /// An address to mark, or to unmark.
    bookmark: Option<u64>,
    /// An address whose references are to be listed.
    references: Option<u64>,
    /// An address to look at byte by byte.
    follow: Option<u64>,
}

impl Asked {
    /// Takes in what one row asked, keeping whatever an earlier row asked
    /// first: only one menu can be open at a time, so at most one of these is
    /// ever set on a frame.
    fn merge(&mut self, row: &Self) {
        self.inspect = self.inspect.or(row.inspect);
        self.annotate = self.annotate.or(row.annotate);
        self.bookmark = self.bookmark.or(row.bookmark);
        self.references = self.references.or(row.references);
        self.follow = self.follow.or(row.follow);
    }
}

/// What the reader asked of the disassembly this frame.
#[derive(Default)]
pub struct Action {
    /// An instruction whose bytes are to be edited.
    pub edit: Option<u64>,
    /// An instruction whose operand is to be explained.
    pub inspect: Option<u64>,
}

/// The transport of the static walk: the buttons a tape deck would have.
///
/// Nothing here runs the program — Desdec never does — and the wording says
/// so: the walk *follows* the flow, one instruction at a time, through what
/// the bytes already say. Where a running program would consult a register or
/// a flag, the walk stops and reports that, and the two step buttons are what
/// let the reader choose the path a condition would otherwise decide.
fn transport(app: &mut DesdecApp, ui: &mut egui::Ui) {
    /// In the order the buttons sit, left to right, as on a tape deck.
    const BUTTONS: &[(Icon, Command)] = &[
        (Icon::WalkToEntry, Command::WalkToEntry),
        (Icon::WalkBack, Command::WalkBack),
        (Icon::WalkInto, Command::WalkStepInto),
        (Icon::WalkOver, Command::WalkStepOver),
        (Icon::WalkOut, Command::WalkStepOut),
        (Icon::WalkClear, Command::WalkClear),
    ];
    let language = app.preferences.language;
    let theme = accent(app.preferences.theme);
    let mut pressed = None;

    ui.horizontal(|ui| {
        let title = ui.strong(text(language, Text::StaticWalk));
        app.tooltip(title, text(language, Text::WalkHelp));
        for (icon, command) in BUTTONS {
            // A button that cannot move is drawn but not offered: a press it
            // swallowed would read as a broken transport.
            let enabled = app.can_run(*command);
            let tooltip = app.optional_command_tooltip(*command);
            let button = ui
                .add_enabled_ui(enabled, |ui| {
                    icons::button(ui, *icon, tooltip, false, theme)
                })
                .inner;
            if button.clicked() {
                pressed = Some(*command);
            }
        }
        if app.walk.steps() > 0 {
            ui.separator();
            ui.small(format!(
                "{} {}",
                text(language, Text::WalkSteps),
                app.walk.steps()
            ));
        }
        if app.walk.depth() > 0 {
            ui.separator();
            ui.small(format!(
                "{} {}",
                text(language, Text::WalkDepth),
                app.walk.depth()
            ));
        }
    });
    next_step(app, ui);
    if let Some(command) = pressed {
        app.run_command(ui.ctx(), command);
    }
}

/// What the next step would do, said before it is taken.
///
/// The reader is following a flow, and the two places a static walk parts
/// company with a running program — a condition it cannot evaluate, a target
/// it cannot resolve — are exactly the places they need warning of.
fn next_step(app: &DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    let Some(analysis) = &app.analysis else {
        return;
    };
    let Some(address) = app.selected_instruction else {
        return;
    };
    let lead = text(language, Text::NextStep);
    let sentence = match walk::next_move(analysis, address, false) {
        walk::Move::Call { target, .. } => {
            format!(
                "{lead} : {} {target:#x}",
                text(language, Text::NextStepCall)
            )
        }
        walk::Move::Branch(target) => {
            format!(
                "{lead} : {} {target:#x}",
                text(language, Text::NextStepBranch)
            )
        }
        walk::Move::Next(_) => format!("{lead} : {}", text(language, Text::NextStepNext)),
        walk::Move::Return => format!("{lead} : {}", text(language, Text::NextStepReturn)),
        walk::Move::Unresolved => text(language, Text::NextStepUnresolved).to_owned(),
        walk::Move::End => text(language, Text::NextStepEnd).to_owned(),
    };
    ui.small(egui::RichText::new(sentence).color(MUTED));
}

/// Moves the selection one instruction at a time, with the arrow keys.
///
/// Bare keys, so they are only taken when nothing else can want them: the
/// windows are drawn after this panel, and a key claimed here would never
/// reach the command palette walking its own list with the same arrows. A
/// text field holding the focus is left alone for the same reason.
///
/// The keys are consumed rather than merely read, so the scroll area does not
/// also act on them and move the listing out from under the selection.
fn step_with_arrow_keys(app: &mut DesdecApp, ctx: &egui::Context) {
    if app.dialogs.any_open() || ctx.wants_keyboard_input() {
        return;
    }
    let step = ctx.input_mut(|input| {
        isize::from(input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown))
            - isize::from(input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp))
    });
    if step == 0 {
        return;
    }
    let Some(analysis) = &app.analysis else {
        return;
    };
    let Some(last) = analysis.instructions.len().checked_sub(1) else {
        return;
    };
    // Nothing selected yet: the first key press lands on the first
    // instruction rather than on nothing.
    let row = app
        .selected_instruction
        .and_then(|address| analysis.instruction_index(address))
        .map_or(0, |row| row.saturating_add_signed(step).min(last));
    let Some(instruction) = analysis.instructions.get(row) else {
        return;
    };
    app.selected_instruction = Some(instruction.address);
    app.pending_instruction_scroll = Some(instruction.address);
    ctx.request_repaint();
}

fn instructions(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    selected_instruction: &mut Option<u64>,
    scroll_target: Option<u64>,
    pending_scroll: &mut Option<u64>,
    attention: Option<u64>,
    listing: &Listing,
) -> Asked {
    let language = listing.language;
    let mut asked = Asked::default();
    decompile::ensure_selected_instruction(analysis, selected_instruction);
    // Only the visible rows are laid out: a decoded binary reaches a hundred
    // thousand instructions, and a widget for each took seconds per frame.
    // The virtualiser draws no row the reader is scrolled away from, so
    // bringing one into view is done by offset rather than by asking an
    // unlaid-out row to scroll itself.
    // The row an instruction sits on is its index plus the section headings
    // above it, so the scroll offset is computed from that rather than from
    // the index alone — otherwise every heading drifts the listing by a row.
    let target_row = scroll_target
        .and_then(|address| analysis.instruction_index(address))
        .map(|index| row_of(listing.sections, LEADING, index));
    let area = decompile::listing_area_at_row(
        egui::ScrollArea::both().id_salt("instructions"),
        ui,
        target_row,
    );
    let total_rows = LEADING + analysis.instructions.len() + listing.sections.len();
    area.auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, total_rows, |ui, rows| {
            let body = window(&analysis.instructions, listing.sections, &rows, LEADING);
            // Where each row's gutter cell ended up, so the arrows can be
            // drawn over the whole listing once it is laid out. Positions
            // rather than row numbers: the listing is virtualised, and what
            // is on screen is whatever the scroll offset left there.
            let mut gutters: Vec<egui::Rect> = Vec::with_capacity(body.len());
            let mut instructions: Vec<&Instruction> = Vec::with_capacity(body.len());
            egui::Grid::new("disassembly")
                .num_columns(6)
                .striped(true)
                .min_row_height(ROW_HEIGHT)
                .show(ui, |ui| {
                    if rows.start == 0 {
                        // The gutter carries no title: the arrows drawn in it
                        // are their own legend, and a word here would be read
                        // as a column of data.
                        gutter_cell(ui);
                        for title in [
                            Text::Address,
                            Text::Bytes,
                            Text::Section,
                            Text::Stack,
                            Text::Instruction,
                        ] {
                            ui.strong(text(language, title));
                        }
                        ui.end_row();
                    }
                    for row in &body {
                        match row {
                            ListingRow::Section { first, last, count } => {
                                section_heading(ui, first, last, *count, listing);
                            }
                            ListingRow::Instruction(instruction) => {
                                let (gutter, row) = instruction_row(
                                    ui,
                                    analysis,
                                    instruction,
                                    selected_instruction,
                                    pending_scroll,
                                    attention,
                                    listing,
                                );
                                gutters.push(gutter);
                                instructions.push(instruction);
                                asked.merge(&row);
                            }
                        }
                    }
                });
            draw_jumps(ui, &instructions, &gutters, *selected_instruction);
        });
    asked
}

/// Where each section begins in the listing: one instruction index per
/// section, in listing order.
///
/// Indexed once per binary rather than looked for while drawing: the scan
/// walks every decoded instruction, and a large shared library holds eighteen
/// million of them. A section appears once, where its first instruction is —
/// the listing is ordered by address, so a section is one run of rows.
#[must_use]
pub fn section_starts(analysis: &Analysis) -> Vec<usize> {
    let mut starts: Vec<usize> = Vec::new();
    let mut current: Option<&str> = None;
    for (index, instruction) in analysis.instructions.iter().enumerate() {
        if current != Some(&*instruction.section) {
            current = Some(&instruction.section);
            starts.push(index);
        }
    }
    starts
}

/// The row an instruction is drawn on, counting the headings above it.
fn row_of(sections: &[usize], leading: usize, index: usize) -> usize {
    leading + index + sections.partition_point(|start| *start <= index)
}

/// What one row of the listing carries.
enum ListingRow<'a> {
    /// The head of a section: where it starts, where it ends, what it holds.
    Section {
        first: &'a Instruction,
        last: &'a Instruction,
        count: usize,
    },
    Instruction(&'a Instruction),
}

/// The rows of one window of the listing, headings included.
///
/// Built for the window rather than kept as a list of every row: a decoded
/// binary reaches eighteen million instructions, and a vector with an entry
/// for each would cost more than the instructions themselves. A row number is
/// an index plus the headings above it, which the section starts give in a
/// binary search.
fn window<'a>(
    instructions: &'a [Instruction],
    sections: &[usize],
    rows: &std::ops::Range<usize>,
    leading: usize,
) -> Vec<ListingRow<'a>> {
    let mut out = Vec::new();
    // A row sits at most one heading per section below its own index, so
    // nothing before this can fall inside the window.
    let mut index = rows.start.saturating_sub(leading + sections.len());
    while index < instructions.len() {
        let row = row_of(sections, leading, index);
        // A heading sits on the row just above the first instruction it names.
        if let Ok(section) = sections.binary_search(&index) {
            let end = sections
                .get(section + 1)
                .copied()
                .unwrap_or(instructions.len());
            if rows.contains(&(row - 1)) && end > index {
                out.push(ListingRow::Section {
                    first: &instructions[index],
                    last: &instructions[end - 1],
                    count: end - index,
                });
            }
        }
        if rows.contains(&row) {
            out.push(ListingRow::Instruction(&instructions[index]));
        }
        if row >= rows.end {
            break;
        }
        index += 1;
    }
    out
}

/// The head of a section: its name, its extent and what it holds.
///
/// A listing of eighteen million rows is one long column of hexadecimal, and
/// the section column alone answers "which section is this?" only for the row
/// under the pointer. A heading where each one begins is what makes the whole
/// listing read as the image it came from.
fn section_heading(
    ui: &mut egui::Ui,
    first: &Instruction,
    last: &Instruction,
    count: usize,
    listing: &Listing,
) {
    let rect = gutter_cell(ui);
    ui.painter().hline(
        ui.clip_rect().x_range(),
        rect.top() - 1.0,
        egui::Stroke::new(1.0_f32, listing.accent.gamma_multiply(0.4)),
    );
    ui.label(
        egui::RichText::new(&*first.section)
            .monospace()
            .strong()
            .color(listing.accent),
    );
    // The columns between are left empty: a heading names a run of rows, it
    // does not describe one.
    for _ in 0..3 {
        ui.label("");
    }
    ui.label(
        egui::RichText::new(format!(
            "{:#x} – {:#x} · {count} {}",
            first.address,
            last.address,
            text(listing.language, Text::SectionInstructions)
        ))
        .small()
        .color(MUTED),
    );
    ui.end_row();
}

/// One row of the listing: its gutter cell, its five columns, and whatever a
/// click on it asked for.
///
/// Returns where the gutter cell was drawn, so the jump arrows can be laid
/// over the listing once every row is in place, and the address whose operand
/// the reader asked to have explained.
fn instruction_row(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    instruction: &Instruction,
    selected_instruction: &mut Option<u64>,
    pending_scroll: &mut Option<u64>,
    attention: Option<u64>,
    listing: &Listing,
) -> (egui::Rect, Asked) {
    let language = listing.language;
    let mut asked = Asked::default();
    let gutter = gutter_cell(ui);
    // Marks in the margin, where a reader's eye runs down the listing looking
    // for the rows they worked on. The note itself rides at the end of the
    // line, which is off screen the moment the listing is scrolled to read the
    // bytes — so a row carrying one says so from the margin, which never
    // scrolls away.
    marks(ui, gutter, instruction.address, listing);
    let selected_fill =
        decompile::instruction_fill(ui, instruction.address, *selected_instruction, attention);
    let patch = listing.patches.patch_at(instruction.address);
    let address = ui
        .add(
            egui::Label::new(syntax::dim(
                ui,
                &format!("{:#018x}", instruction.address),
                selected_fill,
            ))
            .sense(egui::Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    // A patched row shows the bytes that would be written, marked, rather than
    // the ones still in the file: the listing must describe the binary being
    // built.
    let bytes = hex(patch.map_or(&instruction.bytes, |patch| &patch.replacement));
    match patch {
        Some(patch) => {
            ui.label(
                egui::RichText::new(format!("{bytes} *"))
                    .monospace()
                    .color(PATCHED),
            )
            .on_hover_text(format!(
                "{} {}",
                text(language, Text::OriginalBytes),
                hex(&patch.original)
            ));
        }
        None => {
            ui.label(syntax::dim(ui, &bytes, egui::Color32::TRANSPARENT));
        }
    }
    ui.label(syntax::dim(
        ui,
        &instruction.section,
        egui::Color32::TRANSPARENT,
    ));
    // The stack as it stands *before* this instruction runs, which is what a
    // reader stopped on it would see. Looked up by address rather than by row:
    // the listing is virtualised, and an index computed from a visible range
    // drifts the moment either changes.
    stack_cell(ui, analysis, listing.stack, instruction.address, language);
    // The reader's own name and comment ride at the end of the line, where an
    // assembler puts a comment — a column of their own sat off the right edge
    // of the listing, where nobody would ever scroll to find them.
    let assembly = ui
        .add(
            egui::Label::new(syntax::annotated(
                ui,
                &instruction.text,
                listing.notes.label(instruction.address),
                listing.notes.comment(instruction.address),
                selected_fill,
            ))
            .sense(egui::Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    // Hovering answers the question an address in a listing provokes, and only
    // when there is an answer: a bubble saying "nothing here" would follow the
    // pointer down the whole listing. Whether the operand computes an address
    // at all is cheap to ask; reading the file at it waits until the row is
    // actually hovered.
    let assembly = if listing.hints && operand::target_address(instruction).is_some() {
        assembly.on_hover_ui(|ui| target_hint(ui, analysis, instruction, listing))
    } else {
        assembly
    };
    // The right button carries what is asked *of* a row: what its operand
    // designates, and what the reader wants to say about it.
    assembly.context_menu(|ui| {
        if ui.button(text(language, Text::InspectOperand)).clicked() {
            asked.inspect = Some(instruction.address);
            ui.close_menu();
        }
        if ui.button(text(language, Text::EditNote)).clicked() {
            asked.annotate = Some(instruction.address);
            ui.close_menu();
        }
        if ui.button(text(language, Text::Bookmark)).clicked() {
            asked.bookmark = Some(instruction.address);
            ui.close_menu();
        }
        if ui.button(text(language, Text::ReferencesTo)).clicked() {
            asked.references = Some(instruction.address);
            ui.close_menu();
        }
        // Where the operand points if it points anywhere, and otherwise the
        // instruction's own bytes — both are answers to "show me what is
        // actually there".
        if ui.button(text(language, Text::FollowInDump)).clicked() {
            asked.follow =
                Some(operand::target_address(instruction).unwrap_or(instruction.address));
            ui.close_menu();
        }
    });
    if address.clicked() || assembly.clicked() {
        *selected_instruction = Some(instruction.address);
        *pending_scroll = Some(instruction.address);
        ui.ctx().request_repaint();
    }
    ui.end_row();
    (gutter, asked)
}

/// The mark a bookmarked row carries. Drawn from the font rather than painted,
/// so it sits on the same baseline as the rest of the row.
const BOOKMARK: &str = "\u{2605}";

/// What the reader has left on this row, drawn in the margin: a blue dot for a
/// note, a star for a bookmark.
///
/// Hovering the margin says what the note is, so a listing full of dots can be
/// read without opening each one in turn.
fn marks(ui: &mut egui::Ui, gutter: egui::Rect, address: u64, listing: &Listing) {
    if listing.notes.has_note(address) {
        ui.painter().circle_filled(
            egui::pos2(gutter.left() + NOTE_DOT_CENTRE, gutter.center().y),
            NOTE_DOT_RADIUS,
            NOTE,
        );
        // Interacted with after the fact rather than sensed when the cell was
        // allocated: the cell is the arrows' canvas, and giving the whole
        // width a tooltip would follow the pointer down every jump.
        let dot = egui::Rect::from_center_size(
            egui::pos2(gutter.left() + NOTE_DOT_CENTRE, gutter.center().y),
            egui::vec2(NOTE_DOT_RADIUS * 2.0, ROW_HEIGHT),
        );
        ui.interact(
            dot,
            ui.id().with(("note_dot", address)),
            egui::Sense::hover(),
        )
        .on_hover_text(note_text(address, listing));
    }
    if listing.notes.is_bookmarked(address) {
        ui.painter().text(
            gutter.left_center() + egui::vec2(BOOKMARK_OFFSET, 0.0),
            egui::Align2::LEFT_CENTER,
            BOOKMARK,
            egui::FontId::monospace(11.0),
            listing.accent,
        );
    }
}

/// The note behind a dot, as the margin reports it: what it is, then what it
/// says.
fn note_text(address: u64, listing: &Listing) -> String {
    let mut lines = vec![text(listing.language, Text::RowHasNote).to_owned()];
    if let Some(label) = listing.notes.label(address) {
        lines.push(label.to_owned());
    }
    if let Some(comment) = listing.notes.comment(address) {
        lines.push(comment.to_owned());
    }
    lines.join("\n")
}

/// The empty cell each row keeps to the left for the jump arrows.
fn gutter_cell(ui: &mut egui::Ui) -> egui::Rect {
    let (rect, _response) =
        ui.allocate_exact_size(egui::vec2(GUTTER_WIDTH, ROW_HEIGHT), egui::Sense::hover());
    rect
}

/// What the hovered instruction's operand designates, in a few lines.
///
/// The same reading as the inspection window, shortened to what can be said
/// without the reader stopping to read: where it points, what is named there,
/// and either the instruction it lands on or the bytes that live there.
fn target_hint(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    instruction: &Instruction,
    listing: &Listing,
) {
    let language = listing.language;
    let Some(target) = operand::resolve(analysis, instruction, listing.file) else {
        return;
    };
    egui::Grid::new("disassembly_hint")
        .num_columns(2)
        .spacing([14.0, 4.0])
        .show(ui, |ui| {
            ui.strong(text(language, Text::Designates));
            ui.label(egui::RichText::new(format!("{:#018x}", target.address)).monospace());
            ui.end_row();

            if let Some(symbol) = &target.symbol {
                ui.strong(text(language, Text::TargetSymbol));
                ui.label(egui::RichText::new(symbol).monospace());
                ui.end_row();
            }
            if let Some(section) = &target.section {
                ui.strong(text(language, Text::TargetSection));
                ui.label(egui::RichText::new(section).monospace());
                ui.end_row();
            }
            // A jump lands on code, and the instruction there says more than
            // the bytes it is made of.
            if let Some(landing) = analysis.instruction_at(target.address) {
                ui.strong(text(language, Text::TargetInstruction));
                ui.label(syntax::assembly(
                    ui,
                    &landing.text,
                    egui::Color32::TRANSPARENT,
                ));
                ui.end_row();
            } else if let Some(reading) = &target.text {
                ui.strong(text(language, Text::TargetText));
                ui.label(egui::RichText::new(format!("{reading:?}")).monospace());
                ui.end_row();
            } else if !target.bytes.is_empty() {
                ui.strong(text(language, Text::TargetBytes));
                ui.label(syntax::dim(
                    ui,
                    &hex(&target.bytes[..target.bytes.len().min(8)]),
                    egui::Color32::TRANSPARENT,
                ));
                ui.end_row();
            }
        });
    ui.small(egui::RichText::new(text(language, Text::MoreWithRightClick)).color(MUTED));
}

/// The address a branch lands on, or `None` for anything that is not a jump to
/// a fixed address.
///
/// Calls are left out on purpose: they leave the listing and come back, and an
/// arrow for each would fill the gutter with journeys the reader is not
/// making. An indirect jump resolves to nothing and is drawn as nothing —
/// where `jmpq *%rax` goes is not known without running the program.
fn jump_target(instruction: &Instruction) -> Option<u64> {
    let mnemonic = instruction.text.split_whitespace().next()?;
    if !is_jump(mnemonic) {
        return None;
    }
    operand::target_address(instruction)
}

/// Whether a mnemonic branches within the listing: the x86 `j*` family, and
/// the ARM64 forms that jump without leaving a return address behind.
fn is_jump(mnemonic: &str) -> bool {
    mnemonic.starts_with('j')
        || mnemonic.starts_with("b.")
        || matches!(mnemonic, "b" | "cbz" | "cbnz" | "tbz" | "tbnz")
}

/// One jump on screen: where it leaves, and where it lands when that row is on
/// screen too.
struct Arrow {
    from: f32,
    to: Option<f32>,
    /// Which way it goes, for the jumps whose landing is off screen.
    downwards: bool,
    /// Touching the selected instruction, and so drawn plainly rather than
    /// faint: the jumps of the row the reader is standing on are the ones
    /// being followed.
    highlighted: bool,
}

impl Arrow {
    /// The stretch of listing it covers, top first.
    const fn span(&self) -> (f32, f32) {
        let Some(to) = self.to else {
            return (self.from, self.from);
        };
        if to < self.from {
            (to, self.from)
        } else {
            (self.from, to)
        }
    }
}

/// Every jump among the rows on screen, placed where those rows were drawn.
fn arrows(body: &[&Instruction], gutters: &[egui::Rect], selected: Option<u64>) -> Vec<Arrow> {
    body.iter()
        .copied()
        .zip(gutters)
        .filter_map(|(instruction, rect)| {
            let target = jump_target(instruction)?;
            let landing = body
                .iter()
                .copied()
                .zip(gutters)
                .find(|(candidate, _)| candidate.address == target)
                .map(|(_, rect)| rect.center().y);
            Some(Arrow {
                from: rect.center().y,
                to: landing,
                downwards: target > instruction.address,
                highlighted: selected == Some(instruction.address) || selected == Some(target),
            })
        })
        .collect()
}

/// Puts arrows drawn over the same rows in lanes of their own, the shortest
/// nearest the code, so a jump nested inside another is drawn inside it rather
/// than on top of it.
fn lanes(spans: &[(f32, f32)]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by(|left, right| {
        let (left, right) = (spans[*left], spans[*right]);
        (left.1 - left.0).total_cmp(&(right.1 - right.0))
    });
    let mut taken: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut lanes = vec![0; spans.len()];
    for index in order {
        let (top, bottom) = spans[index];
        let free = taken.iter().position(|lane| {
            lane.iter()
                .all(|(other_top, other_bottom)| bottom < *other_top || top > *other_bottom)
        });
        let lane = if let Some(lane) = free {
            lane
        } else {
            taken.push(Vec::new());
            taken.len() - 1
        };
        taken[lane].push((top, bottom));
        lanes[index] = lane;
    }
    lanes
}

/// Draws every visible jump as an arrow from the branch to the row it lands on.
///
/// A jump whose landing is not on screen is drawn to the edge of the view and
/// pointed there: it says which way to scroll, rather than claiming to reach
/// something the reader cannot see.
fn draw_jumps(ui: &egui::Ui, body: &[&Instruction], gutters: &[egui::Rect], selected: Option<u64>) {
    let Some(gutter) = gutters.first() else {
        return;
    };
    let arrows = arrows(body, gutters, selected);
    let spans: Vec<(f32, f32)> = arrows.iter().map(Arrow::span).collect();
    let lanes = lanes(&spans);
    let painter = ui.painter();
    let clip = ui.clip_rect();
    let right = gutter.right() - 2.0;
    for (arrow, lane) in arrows.iter().zip(lanes) {
        let lane = u8::try_from(lane.min(MAX_LANES - 1)).unwrap_or_default();
        let x = right - 4.0 - LANE_SPACING * f32::from(lane);
        let stroke = egui::Stroke::new(
            if arrow.highlighted { 1.6_f32 } else { 1.0_f32 },
            if arrow.highlighted {
                JUMP
            } else {
                JUMP.gamma_multiply(0.55)
            },
        );
        painter.line_segment(
            [egui::pos2(right, arrow.from), egui::pos2(x, arrow.from)],
            stroke,
        );
        if let Some(to) = arrow.to {
            painter.line_segment([egui::pos2(x, arrow.from), egui::pos2(x, to)], stroke);
            painter.line_segment([egui::pos2(x, to), egui::pos2(right, to)], stroke);
            head(painter, egui::pos2(right, to), Facing::Right, stroke);
        } else {
            // Off the top or the bottom of the view: the arrow reaches the
            // edge and points the way, rather than claiming to land anywhere.
            let edge = if arrow.downwards {
                clip.bottom() - 3.0
            } else {
                clip.top() + 3.0
            };
            painter.line_segment([egui::pos2(x, arrow.from), egui::pos2(x, edge)], stroke);
            let facing = if arrow.downwards {
                Facing::Down
            } else {
                Facing::Up
            };
            head(painter, egui::pos2(x, edge), facing, stroke);
        }
    }
}

/// Which way an arrowhead points.
#[derive(Clone, Copy)]
enum Facing {
    Right,
    Up,
    Down,
}

/// The two strokes that make an arrowhead at `tip`.
fn head(painter: &egui::Painter, tip: egui::Pos2, facing: Facing, stroke: egui::Stroke) {
    const SIZE: f32 = 3.5;
    let (back, side) = match facing {
        Facing::Right => (egui::vec2(-SIZE, 0.0), egui::vec2(0.0, SIZE)),
        Facing::Up => (egui::vec2(0.0, SIZE), egui::vec2(SIZE, 0.0)),
        Facing::Down => (egui::vec2(0.0, -SIZE), egui::vec2(SIZE, 0.0)),
    };
    painter.line_segment([tip + back + side, tip], stroke);
    painter.line_segment([tip + back - side, tip], stroke);
}

/// What the stack holds where the reader is standing.
///
/// Under the toolbar rather than beside the row: the listing shows the depth
/// at every instruction, and this says what those bytes are — which is the
/// question the number provokes.
fn stack_summary(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    stack: &Trace,
    selected: Option<u64>,
    language: Language,
) {
    let Some(address) = selected else {
        return;
    };
    let state = stack.state_at(analysis, address);

    // A plain row, not a wrapped one: a wrapped row laid every item of this
    // summary at the same point — the depth, the slots and their separators
    // drawn on top of one another as a single unreadable smudge.
    ui.horizontal(|ui| {
        ui.strong(text(language, Text::Stack))
            .on_hover_text(text(language, Text::StackHelp));
        match state.depth {
            Some(depth) => {
                ui.monospace(format!("{depth:#x}"));
            }
            None => {
                ui.label(egui::RichText::new("?").monospace().color(ERROR))
                    .on_hover_text(text(language, Text::StackUnknown));
            }
        }
        if state.slots.is_empty() {
            ui.label(
                egui::RichText::new(text(language, Text::StackEmpty))
                    .small()
                    .color(MUTED),
            );
        }
        for slot in &state.slots {
            ui.separator();
            ui.label(egui::RichText::new(slot_label(slot, language)).small());
        }
        if state.truncated {
            ui.label(
                egui::RichText::new(text(language, Text::StackFrameNotReached))
                    .small()
                    .color(MUTED),
            );
        }
    });
}

/// One stack slot, in the words the analysis can stand behind.
fn slot_label(slot: &StackSlot, language: Language) -> String {
    match slot {
        StackSlot::ReturnAddress => text(language, Text::StackReturnAddress).to_owned(),
        StackSlot::Saved(register) => format!("{register} {}", text(language, Text::StackSaved)),
        StackSlot::Pushed(what) => format!("{what} {}", text(language, Text::StackPushed)),
        StackSlot::Reserved(bytes) => format!("{bytes:#x} {}", text(language, Text::StackReserved)),
    }
}

/// The stack depth of one row, or a mark saying it is not known.
fn stack_cell(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    stack: &Trace,
    address: u64,
    language: Language,
) {
    let depth = analysis
        .instruction_index(address)
        .and_then(|index| stack.depth(index));
    match depth {
        Some(depth) => {
            ui.label(syntax::dim(
                ui,
                &format!("{depth:#x}"),
                egui::Color32::TRANSPARENT,
            ))
            .on_hover_text(text(language, Text::StackHelp));
        }
        None => {
            ui.label(egui::RichText::new("?").monospace().color(MUTED))
                .on_hover_text(text(language, Text::StackUnknown));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{JUMP, is_jump, jump_target, lanes, row_of, section_starts};
    use crate::{
        app::{Dialog, WorkspaceView},
        commands::Command,
        i18n::{Language, Text, text},
        testing::{drawn, drawn_text, opened_app, reference_analysis, window_input},
        ui::views,
    };
    use eframe::egui;

    /// One frame of the usual window carrying a single key press.
    fn press(key: egui::Key) -> egui::RawInput {
        let mut input = window_input();
        input.events = vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }];
        input
    }

    /// One frame of that window with the pointer held at a position, at a
    /// given moment.
    ///
    /// Sent again on every frame: a tooltip appears only while the pointer is
    /// still over the row, and only once it has rested there — hence the
    /// clock, which a test has to advance itself.
    fn hovering(position: egui::Pos2, time: f64) -> egui::RawInput {
        let mut input = window_input();
        input.events = vec![egui::Event::PointerMoved(position)];
        input.time = Some(time);
        input
    }

    /// Where a row's assembly was drawn, given the address beside it.
    fn assembly_position(
        shapes: &[egui::epaint::ClippedShape],
        address: u64,
        assembly: &str,
    ) -> Option<egui::Pos2> {
        let drawn = drawn(shapes);
        // The address is drawn twice — once in each listing — so the row is
        // the one where the assembly was drawn beside it.
        let rows: Vec<f32> = drawn
            .iter()
            .filter(|(text, _)| *text == format!("{address:#018x}"))
            .map(|(_, position)| position.y)
            .collect();
        drawn
            .iter()
            .find(|(text, position)| {
                text == assembly && rows.iter().any(|row| (position.y - row).abs() < 1.0)
            })
            .map(|(_, position)| *position + egui::vec2(3.0, 4.0))
    }

    /// The colour of every line a frame drew.
    fn line_colours(shapes: &[egui::epaint::ClippedShape]) -> Vec<egui::Color32> {
        fn walk(shape: &egui::Shape, out: &mut Vec<egui::Color32>) {
            match shape {
                egui::Shape::LineSegment { stroke, .. } => out.push(stroke.color),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    /// The first two decoded instructions, or nothing on a host whose own
    /// binary this build cannot decode.
    fn first_two() -> Option<(u64, u64)> {
        let instructions = &reference_analysis().instructions;
        Some((instructions.first()?.address, instructions.get(1)?.address))
    }

    /// Every dot of the note colour a frame drew.
    fn note_dots(shapes: &[egui::epaint::ClippedShape]) -> usize {
        fn walk(shape: &egui::Shape, out: &mut usize) {
            match shape {
                egui::Shape::Circle(circle) if circle.fill == super::NOTE => *out += 1,
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = 0;
        for clipped in shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    /// A note rides at the end of its line, which is off the right edge of a
    /// listing scrolled to read the bytes. The margin says one is there, and
    /// the margin never scrolls away.
    #[test]
    fn a_noted_row_is_marked_in_the_margin() {
        let Some((first, _)) = first_two() else {
            return;
        };
        let mut app = opened_app(WorkspaceView::Disassembly);
        let ctx = egui::Context::default();

        let output = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });
        assert_eq!(
            note_dots(&output.shapes),
            0,
            "nothing has been written about this binary yet"
        );

        app.annotations.set(
            first,
            crate::annotations::Annotation {
                comment: "ce que fait cette ligne".to_owned(),
                ..crate::annotations::Annotation::default()
            },
        );
        let output = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });
        assert_eq!(
            note_dots(&output.shapes),
            1,
            "the noted row, and only it, must carry a dot"
        );
    }

    /// A row marked to come back to is not a row written about: the star is
    /// its own mark, and a dot beside it would promise a note that is not
    /// there.
    #[test]
    fn a_bookmark_alone_draws_no_dot() {
        let Some((first, _)) = first_two() else {
            return;
        };
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.annotations.toggle_bookmark(first);
        let ctx = egui::Context::default();

        let output = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });

        assert_eq!(note_dots(&output.shapes), 0);
        assert!(
            drawn_text(&output.shapes).contains(super::BOOKMARK),
            "the bookmark itself must still be drawn"
        );
    }

    /// The listing is where a reader spends their time, and reaching the next
    /// instruction should not mean finding it with the mouse.
    #[test]
    fn the_arrow_keys_step_from_one_instruction_to_the_next() {
        let Some((first, second)) = first_two() else {
            return;
        };
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.selected_instruction = Some(first);
        let ctx = egui::Context::default();

        let _ = ctx.run(press(egui::Key::ArrowDown), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });
        assert_eq!(app.selected_instruction, Some(second));

        let _ = ctx.run(press(egui::Key::ArrowUp), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });
        assert_eq!(app.selected_instruction, Some(first));
    }

    /// The selection follows the keys, and the listing follows the selection:
    /// a row stepped onto but never scrolled to would be selected off screen.
    #[test]
    fn a_key_press_brings_the_row_it_lands_on_into_view() {
        let Some((first, second)) = first_two() else {
            return;
        };
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.selected_instruction = Some(first);
        let ctx = egui::Context::default();

        let output = ctx.run(press(egui::Key::ArrowDown), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });

        assert!(
            drawn_text(&output.shapes).contains(&format!("{second:#018x}")),
            "the row stepped onto must be drawn"
        );
    }

    /// The windows are drawn after the listing, and the command palette walks
    /// its own list with the same keys: a key taken here would never reach it.
    #[test]
    fn an_open_window_keeps_the_arrow_keys() {
        let Some((first, _)) = first_two() else {
            return;
        };
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.selected_instruction = Some(first);
        app.dialogs.open(Dialog::CommandPalette);
        let ctx = egui::Context::default();

        let _ = ctx.run(press(egui::Key::ArrowDown), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });

        assert_eq!(app.selected_instruction, Some(first));
    }

    /// Calls leave the listing and come back; a branch stays in it. Only the
    /// second is worth an arrow in the gutter.
    #[test]
    fn only_a_branch_counts_as_a_jump() {
        for mnemonic in ["jmp", "jmpq", "jne", "b", "b.eq", "cbz", "tbnz"] {
            assert!(is_jump(mnemonic), "{mnemonic} branches");
        }
        for mnemonic in ["callq", "bl", "blr", "br", "ret", "mov", "lea"] {
            assert!(
                !is_jump(mnemonic),
                "{mnemonic} is not a jump in the listing"
            );
        }
    }

    /// Two jumps drawn over the same rows must not be drawn over each other,
    /// and the shorter one belongs nearer the code it jumps within.
    #[test]
    fn overlapping_jumps_are_given_lanes_of_their_own() {
        let lanes = lanes(&[(0.0, 100.0), (20.0, 60.0), (200.0, 260.0)]);

        assert_ne!(lanes[0], lanes[1]);
        assert!(lanes[1] < lanes[0]);
        assert_eq!(
            lanes[2], 0,
            "a jump clear of the others goes back to the first lane"
        );
    }

    /// A jump is the one thing in a listing the reader follows rather than
    /// reads, so where it lands is drawn rather than left to be worked out.
    #[test]
    fn a_selected_jump_is_drawn_as_a_green_arrow() {
        let analysis = reference_analysis();
        let Some(jump) = analysis
            .instructions
            .iter()
            .find(|instruction| jump_target(instruction).is_some())
        else {
            return; // Nothing branching on this host: nothing to draw.
        };
        let address = jump.address;
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.selected_instruction = Some(address);

        let ctx = egui::Context::default();
        // The first frame is what the scroll area learns its content size
        // from; the offset it was given lands on the second.
        app.pending_instruction_scroll = Some(address);
        let _ = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });
        app.pending_instruction_scroll = Some(address);
        let output = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });

        assert!(
            line_colours(&output.shapes).contains(&JUMP),
            "the selected jump must be drawn in the gutter"
        );
    }

    /// An address in a listing provokes one question — what is there — and
    /// the answer is in the file the reader already has open.
    #[test]
    fn hovering_an_instruction_says_what_its_operand_designates() {
        let analysis = reference_analysis();
        let Some(instruction) = analysis
            .instructions
            .iter()
            .find(|instruction| desdec_core::operand::target_address(instruction).is_some())
        else {
            return; // Nothing on this host computes an address: nothing to say.
        };
        let address = instruction.address;
        let assembly = instruction.text.clone();

        let hinted = |hints: bool| {
            let mut app = opened_app(WorkspaceView::Disassembly);
            app.preferences.language = Language::English;
            app.preferences.show_operand_hints = hints;
            app.selected_instruction = Some(address);
            let ctx = egui::Context::default();
            // Two frames to put the row on screen: the scroll area learns its
            // content size from the first, and lands on the second.
            app.pending_instruction_scroll = Some(address);
            let _ = ctx.run(window_input(), |ctx| {
                views::show_central_panel(&mut app, ctx);
            });
            app.pending_instruction_scroll = Some(address);
            let placed = ctx.run(window_input(), |ctx| {
                views::show_central_panel(&mut app, ctx);
            });
            let position = assembly_position(&placed.shapes, address, &assembly)
                .expect("the row must have been drawn");

            // The pointer rests on the row while the clock runs on, which is
            // what a tooltip waits for.
            let mut drawn = String::new();
            for step in 1..=6 {
                let output = ctx.run(hovering(position, f64::from(step) * 0.25), |ctx| {
                    views::show_central_panel(&mut app, ctx);
                });
                drawn = drawn_text(&output.shapes);
            }
            drawn.contains(text(Language::English, Text::Designates))
        };

        assert!(hinted(true), "hovering must say where the operand points");
        assert!(
            !hinted(false),
            "and must say nothing once the preference is off"
        );
    }

    /// A heading takes a row of its own, so every instruction below it is one
    /// row further down — and the scroll offset is computed from that.
    #[test]
    fn a_section_heading_pushes_the_rows_below_it_down() {
        let sections = [0, 10, 25];

        assert_eq!(row_of(&sections, 1, 0), 2, "the column titles, then .text");
        assert_eq!(row_of(&sections, 1, 9), 11);
        assert_eq!(row_of(&sections, 1, 10), 13, "a second heading above it");
        assert_eq!(row_of(&sections, 1, 25), 29);
    }

    /// A listing of eighteen million rows is one column of hexadecimal unless
    /// it says where each section begins.
    #[test]
    fn the_listing_names_each_section_where_it_begins() {
        let analysis = reference_analysis();
        let starts = section_starts(analysis);
        let Some(first) = starts
            .first()
            .and_then(|start| analysis.instructions.get(*start))
        else {
            return; // Nothing decoded on this host.
        };
        let last = starts
            .get(1)
            .map_or(analysis.instructions.len(), |next| *next)
            - 1;
        let extent = format!(
            "{:#x} – {:#x}",
            first.address, analysis.instructions[last].address
        );

        let mut app = opened_app(WorkspaceView::Disassembly);
        let ctx = egui::Context::default();
        let output = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });

        let drawn = drawn_text(&output.shapes);
        assert!(
            drawn.contains(&extent),
            "the first section must be headed by its own extent"
        );
    }

    /// Stepping over a call lands after it and stepping into it follows it.
    /// That difference is the whole reason there are two buttons.
    #[test]
    fn stepping_over_a_call_skips_it_and_stepping_into_it_follows_it() {
        let analysis = reference_analysis();
        let Some((call, target, after)) = analysis.instructions.windows(2).find_map(|pair| {
            let mnemonic = pair[0].text.split_whitespace().next()?;
            if !mnemonic.starts_with("call") && mnemonic != "bl" {
                return None;
            }
            let target = desdec_core::operand::target_address(&pair[0])?;
            // Only a call into decoded code is one the walk can follow.
            analysis.instruction_index(target)?;
            Some((pair[0].address, target, pair[1].address))
        }) else {
            return; // Nothing on this host calls a fixed address.
        };

        let ctx = egui::Context::default();
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.selected_instruction = Some(call);
        app.run_command(&ctx, Command::WalkStepOver);
        assert_eq!(app.selected_instruction, Some(after));

        app.selected_instruction = Some(call);
        app.run_command(&ctx, Command::WalkStepInto);
        assert_eq!(app.selected_instruction, Some(target));

        // And back out of it, to where the call would have returned.
        app.run_command(&ctx, Command::WalkStepOut);
        assert_eq!(app.selected_instruction, Some(after));
    }

    /// A transport button that cannot move must be greyed out rather than
    /// swallow the press: an unresolved call is an answer, not a dead key.
    #[test]
    fn the_transport_offers_only_what_it_can_do() {
        let analysis = reference_analysis();
        let Some(first) = analysis.instructions.first().map(|i| i.address) else {
            return;
        };
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.selected_instruction = Some(first);

        assert!(
            !app.can_run(Command::WalkBack),
            "nothing has been walked yet"
        );
        assert!(!app.can_run(Command::WalkClear));
        assert!(
            !app.can_run(Command::WalkStepOut),
            "a selection reached by hand has no call to leave"
        );

        let ctx = egui::Context::default();
        app.run_command(&ctx, Command::WalkStepOver);
        assert!(app.can_run(Command::WalkBack), "one step can be undone");
        assert!(app.can_run(Command::WalkClear));
    }

    /// The stack summary used to draw its depth, its slots and their
    /// separators at one and the same point — one unreadable smudge above the
    /// listing.
    #[test]
    fn the_stack_summary_lays_its_parts_out_side_by_side() {
        let analysis = reference_analysis();
        let Some(first) = analysis.instructions.first().map(|i| i.address) else {
            return;
        };
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.preferences.language = Language::English;
        app.selected_instruction = Some(first);

        let ctx = egui::Context::default();
        let output = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });

        let drawn = drawn(&output.shapes);
        let Some(summary) = drawn
            .iter()
            .find(|(drawn, _)| drawn == text(Language::English, Text::Stack))
            .map(|(_, position)| *position)
        else {
            return; // The stack said nothing about this instruction.
        };
        let crowded = drawn
            .iter()
            .filter(|(_, position)| *position == summary)
            .count();
        assert_eq!(crowded, 1, "two labels were drawn at the same point");
    }

    /// What the reader wrote about a row belongs on that row: a note kept in
    /// a window they have to open is a note they will not read.
    #[test]
    fn the_listing_draws_the_note_on_the_row_it_belongs_to() {
        let analysis = reference_analysis();
        let Some(address) = analysis.instructions.first().map(|i| i.address) else {
            return;
        };
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.annotations.set(
            address,
            crate::annotations::Annotation {
                label: "parse_header".to_owned(),
                comment: "reads the magic".to_owned(),
                bookmarked: true,
            },
        );

        let ctx = egui::Context::default();
        let output = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });

        let drawn = drawn_text(&output.shapes);
        assert!(
            drawn.contains("parse_header:"),
            "the name stands on the row"
        );
        assert!(
            drawn.contains("; reads the magic"),
            "and so does the comment"
        );
        assert!(
            drawn.contains('\u{2605}'),
            "and the mark that was put on it"
        );
    }

    /// The listing is virtualised, so the row an instruction sits on is not
    /// laid out until it is scrolled to. Reaching one far down the listing has
    /// to actually put it on screen, or every cross-reference in the interface
    /// leads nowhere.
    #[test]
    fn scrolling_to_a_distant_instruction_brings_it_into_view() {
        let analysis = reference_analysis();
        let Some(target) = analysis.instructions.last() else {
            return; // Nothing decoded on this host: nothing to scroll to.
        };
        let address = target.address;
        let mut app = opened_app(WorkspaceView::Disassembly);
        app.selected_instruction = Some(address);
        app.pending_instruction_scroll = Some(address);

        let ctx = egui::Context::default();
        // The first frame is what the scroll area learns its content size
        // from; the offset it was given lands on the second.
        let _ = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });
        app.pending_instruction_scroll = Some(address);
        let output = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });

        // Twice: the disassembly and the pseudo-code beside it are two
        // virtualised listings, and both have to follow the same address.
        let drawn = drawn_text(&output.shapes);
        assert_eq!(
            drawn.matches(&format!("{address:#018x}")).count(),
            2,
            "both listings must scroll to the instruction"
        );
    }

    /// The virtualiser is told how tall a row is before drawing one, so a row
    /// that grew taller than [`crate::ui::ROW_HEIGHT`] would drift away from
    /// the position the offset was computed for — the further down the
    /// listing, the further off.
    #[test]
    fn rows_are_as_tall_as_the_virtualiser_was_told() {
        if reference_analysis().instructions.is_empty() {
            return;
        }
        let mut app = opened_app(WorkspaceView::Disassembly);
        let ctx = egui::Context::default();
        let output = ctx.run(window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });

        // Everything drawn in the first column of the listing, one item per
        // row: the addresses, and the name heading each section. Taken by
        // their shared left edge rather than by what they say, so a heading
        // row counts as the row it is instead of looking like a gap.
        let drawn = drawn(&output.shapes);
        let Some(column) = drawn
            .iter()
            .find(|(text, _)| text.starts_with("0x00"))
            .map(|(_, position)| position.x)
        else {
            return;
        };
        let mut rows: Vec<f32> = drawn
            .iter()
            .filter(|(_, position)| (position.x - column).abs() < 0.5)
            .map(|(_, position)| position.y)
            .collect();
        rows.sort_by(f32::total_cmp);
        rows.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        assert!(rows.len() > 5, "the listing must have drawn several rows");

        let spacing = ctx.style().spacing.item_spacing.y;
        let tallest = rows
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .fold(0.0_f32, f32::max);
        assert!(
            tallest <= crate::ui::ROW_HEIGHT + spacing + 0.5,
            "a row was {tallest} tall, more than the {} the virtualiser assumes",
            crate::ui::ROW_HEIGHT + spacing
        );
    }
}
