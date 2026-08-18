//! Optional AI assistance: a second reader for a listing Desdec has decoded.
//!
//! Everything else in this crate answers from the bytes and says so. A model
//! does not: it produces a plausible reading, which is a different kind of
//! answer and is labelled as one everywhere it appears. The rules that keep
//! that honest are enforced here rather than left to the interface:
//!
//! - **Opt in, twice.** No provider is configured by default, and nothing is
//!   sent until the reader asks a question. A binary under analysis is
//!   potentially hostile and frequently confidential; neither is anyone's to
//!   hand out by accident.
//! - **Facts, never the file.** What leaves this process is the text Desdec
//!   already decoded — instructions, symbol names, imports, section
//!   entropies — assembled by [`prompt`] and readable in full before it is
//!   sent. The binary itself is never uploaded.
//! - **Bounded.** Every request has a deadline and a size, so a hung endpoint
//!   cannot hold the interface and a large function cannot become a large
//!   bill.
//!
//! Two providers, because the choice is a real one. [`Provider::Ollama`] runs
//! a model on this machine — nothing reaches the network, at the cost of
//! whatever the local model is worth. [`Provider::Claude`] sends the extracted
//! facts to Anthropic, which answers far better and means the listing has left
//! the building. The interface says which is in use, every time it answers.

pub mod prompt;

mod claude;
mod ollama;

use std::{fmt, path::PathBuf, time::Duration};

pub use prompt::{Prompt, Question};

/// Longest a provider may take before the request is abandoned.
///
/// A local model on a large function takes a while; a wedged endpoint takes
/// forever, and the reader is left looking at a spinner either way.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Where the reasoning comes from.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Provider {
    /// Nothing is configured; the assistant answers nothing.
    #[default]
    None,
    /// A model served by `ollama` on this machine.
    Ollama,
    /// Anthropic's API, over the network.
    Claude,
}

impl Provider {
    pub const ALL: &[Self] = &[Self::None, Self::Ollama, Self::Claude];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "—",
            Self::Ollama => "Ollama",
            Self::Claude => "Claude",
        }
    }

    /// Whether using this provider sends the extracted facts off the machine.
    ///
    /// The interface asks this to decide how loudly to say so.
    #[must_use]
    pub const fn leaves_the_machine(self) -> bool {
        matches!(self, Self::Claude)
    }

    /// A sensible model to start from, shown as the placeholder.
    #[must_use]
    pub const fn default_model(self) -> &'static str {
        match self {
            Self::None => "",
            // Small enough to run on a laptop, good enough to read assembly.
            Self::Ollama => "qwen2.5-coder:7b",
            Self::Claude => claude::DEFAULT_MODEL,
        }
    }
}

/// Where a local Ollama server listens unless told otherwise.
pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";

/// Everything a request needs beyond the question itself.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub provider: Provider,
    /// Model name, or empty for [`Provider::default_model`].
    pub model: String,
    /// Base URL of the local Ollama server.
    pub ollama_url: String,
    /// File holding the Anthropic API key.
    ///
    /// A path rather than the key itself: preferences are written to disk in
    /// plain text, and a key belongs in a file the user controls the
    /// permissions of — or in `ANTHROPIC_API_KEY`, which is looked at first.
    pub api_key_path: PathBuf,
    pub timeout: Duration,
}

impl Settings {
    #[must_use]
    pub fn model_or_default(&self) -> &str {
        if self.model.trim().is_empty() {
            self.provider.default_model()
        } else {
            self.model.trim()
        }
    }

    #[must_use]
    pub fn ollama_url_or_default(&self) -> &str {
        let url = self.ollama_url.trim().trim_end_matches('/');
        if url.is_empty() {
            DEFAULT_OLLAMA_URL
        } else {
            url
        }
    }

    fn deadline(&self) -> Duration {
        if self.timeout.is_zero() {
            DEFAULT_TIMEOUT
        } else {
            self.timeout
        }
    }
}

/// One answer, with everything needed to say where it came from.
#[derive(Clone, Debug)]
pub struct Answer {
    pub text: String,
    pub provider: Provider,
    pub model: String,
    /// Whether the provider stopped at its token limit rather than at the end
    /// of what it had to say.
    ///
    /// A paragraph cut off mid-sentence still reads like a paragraph, and the
    /// reader would take the missing half for an answer that ended there. The
    /// interface says so instead.
    pub truncated: bool,
}

/// Why an answer could not be produced.
///
/// Separate variants rather than one string: the interface says something
/// different for each — a missing key is a settings problem, a refusal is not
/// a failure at all, and a timeout invites trying a smaller question.
#[derive(Clone, Debug)]
pub enum Error {
    /// No provider chosen, so nothing was attempted.
    NotConfigured,
    /// No API key in the environment or in the configured file.
    NoApiKey,
    /// The provider could not be reached at all.
    Unreachable(String),
    /// It answered, unhappily. The status and its message, as it gave them.
    Rejected { status: u16, message: String },
    /// The deadline passed with no answer.
    TimedOut,
    /// The model declined to answer. Not an error in the machinery: an
    /// outcome, which the interface reports as such.
    Declined { category: Option<String> },
    /// The answer arrived in a shape this code does not know how to read.
    Unreadable(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(formatter, "no assistant is configured"),
            Self::NoApiKey => write!(formatter, "no API key was found"),
            Self::Unreachable(detail) => write!(formatter, "unreachable: {detail}"),
            Self::Rejected { status, message } => write!(formatter, "HTTP {status}: {message}"),
            Self::TimedOut => write!(formatter, "the request timed out"),
            Self::Declined { category } => match category {
                Some(category) => write!(formatter, "declined ({category})"),
                None => write!(formatter, "declined"),
            },
            Self::Unreadable(detail) => write!(formatter, "unreadable answer: {detail}"),
        }
    }
}

impl std::error::Error for Error {}

/// Asks the configured provider, and blocks until it answers.
///
/// Called from a worker thread: a local model takes seconds and a remote one
/// can take longer, and the interface draws sixty frames in between.
///
/// # Errors
///
/// See [`Error`]. Nothing here panics on a hostile answer: a provider is
/// another program, and its output is parsed as untrusted input.
pub fn ask(settings: &Settings, prompt: &Prompt) -> Result<Answer, Error> {
    match settings.provider {
        Provider::None => Err(Error::NotConfigured),
        Provider::Ollama => ollama::ask(settings, prompt),
        Provider::Claude => claude::ask(settings, prompt),
    }
}

/// What is known about the configured provider, without asking it anything.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Availability {
    /// Ready to be asked.
    Ready,
    /// Nothing is configured.
    NotConfigured,
    /// Configured, but something it needs is missing.
    Missing(String),
}

/// Checks the provider is usable, cheaply enough for a preferences window.
///
/// For Ollama this is a request to the server, which is local; for Claude it
/// only looks for a key, since finding out whether a key works costs a
/// request the user did not ask for.
#[must_use]
pub fn availability(settings: &Settings) -> Availability {
    match settings.provider {
        Provider::None => Availability::NotConfigured,
        Provider::Ollama => ollama::availability(settings),
        Provider::Claude => match claude::api_key(settings) {
            Some(_) => Availability::Ready,
            None => Availability::Missing("ANTHROPIC_API_KEY".to_owned()),
        },
    }
}

/// Turns a ureq failure into one of ours, keeping the distinctions the
/// interface acts on.
fn transport_error(error: &ureq::Error) -> Error {
    match error {
        ureq::Error::Timeout(_) => Error::TimedOut,
        other => Error::Unreachable(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unconfigured_assistant_asks_nothing() {
        let settings = Settings::default();
        let prompt = Prompt {
            system: String::new(),
            user: String::new(),
        };
        assert!(matches!(ask(&settings, &prompt), Err(Error::NotConfigured)));
        assert_eq!(availability(&settings), Availability::NotConfigured);
    }

    /// Every provider must name a model it can actually be asked for, or the
    /// placeholder in the preferences window is a lie.
    #[test]
    fn every_provider_suggests_a_model() {
        for provider in Provider::ALL {
            let named = !provider.default_model().is_empty();
            assert_eq!(named, *provider != Provider::None, "{provider:?}");
        }
    }

    #[test]
    fn only_the_remote_provider_leaves_the_machine() {
        assert!(!Provider::None.leaves_the_machine());
        assert!(!Provider::Ollama.leaves_the_machine());
        assert!(Provider::Claude.leaves_the_machine());
    }

    #[test]
    fn blank_settings_fall_back_to_the_defaults() {
        let settings = Settings {
            provider: Provider::Ollama,
            model: "  ".to_owned(),
            ollama_url: "http://elsewhere:1234/".to_owned(),
            ..Settings::default()
        };
        assert_eq!(settings.model_or_default(), "qwen2.5-coder:7b");
        assert_eq!(settings.ollama_url_or_default(), "http://elsewhere:1234");
        assert_eq!(
            Settings::default().ollama_url_or_default(),
            DEFAULT_OLLAMA_URL
        );
        assert_eq!(Settings::default().deadline(), DEFAULT_TIMEOUT);
    }
}
