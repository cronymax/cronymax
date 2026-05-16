//! Model capability probing and caching.
//!
//! On first use, [`CapabilityResolver::resolve`] attempts a `GET {base_url}/v1/models`
//! call to discover thinking/reasoning capabilities. Results are cached to
//! `~/.cronymax/model-caps-<sha256(base_url)>.json` with a 1-hour TTL. On any
//! failure the built-in static table is used as a fallback.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, warn};

use super::messages::ThinkingConfig;

// ── ThinkingSupport ──────────────────────────────────────────────────────────

/// Per-model thinking capability declaration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThinkingSupport {
    /// Model does not support thinking/reasoning output.
    None,
    /// Model supports adaptive thinking (Anthropic claude-*). Budget bounds
    /// come from the model metadata.
    Adaptive { min_budget: u32, max_budget: u32 },
    /// Model supports a fixed token budget (non-adaptive providers).
    Budget { min: u32, max: u32 },
    /// Model supports reasoning effort levels (OpenAI o-series).
    ReasoningEffort { levels: Vec<String> },
}

/// Resolved capability descriptor for one model.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub id: String,
    pub thinking: ThinkingSupport,
}

impl ModelCapabilities {
    /// Derive the `ThinkingConfig` to attach to an [`super::messages::LlmRequest`]
    /// from these capabilities. Returns `None` when the model does not
    /// support thinking.
    pub fn thinking_config(&self) -> Option<ThinkingConfig> {
        match &self.thinking {
            ThinkingSupport::None => None,
            ThinkingSupport::Adaptive { .. } => Some(ThinkingConfig::Adaptive {
                summarized: true,
                effort: None,
            }),
            ThinkingSupport::Budget { min, max } => {
                // Cap at min(max, 16000) but keep at least min.
                let budget = (*max).min(16_000).max(*min);
                Some(ThinkingConfig::Budget {
                    budget_tokens: budget,
                })
            }
            ThinkingSupport::ReasoningEffort { levels } => {
                let effort = levels
                    .iter()
                    .find(|l| l.as_str() == "medium")
                    .or_else(|| levels.get(levels.len() / 2))
                    .cloned()
                    .unwrap_or_else(|| "medium".to_string());
                Some(ThinkingConfig::ReasoningEffort { effort })
            }
        }
    }
}

// ── Static fallback table ─────────────────────────────────────────────────────

/// Match model id against the built-in capability table. Called when dynamic
/// probing fails or the model is not found in the `/models` response.
fn static_capabilities(model_id: &str) -> ModelCapabilities {
    let thinking = if model_id.starts_with("claude-") {
        ThinkingSupport::Adaptive {
            min_budget: 1024,
            max_budget: 32_000,
        }
    } else if model_id.starts_with("o1-")
        || model_id.starts_with("o3-")
        || model_id.starts_with("o4-")
        || model_id == "o1"
        || model_id == "o3"
    {
        ThinkingSupport::ReasoningEffort {
            levels: vec!["low".into(), "medium".into(), "high".into()],
        }
    } else if model_id.starts_with("gemini-2.") {
        ThinkingSupport::Adaptive {
            min_budget: 512,
            max_budget: 24_576,
        }
    } else {
        ThinkingSupport::None
    };
    ModelCapabilities {
        id: model_id.to_owned(),
        thinking,
    }
}

// ── Cache helpers ────────────────────────────────────────────────────────────

const CACHE_TTL: Duration = Duration::from_secs(3600); // 1 hour

fn cache_path(base_url: &str) -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| anyhow::anyhow!("HOME not set"))?;
    let dir = home.join(".cronymax");
    // Use the first 16 hex chars of SHA-256(base_url) as a compact suffix.
    let mut hasher = Sha256::new();
    hasher.update(base_url.as_bytes());
    let hash_hex = format!("{:x}", hasher.finalize());
    let filename = format!("model-caps-{}.json", &hash_hex[..16]);
    Ok(dir.join(filename))
}

/// Load cached capabilities for `model_id` if the cache exists and is fresh.
fn load_cache(base_url: &str, model_id: &str) -> Option<ModelCapabilities> {
    let path = cache_path(base_url).ok()?;
    let meta = std::fs::metadata(&path).ok()?;
    let age = meta.modified().ok()?.elapsed().ok()?;
    if age > CACHE_TTL {
        debug!("capabilities cache expired for {base_url}");
        return None;
    }
    let raw = std::fs::read_to_string(&path).ok()?;
    let map: std::collections::HashMap<String, ModelCapabilities> =
        serde_json::from_str(&raw).ok()?;
    map.get(model_id).cloned()
}

/// Persist capability for `model_id` into the cache file (merge with existing
/// entries for the same base_url so other models aren't evicted).
fn save_cache(base_url: &str, caps: &ModelCapabilities) {
    let Ok(path) = cache_path(base_url) else {
        return;
    };
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
    }
    // Read existing cache (best-effort) and merge.
    let mut map: std::collections::HashMap<String, ModelCapabilities> = path
        .exists()
        .then(|| {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
        })
        .flatten()
        .unwrap_or_default();
    // Touch modification time by writing the file fresh even if the entry exists.
    map.insert(caps.id.clone(), caps.clone());
    if let Ok(json) = serde_json::to_string_pretty(&map) {
        let _ = std::fs::write(&path, json);
    }
}

// ── Dynamic probe ────────────────────────────────────────────────────────────

/// Wire shape for the `/models` response (subset we care about).
#[derive(Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    capabilities: Option<ModelEntryCapabilities>,
}

#[derive(Deserialize)]
struct ModelEntryCapabilities {
    #[serde(default)]
    supports: Option<ModelSupports>,
}

#[derive(Deserialize)]
struct ModelSupports {
    #[serde(default)]
    adaptive_thinking: bool,
    #[serde(default)]
    min_thinking_budget: Option<u32>,
    #[serde(default)]
    max_thinking_budget: Option<u32>,
    #[serde(default)]
    reasoning_effort: Option<Vec<String>>,
}

/// Try `GET {base_url}/v1/models` and find `model_id` in the response. Returns
/// `None` on any failure so the caller can fall back to the static table.
async fn probe_models(
    http: &reqwest::Client,
    base_url: &str,
    model_id: &str,
    api_key: Option<&str>,
) -> Option<ModelCapabilities> {
    // Normalize base_url so a trailing `/v1` (legacy convention) doesn't
    // produce `/v1/v1/models`.
    let base = base_url.trim_end_matches('/').trim_end_matches("/v1");
    let url = format!("{base}/v1/models");
    let mut req = http.get(&url).header("accept", "application/json");
    if let Some(key) = api_key {
        req = req.bearer_auth(key);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        debug!(url, status = %resp.status(), "capabilities probe: non-success status");
        return None;
    }
    let body: ModelsResponse = resp.json().await.ok()?;
    let entry = body.data.into_iter().find(|m| m.id == model_id)?;
    let sup = entry.capabilities.as_ref()?.supports.as_ref()?;

    let thinking = if sup.adaptive_thinking {
        ThinkingSupport::Adaptive {
            min_budget: sup.min_thinking_budget.unwrap_or(1024),
            max_budget: sup.max_thinking_budget.unwrap_or(32_000),
        }
    } else if let Some(min) = sup.min_thinking_budget {
        ThinkingSupport::Budget {
            min,
            max: sup.max_thinking_budget.unwrap_or(32_000),
        }
    } else if let Some(levels) = sup.reasoning_effort.clone() {
        if levels.is_empty() {
            return None; // no useful info
        }
        ThinkingSupport::ReasoningEffort { levels }
    } else {
        return None; // no capability fields present
    };

    Some(ModelCapabilities {
        id: model_id.to_owned(),
        thinking,
    })
}

// ── CapabilityResolver ───────────────────────────────────────────────────────

/// Resolves per-model thinking capabilities via dynamic probe + cache + static
/// fallback. Construct once; cheap to call repeatedly (cache prevents HTTP spam).
pub struct CapabilityResolver;

impl CapabilityResolver {
    /// Resolve capabilities for `model_id` against `base_url`.
    ///
    /// Order:
    /// 1. In-process cache (1-hour TTL at `~/.cronymax/model-caps-*.json`)
    /// 2. Dynamic `GET {base_url}/v1/models` probe
    /// 3. Static fallback table
    pub async fn resolve(
        model_id: &str,
        base_url: &str,
        api_key: Option<&str>,
    ) -> ModelCapabilities {
        // 1. Cache
        if let Some(cached) = load_cache(base_url, model_id) {
            debug!(model_id, "capabilities: cache hit");
            return cached;
        }

        // 2. Dynamic probe
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        if let Some(caps) = probe_models(&http, base_url, model_id, api_key).await {
            debug!(model_id, ?caps.thinking, "capabilities: probe succeeded");
            save_cache(base_url, &caps);
            return caps;
        }

        warn!(
            model_id,
            base_url, "capabilities: probe failed, using static table"
        );

        // 3. Static fallback
        let caps = static_capabilities(model_id);
        // Cache the static result so we don't retry the probe on every turn.
        save_cache(base_url, &caps);
        caps
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_claude_is_adaptive() {
        let caps = static_capabilities("claude-3-5-sonnet-20241022");
        assert!(matches!(caps.thinking, ThinkingSupport::Adaptive { .. }));
        assert!(caps.thinking_config().is_some());
    }

    #[test]
    fn static_o4_mini_is_reasoning_effort() {
        let caps = static_capabilities("o4-mini");
        assert!(matches!(
            caps.thinking,
            ThinkingSupport::ReasoningEffort { .. }
        ));
        if let Some(ThinkingConfig::ReasoningEffort { effort }) = caps.thinking_config() {
            assert_eq!(effort, "medium");
        } else {
            panic!("expected ReasoningEffort config");
        }
    }

    #[test]
    fn static_unknown_model_is_none() {
        let caps = static_capabilities("gpt-4o");
        assert!(matches!(caps.thinking, ThinkingSupport::None));
        assert!(caps.thinking_config().is_none());
    }

    #[test]
    fn budget_config_is_capped_at_16k() {
        let caps = ModelCapabilities {
            id: "test".into(),
            thinking: ThinkingSupport::Budget {
                min: 1024,
                max: 32_000,
            },
        };
        if let Some(ThinkingConfig::Budget { budget_tokens }) = caps.thinking_config() {
            assert_eq!(budget_tokens, 16_000);
        } else {
            panic!("expected Budget config");
        }
    }

    #[test]
    fn adaptive_config_has_summarized_true() {
        let caps = ModelCapabilities {
            id: "claude-opus-4".into(),
            thinking: ThinkingSupport::Adaptive {
                min_budget: 1024,
                max_budget: 32_000,
            },
        };
        if let Some(ThinkingConfig::Adaptive { summarized, effort }) = caps.thinking_config() {
            assert!(summarized);
            assert!(effort.is_none());
        } else {
            panic!("expected Adaptive config");
        }
    }
}
