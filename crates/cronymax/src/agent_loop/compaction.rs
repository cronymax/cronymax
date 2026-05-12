//! Thread compaction: heuristic token-counting + LLM summarisation.
//!
//! When a session thread approaches the model's context limit we:
//!
//!   1. Summarise the *older* portion of the thread with a single LLM call.
//!   2. Replace that portion with a synthetic `system` message carrying
//!      the summary.
//!   3. Keep the most recent `recency_turns` assistant+user turn-pairs intact
//!      so the agent retains immediate context.
//!
//! The compaction is intentionally conservative: it only fires when the
//! token estimate exceeds `threshold_pct` percent of a nominal 128 k-token
//! context window, and it always preserves at least the system prompt
//! (first message if role == System).

use std::sync::Arc;

use futures_util::StreamExt;
use tracing::{info, warn};

use crate::llm::{ChatMessage, ChatRole, LlmProvider, LlmRequest};

/// Rough token estimate: 4 characters ≈ 1 token.
pub fn token_estimate(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|m| m.content.as_deref().unwrap_or("").len() / 4)
        .sum()
}

/// Context window we measure against (nominal 128 k tokens).
const CONTEXT_WINDOW_TOKENS: usize = 128_000;

/// Default threshold: compact when thread exceeds this fraction.
pub const DEFAULT_THRESHOLD_PCT: u8 = 80;

/// Default number of recent turns to preserve verbatim after compaction.
pub const DEFAULT_RECENCY_TURNS: usize = 6;

/// Outcome of a compaction attempt.
pub struct CompactionResult {
    /// The (possibly compacted) thread to use for the next run.
    pub thread: Vec<ChatMessage>,
    /// Whether compaction actually occurred (false ⇒ thread unchanged).
    pub compacted: bool,
    /// Plain-text summary written to memory, if compacted.
    pub summary: Option<String>,
}

/// Attempt to compact `thread` if it exceeds the token threshold.
///
/// * `model`         – model name used for the summary call
/// * `threshold_pct` – fire when `token_estimate / CONTEXT_WINDOW * 100 >= threshold_pct`
/// * `recency_turns` – number of user+assistant turn-pairs to preserve
///
/// Returns the (possibly unchanged) thread and a flag indicating whether
/// compaction ran.  Errors from the LLM are logged and treated as
/// non-fatal: the original thread is returned unchanged so the run can
/// still proceed.
pub async fn maybe_compact(
    thread: Vec<ChatMessage>,
    llm: Arc<dyn LlmProvider>,
    model: &str,
    threshold_pct: u8,
    recency_turns: usize,
) -> CompactionResult {
    let estimated_tokens = token_estimate(&thread);
    let threshold_tokens = CONTEXT_WINDOW_TOKENS * threshold_pct as usize / 100;

    if estimated_tokens < threshold_tokens {
        return CompactionResult {
            thread,
            compacted: false,
            summary: None,
        };
    }

    info!(
        estimated_tokens,
        threshold_tokens,
        messages = thread.len(),
        "compaction: threshold exceeded, summarising thread"
    );

    // Split the thread: keep the leading system prompt (if any) and the
    // most recent `recency_turns` turn-pairs at the tail; summarise the
    // middle section.
    let (prefix_end, recency_start) = split_thread(&thread, recency_turns);

    let to_summarise = &thread[prefix_end..recency_start];
    if to_summarise.is_empty() {
        // Nothing to compact between prefix and recency anchor.
        return CompactionResult {
            thread,
            compacted: false,
            summary: None,
        };
    }

    // Build a summarisation prompt from the middle messages.
    let formatted = format_for_summary(to_summarise);
    let summary_request = LlmRequest::new(
        model,
        vec![
            ChatMessage::system(crate::crony::prompts::COMPACTION_SUMMARIZE.to_owned()),
            ChatMessage::user(formatted),
        ],
    );

    let summary = match collect_text(llm, summary_request).await {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, "compaction: LLM summarisation failed, keeping full thread");
            return CompactionResult {
                thread,
                compacted: false,
                summary: None,
            };
        }
    };

    // Rebuild the thread: [original prefix] + [summary system msg] + [recency tail]
    let mut compacted = thread[..prefix_end].to_vec();
    compacted.push(ChatMessage::system(format!(
        "[Conversation summary]\n{summary}"
    )));
    compacted.extend_from_slice(&thread[recency_start..]);

    info!(
        original_msgs = thread.len(),
        compacted_msgs = compacted.len(),
        "compaction: thread compacted"
    );

    CompactionResult {
        thread: compacted,
        compacted: true,
        summary: Some(summary),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Return `(prefix_end, recency_start)` indices for the split.
///
/// * `prefix_end`    – end of the leading system messages (preserved as-is)
/// * `recency_start` – start of the tail we keep verbatim
fn split_thread(thread: &[ChatMessage], recency_turns: usize) -> (usize, usize) {
    // Count leading system messages.
    let prefix_end = thread
        .iter()
        .take_while(|m| m.role == ChatRole::System)
        .count();

    // A "turn" is one user message + one or more assistant messages that follow.
    // We count backwards from the tail.
    let mut turns_seen = 0usize;
    let mut recency_start = thread.len();
    let mut prev_role: Option<ChatRole> = None;

    for (i, msg) in thread.iter().enumerate().rev() {
        if msg.role == ChatRole::User {
            if prev_role.map(|r| r == ChatRole::Assistant).unwrap_or(false) {
                turns_seen += 1;
            }
            if turns_seen >= recency_turns {
                recency_start = i;
                break;
            }
        }
        prev_role = Some(msg.role.clone());
    }

    // Ensure recency_start doesn't overlap with the prefix.
    let recency_start = recency_start.max(prefix_end);

    (prefix_end, recency_start)
}

/// Format a slice of messages into a text block for the summariser.
fn format_for_summary(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .map(|m| {
            let role = match m.role {
                ChatRole::User => "User",
                ChatRole::Assistant => "Assistant",
                ChatRole::System => "System",
                ChatRole::Tool => "Tool",
            };
            let content = m.content.as_deref().unwrap_or("[no text]");
            format!("{role}: {content}")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Drive the LLM stream to completion and collect all `Delta.content`
/// chunks into a single string.
async fn collect_text(
    llm: Arc<dyn LlmProvider>,
    request: LlmRequest,
) -> anyhow::Result<String> {
    let mut stream = llm.stream(request).await?;
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        use crate::llm::LlmEvent;
        match event {
            LlmEvent::Delta { content } => text.push_str(&content),
            LlmEvent::Done { .. } => break,
            LlmEvent::Error { message } => {
                return Err(anyhow::anyhow!("LLM error during compaction: {message}"));
            }
            _ => {}
        }
    }
    Ok(text)
}
