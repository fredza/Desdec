//! Anthropic's Messages API.
//!
//! The one provider here that sends anything off the machine, which is why the
//! interface names it on every answer and why the key is never stored in the
//! preferences file.

use std::{fs, path::Path};

use serde_json::{Value, json};

use super::{Answer, Error, Prompt, Provider, Settings, transport_error};

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

/// The model asked unless the reader names another.
pub const DEFAULT_MODEL: &str = "claude-opus-5";

/// Room for the answer, and for the thinking that precedes it.
///
/// On this model `max_tokens` bounds both together, so a budget sized for the
/// paragraph alone would cut the paragraph off before it started.
const MAX_TOKENS: u32 = 8_000;

/// Server-side fallback, which is not optional for this application.
///
/// Reading a binary is exactly the shape of request Anthropic's safety
/// classifiers are trained to be careful about, and a legitimate one — a
/// crackme, a suspicious download, one's own program — can trip them. With
/// this, a declined request is re-served by another model inside the same
/// call instead of coming back empty.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// Where the key is read from, in order.
///
/// The environment first, so a key already exported for other tools is found
/// without being copied anywhere; then a file the user names, whose
/// permissions are theirs to set. The key itself is never written to the
/// preferences file, which is plain text and backed up with everything else.
pub fn api_key(settings: &Settings) -> Option<String> {
    if let Some(key) = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty())
    {
        return Some(key);
    }
    key_from_file(&settings.api_key_path)
}

fn key_from_file(path: &Path) -> Option<String> {
    if path.as_os_str().is_empty() {
        return None;
    }
    fs::read_to_string(path)
        .ok()
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty())
}

pub fn ask(settings: &Settings, prompt: &Prompt) -> Result<Answer, Error> {
    let key = api_key(settings).ok_or(Error::NoApiKey)?;
    let model = settings.model_or_default().to_owned();

    let body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        // The reader is waiting in front of a window: a shorter, quicker
        // answer beats a deeper one they have watched a spinner for.
        "output_config": {"effort": "medium"},
        "fallbacks": "default",
        "system": prompt.system,
        "messages": [{"role": "user", "content": prompt.user}],
    });

    let mut response = ureq::post(ENDPOINT)
        .header("x-api-key", &key)
        .header("anthropic-version", API_VERSION)
        .header("anthropic-beta", FALLBACK_BETA)
        .config()
        // Read the body of a rejection rather than only its number: the
        // message says which of a dozen things was wrong with the request.
        .http_status_as_error(false)
        .timeout_global(Some(settings.deadline()))
        .build()
        .send_json(&body)
        .map_err(|error| transport_error(&error))?;

    let status = response.status().as_u16();
    let answer: Value = response
        .body_mut()
        .read_json()
        .map_err(|error| Error::Unreadable(error.to_string()))?;
    if status != 200 {
        return Err(Error::Rejected {
            status,
            message: message_of(&answer),
        });
    }
    read_answer(&answer, settings.provider, &model)
}

/// Reads the message, or says why there is none.
fn read_answer(answer: &Value, provider: Provider, model: &str) -> Result<Answer, Error> {
    // Checked before the content: a declined request answers 200 with an
    // empty or partial body, and reading it as an answer would present
    // nothing as if it were something.
    if answer["stop_reason"] == "refusal" {
        return Err(Error::Declined {
            category: answer["stop_details"]["category"]
                .as_str()
                .map(str::to_owned),
        });
    }

    let text: String = answer["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block["type"] == "text")
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    if text.trim().is_empty() {
        return Err(Error::Unreadable("the answer carried no text".to_owned()));
    }

    Ok(Answer {
        text,
        provider,
        // The model that answered, which is not always the one asked for: a
        // fallback names itself here, and the reader should see that.
        model: answer["model"].as_str().unwrap_or(model).to_owned(),
        // `MAX_TOKENS` covers the thinking and the paragraph together, so a
        // long enough reading can end here rather than at its own conclusion.
        truncated: answer["stop_reason"] == "max_tokens",
    })
}

/// The message out of an error body, or the body itself when it is shaped
/// some other way.
fn message_of(answer: &Value) -> String {
    answer["error"]["message"]
        .as_str()
        .map_or_else(|| answer.to_string(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_blocks_are_joined_and_the_answering_model_is_kept() {
        let answer = json!({
            "model": "claude-opus-4-8",
            "stop_reason": "end_turn",
            "content": [
                {"type": "thinking", "thinking": ""},
                {"type": "text", "text": "First half. "},
                {"type": "text", "text": "Second half."},
            ],
        });

        let answer =
            read_answer(&answer, Provider::Claude, "claude-opus-5").expect("a text answer");
        assert_eq!(answer.text, "First half. Second half.");
        assert_eq!(answer.model, "claude-opus-4-8");
        assert!(!answer.truncated);
    }

    /// `max_tokens` bounds the thinking and the paragraph together, so a
    /// reading can end at the limit rather than at its own conclusion. Shown as
    /// a whole answer, the half that arrived would be read as all there was.
    #[test]
    fn an_answer_cut_off_at_the_limit_is_marked_as_cut_off() {
        let answer = json!({
            "stop_reason": "max_tokens",
            "content": [{"type": "text", "text": "It reads a password and then"}],
        });

        let answer = read_answer(&answer, Provider::Claude, "claude-opus-5").expect("the partial");
        assert!(answer.truncated);
        assert_eq!(answer.text, "It reads a password and then");
    }

    /// A declined request answers 200 with nothing useful in it. Reading that
    /// as an answer would show the reader an empty paragraph and call it a
    /// reading.
    #[test]
    fn a_refusal_is_reported_as_one_rather_than_as_an_empty_answer() {
        let answer = json!({
            "stop_reason": "refusal",
            "stop_details": {"type": "refusal", "category": "cyber"},
            "content": [],
        });

        match read_answer(&answer, Provider::Claude, "claude-opus-5") {
            Err(Error::Declined { category }) => assert_eq!(category.as_deref(), Some("cyber")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_answer_with_no_text_is_not_passed_off_as_one() {
        let answer = json!({"stop_reason": "end_turn", "content": []});
        assert!(matches!(
            read_answer(&answer, Provider::Claude, ""),
            Err(Error::Unreadable(_))
        ));
    }

    #[test]
    fn a_rejection_reports_what_the_service_said() {
        let answer = json!({"type": "error", "error": {"message": "credit balance is too low"}});
        assert_eq!(message_of(&answer), "credit balance is too low");
        assert!(message_of(&json!({"unexpected": true})).contains("unexpected"));
    }

    /// The key is the one secret this application handles; it must never be
    /// read out of a path that was never configured.
    #[test]
    fn no_configured_file_means_no_key_from_a_file() {
        assert_eq!(key_from_file(Path::new("")), None);
        assert_eq!(key_from_file(Path::new("/nonexistent/desdec-key")), None);
    }
}
