# Local OpenAI-compatible providers and skills setup

This guide covers two common offline/local workflows:

1. running Gi against an OpenAI-compatible local model server such as Ollama, llama.cpp, or vLLM; and
2. installing local skills from disk so Gi can discover them without network access.

## Gi is not Claude-only

Gi Code is a Claude-Code-shaped workflow/runtime, not a Claude-only product. It supports Anthropic directly and can target OpenAI-compatible, provider-routed, and local models depending on configuration. Non-Claude providers are supported honestly: they may require stricter tool-call and response-shape compatibility, and some slash/tool workflows can be rougher than first-party Anthropic/OpenAI paths. Provider-specific identity leaks are bugs, not intended product positioning.

If you need the most polished daily-driver experience for a specific non-Claude model today, compare that provider’s native tools. If you need runtime/provider hackability, Gi’s OpenAI-compatible route is the intended extension path.

## OpenAI-compatible routing basics

Set `OPENAI_BASE_URL` to the server’s `/v1` endpoint and set `OPENAI_API_KEY` to either the required token or a harmless placeholder for local servers that expect an Authorization header. Authless local/private OpenAI-compatible servers can leave `OPENAI_API_KEY` unset. The model name must match what the server exposes.

```bash
export OPENAI_BASE_URL="http://127.0.0.1:11434/v1"
export OPENAI_API_KEY="local-dev-token"
gi --model "qwen3:latest" prompt "Reply exactly HELLO_WORLD_123"
```

Routing notes:

- Use the `openai/` prefix for OpenAI-compatible gateways when you need prefix routing to win over ambient Anthropic credentials, for example `--model "openai/gpt-4.1-mini"` with OpenRouter.
- For local servers, prefer the exact model ID reported by the server (`qwen3:latest`, `llama3.2`, etc.). If your local gateway exposes slash-containing IDs, prefix the exact slug with `local/` so Gi routes through OpenAI-compatible transport while sending the rest verbatim, for example `--model "local/Qwen/Qwen2.5-Coder-7B-Instruct"`.
- If you have multiple provider keys in your environment, `OPENAI_BASE_URL` plus local-looking tags such as `llama3.2` or `qwen2.5-coder:7b` selects the local OpenAI-compatible route; use `local/` for slash-containing local IDs.
- Tool workflows need model/server support for OpenAI-compatible tool calls. Plain prompt smoke tests can pass even when slash/tool workflows still fail because the server returns an incompatible tool-call shape.

## Raw `/v1/chat/completions` smoke test

Before debugging Gi, verify the local server speaks the expected wire format:

```bash
curl -sS "$OPENAI_BASE_URL/chat/completions" \
  -H "Authorization: Bearer ${OPENAI_API_KEY:-local-dev-token}" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "qwen3:latest",
    "messages": [{"role": "user", "content": "Reply exactly HELLO_WORLD_123"}],
    "stream": false
  }'
```

Expected result: a JSON response with one assistant message containing `HELLO_WORLD_123`. If this fails, fix the local server, model name, or auth token before changing Gi settings.

## Ollama

Start Ollama and pull a model:

```bash
ollama pull qwen3:latest
ollama serve
```

In another shell:

```bash
export OLLAMA_HOST="http://127.0.0.1:11434"
gi --model "qwen3:latest" prompt "Reply exactly HELLO_WORLD_123"
```

`OLLAMA_HOST` is the preferred env var for Ollama. Gi routes all models to the local OpenAI-compatible endpoint automatically when this is set, and no API key is needed. The older `OPENAI_BASE_URL` + `OPENAI_API_KEY` workaround is also supported for existing setups.

If Ollama is running without auth, `unset OPENAI_API_KEY` is acceptable. Use a placeholder token rather than a real cloud API key if your local server requires an Authorization header.

## llama.cpp server

Start a llama.cpp OpenAI-compatible server with the model name you want Gi to send:

```bash
llama-server -m ./models/qwen2.5-coder.gguf --host 127.0.0.1 --port 8080 --alias qwen2.5-coder
```

Then smoke-test through Gi:

```bash
export OPENAI_BASE_URL="http://127.0.0.1:8080/v1"
export OPENAI_API_KEY="local-dev-token"
gi --model "qwen2.5-coder" prompt "Reply exactly HELLO_WORLD_123"
```

## vLLM or another OpenAI-compatible server

Start vLLM with an OpenAI-compatible API server:

```bash
vllm serve Qwen/Qwen2.5-Coder-7B-Instruct --host 127.0.0.1 --port 8000
```

Then route Gi to it:

```bash
export OPENAI_BASE_URL="http://127.0.0.1:8000/v1"
export OPENAI_API_KEY="local-dev-token"
gi --model "Qwen/Qwen2.5-Coder-7B-Instruct" prompt "Reply exactly HELLO_WORLD_123"
```

## Local skills install from disk

Skills are discovered from Gi skill roots such as `.gi/skills/` in a workspace and `~/.gi/skills/` for user-level installs. Legacy `.codex/skills/` roots may also be scanned for compatibility, but new local Gi projects should prefer `.gi/skills/`.

A skill directory should contain a `SKILL.md` file with frontmatter:

```text
my-skill/
└── SKILL.md
```

```markdown
---
name: my-skill
description: Explain when this skill should be used.
---

# My Skill

Instructions for the agent go here.
```

Install a skill from a local path in the interactive REPL:

```text
/skills install /absolute/path/to/my-skill
/skills list
/skills my-skill
```

Or inspect skills from the direct CLI surface:

```bash
gi skills --output-format json
```

Offline install checklist:

- Install the specific skill directory, not only the repository root, unless that repository root itself contains `SKILL.md`.
- Keep the frontmatter `name` aligned with the directory name users will type.
- After installing, run `/skills list` or `gi skills --output-format json` to confirm the discovered name and source path.
- If a skill invocation fails with an HTTP/provider error, the skill may have installed correctly but the current model/provider call failed. Run `gi doctor`, verify provider credentials, and try a simple prompt smoke test before reinstalling the skill.

## Troubleshooting

| Symptom | Check |
|---|---|
| Gi still asks for Anthropic credentials | Use an explicit OpenAI-compatible model route or remove unrelated Anthropic env vars during local smoke tests. |
| `model not found` from local server | Use the exact model ID exposed by Ollama/llama.cpp/vLLM. |
| Plain prompt works but tools fail | Confirm the model/server supports OpenAI-compatible tool calls and response shapes. |
| Skill says installed but `/skills <name>` fails | Check `/skills list` for the discovered name and source; verify provider credentials separately with `gi doctor`. |
| A local docs/log file contains secrets | Redact it before using `@path` file context or attaching it to an issue. |
