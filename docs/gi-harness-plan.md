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

- [ ] Define `.sakana/memory/plugin.toml`.
- [ ] Implement local file-backed memory event capture.
- [ ] Implement handoff markdown generation.
- [ ] Implement project memory query over markdown files.
- [ ] Implement pinned durable notes.
- [ ] Add memory diagnostics to `status` and `doctor`.
- [ ] Add config to enable/disable memory plugins.
- [ ] Document ai-memory interoperability and handoff conventions.

### Slice 7: Provider Diagnostics

- [ ] Add an Ollama health probe using `OLLAMA_HOST`.
- [ ] Add OpenAI-compatible `/v1/models` or chat smoke diagnostics where safe.
- [ ] Report tool-call compatibility hints.
- [ ] Improve local model docs for Ollama, llama.cpp, vLLM, and OpenRouter-style gateways.
- [ ] Add tests for local base URL routing and credential fallback.

### Slice 8: Opencode Compatibility

- [ ] Document supported opencode-style config/hook handoff paths.
- [ ] Generate opencode-compatible memory/plugin snippets.
- [ ] Add import/export commands only after the local memory plugin contract is stable.
- [ ] Defer code vendoring until license/API review is complete.

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
