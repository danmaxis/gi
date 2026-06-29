//! Opt-in full-screen TUI scaffold (`gi --tui`, Slice 14b foundation).
//!
//! This module is pure rendering: it draws a status bar, a scrollback
//! transcript, and a bordered multi-line input using ratatui. The control loop
//! and turn execution live in `main.rs` (`run_tui` / `run_tui_loop`), which
//! suspends the TUI to run a turn with the normal streaming output, then records
//! the result here. The default line-stream REPL is untouched.

use ansi_to_tui::IntoText;
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use crate::render::TerminalRenderer;

/// One entry in the scrollback transcript. Tool/thinking entries keep their raw
/// data so Ctrl+O can re-render them at any detail level. Slice 17.
pub(crate) enum TranscriptEntry {
    User(String),
    Assistant(String),
    System(String),
    Tool {
        /// Tool-use id, used to pair a streamed result with its call. Empty for
        /// entries captured post-turn where the id isn't needed.
        id: String,
        name: String,
        /// One-line call detail (e.g. `$ ls -la`, `📄 Reading x`), shown while the
        /// tool runs and above its output. Empty when not captured.
        summary: String,
        output: String,
        is_error: bool,
    },
    Thinking(String),
    /// Captured ANSI output from a slash command (rendered colored). Slice:
    /// unified full-screen mode (Phase 2 followup).
    CommandOutput(String),
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
    /// True while a turn is running (shows a thinking indicator). Slice 14b.
    pub busy: bool,
    /// Detail level (Ctrl+O): controls tool-output truncation + thinking. Slice 17.
    pub verbosity: crate::render::RenderVerbosity,
    /// When set, a permission approval overlay is drawn over the transcript.
    pub approval: Option<&'a ApprovalView>,
    /// Cursor position (char index) within `input`.
    pub input_cursor: usize,
    /// Command/`@`-mention popup rows (label, description); empty = no popup.
    pub popup: &'a [(String, String)],
    /// Highlighted popup row.
    pub popup_selected: usize,
}

/// A pending permission request rendered as a centered overlay. Slice: unified
/// full-screen mode.
pub(crate) struct ApprovalView {
    pub tool_name: String,
    pub action: String,
    pub preview: Vec<String>,
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

/// Accent-highlight a line of plain slash-command output: section headers (no
/// indent, not a `/command`) in bold accent, and `/command  description` rows
/// with the command token in accent and the description dimmed.
fn style_command_line(line: &str, accent_color: Color) -> Line<'static> {
    let dim = Style::default().fg(Color::DarkGray);
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return Line::raw("");
    }
    let indent = &line[..line.len() - trimmed.len()];
    if trimmed.starts_with('/') {
        // Split the command token from its description at the first 2+ spaces.
        if let Some(pos) = trimmed.find("  ") {
            let (cmd, rest) = trimmed.split_at(pos);
            return Line::from(vec![
                Span::raw(indent.to_string()),
                Span::styled(cmd.to_string(), Style::default().fg(accent_color).bold()),
                Span::styled(rest.to_string(), dim),
            ]);
        }
        return Line::from(vec![
            Span::raw(indent.to_string()),
            Span::styled(
                trimmed.to_string(),
                Style::default().fg(accent_color).bold(),
            ),
        ]);
    }
    // A short, non-indented line reads as a section header.
    if indent.is_empty() && trimmed.len() <= 32 {
        return Line::styled(line.to_string(), Style::default().fg(accent_color).bold());
    }
    Line::styled(line.to_string(), Style::default())
}

/// Render one frame of the TUI.
pub(crate) fn draw(frame: &mut Frame, state: &TuiState) {
    let area = frame.area();
    let accent_color = accent(state.mode);
    let border = Style::default().fg(accent_color);

    // Input box grows with its content (up to 7 rows), plus borders. A popup
    // (slash commands / @-mentions) gets its own band below the input.
    let input_rows = state.input.split('\n').count().clamp(1, 7) as u16;
    let popup_rows = (state.popup.len() as u16).min(6);
    let chunks = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(input_rows + 2),
        Constraint::Length(popup_rows),
        Constraint::Length(1),
    ])
    .split(area);

    // Transcript pane.
    let inner_w = chunks[0].width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();
    let block = |body: &str, head_style: Style, rest_dim: bool| -> Vec<Line<'static>> {
        wrap_line(body, inner_w.max(1))
            .into_iter()
            .enumerate()
            .map(|(i, wl)| {
                let style = if i == 0 {
                    head_style
                } else if rest_dim {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default()
                };
                Line::styled(wl, style)
            })
            .collect()
    };
    let dim = Style::default().fg(Color::DarkGray);
    let renderer = TerminalRenderer::new();
    for entry in state.transcript {
        match entry {
            TranscriptEntry::User(text) => {
                lines.extend(block(
                    &format!("❯ {text}"),
                    Style::default().fg(accent_color).bold(),
                    false,
                ));
            }
            TranscriptEntry::Assistant(text) => {
                // `◂ gi` header, then the answer rendered as markdown (bold,
                // code, lists, headings) with a 4-space gutter — same look as the
                // inline REPL. Falls back to plain text if conversion fails.
                lines.extend(block("◂ gi", dim, false));
                let ansi = renderer.markdown_to_ansi(text);
                match ansi.into_text() {
                    Ok(rendered) => {
                        for line in rendered.lines {
                            let mut spans = vec![Span::raw("  ")];
                            spans.extend(line.spans);
                            lines.push(Line::from(spans));
                        }
                    }
                    Err(_) => {
                        for line in text.lines() {
                            lines.extend(block(&format!("  {line}"), Style::default(), false));
                        }
                    }
                }
            }
            TranscriptEntry::System(text) => {
                lines.extend(block(&format!("· {text}"), dim, true));
            }
            TranscriptEntry::Thinking(text) => {
                if !state.verbosity.shows_thinking() {
                    continue;
                }
                lines.extend(block(&format!("▶ thinking  {text}"), dim, true));
            }
            TranscriptEntry::CommandOutput(text) => {
                // If the command emitted ANSI, render it colored; otherwise apply
                // light accent highlighting (section headers + `/command` tokens)
                // so plain reports (like /help) aren't a wall of monochrome.
                if text.contains('\x1b') {
                    match text.into_text() {
                        Ok(rendered) => lines.extend(rendered.lines),
                        Err(_) => lines.extend(block(text, dim, true)),
                    }
                } else {
                    for line in text.lines() {
                        lines.push(style_command_line(line, accent_color));
                    }
                }
            }
            TranscriptEntry::Tool {
                name,
                summary,
                output,
                is_error,
                ..
            } => {
                let mark = if *is_error { "✘" } else { "⚙" };
                let head = Style::default().fg(if *is_error {
                    Color::Rgb(236, 72, 120)
                } else {
                    accent_color
                });
                lines.extend(block(&format!("{mark} {name}"), head, false));
                // The call detail (args) under the header, while running and after.
                for line in summary.lines().filter(|line| !line.trim().is_empty()) {
                    lines.extend(block(&format!("  {line}"), dim, true));
                }
                if output.is_empty() {
                    lines.extend(block("  …", dim, true));
                }
                // Output: full in verbose/raw, else first lines + a Ctrl+O hint.
                let out_lines: Vec<&str> = output.lines().collect();
                let limit = if state.verbosity.shows_full() {
                    out_lines.len()
                } else {
                    6
                };
                for line in out_lines.iter().take(limit) {
                    lines.extend(block(&format!("  {line}"), dim, true));
                }
                if out_lines.len() > limit {
                    lines.extend(block(
                        &format!("  … +{} lines — Ctrl+O to expand", out_lines.len() - limit),
                        dim,
                        true,
                    ));
                }
            }
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
                .border_type(BorderType::Rounded)
                .border_style(border)
                .title(format!(" gi · {} ", state.title)),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(transcript, chunks[0]);

    // Input box — or a thinking indicator while a turn runs.
    let input_line = if state.busy {
        Line::from(vec![Span::styled(
            "技 thinking…",
            Style::default().fg(accent_color).bold(),
        )])
    } else {
        Line::from(vec![
            Span::styled("❯ ", Style::default().fg(accent_color).bold()),
            Span::raw(state.input),
        ])
    };
    let input_title = if state.busy {
        " working — Ctrl+C to interrupt "
    } else {
        " message · Enter to send · Shift+Enter newline · Esc to quit "
    };
    let input = Paragraph::new(input_line)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border)
                .title(input_title),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(input, chunks[1]);

    // Cursor: derive (row, col) from the char index so Left/Right/Home/End and
    // mid-buffer edits land the caret correctly.
    let before: String = state.input.chars().take(state.input_cursor).collect();
    let cursor_row = before.matches('\n').count().min(6) as u16;
    let cursor_col = before.rsplit('\n').next().unwrap_or("").chars().count() as u16;
    let cursor_x = chunks[1].x + 1 + 2 + cursor_col;
    let max_x = chunks[1].x + chunks[1].width.saturating_sub(2);
    frame.set_cursor_position((cursor_x.min(max_x), chunks[1].y + 1 + cursor_row));

    // Popup (slash commands / @-mentions) below the input box.
    if !state.popup.is_empty() {
        let rows: Vec<Line> = state
            .popup
            .iter()
            .enumerate()
            .map(|(i, (label, desc))| {
                let marker = if i == state.popup_selected {
                    "❯ "
                } else {
                    "  "
                };
                let mut spans = vec![
                    Span::styled(marker, Style::default().fg(accent_color)),
                    Span::styled(
                        label.clone(),
                        if i == state.popup_selected {
                            Style::default().fg(accent_color).bold()
                        } else {
                            Style::default()
                        },
                    ),
                ];
                if !desc.is_empty() {
                    spans.push(Span::styled(
                        format!("  {desc}"),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                Line::from(spans)
            })
            .collect();
        frame.render_widget(Paragraph::new(rows), chunks[2]);
    }

    // Status bar.
    let status = Paragraph::new(Line::styled(
        format!(" {} ", state.status),
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(status, chunks[3]);

    // Permission approval overlay (centered, over the transcript).
    if let Some(approval) = state.approval {
        let mut body: Vec<Line> = Vec::new();
        body.push(Line::styled(
            approval.action.clone(),
            Style::default().bold(),
        ));
        for line in &approval.preview {
            body.push(Line::styled(line.clone(), dim));
        }
        body.push(Line::raw(""));
        body.push(Line::styled(
            "[y]es   [n]o   [a]lways this tool   [A]ll tools",
            Style::default().fg(accent_color).bold(),
        ));
        let height = (body.len() as u16 + 2).min(area.height);
        let width = (area.width * 3 / 4).clamp(20, area.width.saturating_sub(2));
        let popup = Rect {
            x: area.x + (area.width.saturating_sub(width)) / 2,
            y: area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, popup);
        let overlay = Paragraph::new(body)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Rgb(236, 72, 120)))
                    .title(format!(" approve · {} ", approval.tool_name)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(overlay, popup);
    }
}

#[cfg(test)]
mod tests {
    use super::wrap_line;

    #[test]
    fn markdown_answer_converts_to_ratatui_text() {
        use ansi_to_tui::IntoText;
        let renderer = crate::render::TerminalRenderer::new();
        let ansi = renderer.markdown_to_ansi("**bold** and `code`\n\n- one\n- two");
        let text = ansi
            .into_text()
            .expect("markdown ANSI should convert to ratatui Text");
        // Non-empty and carries the list content (styling is applied via spans).
        let joined: String = text
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect();
        assert!(joined.contains("bold"));
        assert!(joined.contains("one") && joined.contains("two"));
    }

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
