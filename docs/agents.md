# Agents

An **agent** in gi is a named profile — a description, an optional model, and an
optional reasoning effort — that you can switch to mid-session. Selecting an agent
applies its model and effort to subsequent turns, so you can keep a fast/cheap model
for routine work and switch to a stronger one for review or planning without retyping
`/model`.

## Defining an agent

Agents are plain files under an `agents/` directory. gi discovers them from several
scopes (project first, then user), so a project can override a user-level agent of the
same name:

- `.gi/agents/` (project) · `.codex/agents/` · `.claude/agents/`
- `$GI_CONFIG_HOME/agents/` · `$CODEX_HOME/agents/` · `$CLAUDE_CONFIG_DIR/agents/`
- `~/.gi/agents/` · `~/.codex/agents/` · `~/.claude/agents/` (user)

A definition is TOML or Markdown-with-frontmatter. Both forms carry the same fields;
`model` and `model_reasoning_effort` are optional.

`.gi/agents/reviewer.toml`:

```toml
name = "reviewer"
description = "Careful code reviewer."
model = "anthropic/claude-sonnet-4-6"     # optional
model_reasoning_effort = "high"            # optional: low | medium | high
```

`.gi/agents/planner.md`:

```markdown
---
name: planner
description: Breaks work into small, verifiable steps.
model: anthropic/claude-opus-4-8
model_reasoning_effort: medium
---
```

## Switching agents in the REPL

```
/agent                 # list agents; the active one is marked ▸
/agent reviewer        # switch: applies the agent's model + reasoning effort
/agent reset           # revert to the base/default model, clear the active agent
```

Switching reuses the same runtime path as `/model`, so the session is preserved — only
the model (and effort) change. The active agent is shown in the startup banner and in
`/status`.

## A default agent

Set `defaultAgent` in `.gi/settings.json` (project) or `~/.gi/settings.json` (user) to
auto-activate an agent when a session starts:

```json
{ "defaultAgent": "reviewer" }
```

The default agent's model becomes the session's starting model. An explicit `--model`
flag still wins — it is not overridden by `defaultAgent`. `gi status --output-format
json` reports the configured `default_agent`.

## Model resolution (precedence)

When you switch to an agent interactively:

1. the agent's `model` (if it declares one), else
2. the current session model is kept.

Reasoning effort uses the agent's `model_reasoning_effort` if set, otherwise it is left
unchanged. Model aliases in an agent definition resolve the same way as `--model` /
`/model` (via your `aliases` config), so `model = "opus"` works.

The base/default model itself follows the usual precedence: `--model` flag → `GI_MODEL`
/ `ANTHROPIC_MODEL` env → `model` config key → compiled default.

## CLI

The `gi agent` subcommand previews agents non-interactively (switching is REPL-only,
since it mutates a live session):

```bash
gi agent list                          # all discovered agents + their model/effort
gi agent show reviewer                 # one agent's resolved model/effort/source
gi agent show reviewer --output-format json
```

## Agent instructions

For Markdown agents, everything after the closing `---` of the frontmatter is the
agent's **instruction body** — its system prompt when it runs as a spawned subagent
(below). TOML agents can carry the same via a `prompt = """…"""` (or `instructions`)
key. The body is optional; without it, an agent is just a model/effort profile.

## Spawning subagents (`spawn_agent`)

When enabled, the model can **delegate a focused subtask to a subagent** via the
`spawn_agent` tool — opencode-style. The subagent runs to completion with its own model
and returns its findings as the tool result, so the main agent can parallelize analysis,
code review, search, or planning.

Enable it (opt-in, off by default) in `.gi/settings.json`:

```json
{
  "subagents": {
    "enabled": true,
    "model": "anthropic/claude-haiku-4-5",   // default subagent model (optional)
    "maxIterations": 16                         // per-subagent tool-iteration cap (optional)
  }
}
```

The model calls it with a self-contained `prompt`, optionally naming an `agent` (to use
its model + instructions) and/or overriding `model` / `reasoning_effort`:

```json
{ "agent": "reviewer", "prompt": "Review crates/api/src/client.rs for error handling gaps." }
```

**Safety model.** A subagent:

- runs **read-only** — it can read/search the workspace but cannot edit files or run
  commands (writes are denied), regardless of the parent's permission mode;
- runs **non-interactively** (it cannot prompt the user);
- is **bounded** by `maxIterations` (default 16) to cap cost;
- **cannot spawn further subagents** — `spawn_agent` is never advertised to a subagent,
  so nesting is capped at one level;
- runs on its **own runtime** (its intermediate output is suppressed; you see a brief
  `⟳ spawning …` / `✓ … finished` marker).

The subagent model is resolved as: the call's `model` → the named agent's `model` →
`subagents.model` → the compiled default.
