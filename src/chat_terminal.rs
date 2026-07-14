use std::io::{self, Write};

use crate::redaction::Redactor;
use crossterm::{
    cursor::{MoveDown, MoveTo, MoveToColumn, MoveUp, RestorePosition, SavePosition},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use serde_json::Value;

pub struct Repl {
    stdout: io::Stdout,
    waiting: bool,
    unverified_development_mode: bool,
    redactor: Redactor,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReplCommand {
    History,
    Open(String),
    Unknown(String),
}

const COMMAND_PALETTE: [(&str, &str); 2] = [
    ("/history", "Browse saved local sessions"),
    ("/open <id>", "Reopen a saved transcript"),
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
        _ => Some(ReplCommand::Unknown(format!(
            "unknown command `{input}`; use /history or /open <id>"
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

impl Repl {
    pub fn start(unverified_development_mode: bool) -> io::Result<Self> {
        let mut repl = Self {
            stdout: io::stdout(),
            waiting: false,
            unverified_development_mode,
            redactor: Redactor::default(),
        };
        repl.clear_transcript()?;
        Ok(repl)
    }

    pub fn set_redactor(&mut self, redactor: Redactor) {
        self.redactor = redactor;
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
        self.write_plain(": is working…\r\n")?;
        self.waiting = true;
        Ok(())
    }

    pub fn show_you(&mut self, message: &str) -> io::Result<()> {
        self.write_message("You", Color::Cyan, &self.redactor.text(message))
    }

    pub fn show_notice(&mut self, message: &str) -> io::Result<()> {
        self.write_message("History", Color::Green, &self.redactor.text(message))
    }

    pub fn show_adapt(&mut self, message: &str) -> io::Result<()> {
        self.clear_working()?;
        self.write_message("Adapt", Color::Magenta, &self.redactor.text(message))
    }

    pub fn show_structured_result(&mut self, value: Value) -> io::Result<()> {
        self.clear_working()?;
        let rendered = serde_json::to_string_pretty(&self.redactor.value(value))
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
            &format!(
                "Could not complete this prompt: {}",
                self.redactor.text(message)
            ),
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
        let mut lines = message.lines();
        if let Some(first_line) = lines.next() {
            self.write_label(label, color)?;
            self.write_plain(": ")?;
            self.write_plain(first_line)?;
            self.write_plain("\r\n")?;
        } else {
            self.write_label(label, color)?;
            self.write_plain(":\r\n")?;
        }
        for line in lines {
            self.write_plain("  ")?;
            self.write_plain(line)?;
            self.write_plain("\r\n")?;
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

#[cfg(test)]
mod tests {
    use super::{
        ReplCommand, completed_command, matching_commands, parse_command, shows_command_palette,
    };

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

    #[test]
    fn slash_opens_the_command_palette() {
        assert!(shows_command_palette("/"));
        assert!(shows_command_palette("/his"));
        assert!(shows_command_palette("/ope"));
        assert!(shows_command_palette("/history"));
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
}
