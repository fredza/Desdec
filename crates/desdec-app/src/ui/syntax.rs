//! Syntax colouring for the disassembly and the local pseudo-code.
//!
//! Every listing is built as a `LayoutJob` so a single clickable label can mix
//! colours while keeping the selection background of the row it belongs to.
//! The colouring is purely presentational: the scanners never rewrite the text
//! handed to them, so what the decoder produced is what the user reads.

use eframe::egui::{
    self,
    text::{LayoutJob, TextFormat},
};

/// A colour role, resolved against the active theme rather than hard-coded, so
/// the light theme stays readable instead of inheriting dark-theme pastels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Class {
    /// Instruction mnemonic that keeps the flow inside the block.
    Mnemonic,
    /// Branch, call or return: the reader looks for these first.
    Control,
    /// Machine register, in either the AT&T (`%rax`) or ARM64 (`x0`) spelling.
    Register,
    /// Immediate operand (`$0x10`, `#-8`) or bare numeric literal.
    Number,
    /// C keyword of the pseudo-code.
    Keyword,
    /// Name used as a call target.
    Call,
    /// Comment, including the `/* … */` notes the pseudo-code leaves behind.
    Comment,
    /// Separators and operators.
    Punctuation,
    /// Addresses, raw bytes: present but never competing with the code.
    Dim,
    /// Anything the scanner does not claim, drawn in the ordinary text colour.
    Plain,
}

fn colour(class: Class, visuals: &egui::Visuals) -> egui::Color32 {
    let (dark, light) = match class {
        Class::Mnemonic => ((126, 171, 255), (24, 84, 176)),
        Class::Control => ((226, 148, 205), (154, 44, 128)),
        Class::Register => ((111, 211, 195), (12, 116, 104)),
        Class::Number => ((224, 164, 104), (164, 88, 20)),
        Class::Keyword => ((199, 146, 234), (112, 46, 168)),
        Class::Call => ((230, 208, 122), (132, 98, 18)),
        Class::Comment => ((124, 154, 132), (86, 118, 96)),
        Class::Punctuation => ((150, 160, 182), (110, 120, 140)),
        Class::Dim => ((132, 142, 166), (118, 128, 148)),
        Class::Plain => return visuals.text_color(),
    };
    let (red, green, blue) = if visuals.dark_mode { dark } else { light };
    egui::Color32::from_rgb(red, green, blue)
}

/// A coloured assembly line, ready to hand to [`egui::Label`].
pub fn assembly(ui: &egui::Ui, text: &str, background: egui::Color32) -> LayoutJob {
    job(ui, background, &assembly_spans(text))
}

/// A coloured pseudo-C line. Leading indentation is preserved as written.
pub fn pseudo_code(ui: &egui::Ui, text: &str, background: egui::Color32) -> LayoutJob {
    job(ui, background, &pseudo_code_spans(text))
}

/// An assembly line with what the reader wrote about it after it, the way an
/// assembler carries a comment.
///
/// After the instruction and never in place of it: the name a reader gives an
/// address is theirs, and a listing that let it stand where the decoded text
/// goes would be showing an opinion as a fact.
pub fn annotated(
    ui: &egui::Ui,
    text: &str,
    label: Option<&str>,
    comment: Option<&str>,
    background: egui::Color32,
) -> LayoutJob {
    let mut job = job(ui, background, &assembly_spans(text));
    if let Some(label) = label {
        append(
            &mut job,
            ui,
            background,
            &format!("   {label}:"),
            Class::Call,
        );
    }
    if let Some(comment) = comment {
        append(
            &mut job,
            ui,
            background,
            &format!("   ; {comment}"),
            Class::Comment,
        );
    }
    job
}

/// Supporting monospace text — an address, a run of opcode bytes — drawn in a
/// single dim colour so the code itself keeps the eye.
pub fn dim(ui: &egui::Ui, text: &str, background: egui::Color32) -> LayoutJob {
    job(ui, background, &[(text, Class::Dim)])
}

fn job(ui: &egui::Ui, background: egui::Color32, spans: &[(&str, Class)]) -> LayoutJob {
    let mut job = LayoutJob::default();
    for &(text, class) in spans {
        append(&mut job, ui, background, text, class);
    }
    job
}

fn append(job: &mut LayoutJob, ui: &egui::Ui, background: egui::Color32, text: &str, class: Class) {
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: egui::TextStyle::Monospace.resolve(ui.style()),
            color: colour(class, ui.visuals()),
            background,
            ..TextFormat::default()
        },
    );
}

/// Splits off a leading token: its first character, then every following
/// character `accept` claims. Taking the first character unconditionally keeps
/// the scanners moving forward whatever the input.
fn take(source: &str, accept: impl Fn(char) -> bool) -> (&str, &str) {
    let skip = source.chars().next().map_or(0, char::len_utf8);
    let end = source[skip..]
        .char_indices()
        .find(|&(_, character)| !accept(character))
        .map_or(source.len(), |(index, _)| skip + index);
    source.split_at(end)
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '_' | '.' | '@' | '$')
}

fn is_word_start(character: char) -> bool {
    character.is_alphabetic() || matches!(character, '_' | '.')
}

/// GAS prints prefixes as separate words, so the mnemonic that carries the
/// meaning is the one after them.
fn is_prefix(word: &str) -> bool {
    matches!(
        word,
        "lock"
            | "rep"
            | "repe"
            | "repz"
            | "repne"
            | "repnz"
            | "bnd"
            | "notrack"
            | "data16"
            | "xacquire"
            | "xrelease"
    )
}

/// Whether a mnemonic leaves the current basic block. Covers the x86 `j*`
/// family and the ARM64 branch forms, including their `b.cond` spelling.
fn is_control_flow(word: &str) -> bool {
    word.starts_with("b.")
        || word.starts_with('j')
        || word.starts_with("ret")
        || matches!(
            word,
            "call"
                | "callq"
                | "b"
                | "bl"
                | "blr"
                | "br"
                | "cbz"
                | "cbnz"
                | "tbz"
                | "tbnz"
                | "syscall"
                | "sysret"
                | "int"
                | "int3"
                | "iret"
                | "iretq"
                | "loop"
                | "hlt"
                | "ud2"
                | "svc"
                | "eret"
        )
}

/// ARM64 operands name their registers bare, so they are recognised by shape:
/// a bank letter followed by a number, plus the handful of named registers.
fn is_arm_register(word: &str) -> bool {
    if matches!(word, "sp" | "lr" | "pc" | "fp" | "xzr" | "wzr" | "wsp") {
        return true;
    }
    let Some(bank) = word.chars().next() else {
        return false;
    };
    matches!(bank, 'x' | 'w' | 'v' | 'd' | 's' | 'q' | 'h' | 'b')
        && !word[1..].is_empty()
        && word[1..].chars().all(|c| c.is_ascii_digit())
}

fn assembly_spans(source: &str) -> Vec<(&str, Class)> {
    let mut spans = Vec::new();
    let mut rest = source;
    // Everything up to the first word that is not a prefix is the mnemonic.
    let mut awaiting_mnemonic = true;
    while let Some(first) = rest.chars().next() {
        // Capstone ends a line with a comment; the rest of it is one span.
        if rest.starts_with("//") || first == ';' {
            spans.push((rest, Class::Comment));
            break;
        }
        let ((text, tail), class) = if first.is_whitespace() {
            (take(rest, char::is_whitespace), Class::Plain)
        } else if first == '%' {
            (take(rest, is_word), Class::Register)
        } else if first == '$' || first == '#' {
            (take(rest, |c| is_word(c) || c == '-'), Class::Number)
        } else if first.is_ascii_digit()
            || (first == '-' && rest[1..].starts_with(|c: char| c.is_ascii_digit()))
        {
            (take(rest, char::is_alphanumeric), Class::Number)
        } else if is_word_start(first) {
            let (word, tail) = take(rest, is_word);
            let class = if awaiting_mnemonic {
                awaiting_mnemonic = is_prefix(word);
                if is_control_flow(word) {
                    Class::Control
                } else {
                    Class::Mnemonic
                }
            } else if is_arm_register(word) {
                Class::Register
            } else {
                Class::Plain
            };
            ((word, tail), class)
        } else {
            (take(rest, |_| false), Class::Punctuation)
        };
        rest = tail;
        spans.push((text, class));
    }
    spans
}

fn is_c_keyword(word: &str) -> bool {
    matches!(
        word,
        "if" | "else"
            | "goto"
            | "return"
            | "while"
            | "for"
            | "do"
            | "switch"
            | "case"
            | "break"
            | "continue"
            | "void"
            | "int"
            | "char"
            | "long"
            | "unsigned"
            | "signed"
            | "struct"
            | "sizeof"
    )
}

fn pseudo_code_spans(source: &str) -> Vec<(&str, Class)> {
    let mut spans = Vec::new();
    let mut rest = source;
    while let Some(first) = rest.chars().next() {
        if rest.starts_with("//") {
            spans.push((rest, Class::Comment));
            break;
        }
        let ((text, tail), class) = if let Some(body) = rest.strip_prefix("/*") {
            // An unterminated comment runs to the end of the line, the way a C
            // compiler would read it.
            let end = body.find("*/").map_or(rest.len(), |index| index + 4);
            (rest.split_at(end), Class::Comment)
        } else if first.is_whitespace() {
            (take(rest, char::is_whitespace), Class::Plain)
        } else if first.is_ascii_digit() {
            (take(rest, char::is_alphanumeric), Class::Number)
        } else if is_word_start(first) {
            let (word, tail) = take(rest, is_word);
            let class = if is_c_keyword(word) {
                Class::Keyword
            } else if tail.trim_start().starts_with('(') {
                Class::Call
            } else {
                Class::Plain
            };
            ((word, tail), class)
        } else {
            (take(rest, |_| false), Class::Punctuation)
        };
        rest = tail;
        spans.push((text, class));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scanners colour text without ever altering it; a listing that lost
    /// or gained a character would be lying about the decoded bytes.
    fn assert_lossless(source: &str, spans: &[(&str, Class)]) {
        let rebuilt: String = spans.iter().map(|&(text, _)| text).collect();
        assert_eq!(rebuilt, source);
    }

    fn class_of(spans: &[(&str, Class)], text: &str) -> Option<Class> {
        spans
            .iter()
            .find(|&&(span, _)| span == text)
            .map(|&(_, class)| class)
    }

    #[test]
    fn colours_att_syntax_operands() {
        let source = "mov    -0x8(%rbp),%rax";
        let spans = assembly_spans(source);
        assert_lossless(source, &spans);
        assert_eq!(class_of(&spans, "mov"), Some(Class::Mnemonic));
        assert_eq!(class_of(&spans, "-0x8"), Some(Class::Number));
        assert_eq!(class_of(&spans, "%rbp"), Some(Class::Register));
        assert_eq!(class_of(&spans, "%rax"), Some(Class::Register));
    }

    #[test]
    fn marks_branches_and_calls_as_control_flow() {
        for source in ["jne 0x401050", "callq *%rax", "ret", "b.eq #0x24", "bl x8"] {
            let spans = assembly_spans(source);
            assert_lossless(source, &spans);
            assert_eq!(
                spans.first().map(|&(_, class)| class),
                Some(Class::Control),
                "{source} should open with a control-flow mnemonic"
            );
        }
    }

    #[test]
    fn a_prefix_does_not_hide_the_mnemonic() {
        let spans = assembly_spans("lock cmpxchg %rbx,(%rdi)");
        assert_eq!(class_of(&spans, "lock"), Some(Class::Mnemonic));
        assert_eq!(class_of(&spans, "cmpxchg"), Some(Class::Mnemonic));
        assert_eq!(class_of(&spans, "%rbx"), Some(Class::Register));
    }

    #[test]
    fn colours_arm64_registers_and_immediates() {
        let source = "ldr x1, [sp, #0x10]";
        let spans = assembly_spans(source);
        assert_lossless(source, &spans);
        assert_eq!(class_of(&spans, "ldr"), Some(Class::Mnemonic));
        assert_eq!(class_of(&spans, "x1"), Some(Class::Register));
        assert_eq!(class_of(&spans, "sp"), Some(Class::Register));
        assert_eq!(class_of(&spans, "#0x10"), Some(Class::Number));
    }

    #[test]
    fn colours_pseudo_code_keywords_calls_and_comments() {
        let source = "    if (/* jne condition from flags */) goto label_0x401050;";
        let spans = pseudo_code_spans(source);
        assert_lossless(source, &spans);
        assert_eq!(class_of(&spans, "if"), Some(Class::Keyword));
        assert_eq!(class_of(&spans, "goto"), Some(Class::Keyword));
        assert_eq!(class_of(&spans, "label_0x401050"), Some(Class::Plain));
        assert_eq!(
            class_of(&spans, "/* jne condition from flags */"),
            Some(Class::Comment)
        );
    }

    #[test]
    fn a_name_before_a_parenthesis_reads_as_a_call() {
        let spans = pseudo_code_spans("    stack_push(rbp);");
        assert_eq!(class_of(&spans, "stack_push"), Some(Class::Call));
        assert_eq!(class_of(&spans, "rbp"), Some(Class::Plain));
    }

    #[test]
    fn an_unterminated_comment_runs_to_the_end_of_the_line() {
        let source = "/* unsupported: cpuid";
        let spans = pseudo_code_spans(source);
        assert_lossless(source, &spans);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].1, Class::Comment);
    }

    /// Every pseudo-code line the translator can produce must survive the
    /// scanner untouched.
    #[test]
    fn every_translated_line_is_scanned_losslessly() {
        for asm in [
            "ret",
            "callq  0x401040",
            "jmp    0x401000",
            "mov    %rsp,%rbp",
            "lea    0x2f61(%rip),%rdi",
            "add    $0x10,%rsp",
            "cmp    $0x0,%eax",
            "push   %rbp",
            "pop    %rbp",
            "cpuid",
        ] {
            let translated = crate::ui::decompile::pseudo_c(asm);
            assert_lossless(&translated, &pseudo_code_spans(&translated));
            assert_lossless(asm, &assembly_spans(asm));
        }
    }
}
