//! Local, file-backed project memory under `.gi/memory/`.
//!
//! Opt-in and disabled by default (see [`crate::config::RuntimeMemoryConfig`]).
//! The store keeps three kinds of artifact, all plain text so they stay
//! inspectable and interoperable (ai-memory style):
//!
//! - `notes/<id>.md` — durable notes with YAML-ish frontmatter (`id`,
//!   `created_at`, `pinned`, `tags`); pinned notes are exempt from cleanup and
//!   surface first in queries.
//! - `handoffs/<id>.md` — markdown handoffs written for a future session.
//! - `events.jsonl` — append-only log of per-turn observations.
//!
//! The store directory is always resolved under the project root (git root, or
//! the cwd when there is no repo), so writes never escape the workspace.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A durable note read back from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryNote {
    pub id: String,
    pub created_at_ms: u128,
    pub pinned: bool,
    pub tags: Vec<String>,
    pub text: String,
}

/// A search result across notes / handoffs / events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryHit {
    pub kind: &'static str,
    pub id: String,
    pub pinned: bool,
    pub snippet: String,
}

/// Counts for diagnostics (`/memory`, doctor, status).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemoryStoreSummary {
    pub notes: usize,
    pub pinned: usize,
    pub handoffs: usize,
    pub events: usize,
}

/// A file-backed memory store rooted at `<project>/.gi/memory/`.
#[derive(Debug, Clone)]
pub struct MemoryStore {
    root: PathBuf,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |delta| delta.as_millis())
}

/// Keep only path-safe characters so a caller-supplied id can never traverse.
fn sanitize_id(id: &str) -> String {
    id.chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect()
}

fn nearest_git_root(cwd: &Path) -> Option<PathBuf> {
    let mut cursor = Some(cwd);
    while let Some(dir) = cursor {
        let marker = dir.join(".git");
        if marker.is_dir() || marker.is_file() {
            return Some(dir.to_path_buf());
        }
        cursor = dir.parent();
    }
    None
}

impl MemoryStore {
    /// Resolve the store directory for `cwd`. `store_root_override` is treated as
    /// relative to the project root; absolute or `..`-bearing overrides are
    /// ignored so the store stays inside the workspace.
    #[must_use]
    pub fn discover(cwd: &Path, store_root_override: Option<&str>) -> Self {
        let project_root = nearest_git_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
        let root = match store_root_override {
            Some(value)
                if !value.is_empty()
                    && !value.contains("..")
                    && !Path::new(value).is_absolute() =>
            {
                project_root.join(value)
            }
            _ => project_root.join(".gi").join("memory"),
        };
        Self { root }
    }

    /// Construct a store at an explicit directory (used in tests).
    #[must_use]
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn notes_dir(&self) -> PathBuf {
        self.root.join("notes")
    }

    fn handoffs_dir(&self) -> PathBuf {
        self.root.join("handoffs")
    }

    fn events_path(&self) -> PathBuf {
        self.root.join("events.jsonl")
    }

    fn unique_id(&self, dir: &Path, prefix: &str) -> String {
        let base = now_ms();
        let mut id = format!("{prefix}-{base}");
        let mut suffix = 1u32;
        while dir.join(format!("{id}.md")).exists() {
            id = format!("{prefix}-{base}-{suffix}");
            suffix += 1;
        }
        id
    }

    /// Write a durable note; returns its id.
    ///
    /// # Errors
    /// Returns the underlying I/O error when the store directory can't be written.
    pub fn write_note(&self, text: &str, tags: &[String]) -> io::Result<String> {
        let dir = self.notes_dir();
        fs::create_dir_all(&dir)?;
        let id = self.unique_id(&dir, "note");
        let body = format!(
            "---\nid: {id}\ncreated_at: {created}\npinned: false\ntags: [{tags}]\n---\n\n{text}\n",
            created = now_ms(),
            tags = tags.join(", "),
            text = text.trim(),
        );
        fs::write(dir.join(format!("{id}.md")), body)?;
        Ok(id)
    }

    /// Set (or clear) a note's pinned flag. Returns false when the id is unknown.
    ///
    /// # Errors
    /// Returns the underlying I/O error when the note can't be rewritten.
    pub fn pin(&self, id: &str, pinned: bool) -> io::Result<bool> {
        let id = sanitize_id(id);
        let path = self.notes_dir().join(format!("{id}.md"));
        if !path.is_file() {
            return Ok(false);
        }
        let content = fs::read_to_string(&path)?;
        fs::write(&path, set_frontmatter_pinned(&content, pinned))?;
        Ok(true)
    }

    /// Write a markdown handoff for a future session; returns its id.
    ///
    /// # Errors
    /// Returns the underlying I/O error when the handoff can't be written.
    pub fn write_handoff(&self, summary: &str) -> io::Result<String> {
        let dir = self.handoffs_dir();
        fs::create_dir_all(&dir)?;
        let id = self.unique_id(&dir, "handoff");
        let body = format!(
            "---\nid: {id}\ncreated_at: {created}\n---\n\n# Handoff\n\n{summary}\n",
            created = now_ms(),
            summary = summary.trim(),
        );
        fs::write(dir.join(format!("{id}.md")), body)?;
        Ok(id)
    }

    /// Append an observation to the event log.
    ///
    /// # Errors
    /// Returns the underlying I/O error when the log can't be appended to.
    pub fn capture_event(&self, observation: &str) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        let line = serde_json::json!({ "ts": now_ms() as u64, "observation": observation });
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.events_path())?;
        writeln!(file, "{line}")
    }

    /// All notes, newest first (pinned notes float to the top).
    ///
    /// # Errors
    /// Returns the underlying I/O error when the notes directory can't be read.
    pub fn list_notes(&self) -> io::Result<Vec<MemoryNote>> {
        let mut notes = Vec::new();
        let dir = self.notes_dir();
        if !dir.is_dir() {
            return Ok(notes);
        }
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&path) {
                notes.push(parse_note(&path, &content));
            }
        }
        notes.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.created_at_ms.cmp(&a.created_at_ms))
        });
        Ok(notes)
    }

    /// Search notes, handoffs, and events for `query` (case-insensitive),
    /// pinned-first, newest-first, capped at `limit`.
    ///
    /// # Errors
    /// Returns the underlying I/O error when the store can't be read.
    pub fn query(&self, query: &str, limit: usize) -> io::Result<Vec<MemoryHit>> {
        let needle = query.trim().to_lowercase();
        let mut hits = Vec::new();
        if needle.is_empty() {
            return Ok(hits);
        }

        for note in self.list_notes()? {
            let haystack = format!("{} {}", note.text, note.tags.join(" ")).to_lowercase();
            if haystack.contains(&needle) {
                hits.push(MemoryHit {
                    kind: "note",
                    id: note.id,
                    pinned: note.pinned,
                    snippet: snippet(&note.text, &needle),
                });
            }
        }

        let handoffs = self.handoffs_dir();
        if handoffs.is_dir() {
            for entry in fs::read_dir(&handoffs)? {
                let path = entry?.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                    continue;
                }
                if let Ok(content) = fs::read_to_string(&path) {
                    if content.to_lowercase().contains(&needle) {
                        hits.push(MemoryHit {
                            kind: "handoff",
                            id: file_stem(&path),
                            pinned: false,
                            snippet: snippet(&content, &needle),
                        });
                    }
                }
            }
        }

        if let Ok(content) = fs::read_to_string(self.events_path()) {
            for line in content.lines() {
                if line.to_lowercase().contains(&needle) {
                    hits.push(MemoryHit {
                        kind: "event",
                        id: String::new(),
                        pinned: false,
                        snippet: snippet(line, &needle),
                    });
                }
            }
        }

        hits.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.id.cmp(&a.id)));
        hits.truncate(limit);
        Ok(hits)
    }

    /// Counts for diagnostics.
    #[must_use]
    pub fn summary(&self) -> MemoryStoreSummary {
        let notes = self.list_notes().unwrap_or_default();
        let pinned = notes.iter().filter(|note| note.pinned).count();
        let handoffs = count_md(&self.handoffs_dir());
        let events = fs::read_to_string(self.events_path())
            .map(|content| {
                content
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .count()
            })
            .unwrap_or(0);
        MemoryStoreSummary {
            notes: notes.len(),
            pinned,
            handoffs,
            events,
        }
    }
}

fn count_md(dir: &Path) -> usize {
    if !dir.is_dir() {
        return 0;
    }
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
                .count()
        })
        .unwrap_or(0)
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Extract a short snippet of text around the first match of `needle`.
fn snippet(text: &str, needle: &str) -> String {
    let body = text
        .lines()
        .find(|line| line.to_lowercase().contains(needle))
        .unwrap_or_else(|| text.lines().next().unwrap_or(""))
        .trim();
    let trimmed: String = body.chars().take(120).collect();
    if body.chars().count() > 120 {
        format!("{trimmed}…")
    } else {
        trimmed
    }
}

/// Rewrite the `pinned:` frontmatter line, inserting one if absent.
fn set_frontmatter_pinned(content: &str, pinned: bool) -> String {
    let mut lines: Vec<String> = content.lines().map(ToString::to_string).collect();
    let mut in_frontmatter = false;
    let mut replaced = false;
    for line in &mut lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            if in_frontmatter {
                break;
            }
            in_frontmatter = true;
            continue;
        }
        if in_frontmatter && trimmed.starts_with("pinned:") {
            *line = format!("pinned: {pinned}");
            replaced = true;
            break;
        }
    }
    if !replaced {
        // Insert after the opening fence.
        if let Some(pos) = lines.iter().position(|line| line.trim() == "---") {
            lines.insert(pos + 1, format!("pinned: {pinned}"));
        }
    }
    let mut out = lines.join("\n");
    if content.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Parse a note file's frontmatter + body.
fn parse_note(path: &Path, content: &str) -> MemoryNote {
    let mut id = file_stem(path);
    let mut created_at_ms = 0u128;
    let mut pinned = false;
    let mut tags = Vec::new();
    let mut lines = content.lines();
    let mut body_start = 0usize;

    if lines.next().map(str::trim) == Some("---") {
        let mut consumed = 1usize;
        for line in lines.by_ref() {
            consumed += 1;
            let trimmed = line.trim();
            if trimmed == "---" {
                break;
            }
            if let Some(value) = trimmed.strip_prefix("id:") {
                id = value.trim().to_string();
            } else if let Some(value) = trimmed.strip_prefix("created_at:") {
                created_at_ms = value.trim().parse().unwrap_or(0);
            } else if let Some(value) = trimmed.strip_prefix("pinned:") {
                pinned = value.trim() == "true";
            } else if let Some(value) = trimmed.strip_prefix("tags:") {
                tags = parse_tag_list(value.trim());
            }
        }
        body_start = consumed;
    }

    let text = content
        .lines()
        .skip(body_start)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    MemoryNote {
        id,
        created_at_ms,
        pinned,
        tags,
        text,
    }
}

fn parse_tag_list(value: &str) -> Vec<String> {
    value
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> MemoryStore {
        // Unique per call: `now_ms()` alone collides when tests run in parallel
        // (two stores in the same millisecond would share a directory and
        // cross-contaminate), so add a process-unique counter.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("gi-memory-test-{}-{unique}", now_ms()));
        MemoryStore::at(dir)
    }

    #[test]
    fn note_roundtrip_pin_and_query() {
        let store = temp_store();
        let id = store
            .write_note(
                "Use the run_async pattern for sync→async",
                &["arch".to_string()],
            )
            .expect("write note");
        let notes = store.list_notes().expect("list");
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, id);
        assert!(!notes[0].pinned);
        assert_eq!(notes[0].tags, vec!["arch".to_string()]);
        assert!(notes[0].text.contains("run_async"));

        assert!(store.pin(&id, true).expect("pin"));
        assert!(store.list_notes().expect("list2")[0].pinned);

        let hits = store.query("run_async", 10).expect("query");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, "note");
        assert!(hits[0].pinned);

        // Unknown id does not panic and reports false.
        assert!(!store.pin("does-not-exist", true).expect("pin missing"));

        let _ = fs::remove_dir_all(store.root());
    }

    #[test]
    fn handoff_and_event_capture_and_query() {
        let store = temp_store();
        store
            .write_handoff("Finished provider diagnostics; memory store is next.")
            .expect("handoff");
        store
            .capture_event("ran cargo test --workspace")
            .expect("event");
        store.capture_event("committed slice 7").expect("event2");

        let summary = store.summary();
        assert_eq!(summary.handoffs, 1);
        assert_eq!(summary.events, 2);

        let hits = store.query("provider diagnostics", 10).expect("query");
        assert!(hits.iter().any(|hit| hit.kind == "handoff"));
        let hits = store.query("cargo test", 10).expect("query2");
        assert!(hits.iter().any(|hit| hit.kind == "event"));

        let _ = fs::remove_dir_all(store.root());
    }

    #[test]
    fn pin_id_is_sanitized_against_traversal() {
        let store = temp_store();
        // A traversal id can't escape the notes dir; it simply finds nothing.
        assert!(!store.pin("../../etc/passwd", true).expect("sanitized pin"));
        let _ = fs::remove_dir_all(store.root());
    }
}
