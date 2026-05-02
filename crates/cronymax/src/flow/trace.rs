//! Structured trace events and async append-only trace writer.
//!
//! Every flow run maintains a `trace.jsonl` file. Each line is a
//! self-contained JSON object produced by [`TraceEvent::to_json_line()`].
//!
//! [`TraceWriter`] enqueues events in a `tokio` channel and flushes them
//! to disk on a dedicated background task, mirroring the behaviour of
//! `app/flow/TraceWriter`. Subscribers (e.g. the event-bus bridge) receive
//! each event after it has been appended, including a replay of existing
//! events via [`TraceWriter::subscribe_replay()`].

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::mpsc;

// ── TraceKind ─────────────────────────────────────────────────────────────────

/// Discriminant for a trace event line. Serialised as `snake_case` in JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceKind {
    RunStarted,
    RunCompleted,
    RunCancelled,
    RunFailed,
    AgentStarted,
    AgentScheduled,
    AgentEnded,
    ToolCall,
    ToolResult,
    DocumentSubmitted,
    ReviewRequested,
    ReviewVerdict,
    ReviewExhausted,
    Routed,
    Mention,
    Error,
}

// ── TraceEvent ────────────────────────────────────────────────────────────────

/// One structured event in the trace stream.
///
/// Absent fields are serialised as empty strings. `ts_ms` is
/// Unix-epoch milliseconds, filled automatically by
/// [`TraceEvent::now()`].
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TraceEvent {
    pub kind: Option<TraceKind>,
    pub ts_ms: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub space_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub run_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub doc_type: String,
    /// Invocation ID for agent scheduling events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_id: Option<String>,
    /// Pending ports list emitted on `agent_scheduled` events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_ports: Vec<String>,
    /// Raw JSON payload (optional). Embedded verbatim into the line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl TraceEvent {
    /// Create a new event with the current Unix-epoch timestamp.
    pub fn now(kind: TraceKind) -> Self {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Self { kind: Some(kind), ts_ms, ..Default::default() }
    }

    /// Serialise to a single JSON line (includes trailing `\n`).
    pub fn to_json_line(&self) -> String {
        let mut line = serde_json::to_string(self).unwrap_or_default();
        line.push('\n');
        line
    }
}

// ── TraceWriter ───────────────────────────────────────────────────────────────

/// Subscriber callback type.
pub type Subscriber = Box<dyn Fn(&TraceEvent) + Send + 'static>;

/// Append-only, async trace writer.
///
/// * `append()` enqueues an event; the background task flushes it to disk
///   and notifies live subscribers.
/// * `subscribe_replay()` replays events that are already on disk, then
///   attaches the subscriber for future events.
/// * `flush()` waits for the queue to drain.
pub struct TraceWriter {
    tx: mpsc::UnboundedSender<TraceEvent>,
    subscribers: Arc<Mutex<Vec<(usize, Subscriber)>>>,
    next_token: Arc<std::sync::atomic::AtomicUsize>,
    path: PathBuf,
    // Keep the task alive
    _task: Arc<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for TraceWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceWriter").field("path", &self.path).finish()
    }
}

impl TraceWriter {
    /// Create a new `TraceWriter` that appends to `trace_path`.
    ///
    /// The file is created (and parent directories made) if absent.
    pub fn new(trace_path: PathBuf) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<TraceEvent>();
        let subs: Arc<Mutex<Vec<(usize, Subscriber)>>> = Arc::new(Mutex::new(vec![]));
        let subs_clone = Arc::clone(&subs);
        let path_clone = trace_path.clone();

        let task = tokio::spawn(async move {
            // Ensure parent directory exists.
            if let Some(parent) = path_clone.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let mut file = match tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path_clone)
                .await
            {
                Ok(f) => f,
                Err(e) => {
                    tracing::error!("TraceWriter: failed to open {:?}: {e}", path_clone);
                    return;
                }
            };

            while let Some(evt) = rx.recv().await {
                let line = evt.to_json_line();
                let _ = file.write_all(line.as_bytes()).await;
                // Notify subscribers
                let locked = subs_clone.lock();
                for (_, cb) in locked.iter() {
                    cb(&evt);
                }
            }
        });

        Self {
            tx,
            subscribers: subs,
            next_token: Arc::new(std::sync::atomic::AtomicUsize::new(1)),
            path: trace_path,
            _task: Arc::new(task),
        }
    }

    /// Enqueue an event. Non-blocking.
    pub fn append(&self, evt: TraceEvent) {
        let _ = self.tx.send(evt);
    }

    /// Register a subscriber. Returns an opaque token for unsubscribing.
    ///
    /// Existing events on disk are replayed synchronously before attaching.
    pub fn subscribe_replay(&self, cb: Subscriber) -> usize {
        // Replay existing lines from disk (best-effort, sync read).
        if let Ok(content) = std::fs::read_to_string(&self.path) {
            for line in content.lines() {
                if let Ok(evt) = serde_json::from_str::<TraceEvent>(line) {
                    cb(&evt);
                }
            }
        }
        let token = self
            .next_token
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.subscribers.lock().push((token, cb));
        token
    }

    /// Unsubscribe a previously registered callback.
    pub fn unsubscribe(&self, token: usize) {
        self.subscribers.lock().retain(|(t, _)| *t != token);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_event_json_line_has_newline() {
        let evt = TraceEvent::now(TraceKind::RunStarted);
        let line = evt.to_json_line();
        assert!(line.ends_with('\n'));
        assert!(line.contains("\"run_started\""));
    }

    #[test]
    fn trace_kind_serde_roundtrip() {
        let kind = TraceKind::ToolCall;
        let s = serde_json::to_string(&kind).unwrap();
        assert_eq!(s, "\"tool_call\"");
        let back: TraceKind = serde_json::from_str(&s).unwrap();
        assert_eq!(back, kind);
    }

    #[tokio::test]
    async fn trace_writer_appends_to_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("trace.jsonl");
        let writer = TraceWriter::new(path.clone());

        let mut evt = TraceEvent::now(TraceKind::AgentStarted);
        evt.agent_id = "pm".into();
        writer.append(evt);

        // Give the background task a moment to flush.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("agent_started"));
        assert!(content.contains("pm"));
    }

    #[tokio::test]
    async fn subscribe_replay_sees_existing_events() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("trace.jsonl");

        // Write one event.
        let writer = TraceWriter::new(path.clone());
        let mut evt = TraceEvent::now(TraceKind::RunStarted);
        evt.run_id = "r1".into();
        writer.append(evt);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        drop(writer);

        // New writer over same file — subscribe should replay the previous event.
        let writer2 = TraceWriter::new(path);
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen_clone = Arc::clone(&seen);
        writer2.subscribe_replay(Box::new(move |e| {
            seen_clone.lock().push(e.run_id.clone());
        }));
        assert!(seen.lock().contains(&"r1".to_owned()));
    }
}
