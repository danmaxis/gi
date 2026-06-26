# Sakana-GI Harness Plan

## Summary

Sakana-GI is a Rust-first fork of `ultraworkers/claw-code`. It keeps claw-code as the implementation base and treats opencode as a compatibility target for config, memory, and workflow interoperability. The differentiator is an agent harness that can pause and ask the user structured interactive questions, run well against Ollama/OpenAI-compatible providers, use stronger project instruction discovery, expose better `SKILL.md` capabilities, and optionally connect to local memory plugins inspired by `akitaonrails/ai-memory`.

The first implemented slice establishes the runtime seams:

- `ask_user` is a model-callable runtime tool that works without MCP.
- The system prompt includes Sakana-GI operating principles.
- `SAKANA_GI_THEME=sakana-dark|sakana-light` selects expressive Japanese/Sakana terminal palettes.
- This document is the canonical roadmap for the rest of the fork.

## Product Direction

Sakana-GI should feel like a pragmatic coding harness, not a research demo. The interface should support repeated engineering work, precise approvals, local model experimentation, and cross-agent context handoff.

Core principles:

- Base all implementation on the claw-code Rust workspace.
- Keep local and OpenAI-compatible models first-class; Ollama is not a workaround path.
- Ask the user when a preference materially changes the result instead of guessing.
- Use Sakana-inspired collective intelligence: combine specialized tools, local project memory, model/provider pluralism, and explicit verification.
- Learn from Sakana AI Scientist limitations by requiring implementation checks, reproducible command evidence, numeric care, sandboxing, and human approval for risky actions.
- Keep opencode compatibility at the boundary layer: config, hooks, generated docs, and memory interoperability before any code vendoring.

## Public Interfaces

### `ask_user` Runtime Tool

Schema:

```json
{
  "question": "Which implementation should I use?",
  "choices": [
    {
      "id": "minimal",
      "label": "Minimal",
      "description": "Smallest change that satisfies the request.",
      "recommended": true
    }
  ],
  "allow_free_text": true,
  "timeout_ms": 120000
}
```

Result:

```json
{
  "answer": "Minimal",
  "choice_id": "minimal",
  "timed_out": false
}
```

V1 behavior:

- The tool is registered as a read-only runtime tool.
- It renders a terminal prompt and accepts a numeric choice, choice id, choice label, default choice, or free text when enabled.
- It returns the answer as a normal tool result, so the same model turn can continue.
- Timeout metadata is accepted in the schema, but strict terminal timeout enforcement is deferred to avoid stranded stdin reader threads.

### Theme Selection

Current interface:

```bash
SAKANA_GI_THEME=sakana-dark gi
SAKANA_GI_THEME=sakana-light gi
```

Follow-up interface:

```text
/theme sakana-dark
/theme sakana-light
```

Theme behavior:

- `sakana-dark`: sumi/indigo foundations with koi, sakura, teal, and matcha accents.
- `sakana-light`: washi/ink foundations with ai-iro, vermilion, matcha, and muted borders.
- Persist selected theme in config once `/theme` is implemented.

### Instruction Loading

Keep the existing claw-code baseline:

- `CLAUDE.md`
- `GI.md`
- `AGENTS.md`
- `CLAUDE.local.md`
- `.gi/CLAUDE.md`
- `.claude/CLAUDE.md`
- `.gi/instructions.md`
- `.gi/rules/`
- `.gi/rules.local/`

Add only opt-in extra patterns through config. Do not silently scan broad globs, home directories, dependency folders, or generated build outputs.

### `SKILL.md` Extensions

Planned frontmatter:

```yaml
---
name: refactor-review
description: Use when reviewing a refactor for behavioral regressions.
version: 1.0.0
tags: [review, refactor]
required_tools: [read_file, grep_search]
provider_hints:
  local_ok: true
references:
  - references/checklist.md
---
```

Behavior:

- Existing `name` and `description` remain compatible.
- Unknown fields should not break old skills.
- Missing referenced files should show diagnostics in `skills list --output-format json`.
- Referenced files must be read before skill execution when the selected skill requires them.

### Local Memory Plugins

V1 memory should be local-only and disabled by default.

Proposed default root:

```text
.sakana/memory/
```

Minimum plugin contract:

- `capture_event`: append session/user/tool/assistant observations.
- `write_handoff`: write a markdown handoff for a future session.
- `query`: search project memory.
- `write_note`: persist a durable decision or convention.
- `pin`: mark a note as exempt from cleanup.

Memory rules:

- Derive project identity from git root by default.
- Keep memory as plain markdown plus small metadata files.
- Treat memory as lower priority than current user messages, system instructions, and repo-local instruction files.
- Support future MCP/server-backed memory without requiring it in v1.

### Provider Compatibility

Ollama and OpenAI-compatible providers should have first-class diagnostics:

- model id;
- effective base URL;
- auth mode;
- tool-call support status;
- streaming support;
- local server health probe result;
- whether slash-containing model ids are preserved or stripped.

Existing `OLLAMA_HOST` support remains the preferred Ollama path.

## Implementation Task List

### Slice 1: Foundation

- [x] Clone `ultraworkers/claw-code` as the Sakana-GI base.
- [x] Add this plan at `docs/sakana-gi-harness-plan.md`.
- [x] Add Sakana-GI operating principles to runtime system prompt assembly.
- [x] Add `sakana-dark` and `sakana-light` theme constructors.
- [x] Add environment-driven theme selection with `SAKANA_GI_THEME`.
- [x] Register `ask_user` as a runtime tool available without MCP.
- [x] Implement terminal execution for `ask_user`.

### Slice 2: Hardening Current Slice

- [x] Add a unit test proving `ask_user` is registered as a read-only runtime tool.
- [x] Add prompt tests proving the Sakana-GI section appears in `system-prompt`.
- [x] Add render tests for `sakana-dark`, `sakana-light`, and unknown theme fallback.
- [x] Add pure tests for `ask_user` default, numeric, id, label, free-text, and invalid-answer resolution.
- [x] Add stdin-backed integration tests for `ask_user` (mock-LLM parity scenario `ask_user_interactive` drives the real `gi` binary with piped non-TTY stdin and asserts the typed `interactive_required` result). A fully-interactive numeric/free-text path over a PTY remains a possible future addition.
- [x] Run `cargo fmt --all`.
- [x] Run targeted tests for `runtime` and `rusty-claude-cli`.
- [x] Run relevant `tools` registry coverage through `cargo test -p tools`.
- [ ] Make the full `tools` package test suite pass in this environment; current blockers are missing `python` and missing `pwsh`/`powershell`.
- [x] Add README/USAGE pointers to this plan and to `SAKANA_GI_THEME`.

### Slice 3: Persistent Themes

- [x] Implement `/theme sakana-dark|sakana-light`.
- [x] Persist theme in the user-level `~/.gi/settings.json` (top-level `theme` key); `SAKANA_GI_THEME` overrides it at render time.
- [x] Include selected theme (`name` + `source`) in `status --output-format json` and the rich status report.
- [x] Include theme diagnostics in `doctor` (effective theme, persisted value, env override, available themes).
- [x] Ensure non-TTY and JSON modes never emit ANSI theme noise (covered by a CLI integration test).

Implementation notes:

- A process-global runtime theme slot in `crates/rusty-claude-cli/src/render.rs` is consulted by `ColorTheme::default()` with precedence `SAKANA_GI_THEME` env → runtime slot (seeded from config / set by `/theme`) → built-in default. This makes a live `/theme` switch affect every `TerminalRenderer::new()` call site.
- Persistence reuses `read_settings_root`/`write_settings_root` via the new `save_user_theme` in `crates/runtime/src/config.rs`; `theme` is registered as a known top-level settings key in `config_validate.rs`.
- `/theme` is wired both in the interactive REPL dispatch and the resume-safe command path (it is `resume_supported`).

### Slice 4: Interactive Question Polish

- [x] Add strict timeout support without leaking blocked stdin readers (Unix polls the stdin fd via `nix::poll` with the requested timeout; no reader thread is spawned, so a timeout strands nothing. Returns a typed `timed_out` result).
- [x] Add typed transcript metadata for questions and answers (`AskUserResult` with `question`, `answer`, `choice_id`, `source`, `timed_out`, `cancelled`, `interactive_required`, serialized as the tool result — no `ContentBlock` schema change).
- [x] Add JSON event output for external UIs and opencode-compatible bridges (`ask_user` `prompt`/`answer` events emitted as JSON lines on stderr when JSON output mode is active; stdout tool-result contract preserved).
- [x] Add cancellation semantics for empty answers when no default exists (resolves to a typed `cancelled` result instead of a hard error).
- [x] Add safeguards so unattended/non-interactive mode reports `interactive_required` instead of blocking (non-TTY stdin short-circuits before any read).

### Slice 5: Skill System Upgrade

- [x] Extend `SKILL.md` frontmatter parsing for `version`, `tags`, `required_tools`, `provider_hints`, and `references` (inline lists, YAML block lists, and nested `provider_hints.local_ok`; unknown keys ignored).
- [x] Add validation diagnostics for malformed metadata (graceful degradation without panics; missing referenced files surfaced as `reference_diagnostics`).
- [~] Add reference-file discovery and read-before-use enforcement. Discovery + missing-file diagnostics are done; read-before-use enforcement in `execute_skill` (`crates/tools/src/lib.rs`) is the next increment.
- [x] Add JSON output fields for skill metadata and diagnostics (`version`, `tags`, `required_tools`, `provider_hints`, `references` per skill; top-level `reference_diagnostics` + `reference_diagnostics_count`, folded into the `degraded` status).
- [x] Add tests for malformed frontmatter, missing references, and legacy skills; duplicate-name (shadowing) behavior is unchanged and still covered by existing tests.

### Slice 6: Local Memory Plugins

- [x] Define the local memory store layout under `.gi/memory/` (notes / handoffs / events).
- [x] Implement local file-backed memory event capture (`events.jsonl`, opt-in auto-capture).
- [x] Implement handoff markdown generation (`handoffs/<id>.md`).
- [x] Implement project memory query over markdown files (notes + handoffs + events).
- [x] Implement pinned durable notes (`/memory pin`, surfaced first in search).
- [x] Add memory diagnostics to `status` (`memory_store`) and `doctor` ("Memory store" check).
- [x] Add config to enable/disable memory (`RuntimeMemoryConfig`, opt-in, disabled by default).
- [x] Document ai-memory interoperability and handoff conventions (`docs/memory.md`).

Done 2026-06-23: opt-in store in `crates/runtime/src/memory.rs`; model-callable
`memory_query`/`memory_write` tools (gated, writes confined to `.gi/memory`); opt-in
per-turn auto-capture; `/memory` REPL subcommands + a `gi memory` CLI (instruction-file
view preserved under `/memory files`).

### Slice 7: Provider Diagnostics

- [x] Add an Ollama health probe using `OLLAMA_HOST` (doctor "Providers" check; `/api/tags`).
- [x] Add OpenAI-compatible `/v1/models` smoke diagnostics where safe (local/LAN base URLs only).
- [x] Report tool-call compatibility hints (from `provider_capabilities_for_model`).
- [x] Improve local model docs for Ollama, llama.cpp, vLLM, and OpenRouter-style gateways.
- [x] Add tests for local base URL routing and credential fallback.

Done 2026-06-23: `check_providers_health` in `gi doctor` resolves the active model's
provider, probes reachability for local endpoints only (never remote SaaS, never an
inference call), and surfaces tool-call/streaming hints; `gi status --output-format json`
gained a `provider` object. Shipped with the new karateka `「技」` splash.

### Slice 8: Opencode Compatibility

- [x] Document supported opencode-style config/hook handoff paths.
- [x] Generate opencode-compatible memory/plugin snippets.
- [x] Add import/export commands only after the local memory plugin contract is stable.
- [x] Defer code vendoring until license/API review is complete.

Done 2026-06-24: `gi opencode export | import | status` (and `/opencode`) translate
config between gi and opencode with a warn-and-skip contract — `model` provider-slug
add/strip, `mcpServers` ⇄ `mcp`, `permissions.defaultMode` → opencode `permission`,
instruction files → `instructions`. `AGENTS.md` and MCP were already natively
interoperable. Export can emit a generated `.opencode/plugin/gi-memory.js` bridge
(read-only `gi_memory` tool over `.gi/memory/`) when memory is enabled. No opencode
source is vendored; shell-hook ↔ TS-plugin handoff is documented, not auto-translated.
Pure translation core in `crates/gi-cli/src/opencode_interop.rs`; mapping reference in
`docs/opencode-compat.md`.

## Roadmap extension: UX & Agents (Slices 9–14)

Goal: make gi feel as polished and capable as more mature terminal agents (opencode,
Claude Code) — opencode-style agents with easy per-agent model switching and a default
model, plus a more aesthetically pleasing interface (kanji motion, margins, bounding
boxes). Sequencing is **agents first**, then a **hybrid** visual track (inline panels
now; an opt-in full-screen TUI later — the line-stream REPL stays the default).

### Slice 9: Agent profiles — per-agent model, default model, `/agent` switching

- [x] `/agent [<name> | reset | list]` switches the active agent, applying its declared
  `model` + `model_reasoning_effort` via the existing runtime-switch path.
- [x] `defaultAgent` config key auto-activates an agent at REPL start (explicit `--model`
  still wins); `model` config remains the base default model.
- [x] Surface the active agent in the banner + `/status`; surface `defaultAgent` in
  `gi status --output-format json`.
- [x] `gi agent list | show <name>` CLI (read-only preview, `--output-format json`).

Done 2026-06-24: agent definitions (already carrying optional `model`/`model_reasoning_effort`)
are now *active*. `/agent` reuses `LiveCli::set_model`/`set_reasoning_effort`; a public
`resolve_agent_profile`/`list_agent_profiles` in the commands crate backs the REPL switch,
the resume preview, and the `gi agent` CLI. `defaultAgent` lives in `RuntimeFeatureConfig`.
Docs: `docs/agents.md`.

### Slice 10: Subagent spawn tool (opencode-style)

- [x] A model-callable `spawn_agent` tool runs a *named agent as a subagent* with its own
  model + instructions to completion and returns the result.
- [x] Per-spawn model/effort override; result captured back into the parent turn.

Done 2026-06-24: opt-in (`subagents.enabled`) `spawn_agent` runtime tool. The subagent
gets its own runtime (own tokio executor → no deadlock when run inside a tool dispatch),
runs **read-only** + non-interactively + iteration-bounded with `emit_output` suppressed,
and is never advertised to itself (thread-local recursion guard → depth ≤ 1). Model
precedence: call `model` → agent `model` → `subagents.model` → default. Agent `.md`
bodies / `.toml` `prompt` keys now flow into `AgentProfile.instructions` as the subagent
system prompt. `RuntimeSubagentConfig` in `crates/runtime/src/config.rs`. Docs:
`docs/agents.md`.

### Slice 11: Inline visual panels, margins & command-popup ranking (Phase-1 aesthetics)

- [x] Width-aware panel foundation: `render::terminal_width()` (clamped), `render::panel()`
  (rounded, themed, rectangular, `NO_COLOR`/narrow-safe), `render::with_gutter()`.
- [x] Frame the startup banner's session info in a panel under the logo.
- [x] Order the live `/` popup by context/usefulness: with an empty filter, rank by a
  curated priority list + recent use, and keep answer-style tokens (`/y`, `/n`) out of the
  initial suggestions (the fuzzy filter takes over once the user types).
- [ ] Apply panels/gutters to **streamed** assistant + tool output and add turn separators.
  Deferred: that output streams live (markdown deltas → stdout), so it needs streaming-safe
  wrapping that can't be verified in the headless sandbox; revisit alongside Slice 13.

Done 2026-06-24 (partial): foundation + popup ranking + framed banner shipped;
streamed-output panels/gutters deferred (see unchecked item).

### Slice 12: Kanji motion & richer thinking states

- [x] Kanji-motion thinking spinner: cycles dojo kanji (技 道 心 気 拳 武) with a rotating
  verb and an indigo↔vermilion shimmer (`thinking_label`/`thinking_shimmer`); NO_COLOR-safe,
  TTY-gated. Builds on the `start_thinking_animation` thread.
- [x] Animated splash: a brief staggered line-by-line reveal of the banner on startup
  (`reveal_banner`), interactive-TTY + color only; scripts / non-TTY / `NO_COLOR` get the
  single-write banner unchanged. Reveal capped (~450 ms) so startup stays snappy.

Done 2026-06-24: pure, unit-tested frame logic (`thinking_label`/`thinking_shimmer`); visuals
TTY/`NO_COLOR`-gated. Interactive rendering needs live confirmation (PTY hangs headless).

### Slice 13: Theme expansion & status line

- [x] More palettes beyond dark/light: `gi-matcha` (green), `gi-sumi` (ink/mono), `gi-sunrise`
  (warm), each with aliases; `render::THEME_NAMES` is the single source of truth.
- [x] A persistent status line (`◈ model · agent · ~tokens · branch`) printed before each REPL
  prompt; dim, NO_COLOR-safe, TTY-gated.
- [x] `/theme` (no arg) now previews every palette with colored swatches.

Done 2026-06-24: pure, unit-tested `compose_status_line` / `theme_swatch` and palette
constructors; visuals TTY/`NO_COLOR`-gated. Live rendering needs visual confirmation.

### Slice 14a: Inline prompt shell (agent + mode bounding box, themed indicator)

- [x] Themed prompt indicator (`render::prompt_glyph` → a colored `❯`) replacing `> `.
- [x] A themed bounding-box header above the prompt (`render::prompt_header`) naming the
  active agent + mode (`╭─ <mode> · <agent> ─╮`).
- [x] `input.rs` tracks the prompt's visible width so the cursor positions correctly with a
  colored prompt; agent moved out of the status line into the box.

Done 2026-06-25.

### Slice 15: Operating modes (default / plan / edit / mugen)

- [x] `SessionMode {default, plan, edit, mugen}` over the permission system; `/mode` +
  Shift+Tab switching; `defaultMode` config; mode shown in the prompt box.
- [x] **default asks before edits, edit auto-accepts** (`PermissionPolicy::with_ask_tools`).
- [x] **per-mode accent colors** on the prompt box + `❯` (`render::mode_accent`); Shift+Tab
  preserves the typed buffer (`ReadOutcome::CycleMode(String)` + `read_line_with_initial`).
- [x] **plan → approve → execute** (`exit_plan_mode` tool, gated on plan mode): the model
  drafts a plan, gi shows it in a panel + asks approval; approve flips to edit mode and
  auto-continues one turn to execute; reject returns feedback and stays in plan.
- [x] **mugen auto-continue** loop (`task_complete` tool + cap + ESC, gated on mugen mode):
  with `modes.mugen.enabled` the REPL re-prompts itself (MUGEN_NUDGE) across turns until the
  model calls `task_complete`, `modes.mugen.maxTurns` (default 25) is hit, or ESC interrupts.
  Opt-in (default off); each auto-turn prints `⟳ MUGEN auto-continue (n/max)`.

Progress 2026-06-25: Slice 15 complete — mode-system core, the two prompt tweaks,
plan→approve→execute, and mugen auto-continue all shipped & unit-tested (gating, config parse,
build/fmt/clippy green). The interactive bits (plan-approval prompt + mode flip; the mugen loop
running across turns + ESC interrupt) need live-terminal smoke (raw-mode tty, not runnable in
the sandbox).

### Slice 16: input box, plan-mode behavior, answer gutter

- [x] **Plan-mode guidance** injected per turn (`mode_guidance` in `prepare_turn_runtime`) so
  models actually draft a plan + call `exit_plan_mode` (and work autonomously + `task_complete`
  in mugen). Plan-approval target is per-agent (`AgentProfile.plan_execute_mode`) or asks
  `[e]dit/[m]ugen`.
- [x] **Answer left-gutter** (`render::StreamGutter`) — a `│ ` bar down streamed answers,
  identity passthrough when piped.
- [x] **Permission-prompt input summary** (`summarize_permission_input`) — path + size for
  writes, command for bash, truncation otherwise (no more full escaped-JSON dumps).
- [x] **Bordered multi-line input box** (`input.rs`): the mode/agent header is the box's
  top-border title; content wraps + grows to 7 rows then scrolls; arrows move within the
  buffer; the redraw tracks rendered rows (`last_block_rows`/`last_cursor_offset`) which fixes
  the wrapped/multi-line first-line **duplication bug**.

Progress 2026-06-25: B/C/D + the input box shipped & unit-tested (wrap/cursor math, gutter,
summary, frontmatter, target parsing; fmt/clippy green). The box's live rendering (growth,
scroll, cursor placement, no duplication) and the plan/gutter visuals need real-terminal smoke
(raw-mode tty, not runnable in the sandbox).

### Slice 14b: Opt-in full-screen TUI (`gi --tui`, Phase 2)

- [x] **Foundation** (2026-06-26): ratatui-based full-screen layout — status bar + scrollback
  transcript + bordered multi-line input — in `crates/gi-cli/src/tui.rs`, driven by
  `run_tui`/`run_tui_loop` (`main.rs`). Mode-aware accent + header title; PageUp/PageDown
  scroll; Shift+Enter newline; `Esc`/`/exit` to quit. A submitted prompt **suspends** the TUI,
  runs the turn with the normal streaming output (markdown, gutter, tool boxes, permission
  prompts, cancellation all intact), then records the result in the transcript and resumes.
  Strictly opt-in (`--tui`); the line-stream REPL stays default and untouched. ratatui 0.29
  reuses the existing crossterm 0.28 (no duplicate).
- [x] **Same-screen turns** (2026-06-26): a submitted prompt now runs **on the same screen**
  (no alt-screen flip) via `run_turn_capture` — a silent (`emit_output=false`) auto-approving
  turn (`TuiAutoPrompter`) whose final text lands in the transcript; a `技 thinking…` indicator
  shows while it works. Interactive runtime tools (`ask_user`/`exit_plan_mode`/`task_complete`)
  are gated off in the TUI (`TUI_ACTIVE`) so they can't block on raw stdin. Verified end-to-end
  in tmux (prompt → thinking → response, clean Esc exit).
- [ ] **Follow-up** (needs live iteration): live token streaming into the pane (background turn
  thread + output sink); in-TUI permission prompts (so it needn't auto-approve); full in-input
  cursor editing (left/right, mid-line); independent transcript scroll polish.

## Acceptance Criteria

- `cargo build --workspace` succeeds.
- `cargo fmt --all --check` passes.
- `gi system-prompt` includes the Sakana-GI operating principles.
- `ask_user` appears in tool definitions and can be called by a model during an interactive REPL session.
- `SAKANA_GI_THEME=sakana-dark` and `SAKANA_GI_THEME=sakana-light` alter rendered terminal colors without affecting JSON output.
- Existing instruction-file loading behavior remains compatible.
- Existing Ollama/OpenAI-compatible provider behavior does not regress.

## Remarks

- The current fork still uses the upstream binary/package names until a deliberate rename pass is planned. Renaming the crate, binary, docs, install scripts, and CI together is safer than changing names opportunistically.
- `ask_user` is intentionally not just a slash command. The model must be able to initiate the question and continue the same turn after the answer.
- The timeout field is included now to stabilize the schema, but real timeout behavior needs careful terminal handling.
- Memory should remain boring and inspectable: markdown first, optional LLM consolidation later, no mandatory vector database.
- Opencode compatibility should be proven through generated config and handoff workflows before importing any implementation code.

## Post-dogfood slice: brand polish + provider/model discovery + command UX (done 2026-06-23)

- [x] Replace residual Claw art: `🦀`→`🥋` spinner; startup splash rewritten to the
  "Cyber Dojo" logo (block `GI` + kanji `技` + scanline + small-caps `harness`, Outrun
  gradient, NO_COLOR-safe) via `startup_logo()`.
- [x] Provider/model discovery: `gi models` + `/models` + first-run scan of provider env
  vars with live model queries (Ollama `/api/tags`, OpenAI-compatible `/v1/models`).
  Providers: Anthropic, OpenAI, xAI, DashScope, **Sakana AI, Kimi (Moonshot), GLM (Zhipu)**,
  Ollama. `apply_saved_provider_env()` applies a persisted provider to the process env at
  startup (in-process only) so a saved pick actually routes.
- [x] Command UX (enhanced rustyline): fuzzy command-name completion + descriptions +
  candidate highlighting; colorized, grouped `/help`.
- [ ] Follow-up: full provider *diagnostics* in `doctor` (Slice 7 health probes); finer
  command sub-grouping; "don't ask again" for first-run.
