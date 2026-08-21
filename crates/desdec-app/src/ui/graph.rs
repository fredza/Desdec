//! One function drawn as its control flow: the graph view.
//!
//! The listing reads a function the way the bytes are laid out, which is not
//! the way it runs. A test at the top and its two arms are three places in the
//! listing, hundreds of lines apart if the compiler put them there, and
//! nothing on screen says the second arm is the *other* answer to the first
//! question. Drawn as a graph they are three boxes and two arrows, and the
//! shape of the function — a guard, a loop, a chain of early returns — is
//! visible before a single instruction has been read.
//!
//! This is x64dbg's graph view, and it is drawn from the same reading the rest
//! of Desdec does: [`desdec_core::blocks`] cuts the body into basic blocks,
//! stating only edges the listing states. A branch through a register leaves a
//! block with no arrow out rather than a guessed one, and the view says as
//! much on the block itself — an arrow that was invented would be a shape the
//! program does not have.

use desdec_core::{
    Instruction,
    blocks::{BasicBlock, Edge, Exit},
};
use eframe::egui;

use crate::{
    app::{DesdecApp, WorkspaceView},
    i18n::{Language, Text, text},
    ui::{MUTED, functions::Function, machine},
};

/// Colours of the three kinds of arrow, which are x64dbg's: the arm taken when
/// a condition holds, the arm taken when it does not, and everything else.
const TAKEN: egui::Color32 = egui::Color32::from_rgb(106, 176, 118);
const NOT_TAKEN: egui::Color32 = egui::Color32::from_rgb(206, 106, 100);
const PLAIN: egui::Color32 = egui::Color32::from_rgb(132, 142, 164);

/// Space between one rank of blocks and the next, and between two blocks of
/// the same rank.
const RANK_GAP: f32 = 54.0;
const COLUMN_GAP: f32 = 32.0;

/// Padding inside a block, and the height of one instruction line.
const PADDING: f32 = 8.0;
const LINE: f32 = 15.0;

/// How many instructions a block shows before it says how many more there are.
///
/// A block of nine hundred instructions — an unrolled `memcpy` says as much —
/// would be a column taller than any screen, and the shape of the function is
/// what this view is for. The count that follows is what stops the trimming
/// from being a lie.
const LINES_SHOWN: usize = 40;

/// How far the view may be zoomed, either way.
const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.35..=2.5;

/// Where the reader has moved and scaled the graph.
///
/// Kept per function: walking away to another function and coming back should
/// find the graph where it was left, and a pan worked out for one function
/// means nothing for the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub of_function: u64,
    pub offset: egui::Vec2,
    pub zoom: f32,
    /// Set until the graph has been placed once, so the entry block is brought
    /// into view rather than the reader landing on empty space.
    pub centre_on_entry: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            of_function: 0,
            offset: egui::Vec2::ZERO,
            zoom: 1.0,
            centre_on_entry: true,
        }
    }
}

impl View {
    /// Starts the view over when the function being looked at has changed.
    fn follow(&mut self, function: u64) {
        if self.of_function != function {
            *self = Self {
                of_function: function,
                ..Self::default()
            };
        }
    }
}

/// Where one block sits once the graph has been placed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// Which block of the function this is.
    pub block: usize,
    pub rect: egui::Rect,
    /// How far down the graph it is, in ranks from the entry.
    pub rank: usize,
}

/// Every block placed, and how large the whole drawing came out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub nodes: Vec<Placed>,
    pub size: egui::Vec2,
}

impl Layout {
    fn of_block(&self, block: usize) -> Option<&Placed> {
        self.nodes.iter().find(|node| node.block == block)
    }
}

/// Places every block of a function, given how large each one is to draw.
///
/// The blocks are laid out in ranks, each rank one step further from the entry
/// than the last, which is what makes a graph read downwards: an arrow that
/// goes down is the flow carrying on, and the few that go up are the loops.
/// Ranks come from a breadth-first walk from the entry block, so a back edge —
/// the arrow that closes a loop — never pushes its target further down, and a
/// block nothing reaches is placed after the rest rather than dropped.
///
/// Within a rank the order is settled by the average position of each block's
/// predecessors, run twice. That is the cheap half of what a layered graph
/// drawer does, and it is what stops the two arms of a test from being drawn
/// crossed over each other.
#[must_use]
pub fn layout(blocks: &[BasicBlock], sizes: &[egui::Vec2]) -> Layout {
    if blocks.is_empty() || blocks.len() != sizes.len() {
        return Layout::default();
    }
    let ranks = ranks(blocks);
    let mut rows = rows_of(&ranks);
    order_within_rows(blocks, &ranks, &mut rows);
    place(&rows, sizes)
}

/// How far each block is from the entry, by a breadth-first walk.
///
/// Blocks nothing reaches — code after an unconditional jump that only a
/// computed branch ever lands on — are given the rank after the deepest one
/// reached, so they are drawn below the graph proper instead of being left
/// out of it.
fn ranks(blocks: &[BasicBlock]) -> Vec<usize> {
    let index_of = |address: u64| blocks.iter().position(|block| block.start == address);
    let mut ranks = vec![usize::MAX; blocks.len()];
    let mut queue = std::collections::VecDeque::from([0]);
    ranks[0] = 0;
    while let Some(current) = queue.pop_front() {
        let rank = ranks[current];
        for successor in &blocks[current].successors {
            let Some(next) = index_of(successor.address) else {
                continue;
            };
            if ranks[next] == usize::MAX {
                ranks[next] = rank + 1;
                queue.push_back(next);
            }
        }
    }

    let deepest = ranks
        .iter()
        .filter(|rank| **rank != usize::MAX)
        .max()
        .copied()
        .unwrap_or(0);
    // Unreached blocks keep their order among themselves, one rank each, so
    // two of them are never drawn on top of one another.
    let mut spare = deepest;
    for rank in &mut ranks {
        if *rank == usize::MAX {
            spare += 1;
            *rank = spare;
        }
    }
    ranks
}

/// The blocks of each rank, in address order to begin with.
fn rows_of(ranks: &[usize]) -> Vec<Vec<usize>> {
    let deepest = ranks.iter().max().copied().unwrap_or(0);
    let mut rows = vec![Vec::new(); deepest + 1];
    for (block, rank) in ranks.iter().enumerate() {
        rows[*rank].push(block);
    }
    rows
}

/// Sorts each rank by where its blocks' predecessors sit in the rank above.
///
/// Two passes: one is enough to pull a block under whatever leads to it, and a
/// second settles the ranks that changed underneath the first.
fn order_within_rows(blocks: &[BasicBlock], ranks: &[usize], rows: &mut [Vec<usize>]) {
    let index_of = |address: u64| blocks.iter().position(|block| block.start == address);
    for _ in 0..2 {
        for row in 1..rows.len() {
            let above: Vec<usize> = rows[row - 1].clone();
            let position_above = |block: usize| above.iter().position(|other| *other == block);
            let mut keyed: Vec<(f32, usize)> = rows[row]
                .iter()
                .map(|block| {
                    // Where the blocks leading here sit, averaged. A block
                    // nothing above leads to keeps its place, at the far end.
                    let mut total = 0.0;
                    let mut count = 0.0;
                    for (other, rank) in ranks.iter().enumerate() {
                        if *rank + 1 != row {
                            continue;
                        }
                        let leads_here = blocks[other]
                            .successors
                            .iter()
                            .any(|out| index_of(out.address) == Some(*block));
                        if leads_here && let Some(at) = position_above(other) {
                            #[expect(
                                clippy::cast_precision_loss,
                                reason = "a position in a row of blocks, never near the limits of f32"
                            )]
                            let at = at as f32;
                            total += at;
                            count += 1.0;
                        }
                    }
                    #[expect(
                        clippy::cast_precision_loss,
                        reason = "a position in a row of blocks, never near the limits of f32"
                    )]
                    let fallback = f32::from(u16::MAX) + *block as f32;
                    let key = if count > 0.0 { total / count } else { fallback };
                    (key, *block)
                })
                .collect();
            // A stable sort, so blocks sharing a key stay in address order —
            // which is the order a reader already knows them in.
            keyed.sort_by(|left, right| {
                left.0
                    .partial_cmp(&right.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            rows[row] = keyed.into_iter().map(|(_, block)| block).collect();
        }
    }
}

/// Turns ranks and orders into rectangles, each rank centred under the widest.
fn place(rows: &[Vec<usize>], sizes: &[egui::Vec2]) -> Layout {
    let widths: Vec<f32> = rows
        .iter()
        .map(|row| {
            let blocks: f32 = row.iter().map(|block| sizes[*block].x).sum();
            #[expect(
                clippy::cast_precision_loss,
                reason = "a count of blocks in one rank, never near the limits of f32"
            )]
            let gaps = COLUMN_GAP * row.len().saturating_sub(1) as f32;
            blocks + gaps
        })
        .collect();
    let total_width = widths.iter().copied().fold(0.0_f32, f32::max);

    let mut nodes = Vec::new();
    let mut y = 0.0_f32;
    for (rank, row) in rows.iter().enumerate() {
        let mut x = (total_width - widths[rank]) / 2.0;
        let mut tallest = 0.0_f32;
        for block in row {
            let size = sizes[*block];
            nodes.push(Placed {
                block: *block,
                rect: egui::Rect::from_min_size(egui::pos2(x, y), size),
                rank,
            });
            x += size.x + COLUMN_GAP;
            tallest = tallest.max(size.y);
        }
        y += tallest + RANK_GAP;
    }

    Layout {
        nodes,
        size: egui::vec2(total_width, (y - RANK_GAP).max(0.0)),
    }
}

/// How large a block is to draw, from what it holds.
fn size_of(ui: &egui::Ui, block: &BasicBlock, body: &[Instruction]) -> egui::Vec2 {
    let lines = block.instruction_count().min(LINES_SHOWN);
    let trimmed = usize::from(block.instruction_count() > LINES_SHOWN);
    // The line saying how the flow leaves, when there is one: counted here or
    // it is drawn below the box that is supposed to hold it.
    let footer = usize::from(block.exit != Exit::Onwards);
    let widest = body
        .get(block.instructions.clone())
        .unwrap_or_default()
        .iter()
        .take(LINES_SHOWN)
        .map(|instruction| line_of(instruction).chars().count())
        .max()
        .unwrap_or(0)
        // The heading is the block's address, which is never the long part.
        .max(20);
    #[expect(
        clippy::cast_precision_loss,
        reason = "a count of lines in one block, never near the limits of f32"
    )]
    let height = LINE * (lines + trimmed + footer + 1) as f32 + PADDING * 2.0;
    egui::vec2(
        crate::ui::disassembly::monospace_width(ui, widest) + PADDING * 2.0,
        // The heading, the instructions, and the "and n more" when there is
        // one.
        height,
    )
}

/// One instruction as the block shows it: its address, short, and its text.
fn line_of(instruction: &Instruction) -> String {
    format!("{:08x}  {}", instruction.address, instruction.text)
}

/// Draws the view. Returns an address the reader asked to see in the listing.
pub fn show(app: &mut DesdecApp, ui: &mut egui::Ui) -> Option<u64> {
    let language = app.preferences.language;
    let analysis = app.analysis.as_ref()?;
    if app.functions.is_empty() {
        ui.label(egui::RichText::new(text(language, Text::NoFunctionSymbols)).color(MUTED));
        return None;
    }

    let chosen = chosen_function(app);
    let function = app
        .functions
        .iter()
        .find(|function| Some(function.start) == chosen)
        .or_else(|| app.functions.first())?;

    heading(ui, function, language);
    ui.add_space(8.0);

    let body = function.body(analysis);
    let blocks = desdec_core::blocks::of(body);
    if blocks.is_empty() {
        ui.label(egui::RichText::new(text(language, Text::NoFunctionBody)).color(MUTED));
        return None;
    }

    app.graph.follow(function.start);
    let sizes: Vec<egui::Vec2> = blocks
        .iter()
        .map(|block| size_of(ui, block, body))
        .collect();
    let placed = layout(&blocks, &sizes);
    // Where the run stands, so the block being executed is marked as the
    // listing marks it.
    let running = app
        .machine
        .as_ref()
        .map(desdec_core::emulate::Machine::instruction_pointer);

    // The surface is given the pan and zoom alone rather than the whole
    // application: what it draws is borrowed from the analysis, which the
    // application also holds.
    canvas(
        &mut app.graph,
        ui,
        &Drawing {
            blocks: &blocks,
            body,
            placed: &placed,
            running,
            language,
        },
    )
}

/// Everything the canvas needs about one function, gathered so the drawing
/// takes one argument rather than six.
struct Drawing<'a> {
    blocks: &'a [BasicBlock],
    body: &'a [Instruction],
    placed: &'a Layout,
    running: Option<u64>,
    language: Language,
}

/// The function the graph shows: the one selected in the functions view, or
/// the one holding the instruction selected in the listing.
fn chosen_function(app: &DesdecApp) -> Option<u64> {
    if let Some(selected) = app.selected_function {
        return Some(selected);
    }
    let address = app.selected_instruction?;
    app.functions
        .iter()
        .find(|function| (function.start..function.end).contains(&address))
        .map(|function| function.start)
}

/// How many characters of a function's name the heading shows.
const NAME_SHOWN: usize = 56;

/// A name cut to what the heading has room for, with an ellipsis when it was.
fn shortened(name: &str) -> String {
    if name.chars().count() <= NAME_SHOWN {
        return name.to_owned();
    }
    let kept: String = name.chars().take(NAME_SHOWN).collect();
    format!("{kept}…")
}

fn heading(ui: &mut egui::Ui, function: &Function, language: Language) {
    ui.horizontal(|ui| {
        // Shortened here rather than by egui: a truncating label in a
        // horizontal row takes all the width there is and pushes everything
        // beside it off the window, and a mangled Rust name runs to a hundred
        // characters. The whole of it stays one hover away.
        ui.strong(shortened(&function.name))
            .on_hover_text(&function.name);
        ui.label(egui::RichText::new(format!("{:#018x}", function.start)).monospace());
        ui.separator();
        ui.label(egui::RichText::new(text(language, Text::GraphHelp)).color(MUTED));
    });
}

/// The pannable, zoomable surface the graph is drawn on.
fn canvas(view: &mut View, ui: &mut egui::Ui, drawing: &Drawing<'_>) -> Option<u64> {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ui.available_height().max(200.0)),
        egui::Sense::click_and_drag(),
    );

    if response.dragged() {
        view.offset += response.drag_delta();
    }
    // Zoom about the pointer, so the block being looked at stays under it.
    if response.hovered() {
        let scroll = ui.ctx().input(|input| input.smooth_scroll_delta.y);
        if scroll.abs() > f32::EPSILON
            && let Some(pointer) = response.hover_pos()
        {
            let previous = view.zoom;
            let zoom = (previous * (1.0 + scroll * 0.002)).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
            let anchor = pointer - rect.min - view.offset;
            view.offset += anchor - anchor * (zoom / previous);
            view.zoom = zoom;
        }
    }
    if view.centre_on_entry {
        // The entry block, brought under the top middle of the surface.
        if let Some(entry) = drawing.placed.of_block(0) {
            view.offset =
                egui::vec2(rect.width() / 2.0 - entry.rect.center().x, 24.0);
        }
        view.centre_on_entry = false;
    }

    let zoom = view.zoom;
    let offset = view.offset;
    let to_screen = |point: egui::Pos2| rect.min + offset + point.to_vec2() * zoom;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, ui.style().visuals.extreme_bg_color);

    edges(&painter, drawing, &to_screen, zoom);
    let mut go_to = None;
    for node in &drawing.placed.nodes {
        let block = &drawing.blocks[node.block];
        let screen = egui::Rect::from_min_size(
            to_screen(node.rect.min),
            node.rect.size() * zoom,
        );
        if !rect.intersects(screen) {
            continue; // Off the surface: not drawn, and not interacted with.
        }
        if let Some(address) = node_of(ui, &painter, drawing, block, screen, zoom) {
            go_to = Some(address);
        }
    }
    go_to
}

/// Draws one block, and answers with the address the reader clicked in it.
fn node_of(
    ui: &egui::Ui,
    painter: &egui::Painter,
    drawing: &Drawing<'_>,
    block: &BasicBlock,
    screen: egui::Rect,
    zoom: f32,
) -> Option<u64> {
    let visuals = ui.style().visuals.clone();
    let holds_the_run = drawing.running.is_some_and(|address| {
        drawing
            .body
            .get(block.instructions.clone())
            .unwrap_or_default()
            .iter()
            .any(|instruction| instruction.address == address)
    });
    let border = if holds_the_run {
        egui::Stroke::new(2.0_f32, machine::CURRENT)
    } else {
        visuals.window_stroke
    };
    painter.rect_filled(screen, 4.0, visuals.faint_bg_color);
    painter.rect_stroke(screen, 4.0, border, egui::StrokeKind::Inside);

    let font = egui::FontId::monospace(11.0 * zoom);
    let mut y = screen.top() + PADDING * zoom;
    let left = screen.left() + PADDING * zoom;
    painter.text(
        egui::pos2(left, y),
        egui::Align2::LEFT_TOP,
        format!("{:#x}", block.start),
        egui::FontId::proportional(11.0 * zoom),
        MUTED,
    );
    y += LINE * zoom;

    let body = drawing
        .body
        .get(block.instructions.clone())
        .unwrap_or_default();
    for instruction in body.iter().take(LINES_SHOWN) {
        let colour = if drawing.running == Some(instruction.address) {
            machine::CURRENT
        } else {
            visuals.text_color()
        };
        painter.text(
            egui::pos2(left, y),
            egui::Align2::LEFT_TOP,
            line_of(instruction),
            font.clone(),
            colour,
        );
        y += LINE * zoom;
    }
    if block.instruction_count() > LINES_SHOWN {
        painter.text(
            egui::pos2(left, y),
            egui::Align2::LEFT_TOP,
            format!(
                "… {} {}",
                block.instruction_count() - LINES_SHOWN,
                text(drawing.language, Text::MoreInstructions)
            ),
            egui::FontId::proportional(11.0 * zoom),
            MUTED,
        );
        y += LINE * zoom;
    }
    // How the flow leaves, when there is no arrow to say it. A return and a
    // branch through a register are said differently on purpose: one goes
    // somewhere perfectly well known, and only the other is a limit of what
    // could be read.
    if let Some(said) = match block.exit {
        Exit::Onwards => None,
        Exit::Returns => Some(Text::BlockReturns),
        Exit::Unstated => Some(Text::FlowLeavesHere),
    } {
        painter.text(
            egui::pos2(left, y),
            egui::Align2::LEFT_TOP,
            text(drawing.language, said),
            egui::FontId::proportional(11.0 * zoom),
            MUTED,
        );
    }

    let response = ui.interact(
        screen,
        ui.id().with(("graph-block", block.start)),
        egui::Sense::click(),
    );
    if response.clicked() {
        return Some(block.start);
    }
    None
}

/// Draws every arrow between blocks.
fn edges(
    painter: &egui::Painter,
    drawing: &Drawing<'_>,
    to_screen: &impl Fn(egui::Pos2) -> egui::Pos2,
    zoom: f32,
) {
    for node in &drawing.placed.nodes {
        let block = &drawing.blocks[node.block];
        for successor in &block.successors {
            let Some(target) = drawing
                .blocks
                .iter()
                .position(|other| other.start == successor.address)
                .and_then(|index| drawing.placed.of_block(index))
            else {
                continue;
            };
            let colour = match successor.edge {
                Edge::Taken => TAKEN,
                Edge::NotTaken => NOT_TAKEN,
                Edge::Jump | Edge::FallThrough => PLAIN,
            };
            arrow(painter, node, target, colour, to_screen, zoom);
        }
    }
}

/// One arrow, from the bottom of a block to the top of another.
///
/// A back edge — an arrow to a block at the same rank or above, which is what
/// closes a loop — is taken out to the side rather than drawn straight through
/// everything between: a loop drawn as a line over its own body is a line a
/// reader cannot follow.
fn arrow(
    painter: &egui::Painter,
    from: &Placed,
    to: &Placed,
    colour: egui::Color32,
    to_screen: &impl Fn(egui::Pos2) -> egui::Pos2,
    zoom: f32,
) {
    let stroke = egui::Stroke::new(1.4 * zoom, colour);
    let start = egui::pos2(from.rect.center().x, from.rect.bottom());
    let end = egui::pos2(to.rect.center().x, to.rect.top());
    let backwards = to.rank <= from.rank;

    let points: Vec<egui::Pos2> = if backwards {
        let side = from.rect.right().max(to.rect.right()) + COLUMN_GAP / 2.0;
        vec![
            egui::pos2(from.rect.right(), from.rect.center().y),
            egui::pos2(side, from.rect.center().y),
            egui::pos2(side, to.rect.center().y),
            egui::pos2(to.rect.right(), to.rect.center().y),
        ]
    } else {
        let middle = f32::midpoint(start.y, end.y);
        vec![
            start,
            egui::pos2(start.x, middle),
            egui::pos2(end.x, middle),
            end,
        ]
    };
    let screen: Vec<egui::Pos2> = points.iter().map(|point| to_screen(*point)).collect();
    for pair in screen.windows(2) {
        painter.line_segment([pair[0], pair[1]], stroke);
    }
    if let [.., before, tip] = screen.as_slice() {
        head(painter, *before, *tip, colour, zoom);
    }
}

/// The head of an arrow, pointing the way the last segment runs.
fn head(
    painter: &egui::Painter,
    from: egui::Pos2,
    tip: egui::Pos2,
    colour: egui::Color32,
    zoom: f32,
) {
    let direction = (tip - from).normalized();
    if !direction.x.is_finite() || !direction.y.is_finite() {
        return;
    }
    let back = tip - direction * 7.0 * zoom;
    let wing = egui::vec2(-direction.y, direction.x) * 4.0 * zoom;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, back + wing, back - wing],
        colour,
        egui::Stroke::NONE,
    ));
}

/// Draws the view and moves the workspace where the reader asked.
pub fn show_in(app: &mut DesdecApp, ui: &mut egui::Ui) {
    if let Some(address) = show(app, ui) {
        let ctx = ui.ctx().clone();
        app.selected_instruction = Some(address);
        app.active_view = WorkspaceView::Disassembly;
        app.go_to_address(&ctx, address);
    }
}

#[cfg(test)]
mod tests {
    use super::{COLUMN_GAP, Layout, layout};
    use desdec_core::{
        Instruction, InstructionBytes,
        blocks::{self, BasicBlock},
    };
    use eframe::egui;

    fn body(lines: &[(u64, &str)]) -> Vec<Instruction> {
        lines
            .iter()
            .map(|(address, text)| Instruction {
                address: *address,
                bytes: InstructionBytes::new(&[0x90]).expect("one byte"),
                text: (*text).to_owned(),
                section: std::sync::Arc::from(".text"),
            })
            .collect()
    }

    /// Every block the same size, so what the placement does is the only thing
    /// the assertions can be reading.
    fn placed(blocks: &[BasicBlock]) -> Layout {
        let sizes = vec![egui::vec2(200.0, 60.0); blocks.len()];
        layout(blocks, &sizes)
    }

    /// A test and its two arms: the arms sit side by side, one rank below the
    /// block that chooses between them.
    #[test]
    fn the_two_arms_of_a_test_are_drawn_side_by_side_below_it() {
        let blocks = blocks::of(&body(&[
            (0x10, "cmp rax, 0"),
            (0x14, "jne 0x20"),
            (0x18, "mov rax, 1"),
            (0x1c, "ret"),
            (0x20, "mov rax, 2"),
            (0x24, "ret"),
        ]));
        let placed = placed(&blocks);
        assert_eq!(placed.nodes.len(), 3);
        let test = placed.nodes[0];
        assert_eq!(test.rank, 0);
        let arms: Vec<_> = placed.nodes.iter().filter(|node| node.rank == 1).collect();
        assert_eq!(arms.len(), 2, "both arms one rank below");
        assert!(
            (arms[0].rect.top() - arms[1].rect.top()).abs() < f32::EPSILON,
            "and level with each other"
        );
        assert!(
            arms[0].rect.bottom() > test.rect.bottom(),
            "below the block that chooses between them"
        );
    }

    /// Nothing may be drawn on top of anything else: two blocks sharing a
    /// rectangle is a graph that cannot be read, and no assertion about ranks
    /// alone would notice.
    #[test]
    fn no_two_blocks_are_placed_over_one_another() {
        let blocks = blocks::of(&body(&[
            (0x10, "cmp rax, 0"),
            (0x14, "jne 0x30"),
            (0x18, "cmp rbx, 0"),
            (0x1c, "je 0x28"),
            (0x20, "mov rax, 1"),
            (0x24, "jmp 0x34"),
            (0x28, "mov rax, 2"),
            (0x2c, "jmp 0x34"),
            (0x30, "mov rax, 3"),
            (0x34, "ret"),
        ]));
        let placed = placed(&blocks);
        assert!(placed.nodes.len() >= 4);
        for (index, node) in placed.nodes.iter().enumerate() {
            for other in &placed.nodes[index + 1..] {
                assert!(
                    !node.rect.intersects(other.rect),
                    "blocks {} and {} overlap: {:?} and {:?}",
                    node.block,
                    other.block,
                    node.rect,
                    other.rect
                );
            }
        }
    }

    /// The arrow that closes a loop must not push its target down the graph:
    /// a loop body drawn below its own back edge reads as a chain.
    #[test]
    fn a_back_edge_does_not_push_the_block_it_returns_to_further_down() {
        let blocks = blocks::of(&body(&[
            (0x10, "mov rcx, 10"),
            (0x14, "dec rcx"),
            (0x18, "jne 0x14"),
            (0x1c, "ret"),
        ]));
        let placed = placed(&blocks);
        let top = placed
            .nodes
            .iter()
            .find(|node| blocks[node.block].start == 0x10)
            .expect("the block the function starts at");
        let looping = placed
            .nodes
            .iter()
            .find(|node| blocks[node.block].start == 0x14)
            .expect("the body of the loop");
        assert_eq!(top.rank, 0);
        assert_eq!(looping.rank, 1, "one step from the entry, not two");
    }

    /// A block nothing reaches is still drawn, below the rest.
    #[test]
    fn a_block_nothing_reaches_is_placed_rather_than_dropped() {
        let blocks = blocks::of(&body(&[
            (0x10, "jmp 0x18"),
            (0x14, "mov rax, 9"),
            (0x18, "ret"),
        ]));
        let placed = placed(&blocks);
        assert_eq!(
            placed.nodes.len(),
            blocks.len(),
            "every block is somewhere on the surface"
        );
    }

    /// The blocks of one rank are spaced, not butted together.
    #[test]
    fn blocks_sharing_a_rank_are_given_room_between_them() {
        let blocks = blocks::of(&body(&[
            (0x10, "cmp rax, 0"),
            (0x14, "jne 0x20"),
            (0x18, "mov rax, 1"),
            (0x1c, "ret"),
            (0x20, "mov rax, 2"),
            (0x24, "ret"),
        ]));
        let placed = placed(&blocks);
        let mut arms: Vec<_> = placed
            .nodes
            .iter()
            .filter(|node| node.rank == 1)
            .map(|node| node.rect)
            .collect();
        arms.sort_by(|left, right| left.left().total_cmp(&right.left()));
        assert!((arms[1].left() - arms[0].right() - COLUMN_GAP).abs() < 0.5);
    }

    /// A mangled Rust name runs to a hundred characters; the heading shows
    /// what it has room for and says that it did.
    #[test]
    fn a_long_name_is_shortened_and_says_so() {
        let long = "_RNvMs4_NtCscdodAO9FK5_5alloc7raw_vecNtB5_11RawVecInner11finish_growCstuaXukgBIa_10proc_macro";
        let shown = super::shortened(long);
        assert!(shown.chars().count() <= super::NAME_SHOWN + 1);
        assert!(shown.ends_with('…'));
        assert_eq!(super::shortened("main"), "main", "a short name is left alone");
    }

    #[test]
    fn a_function_with_no_blocks_places_nothing() {
        assert_eq!(layout(&[], &[]), Layout::default());
    }
}
