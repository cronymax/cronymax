//! OpenAI-chat-compatible streaming client.
//!
//! Implements the same wire shape the renderer used in
//! `web/src/agent_runtime/llm.js`:
//!
//! * `POST {base_url}/chat/completions`
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

/// Per-instance configuration. `model` is the *default* model used
/// when an [`LlmRequest`] doesn't override it; today the loop always
/// passes a model so this acts as a fallback only.
#[derive(Clone, Debug)]
pub struct OpenAiConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub default_model: String,
    pub request_timeout: Duration,
    /// When true, add the required GitHub Copilot request headers
    /// (`Editor-Version`, `Copilot-Integration-Id`, etc.) so the
    /// `api.githubcopilot.com` endpoint accepts the request.
    pub copilot_mode: bool,
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".into(),
            api_key: None,
            default_model: "gpt-4o-mini".into(),
            request_timeout: Duration::from_secs(120),
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
        let http = reqwest::Client::builder()
            .timeout(config.request_timeout)
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
        let url = format!("{}/chat/completions", self.config.base_url.trim_end_matches('/'));
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

async fn pump(response: reqwest::Response, tx: mpsc::UnboundedSender<LlmEvent>) {
    let mut bytes = response.bytes_stream();
    let mut buf = String::new();
    while let Some(chunk) = bytes.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(LlmEvent::Error {
                    message: format!("stream io: {e}"),
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
            let payload = match line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:")) {
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
    let Some(choice) = chunk.choices.into_iter().next() else {
        return Ok(());
    };
    if let Some(delta) = choice.delta {
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
        Self {
            model,
            messages: req.messages.iter().map(WireChatMessage::from).collect(),
            stream: true,
            tools,
            tool_choice,
            temperature: req.temperature,
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
