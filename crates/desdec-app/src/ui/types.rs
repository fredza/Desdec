//! What the bytes at an address mean, through a type the reader wrote.
//!
//! Everywhere else in Desdec, the tool reads and the reader is told. Here it
//! is the other way round: the file states almost nothing about its own data,
//! and what is missing — that the eight bytes at `rbx+0x18` are a length, and
//! the four before them a tag — is the reader's own knowledge. The view is
//! where they write it down, once, in the C their headers are already written
//! in, and see it applied.
//!
//! Where the bytes come from depends on whether anything is running. With the
//! Machine started, they come out of its memory, so a structure is read as it
//! stands at this instant of the run. With no machine, they come out of the
//! file, at the address its sections map to — which is the whole static half
//! of the tool, and the half x64dbg does not have: a structure can be read
//! before anything has executed at all.
//!
//! A pointer is shown and not followed. Following one is a press — and the
//! trail back is kept, because a reader who has walked four links down a list
//! wants to come back up it.

use std::collections::BTreeSet;

use desdec_core::{
    Architecture, Section,
    emulate::{Convention, condition::Expression},
    types::{
        Model, Registry, Type, catalogue, infer, parse,
        read::{self, Depth, Flat, Reading, Source, Value},
    },
};
use eframe::egui;

use crate::{
    app::DesdecApp,
    i18n::{Language, Text, text},
    icons::{self, Icon},
    preferences::accent,
    ui::{ERROR, MUTED, card, columns_over, monospace_value},
};

/// How many elements of one array are read at a time.
const ARRAY_LIMIT: u64 = 32;

/// The button that forgets one recorded saying: a row's height, not a
/// toolbar's.
const FORGET_BUTTON_SIZE: egui::Vec2 = egui::vec2(22.0, 20.0);
/// How deep a reading goes however much is opened, so a type that nests
/// through arrays cannot be asked for more rows than a screen can hold.
const DEPTH_LIMIT: usize = 8;

/// What the reader has written, and what is applied where.
#[derive(Default)]
pub struct State {
    /// The C, as typed. Kept with the notes about the binary; see
    /// [`crate::annotations`].
    pub source: String,
    /// What it last read to. Only replaced when the source reads cleanly, so
    /// a half-typed definition does not take the working ones away.
    pub registry: Registry,
    /// Why the source could not be read, when it could not.
    pub refused: Option<String>,
    /// The type being applied, by name.
    pub applied: Option<String>,
    /// Where it is applied, written as an expression.
    pub address: String,
    /// Whether the number written is a file offset rather than an address.
    ///
    /// The two are different questions and both are worth asking: an address
    /// is where the loader will put something, and an offset is where it sits
    /// in the file on disk — which is the only way to reach the header, since
    /// no section maps it.
    pub by_file_offset: bool,
    /// The register a structure is read out of the code through.
    pub base: String,
    /// What the last reading out of the code found, kept so its report stays
    /// on screen after the definition has been added.
    inferred: Option<Report>,
    /// Which rows are open, by path.
    open: BTreeSet<String>,
    /// Where following a pointer came from, so it can be walked back.
    trail: Vec<(String, String)>,
}

impl State {
    /// Reads the source again, keeping the last good definitions if it does
    /// not read.
    pub fn reread(&mut self) {
        if self.source.trim().is_empty() {
            self.registry.clear();
            self.refused = None;
            self.applied = None;
            return;
        }
        match parse::definitions(&self.source) {
            Ok(definitions) => {
                self.registry.clear();
                for definition in definitions {
                    self.registry.define(definition);
                }
                self.refused = None;
                // A type that was applied and has since been deleted from the
                // source stops being applied, rather than staying on screen as
                // a name nothing answers to.
                if let Some(applied) = &self.applied {
                    if self.registry.get(applied).is_none() {
                        self.applied = None;
                    }
                }
                if self.applied.is_none() {
                    self.applied = self
                        .registry
                        .all()
                        .next()
                        .map(|first| first.name().to_owned());
                }
            }
            Err(error) => self.refused = Some(error.to_string()),
        }
    }

    /// Reads the file's shape into the registry, so `long` and the pointers
    /// are as wide as this binary's are.
    pub fn set_model(&mut self, model: Model) {
        self.registry.set_model(model);
    }

    /// How deep the reading has to go to draw what is open.
    fn depth(&self) -> Depth {
        let deepest = self
            .open
            .iter()
            .map(|path| path.matches('.').count() + 1)
            .max()
            .unwrap_or(0);
        Depth {
            levels: deepest.min(DEPTH_LIMIT) + 1,
            array: ARRAY_LIMIT,
        }
    }
}

/// What each instruction of the listing touches, named through the types the
/// reader said the registers hold.
///
/// Built from the sayings in [`crate::annotations::Annotations::in_code`] and
/// rebuilt whenever those or the definitions change. The listing reads it on
/// every visible row, so it must be a lookup and not a computation: naming an
/// access means decoding an operand and walking a type, and a listing draws
/// forty rows sixty times a second.
#[derive(Default)]
pub struct MemberNames {
    by_address: std::collections::BTreeMap<u64, String>,
}

impl MemberNames {
    /// What the instruction at `address` touches, when something does.
    #[must_use]
    pub fn get(&self, address: u64) -> Option<&str> {
        self.by_address.get(&address).map(String::as_str)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_address.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_address.len()
    }

    /// An index made from names given directly, for the tests that are about
    /// what the listing draws rather than about how the index was filled.
    #[cfg(test)]
    #[must_use]
    pub fn of<'a>(named: impl IntoIterator<Item = (u64, &'a str)>) -> Self {
        Self {
            by_address: named
                .into_iter()
                .map(|(address, path)| (address, path.to_owned()))
                .collect(),
        }
    }

    /// Names every access the reader's sayings cover.
    #[must_use]
    pub fn build(app: &DesdecApp) -> Self {
        let mut by_address = std::collections::BTreeMap::new();
        let Some(analysis) = app.analysis.as_ref() else {
            return Self { by_address };
        };
        let architecture = analysis.summary.architecture;

        for saying in app.annotations.in_code() {
            let kind = Type::Named(saying.kind.clone());
            if app.structures.registry.layout(&kind).is_err() {
                // A type the reader has since deleted names nothing, rather
                // than leaving the listing labelled from a definition that is
                // no longer there.
                continue;
            }
            let Some(function) = app
                .functions
                .iter()
                .find(|function| function.start == saying.function)
            else {
                continue;
            };
            for named in infer::name_accesses(
                function.body(analysis),
                &saying.register,
                architecture,
                &app.structures.registry,
                &kind,
            ) {
                let written = if named.into == 0 {
                    named.path
                } else {
                    // An access part way into a member is part way into it,
                    // and saying so is the difference between a reading and a
                    // guess.
                    format!("{}+{:#x}", named.path, named.into)
                };
                by_address.entry(named.at).or_insert(written);
            }
        }
        Self { by_address }
    }
}

/// What one reading out of the code found, in the reader's terms.
struct Report {
    /// The function it was read from, and the register it followed.
    through: String,
    accesses: usize,
    members: usize,
    /// What was found and deliberately not laid out.
    below: usize,
    indexed: usize,
    unstated: usize,
    overlapping: usize,
}

/// The file's own bytes, at the addresses its sections map to.
///
/// A section that stores nothing — `.bss` and its kin — is not read from: it
/// is mapped, but the file holds no bytes for it, and answering zero would be
/// answering with something the file never said.
struct FileImage<'a> {
    sections: &'a [Section],
    bytes: &'a [u8],
}

impl Source for FileImage<'_> {
    fn read(&self, address: u64, into: &mut [u8]) -> bool {
        let wanted = into.len() as u64;
        for section in self.sections {
            if !section.is_mapped() {
                continue;
            }
            let Some(offset) = address.checked_sub(section.virtual_address) else {
                continue;
            };
            if offset >= section.file_size || offset + wanted > section.file_size {
                continue;
            }
            let start = section.file_offset.saturating_add(offset);
            let Ok(start) = usize::try_from(start) else {
                return false;
            };
            let Some(slice) = self.bytes.get(start..start.saturating_add(into.len())) else {
                return false;
            };
            into.copy_from_slice(slice);
            return true;
        }
        false
    }
}

/// What a press on a row asked for.
#[derive(Default)]
struct Asked {
    /// An address to show in the listing.
    go_to: Option<u64>,
    /// A pointer to follow: the type it points at, and where it points.
    follow: Option<(String, u64)>,
}

pub fn show(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    if app.analysis.is_none() {
        return;
    }
    ui.label(egui::RichText::new(text(language, Text::StructuresExplained)).color(MUTED));
    ui.add_space(10.0);

    let mut asked = Asked::default();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Both panes take the whole application — one writes the
            // registry, the other reads it beside the machine — so they are
            // handed it in turn rather than each capturing it.
            columns_over(
                ui,
                app,
                |ui, app| definitions_pane(app, ui, language),
                |ui, app| asked = reading_pane(app, ui, language),
            );
        });

    if let Some((kind, address)) = asked.follow {
        app.structures.trail.push((
            app.structures.applied.clone().unwrap_or_default(),
            app.structures.address.clone(),
        ));
        app.structures.applied = Some(kind);
        app.structures.address = format!("{address:#x}");
        app.structures.open.clear();
    }
    if let Some(address) = asked.go_to {
        let ctx = ui.ctx().clone();
        app.go_to_address(&ctx, address);
    }
}

/// The C the reader writes, and what it read to.
fn definitions_pane(app: &mut DesdecApp, ui: &mut egui::Ui, language: Language) {
    card(ui, text(language, Text::TypeDefinitions), |ui| {
        ui.label(egui::RichText::new(text(language, Text::TypeDefinitionsHint)).color(MUTED));
        ui.add_space(6.0);
        let typed = ui.add(
            egui::TextEdit::multiline(&mut app.structures.source)
                .desired_width(f32::INFINITY)
                .desired_rows(12)
                .code_editor()
                .hint_text("struct Node {\n    struct Node *next;\n    char name[16];\n};"),
        );
        // Read as it is typed, like the expression window: having to press a
        // key to see whether a definition is good makes the pane a form.
        if typed.changed() {
            app.structures.reread();
            // A definition that changed changes what every named row in the
            // listing says, so the index goes with it.
            app.rebuild_member_names();
        }
        if let Some(refused) = &app.structures.refused {
            ui.add_space(6.0);
            ui.colored_label(ERROR, refused);
        }
        ui.add_space(6.0);
        offer_the_format(app, ui, language);
        ui.add_space(6.0);
        ui.label(egui::RichText::new(text(language, Text::TypesKeptWithTheNotes)).color(MUTED));
    });

    ui.add_space(8.0);
    infer_pane(app, ui, language);

    ui.add_space(8.0);
    card(ui, text(language, Text::DefinedTypes), |ui| {
        if app.structures.registry.is_empty() {
            ui.label(egui::RichText::new(text(language, Text::NoTypesDefined)).color(MUTED));
            return;
        }
        let mut chosen = None;
        egui::Grid::new("desdec.structures.defined")
            .num_columns(4)
            .spacing([16.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                ui.strong(text(language, Text::Name));
                ui.strong(text(language, Text::Size));
                ui.strong(text(language, Text::Alignment));
                ui.label("");
                ui.end_row();

                for definition in app.structures.registry.all() {
                    let name = definition.name().to_owned();
                    let laid_out = app.structures.registry.layout(&Type::Named(name.clone()));
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(definition.keyword()).color(MUTED));
                        ui.monospace(&name);
                    });
                    match &laid_out {
                        Ok(layout) => {
                            monospace_value(ui, &format!("{} B", layout.size));
                            monospace_value(ui, &format!("{} B", layout.alignment));
                        }
                        Err(error) => {
                            ui.colored_label(ERROR, error.to_string());
                            ui.label("");
                        }
                    }
                    let selected = app.structures.applied.as_deref() == Some(name.as_str());
                    if ui
                        .add_enabled(
                            laid_out.is_ok(),
                            egui::SelectableLabel::new(
                                selected,
                                text(language, Text::ReadAtAnAddress),
                            ),
                        )
                        .clicked()
                    {
                        chosen = Some(name);
                    }
                    ui.end_row();
                }
            });
        if let Some(name) = chosen {
            app.structures.applied = Some(name);
            app.structures.open.clear();
            app.structures.trail.clear();
        }
    });
}

/// The declarations of the file's own format, offered rather than applied.
///
/// The first structure anyone wants over a binary is the one it starts with,
/// and typing `Elf64_Ehdr` out of the manual page is twenty lines of work that
/// is the same for every ELF ever opened.
fn offer_the_format(app: &mut DesdecApp, ui: &mut egui::Ui, language: Language) {
    let Some(format) = app
        .analysis
        .as_ref()
        .map(|analysis| analysis.summary.format)
    else {
        return;
    };
    let (Some(source), Some(header)) = (catalogue::of(format), catalogue::header_of(format)) else {
        return;
    };
    // Offered once: the declarations are already there, and adding them twice
    // would only replace them with themselves.
    if app.structures.registry.get(header).is_some() {
        return;
    }

    let label = format!(
        "{} ({})",
        text(language, Text::AddTheFormatsOwn),
        format.label()
    );
    if !ui.button(label).clicked() {
        return;
    }
    if !app.structures.source.is_empty() && !app.structures.source.ends_with('\n') {
        app.structures.source.push('\n');
    }
    app.structures.source.push_str(source);
    app.structures.reread();
    app.structures.applied = Some(header.to_owned());
    // The header sits at the start of the file and no section maps it, so it
    // is reached by offset rather than by address.
    app.structures.by_file_offset = true;
    app.structures.address = String::from("0");
    app.structures.open.clear();
}

/// Reading a structure out of the code that walks it.
///
/// The reader who knows a function takes a pointer still has to work out what
/// it points at, and does it the same way every time: read down the listing
/// and write the offsets in a notebook. This is that, done by the tool — and
/// it is a starting point, never an answer: the names are offsets and the
/// types are widths.
fn infer_pane(app: &mut DesdecApp, ui: &mut egui::Ui, language: Language) {
    card(ui, text(language, Text::ReadFromTheCode), |ui| {
        ui.label(egui::RichText::new(text(language, Text::ReadFromTheCodeHelp)).color(MUTED));
        ui.add_space(6.0);

        let chosen = app
            .selected_function
            .and_then(|start| {
                app.functions
                    .iter()
                    .find(|function| function.start == start)
            })
            .or_else(|| app.functions.first());
        let Some(function) = chosen else {
            ui.label(egui::RichText::new(text(language, Text::NoFunctionSelected)).color(MUTED));
            return;
        };
        let name = function.name.clone();
        let start = function.start;

        if app.structures.base.is_empty() {
            first_argument(app).clone_into(&mut app.structures.base);
        }
        ui.horizontal(|ui| {
            ui.monospace(&name);
            ui.label(text(language, Text::ThroughRegister));
            ui.add(
                egui::TextEdit::singleline(&mut app.structures.base)
                    .desired_width(70.0)
                    .font(egui::TextStyle::Monospace),
            );
            if ui.button(text(language, Text::ReadItOut)).clicked() {
                read_out_of_the_code(app, start, &name);
            }
        });

        // Naming a type the reader already has, against the function and
        // register in front of them: a structure worked out on one function is
        // usually the same one the next function is handed.
        let applied = app.structures.applied.clone();
        if let Some(kind) = applied {
            ui.horizontal(|ui| {
                if ui
                    .button(text(language, Text::NameItInTheListing))
                    .clicked()
                {
                    let register = app.structures.base.trim().to_owned();
                    if !register.is_empty() {
                        app.annotations.say_in_code(crate::annotations::InCode {
                            function: start,
                            register,
                            kind: kind.clone(),
                        });
                        app.rebuild_member_names();
                    }
                }
                ui.label(
                    egui::RichText::new(text(language, Text::NameItInTheListingHelp)).color(MUTED),
                );
            });
        }
        sayings(app, ui, language);

        if let Some(report) = &app.structures.inferred {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(text(language, Text::ReadOutOf).replace("{}", &report.through))
                    .color(MUTED),
            );
            for (count, said) in [
                (report.members, Text::MembersFound),
                (report.accesses, Text::AccessesRead),
                (report.below, Text::AccessesBelowThePointer),
                (report.indexed, Text::AccessesThroughAnIndex),
                (report.unstated, Text::AccessesOfUnstatedWidth),
                (report.overlapping, Text::OffsetsInsideAnotherMember),
            ] {
                if count > 0 {
                    ui.label(
                        egui::RichText::new(format!("{count} — {}", text(language, said)))
                            .color(MUTED),
                    );
                }
            }
        }
    });
}

/// What the reader has said each register holds, and a way to take it back.
fn sayings(app: &mut DesdecApp, ui: &mut egui::Ui, language: Language) {
    if app.annotations.in_code().is_empty() {
        return;
    }
    ui.add_space(6.0);
    // The count is the answer to "did that do anything?", and a saying about a
    // register the function never uses as a base names no row at all.
    let said = if app.member_names.is_empty() {
        text(language, Text::NamedInTheListing).to_owned()
    } else {
        format!(
            "{} — {}",
            text(language, Text::NamedInTheListing),
            text(language, Text::RowsNamed).replace("{}", &app.member_names.len().to_string())
        )
    };
    ui.label(egui::RichText::new(said).color(MUTED));

    let mut forget = None;
    let theme = accent(app.preferences.theme);
    let sayings: Vec<crate::annotations::InCode> = app.annotations.in_code().to_vec();
    egui::Grid::new("desdec.structures.in_code")
        .num_columns(4)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            for saying in &sayings {
                let called = app
                    .functions
                    .iter()
                    .find(|function| function.start == saying.function)
                    .map_or_else(
                        || format!("{:#x}", saying.function),
                        |function| function.name.clone(),
                    );
                ui.monospace(called);
                ui.monospace(&saying.register);
                ui.label(egui::RichText::new(&saying.kind).color(MUTED));
                // Drawn, not written: no font egui ships has a cross in the
                // proportional family, and `✕` reached the row as `◻`.
                if icons::sized_button(ui, Icon::Close, None, false, theme, FORGET_BUTTON_SIZE)
                    .clicked()
                {
                    forget = Some((saying.function, saying.register.clone()));
                }
                ui.end_row();
            }
        });
    if let Some((function, register)) = forget {
        app.annotations.forget_in_code(function, &register);
        app.rebuild_member_names();
    }
}

/// The register the first argument of a call arrives in, which is the one a
/// reader almost always wants.
fn first_argument(app: &DesdecApp) -> &'static str {
    let architecture = app
        .analysis
        .as_ref()
        .map_or(Architecture::Unknown, |analysis| {
            analysis.summary.architecture
        });
    match architecture {
        Architecture::Arm64 => "x0",
        Architecture::X86_64 => match app.machine_convention {
            Convention::Microsoft => "rcx",
            Convention::SystemV => "rdi",
        },
        // A 32-bit call passes its arguments on the stack, so there is no
        // register to start from: the frame pointer is what a reader follows.
        _ => "ebp",
    }
}

/// Reads a structure out of one function and writes it into the source.
fn read_out_of_the_code(app: &mut DesdecApp, start: u64, called: &str) {
    let base = app.structures.base.trim().to_owned();
    let Some(analysis) = app.analysis.as_ref() else {
        return;
    };
    let Some(function) = app
        .functions
        .iter()
        .find(|function| function.start == start)
    else {
        return;
    };
    let body = function.body(analysis);
    let name = suggested_name(called, &base);
    let inferred = infer::from_body(&name, &base, body, analysis.summary.architecture);

    app.structures.inferred = Some(Report {
        through: format!("{called} · {base}"),
        accesses: inferred.accesses.len(),
        members: inferred.definition.members().len(),
        below: inferred.below.len(),
        indexed: inferred.indexed.len(),
        unstated: inferred.unstated.len(),
        overlapping: inferred.overlapping.len(),
    });
    if inferred.is_empty() {
        return;
    }

    // Added to what the reader has written rather than replacing it: what is
    // read out of the code is a draft they then edit, and taking their own
    // definitions away to make room for a draft would be the wrong way round.
    let mut registry = Registry::new(*app.structures.registry.model());
    registry.define(inferred.definition);
    let written = registry.to_source();
    if !app.structures.source.is_empty() && !app.structures.source.ends_with('\n') {
        app.structures.source.push('\n');
    }
    app.structures.source.push_str(&written);
    app.structures.reread();
    app.structures.applied = Some(name.clone());
    app.structures.open.clear();
    // The reading already knows which register in which function it came from,
    // so the listing can be named from it without being asked twice.
    app.annotations.say_in_code(crate::annotations::InCode {
        function: start,
        register: base,
        kind: name,
    });
    app.rebuild_member_names();
}

/// A name for a structure read out of a function: the function's, the
/// register's, and nothing that is not a C identifier.
fn suggested_name(called: &str, base: &str) -> String {
    let cleaned: String = called
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let cleaned = cleaned.trim_matches('_');
    let head = if cleaned.is_empty() {
        "function"
    } else {
        cleaned
    };
    // A C identifier never starts with a digit.
    let head = if head.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{head}")
    } else {
        head.to_owned()
    };
    format!("{head}_{base}")
}

/// The type applied at an address, and what it holds.
fn reading_pane(app: &mut DesdecApp, ui: &mut egui::Ui, language: Language) -> Asked {
    let mut asked = Asked::default();
    card(ui, text(language, Text::ReadAtAnAddress), |ui| {
        if app.structures.registry.is_empty() {
            ui.label(egui::RichText::new(text(language, Text::NoTypesDefined)).color(MUTED));
            return;
        }

        chooser(app, ui, language);
        ui.add_space(8.0);

        let Some(applied) = app.structures.applied.clone() else {
            ui.label(egui::RichText::new(text(language, Text::ChooseAType)).color(MUTED));
            return;
        };
        let Some(address) = address_of(app) else {
            ui.label(
                egui::RichText::new(text(language, Text::StructuresNeedAnAddress)).color(MUTED),
            );
            return;
        };

        // Three places the bytes can come from, and the pane says which: the
        // file on disk when the reader asked for an offset, the machine's
        // memory when one is running, and the file at the address its sections
        // map to otherwise.
        let said = if app.structures.by_file_offset {
            Text::StructuresReadFromTheFileItself
        } else if app.machine.is_some() {
            Text::StructuresReadFromTheMachine
        } else {
            Text::StructuresReadFromTheFile
        };
        ui.label(egui::RichText::new(text(language, said)).color(MUTED));
        ui.add_space(8.0);

        let reading = read_now(app, &applied, address);
        if !reading.any_known() {
            ui.colored_label(ERROR, text(language, Text::NothingMappedHere));
            ui.add_space(8.0);
        }
        asked = tree(app, ui, &reading, address, language);
    });
    asked
}

/// Which type, where, and the way back out of a followed pointer.
fn chooser(app: &mut DesdecApp, ui: &mut egui::Ui, language: Language) {
    ui.horizontal(|ui| {
        let names: Vec<String> = app
            .structures
            .registry
            .all()
            .map(|definition| definition.name().to_owned())
            .collect();
        let mut applied = app
            .structures
            .applied
            .clone()
            .unwrap_or_else(|| names.first().cloned().unwrap_or_default());
        egui::ComboBox::from_id_salt("desdec.structures.applied")
            .selected_text(applied.clone())
            .show_ui(ui, |ui| {
                for name in &names {
                    if ui.selectable_label(*name == applied, name).clicked() {
                        applied.clone_from(name);
                    }
                }
            });
        if app.structures.applied.as_deref() != Some(applied.as_str()) {
            app.structures.applied = Some(applied);
            app.structures.open.clear();
        }

        ui.label(text(
            language,
            if app.structures.by_file_offset {
                Text::Offset
            } else {
                Text::Address
            },
        ));
        ui.add(
            egui::TextEdit::singleline(&mut app.structures.address)
                .desired_width(150.0)
                .font(egui::TextStyle::Monospace)
                .hint_text(if app.structures.by_file_offset {
                    "0"
                } else {
                    "rbx"
                }),
        );
        ui.checkbox(
            &mut app.structures.by_file_offset,
            text(language, Text::InTheFileItself),
        );

        if !app.structures.trail.is_empty()
            && ui
                .button(text(language, Text::BackToWhereItCameFrom))
                .clicked()
        {
            if let Some((kind, address)) = app.structures.trail.pop() {
                app.structures.applied = Some(kind).filter(|name| !name.is_empty());
                app.structures.address = address;
                app.structures.open.clear();
            }
        }
    });
}

/// Where the reader asked to read, in the language the conditions and the
/// calculator are already written in.
fn address_of(app: &DesdecApp) -> Option<u64> {
    let source = app.structures.address.trim();
    if source.is_empty() {
        return None;
    }
    let names = &app.names;
    let parsed = Expression::parse_naming(source, &|name| names.address_of(name)).ok()?;
    if app.structures.by_file_offset {
        // An offset into the file has nothing to do with a run, so a register
        // in one has no value rather than the value it happens to hold.
        return parsed.value(
            &desdec_core::emulate::registers::Registers::new(),
            &desdec_core::emulate::memory::Memory::new(std::sync::Arc::from(Vec::new())),
        );
    }
    match app.machine.as_ref() {
        Some(machine) => parsed.value(&machine.registers, &machine.memory),
        // With no machine there are no registers, so an address written as one
        // has no value — which the pane says, rather than reading at zero.
        None => parsed.value(
            &desdec_core::emulate::registers::Registers::new(),
            &desdec_core::emulate::memory::Memory::new(std::sync::Arc::from(Vec::new())),
        ),
    }
}

/// Reads the applied type at an address, out of whichever bytes there are.
fn read_now(app: &DesdecApp, applied: &str, address: u64) -> Reading {
    let kind = Type::Named(applied.to_owned());
    let depth = app.structures.depth();
    if app.structures.by_file_offset {
        let file = Flat {
            base: 0,
            bytes: &app.file_bytes,
        };
        return read::read(&app.structures.registry, &kind, address, &file, depth);
    }
    if let Some(machine) = app.machine.as_ref() {
        return read::read(
            &app.structures.registry,
            &kind,
            address,
            &machine.memory,
            depth,
        );
    }
    let sections = app
        .analysis
        .as_ref()
        .map_or([].as_slice(), |analysis| analysis.sections.as_slice());
    let image = FileImage {
        sections,
        bytes: &app.file_bytes,
    };
    read::read(&app.structures.registry, &kind, address, &image, depth)
}

/// The members, one row each, indented by how deep they sit.
fn tree(
    app: &mut DesdecApp,
    ui: &mut egui::Ui,
    reading: &Reading,
    base: u64,
    language: Language,
) -> Asked {
    let mut asked = Asked::default();
    let mut toggled = None;
    egui::Grid::new("desdec.structures.tree")
        .num_columns(4)
        .spacing([16.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            ui.strong(text(language, Text::Offset));
            ui.strong(text(language, Text::Member));
            ui.strong(text(language, Text::Type));
            ui.strong(text(language, Text::Value));
            ui.end_row();

            for member in &reading.members {
                rows(
                    app,
                    ui,
                    member,
                    base,
                    0,
                    &member.name.clone(),
                    language,
                    &mut asked,
                    &mut toggled,
                );
            }
        });
    if let Some(path) = toggled {
        if !app.structures.open.remove(&path) {
            app.structures.open.insert(path);
        }
    }
    asked
}

/// One member, and its own members when it is open.
#[expect(
    clippy::too_many_arguments,
    reason = "a row is drawn from its place in the tree, its path and what it can ask for"
)]
fn rows(
    app: &DesdecApp,
    ui: &mut egui::Ui,
    member: &Reading,
    base: u64,
    depth: usize,
    path: &str,
    language: Language,
    asked: &mut Asked,
    toggled: &mut Option<String>,
) {
    let open = app.structures.open.contains(path);
    let expandable = !member.members.is_empty();

    monospace_value(ui, &format!("+{:#06x}", member.address.wrapping_sub(base)));

    ui.horizontal(|ui| {
        // A row is nested a handful deep at most, so the cast is exact.
        #[expect(
            clippy::cast_precision_loss,
            reason = "a tree of members is never thousands of levels deep"
        )]
        ui.add_space(depth as f32 * 14.0);
        if expandable {
            let arrow = if open { "▾" } else { "▸" };
            if ui
                .add(egui::Button::new(arrow).frame(false).small())
                .clicked()
            {
                *toggled = Some(path.to_owned());
            }
        } else {
            ui.add_space(16.0);
        }
        ui.monospace(&member.name);
    });

    let written = match member.bits {
        Some((_, width)) => format!("{} : {width}", member.type_label),
        None => member.type_label.clone(),
    };
    ui.label(egui::RichText::new(written).color(MUTED));

    value_cell(ui, member, language, asked);
    ui.end_row();

    if open {
        for child in &member.members {
            rows(
                app,
                ui,
                child,
                base,
                depth + 1,
                &format!("{path}.{}", child.name),
                language,
                asked,
                toggled,
            );
        }
    }
}

/// What the member holds, said the way its type means it.
fn value_cell(ui: &mut egui::Ui, member: &Reading, language: Language, asked: &mut Asked) {
    match &member.value {
        Value::Unreadable => {
            ui.label(egui::RichText::new(text(language, Text::NotReadable)).color(MUTED));
        }
        Value::Undefined(error) => {
            ui.colored_label(ERROR, error.to_string());
        }
        Value::Aggregate => {
            ui.label(egui::RichText::new("…").color(MUTED));
        }
        Value::Address(address) => {
            ui.horizontal(|ui| {
                monospace_value(ui, &format!("{address:#x}"));
                if let Some(note) = &member.note {
                    ui.label(egui::RichText::new(format!("\"{note}\"")).color(MUTED));
                }
                if *address != 0 {
                    pointer_buttons(ui, member, *address, language, asked);
                }
            });
        }
        Value::Text(said) => {
            monospace_value(ui, &format!("\"{said}\""));
        }
        Value::Enumerated { value, name } => match name {
            Some(name) => {
                ui.horizontal(|ui| {
                    ui.monospace(name);
                    ui.label(egui::RichText::new(format!("({value})")).color(MUTED));
                });
            }
            None => {
                monospace_value(ui, &value.to_string());
            }
        },
        Value::Character(byte) => {
            let shown = match byte {
                0x20..=0x7e => format!("'{}' ({byte})", char::from(*byte)),
                _ => format!("{byte}"),
            };
            monospace_value(ui, &shown);
        }
        Value::Bool(state) => {
            monospace_value(ui, if *state { "true" } else { "false" });
        }
        Value::Float(number) => {
            monospace_value(ui, &format!("{number}"));
        }
        Value::Signed(number) => {
            #[expect(
                clippy::cast_sign_loss,
                reason = "the hexadecimal beside it is the same bits, which is what it is for"
            )]
            monospace_value(ui, &written_number(*number as u64, number.to_string()));
        }
        Value::Unsigned(number) => {
            monospace_value(ui, &written_number(*number, number.to_string()));
        }
    }
}

/// Following a pointer, and going where it points in the listing.
fn pointer_buttons(
    ui: &mut egui::Ui,
    member: &Reading,
    address: u64,
    language: Language,
    asked: &mut Asked,
) {
    if let Type::Pointer(inner) = &member.kind {
        if let Type::Named(name) = inner.as_ref() {
            if ui
                .small_button(text(language, Text::FollowThisPointer))
                .clicked()
            {
                asked.follow = Some((name.clone(), address));
            }
        }
    }
    if ui
        .small_button(text(language, Text::GoToThisAddress))
        .clicked()
    {
        asked.go_to = Some(address);
    }
}

/// A number, with the hexadecimal beside it once it is long enough to be
/// worth reading that way.
fn written_number(raw: u64, decimal: String) -> String {
    if raw > 9 {
        return format!("{decimal} ({raw:#x})");
    }
    decimal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::WorkspaceView,
        testing::{drawn, drawn_text, opened_app, window_input},
    };

    /// One frame of the view, and everything it said.
    fn said_by(app: &mut DesdecApp) -> String {
        let ctx = egui::Context::default();
        let output = ctx.run(window_input(), |ctx| {
            crate::ui::views::show_central_panel(app, ctx);
        });
        drawn_text(&output.shapes)
    }

    /// An address in the file that has bytes behind it.
    fn readable_address(app: &DesdecApp) -> u64 {
        app.analysis
            .as_ref()
            .expect("an analysis")
            .sections
            .iter()
            .find(|section| section.is_mapped() && section.file_size >= 32)
            .map(|section| section.virtual_address)
            .expect("a mapped section with bytes in it")
    }

    /// The whole point, end to end: a definition typed, applied, and the
    /// members named on screen at their own offsets.
    #[test]
    fn a_definition_applied_to_an_address_names_its_members() {
        let mut app = opened_app(WorkspaceView::Structures);
        app.structures.source =
            "struct Header { unsigned int magic; unsigned short version; char tag[4]; };"
                .to_owned();
        app.structures.reread();
        app.structures.applied = Some("Header".to_owned());
        app.structures.address = format!("{:#x}", readable_address(&app));

        let said = said_by(&mut app);
        for member in ["magic", "version", "tag"] {
            assert!(said.contains(member), "the view names {member}: {said}");
        }
        assert!(
            said.contains("+0x0004"),
            "and says where each one sits: {said}"
        );
    }

    /// A half-typed definition must not take the working ones away: the reader
    /// is in the middle of a sentence, not asking for everything to be undone.
    #[test]
    fn a_source_that_stops_making_sense_keeps_the_last_one_that_did() {
        let mut state = State {
            source: "struct S { int a; };".to_owned(),
            ..State::default()
        };
        state.reread();
        assert_eq!(state.registry.len(), 1);

        state.source = "struct S { int a; struct".to_owned();
        state.reread();
        assert_eq!(
            state.registry.len(),
            1,
            "the good definition is still there"
        );
        assert!(state.refused.is_some(), "and the reason is said");

        state.source = "struct S { int a; int b; };".to_owned();
        state.reread();
        assert!(state.refused.is_none());
        assert_eq!(
            state
                .registry
                .members_of(&Type::Named("S".to_owned()), 8)
                .expect("laid out")
                .len(),
            2
        );
    }

    /// Emptying the source empties the registry: a type nothing defines any
    /// more must not stay applied.
    #[test]
    fn deleting_every_definition_leaves_nothing_applied() {
        let mut state = State {
            source: "struct S { int a; };".to_owned(),
            applied: Some("S".to_owned()),
            ..State::default()
        };
        state.reread();

        state.source.clear();
        state.reread();
        assert!(state.registry.is_empty());
        assert!(state.applied.is_none());
    }

    /// A type deleted from the source while it was applied stops being
    /// applied, rather than staying on screen as a name nothing answers to.
    #[test]
    fn a_type_that_is_deleted_stops_being_the_one_applied() {
        let mut state = State {
            source: "struct A { int a; }; struct B { int b; };".to_owned(),
            applied: Some("B".to_owned()),
            ..State::default()
        };
        state.reread();

        state.source = "struct A { int a; };".to_owned();
        state.reread();
        assert_eq!(state.applied.as_deref(), Some("A"));
    }

    #[test]
    fn the_file_is_read_at_the_address_its_sections_map_to() {
        let app = opened_app(WorkspaceView::Structures);
        let analysis = app.analysis.as_ref().expect("an analysis");
        let section = analysis
            .sections
            .iter()
            .find(|section| section.is_mapped() && section.file_size >= 8)
            .expect("a mapped section");
        let image = FileImage {
            sections: &analysis.sections,
            bytes: &app.file_bytes,
        };

        let mut read = [0u8; 4];
        assert!(image.read(section.virtual_address, &mut read));
        let offset = usize::try_from(section.file_offset).expect("a file offset");
        assert_eq!(
            read,
            app.file_bytes[offset..offset + 4],
            "the bytes are the file's own, at the offset the section names"
        );

        assert!(
            !image.read(0xffff_0000_0000, &mut read),
            "and an address no section maps reads nothing"
        );
    }

    /// A section that is mapped but stores nothing — `.bss` and its kin — must
    /// not answer with zeroes the file never held.
    #[test]
    fn a_section_the_file_stores_no_bytes_for_reads_nothing() {
        let sections = vec![Section {
            name: ".bss".to_owned(),
            virtual_address: 0x5000,
            file_offset: 0,
            virtual_size: 0x1000,
            file_size: 0,
            permissions: desdec_core::Permissions {
                read: true,
                write: true,
                execute: false,
            },
            entropy: None,
        }];
        let image = FileImage {
            sections: &sections,
            bytes: &[0xff; 64],
        };
        let mut read = [0u8; 4];
        assert!(!image.read(0x5000, &mut read));
    }

    /// The reading goes no deeper than what is open, so a type that nests
    /// through arrays cannot be asked for more rows than a screen can hold.
    #[test]
    fn how_deep_the_reading_goes_follows_what_the_reader_has_opened() {
        let mut state = State::default();
        assert_eq!(state.depth().levels, 1, "closed, only the members");

        state.open.insert("outer".to_owned());
        assert_eq!(state.depth().levels, 2);
        state.open.insert("outer.inner.deeper".to_owned());
        assert_eq!(state.depth().levels, 4);

        for step in 0..20 {
            state.open.insert("a.".repeat(step) + "z");
        }
        assert!(
            state.depth().levels <= DEPTH_LIMIT + 1,
            "however much is opened"
        );
    }

    /// Everything the view drew, with where it drew it.
    fn rendered() -> Vec<(String, egui::Pos2)> {
        let mut app = opened_app(WorkspaceView::Structures);
        app.structures.source = String::from(
            "struct Header {
                 unsigned int magic;
                 unsigned short version;
                 char name[8];
                 struct Header *next;
             };",
        );
        app.structures.reread();
        app.structures.applied = Some(String::from("Header"));
        app.structures.address = format!("{:#x}", readable_address(&app));

        let ctx = egui::Context::default();
        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| super::show(&mut app, ui));
        };
        // Two frames: a panel is measured on the first and painted after.
        let _ = ctx.run(window_input(), &mut draw);
        let output = ctx.run(window_input(), &mut draw);
        drawn(&output.shapes)
    }

    /// Four columns of a member's row, drawn one over another, read as a smear
    /// rather than as four facts — and no assertion about the strings would
    /// ever notice, since they are all on screen either way. Only where they
    /// landed says it.
    #[test]
    fn nothing_in_the_view_is_drawn_on_top_of_anything_else() {
        let mut seen: Vec<egui::Pos2> = Vec::new();
        for (said, at) in rendered() {
            assert!(
                !seen.contains(&at),
                "{said:?} is drawn on top of something else, at {at:?}"
            );
            seen.push(at);
        }
    }

    /// The test that ties the whole module to something already known: the
    /// entry point read out of the ELF header, through declarations the reader
    /// could have typed, must be the entry point the analysis found by other
    /// means entirely.
    #[test]
    fn the_files_own_header_read_through_its_format_says_what_the_analysis_says() {
        let mut app = opened_app(WorkspaceView::Structures);
        let format = app.analysis.as_ref().expect("an analysis").summary.format;
        let source = catalogue::of(format).expect("the host's binary is an ELF, a PE or a Mach-O");
        app.structures.source = source.to_owned();
        app.structures.reread();

        let header = catalogue::header_of(format).expect("a header structure");
        app.structures.applied = Some(header.to_owned());
        // The header is at the start of the file and no section maps it.
        app.structures.by_file_offset = true;
        app.structures.address = String::from("0");

        let reading = super::read_now(&app, header, 0);
        // The field each format opens its header with, or names its entry
        // point in: `e_entry` for an ELF, `e_magic` for a PE, `magic` for a
        // Mach-O. Listing only the first two made this a test of the host —
        // it passed on Linux and failed on the macOS runner, on a header it
        // had read perfectly well.
        let known = reading
            .members
            .iter()
            .find(|member| matches!(member.name.as_str(), "e_entry" | "e_magic" | "magic"))
            .expect("a member of the header");
        assert!(known.value.is_known(), "the header's bytes are there");

        // And where the header carries the entry point, it must be the one
        // the analysis found by other means entirely — which is the tie this
        // test exists for. A Mach-O keeps its entry in a load command rather
        // than in the header, so there it is the reading above that stands.
        if let Some(expected) = app
            .analysis
            .as_ref()
            .and_then(|analysis| analysis.entry_point)
        {
            if let Some(read) = reading
                .members
                .iter()
                .find(|member| member.name == "e_entry")
                .and_then(|member| member.value.as_u64())
            {
                assert_eq!(
                    read, expected,
                    "the entry point read through the declarations is the one the analysis found"
                );
            }
        }
    }

    /// Reading a structure out of a function writes C into the editor, beside
    /// whatever the reader had already written there.
    #[test]
    fn what_is_read_out_of_the_code_is_added_to_what_the_reader_wrote() {
        let mut app = opened_app(WorkspaceView::Structures);
        app.structures.source = "struct Mine { int a; };\n".to_owned();
        app.structures.reread();
        app.structures.base = "rdi".to_owned();

        // A function with a body to read, whichever the host's binary has.
        let start = app
            .functions
            .iter()
            .find(|function| function.end.saturating_sub(function.start) > 64)
            .map(|function| function.start)
            .expect("a function with a body");
        let called = app
            .functions
            .iter()
            .find(|function| function.start == start)
            .map(|function| function.name.clone())
            .expect("its name");
        super::read_out_of_the_code(&mut app, start, &called);

        assert!(
            app.structures.source.starts_with("struct Mine"),
            "what the reader wrote is still there: {}",
            app.structures.source
        );
        assert!(
            app.structures.inferred.is_some(),
            "and the reading says what it found"
        );
        // Whatever was written must read back: a draft that does not parse is
        // a draft the reader has to fix before they can use it.
        assert!(
            parse::definitions(&app.structures.source).is_ok(),
            "the source still reads: {}",
            app.structures.source
        );
    }

    /// A name read out of the code has to be a C identifier, whatever the
    /// symbol table called the function.
    #[test]
    fn a_name_read_out_of_the_code_is_one_c_accepts() {
        assert_eq!(super::suggested_name("main", "rdi"), "main_rdi");
        assert_eq!(
            super::suggested_name("_ZN4core3fmtE", "rsi"),
            "ZN4core3fmtE_rsi"
        );
        assert_eq!(super::suggested_name("sub_4010a0", "rbx"), "sub_4010a0_rbx");
        assert_eq!(
            super::suggested_name("42nd", "rdi"),
            "_42nd_rdi",
            "and never starts with a digit"
        );
        assert_eq!(super::suggested_name("", "rdi"), "function_rdi");
    }

    /// The reference binary with a saying already made about a function that
    /// really does reach through its stack pointer.
    ///
    /// The largest function used to stand in for this. On the x86-64 ELF a
    /// Linux run analyses, the largest function reliably keeps a frame; on the
    /// aarch64 Mach-O the macOS runner analyses, it is four megabytes of
    /// `br x16` stubs whose only touch of `sp` is a pre-indexed push at a
    /// negative offset — outside anything the pointer points at, and rightly
    /// named nothing. Both tests below then failed on a claim neither was
    /// making: what they are about is that a saying names the rows, not that
    /// the biggest function in any binary anywhere happens to keep a frame.
    ///
    /// So the frame is looked for rather than assumed. A binary in which no
    /// function at all reaches through its stack pointer is itself a failure —
    /// that is what `name_accesses` not knowing an architecture looks like —
    /// and the panic says so.
    fn app_saying_about_a_frame() -> (DesdecApp, u64, &'static str) {
        // Bodies are walked instruction by instruction, and a stripped binary
        // holds one function of a million of them: the ones worth trying are
        // the ordinary ones, and a frame turns up in the first few.
        const LONGEST_TRIED: usize = 4096;

        let mut app = opened_app(WorkspaceView::Structures);
        app.structures.source = FRAME.to_owned();
        app.structures.reread();
        let register = crate::testing::stack_register();

        let candidates: Vec<u64> = app
            .functions
            .iter()
            .filter(|function| {
                app.analysis
                    .as_ref()
                    .is_some_and(|analysis| function.body(analysis).len() <= LONGEST_TRIED)
            })
            .map(|function| function.start)
            .collect();

        for start in candidates {
            app.annotations.say_in_code(crate::annotations::InCode {
                function: start,
                register: register.to_owned(),
                kind: String::from("Frame"),
            });
            app.rebuild_member_names();
            if !app.member_names.is_empty() {
                return (app, start, register);
            }
            app.annotations.forget_in_code(start, register);
        }
        panic!("no function of the host's binary reaches through {register}");
    }

    /// One structure wide enough to cover a frame, used by both tests below.
    const FRAME: &str = "struct Frame { unsigned long long slot[64]; };";

    /// A saying about one register in one function names every access it makes
    /// through it — which is the column of offsets a reader translates in
    /// their head, translated.
    #[test]
    fn a_saying_about_a_register_names_the_rows_that_go_through_it() {
        let (app, _, _) = app_saying_about_a_frame();
        let named = app
            .member_names
            .get(
                *app.member_names
                    .by_address
                    .keys()
                    .next()
                    .expect("at least one row"),
            )
            .expect("a name for it");
        assert!(
            named.starts_with("slot["),
            "the name is the member the offset falls in: {named}"
        );
    }

    /// A type the reader has deleted names nothing: a listing labelled from a
    /// definition that is no longer there would be showing what it was told
    /// once and never since.
    #[test]
    fn a_saying_about_a_type_that_no_longer_exists_names_nothing() {
        let (mut app, _, _) = app_saying_about_a_frame();

        app.structures.source.clear();
        app.structures.reread();
        app.rebuild_member_names();
        assert!(
            app.member_names.is_empty(),
            "the saying stands, but there is no type to read it through"
        );
    }

    /// Saying it twice about the same register is saying it once: the second
    /// is a correction, not a second reading.
    #[test]
    fn saying_it_again_about_one_register_replaces_what_was_said() {
        let mut app = opened_app(WorkspaceView::Structures);
        for kind in ["A", "B"] {
            app.annotations.say_in_code(crate::annotations::InCode {
                function: 0x1000,
                register: "rdi".to_owned(),
                kind: kind.to_owned(),
            });
        }
        assert_eq!(app.annotations.in_code().len(), 1);
        assert_eq!(app.annotations.in_code()[0].kind, "B");

        app.annotations.forget_in_code(0x1000, "rdi");
        assert!(app.annotations.in_code().is_empty());
    }

    /// The definitions are the reader's work on one binary, and come back with
    /// it rather than following them to the next file.
    #[test]
    fn the_definitions_are_kept_with_the_notes_about_the_binary() {
        let mut app = opened_app(WorkspaceView::Structures);
        app.structures.source = "struct S { int a; };".to_owned();
        app.structures.reread();

        let ctx = egui::Context::default();
        let _ = ctx.run(window_input(), |ctx| {
            crate::ui::views::show_central_panel(&mut app, ctx);
            crate::ui::status_bar::show(&mut app, ctx);
        });
        app.persist_settled_annotations_for_a_test(&ctx);

        assert_eq!(
            app.annotations.types(),
            "struct S { int a; };",
            "what was typed is part of what is written about this binary"
        );
    }
}
