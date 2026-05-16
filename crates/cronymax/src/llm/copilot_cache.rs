//! Copilot API token cache with background refresh.
//!
//! [`CopilotTokenCache`] avoids the per-agent-spawn cost of calling
//! `exchange_for_copilot_token` (30-second timeout) by caching the most
//! recently exchanged [`CopilotToken`] per GitHub token and reusing it as
//! long as it is not close to expiry.
//!
//! When a cached token is about to expire (within 60 s) a background
//! `tokio::spawn` task wakes up and refreshes it silently so the next
//! `get_token` call returns from cache. The task is cancelled when the
//! cache entry is evicted via the [`tokio::sync::watch`] cancel signal
//! stored alongside each entry.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use tokio::sync::watch;
use tracing::{info, warn};

use super::copilot_auth::{copilot_token_needs_refresh, exchange_for_copilot_token, CopilotToken};

// ── Cache internals ───────────────────────────────────────────────────────────

struct CacheEntry {
    token: CopilotToken,
    /// Dropping the Sender cancels the background refresh task.
    _cancel_tx: watch::Sender<bool>,
}

/// Shared, cheaply-cloneable Copilot token cache.
///
/// Keyed by GitHub Personal Access Token string so different users/workspaces
/// maintain independent caches.
#[derive(Clone, Default)]
pub struct CopilotTokenCache {
    inner: Arc<Mutex<HashMap<String, CacheEntry>>>,
}

impl CopilotTokenCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a valid [`CopilotToken`] for the given GitHub PAT.
    ///
    /// * If a non-expired cached token exists it is returned immediately.
    /// * Otherwise `exchange_for_copilot_token` is called and the result
    ///   is cached along with a background refresh task.
    pub async fn get_token(&self, github_token: &str) -> anyhow::Result<CopilotToken> {
        // Fast path — valid token in cache.
        {
            let guard = self.inner.lock();
            if let Some(entry) = guard.get(github_token) {
                if !copilot_token_needs_refresh(&entry.token) {
                    return Ok(entry.token.clone());
                }
            }
        }

        // Slow path — exchange a fresh token.
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        let token = exchange_for_copilot_token(&http, github_token).await?;
        info!(
            "CopilotTokenCache: token exchanged, expires_at={}",
            token.expires_at
        );

        let (cancel_tx, cancel_rx) = watch::channel(false);
        let entry = CacheEntry {
            token: token.clone(),
            _cancel_tx: cancel_tx,
        };
        self.inner.lock().insert(github_token.to_owned(), entry);

        // Schedule a background refresh before the token expires.
        Self::spawn_refresh(self.inner.clone(), github_token.to_owned(), cancel_rx);
        Ok(token)
    }

    fn spawn_refresh(
        cache: Arc<Mutex<HashMap<String, CacheEntry>>>,
        github_token: String,
        mut cancel_rx: watch::Receiver<bool>,
    ) {
        tokio::spawn(async move {
            // Determine when to wake up.
            let sleep_secs = {
                let guard = cache.lock();
                match guard.get(&github_token) {
                    Some(e) => {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        e.token.expires_at.saturating_sub(60).saturating_sub(now)
                    }
                    None => return,
                }
            };

            // Sleep until refresh window, or exit early on cancel.
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(sleep_secs)) => {},
                _ = cancel_rx.changed() => {
                    return;
                }
            }

            // Re-exchange.
            let http = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default();
            match exchange_for_copilot_token(&http, &github_token).await {
                Ok(new_token) => {
                    info!(
                        "CopilotTokenCache: background refresh succeeded, expires_at={}",
                        new_token.expires_at
                    );
                    let (cancel_tx, next_cancel_rx) = watch::channel(false);
                    let mut guard = cache.lock();
                    guard.insert(
                        github_token.clone(),
                        CacheEntry {
                            token: new_token,
                            _cancel_tx: cancel_tx,
                        },
                    );
                    drop(guard);
                    // Chain the next refresh.
                    Self::spawn_refresh(cache, github_token, next_cancel_rx);
                }
                Err(e) => {
                    warn!(error = %e, "CopilotTokenCache: background refresh failed");
                }
            }
        });
    }
}

impl std::fmt::Debug for CopilotTokenCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CopilotTokenCache").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::watch;

    fn make_non_expired_token() -> CopilotToken {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        CopilotToken {
            token: "mock-copilot-token".into(),
            // Expires 10 minutes from now — well within the non-refresh window.
            expires_at: now + 600,
        }
    }

    /// 11.3 — Warm-cache path: `get_token` returns the cached value without
    /// calling `exchange_for_copilot_token` (which would require HTTP).
    #[tokio::test]
    async fn warm_cache_returns_cached_token_without_http() {
        let cache = CopilotTokenCache::new();

        // Manually seed the cache with a non-expired token.
        let token = make_non_expired_token();
        let (cancel_tx, _cancel_rx) = watch::channel(false);
        {
            let mut inner = cache.inner.lock();
            inner.insert(
                "my-github-token".to_owned(),
                CacheEntry {
                    token: token.clone(),
                    _cancel_tx: cancel_tx,
                },
            );
        }

        // get_token should return from cache without making any HTTP call.
        // (If it tried to call exchange_for_copilot_token it would fail because
        // there is no real HTTP server — confirming the warm path was taken.)
        let result = cache.get_token("my-github-token").await;
        let returned = result.expect("expected Ok from warm cache");

        assert_eq!(
            returned.token, token.token,
            "warm-cache path should return the same token that was seeded"
        );
        assert_eq!(
            returned.expires_at, token.expires_at,
            "returned expires_at should match the cached value"
        );
    }
}
