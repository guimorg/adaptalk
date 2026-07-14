use std::{io, time::Duration};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    PromptSubmitted(String),
    ResponseStarted,
    ResponseChunk(String),
    StructuredResult(Value),
    ResponseCompleted,
    Error(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConnectionStatus {
    #[default]
    Connecting,
    Ready,
    Loading,
    Complete,
    Error,
}

impl ConnectionStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Ready => "Ready",
            Self::Loading => "Loading…",
            Self::Complete => "Complete",
            Self::Error => "Error",
        }
    }
}

#[derive(Debug, Default)]
pub struct ChatState {
    transcript: Vec<String>,
    input: String,
    status: ConnectionStatus,
    scroll: u16,
    pending_response: Option<usize>,
}

impl ChatState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: ClientEvent) {
        match event {
            ClientEvent::PromptSubmitted(prompt) => {
                self.transcript
                    .push(format!("You: {}", redact_text(&prompt)));
                self.input.clear();
                self.status = ConnectionStatus::Loading;
            }
            ClientEvent::ResponseStarted => {
                self.transcript.push("Adapt: ".to_owned());
                self.pending_response = Some(self.transcript.len() - 1);
                self.status = ConnectionStatus::Loading;
            }
            ClientEvent::ResponseChunk(chunk) => {
                if let Some(index) = self.pending_response {
                    self.transcript[index].push_str(&redact_text(&chunk));
                } else {
                    self.transcript
                        .push(format!("Adapt: {}", redact_text(&chunk)));
                }
            }
            ClientEvent::StructuredResult(value) => {
                let value = redact_value(value);
                let rendered = serde_json::to_string_pretty(&value)
                    .unwrap_or_else(|_| "[unrenderable structured result]".to_owned());
                self.transcript.push(format!("Result:\n{rendered}"));
            }
            ClientEvent::ResponseCompleted => {
                self.pending_response = None;
                self.status = ConnectionStatus::Complete;
            }
            ClientEvent::Error(message) => {
                self.pending_response = None;
                self.transcript
                    .push(format!("Error: {}", redact_text(&message)));
                self.status = ConnectionStatus::Error;
            }
        }
    }

    pub fn set_ready(&mut self) {
        self.status = ConnectionStatus::Ready;
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn push_input(&mut self, character: char) {
        self.input.push(character);
    }

    pub fn pop_input(&mut self) {
        self.input.pop();
    }

    pub fn take_prompt(&mut self) -> Option<String> {
        let prompt = self.input.trim().to_owned();
        (!prompt.is_empty()).then_some(prompt)
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn transcript(&self) -> &[String] {
        &self.transcript
    }
}

pub struct TerminalSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TerminalSession {
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout));
        match terminal {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                let _ = disable_raw_mode();
                Err(error)
            }
        }
    }

    pub fn draw(&mut self, state: &ChatState) -> io::Result<()> {
        self.terminal.draw(|frame| render(frame, state)).map(|_| ())
    }

    pub fn next_key(&mut self) -> io::Result<Option<KeyEvent>> {
        if !event::poll(Duration::from_millis(250))? {
            return Ok(None);
        }
        match event::read()? {
            Event::Key(key) => Ok(Some(key)),
            _ => Ok(None),
        }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

pub fn is_exit_key(key: KeyEvent) -> bool {
    key.code == KeyCode::Esc
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

fn render(frame: &mut ratatui::Frame<'_>, state: &ChatState) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(frame.area());
    let transcript = Text::from(
        state
            .transcript
            .iter()
            .flat_map(|entry| [Line::from(entry.clone()), Line::from("")])
            .collect::<Vec<_>>(),
    );
    frame.render_widget(
        Paragraph::new(transcript)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Adapt conversation "),
            )
            .wrap(Wrap { trim: false })
            .scroll((state.scroll, 0)),
        areas[0],
    );
    frame.render_widget(
        Paragraph::new(state.input.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Prompt (Enter to send) "),
        ),
        areas[1],
    );
    let status_style = if state.status == ConnectionStatus::Error {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Green)
    };
    frame.render_widget(
        Paragraph::new(format!(
            "Status: {}  ·  Esc/Ctrl-C to exit",
            state.status.label()
        ))
        .style(status_style),
        areas[2],
    );
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

fn redact_text(text: &str) -> String {
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
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn client_events_produce_a_visible_conversation() {
        let mut state = ChatState::new();
        state.set_ready();
        state.apply(ClientEvent::PromptSubmitted("find incidents".into()));
        state.apply(ClientEvent::ResponseStarted);
        state.apply(ClientEvent::ResponseChunk("No active ".into()));
        state.apply(ClientEvent::ResponseChunk("incidents.".into()));
        state.apply(ClientEvent::ResponseCompleted);

        assert_eq!(state.status, ConnectionStatus::Complete);
        assert_eq!(
            state.transcript(),
            ["You: find incidents", "Adapt: No active incidents."]
        );
    }

    #[test]
    fn structured_results_and_errors_are_rendered_safely() {
        let mut state = ChatState::new();
        state.apply(ClientEvent::StructuredResult(serde_json::json!({
            "citation": "runbook",
            "bearer_token": "secret"
        })));
        state.apply(ClientEvent::Error(
            "request failed for Bearer secret".into(),
        ));

        let transcript = state.transcript().join("\n");
        assert!(transcript.contains("runbook"));
        assert!(transcript.contains("[redacted]"));
        assert!(!transcript.contains("secret"));
        assert_eq!(state.status, ConnectionStatus::Error);
    }

    #[test]
    fn input_and_exit_keys_are_handled_without_widget_state() {
        let mut state = ChatState::new();
        state.push_input('h');
        state.push_input('i');
        assert_eq!(state.take_prompt().as_deref(), Some("hi"));
        state.pop_input();
        assert_eq!(state.input(), "h");
        assert!(is_exit_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(is_exit_key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
    }

    #[test]
    fn render_shows_the_prompt_transcript_and_safe_error() {
        let mut state = ChatState::new();
        state.push_input('h');
        state.push_input('i');
        state.apply(ClientEvent::PromptSubmitted("hi".into()));
        state.apply(ClientEvent::Error("denied for Bearer secret".into()));
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| render(frame, &state)).unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("Adapt conversation"));
        assert!(rendered.contains("Prompt (Enter to send)"));
        assert!(rendered.contains("You: hi"));
        assert!(rendered.contains("Error: denied for Bearer [redacted]"));
        assert!(!rendered.contains("secret"));
    }
}
