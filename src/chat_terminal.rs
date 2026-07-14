use std::io::{self, BufRead, Write};

use crossterm::{
    cursor::{MoveTo, MoveToColumn, MoveUp},
    execute,
    style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use serde_json::Value;

pub struct Repl {
    stdout: io::Stdout,
    waiting: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReplCommand {
    History,
    Open(String),
    Unknown(String),
}

pub fn parse_command(input: &str) -> Option<ReplCommand> {
    let input = input.trim();
    if !input.starts_with('/') {
        return None;
    }
    match input.split_once(char::is_whitespace) {
        Some(("/open", id)) if !id.trim().is_empty() => {
            Some(ReplCommand::Open(id.trim().to_owned()))
        }
        Some(("/open", _)) | None if input == "/open" => {
            Some(ReplCommand::Unknown("/open requires a session ID".into()))
        }
        Some(("/history", _)) | None if input == "/history" => Some(ReplCommand::History),
        _ => Some(ReplCommand::Unknown(format!(
            "unknown command `{input}`; use /history or /open <id>"
        ))),
    }
}

impl Repl {
    pub fn start(unverified_development_mode: bool) -> io::Result<Self> {
        let mut repl = Self {
            stdout: io::stdout(),
            waiting: false,
        };
        execute!(repl.stdout, Clear(ClearType::All), MoveTo(0, 0))?;
        repl.write_plain("AdaptTUI · Read-only mode · Ctrl-C to exit\n")?;
        if unverified_development_mode {
            repl.write_colored(
                "⚠ DEVELOPMENT MODE: ask_adapt is unverified and may perform mutations.\n",
                Color::Yellow,
            )?;
        }
        Ok(repl)
    }

    pub fn read_prompt(&mut self) -> io::Result<Option<String>> {
        self.write_label("You", Color::Cyan)?;
        self.write_plain(" › ")?;
        let mut input = String::new();
        let read = io::stdin().lock().read_line(&mut input)?;
        if read == 0 {
            self.write_plain("\n")?;
            return Ok(None);
        }
        Ok(Some(input.trim().to_owned()))
    }

    pub fn show_working(&mut self) -> io::Result<()> {
        self.write_label("Adapt", Color::Yellow)?;
        self.write_plain(": is working…\n")?;
        self.waiting = true;
        Ok(())
    }

    pub fn show_you(&mut self, message: &str) -> io::Result<()> {
        self.write_message("You", Color::Cyan, &redact_text(message))
    }

    pub fn show_notice(&mut self, message: &str) -> io::Result<()> {
        self.write_message("History", Color::Green, message)
    }

    pub fn show_adapt(&mut self, message: &str) -> io::Result<()> {
        self.clear_working()?;
        self.write_message("Adapt", Color::Magenta, &redact_text(message))
    }

    pub fn show_structured_result(&mut self, value: Value) -> io::Result<()> {
        self.clear_working()?;
        let rendered = serde_json::to_string_pretty(&redact_value(value))
            .unwrap_or_else(|_| "[unrenderable structured result]".to_owned());
        self.write_message("Result", Color::Blue, &rendered)
    }

    pub fn finish_response(&mut self) -> io::Result<()> {
        self.clear_working()
    }

    pub fn show_error(&mut self, message: &str) -> io::Result<()> {
        self.clear_working()?;
        self.write_message(
            "Error",
            Color::Red,
            &format!("Could not complete this prompt: {}", redact_text(message)),
        )
    }

    fn clear_working(&mut self) -> io::Result<()> {
        if self.waiting {
            execute!(
                self.stdout,
                MoveUp(1),
                MoveToColumn(0),
                Clear(ClearType::CurrentLine)
            )?;
            self.waiting = false;
        }
        Ok(())
    }

    fn write_message(&mut self, label: &str, color: Color, message: &str) -> io::Result<()> {
        let mut lines = message.lines();
        if let Some(first_line) = lines.next() {
            self.write_label(label, color)?;
            self.write_plain(": ")?;
            self.write_plain(first_line)?;
            self.write_plain("\n")?;
        } else {
            self.write_label(label, color)?;
            self.write_plain(":\n")?;
        }
        for line in lines {
            self.write_plain("  ")?;
            self.write_plain(line)?;
            self.write_plain("\n")?;
        }
        self.stdout.flush()
    }

    fn write_label(&mut self, label: &str, color: Color) -> io::Result<()> {
        execute!(
            self.stdout,
            SetForegroundColor(color),
            SetAttribute(Attribute::Bold)
        )?;
        write!(self.stdout, "{label}")?;
        execute!(self.stdout, SetAttribute(Attribute::Reset))?;
        Ok(())
    }

    fn write_colored(&mut self, text: &str, color: Color) -> io::Result<()> {
        execute!(self.stdout, SetForegroundColor(color))?;
        write!(self.stdout, "{text}")?;
        execute!(self.stdout, ResetColor)?;
        self.stdout.flush()
    }

    fn write_plain(&mut self, text: &str) -> io::Result<()> {
        write!(self.stdout, "{text}")?;
        self.stdout.flush()
    }
}

fn redact_value(value: Value) -> Value {
    match value {
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    let sensitive = ["token", "authorization", "credential"]
                        .iter()
                        .any(|term| key.to_ascii_lowercase().contains(term));
                    (
                        key,
                        if sensitive {
                            Value::String("[redacted]".to_owned())
                        } else {
                            redact_value(value)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(redact_value).collect()),
        Value::String(text) => Value::String(redact_text(&text)),
        other => other,
    }
}

pub fn redact_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut redact_next = false;
    let mut word_start = None;
    for (index, character) in text.char_indices() {
        if character.is_whitespace() {
            if let Some(start) = word_start.take() {
                let word = &text[start..index];
                if redact_next {
                    output.push_str("[redacted]");
                } else {
                    output.push_str(word);
                }
                redact_next = word.eq_ignore_ascii_case("bearer");
            }
            output.push(character);
        } else if word_start.is_none() {
            word_start = Some(index);
        }
    }
    if let Some(start) = word_start {
        let word = &text[start..];
        if redact_next {
            output.push_str("[redacted]");
        } else {
            output.push_str(word);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{ReplCommand, parse_command, redact_text, redact_value};

    #[test]
    fn text_errors_redact_bearer_credentials() {
        assert_eq!(
            redact_text("request failed for Bearer secret"),
            "request failed for Bearer [redacted]"
        );
    }

    #[test]
    fn structured_results_redact_sensitive_values() {
        let value =
            redact_value(serde_json::json!({"citation": "runbook", "bearer_token": "secret"}));
        assert_eq!(value["citation"], "runbook");
        assert_eq!(value["bearer_token"], "[redacted]");
    }

    #[test]
    fn recognizes_history_and_open_commands() {
        assert_eq!(parse_command("/history"), Some(ReplCommand::History));
        assert_eq!(
            parse_command("/open abc"),
            Some(ReplCommand::Open("abc".into()))
        );
        assert!(matches!(
            parse_command("/wat"),
            Some(ReplCommand::Unknown(_))
        ));
    }
}
