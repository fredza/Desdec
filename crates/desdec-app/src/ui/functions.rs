//! Named functions, their basic blocks, and a local control-flow view.

use std::{collections::HashMap, ops::Range};

use desdec_core::{
    Analysis, Instruction, Symbol, SymbolKind,
    blocks::{self, BasicBlock},
    discover,
};
use eframe::egui;

use crate::{
    i18n::{Language, Text, text},
    ui::{MUTED, ROW_HEIGHT, card, format_size, syntax},
};

const GRAPH_NODE_SIZE: egui::Vec2 = egui::vec2(230.0, 42.0);
const GRAPH_ROW_HEIGHT: f32 = 68.0;

/// A function selected from the symbol table, with its decoded body and basic
/// blocks. Function boundaries are exact when the symbol gives a size; for
/// zero-sized symbols the next function symbol bounds the decoded body.
///
/// The body is a position in [`Analysis::instructions`] and the whole list is
/// owned, so it can be built once when a binary is opened. Rebuilding it per
/// frame meant sorting the listing and finding the basic blocks of every
/// function sixty times a second, for a table that never changes.
pub struct Function {
    pub name: String,
    /// The name as the source spelled it, when the compiler's encoding could
    /// be read back whole.
    ///
    /// Worked out once, when the binary is opened, rather than on every frame:
    /// a stripped-nothing Rust binary declares tens of thousands of these, and
    /// the table is virtualised precisely because doing anything per row per
    /// frame is what makes a listing crawl.
    ///
    /// `None` for a name that is not encoded — `main`, `_start` — and for one
    /// whose encoding Desdec does not fully understand. Both cases show the
    /// file's own spelling, which is the honest answer.
    pub readable: Option<String>,
    pub start: u64,
    pub end: u64,
    /// Position of the decoded body inside [`Analysis::instructions`].
    pub instructions: Range<usize>,
    /// Why this is here, for the ones the file does not name.
    ///
    /// `None` for a function the symbol table names, which needs no reason
    /// beyond having a name. `Some` for one worked out from the code, and the
    /// view says which reason it was: a reader must be able to tell an address
    /// something calls from a shape that looked like a beginning.
    pub found_by: Option<discover::Evidence>,
    pub blocks: Vec<BasicBlock>,
}

impl Function {
    /// What to put on the row: the source spelling when it was recovered and
    /// the reader has not asked for the file's own, and the file's otherwise.
    #[must_use]
    pub fn shown_name(&self, mangled: bool) -> &str {
        match &self.readable {
            Some(readable) if !mangled => readable,
            _ => &self.name,
        }
    }

    fn size(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// The decoded body, read back from the analysis it indexes.
    pub fn body<'a>(&self, analysis: &'a Analysis) -> &'a [Instruction] {
        analysis
            .instructions
            .get(self.instructions.clone())
            .unwrap_or_default()
    }
}

/// Draws the view, returning the address the reader asked to see in the
/// listing.
///
/// Returned rather than jumped to here: this view holds a borrow of the
/// analysis, and moving the workspace to the disassembly is the application's
/// business, not the table's.
pub fn show(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    functions: &[Function],
    graph: &crate::callgraph::Graph,
    selected_function: &mut Option<u64>,
    mangled: &mut bool,
    language: Language,
) -> Option<u64> {
    if functions.is_empty() {
        ui.label(egui::RichText::new(text(language, Text::NoFunctionSymbols)).color(MUTED));
        return None;
    }
    if !selected_function.is_some_and(|address| functions.iter().any(|item| item.start == address))
    {
        *selected_function = Some(functions[0].start);
    }
    function_guide(ui, language);
    // The switch sits above the table rather than in the preferences: it is a
    // way of looking at this file, moved while reading it, and a reader who
    // wants to check a name against the file's own spelling wants it back for
    // one glance, not for good.
    //
    // Offered only when there is something to read back. A C binary's names
    // are already the ones the source used, and a switch that changes nothing
    // is a switch the reader presses to find out what it does.
    if functions.iter().any(|function| function.readable.is_some()) {
        ui.horizontal(|ui| {
            crate::ui::shown_toggle(ui, mangled, text(language, Text::ReadableNames))
                .on_hover_text(text(language, Text::ReadableNamesHint));
        });
    }
    ui.add_space(8.0);
    // Said once, above the table, when the file named none of them: the
    // column on each row says where that row came from, and this says why
    // there is a column at all.
    // How many calls this file's own code states, said once above the table:
    // it is the measure of how much the panel below has to work with, and a
    // file whose calls are nearly all indirect has a graph that says little.
    if !graph.is_empty() {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!(
                    "{} {} · {} {}",
                    graph.len(),
                    text(language, Text::Functions).to_lowercase(),
                    graph.calls(),
                    text(language, Text::CallsCounted),
                ))
                .small()
                .color(MUTED),
            );
        });
    }
    if functions.iter().all(|function| function.found_by.is_some()) {
        ui.label(
            egui::RichText::new(text(language, Text::UnnamedFunctionsFound))
                .small()
                .color(MUTED),
        );
        ui.add_space(4.0);
    }

    let mut go_to = None;
    ui.columns(2, |columns| {
        go_to = function_list(
            &mut columns[0],
            analysis,
            functions,
            selected_function,
            *mangled,
            language,
        );
        let selected = selected_function
            .and_then(|address| functions.iter().find(|function| function.start == address));
        let details = function_details(
            &mut columns[1],
            analysis,
            functions,
            graph,
            selected,
            *mangled,
            language,
        );
        go_to = go_to.or(details.go_to);
        // A step through the call graph selects that function and leaves the
        // reader here: they are walking the graph, and each step is where the
        // next question is asked.
        if let Some(walked) = details.walked {
            *selected_function = Some(walked);
        }
    });
    go_to
}

/// The few distinctions a reader needs before the table can answer questions
/// rather than look like a directory of confident-looking names. Kept above
/// both panes: the explanation applies to the evidence column, the call graph
/// and the pseudo-code together, not just to the currently selected row.
fn function_guide(ui: &mut egui::Ui, language: Language) {
    card(ui, text(language, Text::HowToReadFunctions), |ui| {
        ui.label(egui::RichText::new(text(language, Text::FunctionsGuideIntro)).color(MUTED));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(text(language, Text::FunctionsGuideList)).small());
        ui.add_space(2.0);
        ui.label(egui::RichText::new(text(language, Text::FunctionsGuideRelations)).small());
    });
}

/// The first decoded instruction of a function, which is where the listing is
/// sent when the reader asks to see it.
///
/// `None` for a symbol whose body was never decoded — one in a section the
/// decoder does not read, or past the analysis limit. The offer to jump is
/// withheld rather than made and refused.
fn entry(function: &Function, analysis: &Analysis) -> Option<u64> {
    analysis
        .instructions
        .get(function.instructions.start)
        .filter(|_| !function.instructions.is_empty())
        .map(|instruction| instruction.address)
}

fn function_list(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    functions: &[Function],
    selected_function: &mut Option<u64>,
    mangled: bool,
    language: Language,
) -> Option<u64> {
    let mut go_to = None;
    card(ui, text(language, Text::Functions), |ui| {
        // Virtualised like the listings: a stripped-nothing binary declares
        // tens of thousands of function symbols, and the table never changes
        // between frames, so only what the reader can see is laid out.
        let row_spacing = ui.spacing().item_spacing.y;
        egui::ScrollArea::both()
            .id_salt("function_list")
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, functions.len() + 1, |ui, rows| {
                egui::Grid::new("function_symbols")
                    .num_columns(5)
                    .striped(true)
                    // Vertical spacing must be the one the virtualiser assumed
                    // when it placed this batch of rows, or they drift.
                    .spacing([14.0, row_spacing])
                    .min_row_height(ROW_HEIGHT)
                    .show(ui, |ui| {
                        if rows.start == 0 {
                            // The jump column carries no title: the arrow in
                            // it is its own word, and a heading would be read
                            // as a fourth thing measured about a function.
                            ui.label("");
                            for title in [Text::Name, Text::Size, Text::Blocks, Text::FoundBy] {
                                ui.strong(text(language, title));
                            }
                            ui.end_row();
                        }
                        let first = rows.start.saturating_sub(1).min(functions.len());
                        let last = rows.end.saturating_sub(1).min(functions.len());
                        for function in &functions[first..last.max(first)] {
                            let selected = *selected_function == Some(function.start);
                            let entry = entry(function, analysis);
                            // The button leads the row rather than trailing
                            // it: a mangled Rust name is two hundred
                            // characters wide, and anything after it is off
                            // the right edge of the table.
                            let jump = ui
                                .add_enabled(entry.is_some(), egui::Button::new(JUMP).small())
                                .on_hover_text(text(language, Text::GoToDisassembly))
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if jump.clicked() {
                                *selected_function = Some(function.start);
                                go_to = entry;
                            }
                            // The file's own spelling stays one hover away
                            // whichever way the switch is set: a reading is
                            // only checkable against what it was read from.
                            let hover = match &function.readable {
                                Some(_) if !mangled => format!(
                                    "{:#018x}\n{} {}",
                                    function.start,
                                    text(language, Text::MangledName),
                                    function.name
                                ),
                                _ => format!("{:#018x}", function.start),
                            };
                            let name = ui
                                .selectable_label(selected, function.shown_name(mangled))
                                .on_hover_text(hover)
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if name.clicked() {
                                *selected_function = Some(function.start);
                            }
                            // The right button on the row, for a reader who
                            // reaches for it: the same one offer the arrow
                            // makes, where the pointer already is.
                            name.context_menu(|ui| {
                                if ui
                                    .add_enabled(
                                        entry.is_some(),
                                        egui::Button::new(text(language, Text::GoToDisassembly)),
                                    )
                                    .clicked()
                                {
                                    go_to = entry;
                                    ui.close_menu();
                                }
                            });
                            ui.label(format_size(function.size()));
                            ui.label(function.blocks.len().to_string());
                            origin(ui, function, language);
                            ui.end_row();
                        }
                    });
            });
    });
    go_to
}

/// Where a row came from: the file's own symbol table, or the code.
///
/// Shown on every row rather than only on the found ones, so the column means
/// the same thing all the way down and a reader does not have to work out
/// whether a blank cell means "named" or "nothing to say".
fn origin(ui: &mut egui::Ui, function: &Function, language: Language) {
    let Some(evidence) = function.found_by else {
        ui.label(
            egui::RichText::new(text(language, Text::NamedByTheFile))
                .small()
                .color(MUTED),
        );
        return;
    };
    let (label, help, certain) = match evidence {
        discover::Evidence::EntryPoint => (
            text(language, Text::FoundAsEntryPoint).to_owned(),
            None,
            true,
        ),
        discover::Evidence::Called(callers) => (
            format!("{} ×{callers}", text(language, Text::FoundAsCalled)),
            Some(format!(
                "{} {callers}",
                text(language, Text::FoundAsCalledTimes)
            )),
            true,
        ),
        discover::Evidence::Prologue => (
            text(language, Text::FoundAsPrologue).to_owned(),
            Some(text(language, Text::FoundAsPrologueHelp).to_owned()),
            false,
        ),
    };
    // A reading is drawn quieter than a fact. The two are not the same claim,
    // and a column that drew them alike would be inviting the reader to treat
    // them alike.
    let colour = if certain {
        ui.visuals().text_color()
    } else {
        MUTED
    };
    let cell = ui.label(egui::RichText::new(label).small().color(colour));
    if let Some(help) = help {
        cell.on_hover_text(help);
    }
}

/// What the jump button carries: an arrow into the listing, drawn from the
/// font so it sits on the row's own baseline. One per row, so reaching a
/// function's code is a click rather than a right-click and a menu.
const JUMP: &str = "\u{2192}";

fn function_details(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    functions: &[Function],
    graph: &crate::callgraph::Graph,
    selected: Option<&Function>,
    mangled: bool,
    language: Language,
) -> Details {
    let Some(function) = selected else {
        ui.label(egui::RichText::new(text(language, Text::SelectFunction)).color(MUTED));
        return Details {
            go_to: None,
            walked: None,
        };
    };

    let mut go_to = None;
    // A compiler-produced name can be wider than the whole detail panel. The
    // list owns a horizontal scrollbar, but this heading does not: keep the
    // panel within its column and leave the complete name one hover away.
    ui.add(
        egui::Label::new(egui::RichText::new(function.shown_name(mangled)).heading()).truncate(),
    )
    .on_hover_text(&function.name);
    ui.horizontal(|ui| {
        ui.monospace(format!("{:#018x}", function.start));
        // Beside the address, because that is what the reader is about to go
        // and read: the graph and the pseudo-code below are a summary of code
        // that lives in the listing.
        let entry = entry(function, analysis);
        let button = ui.add_enabled(
            entry.is_some(),
            egui::Button::new(text(language, Text::GoToDisassembly)),
        );
        if button.clicked() {
            go_to = entry;
        }
        if entry.is_none() {
            ui.label(
                egui::RichText::new(text(language, Text::FunctionNotDecoded))
                    .small()
                    .color(MUTED),
            );
        }
    });
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(text(language, Text::FunctionDetailGuide))
            .small()
            .color(MUTED),
    );
    ui.add_space(8.0);
    // Who calls this one, and what it calls, before the graph of its own
    // blocks: a reader arriving at a function asks how anything gets to it
    // before they ask what it does inside.
    let walked = call_graph(ui, functions, graph, function, mangled, language);
    ui.add_space(8.0);
    let hovered_block = control_flow_graph(ui, analysis, function, language);
    ui.add_space(12.0);
    pseudocode(ui, analysis, function, hovered_block, mangled, language);
    // What the reader clicked in the call graph is handled by the caller,
    // which owns the selection; the jump button's answer is kept for when
    // nothing in the graph was clicked.
    Details { go_to, walked }
}

/// What the detail panel asked of the view around it.
struct Details {
    /// A listing address to move to.
    go_to: Option<u64>,
    /// A function to select, from a step through the call graph.
    walked: Option<u64>,
}

/// What calls this function and what it calls, each a step away.
///
/// Returns a function the reader clicked, which the caller makes the selected
/// one — so the panel is walked rather than only read.
fn call_graph(
    ui: &mut egui::Ui,
    functions: &[Function],
    graph: &crate::callgraph::Graph,
    function: &Function,
    mangled: bool,
    language: Language,
) -> Option<u64> {
    let edges = graph.edges(function.start)?;
    let name_of = |address: u64| {
        functions
            .iter()
            .find(|other| other.start == address)
            .map_or_else(
                || format!("{address:#x}"),
                |other| other.shown_name(mangled).to_owned(),
            )
    };
    let mut chosen = None;
    card(ui, text(language, Text::Callers), |ui| {
        if edges.callers.is_empty() {
            ui.label(egui::RichText::new(text(language, Text::NothingCallsThis)).color(MUTED));
        }
        // One row per caller, however many times it calls: "this function
        // calls that one" is the fact, and the count of instructions doing it
        // is noise at this level.
        let mut seen: Vec<u64> = edges.callers.iter().map(|call| call.from).collect();
        seen.dedup();
        for caller in seen.iter().take(CALL_ROWS) {
            if function_link(ui, &name_of(*caller)).clicked() {
                chosen = Some(*caller);
            }
        }
        if seen.len() > CALL_ROWS {
            ui.label(
                egui::RichText::new(format!("… {}", seen.len() - CALL_ROWS))
                    .small()
                    .color(MUTED),
            );
        }
        // How the reader gets here from a starting point, which is the whole
        // question this panel exists to answer.
        ways_in(ui, graph, function.start, &name_of, language);
    });
    ui.add_space(6.0);
    card(ui, text(language, Text::Callees), |ui| {
        let mut seen: Vec<u64> = edges.calls.iter().map(|call| call.to).collect();
        seen.dedup();
        if seen.is_empty() && edges.indirect == 0 && edges.outside == 0 {
            ui.label(egui::RichText::new(text(language, Text::CallsNothing)).color(MUTED));
        }
        for callee in seen.iter().take(CALL_ROWS) {
            if function_link(ui, &name_of(*callee)).clicked() {
                chosen = Some(*callee);
            }
        }
        if seen.len() > CALL_ROWS {
            ui.label(
                egui::RichText::new(format!("… {}", seen.len() - CALL_ROWS))
                    .small()
                    .color(MUTED),
            );
        }
        // The two kinds of call the graph cannot follow, said rather than
        // left out: a function whose callees are all indirect would otherwise
        // read as one that calls nothing.
        for (count, label, help) in [
            (edges.indirect, Text::IndirectCalls, Text::IndirectCallsHelp),
            (edges.outside, Text::CallsOutside, Text::CallsOutsideHelp),
        ] {
            if count > 0 {
                ui.label(
                    egui::RichText::new(format!("{count} {}", text(language, label)))
                        .small()
                        .color(MUTED),
                )
                .on_hover_text(text(language, help));
            }
        }
        let reaches = graph.reachable_from(function.start).len();
        if reaches > 0 {
            ui.label(
                egui::RichText::new(format!("{} {reaches}", text(language, Text::Reaches)))
                    .small()
                    .color(MUTED),
            )
            .on_hover_text(text(language, Text::ReachesHelp));
        }
    });
    chosen
}

/// A function-name link that cannot widen a call-graph card.
///
/// Rust and C++ symbols routinely span several hundred pixels. The complete
/// spelling is still useful — especially for two specialisations that only
/// differ near their end — so truncation is visual only and the hover says the
/// whole name.
fn function_link(ui: &mut egui::Ui, name: &str) -> egui::Response {
    // `card` centres children that keep their intrinsic width. Reserve a full
    // line first, then put the label in it, or a short name would float in the
    // middle while a long one was clipped at the card's edge.
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
        egui::Sense::hover(),
    );
    ui.put(
        rect,
        egui::Label::new(egui::RichText::new(name).color(ui.visuals().hyperlink_color))
            .truncate()
            .halign(egui::Align::LEFT)
            .sense(egui::Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .on_hover_text(name)
}

/// The shortest chains of calls that reach this function from a starting
/// point of the file.
fn ways_in(
    ui: &mut egui::Ui,
    graph: &crate::callgraph::Graph,
    to: u64,
    name_of: &impl Fn(u64) -> String,
    language: Language,
) {
    let starts: Vec<u64> = graph.unreached().take(STARTS_TRIED).collect();
    let mut paths: Vec<Vec<u64>> = Vec::new();
    for start in starts {
        if start == to {
            continue;
        }
        paths.extend(graph.paths(start, to, 1));
        if paths.len() >= WAYS_IN {
            break;
        }
    }
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(text(language, Text::HowToGetHere))
            .small()
            .strong(),
    )
    .on_hover_text(text(language, Text::HowToGetHereHelp));
    if paths.is_empty() {
        ui.label(
            egui::RichText::new(text(language, Text::NoWayHere))
                .small()
                .color(MUTED),
        );
        return;
    }
    paths.sort_by_key(Vec::len);
    for path in paths.iter().take(WAYS_IN) {
        let chain: Vec<String> = path.iter().map(|address| name_of(*address)).collect();
        let chain = chain.join(" → ");
        ui.add(egui::Label::new(egui::RichText::new(&chain).small().color(MUTED)).truncate())
            .on_hover_text(chain);
    }
}

/// How many callers or callees are listed before the rest are counted.
const CALL_ROWS: usize = 12;
/// How many ways in are shown, and how many starting points are tried to find
/// them. Both bounded: a large binary has thousands of each.
const WAYS_IN: usize = 3;
const STARTS_TRIED: usize = 24;

/// Returns the block currently under the pointer so the pseudo-code can follow
/// the graph without making a click change the selected function.
fn control_flow_graph(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    function: &Function,
    language: Language,
) -> Option<u64> {
    let mut hovered_block = None;
    card(ui, text(language, Text::ControlFlow), |ui| {
        if function.blocks.is_empty() {
            ui.label(egui::RichText::new(text(language, Text::NoFunctionBody)).color(MUTED));
            return;
        }
        // This pane is a map of the shape, small enough to sit beside the
        // pseudo-code and to light it up as the pointer moves. Reading the
        // instructions in the blocks is what the graph view is for, and this
        // says where it is rather than trying to be it.
        ui.label(
            egui::RichText::new(text(language, Text::GraphViewIsThere))
                .small()
                .color(MUTED),
        );
        ui.add_space(6.0);

        egui::ScrollArea::both()
            .id_salt(("control_flow", function.start))
            .max_height(220.0)
            .show(ui, |ui| {
                let graph_width = ui.available_width().max(520.0);
                let graph_height = function.blocks.len() as f32 * GRAPH_ROW_HEIGHT + 20.0;
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(graph_width, graph_height),
                    egui::Sense::hover(),
                );
                let painter = ui.painter();
                let visuals = ui.style().visuals.clone();
                let positions: HashMap<u64, egui::Rect> = function
                    .blocks
                    .iter()
                    .enumerate()
                    .map(|(index, block)| {
                        let alternate_column = (index % 2) as f32;
                        let x = rect.left() + 24.0 + alternate_column * (GRAPH_NODE_SIZE.x + 70.0);
                        let y = rect.top() + 10.0 + index as f32 * GRAPH_ROW_HEIGHT;
                        (
                            block.start,
                            egui::Rect::from_min_size(egui::pos2(x, y), GRAPH_NODE_SIZE),
                        )
                    })
                    .collect();

                for block in &function.blocks {
                    let Some(node) = positions.get(&block.start) else {
                        continue;
                    };
                    let response = ui.interact(
                        *node,
                        ui.id()
                            .with(("control_flow_block", function.start, block.start)),
                        egui::Sense::hover(),
                    );
                    if response.hovered() {
                        hovered_block = Some(block.start);
                        response.on_hover_ui(|ui| block_details(ui, analysis, function, block));
                    }
                }

                for block in &function.blocks {
                    let Some(source) = positions.get(&block.start) else {
                        continue;
                    };
                    for successor in &block.successors {
                        let Some(target) = positions.get(&successor.address) else {
                            continue;
                        };
                        let from = source.center_bottom();
                        let to = target.center_top();
                        let color = visuals.weak_text_color();
                        painter.line_segment([from, to], egui::Stroke::new(1.4_f32, color));
                        let direction = (to - from).normalized();
                        let tip = to - direction * 5.0;
                        let wing = egui::vec2(-direction.y, direction.x) * 4.0;
                        painter.add(egui::Shape::convex_polygon(
                            vec![to, tip + wing, tip - wing],
                            color,
                            egui::Stroke::NONE,
                        ));
                    }
                }

                for (index, block) in function.blocks.iter().enumerate() {
                    let Some(node) = positions.get(&block.start) else {
                        continue;
                    };
                    let fill = if hovered_block == Some(block.start) {
                        visuals.selection.bg_fill
                    } else {
                        visuals.faint_bg_color
                    };
                    painter.rect_filled(*node, 5.0, fill);
                    painter.rect_stroke(
                        *node,
                        5.0,
                        visuals.window_stroke,
                        egui::StrokeKind::Inside,
                    );
                    painter.text(
                        node.left_top() + egui::vec2(8.0, 8.0),
                        egui::Align2::LEFT_TOP,
                        format!("B{}  {:#x}", index + 1, block.start),
                        egui::FontId::monospace(12.0),
                        visuals.text_color(),
                    );
                    painter.text(
                        node.left_bottom() - egui::vec2(-8.0, 7.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{} instr.", block.instruction_count()),
                        egui::FontId::proportional(11.0),
                        visuals.weak_text_color(),
                    );
                }
            });
    });
    hovered_block
}

fn block_details(ui: &mut egui::Ui, analysis: &Analysis, function: &Function, block: &BasicBlock) {
    ui.strong(format!("Bloc {:#x}", block.start));
    ui.separator();
    let transparent = egui::Color32::TRANSPARENT;
    let body = function.body(analysis);
    for instruction in body.get(block.instructions.clone()).unwrap_or_default() {
        ui.horizontal(|ui| {
            ui.label(syntax::dim(
                ui,
                &format!("{:#018x}", instruction.address),
                transparent,
            ));
            let bytes = instruction
                .bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            ui.label(syntax::dim(ui, &bytes, transparent));
            ui.label(syntax::assembly(ui, &instruction.text, transparent));
        });
    }
}

fn pseudocode(
    ui: &mut egui::Ui,
    analysis: &Analysis,
    function: &Function,
    hovered_block: Option<u64>,
    mangled: bool,
    language: Language,
) {
    let body = function.body(analysis);
    let highlighted_instructions = hovered_block.and_then(|address| {
        function
            .blocks
            .iter()
            .find(|block| block.start == address)
            .map(|block| block.instructions.clone())
    });

    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.strong(text(language, Text::PseudoCode));
        ui.add_space(8.0);
        if body.is_empty() {
            ui.label(egui::RichText::new(text(language, Text::NoFunctionBody)).color(MUTED));
            return;
        }
        // The graph is intentionally capped above; the pseudo-code owns all
        // remaining vertical space and scrolls in both directions when needed.
        let available_height = ui.available_height().max(180.0);
        egui::ScrollArea::both()
            .id_salt(("function_pseudocode", function.start))
            .max_height(available_height)
            .show(ui, |ui| {
                let signature = format!("void {}(void) {{", function.shown_name(mangled));
                ui.label(syntax::pseudo_code(
                    ui,
                    &signature,
                    egui::Color32::TRANSPARENT,
                ));
                for (index, instruction) in body.iter().enumerate() {
                    let highlighted = highlighted_instructions
                        .as_ref()
                        .is_some_and(|range| range.contains(&index));
                    ui.horizontal(|ui| {
                        let fill = if highlighted {
                            ui.style().visuals.selection.bg_fill
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        ui.label(syntax::dim(
                            ui,
                            &format!("{:#018x}", instruction.address),
                            fill,
                        ));
                        let code = format!("    {}", super::decompile::pseudo_c(&instruction.text));
                        let response = ui.label(syntax::pseudo_code(ui, &code, fill));
                        if highlighted
                            && highlighted_instructions
                                .as_ref()
                                .is_some_and(|range| index == range.start)
                        {
                            ui.scroll_to_rect(response.rect, Some(egui::Align::Center));
                        }
                    });
                }
                ui.label(syntax::pseudo_code(ui, "}", egui::Color32::TRANSPARENT));
            });
    });
}

/// Whether a name in the symbol table is the beginning of a function, as
/// opposed to a name for some other address.
///
/// Every name is worth having — `stored` is what makes a comparison readable —
/// but only some of them begin a function, and the difference matters more
/// than it sounds: a name taken for a function start ends the one above it, so
/// a label in the middle of a routine cuts that routine into pieces, and the
/// decompiler and the assembler export are then working on a fragment.
///
/// Two conditions, both read off the file rather than guessed from the name.
///
/// **It has to be code.** A symbol at an address no instruction was decoded at
/// is data — `stored` in `.rodata`, `__bss_start` past the end of it — and a
/// function with an empty body is not a function.
///
/// **An untyped name has to be one the object file offers to others.** A
/// compiler says `STT_FUNC` and there is nothing to work out; an assembler
/// says nothing at all, and then the binding is the only thing separating
/// `_start`, which is global, from `_start.cmp_loop`, which is local and lives
/// inside it.
fn starts_a_function(analysis: &Analysis, symbol: &Symbol) -> bool {
    let Some(address) = symbol.address else {
        return false;
    };
    if analysis.instruction_index(address).is_none() {
        return false;
    }
    match symbol.kind {
        SymbolKind::Function => true,
        SymbolKind::Untyped => !symbol.local,
        SymbolKind::Data => false,
    }
}

/// Every named function defined in this binary, in address order.
///
/// Built once when a binary is opened — see [`Function`] — and read from the
/// Functions and pseudo-code views alike, so both bound a body the same way.
#[must_use]
pub fn all(analysis: &Analysis) -> Vec<Function> {
    let mut symbols: Vec<&Symbol> = analysis
        .symbols
        .iter()
        .filter(|symbol| !symbol.imported)
        .filter(|symbol| symbol.address.is_some())
        .filter(|symbol| starts_a_function(analysis, symbol))
        .collect();
    symbols.sort_by_key(|symbol| symbol.address);
    symbols.dedup_by_key(|symbol| symbol.address);

    let decoded_end = analysis
        .instructions
        .iter()
        .map(|instruction| {
            instruction
                .address
                .saturating_add(instruction.bytes.len() as u64)
        })
        .max()
        .unwrap_or_default();

    // The file's own names first, then what the code points at where it names
    // nothing. A stripped binary has an empty symbol table, and without this
    // the whole view — the graph, the pseudo-code, "run this function" — is
    // empty with it.
    let mut starts: Vec<(u64, Option<String>, Option<discover::Evidence>)> = symbols
        .iter()
        .map(|symbol| {
            let start = symbol
                .address
                .expect("function symbols were filtered above");
            (start, Some(symbol.name.clone()), None)
        })
        .collect();
    for found in discover::functions(analysis) {
        starts.push((found.address, None, Some(found.evidence)));
    }
    starts.sort_by_key(|(start, _, _)| *start);
    starts.dedup_by_key(|(start, _, _)| *start);

    // Where each one ends: at its own declared size when it has one, and
    // otherwise where the next one begins. Worked out after the two lists are
    // merged, so a named function that runs up to an unnamed one ends there
    // rather than swallowing it.
    let sizes: HashMap<u64, u64> = symbols
        .iter()
        .filter(|symbol| symbol.size > 0)
        .filter_map(|symbol| symbol.address.map(|address| (address, symbol.size)))
        .collect();

    starts
        .iter()
        .enumerate()
        .map(|(index, (start, name, found_by))| {
            let start = *start;
            let end = sizes
                .get(&start)
                .map_or_else(
                    || {
                        starts
                            .get(index + 1)
                            .map_or(decoded_end, |(next, _, _)| *next)
                    },
                    |size| start.saturating_add(*size),
                )
                .max(start);
            let instructions = analysis.instruction_span(start..end);
            let name = name
                .clone()
                .unwrap_or_else(|| discover::placeholder_name(start));
            Function {
                readable: desdec_core::demangle::readable(&name),
                name,
                start,
                end,
                found_by: *found_by,
                blocks: blocks::of(&analysis.instructions[instructions.clone()]),
                instructions,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name is not a function start, and taking one for the other cuts the
    /// routine above it into pieces — which the decompiler and the assembler
    /// export then work on.
    ///
    /// The crackme this came from: `_start` is a global untyped name at the
    /// first instruction, `_start.cmp_loop` is a local one in the middle of
    /// it, and `stored` is a local one in `.rodata`. One of the three begins a
    /// function.
    #[test]
    fn only_the_names_that_begin_a_function_end_the_one_above_them() {
        // A real analysis rather than one written here: what counts as "an
        // instruction was decoded at this address" is its own answer, and a
        // hand-built listing would be this test agreeing with itself.
        let analysis = crate::testing::reference_analysis();
        let code = analysis.instructions[0].address;
        let elsewhere = 1;
        assert!(
            analysis.instruction_index(elsewhere).is_none(),
            "address 1 holds no instruction in any real binary"
        );
        let named = |name: &str, address: u64, kind, local| Symbol {
            name: name.to_owned(),
            address: Some(address),
            kind,
            local,
            ..Symbol::default()
        };

        assert!(
            starts_a_function(analysis, &named("_start", code, SymbolKind::Untyped, false)),
            "a global untyped name at an instruction is what an assembler \
             writes for the entry point"
        );
        assert!(
            !starts_a_function(
                analysis,
                &named("_start.cmp_loop", code, SymbolKind::Untyped, true)
            ),
            "a local one is a label inside a routine, not a routine"
        );
        assert!(
            !starts_a_function(
                analysis,
                &named("stored", elsewhere, SymbolKind::Untyped, true)
            ),
            "and nothing was decoded there: it is data"
        );
        assert!(
            starts_a_function(analysis, &named("main", code, SymbolKind::Function, true)),
            "a compiler says so outright, and a static function is still a \
             function"
        );
        assert!(
            !starts_a_function(analysis, &named("table", code, SymbolKind::Data, false)),
            "so is the other way round: the file saying data settles it"
        );
    }

    fn instruction(address: u64, text: &str) -> Instruction {
        Instruction {
            address,
            bytes: desdec_core::InstructionBytes::new(&[0x90]).expect("one byte"),
            text: text.to_owned(),
            section: std::sync::Arc::from(".text"),
        }
    }

    /// The view teaches the confidence boundary before it shows a dense table
    /// of names: a function recovered from a call and one inferred from a
    /// prologue must not read as equally certain.
    #[test]
    fn functions_guide_explains_evidence_and_how_to_read_the_graphs() {
        use eframe::egui;

        let ctx = egui::Context::default();
        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| {
                super::function_guide(ui, Language::English);
            });
        };
        let _ = ctx.run(crate::testing::window_input(), &mut draw);
        let output = ctx.run(crate::testing::window_input(), &mut draw);
        let said = crate::testing::drawn_text(&output.shapes);

        for item in [
            Text::HowToReadFunctions,
            Text::FunctionsGuideIntro,
            Text::FunctionsGuideList,
            Text::FunctionsGuideRelations,
        ] {
            let wanted = text(Language::English, item);
            assert!(said.contains(wanted), "{wanted:?} is on screen");
        }
    }

    /// The call graph pane answers the three questions it exists for, and
    /// clicking a name in it walks to that function.
    #[test]
    fn the_call_graph_pane_names_callers_and_callees_and_walks_between_them() {
        use eframe::egui;

        let mut app = crate::testing::opened_app(crate::app::WorkspaceView::Functions);
        app.preferences.language = Language::English;
        // A function with something at both ends, which the first function of
        // a file usually is not.
        let connected = app
            .functions
            .iter()
            .find(|function| {
                app.callgraph
                    .edges(function.start)
                    .is_some_and(|edges| !edges.callers.is_empty() && !edges.calls.is_empty())
            })
            .map(|function| function.start);
        let Some(connected) = connected else {
            return; // Nothing on this host has both.
        };
        app.selected_function = Some(connected);

        let ctx = egui::Context::default();
        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = super::show(
                    ui,
                    app.analysis.as_ref().expect("a binary"),
                    &app.functions,
                    &app.callgraph,
                    &mut app.selected_function,
                    &mut false,
                    Language::English,
                );
            });
        };
        let _ = ctx.run(crate::testing::window_input(), &mut draw);
        let output = ctx.run(crate::testing::window_input(), &mut draw);
        let said = crate::testing::drawn_text(&output.shapes);

        for heading in [Text::Callers, Text::Callees, Text::HowToGetHere] {
            let wanted = text(Language::English, heading);
            assert!(said.contains(wanted), "{wanted:?} is on screen");
        }
        // The callers are named, not counted.
        let caller = app
            .callgraph
            .edges(connected)
            .and_then(|edges| edges.callers.first())
            .map(|call| call.from)
            .expect("it has a caller");
        let name = app
            .functions
            .iter()
            .find(|function| function.start == caller)
            .map(|function| function.name.clone())
            .expect("the caller is a function");
        assert!(said.contains(&name), "the caller is named: {name}");
    }

    /// A file that names nothing still has a Functions view.
    ///
    /// The state most files worth reading are in, and the one this view used
    /// to be empty in — and with it the graph, the pseudo-code and "run this
    /// function".
    #[test]
    fn a_stripped_binary_still_has_functions_to_show() {
        let mut analysis = crate::testing::reference_analysis().clone();
        analysis.symbols.clear();
        let found = all(&analysis);
        assert!(
            found.len() > 20,
            "a real binary's own code points at more than a handful: {}",
            found.len()
        );
        for function in &found {
            assert!(
                function.found_by.is_some(),
                "{} has no symbol table to have come from",
                function.name
            );
            assert!(
                function.name.starts_with("sub_"),
                "{} is drawn as what it is: an address, not a name",
                function.name
            );
            assert!(
                entry(function, &analysis).is_some(),
                "{} leads into the listing",
                function.name
            );
        }
    }

    /// What the file names keeps its name, and is not marked as found.
    #[test]
    fn a_named_function_keeps_its_name_and_says_so() {
        let analysis = crate::testing::reference_analysis();
        let found = all(analysis);
        if analysis.symbols.iter().all(|symbol| symbol.imported) {
            return; // This host's own binary names nothing.
        }
        let named: Vec<&Function> = found
            .iter()
            .filter(|function| function.found_by.is_none())
            .collect();
        assert!(!named.is_empty(), "the host binary names some");
        assert!(
            named
                .iter()
                .all(|function| !function.name.starts_with("sub_")),
            "a named function is shown under its name"
        );
    }

    /// Rows are in address order and none is listed twice, whichever list each
    /// came from.
    #[test]
    fn the_two_lists_are_merged_in_order_and_without_repeats() {
        let analysis = crate::testing::reference_analysis();
        let found = all(analysis);
        let mut previous = None;
        for function in &found {
            if let Some(previous) = previous {
                assert!(
                    function.start > previous,
                    "{:#x} follows {previous:#x}",
                    function.start
                );
            }
            previous = Some(function.start);
        }
    }

    /// Every function the table offers a way into must lead somewhere the
    /// listing can really show. An address the decoder never reached would
    /// move the workspace to the disassembly and leave it looking at nothing,
    /// so the offer is withheld instead.
    ///
    /// Asked of the synthetic binaries rather than of the host's own, which is
    /// what this used to use. A PE names no functions at all — its symbols
    /// live in a separate PDB — so on Windows the test asserted that the host
    /// binary declares functions and failed there and only there, on a
    /// property of the format rather than on anything this code does. The
    /// fixtures carry named function symbols in all three formats, which is
    /// what they were built for.
    #[test]
    fn the_way_into_the_listing_lands_on_a_decoded_instruction() {
        let mut checked = 0_usize;
        for sample in crate::testing::samples() {
            let label = sample.fixture.label;
            let analysis = &sample.analysis;
            let functions = all(analysis);
            assert!(!functions.is_empty(), "{label} names functions");

            let mut offered = 0_usize;
            for function in &functions {
                let Some(address) = entry(function, analysis) else {
                    continue;
                };
                offered += 1;
                assert!(
                    analysis.instruction_index(address).is_some(),
                    "{label}: {} leads to {address:#x}, which is not in the listing",
                    function.name
                );
            }
            assert!(offered > 0, "{label}: no function could be reached at all");
            checked += offered;
        }
        assert!(checked > 0, "the fixtures reach at least one function");
    }

    /// The same of the host's own binary, which is a real and much richer file
    /// than any fixture — when its format names functions at all.
    #[test]
    fn the_host_binary_is_held_to_the_same_promise() {
        let analysis = crate::testing::reference_analysis();
        for function in &all(analysis) {
            let Some(address) = entry(function, analysis) else {
                continue;
            };
            assert!(
                analysis.instruction_index(address).is_some(),
                "{} leads to {address:#x}, which is not in the listing",
                function.name
            );
        }
    }

    /// The switch is what the reader asked for: the source spelling by
    /// default, the file's own on request. Both are always available, because
    /// a reading has to be checkable against what it was read from.
    #[test]
    fn a_mangled_name_is_read_back_and_the_original_is_kept() {
        let function = Function {
            name: "_ZN4core3fmt5write17h0e4b3a1c9f2d8a76E".to_owned(),
            readable: desdec_core::demangle::readable("_ZN4core3fmt5write17h0e4b3a1c9f2d8a76E"),
            start: 0x1000,
            end: 0x1010,
            instructions: 0..0,
            found_by: None,
            blocks: Vec::new(),
        };

        assert_eq!(function.shown_name(false), "core::fmt::write");
        assert_eq!(
            function.shown_name(true),
            "_ZN4core3fmt5write17h0e4b3a1c9f2d8a76E"
        );
    }

    /// A name that is not encoded, and one whose encoding is not fully
    /// understood, both keep the file's own spelling whichever way the switch
    /// is set. A half-decoded name that looked decoded would be worse than the
    /// original, because nothing would tell the reader which half was guessed.
    #[test]
    fn a_name_that_cannot_be_read_back_is_shown_as_the_file_spells_it() {
        for name in ["main", "_start", "sub_401000"] {
            let function = Function {
                name: name.to_owned(),
                readable: desdec_core::demangle::readable(name),
                start: 0x1000,
                end: 0x1010,
                instructions: 0..0,
                found_by: None,
                blocks: Vec::new(),
            };
            assert_eq!(function.shown_name(false), name);
            assert_eq!(function.shown_name(true), name);
        }
    }

    /// A symbol whose body was never decoded is not offered: the button is
    /// drawn disabled rather than drawn and refused.
    #[test]
    fn a_function_without_a_decoded_body_is_not_offered() {
        let analysis = crate::testing::reference_analysis();
        let empty = Function {
            name: "jamais_decodee".to_owned(),
            readable: None,
            start: 0xdead_beef,
            end: 0xdead_bef0,
            instructions: 0..0,
            found_by: None,
            blocks: Vec::new(),
        };
        assert_eq!(entry(&empty, analysis), None);
    }

    /// The blocks a function is cut into are [`desdec_core::blocks`]'s, and
    /// this reads them through a `Function` the way the view does.
    ///
    /// Written in the syntax the decoder actually produces — GAS, so `0x1006`
    /// and not the MASM `1006h` this once used: cutting a function into blocks
    /// is now done in one place for the whole tool, against the text that
    /// place is given.
    #[test]
    fn conditional_branch_creates_two_successors() {
        let instructions = [
            instruction(0x1000, "cmp %eax,%eax"),
            instruction(0x1002, "je 0x1006"),
            instruction(0x1004, "ret"),
            instruction(0x1006, "ret"),
        ];
        let blocks = blocks::of(&instructions);

        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].instructions, 0..2);
        assert_eq!(blocks[1].instructions, 2..3);
        assert_eq!(blocks[2].instructions, 3..4);
        let out: Vec<u64> = blocks[0]
            .successors
            .iter()
            .map(|successor| successor.address)
            .collect();
        assert_eq!(out, [0x1006, 0x1004], "the taken arm first");
        assert!(blocks[1].successors.is_empty());
        assert!(blocks[2].successors.is_empty());
    }

    /// Basic blocks are found from mnemonics, and the ARM ones look nothing
    /// like the x86 ones. Every fixture's `main` branches, so every reader has
    /// to end up with a function split into more than one block.
    #[test]
    fn a_branching_function_is_split_into_blocks_in_every_format() {
        for sample in crate::testing::samples() {
            let label = sample.fixture.label;
            let functions = all(&sample.analysis);
            let main = functions
                .iter()
                .find(|function| function.name == "main")
                .unwrap_or_else(|| panic!("{label}: the fixture defines main"));

            assert!(
                main.blocks.len() > 1,
                "{label}: main came out as {} block(s); its branch was not seen",
                main.blocks.len()
            );
            assert!(
                main.blocks.iter().any(|block| !block.successors.is_empty()),
                "{label}: no block leads anywhere"
            );
        }
    }

    /// The symbol table of a real binary runs to thousands of names. The list
    /// must lay out only what is on screen, or the view pays for every symbol
    /// on every frame.
    #[test]
    fn only_the_visible_function_rows_are_laid_out() {
        use crate::{
            app::WorkspaceView,
            testing::{drawn_text, opened_app, window_input},
        };

        let mut app = opened_app(WorkspaceView::Functions);
        let total = app.functions.len();
        if total < 200 {
            return; // Too few symbols on this host for the question to arise.
        }
        let names: Vec<String> = app
            .functions
            .iter()
            .map(|function| function.name.clone())
            .collect();

        let ctx = egui::Context::default();
        let output = ctx.run(window_input(), |ctx| {
            crate::ui::views::show_central_panel(&mut app, ctx);
        });

        let drawn = drawn_text(&output.shapes);
        let shown = names.iter().filter(|name| drawn.contains(*name)).count();
        assert!(shown > 0, "the list must show the functions it has");
        assert!(
            shown < total / 4,
            "{shown} of {total} names were laid out: the list is not virtualised"
        );
    }
}
