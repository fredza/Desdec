//! A model's reading of what Desdec decoded.
//!
//! Every other view in this application shows what the bytes say. This one
//! shows what something else thinks they mean, and the whole design of the
//! view is about keeping those two apart: the answer is always headed as a
//! proposed reading, always names the model that produced it, and sits next to
//! a panel holding the exact text that was sent to get it.
//!
//! Nothing here starts on its own. The buttons are the only thing that speaks
//! to a provider, and with no provider chosen there is nothing to press.

use desdec_core::assistant::{Error, Question};
use eframe::egui;

use crate::{
    app::DesdecApp,
    i18n::Text,
    ui::{ERROR, MUTED, card, section_title},
};

pub fn show(app: &mut DesdecApp, ui: &mut egui::Ui) {
    card(ui, app.t(Text::AiAssistance), |ui| {
        ui.small(app.t(Text::AiAssistanceIntro));
        ui.add_space(8.0);

        let provider = app.preferences.assistant.provider();
        if provider == desdec_core::assistant::Provider::None {
            ui.label(egui::RichText::new(app.t(Text::AssistantNotConfigured)).color(MUTED));
            return;
        }
        ui.small(app.t(if provider.leaves_the_machine() {
            Text::AssistantLeavesMachine
        } else {
            Text::AssistantStaysLocal
        }));
        ui.add_space(8.0);

        questions(app, ui);
        ui.add_space(10.0);
        answer(app, ui);
    });
}

/// The three things that can be asked, each enabled only when there is
/// something to ask it about.
fn questions(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let busy = app.assistance.running;
    let has_binary = app.analysis.is_some();
    let function = app.selected_function;
    let instruction = app.selected_instruction;

    let mut asked = None;
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                has_binary && !busy,
                egui::Button::new(app.t(Text::AskAboutBinary)),
            )
            .clicked()
        {
            asked = Some(Question::Binary);
        }
        let ask_function = ui.add_enabled(
            function.is_some() && !busy,
            egui::Button::new(app.t(Text::AskAboutFunction)),
        );
        if ask_function.clicked()
            && let Some(address) = function
        {
            asked = Some(Question::Function { address });
        }
        if function.is_none() && has_binary {
            ask_function.on_hover_text(app.t(Text::SelectFunctionFirst));
        }
        let ask_instruction = ui.add_enabled(
            instruction.is_some() && !busy,
            egui::Button::new(app.t(Text::AskAboutInstruction)),
        );
        if ask_instruction.clicked()
            && let Some(address) = instruction
        {
            asked = Some(Question::Instruction { address });
        }
        if instruction.is_none() && has_binary {
            ask_instruction.on_hover_text(app.t(Text::SelectInstructionFirst));
        }
    });

    if let Some(question) = asked {
        app.request_assistance(ui.ctx(), question);
    }
}

/// The answer, the wait, or the reason there is neither.
fn answer(app: &mut DesdecApp, ui: &mut egui::Ui) {
    if app.assistance.running {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(app.t(Text::Asking));
        });
    }

    sent_text(app, ui);

    if let Some(error) = app.assistance.error.clone() {
        ui.add_space(8.0);
        report(app, ui, &error);
        return;
    }

    let Some(answer) = app.assistance.answer.clone() else {
        if !app.assistance.running {
            ui.add_space(8.0);
            ui.small(egui::RichText::new(app.t(Text::NothingAskedYet)).color(MUTED));
        }
        return;
    };

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(6.0);
    // The heading comes before the text on purpose: a reader who stops after
    // the first line has still been told what kind of thing they are reading.
    ui.label(section_title(app.t(Text::ProposedReading)));
    ui.horizontal(|ui| {
        // What was asked, beside what answered it: a paragraph whose question
        // has scrolled off the top belongs to nothing in particular.
        if let Some(question) = app.assistance.question {
            ui.small(asked(app, question));
            ui.separator();
        }
        ui.small(app.t(Text::AnsweredBy));
        ui.small(
            egui::RichText::new(format!("{} — {}", answer.provider.label(), answer.model)).strong(),
        );
    });
    ui.add_space(6.0);

    // Said before the text rather than after it: the reader needs to know the
    // end is missing while they are reading, not once they have believed it.
    if answer.truncated {
        ui.small(egui::RichText::new(app.t(Text::AssistantTruncated)).color(ERROR));
        ui.add_space(6.0);
    }

    egui::ScrollArea::vertical()
        .id_salt("assistant_answer")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for line in answer.text.lines() {
                if line.trim().is_empty() {
                    ui.add_space(6.0);
                } else {
                    ui.label(line);
                }
            }
        });
}

/// What was sent, word for word, on demand.
///
/// Folded away by default because it is long, and never further than one
/// click away because a promise about what leaves the machine is worth
/// exactly as much as the reader's ability to check it.
fn sent_text(app: &mut DesdecApp, ui: &mut egui::Ui) {
    let Some(prompt) = app.assistance.prompt.clone() else {
        return;
    };
    ui.add_space(6.0);
    let mut shown = app.assistance.show_prompt;
    ui.checkbox(&mut shown, app.t(Text::ShowWhatIsSent));
    app.assistance.show_prompt = shown;
    if !shown {
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("assistant_prompt")
        .max_height(220.0)
        .show(ui, |ui| {
            ui.monospace(&prompt.system);
            ui.add_space(6.0);
            ui.monospace(&prompt.user);
        });
}

/// The question, named as the button that asked it was.
fn asked(app: &DesdecApp, question: Question) -> String {
    match question {
        Question::Binary => app.t(Text::AskAboutBinary).to_owned(),
        Question::Function { address } => {
            format!("{} {address:#x}", app.t(Text::AskAboutFunction))
        }
        Question::Instruction { address } => {
            format!("{} {address:#x}", app.t(Text::AskAboutInstruction))
        }
    }
}

/// Says what went wrong in the reader's own terms.
///
/// A refusal is reported as an outcome rather than a failure: the model
/// declined, which is a thing that happens to legitimate work on binaries and
/// says nothing about the file.
fn report(app: &DesdecApp, ui: &mut egui::Ui, error: &Error) {
    let (text, detail) = match error {
        Error::NotConfigured => (Text::AssistantNotConfigured, String::new()),
        Error::NoApiKey => (Text::AssistantNoKey, String::new()),
        Error::Unreachable(detail) => (Text::AssistantUnreachable, detail.clone()),
        Error::Rejected { status, message } => {
            (Text::AssistantRejected, format!("{status} {message}"))
        }
        Error::TimedOut => (Text::AssistantTimedOut, String::new()),
        Error::Declined { category } => (
            Text::AssistantDeclined,
            category.clone().unwrap_or_default(),
        ),
        Error::Unreadable(detail) => (Text::AssistantUnreadable, detail.clone()),
    };
    let message = if detail.is_empty() {
        app.t(text).to_owned()
    } else {
        format!("{} {detail}", app.t(text))
    };
    ui.colored_label(ERROR, message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::WorkspaceView;
    use desdec_core::assistant::{Answer, Provider};

    use crate::preferences::AssistantPreference;

    fn frame(app: &mut DesdecApp) -> String {
        let ctx = egui::Context::default();
        let output = ctx.run(crate::testing::window_input(), |ctx| {
            crate::ui::views::show_central_panel(app, ctx);
        });
        crate::testing::drawn_text(&output.shapes)
    }

    /// With nothing configured the view says so and offers nothing to press:
    /// a button that would reach a provider that does not exist is a button
    /// that reports a failure the reader could have been spared.
    #[test]
    fn an_unconfigured_assistant_explains_itself_and_asks_nothing() {
        let mut app = crate::testing::opened_app(WorkspaceView::Assistant);
        let drawn = frame(&mut app);

        assert!(drawn.contains(app.t(Text::AssistantNotConfigured)));
        assert!(!drawn.contains(app.t(Text::AskAboutBinary)));
    }

    /// The reader must be told, in the view itself, whether asking sends the
    /// listing off the machine.
    #[test]
    fn each_provider_says_where_the_listing_goes() {
        let mut app = crate::testing::opened_app(WorkspaceView::Assistant);

        app.preferences.assistant = AssistantPreference::Ollama;
        assert!(frame(&mut app).contains(app.t(Text::AssistantStaysLocal)));

        app.preferences.assistant = AssistantPreference::Claude;
        assert!(frame(&mut app).contains(app.t(Text::AssistantLeavesMachine)));
    }

    /// An answer is never shown as a finding. It carries the heading that says
    /// what it is, and the name of whatever produced it.
    #[test]
    fn an_answer_is_labelled_a_reading_and_names_its_model() {
        let mut app = crate::testing::opened_app(WorkspaceView::Assistant);
        app.preferences.assistant = AssistantPreference::Ollama;
        app.assistance.answer = Some(Answer {
            text: "It reads a password and compares it.".to_owned(),
            provider: Provider::Ollama,
            model: "qwen2.5-coder:7b".to_owned(),
            truncated: false,
        });

        let drawn = frame(&mut app);
        assert!(drawn.contains(app.t(Text::ProposedReading)));
        assert!(drawn.contains("qwen2.5-coder:7b"));
        assert!(drawn.contains("It reads a password and compares it."));
        assert!(!drawn.contains(app.t(Text::AssistantTruncated)));
    }

    /// A reading that stopped at the token limit is shown, because half an
    /// answer is still worth reading — but never as a whole one.
    #[test]
    fn a_truncated_reading_says_that_its_end_is_missing() {
        let mut app = crate::testing::opened_app(WorkspaceView::Assistant);
        app.preferences.assistant = AssistantPreference::Claude;
        app.assistance.answer = Some(Answer {
            text: "It reads a password and then".to_owned(),
            provider: Provider::Claude,
            model: "claude-opus-5".to_owned(),
            truncated: true,
        });

        let drawn = frame(&mut app);
        assert!(drawn.contains(app.t(Text::AssistantTruncated)));
        assert!(drawn.contains("It reads a password and then"));
    }

    /// Whatever was sent must be readable in the view, or the promise about
    /// what leaves the machine is one the reader cannot check.
    #[test]
    fn what_was_sent_can_be_read_in_full() {
        let mut app = crate::testing::opened_app(WorkspaceView::Assistant);
        app.preferences.assistant = AssistantPreference::Claude;
        app.request_assistance(&egui::Context::default(), Question::Binary);
        app.assistance.running = false;
        app.assistance.show_prompt = true;

        let sent = app
            .assistance
            .prompt
            .clone()
            .expect("the request was built before it was sent");
        let drawn = frame(&mut app);
        let first_line = sent.user.lines().next().expect("the request has a body");
        assert!(drawn.contains(first_line), "the sent text was not shown");
    }

    /// A refusal is an outcome, not a crash: it is reported in the reader's
    /// language rather than as a stack of provider jargon.
    #[test]
    fn every_failure_is_reported_in_the_readers_own_words() {
        let mut app = crate::testing::opened_app(WorkspaceView::Assistant);
        app.preferences.assistant = AssistantPreference::Claude;

        for (error, expected) in [
            (
                Error::Declined {
                    category: Some("cyber".to_owned()),
                },
                Text::AssistantDeclined,
            ),
            (Error::NoApiKey, Text::AssistantNoKey),
            (Error::TimedOut, Text::AssistantTimedOut),
            (
                Error::Unreachable("connection refused".to_owned()),
                Text::AssistantUnreachable,
            ),
        ] {
            app.assistance.error = Some(error);
            let drawn = frame(&mut app);
            assert!(drawn.contains(app.t(expected)), "{expected:?}");
        }
    }
}
