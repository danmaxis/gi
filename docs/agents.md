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

## Relationship to `subagentModel` and spawned subagents

`subagentModel` is a separate setting — a fast model intended for *spawned* subagent
subtasks. Per-agent `model` (this page) governs the **interactive** agent you switch to.
A model-callable tool that runs an agent as a spawned subagent (with its own model and
prompt) is planned as a later roadmap slice; until then, agents are activated
interactively via `/agent` or at startup via `defaultAgent`.
