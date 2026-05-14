//! Anthropic Messages API provider.
//!
//! Implements [`LlmProvider`] using the native Anthropic SSE wire format:
//! * `POST {base_url}/v1/messages`
//! * Streaming via `content_block_start/delta/stop` and `message_delta` events
//! * Tool calls via `tool_use` content blocks
//! * Thinking via `thinking` content blocks (requires `interleaved-thinking` beta)
//!
//! # Message translation from OpenAI-style history
//!
//! | OpenAI                        | Anthropic                                         |
//! |-------------------------------|---------------------------------------------------|
//! | `role: system`                | top-level `system` field                          |
//! | `role: user`                  | `{ role: "user", content: [{ type: "text" }] }`  |
//! | `role: assistant` (text)      | `{ role: "assistant", content: [{ type: "text" }] }` |
//! | `role: assistant` (tool call) | `{ role: "assistant", content: [{ type: "tool_use" }] }` |
//! | `role: tool`                  | `{ role: "user", content: [{ type: "tool_result" }] }` |

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::messages::{ChatMessage, ChatRole, FinishReason, LlmRequest, ThinkingConfig};
use super::provider::{LlmEvent, LlmProvider, LlmStream};
use super::stream::UnboundedReceiverStream;

// ── Config ────────────────────────────────────────────────────────────────────

/// Per-instance configuration for `AnthropicProvider`.
#[derive(Clone, Debug)]
pub struct AnthropicConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub default_model: String,
    pub request_timeout: Duration,
    /// Beta feature header values appended when thinking is enabled.
    /// Defaults to `["interleaved-thinking-2025-05-14"]`.
    pub beta_features: Vec<String>,
    /// Maximum tokens to request in each generation. Anthropic requires
    /// `max_tokens` to be set; defaults to 8192.
    pub max_tokens: u32,
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.anthropic.com".into(),
            api_key: None,
            default_model: "claude-3-5-sonnet-20241022".into(),
            request_timeout: Duration::from_secs(120),
            beta_features: vec!["interleaved-thinking-2025-05-14".into()],
            max_tokens: 8192,
        }
    }
}

// ── Provider ──────────────────────────────────────────────────────────────────

/// Anthropic Messages API streaming provider.
#[derive(Clone, Debug)]
pub struct AnthropicProvider {
    config: AnthropicConfig,
    http: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .build()?;
        Ok(Self { config, http })
    }

    pub fn config(&self) -> &AnthropicConfig {
        &self.config
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn stream(&self, request: LlmRequest) -> anyhow::Result<LlmStream> {
        let url = format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'));

        let model = if request.model.is_empty() {
            &self.config.default_model
        } else {
            &request.model
        };

        let body = build_wire_request(&request, model, self.config.max_tokens);

        let mut req = self
            .http
            .post(&url)
            .header("content-type", "application/json")
            .header("anthropic-version", "2023-06-01")
            .header("accept", "text/event-stream");

        if let Some(key) = &self.config.api_key {
            req = req.header("x-api-key", key.as_str());
        }

        // Add beta header when thinking is enabled.
        if request.thinking.is_some() && !self.config.beta_features.is_empty() {
            let beta = self.config.beta_features.join(",");
            req = req.header("anthropic-beta", beta);
        }

        let body_bytes = serde_json::to_vec(&body)?;
        let req = req.body(body_bytes);

        let response = req.send().await?;
        let status = response.status();
        if !status.is_success() {
            let body_text = response.text().await.unwrap_or_default();
            anyhow::bail!("anthropic http {status}: {body_text}");
        }

        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(pump(response, tx));
        Ok(Box::pin(UnboundedReceiverStream::new(rx)))
    }
}

// ── Wire request construction ─────────────────────────────────────────────────

fn build_wire_request(req: &LlmRequest, model: &str, max_tokens: u32) -> serde_json::Value {
    // Extract system message (first message with role=System).
    let system_text: Option<String> = req.messages.iter().find_map(|m| {
        if m.role == ChatRole::System {
            m.content.clone()
        } else {
            None
        }
    });

    // Translate remaining messages to Anthropic format.
    let messages: Vec<serde_json::Value> = translate_messages(&req.messages);

    // Translate tool definitions.
    let tools: Vec<serde_json::Value> = req
        .tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
            })
        })
        .collect();

    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "stream": true,
        "messages": messages,
    });

    if let Some(sys) = system_text {
        body["system"] = serde_json::Value::String(sys);
    }

    if !tools.is_empty() {
        body["tools"] = serde_json::Value::Array(tools);
        body["tool_choice"] = serde_json::json!({ "type": "auto" });
    }

    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }

    // Attach thinking parameters.
    if let Some(thinking) = &req.thinking {
        body["thinking"] = match thinking {
            ThinkingConfig::Adaptive { .. } => {
                serde_json::json!({ "type": "adaptive", "display": "summarized" })
            }
            ThinkingConfig::Budget { budget_tokens } => {
                serde_json::json!({ "type": "enabled", "budget_tokens": budget_tokens })
            }
            // ReasoningEffort is not meaningful for Anthropic; skip.
            ThinkingConfig::ReasoningEffort { .. } => serde_json::Value::Null,
        };
        // Remove null thinking field if it was set (ReasoningEffort case).
        if body["thinking"].is_null() {
            let obj = body.as_object_mut().unwrap();
            obj.remove("thinking");
        }
    }

    body
}

/// Translate OpenAI-style `Vec<ChatMessage>` to the Anthropic `messages` array.
/// System messages are stripped (they belong in the top-level `system` field).
fn translate_messages(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = Vec::new();

    for msg in messages {
        match msg.role {
            ChatRole::System => {
                // Already extracted; skip.
            }
            ChatRole::User => {
                let text = msg.content.as_deref().unwrap_or("");
                out.push(serde_json::json!({
                    "role": "user",
                    "content": [{ "type": "text", "text": text }],
                }));
            }
            ChatRole::Assistant => {
                if !msg.tool_calls.is_empty() {
                    // Assistant issued tool calls — translate each to `tool_use`.
                    let blocks: Vec<serde_json::Value> = msg
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            let input: serde_json::Value =
                                serde_json::from_str(&tc.arguments).unwrap_or_default();
                            serde_json::json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.name,
                                "input": input,
                            })
                        })
                        .collect();
                    out.push(serde_json::json!({
                        "role": "assistant",
                        "content": blocks,
                    }));
                } else {
                    let text = msg.content.as_deref().unwrap_or("");
                    out.push(serde_json::json!({
                        "role": "assistant",
                        "content": [{ "type": "text", "text": text }],
                    }));
                }
            }
            ChatRole::Tool => {
                // Tool result — translate to user message with `tool_result` block.
                let tool_use_id = msg.tool_call_id.as_deref().unwrap_or("");
                let content = msg.content.as_deref().unwrap_or("");
                out.push(serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": content,
                    }],
                }));
            }
        }
    }

    out
}

// ── SSE pump ─────────────────────────────────────────────────────────────────

/// State for a single `tool_use` content block being accumulated across deltas.
#[derive(Default)]
struct AccumToolUse {
    id: String,
    name: String,
    input_json: String,
    index: usize,
}

/// Block-type tracker so deltas are routed to the right accumulator.
enum BlockKind {
    Text,
    Thinking,
    ToolUse(usize), // index into `tool_uses`
    Redacted,
}

async fn pump(response: reqwest::Response, tx: mpsc::UnboundedSender<LlmEvent>) {
    let mut bytes = response.bytes_stream();
    let mut buf = String::new();

    // Active content block state per index.
    let mut block_kinds: BTreeMap<usize, BlockKind> = BTreeMap::new();
    let mut tool_uses: BTreeMap<usize, AccumToolUse> = BTreeMap::new();
    // Current SSE event name (set by `event:` lines).
    let mut current_event: String = String::new();

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
                    message: "non-utf8 chunk from anthropic".into(),
                });
                return;
            }
        }

        // Parse complete lines.
        while let Some(idx) = buf.find('\n') {
            let line: String = buf.drain(..=idx).collect();
            let line = line.trim_end_matches(['\r', '\n']);

            if line.is_empty() {
                // Blank line — SSE event boundary; reset event name.
                current_event.clear();
                continue;
            }
            if line.starts_with(':') {
                continue; // SSE comment
            }
            if let Some(event_name) = line.strip_prefix("event:").map(str::trim) {
                current_event = event_name.to_owned();
                continue;
            }
            let payload = match line
                .strip_prefix("data: ")
                .or_else(|| line.strip_prefix("data:"))
            {
                Some(p) => p.trim_start(),
                None => continue,
            };

            match process_sse_event(
                &current_event,
                payload,
                &mut block_kinds,
                &mut tool_uses,
                &tx,
            ) {
                Ok(true) => return, // stream done
                Ok(false) => {}
                Err(e) => {
                    debug!(event = %current_event, payload = %payload, error = %e, "skipping malformed anthropic sse event");
                }
            }
        }
    }
    if !buf.trim().is_empty() {
        warn!(remaining = %buf, "anthropic stream ended with partial buffered data");
    }
}

/// Process one parsed SSE event. Returns `Ok(true)` when streaming is complete.
fn process_sse_event(
    event_type: &str,
    payload: &str,
    block_kinds: &mut BTreeMap<usize, BlockKind>,
    tool_uses: &mut BTreeMap<usize, AccumToolUse>,
    tx: &mpsc::UnboundedSender<LlmEvent>,
) -> anyhow::Result<bool> {
    let v: serde_json::Value = serde_json::from_str(payload)?;
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match kind {
        "content_block_start" => {
            let index = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
            let block_type = v
                .pointer("/content_block/type")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let bk = match block_type {
                "thinking" => BlockKind::Thinking,
                "redacted_thinking" => BlockKind::Redacted,
                "tool_use" => {
                    let id = v
                        .pointer("/content_block/id")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let name = v
                        .pointer("/content_block/name")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let acc = AccumToolUse {
                        id,
                        name,
                        input_json: String::new(),
                        index,
                    };
                    tool_uses.insert(index, acc);
                    BlockKind::ToolUse(index)
                }
                _ => BlockKind::Text,
            };
            block_kinds.insert(index, bk);
        }

        "content_block_delta" => {
            let index = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
            let delta_type = v
                .pointer("/delta/type")
                .and_then(|t| t.as_str())
                .unwrap_or("");

            match (block_kinds.get(&index), delta_type) {
                (Some(BlockKind::Thinking), "thinking_delta") => {
                    let text = v
                        .pointer("/delta/thinking")
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    if !text.is_empty() {
                        let _ = tx.send(LlmEvent::ThinkingDelta {
                            content: text.to_owned(),
                        });
                    }
                }
                (Some(BlockKind::Text), "text_delta") => {
                    let text = v
                        .pointer("/delta/text")
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    if !text.is_empty() {
                        let _ = tx.send(LlmEvent::Delta {
                            content: text.to_owned(),
                        });
                    }
                }
                (Some(BlockKind::ToolUse(tu_idx)), "input_json_delta") => {
                    let partial = v
                        .pointer("/delta/partial_json")
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    if let Some(acc) = tool_uses.get_mut(tu_idx) {
                        acc.input_json.push_str(partial);
                    }
                    // Emit ToolCallDelta with the accumulated partial JSON so far.
                    // First delta — emit id + name. Subsequent — arguments only.
                    if let Some(acc) = tool_uses.get(tu_idx) {
                        let is_first = acc.input_json.len() == partial.len();
                        let _ = tx.send(LlmEvent::ToolCallDelta {
                            index: acc.index,
                            id: if is_first { Some(acc.id.clone()) } else { None },
                            name: if is_first {
                                Some(acc.name.clone())
                            } else {
                                None
                            },
                            arguments_chunk: if partial.is_empty() {
                                None
                            } else {
                                Some(partial.to_owned())
                            },
                        });
                    }
                }
                (Some(BlockKind::Redacted), _) => {
                    // Redacted thinking blocks — deliberately ignored.
                }
                _ => {
                    debug!(
                        index,
                        delta_type,
                        event = event_type,
                        "anthropic: unknown block/delta combo"
                    );
                }
            }
        }

        "content_block_stop" => {
            let index = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
            block_kinds.remove(&index);
        }

        "message_start" => {
            // Extract input token usage from the message_start event.
            if let Some(n) = v
                .pointer("/message/usage/input_tokens")
                .and_then(|v| v.as_u64())
            {
                if n > 0 {
                    let _ = tx.send(LlmEvent::Usage {
                        input_tokens: n,
                        output_tokens: 0,
                    });
                }
            }
        }

        "message_delta" => {
            // Extract output token usage from the delta event.
            if let Some(n) = v.pointer("/usage/output_tokens").and_then(|v| v.as_u64()) {
                if n > 0 {
                    let _ = tx.send(LlmEvent::Usage {
                        input_tokens: 0,
                        output_tokens: n,
                    });
                }
            }
            let stop_reason = v
                .pointer("/delta/stop_reason")
                .and_then(|s| s.as_str())
                .unwrap_or("end_turn");
            let finish = match stop_reason {
                "end_turn" => FinishReason::Stop,
                "tool_use" => FinishReason::ToolCalls,
                "max_tokens" => FinishReason::Length,
                other => FinishReason::Other(other.to_owned()),
            };
            let _ = tx.send(LlmEvent::Done {
                finish_reason: finish,
            });
            return Ok(true);
        }

        "message_stop" | "error" => {
            if kind == "error" {
                let msg = v
                    .pointer("/error/message")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown anthropic error");
                let _ = tx.send(LlmEvent::Error {
                    message: msg.to_owned(),
                });
            }
            return Ok(true);
        }

        _ => {
            // message_start, ping, etc. — no action needed.
        }
    }

    Ok(false)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::messages::ToolCall;

    fn make_request(messages: Vec<ChatMessage>) -> LlmRequest {
        LlmRequest {
            model: "claude-3-5-sonnet-20241022".into(),
            messages,
            tools: vec![],
            temperature: None,
            thinking: None,
        }
    }

    #[test]
    fn system_message_extracted_to_top_level() {
        let req = make_request(vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::user("Hello"),
        ]);
        let body = build_wire_request(&req, "claude-3-5-sonnet-20241022", 8192);
        assert_eq!(body["system"], "You are a helpful assistant.");
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
    }

    #[test]
    fn tool_result_becomes_user_tool_result() {
        let req = make_request(vec![
            ChatMessage::user("call tool"),
            ChatMessage::tool_result("call-1", "bash", "output here"),
        ]);
        let body = build_wire_request(&req, "claude-3-5-sonnet-20241022", 8192);
        let msgs = body["messages"].as_array().unwrap();
        let tool_msg = &msgs[1];
        assert_eq!(tool_msg["role"], "user");
        assert_eq!(tool_msg["content"][0]["type"], "tool_result");
        assert_eq!(tool_msg["content"][0]["tool_use_id"], "call-1");
        assert_eq!(tool_msg["content"][0]["content"], "output here");
    }

    #[test]
    fn assistant_tool_calls_translated_to_tool_use() {
        let req = make_request(vec![ChatMessage::assistant_tool_calls(vec![ToolCall {
            id: "tc-1".into(),
            name: "bash".into(),
            arguments: r#"{"command":"ls"}"#.into(),
        }])]);
        let body = build_wire_request(&req, "claude-3-5-sonnet-20241022", 8192);
        let msgs = body["messages"].as_array().unwrap();
        let blk = &msgs[0]["content"][0];
        assert_eq!(blk["type"], "tool_use");
        assert_eq!(blk["id"], "tc-1");
        assert_eq!(blk["name"], "bash");
        assert_eq!(blk["input"]["command"], "ls");
    }

    #[test]
    fn adaptive_thinking_config_serialized() {
        let mut req = make_request(vec![ChatMessage::user("think")]);
        req.thinking = Some(ThinkingConfig::Adaptive { summarized: true });
        let body = build_wire_request(&req, "claude-3-5-sonnet-20241022", 8192);
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["thinking"]["display"], "summarized");
    }

    #[test]
    fn reasoning_effort_not_sent_to_anthropic() {
        let mut req = make_request(vec![ChatMessage::user("think")]);
        req.thinking = Some(ThinkingConfig::ReasoningEffort {
            effort: "medium".into(),
        });
        let body = build_wire_request(&req, "claude-3-5-sonnet-20241022", 8192);
        assert!(body.get("thinking").is_none());
    }

    // SSE parsing tests
    fn collect_events(events: &[(&str, &str)]) -> Vec<LlmEvent> {
        let mut block_kinds = BTreeMap::new();
        let mut tool_uses = BTreeMap::new();
        let (tx, mut rx) = mpsc::unbounded_channel();

        for (ev_type, payload) in events {
            let _ = process_sse_event(ev_type, payload, &mut block_kinds, &mut tool_uses, &tx);
        }
        drop(tx);
        let mut out = vec![];
        while let Ok(ev) = rx.try_recv() {
            out.push(ev);
        }
        out
    }

    #[test]
    fn thinking_delta_emitted_as_thinking_delta_event() {
        let events = collect_events(&[
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"I should think"}}"#,
            ),
        ]);
        assert!(
            matches!(&events[0], LlmEvent::ThinkingDelta { content } if content == "I should think")
        );
    }

    #[test]
    fn text_delta_emitted_as_delta_event() {
        let events = collect_events(&[
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
            ),
        ]);
        assert!(matches!(&events[0], LlmEvent::Delta { content } if content == "Hello"));
    }

    #[test]
    fn input_json_delta_emitted_as_tool_call_delta() {
        let events = collect_events(&[
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tc-1","name":"bash","input":{}}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"cmd\":\"ls\"}"}}"#,
            ),
        ]);
        assert!(matches!(&events[0], LlmEvent::ToolCallDelta { id: Some(id), .. } if id == "tc-1"));
    }

    #[test]
    fn redacted_thinking_not_emitted() {
        let events = collect_events(&[
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"redacted_thinking_delta","data":"abc"}}"#,
            ),
        ]);
        assert!(
            events.is_empty(),
            "expected no events for redacted_thinking"
        );
    }

    // ── 10.3: LlmEvent::Usage emitted from Anthropic SSE ─────────────────────

    #[test]
    fn usage_events_emitted_from_message_start_and_message_delta() {
        let events = collect_events(&[
            (
                "message_start",
                r#"{"type":"message_start","message":{"usage":{"input_tokens":42,"output_tokens":0}}}"#,
            ),
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
            ),
            (
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}"#,
            ),
        ]);

        // Find Usage events.
        let usage_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, LlmEvent::Usage { .. }))
            .collect();

        assert_eq!(
            usage_events.len(),
            2,
            "expected 2 Usage events, got {:?}",
            usage_events
        );

        // First Usage: input_tokens=42, output_tokens=0 (from message_start).
        assert!(
            matches!(
                usage_events[0],
                LlmEvent::Usage {
                    input_tokens: 42,
                    output_tokens: 0
                }
            ),
            "first Usage should be input_tokens=42, got {:?}",
            usage_events[0]
        );

        // Second Usage: input_tokens=0, output_tokens=7 (from message_delta).
        assert!(
            matches!(
                usage_events[1],
                LlmEvent::Usage {
                    input_tokens: 0,
                    output_tokens: 7
                }
            ),
            "second Usage should be output_tokens=7, got {:?}",
            usage_events[1]
        );
    }
}
