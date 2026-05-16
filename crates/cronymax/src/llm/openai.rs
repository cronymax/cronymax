//! OpenAI-chat-compatible streaming client.
//!
//! Implements the same wire shape the renderer used in
//! `web/src/agent_runtime/llm.js`:
//!
//! * `POST {base_url}/v1/chat/completions`
//! * Body: `{ model, messages, stream: true, tools?, tool_choice: "auto" }`
//! * Response: `text/event-stream`, lines prefixed `data: `, each
//!   payload is a JSON `ChatCompletionChunk`.
//!
//! Tool-call deltas come through as
//! `choices[0].delta.tool_calls[{index, id?, function: { name?, arguments? }}]`.
//! We forward each chunk as an [`LlmEvent::ToolCallDelta`]; the agent
//! loop is responsible for splicing them by `index`.

use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Serialize;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::stream::UnboundedReceiverStream;

use super::messages::{FinishReason, LlmRequest, ToolDef};
use super::provider::{LlmEvent, LlmProvider, LlmStream};

use crate::llm::messages::ThinkingConfig;

/// Per-instance configuration. `model` is the *default* model used
/// when an [`LlmRequest`] doesn't override it; today the loop always
/// passes a model so this acts as a fallback only.
#[derive(Clone, Debug)]
pub struct OpenAiConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub default_model: String,
    /// Cap on TCP+TLS handshake time. Stays short — handshake should
    /// complete in seconds; if it doesn't, something is wrong (DNS,
    /// network, TLS).
    pub connect_timeout: Duration,
    /// Cap on time **between** body bytes once streaming starts. NOT
    /// the total request lifetime — reasoning models (gpt-5 high/xhigh)
    /// can legitimately stream for many minutes, and a total `timeout`
    /// would kill them mid-response. As long as tokens keep arriving
    /// faster than this interval, the request runs to completion.
    pub read_timeout: Duration,
    /// When true, add the required GitHub Copilot request headers
    /// (`Editor-Version`, `Copilot-Integration-Id`, etc.) so the
    /// `api.githubcopilot.com` endpoint accepts the request.
    pub copilot_mode: bool,
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com".into(),
            api_key: None,
            default_model: "gpt-4o-mini".into(),
            connect_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(120),
            copilot_mode: false,
        }
    }
}

/// Concrete OpenAI-chat-compatible provider. Cheap to clone (wraps
/// an `Arc`-internal `reqwest::Client`).
#[derive(Clone, Debug)]
pub struct OpenAiProvider {
    config: OpenAiConfig,
    http: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(config: OpenAiConfig) -> anyhow::Result<Self> {
        // Intentionally NOT setting `.timeout()` — that's a total
        // request-lifetime cap that would break long streaming
        // responses from reasoning models. We rely on connect + read
        // (between-bytes) timeouts instead.
        let http = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .read_timeout(config.read_timeout)
            .build()?;
        Ok(Self { config, http })
    }

    pub fn config(&self) -> &OpenAiConfig {
        &self.config
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn stream(&self, request: LlmRequest) -> anyhow::Result<LlmStream> {
        // Normalize base_url so a trailing `/v1` (legacy convention) doesn't
        // produce `/v1/v1/chat/completions` when we append `/v1/...`.
        let base = self
            .config
            .base_url
            .trim_end_matches('/')
            .trim_end_matches("/v1");
        let url = format!("{base}/v1/chat/completions");
        let body = WireRequest::from_request(&request, &self.config.default_model);

        let mut req = self
            .http
            .post(&url)
            .header("accept", "text/event-stream")
            .json(&body);
        if let Some(key) = &self.config.api_key {
            req = req.bearer_auth(key);
        }
        // GitHub Copilot API requires editor identification headers; without
        // them the endpoint returns 403 "Access to this endpoint is forbidden".
        if self.config.copilot_mode {
            req = req
                .header("Editor-Version", "vscode/1.85.0")
                .header("Editor-Plugin-Version", "copilot-chat/0.12.0")
                .header("Copilot-Integration-Id", "vscode-chat")
                .header("User-Agent", "GitHubCopilotChat/0.12.0")
                .header("openai-intent", "conversation-panel");
        }

        let response = req.send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("openai http {status}: {body}");
        }

        let (tx, rx) = mpsc::unbounded_channel();
        // Pump bytes -> SSE events -> LlmEvents on a background task
        // so the caller can poll the stream incrementally.
        tokio::spawn(pump(response, tx));
        Ok(Box::pin(UnboundedReceiverStream::new(rx)))
    }
}

/// Walk reqwest's error source chain — the top-level `Display` is
/// usually a generic phrase like "error decoding response body" while
/// the actual cause (e.g. `unexpected end of file`, `connection reset`,
/// `invalid gzip header`) is one or two `source()` hops deeper.
fn format_reqwest_chain(e: &reqwest::Error) -> String {
    use std::error::Error;
    let mut out = e.to_string();
    let mut src: Option<&dyn Error> = e.source();
    while let Some(s) = src {
        let msg = s.to_string();
        if !out.contains(&msg) {
            out.push_str(": ");
            out.push_str(&msg);
        }
        src = s.source();
    }
    out
}

async fn pump(response: reqwest::Response, tx: mpsc::UnboundedSender<LlmEvent>) {
    let mut bytes = response.bytes_stream();
    let mut buf = String::new();
    while let Some(chunk) = bytes.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(LlmEvent::Error {
                    message: format!("stream io: {}", format_reqwest_chain(&e)),
                });
                return;
            }
        };
        match std::str::from_utf8(&chunk) {
            Ok(s) => buf.push_str(s),
            Err(_) => {
                let _ = tx.send(LlmEvent::Error {
                    message: "non-utf8 chunk from provider".into(),
                });
                return;
            }
        }
        // Parse complete lines out of the rolling buffer.
        while let Some(idx) = buf.find('\n') {
            let line: String = buf.drain(..=idx).collect();
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            let payload = match line
                .strip_prefix("data: ")
                .or_else(|| line.strip_prefix("data:"))
            {
                Some(p) => p.trim_start(),
                None => continue,
            };
            if payload == "[DONE]" {
                // Provider's terminal marker — finish_reason already
                // came through in a prior chunk in OpenAI's protocol.
                return;
            }
            match parse_chunk(payload, &tx) {
                Ok(()) => {}
                Err(e) => {
                    debug!(line = %payload, error = %e, "skipping malformed sse chunk");
                }
            }
        }
    }
    if !buf.trim().is_empty() {
        warn!(remaining = %buf, "stream ended with partial buffered data");
    }
}

fn parse_chunk(payload: &str, tx: &mpsc::UnboundedSender<LlmEvent>) -> anyhow::Result<()> {
    let chunk: WireChunk = serde_json::from_str(payload)?;

    // Emit usage event if present (typically on the final chunk before [DONE])
    if let Some(usage) = chunk.usage {
        let _ = tx.send(LlmEvent::Usage {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
        });
    }

    let Some(choice) = chunk.choices.into_iter().next() else {
        return Ok(());
    };
    if let Some(delta) = choice.delta {
        // Thinking/reasoning content (OpenAI-compat proxies: LiteLLM, OpenRouter, Together)
        if let Some(rc) = delta.reasoning_content {
            if !rc.is_empty() {
                let _ = tx.send(LlmEvent::ThinkingDelta { content: rc });
            }
        }
        if let Some(content) = delta.content {
            if !content.is_empty() {
                let _ = tx.send(LlmEvent::Delta { content });
            }
        }
        if let Some(calls) = delta.tool_calls {
            for c in calls {
                let _ = tx.send(LlmEvent::ToolCallDelta {
                    index: c.index,
                    id: c.id,
                    name: c.function.as_ref().and_then(|f| f.name.clone()),
                    arguments_chunk: c.function.and_then(|f| f.arguments),
                });
            }
        }
    }
    if let Some(reason) = choice.finish_reason {
        let _ = tx.send(LlmEvent::Done {
            finish_reason: FinishReason::from_str(&reason),
        });
    }
    Ok(())
}

// ---------- Wire types -----------------------------------------------------

/// Wire shape for a tool call inside an assistant message:
/// `{ id, type: "function", function: { name, arguments } }`.
#[derive(Serialize)]
struct WireToolCall<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireToolCallFn<'a>,
}

#[derive(Serialize)]
struct WireToolCallFn<'a> {
    name: &'a str,
    arguments: &'a str,
}

impl<'a> From<&'a super::messages::ToolCall> for WireToolCall<'a> {
    fn from(c: &'a super::messages::ToolCall) -> Self {
        Self {
            id: &c.id,
            kind: "function",
            function: WireToolCallFn {
                name: &c.name,
                arguments: &c.arguments,
            },
        }
    }
}

/// Wire shape for a chat message — identical to `ChatMessage` except
/// `tool_calls` uses `WireToolCall` to produce the correct OpenAI format.
#[derive(Serialize)]
struct WireChatMessage<'a> {
    role: super::messages::ChatRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCall<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

impl<'a> From<&'a super::messages::ChatMessage> for WireChatMessage<'a> {
    fn from(m: &'a super::messages::ChatMessage) -> Self {
        Self {
            role: m.role,
            content: m.content.as_deref(),
            tool_calls: m.tool_calls.iter().map(WireToolCall::from).collect(),
            tool_call_id: m.tool_call_id.as_deref(),
            name: m.name.as_deref(),
        }
    }
}

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    messages: Vec<WireChatMessage<'a>>,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    /// OpenAI reasoning_effort for gpt-5 / o-series. Skipped if absent so
    /// non-reasoning models don't choke on the unknown field. Accepts
    /// `"minimal" | "low" | "medium" | "high" | "xhigh"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    /// Anthropic-style thinking block (passed through by some compat proxies).
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<serde_json::Value>,
}

impl<'a> WireRequest<'a> {
    fn from_request(req: &'a LlmRequest, default_model: &'a str) -> Self {
        let model: &str = if req.model.is_empty() {
            default_model
        } else {
            &req.model
        };
        let tools: Vec<WireTool> = req.tools.iter().map(WireTool::from).collect();
        let tool_choice = if tools.is_empty() { None } else { Some("auto") };
        let (reasoning_effort, thinking) = match &req.thinking {
            Some(ThinkingConfig::ReasoningEffort { effort }) => (Some(effort.clone()), None),
            Some(ThinkingConfig::Budget { budget_tokens }) => (
                None,
                Some(serde_json::json!({
                    "type": "enabled",
                    "budget_tokens": budget_tokens,
                })),
            ),
            // Adaptive thinking is Anthropic-native; silently ignore it here
            // so OpenAI-compat endpoints don't reject the request.
            Some(ThinkingConfig::Adaptive { .. }) => (None, None),
            None => (None, None),
        };
        Self {
            model,
            messages: req.messages.iter().map(WireChatMessage::from).collect(),
            stream: true,
            tools,
            tool_choice,
            temperature: req.temperature,
            // Per-request `reasoning_effort` (from LlmRequest) wins; fall back to
            // ThinkingConfig::ReasoningEffort if the caller used that path.
            reasoning_effort: req.reasoning_effort.clone().or(reasoning_effort),
            thinking,
        }
    }
}

#[derive(Serialize)]
struct WireTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: WireFunction<'a>,
}

#[derive(Serialize)]
struct WireFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

impl<'a> From<&'a ToolDef> for WireTool<'a> {
    fn from(t: &'a ToolDef) -> Self {
        Self {
            kind: "function",
            function: WireFunction {
                name: &t.name,
                description: &t.description,
                parameters: &t.parameters,
            },
        }
    }
}

#[derive(serde::Deserialize)]
struct WireChunk {
    #[serde(default)]
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(serde::Deserialize)]
struct WireUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
}

#[derive(serde::Deserialize)]
struct WireChoice {
    #[serde(default)]
    delta: Option<WireDelta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(serde::Deserialize)]
struct WireDelta {
    #[serde(default)]
    content: Option<String>,
    /// OpenAI-compat `reasoning_content` field emitted by LiteLLM, OpenRouter,
    /// Together, etc. when a reasoning model is proxied. Empty/null → skip.
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<WireToolCallDelta>>,
}

#[derive(serde::Deserialize)]
struct WireToolCallDelta {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<WireFunctionDelta>,
}

#[derive(serde::Deserialize)]
struct WireFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn collect_from_payload(payload: &str) -> Vec<LlmEvent> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _ = parse_chunk(payload, &tx);
        drop(tx);
        let mut out = vec![];
        while let Ok(ev) = rx.try_recv() {
            out.push(ev);
        }
        out
    }

    // ── 10.4: LlmEvent::Usage emitted from OpenAI SSE chunk with usage field ──

    #[test]
    fn usage_event_emitted_from_openai_chunk_with_usage() {
        // Typical final chunk from OpenAI stream_options.include_usage=true.
        let payload = r#"{
            "choices": [{"delta": {"content": null}, "finish_reason": "stop", "index": 0}],
            "usage": {"prompt_tokens": 15, "completion_tokens": 8}
        }"#;

        let events = collect_from_payload(payload);

        let usage_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, LlmEvent::Usage { .. }))
            .collect();

        assert_eq!(
            usage_events.len(),
            1,
            "expected 1 Usage event, got {:?}",
            events
        );
        assert!(
            matches!(
                usage_events[0],
                LlmEvent::Usage {
                    input_tokens: 15,
                    output_tokens: 8
                }
            ),
            "expected Usage {{ input=15, output=8 }}, got {:?}",
            usage_events[0]
        );
    }

    #[test]
    fn no_usage_event_when_usage_field_absent() {
        let payload = r#"{
            "choices": [{"delta": {"content": "Hi"}, "finish_reason": null, "index": 0}]
        }"#;
        let events = collect_from_payload(payload);
        let has_usage = events.iter().any(|e| matches!(e, LlmEvent::Usage { .. }));
        assert!(!has_usage, "expected no Usage event, got {:?}", events);
    }
}
