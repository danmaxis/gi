# Local project memory

Gi can keep a small, **local, file-backed memory** for a project under
`.gi/memory/`. It is **opt-in and disabled by default**. When enabled, both you
and the model can record durable context (decisions, conventions, session
handoffs) and search it later — inspired by
[`akitaonrails/ai-memory`](https://github.com/akitaonrails/ai-memory), but with no
server and no database: just plain markdown + JSONL you can read, diff, and commit.

## Enabling it

Add a `memory` block to `.gi/settings.json` (project) or `~/.gi/settings.json`
(user):

```json
{
  "memory": {
    "enabled": true,
    "autoCapture": false,
    "storeRoot": ".gi/memory"
  }
}
```

- `enabled` — turn the whole subsystem on (store, `/memory` store subcommands,
  `gi memory` CLI, and the model-callable `memory_*` tools). Default `false`.
- `autoCapture` — append a one-line observation of every completed turn to the
  event log. Default `false`. Off by default because it records your prompts.
- `storeRoot` — override the store directory (relative to the project root).
  Default `.gi/memory`. Absolute / `..` paths are ignored for safety.

## Layout

```
.gi/memory/
├── notes/<id>.md       durable notes (frontmatter: id, created_at, pinned, tags)
├── handoffs/<id>.md    markdown handoffs written for a future session
└── events.jsonl        append-only per-turn observations (autoCapture)
```

The store always lives under the project root (git root, or the cwd when there is
no repo), so writes never escape the workspace. Pinned notes are surfaced first by
search and are exempt from any future cleanup.

## Using it — you

In the REPL (or via `gi --resume <session> /memory …`):

```text
/memory                  loaded instruction files + a one-line store status
/memory files            loaded instruction files only
/memory note <text>      record a durable note
/memory handoff [text]   write a session handoff
/memory search <query>   search notes + handoffs + events
/memory pin <id>         pin a note (use `unpin` to clear)
/memory list             list notes (pinned first)
```

From scripts, the same surface is a direct CLI subcommand:

```bash
gi memory note "Provider routing is env-driven; OLLAMA_HOST wins"
gi memory list
gi memory search routing
gi memory --output-format json list
```

## Using it — the model

When memory is enabled, two tools are advertised to the model:

- `memory_query { query, limit? }` — read-only; search prior context before asking
  you about decisions that may already be recorded.
- `memory_write { kind: note|handoff|event, text, tags? }` — record reusable
  facts.

Writes are confined to `.gi/memory`, so **enabling memory in config is the
consent** — there is no per-call approval prompt. The model is instructed to record
only genuinely reusable facts, never transient details or secrets.

## Diagnostics

- `gi doctor` shows a **Memory store** check (enabled state + note/handoff/event
  counts, pinned count, store path).
- `gi status --output-format json` includes a `memory_store` object alongside the
  instruction-file `memory_files`.

## Privacy

Everything stays on disk in your workspace; nothing is sent anywhere. `autoCapture`
is the only feature that records prompts, and it is off by default. To wipe the
store, delete `.gi/memory/`. To stop entirely, set `"enabled": false` (or remove the
`memory` block).

## Handoff conventions

A handoff is a short markdown note that lets a future session resume quickly. Good
handoffs answer: *what changed, what's verified, what's next, and any gotchas.*
Write one at the end of a working session with `/memory handoff` (or have the model
call `memory_write` with `kind: handoff`), then `gi memory search` for it next time.
