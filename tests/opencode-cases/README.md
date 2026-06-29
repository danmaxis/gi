# opencode interop test cases

Complex, real-world **opencode** configuration cases used to dogfood and
regression-test gi's opencode interop (`gi opencode import|export|status`,
`crates/gi-cli/src/opencode_interop.rs`). Run them with
[`scripts/dogfood-opencode.sh`](../../scripts/dogfood-opencode.sh).

## Provenance / NOTICE
Derived from **opencode** (https://github.com/sst/opencode, MIT-licensed) and
from a real user `~/.config/opencode/opencode.json`. All `apiKey`/token values are
redacted to `REDACTED`. These are configuration fixtures only — no opencode source
is vendored.

## Cases
- `rich-user-config.json` — a real, rich user config: two custom `provider` blocks
  (a synthetic provider + a local Ollama at `.24`), `model`, five **local** `mcp`
  servers (fetch/memory/sequential-thinking/filesystem/context7), `tools` toggles,
  and five custom `agent` definitions (code/debug/build/terminal/plan). Exercises
  faithful `model` + `mcpServers` translation and warn-skip of
  `provider`/`tools`/`agent`.
- `repo-config.jsonc` — opencode's own repo config, in **JSONC** (comments) — checks
  comment stripping on import.
- `remote-mcp-and-permissions.json` — synthesized to cover branches the real config
  lacks: `small_model` (→ `subagentModel`), a **remote** (`type:"remote"`) MCP
  server alongside a local one, and a `permission` block (→ gi `permissions`).

## What the runner asserts
For each case: `gi opencode import <case> --output-format json` returns valid JSON,
does not crash, translates `model`/`mcpServers`, and reports the expected
warn-skips. One case is round-tripped (import → write → `gi opencode export`).
