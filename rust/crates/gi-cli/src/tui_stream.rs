//! Streaming bridge between a turn running on a worker thread and the
//! full-screen UI loop.
//!
//! The worker emits [`TurnUpdate`]s over a channel as the model streams; the UI
//! loop applies them to its transcript with the pure [`apply_update`] reducer
//! and redraws. Permission requests flow the same way and the UI replies with an
//! [`ApprovalChoice`]. Keeping the reducer pure (no terminal I/O) makes the
//! streaming logic unit-testable. Slice: unified full-screen mode.

use std::collections::HashMap;

use crate::tui::TranscriptEntry;

/// One incremental update from the worker thread to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TurnUpdate {
    /// Assistant answer text fragment (raw markdown).
    TextDelta(String),
    /// Model thinking fragment (shown only at raw verbosity).
    ThinkingDelta(String),
    /// A tool call began: `summary` is the stripped one-line call detail.
    ToolCallStarted {
        id: String,
        name: String,
        summary: String,
    },
    /// A tool call finished with its output.
    ToolResult {
        id: String,
        output: String,
        is_error: bool,
    },
    /// The worker needs a permission decision; the UI replies with an
    /// [`ApprovalChoice`] on the reply channel. `id` correlates request/reply.
    PermissionRequest {
        id: u64,
        tool_name: String,
        action: String,
        preview: Vec<String>,
    },
    /// The turn finished (Ok) or failed (Err message).
    TurnDone(Result<(), String>),
}

/// The user's reply to a [`TurnUpdate::PermissionRequest`] (y/n/a/A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApprovalChoice {
    Yes,
    No,
    AlwaysTool,
    All,
}

/// Tracks which transcript entries the in-flight turn is growing, so streamed
/// deltas append in place rather than creating a new entry per fragment.
#[derive(Debug, Default)]
pub(crate) struct InFlight {
    /// Index of the in-progress `Assistant` entry (created on first text delta).
    answer_idx: Option<usize>,
    /// Index of the in-progress `Thinking` entry.
    thinking_idx: Option<usize>,
    /// Tool-use id → index of its `Tool` entry, to fill the result later.
    tool_idx: HashMap<String, usize>,
}

impl InFlight {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

/// Apply one [`TurnUpdate`] to the transcript, growing in-progress entries in
/// place. Returns `Some(result)` when the turn finished so the caller stops the
/// streaming loop; `PermissionRequest` returns `None` (handled by the UI overlay,
/// not the transcript).
pub(crate) fn apply_update(
    transcript: &mut Vec<TranscriptEntry>,
    inflight: &mut InFlight,
    update: TurnUpdate,
) -> Option<Result<(), String>> {
    match update {
        TurnUpdate::TextDelta(text) => {
            match inflight.answer_idx {
                Some(index) => {
                    if let Some(TranscriptEntry::Assistant(buffer)) = transcript.get_mut(index) {
                        buffer.push_str(&text);
                    }
                }
                None => {
                    transcript.push(TranscriptEntry::Assistant(text));
                    inflight.answer_idx = Some(transcript.len() - 1);
                }
            }
            None
        }
        TurnUpdate::ThinkingDelta(text) => {
            match inflight.thinking_idx {
                Some(index) => {
                    if let Some(TranscriptEntry::Thinking(buffer)) = transcript.get_mut(index) {
                        buffer.push_str(&text);
                    }
                }
                None => {
                    transcript.push(TranscriptEntry::Thinking(text));
                    inflight.thinking_idx = Some(transcript.len() - 1);
                }
            }
            None
        }
        TurnUpdate::ToolCallStarted { id, name, summary } => {
            // A tool call closes the current answer/thinking accumulation; any
            // following text starts a fresh Assistant entry below the tool.
            inflight.answer_idx = None;
            inflight.thinking_idx = None;
            transcript.push(TranscriptEntry::Tool {
                id: id.clone(),
                name,
                summary,
                output: String::new(),
                is_error: false,
            });
            inflight.tool_idx.insert(id, transcript.len() - 1);
            None
        }
        TurnUpdate::ToolResult {
            id,
            output,
            is_error,
        } => {
            // Pair by id; fall back to the most recent tool entry without output
            // (the executor doesn't always surface the id — Phase 1).
            let index = inflight.tool_idx.get(&id).copied().or_else(|| {
                transcript.iter().rposition(|entry| {
                    matches!(entry, TranscriptEntry::Tool { output, .. } if output.is_empty())
                })
            });
            if let Some(index) = index {
                if let Some(TranscriptEntry::Tool {
                    output: out,
                    is_error: err,
                    ..
                }) = transcript.get_mut(index)
                {
                    *out = output;
                    *err = is_error;
                }
            }
            None
        }
        // The UI renders the overlay and replies on the reply channel; nothing
        // to record in the transcript.
        TurnUpdate::PermissionRequest { .. } => None,
        TurnUpdate::TurnDone(result) => Some(result),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_fields(entry: &TranscriptEntry) -> Option<(&str, &str, &str, bool)> {
        match entry {
            TranscriptEntry::Tool {
                id,
                summary,
                output,
                is_error,
                ..
            } => Some((id, summary, output, *is_error)),
            _ => None,
        }
    }

    #[test]
    fn text_deltas_accumulate_into_one_assistant_entry() {
        let mut t = Vec::new();
        let mut f = InFlight::new();
        apply_update(&mut t, &mut f, TurnUpdate::TextDelta("Hel".into()));
        apply_update(&mut t, &mut f, TurnUpdate::TextDelta("lo".into()));
        assert_eq!(t.len(), 1);
        assert!(matches!(&t[0], TranscriptEntry::Assistant(s) if s == "Hello"));
    }

    #[test]
    fn tool_result_fills_the_matching_call_by_id() {
        let mut t = Vec::new();
        let mut f = InFlight::new();
        apply_update(
            &mut t,
            &mut f,
            TurnUpdate::ToolCallStarted {
                id: "call_1".into(),
                name: "bash".into(),
                summary: "$ ls".into(),
            },
        );
        apply_update(
            &mut t,
            &mut f,
            TurnUpdate::ToolResult {
                id: "call_1".into(),
                output: "file.txt".into(),
                is_error: false,
            },
        );
        assert_eq!(t.len(), 1);
        let (id, summary, output, is_error) = tool_fields(&t[0]).expect("tool entry");
        assert_eq!(
            (id, summary, output, is_error),
            ("call_1", "$ ls", "file.txt", false)
        );
    }

    #[test]
    fn text_after_a_tool_starts_a_new_assistant_entry() {
        let mut t = Vec::new();
        let mut f = InFlight::new();
        apply_update(&mut t, &mut f, TurnUpdate::TextDelta("before".into()));
        apply_update(
            &mut t,
            &mut f,
            TurnUpdate::ToolCallStarted {
                id: "c".into(),
                name: "bash".into(),
                summary: String::new(),
            },
        );
        apply_update(&mut t, &mut f, TurnUpdate::TextDelta("after".into()));
        // Assistant("before"), Tool, Assistant("after")
        assert_eq!(t.len(), 3);
        assert!(matches!(&t[0], TranscriptEntry::Assistant(s) if s == "before"));
        assert!(matches!(&t[1], TranscriptEntry::Tool { .. }));
        assert!(matches!(&t[2], TranscriptEntry::Assistant(s) if s == "after"));
    }

    #[test]
    fn thinking_deltas_accumulate_separately() {
        let mut t = Vec::new();
        let mut f = InFlight::new();
        apply_update(&mut t, &mut f, TurnUpdate::ThinkingDelta("plan ".into()));
        apply_update(&mut t, &mut f, TurnUpdate::ThinkingDelta("more".into()));
        assert_eq!(t.len(), 1);
        assert!(matches!(&t[0], TranscriptEntry::Thinking(s) if s == "plan more"));
    }

    #[test]
    fn turn_done_signals_completion() {
        let mut t = Vec::new();
        let mut f = InFlight::new();
        assert_eq!(
            apply_update(&mut t, &mut f, TurnUpdate::TurnDone(Ok(()))),
            Some(Ok(()))
        );
        assert_eq!(
            apply_update(&mut t, &mut f, TurnUpdate::TurnDone(Err("boom".into()))),
            Some(Err("boom".into()))
        );
    }

    #[test]
    fn permission_request_does_not_touch_transcript() {
        let mut t = Vec::new();
        let mut f = InFlight::new();
        let out = apply_update(
            &mut t,
            &mut f,
            TurnUpdate::PermissionRequest {
                id: 1,
                tool_name: "bash".into(),
                action: "Run shell command".into(),
                preview: vec!["$ rm -rf x".into()],
            },
        );
        assert_eq!(out, None);
        assert!(t.is_empty());
    }
}
