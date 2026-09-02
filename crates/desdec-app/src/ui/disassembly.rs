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
use desdec_core::{Analysis, Instruction, Nasm, StackSlot, Trace, operand};
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

/// A thin, clickable overview at the far right of the two code readings.
/// It is an overview ruler, not a miniature listing: hexadecimal at that size
/// says nothing, whereas position and the reader's marks still do.
const OVERVIEW_WIDTH: f32 = 16.0;

/// Search hits are warm, so they do not merge with a note, bookmark or the
/// Machine's green/red state.
const SEARCH_MARK: egui::Color32 = egui::Color32::from_rgb(241, 169, 75);

/// Where a section begins, on the overview ruler. Grey and thin on purpose:
/// this is the shape of the file, drawn under everything the reader put there
/// themselves, and a ruler whose landmarks shout is one where a bookmark is
/// lost among them.
const SECTION_MARK: egui::Color32 = egui::Color32::from_rgb(122, 132, 148);

/// Shortest the viewport rectangle is drawn, however little of the listing is
/// on screen.
///
/// Twenty-four rows out of a hundred and thirty thousand is a fifth of a pixel
/// — a mark the eye reads as a speck of dirt rather than as *where you are*.
/// The rectangle stops being to scale below this, and says so by still being a
/// rectangle: an exact answer nobody can see is worth less than a legible one.
const VIEWPORT_MINIMUM_HEIGHT: f32 = 10.0;

/// How many graduations the ruler is divided into.
///
/// It is called a ruler and had none: an empty tube with one rectangle in it,
/// which says *where you are* only if you already know how tall the tube is.
/// Ten steps make position readable at a glance — half way, a fifth of the way
/// — which is what a reader asks of it while scrolling.
///
/// Sections are drawn too, and are not enough on their own: in a real binary
/// `.init` and `.plt` are the first percent and `.text` is all the rest, so
/// every section mark lands in the top two pixels.
const RULER_STEPS: usize = 10;

/// How near a section mark a click has to land to be taken as a click *on* it.
///
/// The ruler maps a hundred and thirty thousand rows onto some six hundred
/// pixels, so one pixel is two hundred instructions and the start of a section
/// cannot be hit by aiming. Within this many pixels the click means the
/// landmark it is next to, which is the only reading that makes the marks
/// worth drawing.
const SECTION_SNAP: f32 = 4.0;

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
/// Rows the listing draws before its first instruction: the column headings.
///
/// Public to the interface because the pseudo-code beside the listing has to
/// stand on exactly the same rows; see [`ListingRow`].
pub(super) const LEADING: usize = 1;

/// How far the two columns of the view may stand apart before one is taken to
/// have been scrolled on its own.
///
/// Not zero: a scroll area reports its offset in pixels it has itself rounded,
/// and two areas told to stand at the same place can answer a fraction apart.
/// Treating that fraction as a scroll would have each column chasing the other
/// for ever, a pixel at a time.
const LINKED_SLACK: f32 = 0.5;

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
    /// What each row touches, through the types the reader said the registers
    /// hold; see [`crate::ui::types::MemberNames`].
    members: &'a crate::ui::types::MemberNames,
    /// Whether hovering a row says what its operand designates.
    hints: bool,
    /// How wide each column is held, in pixels, so the listing does not walk
    /// sideways as the reader scrolls; see [`Columns`].
    columns: [f32; 5],
    /// How wide an address is written here, `0x` included — see
    /// [`Columns::of`], which reads it off the file rather than fixing it.
    address_width: usize,
    /// The emulated run, when one has been started: where it stands now, and
    /// which rows the reader has put a breakpoint on.
    ///
    /// `None` until the reader asks for a run, so a listing read without ever
    /// opening the Machine view is drawn exactly as it was before.
    machine: Option<&'a desdec_core::emulate::Machine>,
    /// How the assembly column is spelled, when the reader asked for NASM
    /// rather than the AT&T the file was decoded in.
    ///
    /// `None` in the AT&T reading, and on ARM64, where there is one spelling
    /// and Capstone already wrote it. A cell because a formatter writes into
    /// itself as it works and every row is drawn from a shared borrow of the
    /// listing: one speller is built for the frame and lent to each visible
    /// row in turn, rather than sixty built and dropped per frame.
    nasm: Option<&'a std::cell::RefCell<Nasm>>,
    language: Language,
}

/// Draws the disassembly, returning what the reader asked of it.
pub fn show(app: &mut DesdecApp, ui: &mut egui::Ui) -> Action {
    // Before anything is laid out: the arrow keys move the selection, and the
    // row they land on has to be brought into view on this frame — the scroll
    // offset is decided as the listing is prepared, further down.
    step_with_arrow_keys(app, ui.ctx());

    let language = app.preferences.language;
    let mut action = Action::default();
    if !toolbar(app, ui, &mut action) {
        return action;
    }

    // Built before the listing borrows it, and only when it is asked for: a
    // reader in the AT&T spelling pays nothing at all for this.
    let nasm = app
        .preferences
        .nasm_syntax
        .then(|| {
            app.analysis
                .as_ref()
                .and_then(|analysis| Nasm::for_architecture(analysis.summary.architecture))
        })
        .flatten()
        .map(std::cell::RefCell::new);

    let listing = Listing {
        patches: &app.patches,
        stack: &app.stack,
        file: &app.file_bytes,
        sections: &app.section_starts,
        accent: accent(app.preferences.theme),
        notes: &app.annotations,
        members: &app.member_names,
        // The general switch still governs: a reader who turned the tooltips
        // off asked for a listing and nothing else.
        hints: app.preferences.show_tooltips && app.preferences.show_operand_hints,
        columns: app.listing_columns.pixels(ui, language, nasm.is_some()),
        address_width: app.listing_columns.address,
        machine: app.machine.as_ref(),
        nasm: nasm.as_ref(),
        language,
    };
    let Some(analysis) = &app.analysis else {
        return action;
    };
    let selected = app.selected_instruction;

    // A listing that stopped at the decoder's limit looks exactly like a
    // program that ends there, so it says which one this is.
    if analysis.code_truncated {
        ui.colored_label(ERROR, text(language, Text::TruncatedDisassembly));
    }
    status_line(ui, analysis, listing.stack, selected, language);
    ui.add_space(8.0);
    let scroll_target = app.pending_instruction_scroll;
    let attention = decompile::active_attention(ui.ctx(), &mut app.instruction_attention);
    // Preserve the last completed search rather than re-running it while this
    // view is drawn. The ruler only needs its decoded addresses.
    let search_addresses: Vec<u64> = app.search.result_addresses().collect();
    let overview_width =
        (ui.available_width() - OVERVIEW_WIDTH - ui.spacing().item_spacing.x).max(0.0);
    let overview_height = ui.available_height();
    let mut asked = Asked::default();
    let mut overview_jump = None;
    {
        let selected_instruction = &mut app.selected_instruction;
        let pending_scroll = &mut app.pending_instruction_scroll;
        let scrolled_from_pseudocode = &mut app.pseudocode_scroll;
        let mut visible_rows = 0..0;
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(overview_width, overview_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let result = two_readings(
                        ui,
                        analysis,
                        &listing,
                        &mut Linked {
                            selected_instruction,
                            pending_scroll,
                            scrolled_from_pseudocode,
                        },
                        scroll_target,
                        attention,
                    );
                    asked = result.asked;
                    visible_rows = result.visible_rows;
                },
            );
            overview_jump = overview(
                ui,
                analysis,
                &listing,
                &visible_rows,
                *selected_instruction,
                attention,
                &search_addresses,
                text(language, Text::DisassemblyOverview),
            );
        });
        if *pending_scroll == scroll_target {
            *pending_scroll = None;
        }
    }
    if let Some(address) = overview_jump {
        app.go_to_address(ui.ctx(), address);
    }
    action.inspect = asked.inspect;
    apply(app, &asked);
    action
}

/// Acts on what the rows asked for, once the borrows they were drawn under
/// have ended.
///
/// The notes, the machine and the dump are the application's, not the
/// listing's: a row cannot reach any of them while it is being drawn from a
/// borrow of them, which is why nothing here happens where it was asked for.
fn apply(app: &mut DesdecApp, asked: &Asked) {
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
    // Both of these build the machine if there is not one yet, which is the
    // only place in the listing that does: asking to stop somewhere is asking
    // for a run.
    if let Some(address) = asked.breakpoint
        && let Some(machine) = app.machine()
    {
        machine.toggle_breakpoint(address);
    }
    if let Some(address) = asked.run_to {
        if let Some(machine) = app.machine() {
            machine.run_to(address);
        }
        app.follow_the_run();
    }
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
    /// An address to put a breakpoint on, or to take one off.
    breakpoint: Option<u64>,
    /// An address to run the emulation up to.
    run_to: Option<u64>,
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
        self.breakpoint = self.breakpoint.or(row.breakpoint);
        self.run_to = self.run_to.or(row.run_to);
    }
}

/// What the reader asked of the disassembly this frame.
#[derive(Default)]
pub struct Action {
    /// An instruction whose bytes are to be edited.
    pub edit: Option<u64>,
    /// An instruction whose operand is to be explained.
    pub inspect: Option<u64>,
    /// Whether the reader asked for the current function to be handed to an
    /// assembler IDE. Returned rather than done here for the same reason
    /// everything else is: this view holds borrows of the application it would
    /// have to take by value.
    pub send_to_asm_studio: bool,
}

/// The one row above the listing: the walk's transport, and what is done with
/// the instruction it stopped on.
///
/// Answers whether there is a listing to draw under it. A file whose code was
/// never decoded still gets the transport — a reader looking at an empty view
/// should not also be wondering where the buttons went — and nothing else.
fn toolbar(app: &mut DesdecApp, ui: &mut egui::Ui, action: &mut Action) -> bool {
    let language = app.preferences.language;
    let Some(analysis) = app.analysis.as_ref() else {
        return false;
    };
    let empty = analysis.instructions.is_empty();
    transport(app, ui, action);
    if empty {
        ui.label(egui::RichText::new(text(language, Text::NoDisassembly)).color(MUTED));
    }
    !empty
}

/// What a reader does with the instruction they have selected.
///
/// Drawn into the transport's own row rather than on a line of its own. The
/// two used to be stacked, which put five lines of heading above the first
/// instruction of the listing — a transport, a row of buttons, a stack
/// summary, a sentence about the next step and a sentence about the keys —
/// and the listing itself started a third of the way down the window.
///
/// Both buttons are the same move — leaving Desdec's reading for somewhere the
/// code can be written — which is why they sit together, and why neither is
/// offered when nothing is selected.
fn selection_actions(
    ui: &mut egui::Ui,
    selected: Option<u64>,
    patchable: bool,
    language: Language,
    action: &mut Action,
) {
    let button = ui.add_enabled(
        selected.is_some() && patchable,
        egui::Button::new(text(language, Text::EditInstruction)),
    );
    if button.clicked() {
        action.edit = selected;
    }
    // Handing the function to an assembler IDE. Beside the byte editor
    // because both are the same move — leaving Desdec's reading for a
    // place the code can be written — and the reader looks for them
    // together.
    let send = ui.add_enabled(
        selected.is_some(),
        egui::Button::new(text(language, Text::SendToAsmStudio)),
    );
    if send
        .on_hover_text(text(language, Text::SendToAsmStudioHelp))
        .clicked()
    {
        action.send_to_asm_studio = true;
    }
    if selected.is_some() && !patchable {
        ui.label(egui::RichText::new(text(language, Text::NotPatchable)).color(MUTED));
    }
}

/// The controls of the static walk: a compact map of the code path.
///
/// Nothing here runs the program — Desdec never does — and the wording says
/// so: the walk *follows* the flow, one instruction at a time, through what
/// the bytes already say. Where a running program would consult a register or
/// a flag, the walk stops and reports that, and the two step buttons are what
/// let the reader choose the path a condition would otherwise decide.
fn transport(app: &mut DesdecApp, ui: &mut egui::Ui, action: &mut Action) {
    /// In the order a reader follows the code path, left to right.
    const BUTTONS: &[(Icon, Command)] = &[
        (Icon::WalkToEntry, Command::WalkToEntry),
        (Icon::WalkBack, Command::WalkBack),
        (Icon::WalkInto, Command::WalkStepInto),
        (Icon::WalkOver, Command::WalkStepOver),
        (Icon::WalkOut, Command::WalkStepOut),
        (Icon::WalkClear, Command::WalkClear),
    ];

    // Everything the row needs, read before a single widget is placed.
    //
    // The row asks two things of the application at once — whether each
    // command can run, and whether the selected instruction has bytes in the
    // file — and the second used to be answered by lifting the analysis out of
    // `DesdecApp` for the duration of the drawing. That answered it and broke
    // the first: `can_run` refuses every command that needs a binary, and with
    // the analysis lifted out there was none, so the whole transport was drawn
    // permanently greyed and no step, no restart and no jump to the entry
    // point did anything at all. Reading first and drawing after keeps both
    // questions answerable, and holds no borrow while the row is built.
    let language = app.preferences.language;
    let theme = accent(app.preferences.theme);
    let selected = app.selected_instruction;
    let walk_help = text(language, Text::WalkHelp);
    let steps = app.walk.steps();
    let depth = app.walk.depth();
    let offered: Vec<(Icon, Command, bool, Option<String>)> = BUTTONS
        .iter()
        .map(|(icon, command)| {
            (
                *icon,
                *command,
                app.can_run(*command),
                app.optional_command_tooltip(*command),
            )
        })
        .collect();
    // Whether the selected instruction's bytes are in the file, which is what
    // decides if it can be edited at all. `None` when nothing was decoded, and
    // the two buttons are then left off rather than drawn permanently greyed.
    let patchable = app.analysis.as_ref().and_then(|analysis| {
        (!analysis.instructions.is_empty()).then(|| {
            selected
                .is_some_and(|address| crate::patches::file_offset_of(analysis, address).is_some())
        })
    });
    let tooltips = app.preferences.show_tooltips;
    // Whether the spelling switch has anything to act on. NASM is an x86
    // question; ARM64 has one spelling and Capstone already wrote it, so the
    // choice is drawn but not offered rather than silently doing nothing.
    let respellable = app
        .analysis
        .as_ref()
        .is_some_and(|analysis| Nasm::spells(analysis.summary.architecture));
    let mut pressed = None;

    ui.horizontal(|ui| {
        // The row is given the buttons' height before anything is put in it.
        // egui centres each widget against the height the row has reached so
        // far, so the title written first was centred against its own height
        // and left sitting a few points above the buttons that came after it —
        // which is exactly what a toolbar must not look like.
        ui.set_min_height(icons::BUTTON_SIZE.y);
        // Small, muted and upright, like every other section heading in the
        // interface: it names the group of buttons beside it, and a heading
        // drawn at the weight of the content competes with it.
        let title = ui.label(crate::ui::section_title(
            &text(language, Text::StaticWalk).to_uppercase(),
        ));
        if tooltips {
            title.on_hover_text(walk_help);
        }
        for (icon, command, enabled, tooltip) in offered {
            // A button that cannot move is drawn but not offered: a press it
            // swallowed would read as a broken transport.
            let button = ui
                .add_enabled_ui(enabled, |ui| icons::button(ui, icon, tooltip, false, theme))
                .inner;
            if button.clicked() {
                pressed = Some(command);
            }
        }
        // What is done with the selected instruction, on the same row: it is
        // the same subject — the instruction the transport stopped on — and a
        // second row of its own said so less clearly and cost a line.
        if let Some(patchable) = patchable {
            ui.separator();
            selection_actions(ui, selected, patchable, language, action);
        }
        if steps > 0 || depth > 0 {
            ui.separator();
        }
        if steps > 0 {
            ui.small(format!("{} {steps}", text(language, Text::WalkSteps)));
        }
        if depth > 0 {
            ui.small(format!("{} {depth}", text(language, Text::WalkDepth)));
        }
        // How the listing is spelled, at the far end of the row. Not a move
        // the transport makes — it is how what the walk steps through is
        // written — and pinned to the right edge so it stays where the reader
        // left it as the step counters beside the buttons come and go.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let spelling = ui
                .add_enabled_ui(respellable, |ui| {
                    ui.checkbox(
                        &mut app.preferences.nasm_syntax,
                        text(language, Text::NasmSyntax),
                    )
                })
                .inner;
            if tooltips {
                spelling.on_hover_text(text(language, Text::NasmSyntaxHelp));
            }
        });
    });
    if let Some(command) = pressed {
        app.run_command(ui.ctx(), command);
    }
}

/// What the next step would do, said before it is taken.
///
/// The reader is following a flow, and the two places a static walk parts
/// company with a running program — a condition it cannot evaluate, a target
/// it cannot resolve — are exactly the places they need warning of.
fn next_step_sentence(analysis: &Analysis, address: u64, language: Language) -> String {
    let lead = text(language, Text::NextStep);
    match walk::next_move(analysis, address, false) {
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
    }
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

/// What the virtualised listing drew this frame, including the rows represented
/// by the overview ruler's viewport.
struct InstructionListing {
    asked: Asked,
    visible_rows: std::ops::Range<usize>,
    /// Where the listing ended up scrolled to, which is what the pseudo-code
    /// beside it is then held at. Read back from the scroll area rather than
    /// computed: the reader's wheel, a drag of the scrollbar and a jump to an
    /// address all land here, and only the scroll area knows the sum of them.
    offset: f32,
}

/// What the two locked columns are drawn from and write back to.
///
/// Gathered rather than passed one by one: all three belong to the application
/// and all three are written as the columns are drawn, which is what keeps
/// them out of [`Listing`] — that one is read and never written.
struct Linked<'a> {
    selected_instruction: &'a mut Option<u64>,
    pending_scroll: &'a mut Option<u64>,
    /// See [`crate::app::DesdecApp::pseudocode_scroll`].
    scrolled_from_pseudocode: &'a mut Option<f32>,
}

/// The two readings of the code, side by side and held on the same rows.
fn two_readings(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    listing: &Listing,
    state: &mut Linked,
    scroll_target: Option<u64>,
    attention: Option<u64>,
) -> InstructionListing {
    /// These are two distinct readings of the code, not adjacent fields of one
    /// table. A clear gutter makes it possible to follow a long instruction
    /// line without the pseudo-C beside it joining it visually.
    const PSEUDOCODE_GUTTER: f32 = 36.0;

    let language = listing.language;
    let mut drawn = InstructionListing {
        asked: Asked::default(),
        visible_rows: 0..0,
        offset: 0.0,
    };
    let ordinary_spacing = ui.spacing().item_spacing;
    let full_width = ui.available_width();
    let listing_width = listing_share(ui, listing.columns, full_width, PSEUDOCODE_GUTTER);
    let pseudocode_width = (full_width - PSEUDOCODE_GUTTER - listing_width).max(0.0);
    let top_left = ui.cursor().min;
    // Filled by the right-hand column below. An `Option` rather than a default
    // standing in for it: a panel that reported an offset of zero without
    // having been drawn would read as the reader having scrolled to the top.
    let mut pseudocode: Option<decompile::PanelOutput> = None;

    // Placed here rather than by `Ui::columns`, which halves the width
    // whatever the two sides hold. Half is the wrong answer twice over: the
    // listing's own scroll area then cut every instruction longer than it at
    // the middle of the window — `je 0x00000000000` and no more — while a
    // file of short instructions left that half empty and squeezed the
    // pseudo-code for nothing. See `listing_share`.
    // The columns as they will actually be drawn: narrowed where the listing
    // was given less than they asked for, so that what is on screen fits on
    // screen and no horizontal scrollbar is needed to read an instruction.
    let listing = &Listing {
        columns: fit_columns(ui, listing.columns, listing_width),
        ..*listing
    };
    let heights = [
        crate::ui::column(ui, top_left, listing_width, |ui| {
            ui.spacing_mut().item_spacing = ordinary_spacing;
            // The listing goes first, and the pseudo-code follows the offset
            // it ends up at. It is the one that answers a jump to an address
            // and the one the overview ruler reports, so it decides where the
            // pair stands; drawing it second would leave the panel beside it
            // holding last frame's position.
            drawn = instructions(
                ui,
                analysis,
                state.selected_instruction,
                scroll_target,
                state.pending_scroll,
                attention,
                listing,
                *state.scrolled_from_pseudocode,
            );
        }),
        crate::ui::column(
            ui,
            top_left + egui::vec2(listing_width + PSEUDOCODE_GUTTER, 0.0),
            pseudocode_width,
            |ui| {
                ui.spacing_mut().item_spacing = ordinary_spacing;
                // This is the side that gives way now, so its scrollbar is the
                // one that has to be seen: egui draws a floating bar two
                // pixels tall until the pointer is over it, and a panel whose
                // lines run past its edge with no visible way to follow them
                // reads as a panel that lost them.
                ui.spacing_mut().scroll.floating = false;
                // Clicking a pseudo-code line here only moves the selection:
                // the assembly it stands for is already in the left column, so
                // the address it reports has no window to open.
                pseudocode = Some(decompile::panel(
                    ui,
                    analysis,
                    state.selected_instruction,
                    scroll_target,
                    state.pending_scroll,
                    attention,
                    &decompile::PanelLayout {
                // The same rows the listing draws, headings included, so the
                // two columns say the same thing on the same line all the way
                // down.
                        sections: listing.sections,
                        // No address column: the listing beside it carries the
                        // same address on the same row.
                        with_addresses: false,
                        follow: Some(drawn.offset),
                        // Inside the panel, on row zero, opposite the
                        // listing's own column headings — never above it; see
                        // `PanelLayout::title`.
                        title: Some(decompile::PanelTitle {
                            label: text(language, Text::PseudoCode),
                            help: text(language, Text::PseudoCodeHelp),
                        }),
                    },
                ));
            },
        ),
    ];
    // The wheel over the panel scrolled it away from the listing. Held for the
    // next frame rather than acted on now — the listing has already been drawn
    // — which is one frame of the pair standing apart, and the price of
    // letting the reader scroll from either side.
    if let Some(pseudocode) = pseudocode {
        *state.scrolled_from_pseudocode = ((pseudocode.offset - drawn.offset).abs()
            > LINKED_SLACK)
            .then_some(pseudocode.offset);
    }

    // The row takes the width it was given and the height of its taller side,
    // never the width its contents asked for.
    ui.allocate_rect(
        egui::Rect::from_min_size(
            top_left,
            egui::vec2(full_width, heights[0].max(heights[1])),
        ),
        egui::Sense::hover(),
    );
    drawn
}

/// How much of the width the listing takes, the pseudo-code getting the rest.
///
/// **The listing is served first, and to the character.** It asks for what one
/// whole line of it needs and gets exactly that — no more, so a file of short
/// instructions leaves the rest to the pseudo-code, and no less, so an
/// instruction is never cut in half at the middle of the window. Halving the
/// width, which is what `Ui::columns` did, was wrong in both of those
/// directions at once.
///
/// The two sides are not alike, and that is why the listing wins. An
/// instruction is one line that cannot be wrapped, cannot be shortened and is
/// what the reader came for; scrolling sideways to finish reading `movsd
/// 0x1234(%rip),%xmm0` is a real cost, paid on every line. The pseudo-code
/// wraps nothing either, but a `goto` or an assignment that runs long is one
/// line among many, and its panel carries a horizontal scrollbar of its own —
/// so what it cannot show is a drag away rather than gone.
///
/// The listing is capped only where the pseudo-code would stop being a panel
/// at all: [`PSEUDOCODE_FLOOR`] characters, which is what its scrollbar needs
/// to be usable. Past that cap the listing scrolls too, which is the case
/// nothing can fix — the window is simply narrower than one instruction.
fn listing_share(ui: &egui::Ui, columns: [f32; 5], full: f32, gutter: f32) -> f32 {
    let spacing = ui.spacing().item_spacing.x;
    let wanted = listing_extent(columns, spacing);
    let most = (full - gutter - text_width(ui, PSEUDOCODE_FLOOR)).max(0.0);
    wanted.min(most)
}

/// What the pseudo-code keeps whatever the listing asks for.
///
/// Small, because this panel scrolls sideways: it needs enough width to be
/// recognised and to be dragged, not enough to hold its longest line.
/// Sixteen characters shows `stack_push(0x2` and the bar under it.
const PSEUDOCODE_FLOOR: usize = 16;

/// The width five columns take, gutter and spacing included.
fn listing_extent(columns: [f32; 5], spacing: f32) -> f32 {
    columns.iter().sum::<f32>() + GUTTER_WIDTH + spacing * 6.0
}

/// The five columns, narrowed until they fit the width the listing was given.
///
/// A listing that does not fit gets a horizontal scrollbar, and dragging to
/// finish reading an instruction is a cost paid on every line. So the columns
/// give way instead, in the order of what a reader can most afford to lose:
///
/// 1. **The bytes.** Widest of the four after the address, and the one the
///    file states most plainly elsewhere — the Dump view is nothing but bytes.
///    Down to four characters, which still shows the opcode.
/// 2. **The section**, which repeats on every row of a run and is named again
///    in the heading above that run.
/// 3. **The stack depth**, which is a reading rather than a fact.
///
/// The address and the instruction never give way: one is what the reader
/// navigates by, the other is what they came for. What a narrowed cell cannot
/// show is one hover away, never lost.
fn fit_columns(ui: &egui::Ui, mut columns: [f32; 5], available: f32) -> [f32; 5] {
    let spacing = ui.spacing().item_spacing.x;
    let mut over = listing_extent(columns, spacing) - available;
    if over <= 0.0 {
        return columns;
    }
    // Index into `columns`, and the least each may be narrowed to.
    for (column, floor) in [(1, 4), (2, 3), (3, 2)] {
        let floor = text_width(ui, floor);
        let given = (columns[column] - floor).max(0.0).min(over);
        columns[column] -= given;
        over -= given;
        if over <= 0.0 {
            break;
        }
    }
    columns
}

fn instructions(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    selected_instruction: &mut Option<u64>,
    scroll_target: Option<u64>,
    pending_scroll: &mut Option<u64>,
    attention: Option<u64>,
    listing: &Listing,
    follow: Option<f32>,
) -> InstructionListing {
    let language = listing.language;
    let mut asked = Asked::default();
    let mut visible_rows = 0..0;
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
    // Vertical only. The listing has no horizontal scrollbar because it never
    // needs one: `fit_columns` narrows the bytes, the section and the stack
    // until the row fits the width the view gave it, and the address and the
    // instruction — the one a reader navigates by and the one they came for —
    // are never touched. Dragging sideways to finish reading an instruction is
    // a cost paid on every line, and the answer is to make the line fit rather
    // than to offer a way to chase it.
    let area = decompile::listing_area_at_row(
        egui::ScrollArea::vertical().id_salt("instructions"),
        ui,
        target_row,
    );
    // A scroll the reader made in the pseudo-code panel, carried here. Only
    // when nothing is being jumped to: a reader who asked for an address is
    // asking for it more loudly than the panel beside them is asking to stay
    // where it was.
    let area = match follow.filter(|_| target_row.is_none()) {
        Some(offset) => area.vertical_scroll_offset(offset),
        None => area,
    };
    let total_rows = LEADING + analysis.instructions.len() + listing.sections.len();
    let scrolled =
        area.auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, total_rows, |ui, rows| {
                visible_rows = rows.clone();
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
                            let [address, bytes, section, stack, instruction] = listing.columns;
                            for (title, width) in [
                                (Text::Address, address),
                                (Text::Bytes, bytes),
                                (Text::Section, section),
                                (Text::Stack, stack),
                                (Text::Instruction, instruction),
                            ] {
                                sized_cell(ui, width, |ui| ui.strong(text(language, title)));
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
    InstructionListing {
        asked,
        visible_rows,
        offset: scrolled.state.offset.y,
    }
}

/// Draws the thin overview ruler at the right edge of the disassembly.
///
/// It carries only positions and deliberately uses the listing's row map: a
/// section heading is a row in the virtualiser, so omitting it here would make
/// a click drift farther from its target with every section passed.
#[allow(clippy::too_many_arguments)]
fn overview(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    listing: &Listing,
    visible_rows: &std::ops::Range<usize>,
    selected: Option<u64>,
    attention: Option<u64>,
    search_addresses: &[u64],
    tooltip: &str,
) -> Option<u64> {
    let total_rows = LEADING + analysis.instructions.len() + listing.sections.len();
    let height = ui.available_height().max(ROW_HEIGHT * 8.0);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(OVERVIEW_WIDTH, height),
        egui::Sense::click_and_drag(),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, ui.visuals().faint_bg_color);
    painter.rect_stroke(
        rect,
        3.0,
        ui.visuals().window_stroke,
        egui::StrokeKind::Inside,
    );

    let row_of_address = |address| {
        analysis
            .instruction_index(address)
            .map(|index| row_of(listing.sections, LEADING, index))
    };

    // The shape of the file, underneath everything: the scale, and where each
    // section begins. Drawn before the viewport so the reader's own marks, and
    // the viewport itself, sit over them.
    let section_ys = structure(painter, rect, analysis, listing, total_rows);

    // The rectangle is the reader's viewport; it is deliberately translucent
    // so marks within it remain visible.
    if !visible_rows.is_empty() {
        let start = overview_y(rect, visible_rows.start, total_rows);
        let end = overview_y(rect, visible_rows.end.saturating_sub(1), total_rows);
        // Grown to a legible height when the listing is long, and grown
        // *upwards* once it would otherwise run past the bottom — a viewport
        // pinned to the last rows has to stay inside the ruler, or the reader
        // at the end of a file sees it hang off the edge.
        // `top` is settled — pushed up off the bottom edge, then held at the
        // top one — *before* `bottom` is measured from it. Clamping it
        // afterwards silently ate a pixel of the height at row zero, which is
        // exactly where a reader looks first.
        let top = (start - 1.0)
            .min(rect.bottom() - VIEWPORT_MINIMUM_HEIGHT)
            .max(rect.top());
        let bottom = (end + 1.0)
            .max(top + VIEWPORT_MINIMUM_HEIGHT)
            .min(rect.bottom());
        let viewport =
            egui::Rect::from_x_y_ranges((rect.left() + 1.0)..=(rect.right() - 1.0), top..=bottom);
        painter.rect_filled(
            viewport,
            1.0,
            ui.visuals().selection.bg_fill.gamma_multiply(0.35),
        );
        painter.rect_stroke(
            viewport,
            1.0,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }

    reader_marks(
        painter,
        rect,
        listing,
        total_rows,
        &row_of_address,
        search_addresses,
        selected,
        attention,
        ui.visuals().selection.bg_fill,
    );

    let response = response.on_hover_text(hover_text(
        ui,
        rect,
        analysis,
        listing,
        total_rows,
        &section_ys,
        tooltip,
    ));

    response
        .interact_pointer_pos()
        .filter(|_| response.clicked() || response.dragged())
        .and_then(|pointer| {
            // A click that lands on a section mark means that section, not the
            // two-hundredth instruction inside it. Only for a click: dragging
            // is a reader sweeping the file, and a drag that kept catching on
            // landmarks would stutter.
            let row = if response.dragged() {
                overview_row(rect, pointer.y, total_rows)
            } else {
                snapped_row(rect, pointer.y, total_rows, &section_ys)
            };
            instruction_at_overview_row(analysis, listing.sections, row)
                .map(|instruction| instruction.address)
        })
}

/// Draws everything the reader put on the listing themselves: their notes and
/// bookmarks, the hits of their search, the breakpoints and where a run
/// stands, and the row they have selected.
///
/// Over the ruler's own structure, and thicker than it: the file's shape is
/// background, and what the reader is looking for is not.
#[expect(
    clippy::too_many_arguments,
    reason = "one argument per family of mark, and each is drawn from a different place"
)]
fn reader_marks(
    painter: &egui::Painter,
    rect: egui::Rect,
    listing: &Listing<'_>,
    total_rows: usize,
    row_of_address: &impl Fn(u64) -> Option<usize>,
    search_addresses: &[u64],
    selected: Option<u64>,
    attention: Option<u64>,
    selection: egui::Color32,
) {
    // A note and a bookmark share one row; the bookmark is the stronger mark.
    for (address, annotation) in listing.notes.iter() {
        if let Some(row) = row_of_address(address) {
            let color = if annotation.bookmarked {
                listing.accent
            } else {
                NOTE
            };
            overview_mark(painter, rect, row, total_rows, color, 2.0);
        }
    }
    for address in search_addresses {
        if let Some(row) = row_of_address(*address) {
            overview_mark(painter, rect, row, total_rows, SEARCH_MARK, 1.5);
        }
    }
    if let Some(machine) = listing.machine {
        for (address, _) in machine.breakpoints() {
            if let Some(row) = row_of_address(address) {
                overview_mark(
                    painter,
                    rect,
                    row,
                    total_rows,
                    crate::ui::machine::BREAKPOINT,
                    2.5,
                );
            }
        }
        if let Some(row) = row_of_address(machine.instruction_pointer()) {
            overview_mark(
                painter,
                rect,
                row,
                total_rows,
                crate::ui::machine::CURRENT,
                3.0,
            );
        }
    }
    if let Some(row) = attention.and_then(row_of_address) {
        overview_mark(painter, rect, row, total_rows, SEARCH_MARK, 3.0);
    }
    if let Some(row) = selected.and_then(row_of_address) {
        overview_mark(painter, rect, row, total_rows, selection, 3.0);
    }

    // Hovering says which section is under the pointer, because a grey tick
    // that names nothing is a decoration. The ruler's own explanation comes
    // first; the landmark is what changes as the pointer moves.
}

/// What hovering the ruler says: its own explanation, then the address the
/// pointer is over, then the section when the pointer is on one of its marks.
///
/// A ruler measures position, so the address is the answer to *where is this*;
/// and a grey tick that names nothing is a decoration.
fn hover_text(
    ui: &egui::Ui,
    rect: egui::Rect,
    analysis: &Analysis,
    listing: &Listing<'_>,
    total_rows: usize,
    section_ys: &[(f32, &str)],
    tooltip: &str,
) -> String {
    use std::fmt::Write as _;

    let mut hint = tooltip.to_owned();
    let Some(pointer) = ui
        .input(|input| input.pointer.hover_pos())
        .filter(|pointer| rect.contains(*pointer))
    else {
        return hint;
    };
    let row = overview_row(rect, pointer.y, total_rows);
    if let Some(instruction) = instruction_at_overview_row(analysis, listing.sections, row) {
        let _ = write!(hint, "\n{:#018x}", instruction.address);
    }
    if let Some(section) = section_near(section_ys, pointer.y) {
        let _ = write!(hint, "\n{section}");
    }
    hint
}

/// Draws the ruler's scale and its section marks, and answers where the
/// section marks landed.
///
/// Two families, told apart by their width: a graduation is a short tick from
/// the left edge, and a section crosses the whole ruler. Neither may be
/// mistaken for one of the reader's own marks, which also cross it but are
/// thicker and coloured.
///
/// Both are needed. Sections alone leave the ruler blank: in a real binary
/// `.init` and `.plt` are the first percent and `.text` is all the rest, so
/// every section mark lands in the top two pixels. Graduations alone say how
/// far down you are and nothing about what you are in the middle of.
fn structure<'a>(
    painter: &egui::Painter,
    rect: egui::Rect,
    analysis: &'a Analysis,
    listing: &Listing<'_>,
    total_rows: usize,
) -> Vec<(f32, &'a str)> {
    #[expect(
        clippy::cast_precision_loss,
        reason = "ten steps over a ruler's height, in pixels"
    )]
    for step in 1..RULER_STEPS {
        let y = rect.top() + rect.height() * (step as f32 / RULER_STEPS as f32);
        painter.line_segment(
            [
                egui::pos2(rect.left() + 2.0, y),
                egui::pos2(rect.left() + 6.0, y),
            ],
            egui::Stroke::new(1.0_f32, SECTION_MARK.gamma_multiply(0.55)),
        );
    }

    let section_ys: Vec<(f32, &str)> = listing
        .sections
        .iter()
        .filter_map(|index| {
            let instruction = analysis.instructions.get(*index)?;
            let row = row_of(listing.sections, LEADING, *index);
            Some((
                overview_y(rect, row, total_rows),
                instruction.section.as_ref(),
            ))
        })
        .collect();
    for (y, _) in &section_ys {
        painter.line_segment(
            [
                egui::pos2(rect.left() + 2.0, *y),
                egui::pos2(rect.right() - 2.0, *y),
            ],
            egui::Stroke::new(1.0_f32, SECTION_MARK),
        );
    }
    section_ys
}

/// The section whose mark is nearest the pointer, when one is near enough to
/// be what the pointer is on.
fn section_near<'a>(sections: &[(f32, &'a str)], y: f32) -> Option<&'a str> {
    sections
        .iter()
        .filter(|(mark, _)| (mark - y).abs() <= SECTION_SNAP)
        .min_by(|(a, _), (b, _)| {
            (a - y)
                .abs()
                .partial_cmp(&(b - y).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(_, name)| *name)
}

/// The row a click means: the one under the pointer, unless a section mark is
/// close enough that the click was aimed at it.
fn snapped_row(rect: egui::Rect, y: f32, total_rows: usize, sections: &[(f32, &str)]) -> usize {
    let nearest = sections
        .iter()
        .map(|(mark, _)| *mark)
        .filter(|mark| (mark - y).abs() <= SECTION_SNAP)
        .min_by(|a, b| {
            (a - y)
                .abs()
                .partial_cmp(&(b - y).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    overview_row(rect, nearest.unwrap_or(y), total_rows)
}

/// The first instruction at or below an overview row. It is found through the
/// same section-aware row map that laid out the virtualised listing, rather
/// than by subtracting a fixed number of headings.
fn instruction_at_overview_row<'a>(
    analysis: &'a Analysis,
    sections: &[usize],
    row: usize,
) -> Option<&'a Instruction> {
    let mut low = 0;
    let mut high = analysis.instructions.len();
    while low < high {
        let middle = low + (high - low) / 2;
        if row_of(sections, LEADING, middle) < row {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    analysis
        .instructions
        .get(low.min(analysis.instructions.len().saturating_sub(1)))
}

/// The ruler is a continuous position control, clamped so a click near either
/// end always selects a decoded instruction rather than a row beyond it.
fn overview_row(rect: egui::Rect, y: f32, total_rows: usize) -> usize {
    if total_rows <= 1 || rect.height() <= 0.0 {
        return 0;
    }
    let fraction = ((y - rect.top()) / rect.height()).clamp(0.0, 1.0);
    (fraction * (total_rows.saturating_sub(1)) as f32).round() as usize
}

fn overview_y(rect: egui::Rect, row: usize, total_rows: usize) -> f32 {
    let last = total_rows.saturating_sub(1).max(1);
    rect.top() + rect.height() * (row.min(last) as f32 / last as f32)
}

fn overview_mark(
    painter: &egui::Painter,
    rect: egui::Rect,
    row: usize,
    total_rows: usize,
    color: egui::Color32,
    width: f32,
) {
    let y = overview_y(rect, row, total_rows);
    painter.line_segment(
        [
            egui::pos2(rect.left() + 2.0, y),
            egui::pos2(rect.right() - 2.0, y),
        ],
        egui::Stroke::new(width, color),
    );
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

/// How wide each column of the listing is held, in characters.
///
/// A virtualised grid lays out only the rows on screen, so egui sizes each
/// column to whatever those rows happen to hold. Scrolling from a stretch of
/// two-byte instructions into a stretch of ten-byte ones therefore shifted
/// every column to the right of the bytes — sixty-three pixels, measured — and
/// the listing walked sideways under the reader's eye as they scrolled.
///
/// The answer is to hold each column to the widest thing the *whole* binary
/// could put in it, not the widest thing currently visible. Counted in
/// characters here and turned into pixels where the font is known: the count
/// depends on the file and is worked out once, the pixels depend on the theme
/// and are worked out every frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Columns {
    pub address: usize,
    pub bytes: usize,
    pub section: usize,
    pub stack: usize,
    /// The widest line of assembly the file holds, in each spelling.
    ///
    /// Two numbers, because the listing has two spellings and they are not the
    /// same length: `jmpq *0x2f8e(%rip)` is eighteen characters where NASM
    /// writes `jmp qword [rel 0x3ff8]` in twenty-two. Measuring only the one
    /// the decoder produced left the column too narrow the moment the reader
    /// ticked NASM, and the instruction ran off the end of the listing.
    ///
    /// Both count the decoder's own text and nothing else: the reader's label
    /// and comment ride past the end of the line, and a column sized to hold
    /// them would be mostly empty on every row that carries neither.
    pub instruction: usize,
    pub instruction_nasm: usize,
}

impl Columns {
    /// The widths in pixels, in this frame's font and the reader's language.
    ///
    /// The greater of what the data needs and what the heading needs. The
    /// heading is set in the proportional bold of the theme and the data in a
    /// monospace, so neither can be worked out from the other — and a column
    /// whose heading was wider than its data grew by exactly that much on the
    /// one screenful where the heading is visible, which put it a pixel to the
    /// right of where every other screenful had it.
    fn pixels(self, ui: &egui::Ui, language: Language, nasm: bool) -> [f32; 5] {
        let heading = |title: Text| {
            let text = text(language, title).to_owned();
            let font = egui::TextStyle::Body.resolve(ui.style());
            ui.fonts(|fonts| {
                fonts
                    .layout_no_wrap(text, font, egui::Color32::WHITE)
                    .size()
                    .x
            })
        };
        [
            (self.address, Text::Address),
            (self.bytes, Text::Bytes),
            (self.section, Text::Section),
            (self.stack, Text::Stack),
            (
                if nasm {
                    self.instruction_nasm
                } else {
                    self.instruction
                },
                Text::Instruction,
            ),
        ]
        .map(|(characters, title)| text_width(ui, characters).max(heading(title)))
    }

    /// The widths one binary calls for.
    ///
    /// Walks the listing once, like the section index beside it, and for the
    /// same reason: a large shared library holds eighteen million instructions
    /// and this is not frame work.
    #[must_use]
    pub fn of(analysis: &Analysis, stack: &Trace) -> Self {
        // As many digits as the file's highest address needs, and not one
        // more. Every address used to be written `{:#018x}` — sixteen digits,
        // whatever the file — so a program loaded at `0x400000` spent eight
        // characters of every row on zeros nobody reads, which is a third of
        // the listing's width given away before the instruction gets any.
        //
        // Still a fixed width, and still in pairs: the column stays aligned
        // down the whole file, which is what lets an address be read at a
        // glance, and a hexadecimal number is read in bytes. Eight digits at
        // the least, which is what a reader expects to see even where fewer
        // would do.
        let highest = analysis
            .instructions
            .last()
            .map_or(0, |instruction| instruction.address);
        let digits = (16 - usize::try_from((highest | 1).leading_zeros() / 4).unwrap_or(0)).max(8);
        let address = digits + digits % 2 + 2;
        let mut bytes = 1;
        let mut section = 1;
        let mut instruction_text = 1;
        for instruction in &analysis.instructions {
            bytes = bytes.max(instruction.bytes.as_slice().len());
            section = section.max(instruction.section.chars().count());
            instruction_text = instruction_text.max(instruction.text.chars().count());
        }

        // The same measurement in the other spelling, which costs decoding
        // each instruction a second time — and is bounded, because a large
        // shared library holds eighteen million of them and this is a column
        // width, not an analysis. Past the bound the widest line seen so far
        // stands, and the listing's own scrollbar carries whatever a later
        // line needs: a rare drag, against a second full decode of the file.
        const MEASURED_IN_NASM: usize = 200_000;
        let mut instruction_nasm = instruction_text;
        if let Some(mut speller) = Nasm::for_architecture(analysis.summary.architecture) {
            for instruction in analysis.instructions.iter().take(MEASURED_IN_NASM) {
                if let Some(spelled) = speller.spell(instruction) {
                    instruction_nasm = instruction_nasm.max(spelled.chars().count());
                }
            }
        }
        let mut stack_width = 1;
        for index in 0..analysis.instructions.len() {
            if let Some(depth) = stack.depth(index) {
                stack_width = stack_width.max(format!("{depth:#x}").len());
            }
        }
        Self {
            address,
            // Two hexadecimal digits each, a space between them, and the two
            // the patch marker takes — reserved whether or not anything is
            // patched, so writing a byte does not move the columns either.
            bytes: bytes * 3 - 1 + 2,
            section,
            stack: stack_width,
            instruction: instruction_text,
            instruction_nasm,
        }
    }
}

/// The width of `characters` monospace digits, in this frame's font.
fn text_width(ui: &egui::Ui, characters: usize) -> f32 {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    // Every digit of a monospace font is one advance wide, and so is every
    // other glyph in it; measuring one and multiplying is exact here and
    // costs nothing per row.
    let advance = ui.fonts(|fonts| fonts.glyph_width(&font, '0'));
    // A column is never more characters than a screen has pixels; the cast is
    // exact for every width this can be asked for.
    advance * f32::from(u16::try_from(characters).unwrap_or(u16::MAX))
}

/// An address as the listing writes it.
///
/// One function rather than a `format!` at each call site, because the width
/// is no longer a constant: it comes from the file's highest address, and a
/// test that spelled `{:#018x}` by hand was testing a format the view had
/// stopped using. Anything that needs to find an address on screen asks here.
#[must_use]
pub fn address_text(width: usize, address: u64) -> String {
    format!("{address:#0width$x}")
}

/// Draws a cell held to a column's width.
///
/// The width is a floor rather than a ceiling: it is computed from the widest
/// content the file can produce, so nothing ever exceeds it, and a heading
/// wider than the data still gets the room it needs.
pub fn sized_cell(
    ui: &mut egui::Ui,
    width: f32,
    contents: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> egui::Response {
    ui.scope(|ui| {
        ui.set_min_width(width);
        // And no wider. The width is a floor for the content the file can
        // produce, but on a narrow window the columns are narrowed below what
        // they asked for — see `fit_columns` — and a cell that then took the
        // room its text wanted would push the instruction off the end of the
        // listing, which is the whole thing this arrangement exists to keep on
        // screen.
        ui.set_max_width(width);
        contents(ui)
    })
    .inner
}

/// The width of `characters` monospace glyphs, for a caller holding a cell to
/// a width the listing does not decide — the assembly window's address column.
#[must_use]
pub fn monospace_width(ui: &egui::Ui, characters: usize) -> f32 {
    text_width(ui, characters)
}

/// The row an instruction is drawn on, counting the headings above it.
pub(super) fn row_of(sections: &[usize], leading: usize, index: usize) -> usize {
    leading + index + sections.partition_point(|start| *start <= index)
}

/// What one row of the listing carries.
///
/// The pseudo-code panel beside the listing reads this too, so that both draw
/// a row in the same places: a heading the listing draws and the pseudo-code
/// does not is a row of drift, and every section adds another. Two walks that
/// have to agree exactly are one walk.
pub(super) enum ListingRow<'a> {
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
pub(super) fn window<'a>(
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
    // The name is painted rather than laid out, and the cell under it is
    // allocated to the address column's width and no more. Laid out it was a
    // cell like any other, and a section name is not held to any column's
    // width: `__TEXT,__text` is wider than a Mach-O address, so the address
    // column grew by the difference on every screenful a section began on and
    // shrank again on the next one — the sideways walk this whole arrangement
    // exists to stop, seen on macOS and not here, where sections are called
    // `.text`. The three cells beside this one are empty, so a long name has
    // the room to run into and nothing is lost by not measuring it.
    let name = ui
        .allocate_exact_size(
            egui::vec2(listing.columns[0], ROW_HEIGHT),
            egui::Sense::hover(),
        )
        .0;
    ui.painter().text(
        name.left_center(),
        egui::Align2::LEFT_CENTER,
        &*first.section,
        egui::TextStyle::Monospace.resolve(ui.style()),
        listing.accent,
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
    // Room kept in the paint list for the band behind the selected row, taken
    // before anything of the row is drawn so that what follows lands on top of
    // it. Painted afterwards would put a solid rectangle over the very text it
    // is meant to pick out.
    let band = ui.painter().add(egui::Shape::Noop);
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
    // The run's two marks ride on the address itself, in the way a debugger
    // has always drawn them: red for a row the run will stop on, green for the
    // row it is standing on. They win over the selection's own fill, because
    // where the processor is matters more than where the pointer last clicked.
    let running = run_marks(listing, instruction.address);
    let address_fill = running.map_or(selected_fill, |(colour, _)| colour);
    let address = sized_cell(ui, listing.columns[0], |ui| {
        ui.add(
            egui::Label::new(syntax::dim(
                ui,
                &address_text(listing.address_width, instruction.address),
                address_fill,
            ))
            .sense(egui::Sense::click()),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
    });
    let address = match running {
        Some((_, what)) => address.on_hover_text(text(language, what)),
        None => address,
    };
    // A patched row shows the bytes that would be written, marked, rather than
    // the ones still in the file: the listing must describe the binary being
    // built.
    let bytes = hex(patch.map_or(&instruction.bytes, |patch| &patch.replacement));
    sized_cell(ui, listing.columns[1], |ui| match patch {
        Some(patch) => ui
            .label(
                egui::RichText::new(format!("{bytes} *"))
                    .monospace()
                    .color(PATCHED),
            )
            .on_hover_text(format!(
                "{} {}",
                text(language, Text::OriginalBytes),
                hex(&patch.original)
            )),
        // Truncated rather than allowed to overflow: `fit_columns` narrows
        // this column first on a window too small for everything, and the
        // whole of it is one hover away.
        None => ui
            .add(egui::Label::new(syntax::dim(ui, &bytes, egui::Color32::TRANSPARENT)).truncate())
            .on_hover_text(&bytes),
    });
    sized_cell(ui, listing.columns[2], |ui| {
        ui.add(
            egui::Label::new(syntax::dim(
                ui,
                &instruction.section,
                egui::Color32::TRANSPARENT,
            ))
            .truncate(),
        )
        .on_hover_text(&*instruction.section)
    });
    // The stack as it stands *before* this instruction runs, which is what a
    // reader stopped on it would see. Looked up by address rather than by row:
    // the listing is virtualised, and an index computed from a visible range
    // drifts the moment either changes.
    sized_cell(ui, listing.columns[3], |ui| {
        stack_cell(ui, analysis, listing.stack, instruction.address, language)
    });
    // What this row reads as assembly. When NASM was asked for, it is this
    // instruction's own bytes decoded again — never the AT&T text with its
    // words moved around — and the AT&T stands wherever those bytes decode
    // back to nothing, which is the listing saying so about them.
    let respelled = listing
        .nasm
        .and_then(|nasm| nasm.borrow_mut().spell(instruction));
    let spelling = respelled.as_deref().unwrap_or(&instruction.text);
    // The reader's own name and comment ride at the end of the line, where an
    // assembler puts a comment — a column of their own sat off the right edge
    // of the listing, where nobody would ever scroll to find them.
    let assembly = ui
        .add(
            egui::Label::new(syntax::annotated(
                ui,
                spelling,
                listing.members.get(instruction.address),
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
    // designates, what the reader wants to say about it, and where a run
    // should stop.
    assembly.context_menu(|ui| row_menu(ui, instruction, language, &mut asked));
    if address.clicked() || assembly.clicked() {
        *selected_instruction = Some(instruction.address);
        *pending_scroll = Some(instruction.address);
        ui.ctx().request_repaint();
    }
    // The whole row, and not only the text on it. The fill used to reach no
    // further than the glyphs of each cell, so a selected line was four short
    // patches of blue with the gaps between the columns showing through — at a
    // glance, less visible than the table's own striping. A band across the
    // row is what a debugger draws, and it is legible from across the screen.
    if selected_fill != egui::Color32::TRANSPARENT {
        ui.painter().set(
            band,
            selection_band(gutter, assembly.rect, ui.max_rect(), selected_fill, listing),
        );
    }
    ui.end_row();
    (gutter, asked)
}

/// The band behind a selected row: a fill, and the accent drawn round it.
///
/// The outline is what makes the selection read as *this row* rather than as a
/// patch of colour. It matters most where the fill has least to work with — a
/// light theme, or a row whose cells are nearly empty — and it costs one
/// stroke.
fn selection_band(
    gutter: egui::Rect,
    last_cell: egui::Rect,
    available: egui::Rect,
    fill: egui::Color32,
    listing: &Listing,
) -> egui::Shape {
    let band = egui::Rect::from_min_max(
        egui::pos2(gutter.left(), gutter.top()),
        egui::pos2(available.right().max(last_cell.right()), gutter.bottom()),
    );
    egui::Shape::Vec(vec![
        egui::Shape::rect_filled(band, 2.0, fill),
        egui::Shape::rect_stroke(
            band,
            2.0,
            egui::Stroke::new(1.0_f32, listing.accent),
            egui::StrokeKind::Inside,
        ),
    ])
}

/// Everything the right button offers on one row.
///
/// Its own function because a menu grows: it started at two entries and now
/// carries six, and every one of them is a sentence rather than a word.
fn row_menu(ui: &mut egui::Ui, instruction: &Instruction, language: Language, asked: &mut Asked) {
    let address = instruction.address;

    if ui.button(text(language, Text::InspectOperand)).clicked() {
        asked.inspect = Some(address);
        ui.close_menu();
    }
    if ui.button(text(language, Text::EditNote)).clicked() {
        asked.annotate = Some(address);
        ui.close_menu();
    }
    if ui.button(text(language, Text::Bookmark)).clicked() {
        asked.bookmark = Some(address);
        ui.close_menu();
    }
    if ui.button(text(language, Text::ReferencesTo)).clicked() {
        asked.references = Some(address);
        ui.close_menu();
    }
    // Running is offered from the listing because that is where a reader
    // decides they want to stop somewhere: the alternative is selecting a
    // row here and pressing a key in another view.
    if ui.button(text(language, Text::ToggleBreakpoint)).clicked() {
        asked.breakpoint = Some(address);
        ui.close_menu();
    }
    if ui.button(text(language, Text::RunToCursor)).clicked() {
        asked.run_to = Some(address);
        ui.close_menu();
    }
    // Where the operand points if it points anywhere, and otherwise the
    // instruction's own bytes — both are answers to "show me what is
    // actually there".
    if ui.button(text(language, Text::FollowInDump)).clicked() {
        asked.follow = Some(operand::target_address(instruction).unwrap_or(address));
        ui.close_menu();
    }
}

/// The fill the run's marks put behind an address, and what hovering it says.
///
/// Nothing when there is no run: the marks belong to a machine that exists,
/// and a listing opened without one carries neither.
fn run_marks(listing: &Listing, address: u64) -> Option<(egui::Color32, Text)> {
    let machine = listing.machine?;
    if machine.instruction_pointer() == address {
        return Some((
            crate::ui::machine::CURRENT.gamma_multiply(RUN_MARK_FILL),
            Text::NextInstruction,
        ));
    }
    machine.has_breakpoint(address).then_some((
        crate::ui::machine::BREAKPOINT.gamma_multiply(RUN_MARK_FILL),
        Text::ToggleBreakpoint,
    ))
}

/// How much of the mark's colour is laid behind the address. Enough to be
/// unmissable running down the listing, faint enough to read the digits over.
const RUN_MARK_FILL: f32 = 0.42;

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
                // Spelled the way the row it came from is: a hover that
                // answered in AT&T over a NASM listing would read as another
                // instruction rather than as the one on that line.
                let respelled = listing
                    .nasm
                    .and_then(|nasm| nasm.borrow_mut().spell(landing));
                ui.strong(text(language, Text::TargetInstruction));
                ui.label(syntax::assembly(
                    ui,
                    respelled.as_deref().unwrap_or(&landing.text),
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
    // Through the branch reader, not the general one: AArch64 writes every
    // branch target as an immediate, which the general reader refuses on
    // purpose — so the gutter used to draw no arrows at all on those files.
    operand::branch_target(instruction)
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
/// One line under the toolbar, saying where the walk stands and what the
/// selected instruction has done to the stack.
///
/// It replaces three lines that used to be stacked here — the next step, the
/// stack, and a sentence about the keys — because a listing whose heading
/// takes a third of the window is a listing you have to scroll to reach. What
/// was a sentence about the keys is now under the `?` at the end of the row,
/// where an explanation of the view belongs and where it costs no room at all.
///
/// A plain row, never a wrapped one: a wrapped row draws every one of its
/// parts at the same place in egui 0.31.
fn status_line(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    stack: &Trace,
    selected: Option<u64>,
    language: Language,
) {
    ui.horizontal(|ui| {
        if let Some(address) = selected {
            ui.label(
                egui::RichText::new(next_step_sentence(analysis, address, language))
                    .small()
                    .color(MUTED),
            );
            ui.separator();
        }
        stack_summary(ui, analysis, stack, selected, language);
        // The explanations of the view itself, gathered under one mark at the
        // end of the row: what the keys and the gutter arrows do, and which
        // decoders produced the listing.
        ui.separator();
        ui.small("?").on_hover_text(format!(
            "{}\n\n{}",
            text(language, Text::DisassemblyHelp),
            text(language, Text::LocalDecoders)
        ));
    });
}

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
) -> egui::Response {
    let depth = analysis
        .instruction_index(address)
        .and_then(|index| stack.depth(index));
    match depth {
        Some(depth) => ui
            .label(syntax::dim(
                ui,
                &format!("{depth:#x}"),
                egui::Color32::TRANSPARENT,
            ))
            .on_hover_text(text(language, Text::StackHelp)),
        None => ui
            .label(egui::RichText::new("?").monospace().color(MUTED))
            .on_hover_text(text(language, Text::StackUnknown)),
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui;

    use super::{
        JUMP, LEADING, Nasm, SECTION_SNAP, VIEWPORT_MINIMUM_HEIGHT, instruction_at_overview_row,
        is_jump, jump_target, lanes, overview_row, overview_y, row_of, run_marks, section_near,
        section_starts, snapped_row,
    };
    use crate::{
        app::{Dialog, WorkspaceView},
        commands::Command,
        i18n::{Language, Text, text},
        testing::{
            drawn, drawn_text, listing_window_input, opened_app, reference_analysis, window_input,
        },
        ui::views,
    };

    #[test]
    fn the_overview_clamps_its_ends_and_lands_on_decoded_code() {
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 100.0), egui::vec2(16.0, 300.0));
        let analysis = reference_analysis();
        let sections = section_starts(analysis);
        let total = LEADING + analysis.instructions.len() + sections.len();

        assert_eq!(overview_row(rect, -1_000.0, total), 0);
        assert_eq!(overview_row(rect, 1_000.0, total), total.saturating_sub(1));
        assert!(
            instruction_at_overview_row(analysis, &sections, overview_row(rect, rect.top(), total))
                .is_some()
        );
        assert!(
            instruction_at_overview_row(
                analysis,
                &sections,
                overview_row(rect, rect.bottom(), total),
            )
            .is_some()
        );
    }

    /// Twenty-four rows out of a hundred and thirty thousand is a fifth of a
    /// pixel, and the reader is left looking for a speck to know where they
    /// are. The rectangle stops being to scale below a readable height — and
    /// still has to stay inside the ruler when the listing is at its end,
    /// which growing it downwards would not.
    #[test]
    fn the_viewport_is_legible_however_long_the_listing_and_never_leaves_the_ruler() {
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 100.0), egui::vec2(16.0, 300.0));
        let total = 139_000;

        for first in [0_usize, 60_000, total - 25] {
            let start = overview_y(rect, first, total);
            let end = overview_y(rect, first + 24, total);
            let top = (start - 1.0)
                .min(rect.bottom() - VIEWPORT_MINIMUM_HEIGHT)
                .max(rect.top());
            let bottom = (end + 1.0)
                .max(top + VIEWPORT_MINIMUM_HEIGHT)
                .min(rect.bottom());

            assert!(
                bottom - top >= VIEWPORT_MINIMUM_HEIGHT - 0.01,
                "at row {first} the viewport is {} pixels tall",
                bottom - top
            );
            assert!(
                top >= rect.top() && bottom <= rect.bottom(),
                "at row {first} the viewport runs outside the ruler"
            );
        }
    }

    /// The ruler maps a hundred and thirty thousand rows onto six hundred
    /// pixels, so a section start cannot be hit by aiming: one pixel is two
    /// hundred instructions. A click near a mark means the mark.
    #[test]
    fn a_click_beside_a_section_mark_means_the_section_and_not_the_pixel() {
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 100.0), egui::vec2(16.0, 300.0));
        let total = 139_000;
        let mark = rect.top() + 120.0;
        let sections = [(mark, ".text")];

        assert_eq!(
            snapped_row(rect, mark + SECTION_SNAP - 1.0, total, &sections),
            overview_row(rect, mark, total),
            "a click within reach of the mark lands on it"
        );
        assert_ne!(
            snapped_row(rect, mark + SECTION_SNAP + 6.0, total, &sections),
            overview_row(rect, mark, total),
            "and a click clearly elsewhere is left where it was"
        );
        assert_eq!(
            section_near(&sections, mark + 1.0),
            Some(".text"),
            "hovering the mark names what it is"
        );
        assert_eq!(
            section_near(&sections, mark + 40.0),
            None,
            "and hovering away from it names nothing"
        );
    }

    /// The listing must not walk sideways as the reader scrolls.
    ///
    /// A virtualised grid sizes each column to the rows on screen, so
    /// scrolling from short instructions into long ones used to shift every
    /// column right of the bytes — sixty-three pixels, measured on the
    /// reference binary. Drawn at two scroll positions here, and every column
    /// has to land in the same place in both.
    /// The listing gets the width a whole line of it needs, and the
    /// pseudo-code keeps a floor.
    ///
    /// Halving it — which is what `Ui::columns` did — is wrong in both
    /// directions: a file whose longest line is `mov %rax,%rbx` left half the
    /// listing empty while the pseudo-code went short, and one holding
    /// `movsd 0x1234(%rip),%xmm0` had its instructions cut off mid-operand at
    /// the middle of the window. Cut off rather than painted over: the scroll
    /// area clips its own contents, which is why nothing about this was
    /// visible in a rendered sheet — that draws every shape, clipped or not.
    /// The listing is served first, and to the character.
    ///
    /// It gets exactly what one whole line of it needs — no more, so a file of
    /// short instructions leaves the rest to the pseudo-code; no less, so an
    /// instruction is never cut at the middle of the window. Halving the
    /// width, which is what `Ui::columns` did, was wrong in both of those
    /// directions at once.
    /// An address as the listing writes it for this file.
    ///
    /// Asked of the same function the view uses, with the same width: the
    /// number of digits comes from the file's highest address, so a test that
    /// spelled sixteen of them by hand was looking for a string the view had
    /// stopped drawing.
    fn address_of(app: &crate::app::DesdecApp, address: u64) -> String {
        super::address_text(app.listing_columns.address, address)
    }

    /// The same, where only the width is at hand.
    fn address_at_width(width: usize, address: u64) -> String {
        super::address_text(width, address)
    }

    #[test]
    fn the_listing_is_given_the_width_a_line_of_it_needs() {
        let ctx = egui::Context::default();
        let _ = ctx.run(window_input(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // Address, bytes, section, stack, instruction — a file whose
                // longest line is wide, and one whose longest line is short.
                let wide = [200.0, 300.0, 60.0, 40.0, 400.0];
                let narrow = [200.0, 300.0, 60.0, 40.0, 40.0];
                let gutter = 36.0;
                let spacing = ui.spacing().item_spacing.x;
                let wanted =
                    |columns: [f32; 5]| columns.iter().sum::<f32>() + super::GUTTER_WIDTH + spacing * 6.0;

                // Room for everything: exactly what it asked for.
                let plenty = wanted(wide) + 800.0;
                assert!(
                    (super::listing_share(ui, wide, plenty, gutter) - wanted(wide)).abs() < 0.5,
                    "with room to spare the listing should take exactly what it needs"
                );

                // A file of short instructions does not hold on to half the
                // window: what it does not need goes to the pseudo-code.
                let short = super::listing_share(ui, narrow, plenty, gutter);
                assert!(
                    (short - wanted(narrow)).abs() < 0.5,
                    "a short listing takes {short} where it needs {}",
                    wanted(narrow)
                );
                assert!(
                    short < (plenty - gutter) / 2.0,
                    "and leaves the pseudo-code more than half the window"
                );

                // Too little room for both: the listing still gets everything
                // it can, and the pseudo-code keeps just enough to be a panel
                // with a scrollbar. It is the side that gives way now.
                let tight = 700.0;
                let share = super::listing_share(ui, wide, tight, gutter);
                let left_over = tight - gutter - share;
                assert!(
                    left_over > 0.0,
                    "the pseudo-code is left {left_over} pixels of a {tight} pixel window"
                );
                assert!(
                    share > (tight - gutter) / 2.0,
                    "a long listing takes more than half when it needs more than half"
                );
            });
        });
    }

    #[test]
    fn the_columns_stay_where_they_are_as_the_listing_scrolls() {
        fn column_positions(at: Option<u64>) -> Vec<f32> {
            let ctx = egui::Context::default();
            let mut app = opened_app(WorkspaceView::Disassembly);
            app.selected_instruction = at;
            app.pending_instruction_scroll = at;
            let mut draw = |ctx: &egui::Context| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = super::show(&mut app, ui);
                });
            };
            let _ = ctx.run(window_input(), &mut draw);
            let output = ctx.run(window_input(), &mut draw);
            // Every position a row put text at, kept only where several rows
            // agree — which is what a column is. The address column alone
            // would prove nothing: it is the first, and the first never moves.
            // What moved was everything to the right of the bytes.
            let mut seen: Vec<f32> = drawn(&output.shapes)
                .into_iter()
                .map(|(_, at)| at.x)
                .collect();
            seen.sort_by(f32::total_cmp);
            let mut columns = Vec::new();
            let mut run = 0_usize;
            for index in 0..seen.len() {
                run += 1;
                // Two positions are the same column when they are the same
                // number: they come from the same layout, not from arithmetic
                // on it, so there is nothing to be tolerant of here.
                let ends = index + 1 == seen.len()
                    || seen[index + 1].total_cmp(&seen[index]) != std::cmp::Ordering::Equal;
                if ends {
                    if run >= 5 {
                        columns.push(seen[index]);
                    }
                    run = 0;
                }
            }
            columns
        }

        let analysis = reference_analysis();
        let top = analysis.instructions.first().expect("a listing").address;
        // A row far enough down that the visible instructions are other ones.
        let far = analysis
            .instructions
            .get(analysis.instructions.len() / 2)
            .expect("a listing")
            .address;
        assert_ne!(top, far, "the two positions are different rows");
        assert_eq!(
            column_positions(Some(top)),
            column_positions(Some(far)),
            "the columns moved between two scroll positions"
        );
    }

    /// The two marks a run puts on the listing.
    ///
    /// Checked through [`run_marks`], which is what decides them: the marks
    /// themselves are a colour laid behind the address by the text layout, and
    /// a test that went looking for a rectangle would be testing egui rather
    /// than this. What matters here is that a listing with no run carries
    /// neither mark, that the row the run stands on carries the one, and the
    /// row it would stop on the other.
    #[test]
    fn the_listing_marks_where_the_run_stands_and_where_it_will_stop() {
        use crate::ui::machine::{BREAKPOINT, CURRENT};

        // A fresh borrow each time, so the application stays free to be told
        // about the next breakpoint between two questions.
        fn marks(app: &crate::app::DesdecApp, address: u64) -> Option<(egui::Color32, Text)> {
            run_marks(
                &super::Listing {
                    patches: &app.patches,
                    stack: &app.stack,
                    file: &app.file_bytes,
                    sections: &app.section_starts,
                    accent: egui::Color32::WHITE,
                    notes: &app.annotations,
                    members: &app.member_names,
                    hints: false,
                    columns: [0.0; 5],
                    address_width: 18,
                    machine: app.machine.as_ref(),
                    nasm: None,
                    language: Language::English,
                },
                address,
            )
        }

        let ctx = egui::Context::default();
        // The x86-64 fixture, for the reason given on `emulatable_sample`.
        let mut app = crate::testing::emulatable_sample().opened(WorkspaceView::Disassembly);
        let entry = app
            .analysis
            .as_ref()
            .and_then(|analysis| analysis.entry_point)
            .expect("the fixture has an entry point");
        app.selected_instruction = Some(entry);

        assert!(
            app.machine.is_none(),
            "opening a binary starts nothing at all"
        );
        app.run_command(&ctx, Command::MachineToggleBreakpoint);
        let machine = app.machine.as_ref().expect("asking for one builds it");
        assert!(machine.has_breakpoint(entry));
        assert_eq!(machine.instruction_pointer(), entry);

        // The run stands on the entry point *and* has a breakpoint on it, and
        // where it stands is what a reader needs to see first.
        assert_eq!(
            marks(&app, entry).map(|(colour, _)| colour),
            Some(fill(CURRENT))
        );
        let elsewhere = entry.wrapping_add(1);
        assert_eq!(
            marks(&app, elsewhere),
            None,
            "a row with neither carries neither"
        );

        app.selected_instruction = Some(elsewhere);
        app.run_command(&ctx, Command::MachineToggleBreakpoint);
        assert_eq!(
            marks(&app, elsewhere).map(|(colour, _)| colour),
            Some(fill(BREAKPOINT)),
            "a row the run would stop on is marked"
        );
    }

    /// The colour a mark is painted in, once faded behind the text.
    fn fill(colour: egui::Color32) -> egui::Color32 {
        colour.gamma_multiply(super::RUN_MARK_FILL)
    }

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
        width: usize,
        address: u64,
        assembly: &str,
    ) -> Option<egui::Pos2> {
        let drawn = drawn(shapes);
        // The address is drawn twice — once in each listing — so the row is
        // the one where the assembly was drawn beside it.
        let rows: Vec<f32> = drawn
            .iter()
            .filter(|(text, _)| *text == address_at_width(width, address))
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
            drawn_text(&output.shapes).contains(&address_of(&app, second)),
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
            let _ = ctx.run(listing_window_input(), |ctx| {
                views::show_central_panel(&mut app, ctx);
            });
            app.pending_instruction_scroll = Some(address);
            let placed = ctx.run(listing_window_input(), |ctx| {
                views::show_central_panel(&mut app, ctx);
            });
            let position = assembly_position(&placed.shapes, app.listing_columns.address, address, &assembly)
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

    /// The file is decoded once, in AT&T, and can be read in NASM: the switch
    /// changes what the assembly column says, and says it from the row's own
    /// bytes.
    ///
    /// The reference binary is this host's own executable, so an `AArch64`
    /// runner has one spelling and nothing to check; the test says so by
    /// stopping rather than by asserting something true everywhere.
    #[test]
    fn the_listing_can_be_read_in_the_nasm_spelling() {
        let analysis = reference_analysis();
        let Some(mut nasm) = Nasm::for_architecture(analysis.summary.architecture) else {
            return; // One spelling on this host: no choice to make.
        };
        // A row whose two spellings differ, which is what tells the reading
        // apart from the decoding.
        let Some((address, att, intel)) = analysis.instructions.iter().find_map(|instruction| {
            let spelled = nasm.spell(instruction)?;
            (spelled != instruction.text)
                .then(|| (instruction.address, instruction.text.clone(), spelled))
        }) else {
            return;
        };

        let read = |nasm_syntax: bool| {
            let mut app = opened_app(WorkspaceView::Disassembly);
            app.preferences.language = Language::English;
            app.preferences.nasm_syntax = nasm_syntax;
            app.selected_instruction = Some(address);
            let ctx = egui::Context::default();
            // Two frames to put the row on screen: the scroll area learns its
            // content size from the first and lands on the second.
            for _ in 0..2 {
                app.pending_instruction_scroll = Some(address);
                let _ = ctx.run(listing_window_input(), |ctx| {
                    views::show_central_panel(&mut app, ctx);
                });
            }
            app.pending_instruction_scroll = Some(address);
            let placed = ctx.run(listing_window_input(), |ctx| {
                views::show_central_panel(&mut app, ctx);
            });
            crate::testing::drawn_text(&placed.shapes)
        };

        let in_att = read(false);
        assert!(
            in_att.contains(&att),
            "the listing is written in AT&T until it is asked otherwise"
        );
        let in_nasm = read(true);
        assert!(
            in_nasm.contains(&intel),
            "the row must be re-spelled: {intel} was not drawn"
        );
        assert!(
            !in_nasm.contains(&att),
            "and the AT&T spelling of that row must be gone with it"
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
        let output = ctx.run(listing_window_input(), |ctx| {
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
        let output = ctx.run(listing_window_input(), |ctx| {
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

    /// The offset a reader translates in their head, translated on the row
    /// itself. This is what a structure is written down *for*.
    #[test]
    fn the_listing_names_what_a_row_touches_through_the_type_it_was_told_about() {
        let analysis = reference_analysis();
        let Some(address) = analysis.instructions.first().map(|i| i.address) else {
            return;
        };
        let mut app = opened_app(WorkspaceView::Disassembly);
        // Said about the first row directly rather than worked out from a
        // function: what is under test is that the listing draws it, not how
        // the index was filled.
        app.member_names = crate::ui::types::MemberNames::of([(address, "header.count")]);

        let ctx = egui::Context::default();
        let output = ctx.run(listing_window_input(), |ctx| {
            views::show_central_panel(&mut app, ctx);
        });

        let drawn = drawn_text(&output.shapes);
        assert!(
            drawn.contains("→ header.count"),
            "the row says which member it touches: {drawn}"
        );
    }

    /// The listing is virtualised, so the row an instruction sits on is not
    /// laid out until it is scrolled to. Reaching one far down the listing has
    /// to actually put it on screen, or every cross-reference in the interface
    /// leads nowhere.
    ///
    /// Both listings, not one: the pseudo-code beside the disassembly is a
    /// second virtualised list of the same instructions, and it has to follow
    /// the same address. It is checked by its own text rather than by the
    /// address, which it no longer draws — the listing next to it carries the
    /// same address on the same row, and a second column of them was a fifth
    /// of the window spent saying everything twice.
    #[test]
    fn scrolling_to_a_distant_instruction_brings_it_into_view() {
        let analysis = reference_analysis();
        let Some(target) = analysis.instructions.last() else {
            return; // Nothing decoded on this host: nothing to scroll to.
        };
        let address = target.address;
        let pseudo_code = crate::ui::decompile::pseudo_c(&target.text);
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

        let drawn = drawn_text(&output.shapes);
        assert_eq!(
            drawn.matches(&address_of(&app, address)).count(),
            1,
            "the listing must scroll to the instruction, and say its address once"
        );
        assert!(
            drawn.contains(&pseudo_code),
            "the pseudo-code beside it must scroll to the same instruction: \
             {pseudo_code} is not in what was drawn"
        );
    }

    /// The transport has to be *offered*, not merely drawn: pressing a step
    /// has to take one.
    ///
    /// Every one of these commands refuses to run without a binary, and the
    /// row that holds them once lifted the analysis out of `DesdecApp` for the
    /// duration of its own drawing — so `can_run` answered no to all of them,
    /// every button came out greyed, and stepping, restarting and jumping to
    /// the entry point did nothing at all.
    ///
    /// Two weaker versions of this test were written first and both passed
    /// against that very defect. Asking `can_run` once the view has been drawn
    /// proves nothing, because the analysis is back by then. Counting the
    /// colours the row was painted in proves nothing either: a greyed
    /// transport still shares its row with a separator drawn in another shade,
    /// which was enough to satisfy "more than one colour". Only pressing the
    /// button and looking at what moved says anything at all.
    #[test]
    fn pressing_a_step_in_the_transport_takes_one() {
        let analysis = reference_analysis();
        if analysis.instructions.is_empty() {
            return; // Nothing decoded on this host.
        }
        let mut app = opened_app(WorkspaceView::Disassembly);
        let ctx = egui::Context::default();
        app.run_command(&ctx, Command::WalkToEntry);
        let before = app.walk.steps();

        let mut draw = |ctx: &egui::Context| {
            views::show_central_panel(&mut app, ctx);
        };
        let _ = ctx.run(listing_window_input(), &mut draw);
        let output = ctx.run(listing_window_input(), &mut draw);

        // Where the buttons are, read off the frame rather than guessed at:
        // the glyphs are the only thing drawn on that row, and they fall into
        // one cluster of ink per button.
        let heading = text(Language::French, Text::StaticWalk).to_uppercase();
        let (_, at) = drawn(&output.shapes)
            .into_iter()
            .find(|(said, _)| *said == heading)
            .expect("the transport names itself");
        let mut marks = Vec::new();
        for clipped in &output.shapes {
            collect_ink(&clipped.shape, at, &mut marks);
        }
        let buttons = buttons_from(&marks, ctx.style().spacing.item_spacing.x);
        assert!(
            buttons.len() >= 6,
            "the transport draws six buttons; found {} clusters of ink",
            buttons.len()
        );

        // The fourth of them, in the order the row lays them out: step over.
        let _ = ctx.run(crate::testing::click_at(buttons[3]), &mut draw);
        let _ = ctx.run(listing_window_input(), &mut draw);

        assert!(
            app.walk.steps() > before,
            "pressing step-over left the walk where it was: {} steps",
            app.walk.steps()
        );
    }

    /// Where line work was laid down on the transport's own row.
    ///
    /// Only that row and only to the right of its heading, so the rest of the
    /// view cannot be mistaken for a button.
    fn collect_ink(shape: &egui::Shape, row: egui::Pos2, out: &mut Vec<egui::Pos2>) {
        let mut take = |at: egui::Pos2, width: f32| {
            if width > 0.0 && at.x > row.x && (at.y - row.y).abs() < crate::ui::ROW_HEIGHT {
                out.push(at);
            }
        };
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_ink(shape, row, out);
                }
            }
            egui::Shape::LineSegment { points, stroke } => take(points[0], stroke.width),
            egui::Shape::Path(path) => {
                if let Some(first) = path.points.first() {
                    take(*first, path.stroke.width);
                }
            }
            _ => {}
        }
    }

    /// The centre of each button of the transport, from the ink on its row.
    ///
    /// Bucketed by the row's own pitch — a button's width plus the spacing
    /// between two of them — rather than by the gaps between marks. A glyph is
    /// about as wide as the space between two of them, so grouping by gap
    /// merged neighbours and found three buttons where there are six.
    fn buttons_from(marks: &[egui::Pos2], spacing: f32) -> Vec<egui::Pos2> {
        let pitch = crate::icons::BUTTON_SIZE.x + spacing;
        let Some(left) = marks.iter().map(|at| at.x).min_by(f32::total_cmp) else {
            return Vec::new();
        };
        let mut buckets: std::collections::BTreeMap<i64, Vec<egui::Pos2>> =
            std::collections::BTreeMap::new();
        for mark in marks {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a toolbar is a handful of buttons wide"
            )]
            let bucket = ((mark.x - left) / pitch).round() as i64;
            buckets.entry(bucket).or_default().push(*mark);
        }
        buckets
            .into_values()
            .map(|cluster| {
                #[expect(clippy::cast_precision_loss, reason = "a handful of marks per glyph")]
                let count = cluster.len() as f32;
                let sum = cluster
                    .iter()
                    .fold(egui::Vec2::ZERO, |sum, at| sum + at.to_vec2());
                (sum / count).to_pos2()
            })
            .collect()
    }

    /// The two columns of the view are one listing read two ways, so the C for
    /// an instruction has to sit on the row the assembly for it sits on.
    ///
    /// They used to be counted differently: the listing draws a heading where
    /// each section begins and the pseudo-code drew none, so the two drifted a
    /// row further apart at every section boundary. On top of that the panel's
    /// title sat *outside* its scroll area and pushed every one of its rows
    /// twenty-six pixels down. Reading the positions is the only way to see
    /// either — every line is drawn whatever happens, just not beside the one
    /// it belongs to.
    ///
    /// Only lines drawn once are checked. A `.plt` is a hundred entries that
    /// translate to the same two statements, and pairing an address with the
    /// first line that merely *reads* like its own answers nothing: the first
    /// version of this test failed on exactly that, and against code that was
    /// by then already aligned.
    #[test]
    fn each_pseudo_code_line_sits_on_the_row_of_its_instruction() {
        let analysis = reference_analysis();
        if analysis.instructions.is_empty() {
            return; // Nothing decoded on this host.
        }
        let mut app = opened_app(WorkspaceView::Disassembly);
        let ctx = egui::Context::default();
        let mut draw = |ctx: &egui::Context| {
            views::show_central_panel(&mut app, ctx);
        };
        let _ = ctx.run(listing_window_input(), &mut draw);
        let output = ctx.run(listing_window_input(), &mut draw);

        let drawn = drawn(&output.shapes);
        let once = |wanted: &str| {
            let mut found = drawn.iter().filter(|(said, _)| said == wanted);
            match (found.next(), found.next()) {
                (Some((_, at)), None) => Some(*at),
                _ => None,
            }
        };

        let mut checked = 0;
        for instruction in &analysis.instructions {
            let Some(at) = once(&address_of(&app, instruction.address)) else {
                continue; // Scrolled away from, or not the only one of its kind.
            };
            let code = crate::ui::decompile::pseudo_c(&instruction.text);
            let Some(beside) = once(&format!("    {code}")) else {
                continue; // A translation this file draws more than once.
            };
            assert!(
                (beside.y - at.y).abs() < crate::ui::ROW_HEIGHT / 2.0,
                "{:#018x} is drawn at y={} and its pseudo-code at y={}",
                instruction.address,
                at.y,
                beside.y
            );
            checked += 1;
        }
        assert!(
            checked >= 5,
            "too few rows could be paired to say anything: {checked}"
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
