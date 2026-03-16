//! LLM provider auto-detection for lifecycle initialization.

use crate::services::secret::SecretStore;

/// Auto-detect the best available LLM provider from keychain + env vars.
pub(super) fn detect_provider(
    secret_store: &SecretStore,
) -> (
    crate::ai::client::LlmProvider,
    Option<String>,
    Option<String>,
) {
    let has_gh = secret_store
        .resolve(
            &crate::services::secret::provider_api_key("copilot"),
            Some("GH_TOKEN"),
            &crate::services::secret::SecretStorage::Auto,
        )
        .ok()
        .flatten()
        .is_some();
    let has_openai = secret_store
        .resolve(
            &crate::services::secret::provider_api_key("openai"),
            Some("OPENAI_API_KEY"),
            &crate::services::secret::SecretStorage::Auto,
        )
        .ok()
        .flatten()
        .is_some();
    let has_anthropic = secret_store
        .resolve(
            &crate::services::secret::provider_api_key("anthropic"),
            Some("ANTHROPIC_API_KEY"),
            &crate::services::secret::SecretStorage::Auto,
        )
        .ok()
        .flatten()
        .is_some();

    if has_gh {
        (
            crate::ai::client::LlmProvider::Copilot,
            Some("https://models.inference.ai.azure.com".into()),
            Some("GH_TOKEN".into()),
        )
    } else if has_openai {
        (
            crate::ai::client::LlmProvider::OpenAI,
            None,
            Some("OPENAI_API_KEY".into()),
        )
    } else if has_anthropic {
        (
            crate::ai::client::LlmProvider::Anthropic,
            Some("https://api.anthropic.com/v1".into()),
            Some("ANTHROPIC_API_KEY".into()),
        )
    } else {
        (
            crate::ai::client::LlmProvider::OpenAI,
            None,
            Some("OPENAI_API_KEY".into()),
        )
    }
}
