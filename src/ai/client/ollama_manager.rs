// Ollama management — HTTP client for Ollama native REST API.
//
// Uses reqwest against http://localhost:11434/api/* for model management
// operations (list, pull, remove, status). Distinct from the OpenAI-compatible
// chat endpoint already used via async-openai.

use std::time::Duration;

use serde::Deserialize;
use winit::event_loop::EventLoopProxy;

use crate::ai::stream::AppEvent;
use crate::terminal::SessionId;

const OLLAMA_BASE: &str = "http://localhost:11434";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Throttle pull progress to at most one event every 2 seconds.
const PROGRESS_THROTTLE: Duration = Duration::from_secs(2);

// ─── Data Types ─────────────────────────────────────────────────────────────

/// A locally available Ollama model.
#[derive(Debug, Clone, Deserialize)]
pub struct LocalModel {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub modified_at: String,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub details: LocalModelDetails,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LocalModelDetails {
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub parameter_size: String,
    #[serde(default)]
    pub quantization_level: String,
}

// Flatten details into LocalModel for convenience.
impl LocalModel {
    pub fn family(&self) -> &str {
        &self.details.family
    }
    pub fn parameter_size(&self) -> &str {
        &self.details.parameter_size
    }
    pub fn quantization_level(&self) -> &str {
        &self.details.quantization_level
    }
}

/// Pull progress status.
#[derive(Debug, Clone)]
pub enum PullStatus {
    PullingManifest,
    Downloading {
        digest: String,
        total: u64,
        completed: u64,
    },
    Verifying,
    WritingManifest,
    Success,
    Failed(String),
}

/// Pull progress event (for AppEvent routing).
#[derive(Debug, Clone)]
pub struct PullProgress {
    pub model_name: String,
    pub status: PullStatus,
}

#[derive(Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<LocalModel>,
}

#[derive(Deserialize)]
struct VersionResponse {
    version: String,
}

#[derive(Deserialize)]
struct PsModel {
    name: String,
    #[serde(default)]
    size: u64,
}

#[derive(Deserialize)]
struct PsResponse {
    #[serde(default)]
    models: Vec<PsModel>,
}

/// Progress line from NDJSON `POST /api/pull`.
#[derive(Deserialize)]
struct PullLine {
    status: String,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    total: Option<u64>,
    #[serde(default)]
    completed: Option<u64>,
}

// ─── OllamaManager ─────────────────────────────────────────────────────────

/// HTTP client for Ollama management API.
pub struct OllamaManager {
    client: reqwest::Client,
    base_url: String,
}

impl Default for OllamaManager {
    fn default() -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            client,
            base_url: OLLAMA_BASE.to_string(),
        }
    }
}

impl OllamaManager {
    /// Detect whether the Ollama daemon is running.
    /// Returns `Ok(version_string)` if reachable, `Err` otherwise.
    pub async fn detect(&self) -> anyhow::Result<String> {
        let resp = self
            .client
            .get(format!("{}/api/version", self.base_url))
            .timeout(CONNECT_TIMEOUT)
            .send()
            .await?;
        let ver: VersionResponse = resp.json().await?;
        Ok(ver.version)
    }

    /// List all locally available models from `GET /api/tags`.
    pub async fn list_models(&self) -> anyhow::Result<Vec<LocalModel>> {
        let resp = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("Ollama /api/tags returned HTTP {}", resp.status());
        }
        let tags: TagsResponse = resp.json().await?;
        Ok(tags.models)
    }

    /// Pull (download) a model, streaming NDJSON progress.
    ///
    /// Sends throttled `AppEvent::OllamaPullProgress` during download,
    /// then `AppEvent::OllamaPullComplete` on success.
    pub async fn pull_model(
        &self,
        model: &str,
        proxy: EventLoopProxy<AppEvent>,
        session_id: Option<SessionId>,
    ) {
        let url = format!("{}/api/pull", self.base_url);
        let body = serde_json::json!({ "name": model, "stream": true });

        let resp = match self
            .client
            .post(&url)
            .timeout(Duration::from_secs(3600)) // pulls can take a long time
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = proxy.send_event(AppEvent::OllamaInfoMessage {
                    session_id,
                    text: format!("❌ Failed to start pull for `{model}`: {e}"),
                });
                return;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            let _ = proxy.send_event(AppEvent::OllamaInfoMessage {
                session_id,
                text: format!("❌ Ollama pull failed (HTTP {status}): {body_text}"),
            });
            return;
        }

        // Stream NDJSON line-by-line.
        use futures::StreamExt;
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut last_progress = std::time::Instant::now() - PROGRESS_THROTTLE;
        let model_name = model.to_string();

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    let _ = proxy.send_event(AppEvent::OllamaInfoMessage {
                        session_id,
                        text: format!("❌ Pull stream error for `{model_name}`: {e}"),
                    });
                    return;
                }
            };

            buf.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete lines.
            while let Some(newline_pos) = buf.find('\n') {
                let line = buf[..newline_pos].trim().to_string();
                buf = buf[newline_pos + 1..].to_string();

                if line.is_empty() {
                    continue;
                }

                if let Ok(pull_line) = serde_json::from_str::<PullLine>(&line) {
                    // Final success.
                    if pull_line.status == "success" {
                        let _ = proxy.send_event(AppEvent::OllamaInfoMessage {
                            session_id,
                            text: format!("✅ `{model_name}` pulled successfully."),
                        });
                        let _ = proxy.send_event(AppEvent::OllamaPullComplete {
                            model: model_name.clone(),
                        });
                        return;
                    }

                    // Throttled progress update.
                    if last_progress.elapsed() >= PROGRESS_THROTTLE {
                        let text = format_pull_progress(&model_name, &pull_line);
                        let _ = proxy.send_event(AppEvent::OllamaPullProgress { session_id, text });
                        last_progress = std::time::Instant::now();
                    }
                }
            }
        }

        // Stream ended without "success" — treat as error.
        let _ = proxy.send_event(AppEvent::OllamaInfoMessage {
            session_id,
            text: format!("⚠ Pull stream for `{model_name}` ended unexpectedly."),
        });
    }

    /// Remove a model via `DELETE /api/delete`.
    pub async fn remove_model(&self, model: &str) -> anyhow::Result<()> {
        let resp = self
            .client
            .delete(format!("{}/api/delete", self.base_url))
            .json(&serde_json::json!({ "name": model }))
            .send()
            .await?;
        if resp.status().is_success() {
            Ok(())
        } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
            anyhow::bail!("Model `{model}` not found locally")
        } else {
            anyhow::bail!("Ollama delete returned HTTP {}", resp.status())
        }
    }

    /// Show daemon status: version + currently loaded models.
    pub async fn show_status(&self) -> anyhow::Result<String> {
        let version = self
            .detect()
            .await
            .map_err(|e| anyhow::anyhow!("connection refused — Ollama may not be running: {e}"))?;

        let ps: PsResponse = self
            .client
            .get(format!("{}/api/ps", self.base_url))
            .send()
            .await?
            .json()
            .await?;

        let mut lines = vec![format!("**Ollama status**: running (v{version})")];
        if ps.models.is_empty() {
            lines.push("No models currently loaded in memory.".into());
        } else {
            lines.push(format!("**Loaded models** ({}):", ps.models.len()));
            for m in &ps.models {
                let size_mb = m.size / (1024 * 1024);
                lines.push(format!("- `{}` ({}MB in VRAM)", m.name, size_mb));
            }
        }
        Ok(lines.join("\n"))
    }
}

/// Format a pull progress line into a human-readable string.
fn format_pull_progress(model: &str, line: &PullLine) -> String {
    if let (Some(total), Some(completed)) = (line.total, line.completed)
        && total > 0
    {
        let pct = (completed as f64 / total as f64 * 100.0) as u32;
        let completed_mb = completed / (1024 * 1024);
        let total_mb = total / (1024 * 1024);
        return format!("⬇ Pulling `{model}`: {pct}% ({completed_mb}MB / {total_mb}MB)");
    }
    format!("⬇ Pulling `{model}`: {}", line.status)
}
