//! First-run / `gi models` provider + model discovery.
//!
//! `detect_available_providers` reads the environment to report which providers
//! are configured (without exposing secrets), and the `fetch_*` helpers query a
//! provider's live model list when one is reachable. Callers fall back to the
//! per-provider `static_models` when a live query fails.

use std::time::Duration;

/// How a provider exposes a live model list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListStyle {
    /// Ollama native `GET {host}/api/tags`.
    OllamaTags,
    /// OpenAI-compatible `GET {base}/models`.
    OpenAiModels,
}

/// A provider detected from the environment for the model-discovery flow.
/// Carries no secrets — only whether credentials were present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedProvider {
    /// Stable kind token: `anthropic` | `openai` | `xai` | `dashscope` | `ollama`.
    pub kind: &'static str,
    /// Human-friendly label.
    pub label: &'static str,
    /// Effective base URL when known (env override), else `None` (use the default).
    pub base_url: Option<String>,
    /// Whether an API key / host was found in the environment.
    pub credentialed: bool,
    /// Built-in models to suggest when a live list isn't available.
    pub static_models: Vec<&'static str>,
    /// Live model-list endpoint style, if the provider supports one.
    pub list_style: Option<ListStyle>,
}

impl DetectedProvider {
    /// Base URL to use for live model listing (env override or built-in default).
    #[must_use]
    pub fn effective_base_url(&self) -> Option<String> {
        self.base_url
            .clone()
            .or_else(|| default_base_for(self.kind).map(ToString::to_string))
    }
}

fn default_base_for(kind: &str) -> Option<&'static str> {
    match kind {
        "openai" => Some("https://api.openai.com/v1"),
        "xai" => Some("https://api.x.ai/v1"),
        "dashscope" => Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
        "sakana" => Some("https://api.sakana.ai/v1"),
        "kimi" => Some("https://api.moonshot.ai/v1"),
        "glm" => Some("https://open.bigmodel.cn/api/paas/v4"),
        "ollama" => Some("http://127.0.0.1:11434"),
        _ => None,
    }
}

/// True when any of the candidate API-key env vars is set and non-empty.
fn any_key(candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|key| super::openai_compat::has_api_key(key))
}

fn first_base_url(candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
}

/// Detect every known provider, marking which are configured in the environment.
/// All providers are returned (with `credentialed` flags) so the flow can show
/// the full picture; the default Anthropic option is always included.
#[must_use]
pub fn detect_available_providers() -> Vec<DetectedProvider> {
    let anthropic_credentialed = super::anthropic::has_auth_from_env_or_saved().unwrap_or(false);
    vec![
        DetectedProvider {
            kind: "anthropic",
            label: "Anthropic",
            base_url: std::env::var("ANTHROPIC_BASE_URL").ok(),
            credentialed: anthropic_credentialed,
            static_models: vec!["opus", "sonnet", "haiku"],
            list_style: None,
        },
        DetectedProvider {
            kind: "openai",
            label: "OpenAI",
            base_url: std::env::var("OPENAI_BASE_URL").ok(),
            credentialed: super::openai_compat::has_api_key("OPENAI_API_KEY"),
            static_models: vec!["gpt-4o", "gpt-4o-mini", "o3-mini"],
            list_style: Some(ListStyle::OpenAiModels),
        },
        DetectedProvider {
            kind: "xai",
            label: "xAI",
            base_url: std::env::var("XAI_BASE_URL").ok(),
            credentialed: super::openai_compat::has_api_key("XAI_API_KEY"),
            static_models: vec!["grok-3", "grok-3-mini"],
            list_style: Some(ListStyle::OpenAiModels),
        },
        DetectedProvider {
            kind: "dashscope",
            label: "DashScope (Qwen)",
            base_url: std::env::var("DASHSCOPE_BASE_URL").ok(),
            credentialed: super::openai_compat::has_api_key("DASHSCOPE_API_KEY"),
            static_models: vec!["qwen-max", "qwen-plus"],
            list_style: Some(ListStyle::OpenAiModels),
        },
        DetectedProvider {
            kind: "sakana",
            label: "Sakana AI",
            base_url: first_base_url(&["SAKANA_BASE_URL", "SAKANA_AI_BASE_URL"]),
            credentialed: any_key(&["SAKANA_API_KEY", "SAKANA_AI_API_KEY"]),
            static_models: vec![],
            list_style: Some(ListStyle::OpenAiModels),
        },
        DetectedProvider {
            kind: "kimi",
            label: "Kimi (Moonshot)",
            base_url: first_base_url(&["KIMI_BASE_URL", "MOONSHOT_BASE_URL"]),
            credentialed: any_key(&["KIMI_API_KEY", "MOONSHOT_API_KEY"]),
            static_models: vec!["kimi-latest", "moonshot-v1-128k", "moonshot-v1-8k"],
            list_style: Some(ListStyle::OpenAiModels),
        },
        DetectedProvider {
            kind: "glm",
            label: "GLM (Zhipu)",
            base_url: first_base_url(&["GLM_BASE_URL", "ZHIPUAI_BASE_URL"]),
            credentialed: any_key(&["GLM_API_KEY", "ZHIPUAI_API_KEY"]),
            static_models: vec!["glm-4-plus", "glm-4-flash", "glm-4-air"],
            list_style: Some(ListStyle::OpenAiModels),
        },
        DetectedProvider {
            kind: "ollama",
            label: "Ollama (local)",
            base_url: std::env::var("OLLAMA_HOST").ok(),
            credentialed: std::env::var_os("OLLAMA_HOST").is_some(),
            static_models: vec![],
            list_style: Some(ListStyle::OllamaTags),
        },
    ]
}

async fn http_get_json(url: &str, bearer: Option<&str>) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.get(url);
    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| error.to_string())
}

/// Query an Ollama host's installed models via `GET {host}/api/tags`.
///
/// # Errors
/// Returns the transport/HTTP error string when the host is unreachable.
pub async fn fetch_ollama_models(host: &str) -> Result<Vec<String>, String> {
    let base = host.trim_end_matches('/').trim_end_matches("/v1");
    let base = base.trim_end_matches('/');
    let url = format!("{base}/api/tags");
    let json = http_get_json(&url, None).await?;
    Ok(json["models"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model["name"].as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default())
}

/// Query an OpenAI-compatible endpoint's catalog via `GET {base_url}/models`.
///
/// # Errors
/// Returns the transport/HTTP error string when the endpoint is unreachable.
pub async fn fetch_openai_compat_models(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>, String> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/models");
    let json = http_get_json(&url, api_key).await?;
    Ok(json["data"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model["id"].as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_all_known_providers() {
        let providers = detect_available_providers();
        let kinds: Vec<_> = providers.iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            [
                "anthropic",
                "openai",
                "xai",
                "dashscope",
                "sakana",
                "kimi",
                "glm",
                "ollama"
            ]
        );
    }

    #[test]
    fn ollama_effective_base_falls_back_to_localhost() {
        let ollama = DetectedProvider {
            kind: "ollama",
            label: "Ollama (local)",
            base_url: None,
            credentialed: false,
            static_models: vec![],
            list_style: Some(ListStyle::OllamaTags),
        };
        assert_eq!(
            ollama.effective_base_url().as_deref(),
            Some("http://127.0.0.1:11434")
        );
    }

    #[test]
    fn parses_ollama_tags_shape() {
        // Mirrors the parsing in fetch_ollama_models without a live server.
        let json: serde_json::Value = serde_json::from_str(
            r#"{"models":[{"name":"mistral-small3.2:latest"},{"name":"gemma4:12b"}]}"#,
        )
        .unwrap();
        let names: Vec<_> = json["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["name"].as_str())
            .collect();
        assert_eq!(names, ["mistral-small3.2:latest", "gemma4:12b"]);
    }

    #[test]
    fn parses_openai_models_shape() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"data":[{"id":"gpt-4o"},{"id":"o3-mini"}]}"#).unwrap();
        let ids: Vec<_> = json["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["id"].as_str())
            .collect();
        assert_eq!(ids, ["gpt-4o", "o3-mini"]);
    }
}
