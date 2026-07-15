use std::io::{self, Write};
use std::time::Duration;

use crossterm::{
    cursor::{MoveDown, MoveTo, MoveToColumn, MoveUp, RestorePosition, SavePosition},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use serde_json::Value;

pub struct Repl<W: Write = io::Stdout> {
    stdout: W,
    waiting: bool,
    unverified_development_mode: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReplCommand {
    History,
    Open(String),
    ToggleStreaming,
    Unknown(String),
}

const COMMAND_PALETTE: [(&str, &str); 3] = [
    ("/history", "Browse saved local sessions"),
    ("/open <id>", "Reopen a saved transcript"),
    ("/stream", "Toggle mock response streaming"),
];

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
        Some(("/stream", _)) | None if input == "/stream" => Some(ReplCommand::ToggleStreaming),
        _ => Some(ReplCommand::Unknown(format!(
            "unknown command `{input}`; use /history, /open <id>, or /stream"
        ))),
    }
}

fn shows_command_palette(input: &str) -> bool {
    !matching_commands(input).is_empty()
}

fn matching_commands(input: &str) -> Vec<(&'static str, &'static str)> {
    let Some(query) = input.strip_prefix('/') else {
        return Vec::new();
    };
    COMMAND_PALETTE
        .into_iter()
        .filter(|(command, _)| fuzzy_matches(command.trim_start_matches('/'), query))
        .collect()
}

fn fuzzy_matches(command: &str, query: &str) -> bool {
    let mut command = command.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .all(|character| command.any(|candidate| candidate == character))
}

struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

impl Repl<io::Stdout> {
    pub fn start(unverified_development_mode: bool) -> io::Result<Self> {
        let mut repl = Self {
            stdout: io::stdout(),
            waiting: false,
            unverified_development_mode,
        };
        repl.clear_transcript()?;
        Ok(repl)
    }
}

impl<W: Write> Repl<W> {
    pub fn with_output(stdout: W, unverified_development_mode: bool) -> io::Result<Self> {
        let mut repl = Self {
            stdout,
            waiting: false,
            unverified_development_mode,
        };
        repl.clear_transcript()?;
        Ok(repl)
    }

    pub fn into_output(self) -> W {
        self.stdout
    }

    pub fn clear_transcript(&mut self) -> io::Result<()> {
        execute!(self.stdout, Clear(ClearType::All), MoveTo(0, 0))?;
        self.write_plain("AdaptTUI · Read-only mode · Ctrl-C to exit\n")?;
        if self.unverified_development_mode {
            self.write_colored(
                "⚠ DEVELOPMENT MODE: ask_adapt is unverified and may perform mutations.\n",
                Color::Yellow,
            )?;
        }
        self.waiting = false;
        Ok(())
    }

    pub fn read_prompt(&mut self) -> io::Result<Option<String>> {
        self.render_input("", 0)?;
        let _raw_mode = RawModeGuard::enable()?;
        let mut input = String::new();
        let mut palette_rows = 0;
        loop {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Enter => {
                    if let Some(completed) = completed_command(&input) {
                        input = completed;
                        self.render_input(&input, palette_rows)?;
                        palette_rows = command_palette_rows(&input);
                        continue;
                    }
                    self.clear_command_palette(palette_rows)?;
                    self.write_plain("\r\n")?;
                    return Ok(Some(input.trim().to_owned()));
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Tab => {
                    if let Some(completed) = completed_command(&input) {
                        input = completed;
                    }
                }
                KeyCode::Esc => input.clear(),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "interrupted"));
                }
                KeyCode::Char('d')
                    if key.modifiers.contains(KeyModifiers::CONTROL) && input.is_empty() =>
                {
                    self.clear_command_palette(palette_rows)?;
                    self.write_plain("\r\n")?;
                    return Ok(None);
                }
                KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.push(character);
                }
                _ => continue,
            }
            self.render_input(&input, palette_rows)?;
            palette_rows = command_palette_rows(&input);
        }
    }

    pub fn show_working(&mut self) -> io::Result<()> {
        self.write_label("Adapt", Color::Yellow)?;
        self.write_plain(": is typing…\r\n")?;
        self.waiting = true;
        Ok(())
    }

    pub fn show_you(&mut self, message: &str) -> io::Result<()> {
        self.write_message("You", Color::Cyan, message)
    }

    pub fn show_notice(&mut self, message: &str) -> io::Result<()> {
        self.write_message("History", Color::Green, message)
    }

    pub fn show_adapt(&mut self, message: &str) -> io::Result<()> {
        self.clear_working()?;
        self.write_message("Adapt", Color::Magenta, message)
    }

    pub async fn show_adapt_streaming(&mut self, message: &str, delay: Duration) -> io::Result<()> {
        self.clear_working()?;
        let mut writer = LineAwareMessageWriter::start(&mut self.stdout, "Adapt", Color::Magenta)?;

        let chunks = streaming_chunks(message);
        for (index, chunk) in chunks.iter().enumerate() {
            writer.write_text(chunk)?;
            writer.flush()?;
            if index + 1 < chunks.len() {
                tokio::time::sleep(delay).await;
            }
        }
        writer.finish()
    }

    pub fn show_structured_result(&mut self, value: Value) -> io::Result<()> {
        self.clear_working()?;
        let rendered = serde_json::to_string_pretty(&value)
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
            &format!("Could not complete this prompt: {}", message),
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

    fn render_input(&mut self, input: &str, previous_palette_rows: u16) -> io::Result<()> {
        execute!(self.stdout, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
        self.write_label("You", Color::Cyan)?;
        self.write_plain(" › ")?;
        self.write_plain(input)?;
        self.clear_command_palette(previous_palette_rows)?;
        let commands = matching_commands(input);
        if shows_command_palette(input) {
            execute!(
                self.stdout,
                SavePosition,
                MoveDown(1),
                MoveToColumn(0),
                Clear(ClearType::CurrentLine)
            )?;
            self.write_colored("  Commands", Color::DarkGrey)?;
            for (command, description) in commands {
                execute!(
                    self.stdout,
                    MoveDown(1),
                    MoveToColumn(0),
                    Clear(ClearType::CurrentLine)
                )?;
                self.write_colored(&format!("  {command:<12}"), Color::Cyan)?;
                self.write_colored(description, Color::DarkGrey)?;
            }
            execute!(self.stdout, RestorePosition)?;
        }
        self.stdout.flush()
    }

    fn clear_command_palette(&mut self, rows: u16) -> io::Result<()> {
        if rows > 0 {
            for _ in 0..rows {
                execute!(
                    self.stdout,
                    MoveDown(1),
                    MoveToColumn(0),
                    Clear(ClearType::CurrentLine)
                )?;
            }
            execute!(self.stdout, MoveUp(rows))?;
        }
        Ok(())
    }

    fn write_message(&mut self, label: &str, color: Color, message: &str) -> io::Result<()> {
        let mut writer = LineAwareMessageWriter::start(&mut self.stdout, label, color)?;
        writer.write_text(message)?;
        writer.finish()
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

struct LineAwareMessageWriter<'a, W: Write> {
    output: &'a mut W,
    first_line: bool,
    at_line_start: bool,
    ended_with_newline: bool,
    pending_carriage_return: bool,
    saw_input: bool,
}

impl<'a, W: Write> LineAwareMessageWriter<'a, W> {
    fn start(output: &'a mut W, label: &str, color: Color) -> io::Result<Self> {
        execute!(
            output,
            SetForegroundColor(color),
            SetAttribute(Attribute::Bold)
        )?;
        write!(output, "{label}")?;
        execute!(output, SetAttribute(Attribute::Reset))?;
        Ok(Self {
            output,
            first_line: true,
            at_line_start: true,
            ended_with_newline: false,
            pending_carriage_return: false,
            saw_input: false,
        })
    }

    fn write_text(&mut self, text: &str) -> io::Result<()> {
        for character in text.chars() {
            if self.pending_carriage_return {
                if character == '\n' {
                    self.write_newline()?;
                    self.pending_carriage_return = false;
                    continue;
                }
                self.write_content('\r')?;
                self.pending_carriage_return = false;
            }
            if character == '\r' {
                self.pending_carriage_return = true;
            } else if character == '\n' {
                self.write_newline()?;
            } else {
                self.write_content(character)?;
            }
        }
        Ok(())
    }

    fn finish(mut self) -> io::Result<()> {
        if self.pending_carriage_return {
            self.write_content('\r')?;
        }
        if !self.ended_with_newline {
            if !self.saw_input {
                write!(self.output, ":")?;
            }
            write!(self.output, "\r\n")?;
        }
        self.output.flush()
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }

    fn write_content(&mut self, character: char) -> io::Result<()> {
        if self.at_line_start {
            if self.first_line {
                write!(self.output, ": ")?;
            } else {
                write!(self.output, "  ")?;
            }
        }
        write!(self.output, "{character}")?;
        self.at_line_start = false;
        self.ended_with_newline = false;
        self.saw_input = true;
        Ok(())
    }

    fn write_newline(&mut self) -> io::Result<()> {
        if self.at_line_start {
            if self.first_line {
                write!(self.output, ": ")?;
            } else {
                write!(self.output, "  ")?;
            }
        }
        write!(self.output, "\r\n")?;
        self.first_line = false;
        self.at_line_start = true;
        self.ended_with_newline = true;
        self.saw_input = true;
        Ok(())
    }
}

fn command_palette_rows(input: &str) -> u16 {
    let count: u16 = matching_commands(input)
        .len()
        .try_into()
        .unwrap_or(u16::MAX);
    if count == 0 {
        0
    } else {
        count.saturating_add(1)
    }
}

fn completed_command(input: &str) -> Option<String> {
    if input.chars().any(char::is_whitespace) {
        return None;
    }
    let commands = matching_commands(input);
    let (command, _) = commands.as_slice().first()?;
    if commands.len() != 1 || *command == input {
        return None;
    }
    Some(if *command == "/open <id>" {
        "/open ".into()
    } else {
        (*command).into()
    })
}

fn streaming_chunks(message: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut leading_whitespace = String::new();

    for segment in message.split_inclusive(char::is_whitespace) {
        if segment.chars().all(char::is_whitespace) {
            if let Some(last) = chunks.last_mut() {
                last.push_str(segment);
            } else {
                leading_whitespace.push_str(segment);
            }
        } else {
            chunks.push(format!("{leading_whitespace}{segment}"));
            leading_whitespace.clear();
        }
    }

    if !leading_whitespace.is_empty() {
        if let Some(last) = chunks.last_mut() {
            last.push_str(&leading_whitespace);
        } else {
            chunks.push(leading_whitespace);
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        ReplCommand, completed_command, matching_commands, parse_command, shows_command_palette,
        streaming_chunks,
    };

    #[test]
    fn recognizes_history_and_open_commands() {
        assert_eq!(parse_command("/history"), Some(ReplCommand::History));
        assert_eq!(parse_command("/stream"), Some(ReplCommand::ToggleStreaming));
        assert_eq!(
            parse_command("/open abc"),
            Some(ReplCommand::Open("abc".into()))
        );
        assert!(matches!(
            parse_command("/wat"),
            Some(ReplCommand::Unknown(_))
        ));
    }

    #[test]
    fn slash_opens_the_command_palette() {
        assert!(shows_command_palette("/"));
        assert!(shows_command_palette("/his"));
        assert!(shows_command_palette("/ope"));
        assert!(shows_command_palette("/history"));
        assert!(shows_command_palette("/stream"));
        assert!(!shows_command_palette("/unknown"));
        assert!(!shows_command_palette(""));
        assert!(!shows_command_palette("ask Adapt"));
    }

    #[test]
    fn palette_fuzzy_filters_to_matching_commands() {
        assert_eq!(
            matching_commands("/his")
                .into_iter()
                .map(|(command, _)| command)
                .collect::<Vec<_>>(),
            vec!["/history"]
        );
        assert_eq!(
            matching_commands("/pn")
                .into_iter()
                .map(|(command, _)| command)
                .collect::<Vec<_>>(),
            vec!["/open <id>"]
        );
    }

    #[test]
    fn fuzzy_match_completes_to_a_canonical_command() {
        assert_eq!(completed_command("/his"), Some("/history".into()));
        assert_eq!(completed_command("/pn"), Some("/open ".into()));
        assert_eq!(completed_command("/history"), None);
    }

    #[test]
    fn streaming_chunks_keep_word_boundaries_and_whitespace() {
        assert_eq!(
            streaming_chunks("Hello,  world\nfrom Adapt"),
            vec!["Hello,  ", "world\n", "from ", "Adapt"]
        );
    }

    #[test]
    fn streaming_chunks_handle_empty_and_whitespace_only_messages() {
        assert!(streaming_chunks("").is_empty());
        assert_eq!(streaming_chunks("  \n"), vec!["  \n"]);
    }

    #[test]
    fn renders_typing_indicator_at_the_user_visible_seam() {
        let mut repl = super::Repl::with_output(Vec::new(), false).unwrap();
        repl.show_working().unwrap();
        let output = visible(&repl.into_output());
        assert!(output.contains("Adapt: is typing…\r\n"));
    }

    #[test]
    fn immediate_multiline_output_indents_continuation_lines() {
        let mut repl = super::Repl::with_output(Vec::new(), false).unwrap();
        repl.show_adapt("first\nsecond").unwrap();
        let output = visible(&repl.into_output());
        assert!(output.contains("Adapt: first\r\n  second\r\n"));
    }

    #[test]
    fn immediate_output_preserves_canonical_line_semantics() {
        for (message, expected) in [
            ("", "Adapt:\r\n"),
            ("first\nsecond", "Adapt: first\r\n  second\r\n"),
            ("first\r\nsecond", "Adapt: first\r\n  second\r\n"),
            ("trailing\n", "Adapt: trailing\r\n"),
        ] {
            let mut repl = super::Repl::with_output(Vec::new(), false).unwrap();
            repl.show_adapt(message).unwrap();
            let output = visible(&repl.into_output());
            assert!(
                output.ends_with(expected),
                "message {message:?}: {output:?}"
            );
        }
    }

    #[tokio::test]
    async fn streamed_multiline_output_uses_the_same_line_formatting() {
        let mut repl = super::Repl::with_output(Vec::new(), false).unwrap();
        repl.show_adapt_streaming("first\nsecond", Duration::ZERO)
            .await
            .unwrap();
        let output = visible(&repl.into_output());
        assert!(output.contains("Adapt: first\r\n  second\r\n"));
    }

    fn visible(output: &[u8]) -> String {
        let text = String::from_utf8_lossy(output);
        let mut visible = String::new();
        let mut in_escape = false;
        for character in text.chars() {
            if in_escape {
                if character.is_ascii_alphabetic() {
                    in_escape = false;
                }
            } else if character == '\x1b' {
                in_escape = true;
            } else {
                visible.push(character);
            }
        }
        visible
    }
}
