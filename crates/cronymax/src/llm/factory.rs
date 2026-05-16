//! [`LlmProviderFactory`] trait and [`DefaultLlmProviderFactory`] implementation.
//!
//! Centralises all LLM provider construction so callers never build providers
//! inline. The factory handles:
//!
//! * OpenAI-compatible providers (direct construction).
//! * Anthropic providers (direct construction + capability probe).
//! * Copilot providers (token exchange via [`CopilotTokenCache`], then
//!   OpenAI-compat with `copilot_mode: true`).
//!
//! [`CopilotTokenCache`] is defined in [`super::copilot_cache`].

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{info, warn};

use super::anthropic::{AnthropicConfig, AnthropicProvider};
use super::capabilities::CapabilityResolver;
use super::config::LlmConfig;
use super::copilot_cache::CopilotTokenCache;
use super::openai::{OpenAiConfig, OpenAiProvider};
use super::provider::LlmProvider;

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Factory for constructing [`LlmProvider`] instances from a typed
/// [`LlmConfig`]. Implementations handle provider-specific concerns such as
/// Copilot token exchange.
#[async_trait]
pub trait LlmProviderFactory: Send + Sync {
    /// Build an [`LlmProvider`] for the given configuration.
    async fn build(&self, config: &LlmConfig) -> anyhow::Result<Arc<dyn LlmProvider>>;
}

// ── DefaultLlmProviderFactory ─────────────────────────────────────────────────

/// Default implementation of [`LlmProviderFactory`].
///
/// * `OpenAi` and `Anthropic` — constructed directly.
/// * `Copilot` — token exchange delegated to a shared [`CopilotTokenCache`].
#[derive(Clone)]
pub struct DefaultLlmProviderFactory {
    copilot_cache: Arc<CopilotTokenCache>,
}

impl DefaultLlmProviderFactory {
    pub fn new() -> Self {
        Self {
            copilot_cache: Arc::new(CopilotTokenCache::new()),
        }
    }
}

impl Default for DefaultLlmProviderFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for DefaultLlmProviderFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultLlmProviderFactory").finish()
    }
}

#[async_trait]
impl LlmProviderFactory for DefaultLlmProviderFactory {
    async fn build(&self, config: &LlmConfig) -> anyhow::Result<Arc<dyn LlmProvider>> {
        match config {
            LlmConfig::OpenAi {
                base_url,
                api_key,
                model,
            } => {
                let cfg = OpenAiConfig {
                    base_url: base_url.clone(),
                    api_key: api_key.clone(),
                    default_model: model.clone(),
                    copilot_mode: false,
                    ..Default::default()
                };
                Ok(Arc::new(OpenAiProvider::new(cfg)?))
            }

            LlmConfig::Anthropic {
                base_url,
                api_key,
                model,
            } => {
                // Probe model capabilities (thinking support, etc.).
                let _caps = CapabilityResolver::resolve(model, base_url, api_key.as_deref()).await;
                let cfg = AnthropicConfig {
                    base_url: base_url.clone(),
                    api_key: api_key.clone(),
                    default_model: model.clone(),
                    ..Default::default()
                };
                Ok(Arc::new(AnthropicProvider::new(cfg)?))
            }

            LlmConfig::Copilot {
                github_token,
                model,
                base_url,
            } => {
                // Obtain a valid Copilot API token from the cache (exchanges if
                // expired or not yet cached).
                let copilot_token = match github_token.as_deref() {
                    Some(gt) if !gt.is_empty() => match self.copilot_cache.get_token(gt).await {
                        Ok(ct) => {
                            info!("DefaultLlmProviderFactory: copilot token obtained from cache");
                            Some(ct.token)
                        }
                        Err(e) => {
                            warn!(error = %e, "DefaultLlmProviderFactory: copilot token exchange failed");
                            github_token.clone()
                        }
                    },
                    _ => github_token.clone(),
                };

                let cfg = OpenAiConfig {
                    base_url: base_url.clone(),
                    api_key: copilot_token,
                    default_model: model.clone(),
                    copilot_mode: true,
                    ..Default::default()
                };
                Ok(Arc::new(OpenAiProvider::new(cfg)?))
            }
        }
    }
}
