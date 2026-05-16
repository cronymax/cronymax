//! Session lifecycle helpers (`sessions` module).
//!
//! Wraps [`RuntimeAuthority`] and integrates with [`MemoryManager`] to provide
//! higher-level session operations used by the runtime handler.
//!
//! ## What lives here
//!
//! * `get_or_create_session` — look up or mint a session, returning
//!   its current LLM thread and active namespace bindings.
//! * `flush_thread` — persist the final thread back to the session
//!   after a run, optionally filtering/summarising reflection messages
//!   when the session has reflections enabled (see task 6.5).
//! * `flush_thread_with_reflections` — reflection-aware variant:
//!   extracts `[REFLECTION]` system messages; if ≥ 2 are present it
//!   calls the LLM once to summarise the older ones, writes the decay
//!   summary to the session's write-namespace under
//!   `"reflect/{run_id}/decay"`, replaces them with a single summary
//!   sentinel, and keeps the newest verbatim.
//! * `maybe_compact_and_flush` — pre-run context-window compaction;
//!   writes the compaction summary via `MemoryManager::write` when a
//!   write-namespace is configured (task 4.3).

use std::sync::Arc;

use tracing::{debug, info};

use crate::agent_loop::{maybe_compact, DEFAULT_RECENCY_TURNS, DEFAULT_THRESHOLD_PCT};
use crate::llm::{ChatMessage, ChatRole};
use crate::memory::MemoryManager;
use crate::runtime::authority::RuntimeAuthority;
use crate::runtime::state::{MemoryNamespaceId, SessionId};

/// Outcome returned by [`SessionManager::get_or_create_session`].
#[derive(Debug)]
pub struct SessionInfo {
    pub thread: Vec<ChatMessage>,
    pub read_namespace: Option<MemoryNamespaceId>,
    pub write_namespace: Option<MemoryNamespaceId>,
}

/// Session-lifecycle helper.
///
/// Cheap to clone — wraps `Arc`s internally.
#[derive(Clone, Debug)]
pub struct SessionManager {
    authority: RuntimeAuthority,
    memory: Option<Arc<MemoryManager>>,
}

impl SessionManager {
    /// Create a new `SessionManager` backed by the given authority.
    pub fn new(authority: RuntimeAuthority, memory: Option<Arc<MemoryManager>>) -> Self {
        Self { authority, memory }
    }

    /// Look up an existing session or create a new one. Returns the
    /// session's current LLM thread and active namespace bindings.
    pub fn get_or_create_session(
        &self,
        session_id: impl Into<SessionId>,
        space_id: crate::runtime::state::SpaceId,
        name: Option<String>,
    ) -> Result<SessionInfo, crate::runtime::authority::AuthorityError> {
        let session_id = session_id.into();
        let (thread, read_namespace, write_namespace) = {
            // Authority manages the lock; we delegate the create/lookup.
            let thread =
                self.authority
                    .get_or_create_session(session_id.clone(), space_id, name)?;
            // Fetch the namespace bindings that were set on the session.
            let (rns, wns) = self.authority.session_namespaces(&session_id);
            (thread, rns, wns)
        };
        Ok(SessionInfo {
            thread,
            read_namespace,
            write_namespace,
        })
    }

    /// Persist the final thread back into the session. If a
    /// `write_namespace` is provided and reflection messages are
    /// present in `thread`, delegate to `flush_thread_with_reflections`.
    pub async fn flush_thread(
        &self,
        session_id: &SessionId,
        thread: Vec<ChatMessage>,
    ) -> Result<(), crate::runtime::authority::AuthorityError> {
        self.authority.flush_thread(session_id, thread)
    }

    /// Reflection-aware flush (task 6.5).
    ///
    /// * Splits the thread into reflection messages and non-reflection
    ///   messages.
    /// * If ≥ 2 reflection messages are present: calls the LLM to
    ///   produce a decay summary of all-but-last, writes it to
    ///   `write_namespace` at `"reflect/{run_id}/decay"`, replaces
    ///   older reflections in the thread with a single sentinel, keeps
    ///   the newest verbatim.
    /// * Persists the resulting thread via `authority.flush_thread`.
    pub async fn flush_thread_with_reflections(
        &self,
        session_id: &SessionId,
        run_id: &crate::runtime::state::RunId,
        thread: Vec<ChatMessage>,
        write_namespace: Option<&MemoryNamespaceId>,
        llm: Arc<dyn crate::llm::LlmProvider>,
        model: &str,
    ) -> Result<(), crate::runtime::authority::AuthorityError> {
        // Partition into reflection indices (messages that are System + start with "[REFLECTION]")
        let reflection_indices: Vec<usize> = thread
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                if matches!(m.role, ChatRole::System)
                    && m.content
                        .as_deref()
                        .map(|s| s.starts_with("[REFLECTION]"))
                        .unwrap_or(false)
                {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();

        if reflection_indices.len() < 2 {
            // 0 or 1 reflections — nothing to decay, just flush.
            return self.authority.flush_thread(session_id, thread);
        }

        // We have ≥ 2 reflections. Summarise all but the last.
        let newest_idx = *reflection_indices.last().unwrap();
        let older_indices: Vec<usize> = reflection_indices[..reflection_indices.len() - 1].to_vec();

        let older_texts: Vec<String> = older_indices
            .iter()
            .map(|&i| thread[i].content.clone().unwrap_or_default())
            .collect();

        let decay_summary = self
            .llm_summarise_reflections(&older_texts, llm, model)
            .await;

        // Write the decay summary to memory if a write namespace is configured.
        if let (Some(wns), Some(mm)) = (write_namespace, self.memory.as_ref()) {
            let key = format!("reflect/{}/decay", run_id);
            let _ = mm.write(&wns.0, key.clone(), decay_summary.clone()).await;
            // Emit a trace so the UI can show memory-write events.
            self.authority.emit_for_run(
                *run_id,
                crate::protocol::events::RuntimeEventPayload::Trace {
                    run_id: run_id.to_string(),
                    trace: serde_json::json!({
                        "kind": "memory_write",
                        "namespace": wns.0,
                        "key": key,
                        "source": "reflection_decay",
                    }),
                },
            );
        }

        // Build the compacted thread: replace older reflections with a
        // single summary sentinel, keep newest verbatim.
        let sentinel_msg = ChatMessage::system(format!("[REFLECTION:SUMMARY] {}", decay_summary));

        let mut compacted: Vec<ChatMessage> = Vec::with_capacity(thread.len());
        let mut sentinel_inserted = false;
        for (i, msg) in thread.into_iter().enumerate() {
            if older_indices.contains(&i) {
                // Replace the first older reflection with the sentinel,
                // drop subsequent ones.
                if !sentinel_inserted {
                    compacted.push(sentinel_msg.clone());
                    sentinel_inserted = true;
                }
            } else if i == newest_idx {
                // Adjust for inserted sentinel — index shift is fine
                // since we iterate by original index.
                compacted.push(msg);
            } else {
                compacted.push(msg);
            }
        }

        info!(
            session = %session_id,
            older = older_indices.len(),
            "flush_thread_with_reflections: decay summary written, thread compacted"
        );
        self.authority.flush_thread(session_id, compacted)
    }

    /// Pre-run context-window compaction (task 4.3).
    ///
    /// If the thread approaches the model's context limit, compacts it
    /// using the LLM and writes the summary to the configured
    /// `write_namespace` via `MemoryManager::write` (instead of the
    /// legacy `authority.put_memory`).
    ///
    /// Returns the (possibly compacted) thread.
    pub async fn maybe_compact_and_flush(
        &self,
        session_id: &SessionId,
        run_id: Option<&crate::runtime::state::RunId>,
        thread: Vec<ChatMessage>,
        write_namespace: Option<&MemoryNamespaceId>,
        llm: Arc<dyn crate::llm::LlmProvider>,
        model: &str,
    ) -> Vec<ChatMessage> {
        if thread.is_empty() {
            return thread;
        }

        let result = maybe_compact(
            thread,
            llm,
            model,
            DEFAULT_THRESHOLD_PCT,
            DEFAULT_RECENCY_TURNS,
        )
        .await;

        if result.compacted {
            // Persist the compacted thread.
            let _ = self
                .authority
                .flush_thread(session_id, result.thread.clone());

            // Write summary to MemoryManager (task 4.3).
            if let Some(summary) = &result.summary {
                if let (Some(wns), Some(mm)) = (write_namespace, self.memory.as_ref()) {
                    let count = self
                        .authority
                        .session_thread(session_id)
                        .map(|t| t.len())
                        .unwrap_or(0);
                    let key = format!("compaction/{}", count);
                    let value = serde_json::json!({ "summary": summary }).to_string();
                    let _ = mm.write(&wns.0, key.clone(), value).await;
                    // Emit a trace so the UI can show memory-write events.
                    if let Some(rid) = run_id {
                        self.authority.emit_for_run(
                            *rid,
                            crate::protocol::events::RuntimeEventPayload::Trace {
                                run_id: rid.to_string(),
                                trace: serde_json::json!({
                                    "kind": "memory_write",
                                    "namespace": wns.0,
                                    "key": key,
                                    "source": "compaction",
                                }),
                            },
                        );
                    }
                    debug!(
                        session = %session_id,
                        namespace = %wns,
                        "compaction summary written to memory"
                    );
                }
            }
        }

        result.thread
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    /// Call the LLM to produce a concise summary of a set of
    /// reflection messages. Falls back to a concatenation when the
    /// LLM call fails.
    async fn llm_summarise_reflections(
        &self,
        reflections: &[String],
        llm: Arc<dyn crate::llm::LlmProvider>,
        model: &str,
    ) -> String {
        use crate::llm::{LlmEvent, LlmRequest};
        use futures_util::StreamExt;

        let combined = reflections
            .iter()
            .enumerate()
            .map(|(i, r)| format!("{}. {}", i + 1, r))
            .collect::<Vec<_>>()
            .join("\n\n");

        let prompt = format!(
            "Summarise the following agent self-reflection notes into one concise paragraph \
             that captures the most important insights and lessons. \
             Do not repeat each note verbatim.\n\n{combined}"
        );

        let req = LlmRequest {
            model: model.to_owned(),
            messages: vec![
                ChatMessage::system(crate::crony::prompts::REFLECTION_DECAY_SUMMARIZE),
                ChatMessage::user(prompt),
            ],
            tools: Vec::new(),
            temperature: None,
            thinking: None,
        };

        let mut stream = match llm.stream(req).await {
            Ok(s) => s,
            Err(e) => {
                info!(error = %e, "llm_summarise_reflections: LLM call failed, falling back to concat");
                return reflections.join(" | ");
            }
        };

        let mut summary = String::new();
        while let Some(event) = stream.next().await {
            if let LlmEvent::Delta { content } = event {
                summary.push_str(&content);
            }
        }

        let summary = summary.trim().to_owned();
        if summary.is_empty() {
            reflections.join(" | ")
        } else {
            summary
        }
    }
}

/// Metadata returned by authority for a session's namespace bindings.
/// Exposed as a free fn so authority.rs can add the accessor without
/// changing its public surface.
impl RuntimeAuthority {
    /// Return `(read_namespace, write_namespace)` for the given session.
    /// Returns `(None, None)` if the session does not exist.
    pub fn session_namespaces(
        &self,
        session_id: &SessionId,
    ) -> (Option<MemoryNamespaceId>, Option<MemoryNamespaceId>) {
        // We need access to the inner snapshot. Use snapshot() which is a
        // cheap clone.
        let snap = self.snapshot();
        snap.sessions
            .get(session_id)
            .map(|s| (s.read_namespace.clone(), s.write_namespace.clone()))
            .unwrap_or((None, None))
    }

    /// Update the namespace fields of an existing session.
    /// `target` may be `"read"`, `"write"`, or `"both"`.
    /// Returns `false` if the session does not exist.
    pub fn swap_session_namespace(
        &self,
        session_id: &SessionId,
        target: &str,
        namespace_id: MemoryNamespaceId,
    ) -> bool {
        // Delegate to inner so the persistence flush is consistent.
        self.update_session_namespaces(session_id, target, namespace_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::authority::RuntimeAuthority;
    use crate::runtime::state::{Space, SpaceId};

    fn make_authority_and_space() -> (RuntimeAuthority, SpaceId) {
        let auth = RuntimeAuthority::in_memory();
        let space_id = SpaceId::new();
        auth.upsert_space(Space {
            id: space_id,
            name: "test".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        })
        .unwrap();
        (auth, space_id)
    }

    #[tokio::test]
    async fn session_created_with_empty_thread_and_no_namespaces() {
        let (auth, space_id) = make_authority_and_space();
        let mgr = SessionManager::new(auth, None);
        let sid = SessionId::from(uuid::Uuid::new_v4().to_string().as_str());
        let info = mgr
            .get_or_create_session(sid.clone(), space_id, None)
            .unwrap();
        assert!(info.thread.is_empty());
        assert!(info.read_namespace.is_none());
        assert!(info.write_namespace.is_none());
    }

    #[tokio::test]
    async fn flush_thread_with_zero_reflections_is_passthrough() {
        use crate::llm::mock::MockLlmProvider;
        let (auth, space_id) = make_authority_and_space();
        let sid = SessionId::from(uuid::Uuid::new_v4().to_string().as_str());
        auth.get_or_create_session(sid.clone(), space_id, None)
            .unwrap();
        let mgr = SessionManager::new(auth.clone(), None);
        let thread = vec![
            ChatMessage::user("hello"),
            ChatMessage::assistant_text("hi there"),
        ];
        let run_id = crate::runtime::state::RunId::new();
        mgr.flush_thread_with_reflections(
            &sid,
            &run_id,
            thread.clone(),
            None,
            Arc::new(MockLlmProvider::default()),
            "test-model",
        )
        .await
        .unwrap();
        let saved = auth.session_thread(&sid).unwrap();
        assert_eq!(saved.len(), 2);
    }

    #[tokio::test]
    async fn flush_thread_with_one_reflection_is_passthrough() {
        use crate::llm::mock::MockLlmProvider;
        let (auth, space_id) = make_authority_and_space();
        let sid = SessionId::from(uuid::Uuid::new_v4().to_string().as_str());
        auth.get_or_create_session(sid.clone(), space_id, None)
            .unwrap();
        let mgr = SessionManager::new(auth.clone(), None);
        let thread = vec![
            ChatMessage::user("task"),
            ChatMessage::system("[REFLECTION] I should be more concise."),
        ];
        let run_id = crate::runtime::state::RunId::new();
        mgr.flush_thread_with_reflections(
            &sid,
            &run_id,
            thread.clone(),
            None,
            Arc::new(MockLlmProvider::default()),
            "test-model",
        )
        .await
        .unwrap();
        let saved = auth.session_thread(&sid).unwrap();
        // One reflection: passthrough, no summarisation.
        assert_eq!(saved.len(), 2);
        assert!(saved[1]
            .content
            .as_deref()
            .unwrap_or("")
            .starts_with("[REFLECTION]"));
    }

    #[tokio::test]
    async fn flush_thread_with_two_reflections_produces_sentinel() {
        use crate::llm::mock::MockLlmProvider;
        let (auth, space_id) = make_authority_and_space();
        let sid = SessionId::from(uuid::Uuid::new_v4().to_string().as_str());
        auth.get_or_create_session(sid.clone(), space_id, None)
            .unwrap();
        let mgr = SessionManager::new(auth.clone(), None);
        // 4 messages: user, reflection-1, assistant, reflection-2
        let thread = vec![
            ChatMessage::user("task"),
            ChatMessage::system("[REFLECTION] First reflection."),
            ChatMessage::assistant_text("intermediate"),
            ChatMessage::system("[REFLECTION] Second reflection."),
        ];
        let run_id = crate::runtime::state::RunId::new();
        mgr.flush_thread_with_reflections(
            &sid,
            &run_id,
            thread,
            None,
            Arc::new(MockLlmProvider::default()),
            "test-model",
        )
        .await
        .unwrap();
        let saved = auth.session_thread(&sid).unwrap();
        // Original: 4 messages (user, reflection-1, assistant, reflection-2)
        // After: sentinel replaces reflection-1, reflection-2 kept
        // → 4 messages (user, sentinel, assistant, reflection-2)
        assert_eq!(saved.len(), 4, "thread length should be preserved");
        // sentinel is where reflection-1 was
        let sentinel_content = saved[1].content.as_deref().unwrap_or("");
        assert!(
            sentinel_content.starts_with("[REFLECTION:SUMMARY]"),
            "sentinel should be at index 1, got: {}",
            sentinel_content
        );
        // newest (reflection-2) kept verbatim
        let newest_content = saved[3].content.as_deref().unwrap_or("");
        assert!(
            newest_content.starts_with("[REFLECTION]"),
            "newest reflection should be verbatim at index 3"
        );
    }
}
