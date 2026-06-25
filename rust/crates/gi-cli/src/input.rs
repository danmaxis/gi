//! Interactive line editor with an opencode/crush-style live command popup.
//!
//! Typing `/` opens a filtering menu of slash commands (with descriptions) that
//! narrows as you type. `↑/↓` move the highlight, `Enter` runs the highlighted
//! command (or inserts it plus a space when it takes an argument), `Tab` inserts
//! the command without running it, and `Esc` dismisses the menu. History (`↑/↓`
//! when the menu is closed), `Ctrl-C`/`Ctrl-D`, and `Shift+Enter`/`Ctrl+J`
//! newlines are preserved. Non-TTY input falls back to a plain line read.

use std::collections::BTreeSet;
use std::io::{self, IsTerminal, Write};

use crossterm::cursor::{MoveToColumn, MoveToPreviousLine};
use crossterm::event::{read, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::queue;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOutcome {
    Submit(String),
    Cancel,
    Exit,
}

const POPUP_MAX: usize = 5;

/// One row in the command popup.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PopupItem {
    command: String,
    description: String,
    needs_arg: bool,
}

pub struct LineEditor {
    prompt: String,
    /// Visible width of the prompt (ANSI stripped) — used for cursor column math
    /// so a themed/colored prompt indicator positions correctly. Slice 14a.
    prompt_width: usize,
    completions: Vec<String>,
    history: Vec<String>,
    color: bool,
}

impl LineEditor {
    #[must_use]
    pub fn new(prompt: impl Into<String>, completions: Vec<String>) -> Self {
        let prompt = prompt.into();
        let prompt_width = crate::render::strip_ansi(&prompt).chars().count();
        Self {
            prompt,
            prompt_width,
            completions: normalize_completions(completions),
            history: Vec::new(),
            color: std::env::var_os("NO_COLOR").is_none(),
        }
    }

    pub fn push_history(&mut self, entry: impl Into<String>) {
        let entry = entry.into();
        if entry.trim().is_empty() {
            return;
        }
        if self.history.last().map(String::as_str) == Some(entry.as_str()) {
            return;
        }
        self.history.push(entry);
    }

    pub fn set_completions(&mut self, completions: Vec<String>) {
        self.completions = normalize_completions(completions);
    }

    /// Update the prompt indicator (e.g. when the theme changes), recomputing
    /// its visible width for cursor math. Slice 14a.
    pub fn set_prompt(&mut self, prompt: impl Into<String>) {
        self.prompt = prompt.into();
        self.prompt_width = crate::render::strip_ansi(&self.prompt).chars().count();
    }

    /// Build the filtered command popup for the current buffer, or `None` when a
    /// command menu should not be shown (not a bare slash command).
    fn build_popup(&self, buffer: &str) -> Option<Vec<PopupItem>> {
        if !buffer.starts_with('/') || buffer.contains(' ') || buffer.contains('\n') {
            return None;
        }
        let mut scored: Vec<(i64, &String)> = if buffer == "/" {
            // Empty filter: order by context/usefulness (curated priorities +
            // recent use), not shortest-length, and keep answer-style tokens
            // (`/y`, `/n`, …) out of the initial suggestions. Slice 11.
            self.completions
                .iter()
                .filter(|candidate| !candidate.contains(' '))
                .map(|candidate| (empty_filter_rank(candidate, &self.history), candidate))
                .collect()
        } else {
            self.completions
                .iter()
                .filter(|candidate| !candidate.contains(' '))
                .filter_map(|candidate| {
                    fuzzy_score(candidate, buffer).map(|score| (score, candidate))
                })
                .collect()
        };
        // Sort by score descending; tie-break by name for deterministic order.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        scored.truncate(POPUP_MAX);
        Some(
            scored
                .into_iter()
                .map(|(_, candidate)| PopupItem {
                    command: candidate.clone(),
                    description: describe_completion(candidate).unwrap_or("").to_string(),
                    needs_arg: command_needs_arg(candidate),
                })
                .collect(),
        )
    }

    pub fn read_line(&mut self) -> io::Result<ReadOutcome> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return self.read_line_fallback();
        }
        enable_raw_mode()?;
        let outcome = self.edit();
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = write!(stdout, "\r\n");
        let _ = stdout.flush();
        outcome
    }

    #[allow(clippy::too_many_lines)]
    fn edit(&mut self) -> io::Result<ReadOutcome> {
        let mut buffer = String::new();
        let mut cursor = 0usize; // char index
        let mut selected = 0usize;
        let mut dismissed = false;
        let mut hist_pos = self.history.len();
        let mut draft = String::new();

        loop {
            let popup = if dismissed {
                None
            } else {
                self.build_popup(&buffer)
            };
            if let Some(items) = &popup {
                selected = selected.min(items.len().saturating_sub(1));
            }
            self.render(&buffer, cursor, popup.as_deref(), selected)?;

            let Event::Key(key) = read()? else {
                continue;
            };
            if key.kind == KeyEventKind::Release {
                continue;
            }
            let char_count = buffer.chars().count();
            match (key.code, key.modifiers) {
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    self.render(&buffer, cursor, None, 0)?;
                    return Ok(if buffer.is_empty() {
                        ReadOutcome::Exit
                    } else {
                        ReadOutcome::Cancel
                    });
                }
                (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                    if buffer.is_empty() {
                        self.render(&buffer, cursor, None, 0)?;
                        return Ok(ReadOutcome::Exit);
                    }
                }
                (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                    insert_char(&mut buffer, &mut cursor, '\n');
                    selected = 0;
                    dismissed = false;
                }
                (KeyCode::Enter, modifiers) if modifiers.contains(KeyModifiers::SHIFT) => {
                    insert_char(&mut buffer, &mut cursor, '\n');
                    selected = 0;
                    dismissed = false;
                }
                (KeyCode::Enter, _) => {
                    if let Some(items) = &popup {
                        if let Some(item) = items.get(selected) {
                            if item.needs_arg {
                                buffer = format!("{} ", item.command);
                                cursor = buffer.chars().count();
                                selected = 0;
                                continue;
                            }
                            self.render(&item.command, item.command.chars().count(), None, 0)?;
                            return Ok(ReadOutcome::Submit(item.command.clone()));
                        }
                    }
                    self.render(&buffer, cursor, None, 0)?;
                    return Ok(ReadOutcome::Submit(buffer));
                }
                (KeyCode::Tab, _) => {
                    if let Some(items) = &popup {
                        if let Some(item) = items.get(selected) {
                            buffer = if item.needs_arg {
                                format!("{} ", item.command)
                            } else {
                                item.command.clone()
                            };
                            cursor = buffer.chars().count();
                            selected = 0;
                        }
                    }
                }
                (KeyCode::Up, _) => {
                    if popup.is_some() {
                        selected = selected.saturating_sub(1);
                    } else if hist_pos > 0 {
                        if hist_pos == self.history.len() {
                            draft = buffer.clone();
                        }
                        hist_pos -= 1;
                        buffer = self.history[hist_pos].clone();
                        cursor = buffer.chars().count();
                    }
                }
                (KeyCode::Down, _) => {
                    if let Some(items) = &popup {
                        if selected + 1 < items.len() {
                            selected += 1;
                        }
                    } else if hist_pos < self.history.len() {
                        hist_pos += 1;
                        buffer = if hist_pos == self.history.len() {
                            draft.clone()
                        } else {
                            self.history[hist_pos].clone()
                        };
                        cursor = buffer.chars().count();
                    }
                }
                (KeyCode::Left, _) => cursor = cursor.saturating_sub(1),
                (KeyCode::Right, _) => cursor = (cursor + 1).min(char_count),
                (KeyCode::Home, _) => cursor = 0,
                (KeyCode::End, _) => cursor = char_count,
                (KeyCode::Esc, _) => dismissed = true,
                (KeyCode::Backspace, _) => {
                    if cursor > 0 {
                        remove_char(&mut buffer, cursor - 1);
                        cursor -= 1;
                        selected = 0;
                        dismissed = false;
                    }
                }
                (KeyCode::Delete, _) => {
                    if cursor < char_count {
                        remove_char(&mut buffer, cursor);
                        selected = 0;
                        dismissed = false;
                    }
                }
                (KeyCode::Char(ch), modifiers)
                    if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
                {
                    insert_char(&mut buffer, &mut cursor, ch);
                    selected = 0;
                    dismissed = false;
                }
                _ => {}
            }
        }
    }

    fn render(
        &self,
        buffer: &str,
        cursor: usize,
        popup: Option<&[PopupItem]>,
        selected: usize,
    ) -> io::Result<()> {
        let mut out = io::stdout();
        queue!(out, MoveToColumn(0), Clear(ClearType::FromCursorDown))?;
        let display = format!("{}{}", self.prompt, buffer).replace('\n', "\r\n");
        write!(out, "{display}")?;

        let mut popup_rows = 0u16;
        if let Some(items) = popup {
            for (index, item) in items.iter().enumerate() {
                write!(out, "\r\n")?;
                let arg = if item.needs_arg { " …" } else { "" };
                if self.color {
                    if index == selected {
                        write!(
                            out,
                            "  \x1b[7m \x1b[0m \x1b[1;36m{:<14}\x1b[0m\x1b[2m{arg}  {}\x1b[0m",
                            item.command, item.description
                        )?;
                    } else {
                        write!(
                            out,
                            "    \x1b[36m{:<14}\x1b[0m\x1b[2m{arg}  {}\x1b[0m",
                            item.command, item.description
                        )?;
                    }
                } else {
                    let marker = if index == selected { '>' } else { ' ' };
                    write!(
                        out,
                        "  {marker} {:<14}{arg}  {}",
                        item.command, item.description
                    )?;
                }
                popup_rows += 1;
            }
        }

        // Reposition the cursor onto the input line at the editing position.
        let input_rows = buffer.matches('\n').count() as u16;
        let cursor_row = buffer.chars().take(cursor).filter(|ch| *ch == '\n').count() as u16;
        let up = (input_rows + popup_rows).saturating_sub(cursor_row);
        if up > 0 {
            queue!(out, MoveToPreviousLine(up))?;
        } else {
            queue!(out, MoveToColumn(0))?;
        }
        let byte_cursor = byte_index(buffer, cursor);
        let line_start = buffer[..byte_cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let col_in_line = buffer[line_start..byte_cursor].chars().count();
        let col = if cursor_row == 0 {
            self.prompt_width + col_in_line
        } else {
            col_in_line
        };
        queue!(out, MoveToColumn(col as u16))?;
        out.flush()
    }

    fn read_line_fallback(&self) -> io::Result<ReadOutcome> {
        let mut stdout = io::stdout();
        write!(stdout, "{}", self.prompt)?;
        stdout.flush()?;

        let mut buffer = String::new();
        let bytes_read = io::stdin().read_line(&mut buffer)?;
        if bytes_read == 0 {
            return Ok(ReadOutcome::Exit);
        }
        while matches!(buffer.chars().last(), Some('\n' | '\r')) {
            buffer.pop();
        }
        Ok(ReadOutcome::Submit(buffer))
    }
}

fn byte_index(buffer: &str, char_index: usize) -> usize {
    buffer
        .char_indices()
        .nth(char_index)
        .map_or(buffer.len(), |(byte, _)| byte)
}

fn insert_char(buffer: &mut String, cursor: &mut usize, ch: char) {
    let byte = byte_index(buffer, *cursor);
    buffer.insert(byte, ch);
    *cursor += 1;
}

fn remove_char(buffer: &mut String, char_index: usize) {
    let byte = byte_index(buffer, char_index);
    buffer.remove(byte);
}

fn starts_with_ci(candidate: &str, query: &str) -> bool {
    candidate
        .to_ascii_lowercase()
        .starts_with(&query.to_ascii_lowercase())
}

/// Curated, high-to-low priority order for the empty (`/`) popup — the commands
/// most users reach for first. Anything not listed ranks below these.
const CURATED_POPUP_ORDER: &[&str] = &[
    "help",
    "model",
    "agent",
    "memory",
    "status",
    "diff",
    "compact",
    "resume",
    "opencode",
    "permissions",
    "theme",
    "clear",
];

/// Answer-style tokens that should never lead the empty popup (they're replies,
/// not commands a user opens the menu to find).
const POPUP_ANSWER_TOKENS: &[&str] = &["y", "n", "yes", "no"];

/// Rank a candidate for the empty (`/`) popup by context and usefulness:
/// curated priorities first, then a small boost for recently-used commands,
/// with answer-style tokens demoted out of the initial suggestions. Slice 11.
fn empty_filter_rank(candidate: &str, history: &[String]) -> i64 {
    let base = candidate
        .trim_start_matches('/')
        .split_whitespace()
        .next()
        .unwrap_or("");
    if POPUP_ANSWER_TOKENS.contains(&base) {
        return -5000;
    }
    let curated = CURATED_POPUP_ORDER
        .iter()
        .position(|name| *name == base)
        .map_or(0, |index| 1000 - index as i64);
    // Recency: the most recent matching submission lifts the rank a little, so
    // commands you actually use surface above the rest (but below curated).
    let recency = history
        .iter()
        .rev()
        .take(50)
        .position(|entry| entry.trim_start_matches('/').split_whitespace().next() == Some(base))
        .map_or(0, |pos| (30 - pos as i64).max(0));
    curated + recency
}

/// Fuzzy score: a case-insensitive prefix wins (shorter candidates rank higher),
/// then a subsequence match, else `None`.
fn fuzzy_score(candidate: &str, query: &str) -> Option<i64> {
    let cand = candidate.to_ascii_lowercase();
    let query = query.to_ascii_lowercase();
    if cand.starts_with(&query) {
        return Some(2000 - candidate.chars().count() as i64);
    }
    let mut chars = cand.chars().enumerate();
    let mut first = None;
    for needle in query.chars() {
        match chars.by_ref().find(|(_, ch)| *ch == needle) {
            Some((index, _)) => {
                first.get_or_insert(index);
            }
            None => return None,
        }
    }
    Some(1000 - first.unwrap_or(0) as i64)
}

/// The one-line summary for a completion candidate (by its base command name).
fn describe_completion(candidate: &str) -> Option<&'static str> {
    let base = candidate
        .trim_start_matches('/')
        .split_whitespace()
        .next()?;
    // REPL-only session controls aren't in the shared spec list.
    if matches!(base, "exit" | "quit") {
        return Some("Save the session and leave the REPL");
    }
    commands::slash_command_specs()
        .iter()
        .find(|spec| spec.name == base || spec.aliases.contains(&base))
        .map(|spec| spec.summary)
}

/// Whether a slash command takes an argument (so completion should insert it
/// plus a trailing space rather than run it).
fn command_needs_arg(candidate: &str) -> bool {
    let Some(base) = candidate.trim_start_matches('/').split_whitespace().next() else {
        return false;
    };
    commands::slash_command_specs()
        .iter()
        .find(|spec| spec.name == base || spec.aliases.contains(&base))
        .is_some_and(|spec| spec.argument_hint.is_some())
}

fn normalize_completions(completions: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    completions
        .into_iter()
        .filter(|candidate| candidate.starts_with('/'))
        .filter(|candidate| seen.insert(candidate.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        byte_index, command_needs_arg, describe_completion, fuzzy_score, insert_char,
        normalize_completions, remove_char, starts_with_ci, LineEditor,
    };

    fn popup_commands(editor: &LineEditor, buffer: &str) -> Vec<String> {
        editor
            .build_popup(buffer)
            .unwrap_or_default()
            .into_iter()
            .map(|item| item.command)
            .collect()
    }

    #[test]
    fn popup_opens_for_bare_slash_and_filters_fuzzily() {
        let editor = LineEditor::new(
            "> ",
            vec![
                "/model".to_string(),
                "/models".to_string(),
                "/status".to_string(),
                "/model opus".to_string(), // argument candidate is excluded from the menu
            ],
        );
        // `/` shows commands; `/mdl` fuzzy-matches model/models, not status.
        assert!(popup_commands(&editor, "/").contains(&"/status".to_string()));
        let filtered = popup_commands(&editor, "/mdl");
        assert!(filtered.contains(&"/model".to_string()));
        assert!(filtered.contains(&"/models".to_string()));
        assert!(!filtered.contains(&"/status".to_string()));
        // Argument candidates never appear as command-menu rows.
        assert!(!filtered.contains(&"/model opus".to_string()));
    }

    #[test]
    fn empty_popup_ranks_by_usefulness_not_length() {
        let editor = LineEditor::new(
            "> ",
            vec![
                "/n".to_string(),
                "/y".to_string(),
                "/model".to_string(),
                "/help".to_string(),
                "/agent".to_string(),
                "/memory".to_string(),
                "/status".to_string(),
                "/zzz".to_string(),
            ],
        );
        let menu = popup_commands(&editor, "/");
        // Curated commands lead; short answer tokens are demoted out of the top.
        assert_eq!(&menu[0], "/help");
        assert!(menu.iter().position(|c| c == "/model").unwrap() < 3);
        assert!(
            !menu.contains(&"/n".to_string()),
            "answer tokens must not lead: {menu:?}"
        );
        assert!(!menu.contains(&"/y".to_string()));

        // Recency lifts a non-curated command above other non-curated peers.
        let mut recent = LineEditor::new(
            "> ",
            vec!["/zzz".to_string(), "/aaa".to_string(), "/bbb".to_string()],
        );
        recent.push_history("/zzz");
        let menu = popup_commands(&recent, "/");
        assert_eq!(
            &menu[0], "/zzz",
            "recently used should surface first: {menu:?}"
        );

        // Typing a filter still uses fuzzy matching: `/n` leads with the exact
        // prefix match (answer tokens are only demoted in the empty view).
        let typed = popup_commands(&editor, "/n");
        assert_eq!(typed.first().map(String::as_str), Some("/n"));
    }

    #[test]
    fn popup_closes_once_typing_arguments() {
        let editor = LineEditor::new("> ", vec!["/model".to_string()]);
        assert!(editor.build_popup("/model ").is_none());
        assert!(editor.build_popup("hello").is_none());
        assert!(editor.build_popup("/multi\nline").is_none());
    }

    #[test]
    fn fuzzy_prefix_outranks_subsequence() {
        // `/help` (prefix) scores above a subsequence-only match.
        assert!(fuzzy_score("/help", "/he").unwrap() > fuzzy_score("/sphere", "/he").unwrap_or(0));
        assert!(fuzzy_score("/status", "/xyz").is_none());
        assert!(starts_with_ci("/Model", "/mod"));
    }

    #[test]
    fn colored_prompt_tracks_visible_width_not_byte_count() {
        // A themed "❯ " prompt has visible width 2 even with ANSI wrapping, so
        // the row-0 cursor column stays correct. Slice 14a.
        let mut editor = LineEditor::new("\x1b[1;36m❯\x1b[0m ", vec![]);
        assert_eq!(editor.prompt_width, 2);
        editor.set_prompt("> ");
        assert_eq!(editor.prompt_width, 2);
    }

    #[test]
    fn command_needs_arg_reads_the_spec_argument_hint() {
        // `/model` has argument_hint `[model]`; `/help` has none.
        assert!(command_needs_arg("/model"));
        assert!(!command_needs_arg("/help"));
    }

    #[test]
    fn exit_and_quit_appear_in_the_popup_with_descriptions() {
        // REPL-only controls aren't specs, but are offered as completions with
        // a description and take no argument.
        let editor = LineEditor::new("> ", vec!["/exit".to_string(), "/quit".to_string()]);
        let menu = popup_commands(&editor, "/");
        assert!(menu.contains(&"/exit".to_string()));
        assert!(menu.contains(&"/quit".to_string()));
        assert!(describe_completion("/exit").is_some());
        assert!(describe_completion("/quit").is_some());
        assert!(!command_needs_arg("/exit"));
    }

    #[test]
    fn buffer_edits_are_char_safe() {
        let mut buffer = String::new();
        let mut cursor = 0;
        for ch in "/théme".chars() {
            insert_char(&mut buffer, &mut cursor, ch);
        }
        assert_eq!(buffer, "/théme");
        assert_eq!(cursor, 6);
        remove_char(&mut buffer, 2); // remove 'h'
        assert_eq!(buffer, "/téme");
        assert_eq!(byte_index("/téme", 5), "/téme".len());
    }

    #[test]
    fn history_dedupes_and_skips_blanks() {
        let mut editor = LineEditor::new("> ", Vec::new());
        editor.push_history("   ");
        editor.push_history("/help");
        editor.push_history("/help");
        editor.push_history("/status");
        assert_eq!(
            editor.history,
            vec!["/help".to_string(), "/status".to_string()]
        );
    }

    #[test]
    fn normalize_completions_keeps_slash_only_and_dedupes() {
        let normalized = normalize_completions(vec![
            "/model".to_string(),
            "/model".to_string(),
            "status".to_string(),
        ]);
        assert_eq!(normalized, vec!["/model".to_string()]);
    }
}
