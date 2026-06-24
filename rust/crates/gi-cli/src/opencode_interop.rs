//! gi ↔ opencode config interoperability (Slice 8).
//!
//! Pure, I/O-free translation between gi's `.gi/settings.json` (camelCase keys)
//! and opencode's `opencode.json` (snake_case keys). Translation is
//! **warn-and-skip**: keys that map cleanly are translated, keys that have no
//! faithful counterpart are dropped and reported in the returned warning list —
//! never silently lost, never best-effort-guessed.
//!
//! No opencode source is vendored. The one piece of opencode-shaped code we
//! emit is a *generated* memory-bridge plugin template (a string), surfaced via
//! [`render_memory_bridge_plugin`].

use serde_json::{json, Map, Value};

/// JSON Schema URL opencode configs reference.
pub const OPENCODE_SCHEMA_URL: &str = "https://opencode.ai/config.json";

/// gi config keys that have a faithful opencode counterpart and are handled
/// explicitly by [`gi_to_opencode`]. Everything else is warn-and-skipped.
const GI_HANDLED_KEYS: &[&str] = &[
    "$schema",
    "model",
    "subagentModel",
    "mcpServers",
    "permissionMode",
    "permissions",
];

/// opencode config keys handled explicitly by [`opencode_to_gi`]. Everything
/// else is warn-and-skipped.
const OPENCODE_HANDLED_KEYS: &[&str] = &["$schema", "model", "small_model", "mcp"];

/// Translate a merged gi config (`.gi/settings.json` shape) into an opencode
/// `opencode.json` value. `resolved_model` is gi's active model id (already
/// alias-resolved); `instruction_files` are repo-relative instruction-file
/// paths to advertise via opencode's `instructions` array.
///
/// Returns the opencode config plus a list of human-readable warnings for gi
/// keys that were skipped because they do not map onto opencode.
#[must_use]
pub fn gi_to_opencode(
    merged: &Value,
    resolved_model: &str,
    instruction_files: &[String],
) -> (Value, Vec<String>) {
    let mut warnings = Vec::new();
    let mut out = Map::new();
    out.insert("$schema".to_string(), json!(OPENCODE_SCHEMA_URL));

    let gi = merged.as_object();

    // model: bare gi id → "provider/model-id".
    out.insert(
        "model".to_string(),
        json!(model_to_opencode(resolved_model)),
    );

    // subagentModel → small_model.
    if let Some(sub) = gi
        .and_then(|m| m.get("subagentModel"))
        .and_then(Value::as_str)
    {
        out.insert("small_model".to_string(), json!(model_to_opencode(sub)));
    }

    // instruction files → instructions array (AGENTS.md is interoperable both ways).
    if !instruction_files.is_empty() {
        out.insert("instructions".to_string(), json!(instruction_files));
    }

    // permission mode → permission object (coarse, documented mapping). The
    // canonical location is `permissions.defaultMode`; the top-level
    // `permissionMode` is gi's deprecated alias and is honored as a fallback.
    let mode = gi
        .and_then(|m| m.get("permissions"))
        .and_then(Value::as_object)
        .and_then(|p| p.get("defaultMode"))
        .and_then(Value::as_str)
        .or_else(|| {
            gi.and_then(|m| m.get("permissionMode"))
                .and_then(Value::as_str)
        });
    if let Some(mode) = mode {
        out.insert("permission".to_string(), permission_from_gi_mode(mode));
    }
    // gi's allow/deny permission rules are finer-grained than opencode's
    // per-tool object; flag that they aren't carried over.
    if gi
        .and_then(|m| m.get("permissions"))
        .and_then(Value::as_object)
        .is_some_and(|p| p.contains_key("allow") || p.contains_key("deny"))
    {
        warnings.push(
            "skipped `permissions.allow`/`permissions.deny` — rule lists don't map onto opencode's per-tool permission object".to_string(),
        );
    }

    // mcpServers → mcp.
    if let Some(servers) = gi
        .and_then(|m| m.get("mcpServers"))
        .and_then(Value::as_object)
    {
        out.insert("mcp".to_string(), mcp_to_opencode(servers, &mut warnings));
    }

    // Warn-and-skip everything gi-specific.
    if let Some(map) = gi {
        for key in map.keys() {
            if !GI_HANDLED_KEYS.contains(&key.as_str()) {
                warnings.push(skip_message_export(key));
            }
        }
    }

    (Value::Object(out), warnings)
}

/// Translate an opencode `opencode.json` value into a gi `.gi/settings.json`
/// value. Returns the gi config plus warnings for opencode keys that do not map
/// onto gi.
#[must_use]
pub fn opencode_to_gi(opencode: &Value) -> (Value, Vec<String>) {
    let mut warnings = Vec::new();
    let mut out = Map::new();
    let oc = opencode.as_object();

    // gi settings don't carry a `$schema` URL (the schema is named, not
    // remote), so we don't synthesize one here.

    if let Some(model) = oc.and_then(|m| m.get("model")).and_then(Value::as_str) {
        out.insert("model".to_string(), json!(model_from_opencode(model)));
    }
    if let Some(small) = oc
        .and_then(|m| m.get("small_model"))
        .and_then(Value::as_str)
    {
        out.insert(
            "subagentModel".to_string(),
            json!(model_from_opencode(small)),
        );
    }
    if let Some(mcp) = oc.and_then(|m| m.get("mcp")).and_then(Value::as_object) {
        out.insert(
            "mcpServers".to_string(),
            mcp_from_opencode(mcp, &mut warnings),
        );
    }

    if let Some(map) = oc {
        for key in map.keys() {
            if !OPENCODE_HANDLED_KEYS.contains(&key.as_str()) {
                warnings.push(skip_message_import(key));
            }
        }
    }

    (Value::Object(out), warnings)
}

/// Map a gi model id to opencode's `"provider/model-id"` form. The provider
/// slug is derived from the model registry (`api::provider_diagnostics_for_model`),
/// so this stays pure (no env access). If the id already contains a `/`, it is
/// assumed to be qualified and returned unchanged.
#[must_use]
pub fn model_to_opencode(model: &str) -> String {
    if model.contains('/') {
        return model.to_string();
    }
    let diag = api::provider_diagnostics_for_model(model);
    let slug = match diag.provider {
        api::ProviderKind::Anthropic => "anthropic",
        api::ProviderKind::Xai => "xai",
        api::ProviderKind::OpenAi => {
            if diag.auth_env == "DASHSCOPE_API_KEY" {
                "dashscope"
            } else {
                "openai"
            }
        }
    };
    format!("{slug}/{}", diag.resolved_model)
}

/// Strip an opencode `"provider/model-id"` prefix back to a bare gi model id.
/// Unqualified ids pass through unchanged.
#[must_use]
pub fn model_from_opencode(model: &str) -> String {
    model
        .split_once('/')
        .map_or_else(|| model.to_string(), |(_, rest)| rest.to_string())
}

/// Map a gi `mcpServers` object to opencode's `mcp` object.
fn mcp_to_opencode(servers: &Map<String, Value>, warnings: &mut Vec<String>) -> Value {
    let mut out = Map::new();
    for (name, cfg) in servers {
        let Some(obj) = cfg.as_object() else {
            warnings.push(format!("mcpServers.{name}: not an object; skipped"));
            continue;
        };
        let mut entry = Map::new();
        if let Some(command) = obj.get("command").and_then(Value::as_str) {
            // stdio → local: single command array = [command, ...args].
            entry.insert("type".to_string(), json!("local"));
            let mut argv = vec![json!(command)];
            if let Some(args) = obj.get("args").and_then(Value::as_array) {
                argv.extend(args.iter().cloned());
            }
            entry.insert("command".to_string(), Value::Array(argv));
            if let Some(env) = obj.get("env").and_then(Value::as_object) {
                entry.insert("environment".to_string(), Value::Object(env.clone()));
            }
        } else if let Some(url) = obj.get("url").and_then(Value::as_str) {
            // sse/http/ws → remote.
            entry.insert("type".to_string(), json!("remote"));
            entry.insert("url".to_string(), json!(url));
            if let Some(headers) = obj.get("headers").and_then(Value::as_object) {
                entry.insert("headers".to_string(), Value::Object(headers.clone()));
            }
        } else {
            warnings.push(format!(
                "mcpServers.{name}: unsupported transport (no command/url); skipped"
            ));
            continue;
        }
        entry.insert("enabled".to_string(), json!(true));
        out.insert(name.clone(), Value::Object(entry));
    }
    Value::Object(out)
}

/// Map an opencode `mcp` object back to gi's `mcpServers` object.
fn mcp_from_opencode(mcp: &Map<String, Value>, warnings: &mut Vec<String>) -> Value {
    let mut out = Map::new();
    for (name, cfg) in mcp {
        let Some(obj) = cfg.as_object() else {
            warnings.push(format!("mcp.{name}: not an object; skipped"));
            continue;
        };
        let kind = obj.get("type").and_then(Value::as_str);
        let mut entry = Map::new();
        if matches!(kind, Some("local")) || obj.contains_key("command") {
            // local → stdio: split command array into command + args.
            let argv = obj.get("command").and_then(Value::as_array);
            let Some(argv) = argv.filter(|a| !a.is_empty()) else {
                warnings.push(format!("mcp.{name}: local server has no command; skipped"));
                continue;
            };
            entry.insert("type".to_string(), json!("stdio"));
            entry.insert("command".to_string(), argv[0].clone());
            if argv.len() > 1 {
                entry.insert("args".to_string(), Value::Array(argv[1..].to_vec()));
            }
            if let Some(env) = obj.get("environment").and_then(Value::as_object) {
                entry.insert("env".to_string(), Value::Object(env.clone()));
            }
        } else if matches!(kind, Some("remote")) || obj.contains_key("url") {
            let Some(url) = obj.get("url").and_then(Value::as_str) else {
                warnings.push(format!("mcp.{name}: remote server has no url; skipped"));
                continue;
            };
            entry.insert("type".to_string(), json!("http"));
            entry.insert("url".to_string(), json!(url));
            if let Some(headers) = obj.get("headers").and_then(Value::as_object) {
                entry.insert("headers".to_string(), Value::Object(headers.clone()));
            }
            if obj.contains_key("oauth") {
                warnings.push(format!(
                    "mcp.{name}.oauth: review the translated oauth block against gi's schema"
                ));
            }
        } else {
            warnings.push(format!("mcp.{name}: unsupported transport; skipped"));
            continue;
        }
        out.insert(name.clone(), Value::Object(entry));
    }
    Value::Object(out)
}

/// Map a gi `permissionMode` value to an opencode `permission` object. The
/// mapping is intentionally coarse (gi has three modes; opencode has per-tool
/// settings) and is documented in `docs/opencode-compat.md`.
fn permission_from_gi_mode(mode: &str) -> Value {
    let normalized: String = mode
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    match normalized.as_str() {
        // read-only and its gi aliases (default/plan).
        "readonly" | "default" | "plan" => {
            json!({ "edit": "deny", "write": "deny", "bash": "deny", "webfetch": "allow" })
        }
        // danger-full-access and its gi alias (dontAsk).
        "dangerfullaccess" | "dontask" => {
            json!({ "edit": "allow", "write": "allow", "bash": "allow" })
        }
        // workspace-write / acceptEdits / auto and anything unrecognized fall
        // back to the safe middle.
        _ => json!({ "edit": "allow", "write": "allow", "bash": "ask" }),
    }
}

fn skip_message_export(key: &str) -> String {
    let reason = match key {
        "hooks" => "shell-command hooks are not portable to opencode's TS-plugin hooks (see docs/opencode-compat.md)",
        "env" => "no opencode equivalent for process env injection",
        "permissions" => "rule-based permissions don't map to opencode's per-tool permission object",
        "theme" | "aliases" | "sandbox" | "oauth" | "plugins" | "providerFallbacks"
        | "trustedRoots" | "rulesImport" | "memory" => "gi-specific; no opencode counterpart",
        _ => "no opencode counterpart",
    };
    format!("skipped `{key}` — {reason}")
}

fn skip_message_import(key: &str) -> String {
    let reason = match key {
        "instructions" => "gi auto-discovers AGENTS.md/CLAUDE.md/GI.md; add files rather than an instructions list",
        "permission" => "opencode's per-tool permission object can't be reduced to a single gi permissionMode without guessing",
        "agent" | "command" => "custom agents/commands live as gi skills/commands, not config keys",
        "plugin" => "opencode TS plugins are not runnable by gi",
        _ => "no gi counterpart",
    };
    format!("skipped `{key}` — {reason}")
}

/// Strip `//` line comments and `/* */` block comments from a JSONC string so
/// it can be parsed by `serde_json`. String literals are respected so that
/// `//` or `/*` inside a JSON string value is preserved.
#[must_use]
pub fn strip_jsonc_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                // line comment: consume to end of line.
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                // block comment: consume until "*/".
                chars.next(); // consume '*'
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Return a self-contained opencode plugin (JavaScript) that exposes gi's local
/// memory store (`.gi/memory/`) to an opencode session as a read-only
/// `gi_memory` tool. This is *generated* by gi — no opencode code is vendored.
#[must_use]
pub fn render_memory_bridge_plugin() -> String {
    // Kept dependency-light (Node/Bun `fs`) and defensive: missing store → empty
    // result rather than a thrown error.
    r##"// gi-memory.js — GENERATED by `gi opencode export`.
// A read-only bridge that surfaces gi's local memory store (.gi/memory/) to an
// opencode session as a `gi_memory` tool. Plain markdown + JSONL; no database.
// Adapt freely — gi does not vendor or execute opencode plugin code itself.
import { readFileSync, readdirSync, existsSync } from "fs";
import { join } from "path";

function readDir(dir) {
  try {
    return existsSync(dir) ? readdirSync(dir) : [];
  } catch {
    return [];
  }
}

function loadHandoffs(root) {
  const dir = join(root, ".gi", "memory", "handoffs");
  return readDir(dir)
    .filter((f) => f.endsWith(".md"))
    .sort()
    .reverse()
    .slice(0, 5)
    .map((f) => readFileSync(join(dir, f), "utf8"));
}

function loadPinnedNotes(root) {
  const dir = join(root, ".gi", "memory", "notes");
  return readDir(dir)
    .filter((f) => f.endsWith(".md"))
    .map((f) => readFileSync(join(dir, f), "utf8"))
    .filter((body) => /^pinned:\s*true/m.test(body));
}

export const GiMemory = async ({ directory, worktree }) => {
  const root = worktree || directory || process.cwd();
  return {
    tool: {
      gi_memory: {
        description:
          "Read gi's local project memory (.gi/memory): recent handoffs and pinned notes.",
        args: {},
        async execute() {
          const handoffs = loadHandoffs(root);
          const notes = loadPinnedNotes(root);
          if (!handoffs.length && !notes.length) {
            return "gi memory store is empty or disabled (.gi/memory).";
          }
          const parts = [];
          if (handoffs.length) parts.push("# Recent handoffs\n\n" + handoffs.join("\n\n---\n\n"));
          if (notes.length) parts.push("# Pinned notes\n\n" + notes.join("\n\n---\n\n"));
          return parts.join("\n\n");
        },
      },
    },
  };
};
"##
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_round_trips_through_provider_prefix() {
        // Anthropic alias resolves and gains an "anthropic/" prefix.
        let oc = model_to_opencode("opus");
        assert!(oc.starts_with("anthropic/"), "got {oc}");
        // Stripping the prefix yields the resolved bare id.
        assert!(!model_from_opencode(&oc).contains('/'));
        // A grok model maps to the xai slug.
        assert!(model_to_opencode("grok-3").starts_with("xai/"));
        // Already-qualified ids pass through untouched.
        assert_eq!(
            model_to_opencode("anthropic/claude-x"),
            "anthropic/claude-x"
        );
    }

    #[test]
    fn mcp_stdio_round_trips_local() {
        let gi = json!({
            "mcpServers": {
                "fs": { "command": "uvx", "args": ["mcp-fs", "--root", "."], "env": { "X": "1" } }
            }
        });
        let (oc, warnings) = gi_to_opencode(&gi, "opus", &[]);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let server = &oc["mcp"]["fs"];
        assert_eq!(server["type"], json!("local"));
        assert_eq!(server["command"], json!(["uvx", "mcp-fs", "--root", "."]));
        assert_eq!(server["environment"]["X"], json!("1"));

        // Reverse: opencode local → gi stdio with command/args split back out.
        let (back, _) = opencode_to_gi(&oc);
        let gi_server = &back["mcpServers"]["fs"];
        assert_eq!(gi_server["type"], json!("stdio"));
        assert_eq!(gi_server["command"], json!("uvx"));
        assert_eq!(gi_server["args"], json!(["mcp-fs", "--root", "."]));
        assert_eq!(gi_server["env"]["X"], json!("1"));
    }

    #[test]
    fn mcp_remote_round_trips() {
        let gi = json!({
            "mcpServers": {
                "api": { "type": "http", "url": "https://mcp.example/mcp", "headers": { "Authorization": "Bearer x" } }
            }
        });
        let (oc, _) = gi_to_opencode(&gi, "opus", &[]);
        assert_eq!(oc["mcp"]["api"]["type"], json!("remote"));
        assert_eq!(oc["mcp"]["api"]["url"], json!("https://mcp.example/mcp"));

        let (back, _) = opencode_to_gi(&oc);
        assert_eq!(back["mcpServers"]["api"]["type"], json!("http"));
        assert_eq!(
            back["mcpServers"]["api"]["url"],
            json!("https://mcp.example/mcp")
        );
        assert_eq!(
            back["mcpServers"]["api"]["headers"]["Authorization"],
            json!("Bearer x")
        );
    }

    #[test]
    fn permission_mode_maps_to_permission_object() {
        let gi = json!({ "permissionMode": "readOnly" });
        let (oc, _) = gi_to_opencode(&gi, "opus", &[]);
        assert_eq!(oc["permission"]["edit"], json!("deny"));

        let gi = json!({ "permissionMode": "danger-full-access" });
        let (oc, _) = gi_to_opencode(&gi, "opus", &[]);
        assert_eq!(oc["permission"]["bash"], json!("allow"));

        let gi = json!({ "permissionMode": "workspace-write" });
        let (oc, _) = gi_to_opencode(&gi, "opus", &[]);
        assert_eq!(oc["permission"]["edit"], json!("allow"));
        assert_eq!(oc["permission"]["bash"], json!("ask"));
    }

    #[test]
    fn instructions_are_emitted_from_paths() {
        let (oc, _) = gi_to_opencode(
            &json!({}),
            "opus",
            &["AGENTS.md".to_string(), "docs/x.md".to_string()],
        );
        assert_eq!(oc["instructions"], json!(["AGENTS.md", "docs/x.md"]));
        // Empty list omits the key entirely.
        let (oc, _) = gi_to_opencode(&json!({}), "opus", &[]);
        assert!(oc.get("instructions").is_none());
    }

    #[test]
    fn unmapped_gi_keys_warn_and_skip() {
        let gi = json!({
            "model": "opus",
            "hooks": { "PreToolUse": ["echo"] },
            "env": { "FOO": "bar" },
            "theme": "sakana-dark",
            "sandbox": {}
        });
        let (oc, warnings) = gi_to_opencode(&gi, "opus", &[]);
        assert!(oc.get("hooks").is_none());
        assert!(oc.get("env").is_none());
        for key in ["hooks", "env", "theme", "sandbox"] {
            assert!(
                warnings.iter().any(|w| w.contains(key)),
                "expected a warning mentioning `{key}`, got {warnings:?}"
            );
        }
    }

    #[test]
    fn unmapped_opencode_keys_warn_and_skip() {
        let oc = json!({
            "model": "anthropic/claude-x",
            "instructions": ["CONTRIBUTING.md"],
            "permission": { "edit": "ask" },
            "plugin": ["@org/p"]
        });
        let (gi, warnings) = opencode_to_gi(&oc);
        assert_eq!(gi["model"], json!("claude-x"));
        assert!(gi.get("permission").is_none());
        for key in ["instructions", "permission", "plugin"] {
            assert!(
                warnings.iter().any(|w| w.contains(key)),
                "expected a warning mentioning `{key}`, got {warnings:?}"
            );
        }
    }

    #[test]
    fn strips_jsonc_comments_but_preserves_strings() {
        let src = r#"{
  // a line comment
  "model": "anthropic/claude", /* trailing block */
  "url": "https://x/y" /* not // a comment in a string: */,
  "note": "keep // these /* slashes */ literal"
}"#;
        let stripped = strip_jsonc_comments(src);
        let parsed: Value = serde_json::from_str(&stripped).expect("valid JSON after stripping");
        assert_eq!(parsed["model"], json!("anthropic/claude"));
        assert_eq!(parsed["url"], json!("https://x/y"));
        assert_eq!(parsed["note"], json!("keep // these /* slashes */ literal"));
    }

    #[test]
    fn memory_bridge_plugin_is_generated_and_self_describing() {
        let plugin = render_memory_bridge_plugin();
        assert!(plugin.contains("GENERATED by `gi opencode export`"));
        assert!(plugin.contains("gi_memory"));
        assert!(plugin.contains(".gi"));
        // It must not claim to be vendored opencode code.
        assert!(plugin.contains("does not vendor"));
    }
}
