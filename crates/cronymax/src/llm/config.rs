//! Typed LLM provider configuration.
//!
//! Replaces the raw `(provider_kind, base_url, api_key, model)` string tuple
//! previously threaded through `FlowRunContext`. Each variant carries only the
//! fields required by its provider; the [`LlmProviderFactory`] matches on the
//! variant to select the correct construction path.

/// Typed LLM provider configuration passed to [`LlmProviderFactory::build`].
#[derive(Clone, Debug)]
pub enum LlmConfig {
    /// OpenAI-compatible chat completions endpoint (the default).
    OpenAi {
        /// Base URL, e.g. `https://api.openai.com/v1`.
        base_url: String,
        api_key: Option<String>,
        model: String,
    },
    /// Native Anthropic Messages API.
    Anthropic {
        /// Base URL, e.g. `https://api.anthropic.com`.
        base_url: String,
        api_key: Option<String>,
        model: String,
    },
    /// GitHub Copilot — requires a GitHub Personal Access Token which the
    /// factory exchanges for a short-lived Copilot API token.
    Copilot {
        /// GitHub Personal Access Token used to obtain a Copilot API token.
        github_token: Option<String>,
        model: String,
        /// Copilot API endpoint, typically `https://api.githubcopilot.com/v1`.
        base_url: String,
    },
}

impl LlmConfig {
    /// Build an `LlmConfig` from the raw string fields extracted from the JSON
    /// payload. Used by `RuntimeHandler` while the old code-path is still active.
    pub fn from_payload_fields(
        provider_kind: &str,
        base_url: String,
        api_key: Option<String>,
        model: String,
    ) -> Self {
        match provider_kind {
            "anthropic" => LlmConfig::Anthropic {
                base_url,
                api_key,
                model,
            },
            "github_copilot" => LlmConfig::Copilot {
                github_token: api_key,
                model,
                base_url,
            },
            _ => LlmConfig::OpenAi {
                base_url,
                api_key,
                model,
            },
        }
    }
}
