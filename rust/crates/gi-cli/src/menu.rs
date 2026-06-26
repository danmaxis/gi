//! Interactive selection modal for list-style slash commands (`/agents`,
//! `/models`, `/plugins`, `/sessions`, `/memory`). Replaces the "print a list,
//! then type a follow-up command" UX with a navigable, in-place box:
//!
//! - `↑`/`↓` move the highlight (the list scrolls when it's taller than the box).
//! - `Enter` performs the item's default action.
//! - `→` opens a sub-menu of secondary actions; `←` / `Esc` backs out.
//! - Letter shortcuts trigger an action directly (plain key in a non-filterable
//!   menu, `Alt`+key while a filterable menu is capturing filter text).
//! - Destructive actions route through an inline `y/N` confirm.
//! - `Esc` backs out one level (sub-menu → list → cancel); `Ctrl+C` cancels.
//! - Filterable menus narrow as you type (same fuzzy ranking as the `/` popup).
//!
//! The logic lives in [`MenuState`] (a pure state machine driven by `KeyCode`s,
//! unit-tested without a terminal); [`run_menu`] is the thin raw-mode IO driver.

use std::io::{self, IsTerminal, Write};

use crossterm::cursor::{Hide, MoveToColumn, MoveToPreviousLine, Show};
use crossterm::event::{read, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::queue;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};

use crate::input::fuzzy_score;
use crate::render;

/// A secondary action offered in an item's `→` sub-menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuAction {
    pub name: String,
    /// One-letter shortcut (e.g. `p` to pin, `d` to delete).
    pub key: Option<char>,
    /// Route through a `y/N` confirm before emitting the outcome.
    pub destructive: bool,
}

impl MenuAction {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            key: None,
            destructive: false,
        }
    }

    #[must_use]
    pub fn key(mut self, key: char) -> Self {
        self.key = Some(key);
        self
    }

    #[must_use]
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
}

/// One selectable row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// Primary text (agent name, `provider · model`, note text).
    pub label: String,
    /// Dim secondary text (model, `12 msgs · 3m`, tags).
    pub detail: Option<String>,
    /// Small status glyph, e.g. `📌` pinned / `●` enabled.
    pub badge: Option<String>,
    /// Action performed on `Enter` from the list.
    pub default_action: String,
    /// Secondary actions opened with `→`.
    pub actions: Vec<MenuAction>,
}

impl MenuItem {
    pub fn new(label: impl Into<String>, default_action: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            detail: None,
            badge: None,
            default_action: default_action.into(),
            actions: Vec::new(),
        }
    }

    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    #[must_use]
    pub fn badge(mut self, badge: impl Into<String>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    #[must_use]
    pub fn actions(mut self, actions: Vec<MenuAction>) -> Self {
        self.actions = actions;
        self
    }
}

/// What the modal returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuOutcome {
    Selected { item_index: usize, action: String },
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    List,
    SubMenu,
    Confirm,
}

/// The pure, terminal-free state machine. Feed it keys with [`MenuState::on_key`]
/// and render it with [`MenuState::render_lines`].
pub struct MenuState {
    items: Vec<MenuItem>,
    /// Position within the *filtered* (visible) set.
    selected: usize,
    focus: Focus,
    sub_selected: usize,
    /// First visible row (index into the filtered set) — for scrolling.
    scroll: usize,
    viewport: usize,
    filterable: bool,
    filter: String,
    empty_hint: String,
    /// The destructive action awaiting `y/N` while in [`Focus::Confirm`].
    pending_action: Option<String>,
}

impl MenuState {
    #[must_use]
    pub fn new(items: Vec<MenuItem>, viewport: usize, filterable: bool) -> Self {
        Self {
            items,
            selected: 0,
            focus: Focus::List,
            sub_selected: 0,
            scroll: 0,
            viewport: viewport.max(1),
            filterable,
            filter: String::new(),
            empty_hint: "nothing here yet".to_string(),
            pending_action: None,
        }
    }

    #[must_use]
    pub fn empty_hint(mut self, hint: impl Into<String>) -> Self {
        self.empty_hint = hint.into();
        self
    }

    /// Absolute indices of items matching the current filter, best-ranked first.
    fn visible(&self) -> Vec<usize> {
        if !self.filterable || self.filter.is_empty() {
            return (0..self.items.len()).collect();
        }
        let mut scored: Vec<(i64, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let haystack = match &item.detail {
                    Some(detail) => format!("{} {}", item.label, detail),
                    None => item.label.clone(),
                };
                fuzzy_score(&haystack, &self.filter).map(|score| (score, index))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, index)| index).collect()
    }

    /// Feed a keypress. Returns `Some(outcome)` when the modal should close.
    pub fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) -> Option<MenuOutcome> {
        // Ctrl+C always cancels outright.
        if matches!(code, KeyCode::Char('c')) && mods.contains(KeyModifiers::CONTROL) {
            return Some(MenuOutcome::Cancelled);
        }
        // Esc backs out one level, bottoming out at Cancelled.
        if matches!(code, KeyCode::Esc) {
            return match self.focus {
                Focus::Confirm => {
                    self.pending_action = None;
                    self.focus = Focus::SubMenu;
                    None
                }
                Focus::SubMenu => {
                    self.focus = Focus::List;
                    None
                }
                Focus::List => Some(MenuOutcome::Cancelled),
            };
        }
        match self.focus {
            Focus::List => self.on_key_list(code, mods),
            Focus::SubMenu => self.on_key_submenu(code),
            Focus::Confirm => self.on_key_confirm(code),
        }
    }

    fn on_key_list(&mut self, code: KeyCode, mods: KeyModifiers) -> Option<MenuOutcome> {
        let visible = self.visible();
        match code {
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                self.adjust_scroll(visible.len());
            }
            KeyCode::Down => {
                if self.selected + 1 < visible.len() {
                    self.selected += 1;
                    self.adjust_scroll(visible.len());
                }
            }
            KeyCode::Right => {
                if let Some(&abs) = visible.get(self.selected) {
                    if !self.items[abs].actions.is_empty() {
                        self.focus = Focus::SubMenu;
                        self.sub_selected = 0;
                    }
                }
            }
            KeyCode::Enter => {
                if let Some(&abs) = visible.get(self.selected) {
                    let action = self.items[abs].default_action.clone();
                    return Some(MenuOutcome::Selected {
                        item_index: abs,
                        action,
                    });
                }
            }
            KeyCode::Backspace if self.filterable => {
                self.filter.pop();
                self.reclamp();
            }
            KeyCode::Char(ch) => {
                let alt = mods.contains(KeyModifiers::ALT);
                let ctrl = mods.contains(KeyModifiers::CONTROL);
                // Quick-action key: a plain key in a non-filterable menu, or
                // Alt+key while a filterable menu is capturing filter text.
                let quick_ok = if self.filterable { alt } else { !alt && !ctrl };
                if quick_ok {
                    if let Some(&abs) = visible.get(self.selected) {
                        if let Some(action) =
                            self.items[abs].actions.iter().find(|a| a.key == Some(ch))
                        {
                            let action = action.clone();
                            return self.trigger(abs, &action);
                        }
                    }
                }
                if self.filterable && !alt && !ctrl {
                    self.filter.push(ch);
                    self.reclamp();
                }
            }
            _ => {}
        }
        None
    }

    fn on_key_submenu(&mut self, code: KeyCode) -> Option<MenuOutcome> {
        let visible = self.visible();
        let Some(&abs) = visible.get(self.selected) else {
            self.focus = Focus::List;
            return None;
        };
        let actions = self.items[abs].actions.clone();
        match code {
            KeyCode::Up => self.sub_selected = self.sub_selected.saturating_sub(1),
            KeyCode::Down => {
                if self.sub_selected + 1 < actions.len() {
                    self.sub_selected += 1;
                }
            }
            KeyCode::Left => self.focus = Focus::List,
            KeyCode::Enter => {
                if let Some(action) = actions.get(self.sub_selected) {
                    return self.trigger(abs, action);
                }
            }
            KeyCode::Char(ch) => {
                if let Some(action) = actions.iter().find(|a| a.key == Some(ch)) {
                    let action = action.clone();
                    return self.trigger(abs, &action);
                }
            }
            _ => {}
        }
        None
    }

    fn on_key_confirm(&mut self, code: KeyCode) -> Option<MenuOutcome> {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let visible = self.visible();
                if let (Some(&abs), Some(action)) =
                    (visible.get(self.selected), self.pending_action.take())
                {
                    return Some(MenuOutcome::Selected {
                        item_index: abs,
                        action,
                    });
                }
                self.focus = Focus::List;
                None
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Left => {
                self.pending_action = None;
                self.focus = Focus::SubMenu;
                None
            }
            _ => None,
        }
    }

    /// Either emit the outcome, or (destructive) drop into the confirm step.
    fn trigger(&mut self, abs: usize, action: &MenuAction) -> Option<MenuOutcome> {
        if action.destructive {
            self.pending_action = Some(action.name.clone());
            self.focus = Focus::Confirm;
            None
        } else {
            Some(MenuOutcome::Selected {
                item_index: abs,
                action: action.name.clone(),
            })
        }
    }

    fn adjust_scroll(&mut self, visible_len: usize) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + self.viewport {
            self.scroll = self.selected + 1 - self.viewport;
        }
        let max_scroll = visible_len.saturating_sub(self.viewport);
        self.scroll = self.scroll.min(max_scroll);
    }

    fn reclamp(&mut self) {
        let visible_len = self.visible().len();
        if self.selected >= visible_len {
            self.selected = visible_len.saturating_sub(1);
        }
        self.adjust_scroll(visible_len);
    }

    /// Body lines for the panel (no border) at the current state. Consumed by the
    /// IO driver and by snapshot tests.
    pub fn render_lines(&self, use_color: bool) -> Vec<String> {
        let (dim, reset, rev) = if use_color {
            ("\x1b[2m", "\x1b[0m", "\x1b[7m")
        } else {
            ("", "", "")
        };
        let visible = self.visible();
        match self.focus {
            Focus::SubMenu | Focus::Confirm => {
                self.render_actions(visible, dim, reset, rev, use_color)
            }
            Focus::List => self.render_list(visible, dim, reset, rev),
        }
    }

    fn render_list(&self, visible: Vec<usize>, dim: &str, reset: &str, rev: &str) -> Vec<String> {
        let mut lines = Vec::new();
        if self.filterable {
            lines.push(format!("{dim}filter: {}▏{reset}", self.filter));
        }
        if visible.is_empty() {
            lines.push(format!("{dim}(none yet — {}){reset}", self.empty_hint));
            return lines;
        }
        let end = (self.scroll + self.viewport).min(visible.len());
        for (offset, &abs) in visible[self.scroll..end].iter().enumerate() {
            let row = self.scroll + offset;
            let item = &self.items[abs];
            let badge = item
                .badge
                .as_deref()
                .map(|b| format!("{b} "))
                .unwrap_or_default();
            let detail = item
                .detail
                .as_deref()
                .map(|d| format!("  {dim}{d}{reset}"))
                .unwrap_or_default();
            let text = truncate_cols(&format!("{badge}{}", item.label), 60);
            if row == self.selected {
                lines.push(format!("{rev}❯ {text}{reset}{detail}"));
            } else {
                lines.push(format!("  {text}{detail}"));
            }
        }
        if visible.len() > self.viewport {
            lines.push(format!(
                "{dim}  {}/{} · ↑↓ move · ↵ select · → actions{reset}",
                self.selected + 1,
                visible.len()
            ));
        } else {
            lines.push(format!(
                "{dim}  ↑↓ move · ↵ select · → actions · esc cancel{reset}"
            ));
        }
        lines
    }

    fn render_actions(
        &self,
        visible: Vec<usize>,
        dim: &str,
        reset: &str,
        rev: &str,
        _use_color: bool,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let Some(&abs) = visible.get(self.selected) else {
            return lines;
        };
        let item = &self.items[abs];
        lines.push(format!("{dim}{}{reset}", truncate_cols(&item.label, 60)));
        if self.focus == Focus::Confirm {
            let action = self.pending_action.as_deref().unwrap_or("");
            lines.push(String::new());
            lines.push(format!("{rev} {action} this? {reset}  [y] yes   [n] no"));
            return lines;
        }
        for (index, action) in item.actions.iter().enumerate() {
            let key = action
                .key
                .map(|k| format!(" {dim}({k}){reset}"))
                .unwrap_or_default();
            let warn = if action.destructive {
                format!(" {dim}⚠{reset}")
            } else {
                String::new()
            };
            if index == self.sub_selected {
                lines.push(format!("{rev}❯ {}{reset}{key}{warn}", action.name));
            } else {
                lines.push(format!("  {}{key}{warn}", action.name));
            }
        }
        lines.push(format!("{dim}  ↑↓ · ↵ do · ← back{reset}"));
        lines
    }
}

/// Truncate `text` to at most `max` display columns, appending `…` when cut.
fn truncate_cols(text: &str, max: usize) -> String {
    if render::display_width(text) <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut width = 0;
    for ch in text.chars() {
        let w = render::display_width(&ch.to_string());
        if width + w > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        width += w;
    }
    out.push('…');
    out
}

/// Largest content height the modal will use (it scrolls beyond this).
const MENU_MAX_ROWS: usize = 12;

/// Run the modal in raw mode and return the chosen `(item, action)` or
/// `Cancelled`. Caller must ensure stdin/stdout are a TTY. The modal block is
/// cleared on exit so following output flows normally.
pub fn run_menu(
    title: &str,
    items: Vec<MenuItem>,
    filterable: bool,
    use_color: bool,
    empty_hint: &str,
) -> io::Result<MenuOutcome> {
    let rows = crossterm::terminal::size()
        .map(|(_, h)| h as usize)
        .unwrap_or(24);
    let viewport = MENU_MAX_ROWS.min(rows.saturating_sub(6).max(3));
    let mut state = MenuState::new(items, viewport, filterable).empty_hint(empty_hint);

    enable_raw_mode()?;
    let mut out = io::stdout();
    let _ = queue!(out, Hide);
    let _ = out.flush();

    let mut last_rows: u16 = 0;
    let outcome = loop {
        // Redraw in place: clear the previous block, then print the panel.
        let body = state.render_lines(use_color).join("\n");
        let panel = render::panel(Some(title), &body, use_color);
        let row_count = panel.split('\n').count() as u16;
        if last_rows > 0 {
            let _ = queue!(
                out,
                MoveToPreviousLine(last_rows),
                Clear(ClearType::FromCursorDown)
            );
        } else {
            let _ = queue!(out, MoveToColumn(0));
        }
        for line in panel.split('\n') {
            let _ = write!(out, "{line}\r\n");
        }
        let _ = out.flush();
        last_rows = row_count;

        match read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if let Some(outcome) = state.on_key(key.code, key.modifiers) {
                    break outcome;
                }
            }
            Event::Resize(..) => { /* loop re-renders at the new width */ }
            _ => {}
        }
    };

    // Erase the modal so the caller's output starts clean.
    if last_rows > 0 {
        let _ = queue!(
            out,
            MoveToPreviousLine(last_rows),
            Clear(ClearType::FromCursorDown)
        );
    }
    let _ = queue!(out, Show);
    let _ = out.flush();
    let _ = disable_raw_mode();
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, default: &str, actions: Vec<MenuAction>) -> MenuItem {
        MenuItem::new(label, default).actions(actions)
    }

    fn list(n: usize) -> Vec<MenuItem> {
        (0..n)
            .map(|i| item(&format!("item{i}"), "use", vec![]))
            .collect()
    }

    fn key(state: &mut MenuState, code: KeyCode) -> Option<MenuOutcome> {
        state.on_key(code, KeyModifiers::NONE)
    }

    #[test]
    fn down_up_clamp_at_both_ends() {
        let mut s = MenuState::new(list(3), 10, false);
        assert!(key(&mut s, KeyCode::Up).is_none());
        assert_eq!(s.selected, 0); // already at top
        key(&mut s, KeyCode::Down);
        key(&mut s, KeyCode::Down);
        key(&mut s, KeyCode::Down); // past the end
        assert_eq!(s.selected, 2); // clamped to last
    }

    #[test]
    fn enter_from_list_emits_default_action() {
        let mut s = MenuState::new(list(3), 10, false);
        key(&mut s, KeyCode::Down);
        let outcome = key(&mut s, KeyCode::Enter).expect("emits");
        assert_eq!(
            outcome,
            MenuOutcome::Selected {
                item_index: 1,
                action: "use".to_string()
            }
        );
    }

    #[test]
    fn right_opens_submenu_only_when_actions_exist() {
        let mut s = MenuState::new(list(2), 10, false);
        key(&mut s, KeyCode::Right);
        assert_eq!(s.focus, Focus::List); // no actions → no-op

        let mut s = MenuState::new(
            vec![item("a", "use", vec![MenuAction::new("rename")])],
            10,
            false,
        );
        key(&mut s, KeyCode::Right);
        assert_eq!(s.focus, Focus::SubMenu);
        key(&mut s, KeyCode::Left);
        assert_eq!(s.focus, Focus::List);
    }

    #[test]
    fn submenu_enter_emits_chosen_action() {
        let mut s = MenuState::new(
            vec![item(
                "a",
                "use",
                vec![MenuAction::new("rename"), MenuAction::new("show")],
            )],
            10,
            false,
        );
        key(&mut s, KeyCode::Right);
        key(&mut s, KeyCode::Down); // highlight "show"
        let outcome = key(&mut s, KeyCode::Enter).expect("emits");
        assert_eq!(
            outcome,
            MenuOutcome::Selected {
                item_index: 0,
                action: "show".to_string()
            }
        );
    }

    #[test]
    fn destructive_action_requires_confirm() {
        let mut s = MenuState::new(
            vec![item(
                "a",
                "use",
                vec![MenuAction::new("delete").destructive()],
            )],
            10,
            false,
        );
        key(&mut s, KeyCode::Right);
        assert!(key(&mut s, KeyCode::Enter).is_none()); // → Confirm, no emit
        assert_eq!(s.focus, Focus::Confirm);
        // 'n' aborts back to the sub-menu.
        key(&mut s, KeyCode::Char('n'));
        assert_eq!(s.focus, Focus::SubMenu);
        // Re-enter and confirm with 'y'.
        key(&mut s, KeyCode::Enter);
        let outcome = key(&mut s, KeyCode::Char('y')).expect("emits");
        assert_eq!(
            outcome,
            MenuOutcome::Selected {
                item_index: 0,
                action: "delete".to_string()
            }
        );
    }

    #[test]
    fn letter_shortcut_triggers_action_in_nonfilterable_menu() {
        let mut s = MenuState::new(
            vec![item("a", "view", vec![MenuAction::new("pin").key('p')])],
            10,
            false,
        );
        let outcome = key(&mut s, KeyCode::Char('p')).expect("emits");
        assert_eq!(outcome.action(), "pin");
    }

    #[test]
    fn plain_chars_filter_in_filterable_menu_alt_is_shortcut() {
        let mut items = vec![
            item("apple", "view", vec![MenuAction::new("pin").key('p')]),
            item("banana", "view", vec![MenuAction::new("pin").key('p')]),
        ];
        items.push(item("apricot", "view", vec![]));
        let mut s = MenuState::new(items, 10, true);
        // Typing narrows to the 'ap*' matches.
        s.on_key(KeyCode::Char('a'), KeyModifiers::NONE);
        s.on_key(KeyCode::Char('p'), KeyModifiers::NONE);
        assert_eq!(s.visible().len(), 2); // apple, apricot
                                          // Alt+p is the quick action even while filtering.
        let outcome = s
            .on_key(KeyCode::Char('p'), KeyModifiers::ALT)
            .expect("emits");
        assert_eq!(outcome.action(), "pin");
    }

    #[test]
    fn scroll_window_advances_with_selection() {
        let mut s = MenuState::new(list(20), 5, false);
        for _ in 0..6 {
            key(&mut s, KeyCode::Down);
        }
        assert_eq!(s.selected, 6);
        assert!(s.scroll > 0); // window scrolled to keep selection visible
        assert!(s.selected >= s.scroll && s.selected < s.scroll + s.viewport);
    }

    #[test]
    fn esc_backs_out_then_cancels() {
        let mut s = MenuState::new(
            vec![item("a", "use", vec![MenuAction::new("rename")])],
            10,
            false,
        );
        key(&mut s, KeyCode::Right);
        assert_eq!(s.focus, Focus::SubMenu);
        assert!(key(&mut s, KeyCode::Esc).is_none()); // back to list
        assert_eq!(s.focus, Focus::List);
        assert_eq!(key(&mut s, KeyCode::Esc), Some(MenuOutcome::Cancelled));
    }

    #[test]
    fn ctrl_c_cancels_from_anywhere() {
        let mut s = MenuState::new(
            vec![item("a", "use", vec![MenuAction::new("rename")])],
            10,
            false,
        );
        key(&mut s, KeyCode::Right);
        assert_eq!(
            s.on_key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(MenuOutcome::Cancelled)
        );
    }

    #[test]
    fn empty_list_only_cancels() {
        let mut s = MenuState::new(vec![], 10, false);
        assert!(key(&mut s, KeyCode::Enter).is_none());
        assert!(key(&mut s, KeyCode::Right).is_none());
        assert_eq!(key(&mut s, KeyCode::Esc), Some(MenuOutcome::Cancelled));
    }

    #[test]
    fn render_list_marks_selection_without_color() {
        let mut s = MenuState::new(list(3), 10, false);
        key(&mut s, KeyCode::Down);
        let lines = s.render_lines(false);
        assert!(lines.iter().any(|l| l.starts_with("❯ item1")));
        assert!(lines.iter().any(|l| l.starts_with("  item0")));
    }
}

impl MenuOutcome {
    #[cfg(test)]
    fn action(&self) -> &str {
        match self {
            MenuOutcome::Selected { action, .. } => action,
            MenuOutcome::Cancelled => "",
        }
    }
}
