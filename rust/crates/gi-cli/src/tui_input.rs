//! Editable input state for the full-screen mode — the in-box editing model
//! (buffer, cursor, history, slash/`@` popups, completion) without owning the
//! terminal. The full-screen loop drives it with `on_key` and renders it via
//! `tui::draw`, reusing the same popup/fuzzy/mention helpers as the line REPL so
//! the two inputs behave identically. Slice: unified full-screen mode (Phase 2).

use crossterm::event::{KeyCode, KeyModifiers};

use crate::input::{
    at_mention_prefix, command_popup, insert_char, mention_popup, remove_char, splice_token,
    PopupItem,
};

/// Result of feeding a key to [`TuiInput`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InputAction {
    /// Nothing for the caller to do (the input updated itself).
    None,
    /// The user submitted this text — run it as a turn / command.
    Submit(String),
}

/// The full-screen input editing model.
#[derive(Default)]
pub(crate) struct TuiInput {
    buffer: String,
    /// Cursor as a char index into `buffer`.
    cursor: usize,
    completions: Vec<String>,
    mentions: Vec<String>,
    history: Vec<String>,
    /// Position in `history` while browsing with Up/Down; `== history.len()`
    /// means "the live draft".
    hist_pos: usize,
    draft: String,
    /// Highlighted popup row.
    selected: usize,
    /// Esc dismissed the popup until the buffer changes again.
    dismissed: bool,
}

impl TuiInput {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn buffer(&self) -> &str {
        &self.buffer
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(crate) fn set_completions(&mut self, completions: Vec<String>) {
        self.completions = completions;
    }

    pub(crate) fn set_mentions(&mut self, mentions: Vec<String>) {
        self.mentions = mentions;
    }

    pub(crate) fn set_history(&mut self, history: Vec<String>) {
        self.history = history;
        self.hist_pos = self.history.len();
    }

    /// Append a submitted line to history (deduping consecutive repeats).
    pub(crate) fn push_history(&mut self, entry: &str) {
        if entry.trim().is_empty() {
            return;
        }
        if self.history.last().map(String::as_str) != Some(entry) {
            self.history.push(entry.to_string());
        }
        self.hist_pos = self.history.len();
    }

    pub(crate) fn has_popup(&self) -> bool {
        self.popup().0.is_some()
    }

    /// Dismiss the popup (Esc) until the buffer changes.
    pub(crate) fn dismiss_popup(&mut self) {
        self.dismissed = true;
    }

    /// Current popup items plus, for an `@`-mention popup, the char index where
    /// the `@` token starts (so completion replaces just that token).
    fn popup(&self) -> (Option<Vec<PopupItem>>, Option<usize>) {
        if self.dismissed {
            return (None, None);
        }
        if let Some((start, prefix)) = at_mention_prefix(&self.buffer, self.cursor) {
            let items = mention_popup(&prefix, &self.mentions);
            if items.is_some() {
                return (items, Some(start));
            }
        }
        (
            command_popup(&self.buffer, &self.completions, &self.history),
            None,
        )
    }

    /// Popup rows (label, description) + the selected index, for rendering.
    pub(crate) fn popup_view(&self) -> Option<(Vec<(String, String)>, usize)> {
        let (items, _) = self.popup();
        let items = items?;
        let selected = self.selected.min(items.len().saturating_sub(1));
        Some((
            items
                .into_iter()
                .map(|item| (item.command, item.description))
                .collect(),
            selected,
        ))
    }

    fn char_count(&self) -> usize {
        self.buffer.chars().count()
    }

    /// Feed one key; returns [`InputAction::Submit`] when the user sends a line.
    pub(crate) fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) -> InputAction {
        let (popup, mention_start) = self.popup();
        match (code, mods) {
            // Newline gestures (Shift/Alt+Enter, Ctrl+J).
            (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                insert_char(&mut self.buffer, &mut self.cursor, '\n');
                self.after_edit();
            }
            (KeyCode::Enter, m)
                if m.contains(KeyModifiers::SHIFT) || m.contains(KeyModifiers::ALT) =>
            {
                insert_char(&mut self.buffer, &mut self.cursor, '\n');
                self.after_edit();
            }
            (KeyCode::Enter, _) => {
                if let Some(items) = &popup {
                    if let Some(item) = items.get(self.selected.min(items.len().saturating_sub(1)))
                    {
                        if let Some(start) = mention_start {
                            let trailing = if item.needs_arg { "" } else { " " };
                            splice_token(
                                &mut self.buffer,
                                &mut self.cursor,
                                start,
                                &format!("{}{trailing}", item.command),
                            );
                            self.after_edit();
                            return InputAction::None;
                        }
                        if item.needs_arg {
                            self.buffer = format!("{} ", item.command);
                            self.cursor = self.char_count();
                            self.after_edit();
                            return InputAction::None;
                        }
                        let command = item.command.clone();
                        self.reset();
                        return InputAction::Submit(command);
                    }
                }
                let text = std::mem::take(&mut self.buffer);
                self.reset();
                return InputAction::Submit(text);
            }
            (KeyCode::Tab, _) => {
                if let Some(items) = &popup {
                    if let Some(item) = items.get(self.selected.min(items.len().saturating_sub(1)))
                    {
                        if let Some(start) = mention_start {
                            let trailing = if item.needs_arg { "" } else { " " };
                            splice_token(
                                &mut self.buffer,
                                &mut self.cursor,
                                start,
                                &format!("{}{trailing}", item.command),
                            );
                        } else {
                            self.buffer = if item.needs_arg {
                                format!("{} ", item.command)
                            } else {
                                item.command.clone()
                            };
                            self.cursor = self.char_count();
                        }
                        self.selected = 0;
                    }
                }
            }
            (KeyCode::Up, _) => {
                if popup.is_some() {
                    self.selected = self.selected.saturating_sub(1);
                } else {
                    self.history_prev();
                }
            }
            (KeyCode::Down, _) => {
                if let Some(items) = &popup {
                    if self.selected + 1 < items.len() {
                        self.selected += 1;
                    }
                } else {
                    self.history_next();
                }
            }
            (KeyCode::Left, _) => self.cursor = self.cursor.saturating_sub(1),
            (KeyCode::Right, _) => self.cursor = (self.cursor + 1).min(self.char_count()),
            (KeyCode::Home, _) => self.cursor = 0,
            (KeyCode::End, _) => self.cursor = self.char_count(),
            (KeyCode::Backspace, _) => {
                if self.cursor > 0 {
                    remove_char(&mut self.buffer, self.cursor - 1);
                    self.cursor -= 1;
                    self.after_edit();
                }
            }
            (KeyCode::Delete, _) => {
                if self.cursor < self.char_count() {
                    remove_char(&mut self.buffer, self.cursor);
                    self.after_edit();
                }
            }
            (KeyCode::Char(ch), m) if m.is_empty() || m == KeyModifiers::SHIFT => {
                insert_char(&mut self.buffer, &mut self.cursor, ch);
                self.after_edit();
            }
            _ => {}
        }
        InputAction::None
    }

    /// After a text edit: reset the popup highlight + un-dismiss.
    fn after_edit(&mut self) {
        self.selected = 0;
        self.dismissed = false;
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
        self.selected = 0;
        self.dismissed = false;
        self.hist_pos = self.history.len();
    }

    fn history_prev(&mut self) {
        if self.hist_pos == 0 {
            return;
        }
        if self.hist_pos == self.history.len() {
            self.draft = self.buffer.clone();
        }
        self.hist_pos -= 1;
        self.buffer = self.history[self.hist_pos].clone();
        self.cursor = self.char_count();
    }

    fn history_next(&mut self) {
        if self.hist_pos >= self.history.len() {
            return;
        }
        self.hist_pos += 1;
        self.buffer = if self.hist_pos == self.history.len() {
            self.draft.clone()
        } else {
            self.history[self.hist_pos].clone()
        };
        self.cursor = self.char_count();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> TuiInput {
        let mut input = TuiInput::new();
        for ch in text.chars() {
            input.on_key(KeyCode::Char(ch), KeyModifiers::NONE);
        }
        input
    }

    #[test]
    fn typing_and_enter_submits() {
        let mut input = typed("hello");
        assert_eq!(input.buffer(), "hello");
        assert_eq!(
            input.on_key(KeyCode::Enter, KeyModifiers::NONE),
            InputAction::Submit("hello".to_string())
        );
        assert_eq!(input.buffer(), "");
    }

    #[test]
    fn alt_enter_inserts_newline_not_submit() {
        let mut input = typed("a");
        assert_eq!(
            input.on_key(KeyCode::Enter, KeyModifiers::ALT),
            InputAction::None
        );
        assert_eq!(input.buffer(), "a\n");
    }

    #[test]
    fn slash_popup_and_enter_runs_highlighted_command() {
        let mut input = TuiInput::new();
        input.set_completions(vec!["/models".into(), "/memory".into(), "/exit".into()]);
        for ch in "/mo".chars() {
            input.on_key(KeyCode::Char(ch), KeyModifiers::NONE);
        }
        let (rows, _) = input.popup_view().expect("popup should show for /mo");
        assert!(rows.iter().any(|(cmd, _)| cmd == "/models"));
        // Enter runs the top match.
        match input.on_key(KeyCode::Enter, KeyModifiers::NONE) {
            InputAction::Submit(cmd) => assert!(cmd.starts_with('/')),
            other => panic!("expected submit, got {other:?}"),
        }
    }

    #[test]
    fn mention_popup_tab_inserts_path_token() {
        let mut input = TuiInput::new();
        input.set_mentions(vec!["src/main.rs".into(), "src/lib.rs".into()]);
        for ch in "see @src/ma".chars() {
            input.on_key(KeyCode::Char(ch), KeyModifiers::NONE);
        }
        assert!(input.has_popup());
        input.on_key(KeyCode::Tab, KeyModifiers::NONE);
        assert!(input.buffer().contains("@src/main.rs"));
        assert!(input.buffer().starts_with("see "));
    }

    #[test]
    fn up_down_browse_history() {
        let mut input = TuiInput::new();
        input.set_history(vec!["first".into(), "second".into()]);
        input.on_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(input.buffer(), "second");
        input.on_key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(input.buffer(), "first");
        input.on_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(input.buffer(), "second");
    }

    #[test]
    fn cursor_keys_and_backspace_edit_mid_buffer() {
        let mut input = typed("abc");
        input.on_key(KeyCode::Left, KeyModifiers::NONE); // cursor between b|c
        input.on_key(KeyCode::Backspace, KeyModifiers::NONE); // remove b
        assert_eq!(input.buffer(), "ac");
        assert_eq!(input.cursor(), 1);
    }
}
