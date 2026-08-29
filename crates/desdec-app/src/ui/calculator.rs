//! The programmer's calculator: one value, read in every base at once.
//!
//! Nothing here is about the file open — it is the arithmetic a reader does on
//! paper beside the listing, which is why it is a window kept open beside the
//! views rather than a view of its own. What a disassembly makes a reader do a
//! dozen times an hour is exactly this: read `0x1f4` as five hundred, see which
//! bits a mask holds, work out what a field is worth once it has been shifted,
//! find out that `0x6c6c6568` spells `hell` backwards.
//!
//! One value, several readings. The six fields are not six numbers: they are
//! the same number written six ways, and typing in any of them moves all the
//! others. Below them the same value is shown a seventh way — as its bits, one
//! per cell, each of which can be pressed.
//!
//! It is worked out the way a calculator is: a number, an operation, a number,
//! `=`. The value on screen becomes the left-hand side, the operation waits —
//! visibly, beside the keys — and the next number entered is the right-hand
//! side. A second operation pressed before `=` works the first one out, so
//! `2 + 3 × 4 =` is twenty, the way it is on the thing in a drawer. That is
//! the whole difference from what stood here before, which was a box holding
//! an operand and a row of buttons: correct, and not how anybody was taught to
//! add.
//!
//! The keys enter digits in one base, chosen above them, and a digit the base
//! does not have cannot be pressed: `A` is a number in hexadecimal and a
//! mistake in decimal. Everything is also typeable in the six fields, which is
//! the faster way in when a number is long.
//!
//! The shift and rotate buttons are named for the instructions they stand for
//! — `shl`, `shr`, `sar`, `rol`, `ror` — because the reader is looking at a
//! listing full of them. `>>` would have had to mean one of `shr` and `sar`,
//! and it means the other one in half the languages a reader knows. They take
//! their right-hand side like every other operation: `shl 4 =` shifts by four.
//! The arithmetic is written in signs instead — `+`, `−`, `×`, `÷` — because
//! that is what a calculator has on it.
//!
//! Nothing is invented. A value too large for the chosen width is refused and
//! said to be too large, rather than quietly cut down to the bits that fit; a
//! byte that is not printable is written `·` in the string, which is not a
//! character anyone can type, so the field refuses it back rather than
//! pretending the value spelt something.

use eframe::egui;

use crate::{
    app::{DesdecApp, Dialog},
    i18n::{Language, Text, text},
    preferences::accent,
    ui::{ERROR, MUTED},
};

/// Size assumed the first time the window opens, before egui has measured it.
const ASSUMED_SIZE: egui::Vec2 = egui::vec2(580.0, 700.0);

/// How many bits the value is held in.
///
/// Not decoration: it is what decides whether `0xff` is `-1` or `255`, what a
/// rotation brings round to the bottom, and where a shift throws a bit away.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Width {
    Eight,
    Sixteen,
    ThirtyTwo,
    #[default]
    SixtyFour,
}

impl Width {
    pub const ALL: &'static [Self] =
        &[Self::Eight, Self::Sixteen, Self::ThirtyTwo, Self::SixtyFour];

    #[must_use]
    pub const fn bits(self) -> u32 {
        match self {
            Self::Eight => 8,
            Self::Sixteen => 16,
            Self::ThirtyTwo => 32,
            Self::SixtyFour => 64,
        }
    }

    /// The bits a value of this width is allowed to have.
    #[must_use]
    pub const fn mask(self) -> u64 {
        match self {
            // Answered here rather than by the shift below, which is what
            // keeps `1 << 64` — an overflow, not a number — from being asked
            // for at all.
            Self::SixtyFour => u64::MAX,
            _ => (1_u64 << self.bits()) - 1,
        }
    }

    /// The same bits read as a signed number of this width.
    #[must_use]
    #[expect(
        clippy::cast_possible_wrap,
        reason = "reading the same bits as signed is the point of the method"
    )]
    pub const fn signed(self, value: u64) -> i64 {
        let spare = 64 - self.bits();
        // Up to the top of the word and back down again: the arithmetic shift
        // brings the sign bit with it, which is what sign extension is.
        ((value << spare) as i64) >> spare
    }

    /// Named in digits, which read the same in all three languages.
    const fn label(self) -> &'static str {
        match self {
            Self::Eight => "8",
            Self::Sixteen => "16",
            Self::ThirtyTwo => "32",
            Self::SixtyFour => "64",
        }
    }
}

/// One reading of the value, and one editable field on screen.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Field {
    Hexadecimal,
    Decimal,
    /// The same bits read as a signed number, which is the reading a negative
    /// offset in a listing is written in.
    Signed,
    Octal,
    Binary,
    /// The bytes as characters, the most significant first.
    Ascii,
}

/// Why what was typed was not taken.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// It is not a number in this base at all.
    Unreadable,
    /// It is a number, and it does not fit in the chosen width.
    TooWide,
    /// A division by zero was asked for, which has no answer.
    ByZero,
}

impl Refusal {
    const fn text(self) -> Text {
        match self {
            Self::Unreadable => Text::NotReadableHere,
            Self::TooWide => Text::TooWideForTheWidth,
            Self::ByZero => Text::NoDivisionByZero,
        }
    }
}

impl Field {
    pub const ALL: &'static [Self] = &[
        Self::Hexadecimal,
        Self::Decimal,
        Self::Signed,
        Self::Octal,
        Self::Binary,
        Self::Ascii,
    ];

    /// The bases the keys can type in: every reading that is a base, which
    /// leaves out the signed one — it is decimal read differently, not a base
    /// of its own — and the string, which is not a number at all.
    pub const BASES: &'static [Self] =
        &[Self::Hexadecimal, Self::Decimal, Self::Octal, Self::Binary];

    const fn label(self) -> Text {
        match self {
            Self::Hexadecimal => Text::Hexadecimal,
            Self::Decimal => Text::Decimal,
            Self::Signed => Text::Signed,
            Self::Octal => Text::Octal,
            Self::Binary => Text::Binary,
            Self::Ascii => Text::AsciiText,
        }
    }

    /// The three letters a calculator names this base with, which read the
    /// same in all three languages.
    const fn short(self) -> &'static str {
        match self {
            Self::Hexadecimal => "HEX",
            Self::Decimal | Self::Signed => "DEC",
            Self::Octal => "OCT",
            Self::Binary => "BIN",
            Self::Ascii => "TXT",
        }
    }

    /// The base this field is written in, and the prefix it accepts.
    const fn base(self) -> Option<(u32, &'static str)> {
        match self {
            Self::Hexadecimal => Some((16, "0x")),
            Self::Decimal | Self::Signed => Some((10, "")),
            Self::Octal => Some((8, "0o")),
            Self::Binary => Some((2, "0b")),
            Self::Ascii => None,
        }
    }

    /// The value, written the way this field writes it.
    #[must_use]
    pub fn write(self, value: u64, width: Width) -> String {
        let value = value & width.mask();
        match self {
            Self::Hexadecimal => format!("{value:X}"),
            Self::Decimal => value.to_string(),
            Self::Signed => width.signed(value).to_string(),
            Self::Octal => format!("{value:o}"),
            // Grouped in fours, counted from the right: a run of sixty-four
            // digits is a wall, and a nibble is how everyone reads binary.
            Self::Binary => grouped(&format!("{value:b}")),
            Self::Ascii => spelt(value, width),
        }
    }

    /// What is typed here, read as a value.
    ///
    /// An empty field is zero rather than a refusal: clearing a field to type
    /// a new number must not paint the window red on the way.
    ///
    /// # Errors
    ///
    /// [`Refusal::Unreadable`] when it is not a number in this base, and
    /// [`Refusal::TooWide`] when it is one that the width cannot hold.
    pub fn read(self, typed: &str, width: Width) -> Result<u64, Refusal> {
        let Some((radix, prefix)) = self.base() else {
            return spell(typed, width);
        };
        // Whitespace and underscores are how a reader keeps a long number
        // readable — and what this very window writes into the binary field.
        let cleaned: String = typed
            .chars()
            .filter(|character| !character.is_whitespace() && *character != '_')
            .collect();
        let digits = cleaned
            .strip_prefix(prefix)
            .or_else(|| cleaned.strip_prefix(&prefix.to_uppercase()))
            .unwrap_or(&cleaned);
        if digits.is_empty() {
            return Ok(0);
        }
        if self == Self::Signed {
            return signed(digits, width);
        }
        if !digits.chars().all(|character| character.is_digit(radix)) {
            return Err(Refusal::Unreadable);
        }
        // Every character is a digit of this base, so the only way left to
        // fail is a number too long for the type — which is too wide, not
        // unreadable.
        let value = u64::from_str_radix(digits, radix).map_err(|_| Refusal::TooWide)?;
        if value & !width.mask() == 0 {
            Ok(value)
        } else {
            Err(Refusal::TooWide)
        }
    }
}

/// A signed number, held to what the width can hold.
fn signed(digits: &str, width: Width) -> Result<u64, Refusal> {
    let unsigned = digits.strip_prefix(['-', '+']).unwrap_or(digits);
    if unsigned.is_empty() || !unsigned.chars().all(|character| character.is_ascii_digit()) {
        return Err(Refusal::Unreadable);
    }
    let value: i128 = digits.parse().map_err(|_| Refusal::TooWide)?;
    let limit = 1_i128 << (width.bits() - 1);
    if value < -limit || value >= limit {
        return Err(Refusal::TooWide);
    }
    #[expect(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "held inside the width just above, which is what makes the bits fit"
    )]
    Ok((value as i64 as u64) & width.mask())
}

/// The bytes of `value` as characters, the most significant first.
///
/// Leading zero bytes are left out: `0x4142` spells `AB`, not six blanks and
/// `AB`. A byte that cannot be printed is written `·`, which says *a byte was
/// here and it was not a letter* — and cannot be typed back, so the field
/// refuses it rather than reading it as a full stop.
fn spelt(value: u64, width: Width) -> String {
    let mut spelt = String::new();
    for index in (0..width.bits() / 8).rev() {
        let byte = ((value >> (index * 8)) & 0xff) as u8;
        if spelt.is_empty() && byte == 0 {
            continue;
        }
        spelt.push(if byte.is_ascii_graphic() || byte == b' ' {
            char::from(byte)
        } else {
            '·'
        });
    }
    spelt
}

/// The value a string of characters spells, the first character weighing most.
fn spell(typed: &str, width: Width) -> Result<u64, Refusal> {
    if typed.is_empty() {
        return Ok(0);
    }
    if !typed.is_ascii() {
        return Err(Refusal::Unreadable);
    }
    let bytes = typed.as_bytes();
    if bytes.len() > width.bits() as usize / 8 {
        return Err(Refusal::TooWide);
    }
    Ok(bytes
        .iter()
        .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte)))
}

/// A run of digits cut into groups of four, counted from the right.
fn grouped(digits: &str) -> String {
    const GROUP: usize = 4;
    let mut grouped = String::with_capacity(digits.len() + digits.len() / GROUP);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % GROUP == 0 {
            grouped.push(' ');
        }
        grouped.push(digit);
    }
    grouped
}

/// One two-sided operation, waiting for its right-hand side.
///
/// Entered the way a calculator is used: what is on screen becomes the left
/// side, the operation waits, and the next number entered is the right one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Add,
    Subtract,
    Multiply,
    /// Integer division, unsigned. A division by zero is refused rather than
    /// answered: it has no answer.
    Divide,
    Remainder,
    ShiftLeft,
    ShiftRight,
    /// The arithmetic one: the sign comes down with the value.
    ShiftRightSigned,
    RotateLeft,
    RotateRight,
    And,
    Or,
    Xor,
}

impl Operation {
    /// What the button carries, and what is shown while the operation waits.
    ///
    /// The arithmetic in signs, because that is what a calculator has on it;
    /// everything else in the mnemonic of the instruction it stands for,
    /// because the reader is looking at a listing full of them.
    #[must_use]
    pub const fn sign(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "−",
            Self::Multiply => "×",
            Self::Divide => "÷",
            Self::Remainder => "mod",
            Self::ShiftLeft => "shl",
            Self::ShiftRight => "shr",
            Self::ShiftRightSigned => "sar",
            Self::RotateLeft => "rol",
            Self::RotateRight => "ror",
            Self::And => "and",
            Self::Or => "or",
            Self::Xor => "xor",
        }
    }

    /// What it does, for the reader who is not sure.
    const fn hint(self) -> Text {
        match self {
            Self::Add => Text::Add,
            Self::Subtract => Text::Subtract,
            Self::Multiply => Text::Multiply,
            Self::Divide => Text::Divide,
            Self::Remainder => Text::Remainder,
            Self::ShiftLeft => Text::ShiftLeft,
            Self::ShiftRight => Text::ShiftRight,
            Self::ShiftRightSigned => Text::ShiftRightSigned,
            Self::RotateLeft => Text::RotateLeft,
            Self::RotateRight => Text::RotateRight,
            Self::And => Text::BitwiseAnd,
            Self::Or => Text::BitwiseOr,
            Self::Xor => Text::BitwiseXor,
        }
    }

    /// Works the operation out inside `width`.
    ///
    /// Everything happens in the chosen width: what overflows is lost, a shift
    /// throws bits off the end of *this* value, and a rotation brings them
    /// round to its own bottom.
    ///
    /// # Errors
    ///
    /// [`Refusal::ByZero`] for a division by zero, which has no answer and is
    /// therefore not given one.
    pub fn apply(self, left: u64, right: u64, width: Width) -> Result<u64, Refusal> {
        let bits = width.bits();
        let mask = width.mask();
        let left = left & mask;
        let right = right & mask;
        // A distance is a count of bits, and a huge one is simply further than
        // the width — never a shift Rust would call an overflow.
        let distance = u32::try_from(right).unwrap_or(u32::MAX);
        let far = distance >= bits;
        // A rotation by the width is the value itself, so only the remainder
        // counts; the second shift is taken modulo the width as well, because
        // a rotation of nothing would otherwise ask for a shift of the whole
        // word.
        let turn = distance % bits;
        let back = (bits - turn) % bits;
        Ok(match self {
            Self::Add => left.wrapping_add(right) & mask,
            Self::Subtract => left.wrapping_sub(right) & mask,
            Self::Multiply => left.wrapping_mul(right) & mask,
            Self::Divide => left.checked_div(right).ok_or(Refusal::ByZero)?,
            Self::Remainder => left.checked_rem(right).ok_or(Refusal::ByZero)?,
            // Everything left the value: shifting a word by its own width is
            // an overflow in Rust rather than the zero a processor gives.
            Self::ShiftLeft | Self::ShiftRight if far => 0,
            Self::ShiftLeft => (left << distance) & mask,
            Self::ShiftRight => left >> distance,
            Self::ShiftRightSigned => {
                // The sign fills the whole word once it has been shifted out
                // of it, which is what stopping one short of the width does.
                let by = distance.min(bits - 1);
                #[expect(
                    clippy::cast_sign_loss,
                    reason = "the bits are what is kept; the sign was only how they were read"
                )]
                let shifted = (width.signed(left) >> by) as u64;
                shifted & mask
            }
            Self::RotateLeft => ((left << turn) | (left >> back)) & mask,
            Self::RotateRight => ((left >> turn) | (left << back)) & mask,
            Self::And => left & right,
            Self::Or => left | right,
            Self::Xor => left ^ right,
        })
    }
}

/// An operation with only one side, which therefore happens at once.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unary {
    Not,
    Negate,
    /// The bytes the other way round, which is what a little-endian dump of
    /// the value reads as.
    SwapBytes,
}

impl Unary {
    #[must_use]
    pub const fn apply(self, value: u64, width: Width) -> u64 {
        let mask = width.mask();
        let value = value & mask;
        match self {
            Self::Not => !value & mask,
            Self::Negate => value.wrapping_neg() & mask,
            // Swapped inside the width: the bytes of a sixteen-bit value are
            // its two, not two of eight.
            Self::SwapBytes => value.swap_bytes() >> (64 - width.bits()),
        }
    }
}

/// What the window holds, what is half-typed in it, and what it is waiting to
/// work out.
pub struct State {
    /// The one value every field on screen is a reading of.
    value: u64,
    /// How many bits it is held in. The rest are not shown and not kept:
    /// narrowing the width throws the bits above it away, in front of the
    /// reader, rather than keeping them somewhere they cannot be seen.
    pub width: Width,
    /// The base the keys type in. The fields take any of them; the pad has to
    /// be told which, because `11` is three in one base and seventeen in
    /// another.
    entry: Field,
    /// The left-hand side, and the operation waiting for its right.
    pending: Option<(u64, Operation)>,
    /// Whether the next digit starts a number rather than lengthening the one
    /// on screen. True after an operation and after `=`, which is what makes
    /// `12 + 34` the sum of two numbers instead of `1234`.
    fresh: bool,
    /// The field the reader is typing in, and what they have typed in it.
    ///
    /// Every other field is written from the value on every frame. Without
    /// this, `0x` on its way to `0x1f` would be read as a zero and written
    /// straight back over the cursor as `0`, and no number could ever be typed
    /// in more than one keystroke.
    editing: Option<Field>,
    typed: String,
    /// Why the last thing asked for was not done, when it was not.
    refused: Option<Refusal>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            value: 0,
            width: Width::default(),
            // Decimal, because the first thing anyone does with a calculator
            // is add two ordinary numbers. Every other base is one press away,
            // and all six are on screen whichever is chosen.
            entry: Field::Decimal,
            pending: None,
            fresh: true,
            editing: None,
            typed: String::new(),
            refused: None,
        }
    }
}

impl State {
    #[must_use]
    pub const fn value(&self) -> u64 {
        self.value
    }

    /// Puts a value in the window, keeping only the bits the width holds.
    pub const fn set(&mut self, value: u64) {
        self.value = value & self.width.mask();
        self.fresh = true;
        self.editing = None;
        self.refused = None;
    }

    /// Reads the same bits in a different width, dropping what no longer fits.
    pub const fn set_width(&mut self, width: Width) {
        self.width = width;
        self.value &= width.mask();
        self.editing = None;
        self.refused = None;
    }

    /// The operation waiting for its right-hand side, written the way the keys
    /// are typing: `12 +`, so the reader can see what `=` is about to do.
    #[must_use]
    pub fn waiting(&self) -> Option<String> {
        self.pending.map(|(left, operation)| {
            format!(
                "{} {}",
                self.entry.write(left, self.width),
                operation.sign()
            )
        })
    }

    /// The reader chose a two-sided operation.
    ///
    /// What is on screen becomes its left-hand side, and the next number
    /// entered is the right one. A second operation pressed before `=` works
    /// the first one out first, which is what `2 + 3 ×` does on a calculator.
    pub fn press(&mut self, operation: Operation) {
        if self.pending.is_some() {
            self.equals();
            // What could not be worked out is not built upon: the reader is
            // told why, and the operation that failed is still there.
            if self.refused.is_some() {
                return;
            }
        }
        self.pending = Some((self.value, operation));
        self.fresh = true;
        self.editing = None;
    }

    /// Works out what is waiting, if anything is.
    ///
    /// An operation that could not be worked out — a division by zero — keeps
    /// its place: the reader puts another number in and presses again, rather
    /// than starting the sum over.
    pub fn equals(&mut self) {
        let Some((left, operation)) = self.pending else {
            return;
        };
        match operation.apply(left, self.value, self.width) {
            Ok(value) => {
                self.value = value;
                self.pending = None;
                self.refused = None;
            }
            Err(refusal) => self.refused = Some(refusal),
        }
        self.fresh = true;
        self.editing = None;
    }

    /// Whether there is something for `=` to do.
    #[must_use]
    pub const fn can_work_out(&self) -> bool {
        self.pending.is_some()
    }

    /// One of the one-sided operations, which happen at once.
    pub const fn apply(&mut self, operation: Unary) {
        self.value = operation.apply(self.value, self.width);
        self.fresh = true;
        self.editing = None;
        self.refused = None;
    }

    /// A digit pressed on the keys, in the base they are typing in.
    ///
    /// A digit the base does not have is not a number, and does nothing: the
    /// key that would enter it cannot be pressed either.
    pub fn digit(&mut self, digit: u32) {
        let Some((radix, _)) = self.entry.base() else {
            return;
        };
        if digit >= radix {
            return;
        }
        let current = if self.fresh { 0 } else { self.value };
        let next = current
            .checked_mul(u64::from(radix))
            .and_then(|shifted| shifted.checked_add(u64::from(digit)));
        // A digit that would take the number past the width is refused and
        // said to be too wide, rather than dropped in silence or wrapped.
        match next {
            Some(value) if value & !self.width.mask() == 0 => {
                self.value = value;
                self.fresh = false;
                self.editing = None;
                self.refused = None;
            }
            _ => self.refused = Some(Refusal::TooWide),
        }
    }

    /// Takes the last digit back off, in the base the keys are typing in.
    pub fn backspace(&mut self) {
        let Some((radix, _)) = self.entry.base() else {
            return;
        };
        self.value /= u64::from(radix);
        self.fresh = false;
        self.editing = None;
        self.refused = None;
    }

    /// Back to nothing: the value, and whatever was waiting on it.
    pub const fn clear(&mut self) {
        self.value = 0;
        self.pending = None;
        self.fresh = true;
        self.editing = None;
        self.refused = None;
    }

    /// The value with its bytes the other way round.
    fn swapped(&self) -> u64 {
        Unary::SwapBytes.apply(self.value, self.width)
    }
}

pub fn show(app: &mut DesdecApp, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::Calculator) {
        return;
    }
    let mut open = true;
    let mut window = egui::Window::new(app.t(Text::Calculator))
        .id(egui::Id::new("desdec.calculator"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(ASSUMED_SIZE);
    window = crate::ui::centred(
        window,
        ctx,
        app.dialogs.opening_step(Dialog::Calculator).is_some(),
    );

    window.show(ctx, |ui| contents(app, ui));
    app.dialogs.set(Dialog::Calculator, open);
}

fn contents(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let language = app.preferences.language;
    let accent = accent(app.preferences.theme);
    let state = &mut app.calculator;

    // Scrolled, because the window holds a fixed amount of interface — six
    // fields, four rows of bits, a pad of keys — and a laptop screen is not
    // asked how it feels about that.
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.label(egui::RichText::new(text(language, Text::CalculatorHelp)).color(MUTED));
        ui.add_space(8.0);
        width_row(state, ui, language);
        ui.add_space(10.0);
        fields(state, ui, language);

        ui.add_space(6.0);
        match state.refused {
            Some(refusal) => {
                ui.colored_label(ERROR, text(language, refusal.text()));
            }
            // The line is drawn either way, so nothing under it moves the
            // moment a number stops being readable.
            None => {
                ui.label(" ");
            }
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);
        ui.strong(text(language, Text::Bits));
        ui.add_space(6.0);
        bits(state, ui, accent);

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(8.0);
        entry_row(state, ui, language);
        ui.add_space(6.0);
        // The keys on the left, everything they have no room for on the
        // right: side by side the window is a calculator, and stacked it was
        // three hundred pixels taller than a laptop screen is tall.
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| keypad(state, ui, language));
            ui.add_space(18.0);
            ui.vertical(|ui| {
                operations(state, ui, language);
                ui.add_space(10.0);
                readings(state, ui, language);
            });
        });
    });
}

/// How wide the value is read, and what that costs: narrowing it drops the
/// bits above the new width, which is visible in every field at once.
fn width_row(state: &mut State, ui: &mut egui::Ui, language: Language) {
    ui.horizontal(|ui| {
        ui.strong(text(language, Text::BitWidth));
        for width in Width::ALL {
            if ui
                .selectable_label(state.width == *width, width.label())
                .clicked()
            {
                state.set_width(*width);
            }
        }
    });
}

/// The six readings of the value, each of which can be typed in.
fn fields(state: &mut State, ui: &mut egui::Ui, language: Language) {
    let mut changed: Option<(Field, String)> = None;
    let mut finished = false;
    egui::Grid::new("calculator_fields")
        .num_columns(2)
        .spacing([14.0, 6.0])
        .show(ui, |ui| {
            for field in Field::ALL {
                ui.strong(text(language, field.label()));
                // What the reader typed while they are in the field, and the
                // value everywhere else.
                let mut shown = if state.editing == Some(*field) {
                    state.typed.clone()
                } else {
                    field.write(state.value, state.width)
                };
                let response = ui.add(
                    egui::TextEdit::singleline(&mut shown)
                        .id(egui::Id::new(("desdec.calculator", *field)))
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY),
                );
                if response.changed() {
                    changed = Some((*field, shown));
                }
                if response.lost_focus() && state.editing == Some(*field) {
                    finished = true;
                }
                ui.end_row();
            }
        });

    if let Some((field, typed)) = changed {
        state.editing = Some(field);
        match field.read(&typed, state.width) {
            Ok(value) => {
                state.value = value;
                // A number typed in a field is a number entered, exactly like
                // one tapped on the keys: the next digit lengthens it.
                state.fresh = false;
                state.refused = None;
            }
            Err(refusal) => state.refused = Some(refusal),
        }
        state.typed = typed;
    } else if finished {
        // Left alone, the field goes back to showing the value — including
        // the field that was refused, which is how a reader sees that what
        // they typed was not taken.
        state.editing = None;
        state.refused = None;
    }
}

/// One cell of the bit grid.
const CELL: egui::Vec2 = egui::vec2(15.0, 20.0);
/// The gutter naming each row, wide enough for the two digits of `48`.
///
/// A fixed width rather than a number padded to two characters: egui lays out
/// a label without its leading space, so the row starting at bit 0 was drawn
/// one character to the left of the three above it — the whole grid leaned.
const GUTTER: f32 = 24.0;

/// The value as its bits, sixteen to a row at most, the lowest on the right.
///
/// Pressing one turns it over. It is the reading a mask is actually read in,
/// and the one no amount of hexadecimal makes obvious.
fn bits(state: &mut State, ui: &mut egui::Ui, accent: egui::Color32) {
    /// Bits to a row at most: four groups of four, which fits any window this
    /// opens in and keeps a sixty-four bit value to four rows. A value
    /// narrower than that is one row of its own width — sixteen cells for
    /// eight bits would be eight cells of a value and eight of nothing.
    const PER_ROW: u32 = 16;
    /// Space between one group of four and the next.
    const GAP: f32 = 7.0;

    let mut toggled = None;
    let per_row = PER_ROW.min(state.width.bits());
    // From the highest row down, so the number reads left to right the way it
    // is written everywhere above.
    for row in (0..state.width.bits() / per_row).rev() {
        let lowest = row * per_row;
        ui.horizontal(|ui| {
            // Where the row starts, in a gutter down the left: a bit can then
            // be named without counting cells across four rows, and the
            // lowest bit of all stays the last cell on the right.
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(GUTTER, CELL.y), egui::Sense::hover());
            ui.painter().text(
                rect.right_center(),
                egui::Align2::RIGHT_CENTER,
                lowest.to_string(),
                egui::TextStyle::Small.resolve(ui.style()),
                MUTED,
            );
            ui.add_space(GAP);
            for index in (lowest..lowest + per_row).rev() {
                if index % 4 == 3 && index % per_row != per_row - 1 {
                    ui.add_space(GAP);
                }
                let set = (state.value >> index) & 1 == 1;
                if bit(ui, set, accent).clicked() {
                    toggled = Some(index);
                }
            }
        });
    }

    if let Some(index) = toggled {
        state.set(state.value() ^ (1 << index));
    }
}

/// One bit: the digit, lit when it is set, over a cell that answers a press.
///
/// Drawn rather than made of a button so that sixty-four of them are a field
/// of digits and not sixty-four frames, which is what the eye has to read
/// across in one go.
fn bit(ui: &mut egui::Ui, set: bool, accent: egui::Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(CELL, egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            rect.shrink(1.0),
            3.0,
            ui.visuals().widgets.hovered.weak_bg_fill,
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        if set { "1" } else { "0" },
        // The interface's own monospace rather than a size written here: the
        // digits of a bit grid are text, and text follows the reader's theme.
        egui::TextStyle::Monospace.resolve(ui.style()),
        if set { accent } else { MUTED },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The base the keys type in, and what the window is waiting to work out.
fn entry_row(state: &mut State, ui: &mut egui::Ui, language: Language) {
    ui.horizontal(|ui| {
        ui.strong(text(language, Text::EntryBase));
        for base in Field::BASES {
            if ui
                .selectable_label(state.entry == *base, base.short())
                .on_hover_text(text(language, base.label()))
                .clicked()
            {
                state.entry = *base;
            }
        }
        // What `=` is about to do, in the base the keys are typing in. Without
        // it a pressed operation is invisible, and the reader is left typing a
        // second number into what looks like a window that did nothing.
        if let Some(waiting) = state.waiting() {
            ui.add_space(12.0);
            ui.label(egui::RichText::new(waiting).monospace().color(MUTED));
        }
    });
}

/// One key of the pad.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Key {
    Digit(u32),
    Operation(Operation),
    /// Takes the last digit back off.
    Backspace,
    Equals,
}

/// The keys, in the order they are laid out.
///
/// The hexadecimal digits on top, then the three rows of the pad every
/// telephone and every calculator has had for fifty years, with the four
/// operations down the right-hand side where a calculator keeps them.
const KEYS: &[&[Key]] = &[
    &[
        Key::Digit(0xA),
        Key::Digit(0xB),
        Key::Digit(0xC),
        Key::Backspace,
    ],
    &[
        Key::Digit(0xD),
        Key::Digit(0xE),
        Key::Digit(0xF),
        Key::Operation(Operation::Divide),
    ],
    &[
        Key::Digit(7),
        Key::Digit(8),
        Key::Digit(9),
        Key::Operation(Operation::Multiply),
    ],
    &[
        Key::Digit(4),
        Key::Digit(5),
        Key::Digit(6),
        Key::Operation(Operation::Subtract),
    ],
    &[
        Key::Digit(1),
        Key::Digit(2),
        Key::Digit(3),
        Key::Operation(Operation::Add),
    ],
    &[
        Key::Digit(0),
        Key::Operation(Operation::Remainder),
        Key::Equals,
    ],
];

/// The keys themselves.
///
/// Laid out by hand rather than in a grid: `=` is two keys wide, which a grid
/// cannot do without making the column above it two keys wide as well.
fn keypad(state: &mut State, ui: &mut egui::Ui, language: Language) {
    const KEY: egui::Vec2 = egui::vec2(44.0, 28.0);
    const SPACING: f32 = 4.0;

    let radix = state.entry.base().map_or(10, |(radix, _)| radix);
    for row in KEYS {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACING;
            for key in *row {
                // `=` takes the width of the two keys it stands in for.
                let size = if *key == Key::Equals {
                    egui::vec2(KEY.x * 2.0 + SPACING, KEY.y)
                } else {
                    KEY
                };
                let (label, enabled, hint, held) = match key {
                    Key::Digit(digit) => (
                        digit_label(*digit),
                        *digit < radix,
                        String::new(),
                        text(language, Text::NotADigitInThisBase).to_owned(),
                    ),
                    Key::Operation(operation) => (
                        operation.sign().to_owned(),
                        true,
                        text(language, operation.hint()).to_owned(),
                        String::new(),
                    ),
                    // The arrow rather than the usual `⌫`: none of the fonts
                    // the application ships carries that code point, and it
                    // would reach the window as an empty box.
                    Key::Backspace => (
                        String::from("←"),
                        true,
                        text(language, Text::Backspace).to_owned(),
                        String::new(),
                    ),
                    Key::Equals => (
                        String::from("="),
                        state.can_work_out(),
                        format!(
                            "{}\n{}",
                            text(language, Text::WorkItOut),
                            text(language, Text::ArithmeticWraps)
                        ),
                        text(language, Text::ChooseAnOperationFirst).to_owned(),
                    ),
                };
                // Only when there is something to say: egui draws an empty
                // tooltip for an empty string, which is a grey box that
                // follows the pointer over every digit of the pad.
                let mut response = ui.add_enabled(enabled, egui::Button::new(label).min_size(size));
                if !hint.is_empty() {
                    response = response.on_hover_text(hint);
                }
                if !held.is_empty() {
                    response = response.on_disabled_hover_text(held);
                }
                if response.clicked() {
                    match key {
                        Key::Digit(digit) => state.digit(*digit),
                        Key::Operation(operation) => state.press(*operation),
                        Key::Backspace => state.backspace(),
                        Key::Equals => state.equals(),
                    }
                }
            }
        });
    }
}

/// A digit as the one character it is written with, `A` to `F` included.
fn digit_label(digit: u32) -> String {
    char::from_digit(digit, 16)
        .unwrap_or('0')
        .to_ascii_uppercase()
        .to_string()
}

/// The operations the keys have no room for: the shifts and rotations, the
/// bitwise ones, and the two that need no second number.
///
/// Each row under its own name rather than beside it: this is the narrow
/// column at the right of the keys, and a label beside five buttons would be
/// half of it.
fn operations(state: &mut State, ui: &mut egui::Ui, language: Language) {
    let mut pressed = None;
    let mut applied = None;

    ui.strong(text(language, Text::Shifts));
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        for operation in [
            Operation::ShiftLeft,
            Operation::ShiftRight,
            Operation::ShiftRightSigned,
            Operation::RotateLeft,
            Operation::RotateRight,
        ] {
            if button(ui, operation, language) {
                pressed = Some(operation);
            }
        }
    });

    ui.add_space(8.0);
    ui.strong(text(language, Text::BitwiseOperations));
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        for operation in [Operation::And, Operation::Or, Operation::Xor] {
            if button(ui, operation, language) {
                pressed = Some(operation);
            }
        }
        ui.separator();
        // Neither of these waits for a second number: there is nothing to put
        // on the other side of them.
        for (mnemonic, operation, hint) in [
            ("not", Unary::Not, Text::BitwiseNot),
            ("neg", Unary::Negate, Text::Negate),
        ] {
            if ui
                .button(mnemonic)
                .on_hover_text(text(language, hint))
                .clicked()
            {
                applied = Some(operation);
            }
        }
    });

    if let Some(operation) = pressed {
        state.press(operation);
    }
    if let Some(operation) = applied {
        state.apply(operation);
    }
}

/// One button that starts a two-sided operation, with what it does on hover.
fn button(ui: &mut egui::Ui, operation: Operation, language: Language) -> bool {
    ui.button(operation.sign())
        .on_hover_text(text(language, operation.hint()))
        .clicked()
}

/// What is read off the value rather than typed into it: the same bytes the
/// other way round, and how many bits are set.
fn readings(state: &mut State, ui: &mut egui::Ui, language: Language) {
    let mut swap = false;
    let mut clear = false;
    ui.horizontal(|ui| {
        ui.strong(text(language, Text::SwappedBytes));
        ui.monospace(format!("0x{:X}", state.swapped()));
        if ui
            .button("bswap")
            .on_hover_text(text(language, Text::SwapBytes))
            .clicked()
        {
            swap = true;
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.strong(text(language, Text::SetBits));
        ui.monospace(state.value().count_ones().to_string());
        ui.separator();
        if ui.button(text(language, Text::ClearValue)).clicked() {
            clear = true;
        }
    });
    if swap {
        state.apply(Unary::SwapBytes);
    }
    if clear {
        state.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::WorkspaceView;

    /// A window tall enough for the whole of the calculator.
    ///
    /// The contents scroll, and what is scrolled out of view is not painted at
    /// all: a test that read the ordinary test window would be reading what
    /// fits in eight hundred pixels rather than what the window says.
    fn tall_input() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 1400.0),
            )),
            ..Default::default()
        }
    }

    /// Every field writes what the next one reads back.
    ///
    /// The six fields are one number written six ways, and this is what that
    /// sentence means: whatever the window writes in a field, reading that
    /// field again gives the value it was written from.
    #[test]
    fn every_field_reads_back_what_it_wrote() {
        for width in Width::ALL {
            for value in [0, 1, 0x41, 0x7f, 0xff, 0x1234, 0xdead_beef, u64::MAX] {
                let value = value & width.mask();
                for field in Field::ALL {
                    // Except the string, which is a reading and not a writing:
                    // a byte that cannot be printed is marked rather than
                    // spelt, and the mark is deliberately not typeable. What
                    // it does promise is checked on its own, below.
                    if *field == Field::Ascii {
                        continue;
                    }
                    let written = field.write(value, *width);
                    assert_eq!(
                        field.read(&written, *width),
                        Ok(value),
                        "{field:?} at {} bits could not read back {written:?}",
                        width.bits()
                    );
                }
            }
        }
    }

    /// The signed reading is the width's, not the machine's: the same byte is
    /// −1 in eight bits and 255 in sixteen.
    #[test]
    fn the_signed_reading_follows_the_width() {
        assert_eq!(Width::Eight.signed(0xff), -1);
        assert_eq!(Width::Sixteen.signed(0xff), 255);
        assert_eq!(Width::ThirtyTwo.signed(0xffff_ffff), -1);
        assert_eq!(Width::SixtyFour.signed(u64::MAX), -1);
    }

    /// Narrowing the width throws away what no longer fits, in front of the
    /// reader, rather than keeping bits nothing on screen shows.
    #[test]
    fn narrowing_the_width_keeps_only_the_bits_that_fit() {
        let mut state = State::default();
        state.set(0xdead_beef);
        state.set_width(Width::Sixteen);
        assert_eq!(state.value(), 0xbeef);
        assert_eq!(Field::Hexadecimal.write(state.value(), state.width), "BEEF");
    }

    /// A number too large for the chosen width is refused and said to be too
    /// large. Quietly keeping its low bits would answer a question nobody
    /// asked, with a number the reader never typed.
    #[test]
    fn a_value_too_wide_for_the_width_is_refused_rather_than_cut_down() {
        assert_eq!(
            Field::Hexadecimal.read("1ff", Width::Eight),
            Err(Refusal::TooWide)
        );
        assert_eq!(
            Field::Signed.read("200", Width::Eight),
            Err(Refusal::TooWide)
        );
        assert_eq!(Field::Ascii.read("AB", Width::Eight), Err(Refusal::TooWide));
        assert_eq!(
            Field::Hexadecimal.read("zz", Width::SixtyFour),
            Err(Refusal::Unreadable)
        );
    }

    /// What a reader types is what a reader writes: spaces, underscores and
    /// the usual prefixes are all read, and an empty field is a zero rather
    /// than a complaint.
    #[test]
    fn a_field_reads_what_a_reader_would_write_in_it() {
        let width = Width::SixtyFour;
        assert_eq!(Field::Hexadecimal.read("0x1f", width), Ok(0x1f));
        assert_eq!(Field::Hexadecimal.read("de ad", width), Ok(0xdead));
        assert_eq!(Field::Binary.read("1010_1010", width), Ok(0xaa));
        assert_eq!(Field::Binary.read("0b1111", width), Ok(0xf));
        assert_eq!(Field::Octal.read("0o777", width), Ok(0o777));
        assert_eq!(Field::Signed.read("-1", width), Ok(u64::MAX));
        for field in Field::ALL {
            assert_eq!(field.read("", width), Ok(0), "{field:?}");
        }
    }

    /// The string spells the bytes, the first character weighing most — which
    /// is the order they are written in, not the order a dump holds them in.
    #[test]
    fn the_string_spells_the_bytes_it_holds() {
        let width = Width::SixtyFour;
        assert_eq!(Field::Ascii.read("Hello", width), Ok(0x0048_656c_6c6f));
        assert_eq!(Field::Ascii.write(0x0048_656c_6c6f, width), "Hello");
        // A byte that cannot be printed is marked, and the mark cannot be
        // typed back: the field refuses it rather than reading it as a letter.
        assert_eq!(Field::Ascii.write(0x41_00_42, width), "A·B");
        assert_eq!(
            Field::Ascii.read("A·B", width),
            Err(Refusal::Unreadable),
            "the mark for an unprintable byte is not a character"
        );
    }

    /// `12 + 34 =` is forty-six. The whole point of the window's arithmetic is
    /// that it is entered the way everyone was taught to enter it.
    #[test]
    fn a_sum_is_entered_the_way_a_calculator_is_used() {
        let mut state = State::default();
        assert_eq!(state.entry, Field::Decimal, "the keys start in decimal");

        for digit in [1, 2] {
            state.digit(digit);
        }
        assert_eq!(state.value(), 12, "two presses are one number, not two");

        state.press(Operation::Add);
        assert_eq!(
            state.waiting().as_deref(),
            Some("12 +"),
            "what it is about to do is on screen"
        );

        for digit in [3, 4] {
            state.digit(digit);
        }
        assert_eq!(state.value(), 34, "the second number starts afresh");

        state.equals();
        assert_eq!(state.value(), 46);
        assert_eq!(state.waiting(), None, "and nothing is left waiting");
    }

    /// A second operation before `=` works the first one out, which is what
    /// `2 + 3 ×` does on the thing in a drawer.
    #[test]
    fn a_second_operation_works_the_first_one_out() {
        let mut state = State::default();
        state.digit(2);
        state.press(Operation::Add);
        state.digit(3);
        state.press(Operation::Multiply);
        assert_eq!(state.value(), 5, "the sum was worked out on the way");
        assert_eq!(state.waiting().as_deref(), Some("5 ×"));
        state.digit(4);
        state.equals();
        assert_eq!(state.value(), 20);
    }

    /// The arithmetic stays inside the width and wraps there, which is what
    /// the processor in the listing does with the same two numbers.
    #[test]
    fn the_arithmetic_wraps_inside_the_width() {
        assert_eq!(
            Operation::Add.apply(0xff, 1, Width::Eight),
            Ok(0),
            "one past the top of a byte is zero"
        );
        assert_eq!(
            Operation::Subtract.apply(0, 1, Width::Eight),
            Ok(0xff),
            "and one below zero is the top again"
        );
        assert_eq!(
            Operation::Multiply.apply(0x11, 0x10, Width::Eight),
            Ok(0x10),
            "0x110 does not fit in a byte"
        );
        assert_eq!(Operation::Divide.apply(0x41, 0x10, Width::Eight), Ok(4));
        assert_eq!(Operation::Remainder.apply(0x41, 0x10, Width::Eight), Ok(1));
    }

    /// A division by zero has no answer, so none is given: the window says so
    /// and keeps the operation, for the reader to put another number in.
    #[test]
    fn a_division_by_zero_is_refused_and_the_operation_kept() {
        assert_eq!(
            Operation::Divide.apply(42, 0, Width::SixtyFour),
            Err(Refusal::ByZero)
        );

        let mut state = State::default();
        state.digit(4);
        state.digit(2);
        state.press(Operation::Divide);
        state.digit(0);
        state.equals();
        assert_eq!(state.value(), 0, "nothing was worked out");
        assert_eq!(state.refused, Some(Refusal::ByZero), "and it says why");
        assert!(state.can_work_out(), "the division is still there");

        state.digit(7);
        state.equals();
        assert_eq!(state.value(), 6, "42 ÷ 7, once there was something to do");
        assert_eq!(state.refused, None);
    }

    /// A shift throws bits off the end of the chosen width, not off a
    /// sixty-four bit word that happens to hold it.
    #[test]
    fn shifting_stays_inside_the_width() {
        let width = Width::Eight;
        assert_eq!(
            Operation::ShiftLeft.apply(0x81, 1, width),
            Ok(0x02),
            "the top bit left the value"
        );
        assert_eq!(
            Operation::ShiftRight.apply(0x81, 1, width),
            Ok(0x40),
            "a zero came in at the top"
        );
        assert_eq!(
            Operation::ShiftRightSigned.apply(0x81, 1, width),
            Ok(0xc0),
            "the sign came down with it"
        );
    }

    /// A shift by more than the width leaves nothing behind, and asking for
    /// one must not be an overflow.
    #[test]
    fn a_shift_past_the_width_empties_the_value() {
        for right in [64, 65, u64::MAX] {
            assert_eq!(
                Operation::ShiftLeft.apply(u64::MAX, right, Width::SixtyFour),
                Ok(0)
            );
            assert_eq!(
                Operation::ShiftRight.apply(u64::MAX, right, Width::SixtyFour),
                Ok(0)
            );
        }
    }

    /// A rotation loses nothing: turned all the way round, the value is
    /// itself again.
    #[test]
    fn a_rotation_by_the_whole_width_comes_back_to_the_value() {
        for width in Width::ALL {
            let value = 0x8001_2345_6789_abcd & width.mask();
            let bits = u64::from(width.bits());
            assert_eq!(
                Operation::RotateLeft.apply(value, bits, *width),
                Ok(value),
                "{} bits",
                width.bits()
            );
            let there = Operation::RotateLeft
                .apply(value, 4, *width)
                .expect("a rotation is always an answer");
            assert_eq!(
                Operation::RotateRight.apply(there, 4, *width),
                Ok(value),
                "{} bits, there and back",
                width.bits()
            );
        }
    }

    /// The byte swap is the width's own: the two bytes of a sixteen-bit value,
    /// not two of eight.
    #[test]
    fn the_byte_swap_stays_inside_the_width() {
        assert_eq!(Unary::SwapBytes.apply(0x1234, Width::Sixteen), 0x3412);
        assert_eq!(
            Unary::SwapBytes.apply(0x1122_3344, Width::ThirtyTwo),
            0x4433_2211
        );
    }

    /// The operations do what they are named after, inside the width.
    #[test]
    fn the_operations_are_what_they_are_named_after() {
        let width = Width::Eight;
        assert_eq!(Operation::And.apply(0xa5, 0x0f, width), Ok(0x05));
        assert_eq!(Operation::Or.apply(0xa0, 0x0f, width), Ok(0xaf));
        assert_eq!(Operation::Xor.apply(0xff, 0x0f, width), Ok(0xf0));
        assert_eq!(Unary::Not.apply(0x0f, width), 0xf0);
        assert_eq!(
            Unary::Negate.apply(1, width),
            0xff,
            "one less than nothing, in eight bits"
        );
    }

    /// A digit a base does not have is not a number: pressing it does nothing,
    /// and the key that would press it cannot be pressed at all.
    #[test]
    fn a_digit_the_base_does_not_have_does_nothing() {
        let mut state = State::default();
        state.digit(0xa);
        assert_eq!(state.value(), 0, "there is no A in decimal");

        state.entry = Field::Hexadecimal;
        state.digit(0xa);
        assert_eq!(state.value(), 0xa, "and there is one in hexadecimal");

        state.entry = Field::Binary;
        state.digit(1);
        assert_eq!(state.value(), 0x15, "0xa followed by a one, in binary");
        state.digit(2);
        assert_eq!(state.value(), 0x15, "and no two");
    }

    /// The last digit can be taken back off, in the base it was typed in.
    #[test]
    fn the_last_digit_can_be_taken_back_off() {
        let mut state = State::default();
        for digit in [1, 2, 3] {
            state.digit(digit);
        }
        state.backspace();
        assert_eq!(state.value(), 12);

        state.entry = Field::Hexadecimal;
        state.set(0xabc);
        state.backspace();
        assert_eq!(state.value(), 0xab, "a hexadecimal digit is four bits");
    }

    /// A digit that would take the number past the width is refused and said
    /// to be too wide, rather than wrapped into a number nobody typed.
    #[test]
    fn a_digit_past_the_width_is_refused() {
        let mut state = State::default();
        state.set_width(Width::Eight);
        for digit in [2, 5, 5] {
            state.digit(digit);
        }
        assert_eq!(state.value(), 255);
        state.digit(0);
        assert_eq!(state.value(), 255, "2550 does not fit in a byte");
        assert_eq!(state.refused, Some(Refusal::TooWide));
    }

    /// Clearing takes the value and whatever was waiting on it.
    #[test]
    fn clearing_takes_the_waiting_operation_with_it() {
        let mut state = State::default();
        state.digit(7);
        state.press(Operation::Multiply);
        state.clear();
        assert_eq!(state.value(), 0);
        assert_eq!(state.waiting(), None);
    }

    /// A press on a bit turns that bit over and nothing else.
    #[test]
    fn pressing_a_bit_turns_it_over() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.calculator.set_width(Width::Eight);
        app.calculator.set(0);

        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| contents(&mut app, ui));
        };
        let _ = ctx.run(tall_input(), &mut draw);
        let output = ctx.run(tall_input(), &mut draw);
        // Eight bits, all clear: the one row of the window holding eight
        // zeroes on the same line is the bits, and the lowest of them is the
        // rightmost cell of that row.
        let zeroes: Vec<egui::Pos2> = crate::testing::drawn(&output.shapes)
            .into_iter()
            .filter(|(said, _)| said == "0")
            .map(|(_, at)| at)
            .collect();
        // The cells of one row are painted on exactly the same line; half a
        // pixel is only there so this is not a comparison of two floats.
        let same_row = |one: f32, other: f32| (one - other).abs() < 0.5;
        let row = zeroes
            .iter()
            .find(|candidate| {
                zeroes
                    .iter()
                    .filter(|other| same_row(other.y, candidate.y))
                    .count()
                    >= Width::Eight.bits() as usize
            })
            .copied()
            .expect("a row of eight bits");
        let rightmost = zeroes
            .into_iter()
            .filter(|at| same_row(at.y, row.y))
            .max_by(|left, right| left.x.total_cmp(&right.x))
            .expect("the bits are drawn");

        let at = rightmost + egui::vec2(4.0, 6.0);
        let mut input = tall_input();
        input.events = vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ];
        let _ = ctx.run(input, &mut draw);
        assert_eq!(app.calculator.value(), 1, "the lowest bit was pressed");
    }

    /// Every row of the bit grid starts and ends in the same column.
    ///
    /// The gutter naming a row used to be a number padded to two characters,
    /// and egui lays a label out without its leading space: the row starting
    /// at bit 0 was drawn one character to the left of the three above it, and
    /// the whole grid leaned. Nothing about the text drawn says so — every
    /// digit is on screen either way — so this reads where they landed.
    #[test]
    fn every_row_of_the_bit_grid_starts_in_the_same_column() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.calculator.set_width(Width::SixtyFour);
        app.calculator.set(0);

        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| contents(&mut app, ui));
        };
        let _ = ctx.run(tall_input(), &mut draw);
        let output = ctx.run(tall_input(), &mut draw);

        // The cells are the only sixteen-across rows of single digits on the
        // window, and at sixty-four bits there are four such rows.
        let mut rows: Vec<(f32, Vec<f32>)> = Vec::new();
        for (said, at) in crate::testing::drawn(&output.shapes) {
            if said != "0" {
                continue;
            }
            match rows.iter_mut().find(|(y, _)| (*y - at.y).abs() < 0.5) {
                Some((_, xs)) => xs.push(at.x),
                None => rows.push((at.y, vec![at.x])),
            }
        }
        let mut grid: Vec<Vec<f32>> = rows
            .into_iter()
            .map(|(_, xs)| xs)
            .filter(|xs| xs.len() == 16)
            .collect();
        assert_eq!(grid.len(), 4, "sixty-four bits are drawn as four rows");

        for row in &mut grid {
            row.sort_by(f32::total_cmp);
        }
        let first = grid[0].clone();
        for row in &grid {
            for (left, right) in first.iter().zip(row) {
                assert!(
                    (left - right).abs() < 0.5,
                    "a row of bits is drawn at {right} where the one above it is at {left}"
                );
            }
        }
    }

    /// The signs on the keys are drawn, not squares.
    ///
    /// They are literals rather than translations, so the test that holds
    /// every visible string to the installed fonts never sees them — and `×`,
    /// `÷` and the arrow are exactly the kind of code point a text face stops
    /// short of.
    #[test]
    fn the_signs_on_the_keys_have_glyphs() {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let _ = ctx.run(crate::testing::window_input(), |_| {});
        let font = egui::FontId::proportional(14.0);
        let missing: Vec<char> = ctx.fonts(|fonts| {
            "+−×÷·←="
                .chars()
                .filter(|character| !fonts.has_glyph(&font, *character))
                .collect()
        });
        assert!(missing.is_empty(), "the window cannot draw {missing:?}");
    }

    /// Every reading of the value, the keys and the operations reach the
    /// screen, and none of them is drawn on top of another: six numbers
    /// painted in one place read as a smear, and no assertion about the
    /// strings themselves would notice.
    #[test]
    fn the_window_draws_every_reading_without_stacking_them() {
        let ctx = egui::Context::default();
        let mut app = crate::testing::opened_app(WorkspaceView::Overview);
        app.calculator.set(0x0048_656c_6c6f);

        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| contents(&mut app, ui));
        };
        // Two frames: a panel is measured on the first and painted after.
        let _ = ctx.run(tall_input(), &mut draw);
        let output = ctx.run(tall_input(), &mut draw);

        let said = crate::testing::drawn_text(&output.shapes);
        for reading in [
            "48656C6C6F",
            "310939249775",
            "Hello",
            "shl",
            "bswap",
            "=",
            "HEX",
        ] {
            assert!(said.contains(reading), "the window never showed {reading}");
        }

        let mut seen: Vec<egui::Pos2> = Vec::new();
        for (drawn, at) in crate::testing::drawn(&output.shapes) {
            // An empty field paints an empty galley, and the hint text sits
            // exactly where it would have been: nothing is covered by nothing.
            if drawn.trim().is_empty() {
                continue;
            }
            assert!(
                !seen.contains(&at),
                "{drawn:?} is drawn on top of something else, at {at:?}"
            );
            seen.push(at);
        }
    }
}
