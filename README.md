# Gi

**Gi** is a Rust-first terminal coding-agent harness. It drives an LLM through a
tool loop with explicit permissions and a workspace jail, and adds a few things on
top of its base: interactive `ask_user` questions the model can ask mid-turn,
expressive terminal themes, stronger project-instruction discovery, and richer
`SKILL.md` capabilities. Local and OpenAI-compatible providers (Ollama, llama.cpp,
vLLM, …) are first-class, not an afterthought.

The binary is `gi`. Config lives in `~/.gi/`; environment variables use the `GI_`
prefix.

> Gi is a fork of [`ultraworkers/claw-code`](https://github.com/ultraworkers/claw-code)
> (MIT). It keeps that workspace as its implementation base and rebrands it to `gi`.
> See the [ownership disclaimer](#ownership--affiliation) below.

## What Gi adds

- **`ask_user` runtime tool** — the model can pause and ask a structured question
  (choices, free text) and continue the same turn with your answer. Non-TTY runs
  report `interactive_required` instead of blocking; timeouts are enforced without
  leaking reader threads.
- **Terminal themes** — `gi-dark` / `gi-light`, selected with `GI_THEME` or the
  `/theme` slash command and persisted to `~/.gi/settings.json`. Reported by
  `gi status --output-format json` and `gi doctor`; JSON output stays ANSI-free.
- **`SKILL.md` extensions** — optional `version`, `tags`, `required_tools`,
  `provider_hints`, and `references` frontmatter, with missing-reference
  diagnostics surfaced in `gi skills list --output-format json`.
- **Provider/model discovery** — `gi models` (and a first-run prompt) scans your
  environment for configured providers (Anthropic, OpenAI, xAI, DashScope, Sakana,
  Kimi, GLM, Ollama), live-queries their model lists, and persists your pick.
- **Operating principles** baked into the system prompt: compare approaches, prefer
  the smallest verifiable change, stay provider-agnostic, and keep risky actions
  behind explicit approval.

The canonical roadmap is [`docs/gi-harness-plan.md`](./docs/gi-harness-plan.md).

## Quick start

```bash
# 1. Clone and build
git clone https://github.com/danmaxis/gi
cd gi/rust
cargo build --workspace

# 2. Set your API key (an Anthropic API key — not a Claude subscription)
export ANTHROPIC_API_KEY="sk-ant-..."

# 3. Verify everything is wired up
./target/debug/gi doctor

# 4. Run a one-shot prompt
./target/debug/gi prompt "say hello"

# 5. Start an interactive session
./target/debug/gi
```

On Windows (PowerShell) the binary is `gi.exe`: use `.\target\debug\gi.exe` or
`cargo run -- prompt "say hello"`.

Local / OpenAI-compatible providers (Ollama, llama.cpp, vLLM) are supported via
`ANTHROPIC_BASE_URL` / `OLLAMA_HOST`; see
[`docs/local-openai-compatible-providers.md`](./docs/local-openai-compatible-providers.md).

## Repository shape

- **`rust/`** — the Rust workspace and the `gi` CLI binary (crate `gi-cli`).
- **`USAGE.md`** — task-oriented usage guide (commands, auth, sessions, config, themes).
- **`docs/`** — design notes; `gi-harness-plan.md` is the roadmap.
- **`PHILOSOPHY.md` / `ROADMAP.md` / `PARITY.md`** — intent, backlog, and Rust-port parity.

## Verification

From the `rust/` directory:

```bash
../scripts/fmt.sh --check
cargo build --workspace
cargo clippy --workspace
cargo test --workspace
```

## Documentation map

- [`USAGE.md`](./USAGE.md) — commands, auth, sessions, config, terminal themes
- [`docs/gi-harness-plan.md`](./docs/gi-harness-plan.md) — roadmap and design
- [`docs/local-openai-compatible-providers.md`](./docs/local-openai-compatible-providers.md) — Ollama / llama.cpp / vLLM setup
- [`docs/windows-install-release.md`](./docs/windows-install-release.md) — Windows / PowerShell install notes
- [`rust/README.md`](./rust/README.md) — crate map and CLI surface
- [`CONTRIBUTING.md`](./CONTRIBUTING.md), [`SECURITY.md`](./SECURITY.md), [`CODE_OF_CONDUCT.md`](./CODE_OF_CONDUCT.md)
- [`LICENSE`](./LICENSE) — MIT

## Ownership / affiliation

- Gi is a fork of [`ultraworkers/claw-code`](https://github.com/ultraworkers/claw-code)
  (MIT), which is itself part of the broader agent-code / Claude Code lineage.
  Upstream attributions are preserved.
- This repository does **not** claim ownership of the original source material and
  is **not affiliated with, endorsed by, or maintained by Anthropic**.
- Distributed under the [MIT License](./LICENSE).
