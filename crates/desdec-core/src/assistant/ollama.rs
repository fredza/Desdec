//! A model served by `ollama` on this machine.
//!
//! The same shape of request as the remote provider, over a socket that never
//! leaves the loopback interface. What it costs is quality: a seven-billion
//! parameter model reads assembly noticeably less well than a frontier one,
//! which is the trade the reader is making when they choose it.

use std::time::Duration;

use serde_json::{Value, json};

use super::{Answer, Availability, Error, Prompt, Settings, transport_error};

/// Long enough to notice a server that is not there, short enough that the
/// preferences window does not hang while it finds out.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

pub fn ask(settings: &Settings, prompt: &Prompt) -> Result<Answer, Error> {
    let model = settings.model_or_default().to_owned();
    let body = json!({
        "model": model,
        "stream": false,
        "messages": [
            {"role": "system", "content": prompt.system},
            {"role": "user", "content": prompt.user},
        ],
    });

    let mut response = ureq::post(format!("{}/api/chat", settings.ollama_url_or_default()))
        .config()
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
            // Ollama says which model is missing here, which is the whole
            // content of the usual failure.
            message: answer["error"]
                .as_str()
                .map_or_else(|| answer.to_string(), str::to_owned),
        });
    }

    let text = answer["message"]["content"].as_str().unwrap_or_default();
    if text.trim().is_empty() {
        return Err(Error::Unreadable("the answer carried no text".to_owned()));
    }
    Ok(Answer {
        text: text.to_owned(),
        provider: settings.provider,
        model,
        // No token limit is asked for here, but the model's context is a limit
        // of its own, and a long listing can reach it.
        truncated: answer["done_reason"] == "length",
    })
}

/// Whether the server is there, and whether it has the model.
///
/// Both are worth telling apart: a server that is not running and a model that
/// was never pulled fail identically at the moment of asking, and the fix is a
/// different command each time.
pub fn availability(settings: &Settings) -> Availability {
    let url = settings.ollama_url_or_default();
    let wanted = settings.model_or_default();

    let Ok(mut response) = ureq::get(format!("{url}/api/tags"))
        .config()
        .http_status_as_error(false)
        .timeout_global(Some(PROBE_TIMEOUT))
        .build()
        .call()
    else {
        return Availability::Missing(format!("ollama serve ({url})"));
    };
    let Ok(listed) = response.body_mut().read_json::<Value>() else {
        return Availability::Missing(format!("ollama serve ({url})"));
    };

    if pulled(&listed, wanted) {
        Availability::Ready
    } else {
        Availability::Missing(format!("ollama pull {wanted}"))
    }
}

/// Whether the model list names this model.
///
/// Ollama reports `qwen2.5-coder:7b` for a model asked for as
/// `qwen2.5-coder`, so a name without a tag matches the default one.
fn pulled(listed: &Value, wanted: &str) -> bool {
    listed["models"].as_array().is_some_and(|models| {
        models
            .iter()
            .filter_map(|model| model["name"].as_str())
            .any(|name| {
                name == wanted || name.split_once(':').is_some_and(|(head, _)| head == wanted)
            })
    })
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead as _, BufReader, Read as _, Write as _},
        net::TcpListener,
    };

    use super::*;
    use crate::assistant::{Prompt, Provider};

    /// A server that answers one request, and hands back what it was sent.
    ///
    /// The parsing above can be tested against a literal; the request itself
    /// cannot. This is the only place the whole path — headers, body, socket,
    /// answer — is exercised, and it runs on the loopback interface in a few
    /// milliseconds.
    fn one_shot(answer: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
        let url = format!("http://{}", listener.local_addr().expect("bound"));
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("one connection");
            let mut reader = BufReader::new(&stream);
            let mut request = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
                request.push_str(&line);
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).expect("the whole body");

            let mut stream = &stream;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                answer.len()
            );
            let _ = stream.flush();
            String::from_utf8_lossy(&body).into_owned()
        });
        (url, handle)
    }

    /// The whole local path, over a real socket: what is sent is the prompt,
    /// and what comes back is the model's message and nothing else.
    #[test]
    fn the_request_carries_the_prompt_and_the_answer_is_read_back() {
        let (url, server) = one_shot(r#"{"message": {"content": "It compares two strings."}}"#);
        let settings = Settings {
            provider: Provider::Ollama,
            model: "test-model".to_owned(),
            ollama_url: url,
            ..Settings::default()
        };
        let prompt = Prompt {
            system: "Answer in English.".to_owned(),
            user: "0x401000  cmp %rax,%rbx".to_owned(),
        };

        let answer = super::ask(&settings, &prompt).expect("the stub answers");
        assert_eq!(answer.text, "It compares two strings.");
        assert_eq!(answer.model, "test-model");
        assert_eq!(answer.provider, Provider::Ollama);

        let sent = server.join().expect("the server thread");
        assert!(sent.contains("test-model"), "{sent}");
        assert!(sent.contains("Answer in English."), "{sent}");
        assert!(sent.contains("cmp"), "{sent}");
        // Streaming would arrive as fragments this code does not reassemble.
        assert!(sent.contains("\"stream\": false"), "{sent}");
    }

    /// A server that answers something else must not be read as an answer.
    #[test]
    fn an_answer_without_a_message_is_reported_rather_than_shown_empty() {
        let (url, server) = one_shot(r#"{"unexpected": true}"#);
        let settings = Settings {
            provider: Provider::Ollama,
            ollama_url: url,
            ..Settings::default()
        };
        let prompt = Prompt {
            system: String::new(),
            user: String::new(),
        };

        assert!(matches!(
            super::ask(&settings, &prompt),
            Err(Error::Unreadable(_))
        ));
        let _ = server.join();
    }

    #[test]
    fn a_model_is_recognised_with_or_without_its_tag() {
        let listed = json!({"models": [{"name": "qwen2.5-coder:7b"}, {"name": "llama3.2:latest"}]});
        assert!(pulled(&listed, "qwen2.5-coder:7b"));
        assert!(pulled(&listed, "qwen2.5-coder"));
        assert!(pulled(&listed, "llama3.2"));
        assert!(!pulled(&listed, "qwen2.5"));
        assert!(!pulled(&listed, "mistral"));
        assert!(!pulled(&json!({}), "anything"));
    }
}
