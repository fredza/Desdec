//! What a script can say, and what the application does about it.
//!
//! Every function here reads the subject it was handed and nothing else. The
//! ones that change something do not change it: they put an [`Effect`] on a
//! list, after checking that the script was granted the permission it needs,
//! and the application applies the list when the script has finished.
//!
//! Addresses are their own type rather than plain integers. A binary mapped
//! high — a kernel module at `0xffff_ffff_8100_0000`, a driver, anything past
//! the middle of the address space — has addresses that do not fit the
//! engine's signed integer, and a comparison against one would silently come
//! out backwards. [`Address`] holds all sixty-four bits, compares as the
//! unsigned value it is, and prints as hexadecimal, which is how a reader
//! writes an address down anyway.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};

use desdec_core::{Analysis, Instruction, Section};
use rhai::{Array, Blob, Dynamic, Engine, EvalAltResult, INT, Map, Position};

use crate::{i18n::Language, patches, search, xrefs};

use super::{Bound, Context, Effect, Failure, Limits, Outcome, Permission, Subject};

/// A place in the binary, in all sixty-four bits.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Address(pub u64);

impl std::fmt::Display for Address {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:#x}", self.0)
    }
}

/// What a running script has done so far, and what it is allowed to do.
pub struct State {
    granted: Vec<Permission>,
    limits: Limits,
    /// The reader's language, so a refusal from the assembler reads the same
    /// here as it does in the patch editor.
    language: Language,
    deadline: Instant,
    /// Counted so the clock is read once in a while rather than on every
    /// operation: `Instant::now()` fifty million times is itself a second.
    steps: Cell<u64>,
    effects: RefCell<Vec<Effect>>,
    printed: RefCell<Vec<String>>,
    /// The bound that was reached, which is also what stops the script: the
    /// next progress callback sees it and terminates the run.
    bound: Cell<Option<Bound>>,
    /// The first permission that was missing, kept so the reader is told what
    /// to grant rather than shown the engine's error about a failed call.
    refused: Cell<Option<Permission>>,
}

/// How often the wall clock is read, in engine operations.
const CLOCK_INTERVAL: u64 = 4096;

impl State {
    pub fn new(context: &Context) -> Self {
        Self {
            granted: context.granted.clone(),
            limits: context.limits,
            language: context.language,
            deadline: Instant::now() + context.limits.time,
            steps: Cell::new(0),
            effects: RefCell::new(Vec::new()),
            printed: RefCell::new(Vec::new()),
            bound: Cell::new(None),
            refused: Cell::new(None),
        }
    }

    /// Called by the engine as it works; `Some` stops the script.
    pub fn tick(&self) -> Option<Dynamic> {
        if self.bound.get().is_some() {
            return Some(Dynamic::UNIT);
        }
        let steps = self.steps.get().wrapping_add(1);
        self.steps.set(steps);
        if steps % CLOCK_INTERVAL != 0 {
            return None;
        }
        if Instant::now() >= self.deadline {
            self.bound.set(Some(Bound::Time));
            return Some(Dynamic::UNIT);
        }
        None
    }

    pub fn print(&self, line: &str) {
        let mut printed = self.printed.borrow_mut();
        if printed.len() >= self.limits.printed {
            self.bound.set(Some(Bound::Printed));
            return;
        }
        printed.push(line.to_owned());
    }

    /// Records what a script asked for, once its permission is checked.
    fn ask(&self, effect: Effect) -> Result<(), Box<EvalAltResult>> {
        let permission = effect.permission();
        if !self.granted.contains(&permission) {
            self.refused.set(Some(permission));
            return Err(error("this script was not granted that permission"));
        }
        let mut effects = self.effects.borrow_mut();
        if effects.len() >= self.limits.effects {
            self.bound.set(Some(Bound::Effects));
            return Err(error("too many changes asked for"));
        }
        effects.push(effect);
        Ok(())
    }

    /// Reads back everything the run produced.
    ///
    /// The effects and the printed lines are kept whatever went wrong: a
    /// script that names three hundred functions and then fails on its last
    /// line has still found three hundred names.
    pub fn outcome(
        &self,
        result: Result<(), Box<EvalAltResult>>,
        elapsed: std::time::Duration,
    ) -> Outcome {
        let failure = if let Some(permission) = self.refused.get() {
            Some(Failure::Refused(permission))
        } else if let Some(bound) = self.bound.get() {
            Some(Failure::Interrupted(bound))
        } else {
            match result {
                Ok(()) => None,
                Err(error) => Some(match *error {
                    EvalAltResult::ErrorTooManyOperations(_) => {
                        Failure::Interrupted(Bound::Operations)
                    }
                    EvalAltResult::ErrorTerminated(..) => Failure::Interrupted(Bound::Time),
                    other => Failure::Faulted(other.to_string()),
                }),
            }
        };
        Outcome {
            printed: self.printed.take(),
            effects: self.effects.take(),
            failure,
            elapsed,
        }
    }
}

/// A refusal the script sees, in the engine's own terms.
#[expect(
    clippy::unnecessary_box_returns,
    reason = "the engine's own error type for a native function is a boxed one"
)]
fn error(message: impl Into<String>) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(
        Dynamic::from(message.into()),
        Position::NONE,
    ))
}

/// Reads an address from whatever the script passed: an [`Address`], or a
/// plain integer written the way anyone writes one down.
fn wanted_address(value: &Dynamic) -> Result<u64, Box<EvalAltResult>> {
    if let Some(address) = value.clone().try_cast::<Address>() {
        return Ok(address.0);
    }
    if let Some(number) = value.clone().try_cast::<INT>() {
        #[expect(
            clippy::cast_sign_loss,
            reason = "an address written as a negative integer is the same sixty-four bits"
        )]
        return Ok(number as u64);
    }
    Err(error(format!(
        "expected an address, found {}",
        value.type_name()
    )))
}

/// The open binary, or a refusal naming what is missing.
fn analysis(subject: &Subject) -> Result<&Analysis, Box<EvalAltResult>> {
    subject
        .analysis
        .as_ref()
        .ok_or_else(|| error("no binary is open"))
}

fn count(value: INT) -> usize {
    usize::try_from(value).unwrap_or(0)
}

/// Registers everything a script can call.
#[expect(
    clippy::too_many_lines,
    reason = "this is the whole vocabulary a script has; splitting it hides what is in it"
)]
pub fn register(engine: &mut Engine, subject: &Rc<Subject>, state: &Rc<State>) {
    addresses(engine);

    // --- What the binary is -------------------------------------------------

    let held = Rc::clone(subject);
    engine.register_fn("binary", move || -> Result<Map, Box<EvalAltResult>> {
        let analysis = analysis(&held)?;
        let mut map = Map::new();
        map.insert(
            "name".into(),
            analysis
                .summary
                .path
                .file_name()
                .unwrap_or(analysis.summary.path.as_os_str())
                .to_string_lossy()
                .into_owned()
                .into(),
        );
        map.insert(
            "format".into(),
            analysis.summary.format.label().to_string().into(),
        );
        map.insert(
            "architecture".into(),
            analysis.summary.architecture.label().to_string().into(),
        );
        map.insert("size".into(), clamp(analysis.summary.size).into());
        map.insert("analysed".into(), clamp(analysis.analysed_bytes).into());
        map.insert("truncated".into(), analysis.truncated.into());
        map.insert("code_truncated".into(), analysis.code_truncated.into());
        map.insert(
            "entropy".into(),
            analysis
                .entropy
                .map_or(Dynamic::UNIT, |entropy| Dynamic::from_float(entropy.into())),
        );
        map.insert(
            "sha256".into(),
            analysis
                .sha256
                .map_or(Dynamic::UNIT, |digest| Dynamic::from(hex(&digest))),
        );
        map.insert(
            "instructions".into(),
            clamp_usize(analysis.instructions.len()).into(),
        );
        Ok(map)
    });

    let held = Rc::clone(subject);
    engine.register_fn("entry", move || -> Result<Dynamic, Box<EvalAltResult>> {
        Ok(analysis(&held)?
            .entry_point
            .map_or(Dynamic::UNIT, |address| Dynamic::from(Address(address))))
    });

    let held = Rc::clone(subject);
    engine.register_fn("sections", move || -> Result<Array, Box<EvalAltResult>> {
        Ok(analysis(&held)?
            .sections
            .iter()
            .map(|section| Dynamic::from(section_map(section)))
            .collect())
    });

    let held = Rc::clone(subject);
    engine.register_fn(
        "section_at",
        move |address: Dynamic| -> Result<Dynamic, Box<EvalAltResult>> {
            let address = wanted_address(&address)?;
            Ok(analysis(&held)?
                .section_at(address)
                .map_or(Dynamic::UNIT, |section| Dynamic::from(section_map(section))))
        },
    );

    let held = Rc::clone(subject);
    engine.register_fn("functions", move || -> Array {
        held.functions
            .iter()
            .map(|function| {
                let mut map = Map::new();
                map.insert("name".into(), function.name.clone().into());
                map.insert("address".into(), Dynamic::from(Address(function.start)));
                map.insert("end".into(), Dynamic::from(Address(function.end)));
                map.insert(
                    "size".into(),
                    clamp(function.end.saturating_sub(function.start)).into(),
                );
                Dynamic::from(map)
            })
            .collect()
    });

    let held = Rc::clone(subject);
    engine.register_fn("symbols", move || -> Result<Array, Box<EvalAltResult>> {
        Ok(analysis(&held)?
            .symbols
            .iter()
            .map(|symbol| {
                let mut map = Map::new();
                map.insert("name".into(), symbol.name.clone().into());
                map.insert(
                    "address".into(),
                    symbol
                        .address
                        .map_or(Dynamic::UNIT, |address| Dynamic::from(Address(address))),
                );
                map.insert("size".into(), clamp(symbol.size).into());
                map.insert("imported".into(), symbol.imported.into());
                Dynamic::from(map)
            })
            .collect())
    });

    let held = Rc::clone(subject);
    engine.register_fn("strings", move || -> Result<Array, Box<EvalAltResult>> {
        let analysis = analysis(&held)?;
        Ok(analysis
            .strings
            .iter()
            .map(|string| {
                let mut map = Map::new();
                map.insert("text".into(), string.value.clone().into());
                map.insert("offset".into(), clamp(string.file_offset).into());
                map.insert(
                    "address".into(),
                    analysis
                        .address_at(string.file_offset)
                        .map_or(Dynamic::UNIT, |(address, _)| {
                            Dynamic::from(Address(address))
                        }),
                );
                map.insert("truncated".into(), string.truncated.into());
                Dynamic::from(map)
            })
            .collect())
    });

    // --- The listing --------------------------------------------------------

    let held = Rc::clone(subject);
    engine.register_fn(
        "instruction_count",
        move || -> Result<INT, Box<EvalAltResult>> {
            Ok(clamp_usize(analysis(&held)?.instructions.len()))
        },
    );

    let held = Rc::clone(subject);
    engine.register_fn(
        "instruction",
        move |address: Dynamic| -> Result<Dynamic, Box<EvalAltResult>> {
            let address = wanted_address(&address)?;
            let analysis = analysis(&held)?;
            Ok(analysis
                .instruction_at(address)
                .map_or(Dynamic::UNIT, |instruction| {
                    Dynamic::from(instruction_map(instruction, analysis))
                }))
        },
    );

    let held = Rc::clone(subject);
    engine.register_fn(
        "instruction_at_index",
        move |index: INT| -> Result<Dynamic, Box<EvalAltResult>> {
            let analysis = analysis(&held)?;
            Ok(analysis
                .instructions
                .get(count(index))
                .map_or(Dynamic::UNIT, |instruction| {
                    Dynamic::from(instruction_map(instruction, analysis))
                }))
        },
    );

    let held = Rc::clone(subject);
    engine.register_fn(
        "instructions",
        move |from: Dynamic, upto: Dynamic| -> Result<Array, Box<EvalAltResult>> {
            let from = wanted_address(&from)?;
            let upto = wanted_address(&upto)?;
            let analysis = analysis(&held)?;
            Ok(analysis
                .instructions_in(from..upto)
                .iter()
                .map(|instruction| Dynamic::from(instruction_map(instruction, analysis)))
                .collect())
        },
    );

    // --- The bytes ----------------------------------------------------------

    let held = Rc::clone(subject);
    engine.register_fn(
        "read",
        move |address: Dynamic, length: INT| -> Result<Blob, Box<EvalAltResult>> {
            let address = wanted_address(&address)?;
            let analysis = analysis(&held)?;
            let offset = analysis
                .file_offset_of(address)
                .ok_or_else(|| error(format!("{address:#x} is not stored in the file")))?;
            Ok(slice(&held.file, offset, count(length)))
        },
    );

    let held = Rc::clone(subject);
    engine.register_fn("read_at_offset", move |offset: INT, length: INT| -> Blob {
        slice(
            &held.file,
            u64::try_from(offset).unwrap_or(0),
            count(length),
        )
    });

    let held = Rc::clone(subject);
    engine.register_fn(
        "offset_of",
        move |address: Dynamic| -> Result<Dynamic, Box<EvalAltResult>> {
            let address = wanted_address(&address)?;
            Ok(analysis(&held)?
                .file_offset_of(address)
                .map_or(Dynamic::UNIT, |offset| Dynamic::from(clamp(offset))))
        },
    );

    let held = Rc::clone(subject);
    engine.register_fn(
        "address_at",
        move |offset: INT| -> Result<Dynamic, Box<EvalAltResult>> {
            Ok(analysis(&held)?
                .address_at(u64::try_from(offset).unwrap_or(0))
                .map_or(Dynamic::UNIT, |(address, _)| {
                    Dynamic::from(Address(address))
                }))
        },
    );

    // --- Finding things -----------------------------------------------------

    let held = Rc::clone(subject);
    let limits = state.limits;
    engine.register_fn(
        "find_bytes",
        move |pattern: &str| -> Result<Array, Box<EvalAltResult>> {
            let analysis = analysis(&held)?;
            let parsed = search::Pattern::parse(pattern)
                .ok_or_else(|| error(format!("{pattern} is not a byte pattern")))?;
            let found = search::bytes_within(analysis, &held.file, &parsed, limits.hits);
            Ok(found.hits.iter().map(hit).collect())
        },
    );

    let held = Rc::clone(subject);
    engine.register_fn(
        "find_instructions",
        move |needle: &str| -> Result<Array, Box<EvalAltResult>> {
            let found = search::instructions_within(analysis(&held)?, needle, limits.hits);
            Ok(found.hits.iter().map(hit).collect())
        },
    );

    let held = Rc::clone(subject);
    engine.register_fn(
        "find_notes",
        move |needle: &str| -> Result<Array, Box<EvalAltResult>> {
            let found =
                search::notes_within(analysis(&held)?, &held.annotations, needle, limits.hits);
            Ok(found.hits.iter().map(hit).collect())
        },
    );

    // --- Who names an address -----------------------------------------------

    let held = Rc::clone(subject);
    engine.register_fn(
        "refs_to",
        move |address: Dynamic| -> Result<Array, Box<EvalAltResult>> {
            let address = wanted_address(&address)?;
            Ok(held
                .xrefs
                .to(address)
                .map(|reference| {
                    let mut map = Map::new();
                    map.insert("from".into(), Dynamic::from(Address(reference.from)));
                    map.insert("kind".into(), kind(reference.kind).into());
                    Dynamic::from(map)
                })
                .collect())
        },
    );

    let held = Rc::clone(subject);
    engine.register_fn(
        "ref_count",
        move |address: Dynamic| -> Result<INT, Box<EvalAltResult>> {
            let address = wanted_address(&address)?;
            Ok(clamp_usize(held.xrefs.count(address)))
        },
    );

    // --- What the reader already wrote --------------------------------------

    let held = Rc::clone(subject);
    engine.register_fn(
        "label_of",
        move |address: Dynamic| -> Result<Dynamic, Box<EvalAltResult>> {
            let address = wanted_address(&address)?;
            Ok(held
                .annotations
                .label(address)
                .map_or(Dynamic::UNIT, |label| Dynamic::from(label.to_owned())))
        },
    );

    let held = Rc::clone(subject);
    engine.register_fn(
        "comment_of",
        move |address: Dynamic| -> Result<Dynamic, Box<EvalAltResult>> {
            let address = wanted_address(&address)?;
            Ok(held
                .annotations
                .comment(address)
                .map_or(Dynamic::UNIT, |comment| Dynamic::from(comment.to_owned())))
        },
    );

    let held = Rc::clone(subject);
    engine.register_fn(
        "bookmarked",
        move |address: Dynamic| -> Result<bool, Box<EvalAltResult>> {
            let address = wanted_address(&address)?;
            Ok(held.annotations.is_bookmarked(address))
        },
    );

    let held = Rc::clone(subject);
    engine.register_fn("notes", move || -> Array {
        held.annotations
            .iter()
            .map(|(address, annotation)| {
                let mut map = Map::new();
                map.insert("address".into(), Dynamic::from(Address(address)));
                map.insert("label".into(), annotation.label.clone().into());
                map.insert("comment".into(), annotation.comment.clone().into());
                map.insert("bookmarked".into(), annotation.bookmarked.into());
                Dynamic::from(map)
            })
            .collect()
    });

    // --- What it asks for ---------------------------------------------------

    let asked = Rc::clone(state);
    engine.register_fn(
        "label",
        move |address: Dynamic, text: &str| -> Result<(), Box<EvalAltResult>> {
            asked.ask(Effect::Label {
                address: wanted_address(&address)?,
                text: text.to_owned(),
            })
        },
    );

    let asked = Rc::clone(state);
    engine.register_fn(
        "comment",
        move |address: Dynamic, text: &str| -> Result<(), Box<EvalAltResult>> {
            asked.ask(Effect::Comment {
                address: wanted_address(&address)?,
                text: text.to_owned(),
            })
        },
    );

    let asked = Rc::clone(state);
    engine.register_fn(
        "bookmark",
        move |address: Dynamic| -> Result<(), Box<EvalAltResult>> {
            asked.ask(Effect::Bookmark {
                address: wanted_address(&address)?,
                on: true,
            })
        },
    );

    let asked = Rc::clone(state);
    engine.register_fn(
        "unbookmark",
        move |address: Dynamic| -> Result<(), Box<EvalAltResult>> {
            asked.ask(Effect::Bookmark {
                address: wanted_address(&address)?,
                on: false,
            })
        },
    );

    let asked = Rc::clone(state);
    engine.register_fn(
        "clear_note",
        move |address: Dynamic| -> Result<(), Box<EvalAltResult>> {
            asked.ask(Effect::ClearNote {
                address: wanted_address(&address)?,
            })
        },
    );

    let asked = Rc::clone(state);
    engine.register_fn(
        "go_to",
        move |address: Dynamic| -> Result<(), Box<EvalAltResult>> {
            asked.ask(Effect::Goto {
                address: wanted_address(&address)?,
            })
        },
    );

    let asked = Rc::clone(state);
    let held = Rc::clone(subject);
    engine.register_fn(
        "patch",
        move |address: Dynamic, line: &str| -> Result<(), Box<EvalAltResult>> {
            let address = wanted_address(&address)?;
            let analysis = analysis(&held)?;
            let bytes = encode(analysis, address, line, asked.language)?;
            asked.ask(Effect::Patch {
                address,
                bytes,
                source: line.to_owned(),
            })
        },
    );
}

/// The address type, and the arithmetic a script does with one.
fn addresses(engine: &mut Engine) {
    engine
        .register_type_with_name::<Address>("Address")
        .register_fn("to_string", |address: &mut Address| address.to_string())
        .register_fn("to_debug", |address: &mut Address| address.to_string())
        .register_fn("address", |value: INT| {
            #[expect(
                clippy::cast_sign_loss,
                reason = "an address written as a negative integer is the same sixty-four bits"
            )]
            Address(value as u64)
        })
        .register_get("hex", |address: &mut Address| address.to_string())
        // Lossy above the middle of the address space, and named so: a script
        // that wants a number out of an address has to say it wants one.
        .register_get("int", |address: &mut Address| {
            INT::try_from(address.0).unwrap_or(INT::MAX)
        })
        .register_fn("+", |address: Address, step: INT| {
            Address(offset_by(address.0, step))
        })
        .register_fn("-", |address: Address, step: INT| {
            Address(offset_by(address.0, step.saturating_neg()))
        })
        .register_fn("-", |left: Address, right: Address| {
            INT::try_from(left.0.abs_diff(right.0)).unwrap_or(INT::MAX)
                * if left.0 >= right.0 { 1 } else { -1 }
        })
        .register_fn("==", |left: Address, right: Address| left == right)
        .register_fn("!=", |left: Address, right: Address| left != right)
        .register_fn("<", |left: Address, right: Address| left < right)
        .register_fn("<=", |left: Address, right: Address| left <= right)
        .register_fn(">", |left: Address, right: Address| left > right)
        .register_fn(">=", |left: Address, right: Address| left >= right);

    // The same comparisons against a plain number, so `if address > 0x401000`
    // reads the way it is meant and not the way two's complement would have it.
    macro_rules! against_integers {
        ($($symbol:literal => $operator:tt),+ $(,)?) => {
            $(
                engine.register_fn($symbol, |left: Address, right: INT| {
                    #[expect(
                        clippy::cast_sign_loss,
                        reason = "the bits of the written number are the address meant"
                    )]
                    { left.0 $operator right as u64 }
                });
                engine.register_fn($symbol, |left: INT, right: Address| {
                    #[expect(
                        clippy::cast_sign_loss,
                        reason = "the bits of the written number are the address meant"
                    )]
                    { (left as u64) $operator right.0 }
                });
            )+
        };
    }
    against_integers!("==" => ==, "!=" => !=, "<" => <, "<=" => <=, ">" => >, ">=" => >=);
}

/// An address moved by a signed step, without wrapping past either end.
fn offset_by(address: u64, step: INT) -> u64 {
    if step >= 0 {
        address.saturating_add(step.unsigned_abs())
    } else {
        address.saturating_sub(step.unsigned_abs())
    }
}

/// Assembles one line for the address it would be written at.
///
/// The same rules the patch editor works by, because it is the same editor:
/// what encodes shorter than the instruction it replaces is filled out with
/// `nop`, and what encodes longer is refused rather than moving every byte
/// after it.
fn encode(
    analysis: &Analysis,
    address: u64,
    line: &str,
    language: Language,
) -> Result<Vec<u8>, Box<EvalAltResult>> {
    let instruction = analysis
        .instruction_at(address)
        .ok_or_else(|| error(format!("no instruction is decoded at {address:#x}")))?;
    let file_offset = analysis
        .file_offset_of(address)
        .ok_or_else(|| error(format!("{address:#x} is not stored in the file")))?;
    let mut editor = patches::Editor::new(instruction, file_offset);
    line.clone_into(&mut editor.assembly);
    match editor.assembled(analysis.summary.architecture) {
        patches::Assembled::Fits { bytes, .. } => Ok(bytes),
        patches::Assembled::TooLong { encoded, room } => Err(error(format!(
            "{line} encodes to {encoded} bytes, and there is room for {room}"
        ))),
        patches::Assembled::Refused(reason) => Err(error(patches::refusal(&reason, language))),
        patches::Assembled::Empty => Err(error("nothing to assemble")),
    }
}

fn section_map(section: &Section) -> Map {
    let mut map = Map::new();
    map.insert("name".into(), section.name.clone().into());
    map.insert(
        "address".into(),
        Dynamic::from(Address(section.virtual_address)),
    );
    map.insert("size".into(), clamp(section.virtual_size).into());
    map.insert("offset".into(), clamp(section.file_offset).into());
    map.insert("stored".into(), clamp(section.file_size).into());
    map.insert("readable".into(), section.permissions.read.into());
    map.insert("writable".into(), section.permissions.write.into());
    map.insert("executable".into(), section.permissions.execute.into());
    map.insert("mapped".into(), section.is_mapped().into());
    map.insert(
        "entropy".into(),
        section
            .entropy
            .map_or(Dynamic::UNIT, |entropy| Dynamic::from_float(entropy.into())),
    );
    map
}

fn instruction_map(instruction: &Instruction, analysis: &Analysis) -> Map {
    let mut map = Map::new();
    map.insert(
        "address".into(),
        Dynamic::from(Address(instruction.address)),
    );
    map.insert("text".into(), instruction.text.clone().into());
    map.insert(
        "bytes".into(),
        Dynamic::from_blob(instruction.bytes.to_vec()),
    );
    map.insert(
        "size".into(),
        clamp_usize(instruction.bytes.to_vec().len()).into(),
    );
    map.insert("section".into(), instruction.section.to_string().into());
    map.insert(
        "offset".into(),
        analysis
            .file_offset_of(instruction.address)
            .map_or(Dynamic::UNIT, |offset| Dynamic::from(clamp(offset))),
    );
    map
}

fn hit(found: &search::Hit) -> Dynamic {
    let mut map = Map::new();
    map.insert(
        "address".into(),
        found
            .address
            .map_or(Dynamic::UNIT, |address| Dynamic::from(Address(address))),
    );
    map.insert(
        "offset".into(),
        found
            .file_offset
            .map_or(Dynamic::UNIT, |offset| Dynamic::from(clamp(offset))),
    );
    map.insert(
        "section".into(),
        found.section.clone().map_or(Dynamic::UNIT, Dynamic::from),
    );
    map.insert("text".into(), found.text.clone().into());
    Dynamic::from(map)
}

const fn kind(kind: xrefs::Kind) -> &'static str {
    match kind {
        xrefs::Kind::Call => "call",
        xrefs::Kind::Jump => "jump",
        xrefs::Kind::Reads => "reads",
        xrefs::Kind::Table => "table",
        xrefs::Kind::Stub => "stub",
        xrefs::Kind::Pointer => "pointer",
    }
}

/// Bytes of the file from an offset, and however few of them are there.
fn slice(file: &[u8], offset: u64, length: usize) -> Blob {
    let start = usize::try_from(offset).unwrap_or(usize::MAX);
    let end = start.saturating_add(length).min(file.len());
    file.get(start..end).unwrap_or_default().to_vec()
}

fn hex(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;

    digest.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// A size as a number the engine can hold, saturating rather than wrapping.
fn clamp(value: u64) -> INT {
    INT::try_from(value).unwrap_or(INT::MAX)
}

fn clamp_usize(value: usize) -> INT {
    INT::try_from(value).unwrap_or(INT::MAX)
}
