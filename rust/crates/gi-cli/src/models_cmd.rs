//! `gi models` / `/models` / first-run provider + model discovery flow.
//!
//! Scans the environment for configured providers, live-queries each one's
//! model list where possible (Ollama `/api/tags`, OpenAI-compatible `/v1/models`),
//! and — in a terminal — lets the user pick a default model that is persisted to
//! `~/.gi/settings.json`. Non-interactive callers print suggestions or JSON
//! instead of prompting.

use std::io::{self, IsTerminal, Write};

use api::{
    detect_available_providers, fetch_ollama_models, fetch_openai_compat_models, DetectedProvider,
    ListStyle,
};
use serde_json::json;

use crate::CliOutputFormat;

/// A provider plus its resolved model list.
pub struct ProviderModels {
    pub provider: DetectedProvider,
    pub models: Vec<String>,
    pub live: bool,
    pub error: Option<String>,
}

/// Run an async future to completion on a fresh single-threaded runtime, on a
/// dedicated thread so it is safe even if called from within another runtime.
pub(crate) fn run_async<T, F>(future: F) -> T
where
    F: std::future::Future<Output = T> + Send,
    T: Send,
{
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("models runtime should build")
                    .block_on(future)
            })
            .join()
            .expect("models query thread should not panic")
    })
}

fn api_key_for(kind: &str) -> Option<String> {
    let candidates: &[&str] = match kind {
        "openai" => &["OPENAI_API_KEY"],
        "xai" => &["XAI_API_KEY"],
        "dashscope" => &["DASHSCOPE_API_KEY"],
        "sakana" => &["SAKANA_API_KEY", "SAKANA_AI_API_KEY"],
        "kimi" => &["KIMI_API_KEY", "MOONSHOT_API_KEY"],
        "glm" => &["GLM_API_KEY", "ZHIPUAI_API_KEY"],
        _ => return None,
    };
    candidates
        .iter()
        .find_map(|var| std::env::var(var).ok().filter(|value| !value.is_empty()))
}

/// Detect providers and resolve each one's model list (live where reachable,
/// else the built-in suggestions).
#[must_use]
pub fn discover_provider_models() -> Vec<ProviderModels> {
    detect_available_providers()
        .into_iter()
        .map(|provider| {
            let mut models = Vec::new();
            let mut live = false;
            let mut error = None;
            let should_query = provider.list_style.is_some()
                && (provider.credentialed || provider.kind == "ollama");
            if should_query {
                if let Some(base) = provider.effective_base_url() {
                    let result = match provider.list_style {
                        Some(ListStyle::OllamaTags) => run_async(fetch_ollama_models(&base)),
                        Some(ListStyle::OpenAiModels) => {
                            let key = api_key_for(provider.kind);
                            run_async(fetch_openai_compat_models(&base, key.as_deref()))
                        }
                        None => Ok(Vec::new()),
                    };
                    match result {
                        Ok(found) if !found.is_empty() => {
                            models = found;
                            live = true;
                        }
                        Ok(_) => {}
                        Err(message) => error = Some(message),
                    }
                }
            }
            if models.is_empty() {
                models = provider
                    .static_models
                    .iter()
                    .map(|model| (*model).to_string())
                    .collect();
            }
            ProviderModels {
                provider,
                models,
                live,
                error,
            }
        })
        .collect()
}

fn json_value(discovered: &[ProviderModels]) -> serde_json::Value {
    json!({
        "kind": "models",
        "action": "list",
        "status": "ok",
        // Model lists are best-effort metadata queries to already-configured
        // endpoints; the command never requires a provider inference request.
        "requires_provider_request": false,
        "local_only": true,
        "providers": discovered.iter().map(|entry| json!({
            "kind": entry.provider.kind,
            "label": entry.provider.label,
            "base_url": entry.provider.effective_base_url(),
            "credentialed": entry.provider.credentialed,
            "models_source": if entry.live { "live" } else { "suggested" },
            "models": entry.models,
            "error": entry.error,
        })).collect::<Vec<_>>(),
    })
}

/// Flatten credentialed providers into a numbered list of selectable
/// `(kind, base_url, model)` options.
fn selectable_options(discovered: &[ProviderModels]) -> Vec<(&ProviderModels, &str)> {
    let mut options = Vec::new();
    for entry in discovered
        .iter()
        .filter(|entry| entry.provider.credentialed)
    {
        for model in &entry.models {
            options.push((entry, model.as_str()));
        }
    }
    options
}

fn print_report(discovered: &[ProviderModels]) {
    println!("Models");
    for entry in discovered {
        let mark = if entry.provider.credentialed {
            "●"
        } else {
            "○"
        };
        let source = if entry.live {
            "live"
        } else if entry.provider.credentialed {
            "suggested"
        } else {
            "not configured"
        };
        println!("  {mark} {} ({source})", entry.provider.label);
        if let Some(base) = entry.provider.effective_base_url() {
            println!("      base   {base}");
        }
        if entry.models.is_empty() {
            println!("      models <none>");
        } else {
            println!("      models {}", entry.models.join(", "));
        }
        if let Some(error) = &entry.error {
            println!("      note   live query failed: {error}");
        }
    }
}

/// Entry point for `gi models`, `/models`, and first-run. `interactive` enables
/// the picker when stdin/stdout are a TTY; otherwise it prints suggestions.
pub fn run_models_command(
    output_format: CliOutputFormat,
    interactive: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let discovered = discover_provider_models();

    if matches!(output_format, CliOutputFormat::Json) {
        println!(
            "{}",
            serde_json::to_string_pretty(&json_value(&discovered))?
        );
        return Ok(());
    }

    print_report(&discovered);

    let is_tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    if !interactive || !is_tty {
        println!();
        println!("  Pick a default with `gi models` in a terminal, or `gi --model <name>`.");
        return Ok(());
    }

    let options = selectable_options(&discovered);
    if options.is_empty() {
        println!();
        println!(
            "  No configured providers found. Set ANTHROPIC_API_KEY / OPENAI_API_KEY / OLLAMA_HOST"
        );
        println!("  (or run `gi setup`), then rerun `gi models`.");
        return Ok(());
    }

    println!();
    println!("Select a default model:");
    for (index, (entry, model)) in options.iter().enumerate() {
        println!("  {}. {} · {model}", index + 1, entry.provider.label);
    }
    print!("Choice [1-{}, blank to skip]: ", options.len());
    io::stdout().flush()?;

    let mut response = String::new();
    io::stdin().read_line(&mut response)?;
    let response = response.trim();
    if response.is_empty() {
        println!("Skipped — no default model saved.");
        return Ok(());
    }
    let Some(choice) = response
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|index| options.get(index))
    else {
        println!("Unrecognized choice — no default model saved.");
        return Ok(());
    };

    let (entry, model) = choice;
    let kind = entry.provider.kind;
    let base_url = entry.provider.effective_base_url();
    let api_key = api_key_for(kind).unwrap_or_else(|| {
        if kind == "ollama" {
            "ollama".to_string()
        } else {
            String::new()
        }
    });
    runtime::save_user_provider_settings(kind, &api_key, base_url.as_deref(), Some(model))?;
    println!();
    println!(
        "Saved: default model `{model}` via {} (~/.gi/settings.json).",
        entry.provider.label
    );
    println!("New `gi` sessions will use it; override anytime with `gi --model <name>`.");
    Ok(())
}

/// First-run heuristic: no provider env var and no persisted provider/model.
#[must_use]
pub fn is_first_run() -> bool {
    let any_credentialed = detect_available_providers()
        .iter()
        .any(|provider| provider.credentialed);
    if any_credentialed {
        return false;
    }
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(_) => return false,
    };
    match runtime::ConfigLoader::default_for(&cwd).load() {
        Ok(config) => config.provider().kind().is_none() && config.model().is_none(),
        Err(_) => true,
    }
}
