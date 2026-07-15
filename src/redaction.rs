//! The single secret-safety boundary for terminal output and local transcripts.

use serde_json::Value;

use crate::{
    session_history::{RedactedText, RedactedTranscriptResponse},
    transcript::{TextBlock, TranscriptResponse},
};

#[derive(Debug, Clone, Default)]
pub struct Redactor {
    configured_token: String,
}

impl Redactor {
    pub fn new(configured_token: impl Into<String>) -> Self {
        Self {
            configured_token: configured_token.into(),
        }
    }

    pub fn text(&self, text: &str) -> String {
        let text = redact_bearer(text);
        if self.configured_token.is_empty() {
            text
        } else {
            text.replace(&self.configured_token, "[redacted credential]")
        }
    }

    pub fn value(&self, value: Value) -> Value {
        match value {
            Value::Object(values) => Value::Object(
                values
                    .into_iter()
                    .map(|(key, value)| {
                        let sensitive = [
                            "token",
                            "authorization",
                            "credential",
                            "secret",
                            "password",
                            "api_key",
                        ]
                        .iter()
                        .any(|term| key.to_ascii_lowercase().contains(term));
                        (
                            key,
                            if sensitive {
                                Value::String("[redacted]".into())
                            } else {
                                self.value(value)
                            },
                        )
                    })
                    .collect(),
            ),
            Value::Array(values) => {
                Value::Array(values.into_iter().map(|value| self.value(value)).collect())
            }
            Value::String(text) => Value::String(self.text(&text)),
            other => other,
        }
    }

    pub fn transcript_text(&self, text: &str) -> RedactedText {
        RedactedText::new(self.text(text))
    }

    pub fn transcript_response(&self, response: TranscriptResponse) -> RedactedTranscriptResponse {
        RedactedTranscriptResponse::new(TranscriptResponse {
            text_blocks: response
                .text_blocks
                .into_iter()
                .map(|block| TextBlock {
                    text: self.text(&block.text),
                })
                .collect(),
            structured_result: response.structured_result.map(|value| self.value(value)),
            remote_chat_id: response.remote_chat_id.map(|id| self.text(&id)),
        })
    }
}

fn redact_bearer(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut redact_next = false;
    let mut word_start = None;
    for (index, character) in text.char_indices() {
        if character.is_whitespace() {
            if let Some(start) = word_start.take() {
                let word = &text[start..index];
                output.push_str(if redact_next { "[redacted]" } else { word });
                redact_next = word.eq_ignore_ascii_case("bearer");
            }
            output.push(character);
        } else if word_start.is_none() {
            word_start = Some(index);
        }
    }
    if let Some(start) = word_start {
        let word = &text[start..];
        output.push_str(if redact_next { "[redacted]" } else { word });
    }
    output
}

#[cfg(test)]
mod tests {
    use super::Redactor;

    #[test]
    fn redacts_configured_and_structured_secrets() {
        let redactor = Redactor::new("very-secret-token");
        assert_eq!(
            redactor.text("Bearer very-secret-token"),
            "Bearer [redacted]"
        );
        assert_eq!(
            redactor.value(serde_json::json!({"secret": "nope"}))["secret"],
            "[redacted]"
        );
    }
}
