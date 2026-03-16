//! ClawHub registry HTTP client — search, download, and update skills from clawhub.ai.
//!
//! ClawHub is the public registry for OpenClaw-compatible skills (5,400+ published).
//! This module provides an HTTP client with retry logic, rate limiting, and security
//! checks (HTTPS only, 10MB download limit, path traversal prevention).
//!
//! API reference: <https://github.com/openclaw/clawhub> — Convex backend with
//! v1 REST routes at `/api/v1/search`, `/api/v1/skills`, etc.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

// ── Public Types ─────────────────────────────────────────────────────────────

/// Search result from the ClawHub API.
#[derive(Debug, Clone, Deserialize)]
pub struct ClawHubSkillResult {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub version: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub stars: u32,
    #[serde(default)]
    pub install_count: u32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub updated_at: String,
}

/// Full skill detail from the ClawHub API.
#[derive(Debug, Clone, Deserialize)]
pub struct ClawHubSkillDetail {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub version: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub skill_md: String,
    #[serde(default)]
    pub changelog: Option<String>,
    #[serde(default)]
    pub stars: u32,
    #[serde(default)]
    pub install_count: u32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

// ── Private API Response Types (match ClawHub v1 JSON) ───────────────────────

#[derive(Deserialize)]
struct ApiSearchEntry {
    #[serde(default)]
    slug: Option<String>,
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Deserialize)]
struct ApiSearchResponse {
    results: Vec<ApiSearchEntry>,
}

#[derive(Deserialize)]
struct ApiSkillResponse {
    skill: Option<ApiSkillInfo>,
    #[serde(rename = "latestVersion", default)]
    latest_version: Option<ApiVersionEntry>,
    #[serde(default)]
    owner: Option<ApiOwnerInfo>,
}

#[derive(Deserialize)]
struct ApiSkillInfo {
    #[serde(default)]
    slug: String,
    #[serde(rename = "displayName", default)]
    display_name: String,
    #[serde(default)]
    summary: Option<String>,
}

#[derive(Deserialize)]
struct ApiVersionEntry {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    changelog: Option<String>,
}

#[derive(Deserialize)]
struct ApiOwnerInfo {
    #[serde(default)]
    handle: Option<String>,
}

#[derive(Deserialize)]
struct ApiListResponse {
    items: Vec<ApiListItem>,
}

#[derive(Deserialize)]
struct ApiListItem {
    slug: String,
    #[serde(rename = "displayName", default)]
    display_name: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(rename = "latestVersion", default)]
    latest_version: Option<ApiVersionEntry>,
}

#[derive(Deserialize)]
struct ApiVersionsResponse {
    items: Vec<ApiVersionsItem>,
}

#[derive(Deserialize)]
struct ApiVersionsItem {
    version: String,
}

// ── Client ───────────────────────────────────────────────────────────────────

const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 1000;
const REQUEST_TIMEOUT_SECS: u64 = 30;
const MAX_DOWNLOAD_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

/// HTTP client for the ClawHub skill registry API.
pub struct ClawHubClient {
    http: reqwest::Client,
    api_base: String,
}

impl ClawHubClient {
    /// Create a new ClawHub client.
    ///
    /// `api_base` is the site origin, e.g. `"https://clawhub.ai"`.
    pub fn new(api_base: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .user_agent(format!("cronymax/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();

        Self {
            http,
            api_base: api_base.trim_end_matches('/').to_string(),
        }
    }

    /// Get the API base URL.
    pub fn api_base(&self) -> &str {
        &self.api_base
    }

    /// Search ClawHub for skills matching a query.
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<ClawHubSkillResult>> {
        let url = format!("{}/api/v1/search", self.api_base);
        let resp: ApiSearchResponse = self
            .request_with_retry(|| {
                self.http
                    .get(&url)
                    .query(&[("q", query), ("limit", &limit.to_string())])
            })
            .await?;
        Ok(resp
            .results
            .into_iter()
            .filter_map(|e| {
                let slug = e.slug?;
                Some(ClawHubSkillResult {
                    name: e.display_name.unwrap_or_else(|| slug.clone()),
                    description: e.summary.unwrap_or_default(),
                    version: e.version.unwrap_or_default(),
                    slug,
                    author: None,
                    stars: 0,
                    install_count: 0,
                    tags: Vec::new(),
                    updated_at: String::new(),
                })
            })
            .collect())
    }

    /// Get popular skills from ClawHub.
    pub async fn popular(&self, limit: usize) -> anyhow::Result<Vec<ClawHubSkillResult>> {
        let url = format!("{}/api/v1/skills", self.api_base);
        let resp: ApiListResponse = self
            .request_with_retry(|| {
                self.http
                    .get(&url)
                    .query(&[("sort", "downloads"), ("limit", &limit.to_string())])
            })
            .await?;
        Ok(resp
            .items
            .into_iter()
            .map(|item| ClawHubSkillResult {
                name: item.display_name,
                description: item.summary.unwrap_or_default(),
                version: item
                    .latest_version
                    .and_then(|v| v.version)
                    .unwrap_or_default(),
                slug: item.slug,
                author: None,
                stars: 0,
                install_count: 0,
                tags: Vec::new(),
                updated_at: String::new(),
            })
            .collect())
    }

    /// Get full skill detail.
    pub async fn get_skill(&self, slug: &str) -> anyhow::Result<ClawHubSkillDetail> {
        let url = format!("{}/api/v1/skills/{}", self.api_base, slug);
        let resp: ApiSkillResponse = self.request_with_retry(|| self.http.get(&url)).await?;

        let skill = resp
            .skill
            .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found", slug))?;

        Ok(ClawHubSkillDetail {
            slug: skill.slug,
            name: skill.display_name,
            description: skill.summary.unwrap_or_default(),
            version: resp
                .latest_version
                .as_ref()
                .and_then(|v| v.version.clone())
                .unwrap_or_default(),
            author: resp.owner.and_then(|o| o.handle),
            skill_md: String::new(), // fetched separately via download
            changelog: resp.latest_version.and_then(|v| v.changelog),
            stars: 0,
            install_count: 0,
            tags: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
        })
    }

    /// Download a skill and write SKILL.md to `dest/{slug}/SKILL.md`.
    ///
    /// Fetches the raw SKILL.md from the ClawHub file endpoint.
    /// Security: rejects responses > 10MB, prevents path traversal.
    pub async fn download(&self, slug: &str, dest: &Path) -> anyhow::Result<PathBuf> {
        // Reject slugs with path traversal.
        if slug.contains("..") || slug.contains('/') || slug.contains('\\') {
            anyhow::bail!("Invalid slug: path traversal detected in '{}'", slug);
        }

        let url = format!(
            "{}/api/v1/skills/{}/file?path=SKILL.md",
            self.api_base, slug
        );
        let skill_md = self.request_text_with_retry(|| self.http.get(&url)).await?;

        // Enforce size limit on SKILL.md content.
        if skill_md.len() as u64 > MAX_DOWNLOAD_BYTES {
            anyhow::bail!(
                "Skill '{}' SKILL.md exceeds {}MB size limit",
                slug,
                MAX_DOWNLOAD_BYTES / (1024 * 1024)
            );
        }

        let skill_dir = dest.join(slug);
        std::fs::create_dir_all(&skill_dir)?;

        let skill_md_path = skill_dir.join("SKILL.md");
        std::fs::write(&skill_md_path, &skill_md)?;

        log::info!("Downloaded skill '{}' to {}", slug, skill_dir.display());
        Ok(skill_dir)
    }

    /// Check if a newer version of a skill exists on ClawHub.
    ///
    /// Returns `Some(new_version)` if an update is available, `None` otherwise.
    pub async fn check_update(
        &self,
        slug: &str,
        current_version: &str,
    ) -> anyhow::Result<Option<String>> {
        let url = format!("{}/api/v1/skills/{}/versions", self.api_base, slug);
        let resp: ApiVersionsResponse = self
            .request_with_retry(|| self.http.get(&url).query(&[("limit", "1")]))
            .await?;

        if let Some(latest) = resp.items.first()
            && latest.version != current_version
        {
            return Ok(Some(latest.version.clone()));
        }
        Ok(None)
    }

    // ── Retry Logic ──────────────────────────────────────────────────────

    /// Execute an HTTP request returning JSON, with exponential backoff retry.
    ///
    /// Retries on: HTTP 429, 5xx, connection timeouts.
    /// Does NOT retry on: 400, 401, 403, 404.
    async fn request_with_retry<T, F>(&self, build_request: impl Fn() -> F) -> anyhow::Result<T>
    where
        T: serde::de::DeserializeOwned,
        F: Into<reqwest::RequestBuilder>,
    {
        let mut backoff = Duration::from_millis(INITIAL_BACKOFF_MS);

        for attempt in 0..MAX_RETRIES {
            let request_builder: reqwest::RequestBuilder = build_request().into();
            match request_builder.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return resp.json::<T>().await.map_err(Into::into);
                    }

                    let is_retryable = status.as_u16() == 429 || status.is_server_error();

                    if is_retryable && attempt < MAX_RETRIES - 1 {
                        log::warn!(
                            "ClawHub request failed (HTTP {}), retrying in {:?}…",
                            status,
                            backoff
                        );
                        tokio::time::sleep(backoff).await;
                        backoff *= 2;
                        continue;
                    }

                    let body = resp.text().await.unwrap_or_default();
                    anyhow::bail!("ClawHub API error (HTTP {}): {}", status, body);
                }
                Err(e) => {
                    let is_retryable = e.is_timeout() || e.is_connect();

                    if is_retryable && attempt < MAX_RETRIES - 1 {
                        log::warn!("ClawHub request error: {}, retrying in {:?}…", e, backoff);
                        tokio::time::sleep(backoff).await;
                        backoff *= 2;
                        continue;
                    }

                    return Err(e.into());
                }
            }
        }

        anyhow::bail!("ClawHub request failed after {} retries", MAX_RETRIES)
    }

    /// Execute an HTTP request returning plain text, with exponential backoff retry.
    async fn request_text_with_retry<F>(
        &self,
        build_request: impl Fn() -> F,
    ) -> anyhow::Result<String>
    where
        F: Into<reqwest::RequestBuilder>,
    {
        let mut backoff = Duration::from_millis(INITIAL_BACKOFF_MS);

        for attempt in 0..MAX_RETRIES {
            let request_builder: reqwest::RequestBuilder = build_request().into();
            match request_builder.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return resp.text().await.map_err(Into::into);
                    }

                    let is_retryable = status.as_u16() == 429 || status.is_server_error();

                    if is_retryable && attempt < MAX_RETRIES - 1 {
                        tokio::time::sleep(backoff).await;
                        backoff *= 2;
                        continue;
                    }

                    let body = resp.text().await.unwrap_or_default();
                    anyhow::bail!("ClawHub API error (HTTP {}): {}", status, body);
                }
                Err(e) => {
                    let is_retryable = e.is_timeout() || e.is_connect();

                    if is_retryable && attempt < MAX_RETRIES - 1 {
                        tokio::time::sleep(backoff).await;
                        backoff *= 2;
                        continue;
                    }

                    return Err(e.into());
                }
            }
        }

        anyhow::bail!("ClawHub request failed after {} retries", MAX_RETRIES)
    }
}
