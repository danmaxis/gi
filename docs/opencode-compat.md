# Opencode compatibility

Gi and [opencode](https://opencode.ai) are both terminal coding agents. They
overlap enough — both read `AGENTS.md`, both speak MCP — that you can move
config between them instead of hand-translating it. This page documents what is
**natively interoperable**, what gi can **translate** for you, and what does
**not** carry over.

Gi does **not** vendor or execute any opencode code. The one piece of
opencode-shaped output it produces is a *generated* memory-bridge plugin (a
JavaScript snippet you own and can edit).

## The `gi opencode` command

```bash
gi opencode export                 # print an opencode.json built from gi's config
gi opencode export --out ./        # write opencode.json (+ AGENTS.md, + memory bridge)
gi opencode import opencode.json   # print gi settings translated from an opencode.json
gi opencode import opencode.json --out .gi/settings.json
gi opencode status                 # detect opencode artifacts in this directory
```

`--output-format json` is available on all of them. Inside the REPL the same
verbs are available as `/opencode export | import <path> | status`.

Translation is **warn-and-skip**: keys that map cleanly are translated, keys
with no faithful counterpart are dropped and listed in a warning — never
silently lost, never best-effort-guessed.

## Natively interoperable (no translation needed)

- **`AGENTS.md`** — gi reads `AGENTS.md` from the project root (and ancestry) into
  the system prompt, exactly like opencode. Drop the same file in either tool.
  (gi also reads `CLAUDE.md` and `GI.md`.)
- **MCP servers** — both tools speak the Model Context Protocol. The *shape* of
  the config differs (see below), but the servers themselves are identical.

## What `gi opencode` translates

| gi `.gi/settings.json` | opencode `opencode.json` | Notes |
| --- | --- | --- |
| `model` (`claude-opus-4-8`) | `model` (`anthropic/claude-opus-4-8`) | export adds the provider slug from gi's model registry; import strips it |
| `subagentModel` | `small_model` | direct |
| `mcpServers` `{command, args, env}` | `mcp` `{type:"local", command:[…], environment}` | gi's `command` + `args` ⇄ opencode's single `command` array |
| `mcpServers` `{type:"sse"/"http", url, headers}` | `mcp` `{type:"remote", url, headers}` | remote transports |
| `permissions.defaultMode` (or deprecated `permissionMode`) | `permission` `{edit, write, bash, …}` | coarse mapping, see below |
| discovered instruction files | `instructions: […]` | export lists `AGENTS.md`/`CLAUDE.md`/`GI.md` paths |

Permission-mode mapping (gi has three modes; opencode is per-tool):

| gi mode | opencode `permission` |
| --- | --- |
| `read-only` (aliases `default`, `plan`) | `edit: deny, write: deny, bash: deny, webfetch: allow` |
| `workspace-write` (aliases `acceptEdits`, `auto`) | `edit: allow, write: allow, bash: ask` |
| `danger-full-access` (alias `dontAsk`) | `edit: allow, write: allow, bash: allow` |

## What does **not** carry over (warn-and-skip)

- **Hooks.** gi hooks are **shell commands** (`PreToolUse`/`PostToolUse`/
  `PostToolUseFailure`). opencode hooks are **TypeScript/JavaScript plugins**
  that subscribe to events (`tool.execute.before`, `tool.execute.after`, …).
  They are not runtime-portable. See *Hook handoff* below for how to reproduce
  one as the other by hand.
- **gi-specific config:** `env`, `theme`, `aliases`, `sandbox`, `oauth`,
  `plugins`, `providerFallbacks`, `trustedRoots`, `rulesImport`, `memory`,
  and `permissions.allow`/`permissions.deny` rule lists.
- **opencode-specific config (on import):** `instructions` (gi auto-discovers
  instruction files instead), `permission` (opencode's per-tool object can't be
  reduced to one gi mode without guessing), `agent`, `command`, `plugin`,
  `server`, `share`, `autoupdate`, `default_agent`, `tools`.

Every skipped key is reported in the command's warning list, so nothing
disappears quietly.

## Hook handoff (manual)

gi's shell hook:

```json
{ "hooks": { "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "command": "./scripts/guard.sh" } ] } ] } }
```

…corresponds, conceptually, to an opencode plugin hook:

```ts
export const Guard = async ({ $ }) => ({
  event: async ({ event }) => {
    if (event.type === "tool.execute.before" && event.tool === "bash") {
      await $`./scripts/guard.sh`;
    }
  },
});
```

gi does not generate this for you — the two execution models differ too much for
a faithful automatic translation. The mapping above is the documented bridge.

## Memory bridge

gi keeps an opt-in, file-backed memory store under `.gi/memory/` (plain markdown
+ JSONL — see [memory.md](memory.md)). opencode has no built-in persistent
memory; users wire it in via plugins.

When memory is **enabled** and you run `gi opencode export --out <dir>`, gi
writes a generated `.opencode/plugin/gi-memory.js`. It exposes a read-only
`gi_memory` tool to the opencode session that surfaces gi's recent handoffs and
pinned notes straight from `.gi/memory/`. It is a self-contained starting point:
own it, edit it, no opencode code is vendored.

## Scope

This is **boundary-layer** compatibility — config, MCP, instructions, and a
memory bridge. It deliberately does **not** vendor opencode source, run opencode
plugins, or translate opencode custom agents/commands (`.opencode/agent/`,
`.opencode/command/`). Those remain possible future work.
