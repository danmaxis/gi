//! Opt-in full-screen TUI scaffold (`gi --tui`, Slice 14b foundation).
//!
//! This module is pure rendering: it draws a status bar, a scrollback
//! transcript, and a bordered multi-line input using ratatui. The control loop
//! and turn execution live in `main.rs` (`run_tui` / `run_tui_loop`), which
//! suspends the TUI to run a turn with the normal streaming output, then records
//! the result here. The default line-stream REPL is untouched.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

/// Who produced a transcript entry.
pub(crate) enum Role {
    User,
    Assistant,
    System,
}

/// One line of the scrollback transcript.
pub(crate) struct TranscriptEntry {
    pub role: Role,
    pub text: String,
}

/// Everything the renderer needs for one frame.
pub(crate) struct TuiState<'a> {
    pub transcript: &'a [TranscriptEntry],
    pub input: &'a str,
    /// Box title (mode · note · agent).
    pub title: &'a str,
    /// Mode key driving the accent color.
    pub mode: &'a str,
    /// Plain status line (model · tokens · branch).
    pub status: &'a str,
    /// Lines scrolled up from the bottom (PageUp).
    pub scroll_back: u16,
}

/// Accent color for the active mode (mirrors `render::mode_accent`).
fn accent(mode: &str) -> Color {
    match mode {
        "plan" => Color::Rgb(96, 165, 250),
        "edit" => Color::Rgb(122, 199, 120),
        "mugen" => Color::Rgb(236, 72, 120),
        _ => Color::Rgb(120, 120, 140),
    }
}

/// Word-wrap `text` to `width` columns, splitting on existing newlines and
/// hard-breaking words longer than the width.
fn wrap_line(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    for logical in text.split('\n') {
        let mut line = String::new();
        let mut col = 0usize;
        let push_word = |line: &mut String, col: &mut usize, out: &mut Vec<String>, word: &str| {
            let wlen = word.chars().count();
            if *col == 0 {
                if wlen <= width {
                    line.push_str(word);
                    *col = wlen;
                } else {
                    for ch in word.chars() {
                        if *col == width {
                            out.push(std::mem::take(line));
                            *col = 0;
                        }
                        line.push(ch);
                        *col += 1;
                    }
                }
            } else if *col + 1 + wlen <= width {
                line.push(' ');
                line.push_str(word);
                *col += 1 + wlen;
            } else {
                out.push(std::mem::take(line));
                *col = 0;
                if wlen <= width {
                    line.push_str(word);
                    *col = wlen;
                } else {
                    for ch in word.chars() {
                        if *col == width {
                            out.push(std::mem::take(line));
                            *col = 0;
                        }
                        line.push(ch);
                        *col += 1;
                    }
                }
            }
        };
        for word in logical.split(' ') {
            push_word(&mut line, &mut col, &mut out, word);
        }
        out.push(line);
    }
    out
}

/// Render one frame of the TUI.
pub(crate) fn draw(frame: &mut Frame, state: &TuiState) {
    let area = frame.area();
    let accent_color = accent(state.mode);
    let border = Style::default().fg(accent_color);

    // Input box grows with its content (up to 7 rows), plus borders.
    let input_rows = state.input.split('\n').count().clamp(1, 7) as u16;
    let chunks = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(input_rows + 2),
        Constraint::Length(1),
    ])
    .split(area);

    // Transcript pane.
    let inner_w = chunks[0].width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();
    for entry in state.transcript {
        let (prefix, style) = match entry.role {
            Role::User => ("❯ ", Style::default().fg(accent_color).bold()),
            Role::Assistant => ("", Style::default()),
            Role::System => ("· ", Style::default().fg(Color::DarkGray)),
        };
        let body = format!("{prefix}{}", entry.text);
        for (i, wl) in wrap_line(&body, inner_w.max(1)).into_iter().enumerate() {
            let line_style = if i == 0 { style } else { Style::default() };
            lines.push(Line::styled(wl, line_style));
        }
        lines.push(Line::raw(""));
    }
    let total = lines.len() as u16;
    let view = chunks[0].height.saturating_sub(2);
    let scroll = total.saturating_sub(view).saturating_sub(state.scroll_back);
    let transcript = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border)
                .title(format!(" gi · {} ", state.title)),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(transcript, chunks[0]);

    // Input box.
    let input_line = Line::from(vec![
        Span::styled("❯ ", Style::default().fg(accent_color).bold()),
        Span::raw(state.input),
    ]);
    let input = Paragraph::new(input_line)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border)
                .title(" message · Enter to send · Shift+Enter newline · Esc to quit "),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(input, chunks[1]);

    // Cursor: after the `❯ ` glyph on the (last) input line. Foundation: the
    // cursor lives at the end of the buffer.
    let last_input_line = state.input.rsplit('\n').next().unwrap_or("");
    let extra_rows = state.input.matches('\n').count().min(6) as u16;
    let cursor_x = chunks[1].x + 1 + 2 + last_input_line.chars().count() as u16;
    let max_x = chunks[1].x + chunks[1].width.saturating_sub(2);
    frame.set_cursor_position((cursor_x.min(max_x), chunks[1].y + 1 + extra_rows));

    // Status bar.
    let status = Paragraph::new(Line::styled(
        format!(" {} ", state.status),
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(status, chunks[2]);
}

#[cfg(test)]
mod tests {
    use super::wrap_line;

    #[test]
    fn wrap_line_wraps_words_and_hard_splits_long_ones() {
        // Word wrap at width 10.
        assert_eq!(
            wrap_line("the quick brown fox", 10),
            vec!["the quick".to_string(), "brown fox".to_string()]
        );
        // A word longer than the width is hard-split.
        assert_eq!(
            wrap_line("abcdefghijk", 5),
            vec!["abcde".to_string(), "fghij".to_string(), "k".to_string()]
        );
        // Existing newlines start new rows.
        assert_eq!(
            wrap_line("a\nb", 10),
            vec!["a".to_string(), "b".to_string()]
        );
    }
}
