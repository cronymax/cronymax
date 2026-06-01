//! `cronymax.agents.provider` runtime registry — the keystone L2 EP.
//!
//! Mirrors `cep-idl/v1/agents.ts`. The same registry is consumed by both
//! the chat panel and the flow runtime; that symmetry is a spec invariant
//! (plan §2.4). Layout:
//!
//! ```text
//! Extension declares contributes["cronymax.agents.provider"] in manifest
//!     ↓
//! On activate, ext calls `cronymax.agents.registerProvider(id, impl)`
//!     ↓ RPC notify "agents/registerProvider"
//! Platform inserts ProviderEntry { provider_id, owning_ext, …metadata… }
//!     ↓
//! Chat panel / flow runtime calls AgentProviderRegistry::get(...), then
//! drives the session via
//! `ExtensionRuntime::send_to_extension(&entry.owning_ext, "agents/session.create", …)`
//! and receives streamed events back via "agents/event" notify
//! ```
//!
//! `ProviderEntry` is **pure metadata** — it does not hold the host
//! connection. Connections live exclusively in
//! [`crate::extensions::runtime::ExtensionRuntime`]'s handle map, which
//! is the single source of truth for "where is this extension's RPC
//! channel". This avoids the chicken-and-egg between "build handlers"
//! and "spawn host" (handlers used to need to capture an `Arc<Connection>`
//! that didn't exist yet); now handlers just stash metadata and the
//! runtime supplies the conn at call time.
//!
//! Wire layer (RPC method names) lives in [`crate::extensions::rpc::codec::agents_method`].

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::extensions::error::{ExtensionError, ExtensionResult};

/// One registered agent provider. Pure metadata — the RPC connection
/// is looked up via the runtime's handle map at call time (see
/// [`crate::extensions::runtime::ExtensionRuntime::send_to_extension`]).
#[derive(Debug, Clone)]
pub struct ProviderEntry {
    pub provider_id: String,
    pub owning_ext: String,
    /// Human-readable label from manifest (`AgentProviderContribution.label`).
    pub label: String,
    /// Optional manifest icon path.
    pub icon: Option<String>,
    /// Optional manifest description.
    pub description: Option<String>,
    /// Capability flags from manifest.
    pub supports_models: bool,
    pub supports_modes: bool,
    pub supports_mcp: bool,
}

/// Thread-safe map of `provider_id → entry`. Cheap to share.
#[derive(Debug, Default, Clone)]
pub struct AgentProviderRegistry {
    inner: Arc<RwLock<HashMap<String, ProviderEntry>>>,
}

impl AgentProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a provider from an extension. Errors if `provider_id` is
    /// already taken by a different extension, or if the id is in the
    /// `cronymax.*` reserved namespace.
    pub fn register(&self, entry: ProviderEntry) -> ExtensionResult<()> {
        if entry.provider_id.starts_with("cronymax.") || entry.provider_id == "cronymax" {
            return Err(ExtensionError::NamespaceReserved(format!(
                "agent provider id `{}` is reserved by the platform",
                entry.provider_id
            )));
        }
        let mut g = self.inner.write();
        if let Some(existing) = g.get(&entry.provider_id) {
            if existing.owning_ext != entry.owning_ext {
                return Err(ExtensionError::BadContribution {
                    point: "cronymax.agents.provider".into(),
                    ext_id: entry.owning_ext.clone(),
                    reason: format!(
                        "provider id `{}` is already owned by `{}`",
                        entry.provider_id, existing.owning_ext
                    ),
                });
            }
            // Same owner re-registering — overwrite (e.g. on re-activate).
        }
        g.insert(entry.provider_id.clone(), entry);
        Ok(())
    }

    /// Unregister a single provider. Returns whether it was present.
    /// Errors if a different extension claims to be removing it.
    pub fn unregister(&self, ext_id: &str, provider_id: &str) -> ExtensionResult<bool> {
        let mut g = self.inner.write();
        let Some(existing) = g.get(provider_id) else {
            return Ok(false);
        };
        if existing.owning_ext != ext_id {
            return Err(ExtensionError::BadContribution {
                point: "cronymax.agents.provider".into(),
                ext_id: ext_id.to_string(),
                reason: format!(
                    "provider `{provider_id}` is owned by `{}`, not `{ext_id}`",
                    existing.owning_ext
                ),
            });
        }
        g.remove(provider_id);
        Ok(true)
    }

    /// Drop every provider owned by `ext_id` (called on deactivate).
    pub fn unregister_all_for(&self, ext_id: &str) -> usize {
        let mut g = self.inner.write();
        let before = g.len();
        g.retain(|_, e| e.owning_ext != ext_id);
        before - g.len()
    }

    pub fn get(&self, provider_id: &str) -> Option<ProviderEntry> {
        self.inner.read().get(provider_id).cloned()
    }

    /// All providers known to the registry, sorted by id. Used by the chat
    /// panel to render a "provider picker" dropdown.
    pub fn list(&self) -> Vec<ProviderEntry> {
        let mut v: Vec<ProviderEntry> = self.inner.read().values().cloned().collect();
        v.sort_by(|a, b| a.provider_id.cmp(&b.provider_id));
        v
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}

// ── streaming session event routing ─────────────────────────────────────

/// One streamed event coming back from an extension's `AgentSession.prompt`
/// async iterator. Wire-compatible with the IDL `AgentEvent` discriminated
/// union (`cep-idl/v1/agents.ts`); serde's `tag = "kind"` matches the JS
/// shape exactly, so an `agents/event` notify can deserialize straight into
/// this enum without reshaping.
///
/// IDL note: the JS side may emit fields with `null` (e.g. `data: null` on a
/// CRDT-shaped tool output). `serde_json::Value` captures any inner shape,
/// so we don't lose typed payloads even when their structure isn't known
/// statically to the Rust side.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AgentSessionEvent {
    Text {
        text: String,
    },
    Thinking {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        input: serde_json::Value,
        source: String,
        #[serde(default)]
        status: Option<String>,
    },
    ToolCallUpdate {
        id: String,
        status: String,
        #[serde(default)]
        output: serde_json::Value,
    },
    PermissionRequest {
        #[serde(rename = "requestId")]
        request_id: String,
        tool: String,
        #[serde(default)]
        options: serde_json::Value,
    },
    Done {
        #[serde(rename = "stopReason")]
        stop_reason: String,
        #[serde(default, rename = "errorMessage")]
        error_message: Option<String>,
    },
}

/// Internal channel item the chat / flow dispatcher consumes. `Event(_)`
/// carries one streamed `AgentSessionEvent`; `TurnDone` signals the
/// matching `agents/turn.done` notify and that the channel can be closed.
/// Splitting these into two variants (rather than overloading the `Done`
/// event) preserves the explicit turn-boundary signal even when the
/// extension never emitted a `{kind:"done"}` event itself.
#[derive(Clone, Debug)]
pub enum AgentSessionMessage {
    Event(AgentSessionEvent),
    TurnDone,
}

/// Routes inbound `agents/event` and `agents/turn.done` notifies from
/// extension hosts to the in-flight dispatcher waiting on a specific
/// `session_id`. The dispatcher calls [`Self::register`] before sending
/// `agents/session.prompt`, then iterates the returned receiver until it
/// observes a `TurnDone` or a `{kind:"done"}` event. On run completion
/// (or cancellation / error), the dispatcher calls [`Self::unregister`].
///
/// Bounded channels — picked 64 because a single turn can emit hundreds of
/// `text` deltas plus tool-call updates; 64 is large enough that bursty
/// extensions don't block the per-extension RPC dispatch task on backpressure
/// during normal operation, but small enough that a runaway extension can't
/// blow memory. Cancelling the run drops the receiver, which closes the
/// channel and surfaces the next `send` as a routing miss (logged & dropped).
/// One registered session sink plus the id of the extension that owns the
/// provider driving it. The owning id lets [`AgentSessionRouter::close_all_for`]
/// sever every in-flight turn for an extension when it deactivates.
#[derive(Debug, Clone)]
struct SessionSink {
    owning_ext: String,
    tx: mpsc::Sender<AgentSessionMessage>,
}

#[derive(Debug, Default, Clone)]
pub struct AgentSessionRouter {
    inner: Arc<RwLock<HashMap<String, SessionSink>>>,
}

impl AgentSessionRouter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to events for `session_id`. Returns a receiver the
    /// dispatcher iterates; the corresponding sender is held by the
    /// router and consumed by inbound notifies. Errors if a different
    /// dispatcher already registered the same `session_id` — that would
    /// indicate either an extension bug (recycling ids) or a stale
    /// dispatcher entry; either way silently overwriting is worse than
    /// surfacing the conflict.
    pub fn register(
        &self,
        session_id: impl Into<String>,
        owning_ext: impl Into<String>,
    ) -> ExtensionResult<mpsc::Receiver<AgentSessionMessage>> {
        let session_id = session_id.into();
        let (tx, rx) = mpsc::channel(64);
        let mut g = self.inner.write();
        if g.contains_key(&session_id) {
            return Err(ExtensionError::BadContribution {
                point: "cronymax.agents.session".into(),
                ext_id: String::new(),
                reason: format!("session id `{session_id}` already has an active sink"),
            });
        }
        g.insert(
            session_id,
            SessionSink {
                owning_ext: owning_ext.into(),
                tx,
            },
        );
        Ok(rx)
    }

    /// Drop the sink for `session_id`. Subsequent inbound notifies for
    /// that id will be logged & ignored. Returns whether anything was
    /// removed.
    pub fn unregister(&self, session_id: &str) -> bool {
        self.inner.write().remove(session_id).is_some()
    }

    /// Sever every in-flight session whose provider belongs to `ext_id`.
    /// Dropping the held sender closes each channel, so the dispatcher's
    /// `sink.recv()` returns `None` and its event loop unwinds (the run then
    /// fails on the dropped RPC connection rather than hanging forever).
    /// Called from [`crate::extensions::runtime::ExtensionRuntime::deactivate`]
    /// before the Node host is shut down. Returns the number of sinks closed.
    pub fn close_all_for(&self, ext_id: &str) -> usize {
        let mut g = self.inner.write();
        let before = g.len();
        g.retain(|_, sink| sink.owning_ext != ext_id);
        before - g.len()
    }

    /// Forward one event to the dispatcher. Returns:
    /// * `Ok(true)` — delivered
    /// * `Ok(false)` — no sink registered (caller logs and drops)
    /// * `Err(_)` — sink is full or closed; treat as fatal-for-this-turn
    pub async fn route_event(
        &self,
        session_id: &str,
        event: AgentSessionEvent,
    ) -> ExtensionResult<bool> {
        let sender = {
            let g = self.inner.read();
            g.get(session_id).map(|s| s.tx.clone())
        };
        match sender {
            Some(tx) => {
                tx.send(AgentSessionMessage::Event(event))
                    .await
                    .map_err(|_| ExtensionError::BadContribution {
                        point: "cronymax.agents.session".into(),
                        ext_id: String::new(),
                        reason: format!("session id `{session_id}` channel closed"),
                    })?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Forward a `turn.done` marker. Same delivery semantics as
    /// [`Self::route_event`]; dispatchers may treat this as a hard turn
    /// boundary even if no `{kind:"done"}` event arrived first.
    pub async fn route_turn_done(&self, session_id: &str) -> ExtensionResult<bool> {
        let sender = {
            let g = self.inner.read();
            g.get(session_id).map(|s| s.tx.clone())
        };
        match sender {
            Some(tx) => {
                tx.send(AgentSessionMessage::TurnDone).await.map_err(|_| {
                    ExtensionError::BadContribution {
                        point: "cronymax.agents.session".into(),
                        ext_id: String::new(),
                        reason: format!("session id `{session_id}` channel closed"),
                    }
                })?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(provider_id: &str, ext_id: &str) -> ProviderEntry {
        ProviderEntry {
            provider_id: provider_id.into(),
            owning_ext: ext_id.into(),
            label: format!("Label for {provider_id}"),
            icon: None,
            description: None,
            supports_models: true,
            supports_modes: false,
            supports_mcp: false,
        }
    }

    #[test]
    fn register_then_lookup() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("alice.x.gpt", "alice.x")).unwrap();
        let got = reg.get("alice.x.gpt").unwrap();
        assert_eq!(got.label, "Label for alice.x.gpt");
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn cronymax_namespace_rejected() {
        let reg = AgentProviderRegistry::new();
        let err = reg
            .register(entry("cronymax.builtin", "alice.x"))
            .unwrap_err();
        assert!(matches!(err, ExtensionError::NamespaceReserved(_)));
    }

    #[test]
    fn cross_extension_id_collision_rejected() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("shared", "alice.x")).unwrap();
        let err = reg.register(entry("shared", "bob.y")).unwrap_err();
        assert!(matches!(err, ExtensionError::BadContribution { .. }));
        // First entry still present.
        assert_eq!(reg.get("shared").unwrap().owning_ext, "alice.x");
    }

    #[test]
    fn same_extension_can_reregister() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("p1", "alice.x")).unwrap();
        // Re-register (simulating re-activate after crash).
        reg.register(entry("p1", "alice.x")).unwrap();
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn unregister_only_works_for_owner() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("p1", "alice.x")).unwrap();
        assert!(reg.unregister("bob.y", "p1").is_err());
        assert!(reg.unregister("alice.x", "p1").unwrap());
        assert!(reg.get("p1").is_none());
        // Second unregister: returns Ok(false) — not present.
        assert!(!reg.unregister("alice.x", "p1").unwrap());
    }

    #[test]
    fn unregister_all_for_drops_only_that_extensions_providers() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("a1", "alice.x")).unwrap();
        reg.register(entry("a2", "alice.x")).unwrap();
        reg.register(entry("b1", "bob.y")).unwrap();
        let dropped = reg.unregister_all_for("alice.x");
        assert_eq!(dropped, 2);
        assert!(reg.get("b1").is_some());
    }

    #[test]
    fn list_is_sorted_by_provider_id() {
        let reg = AgentProviderRegistry::new();
        reg.register(entry("zeta", "alice.x")).unwrap();
        reg.register(entry("alpha", "alice.x")).unwrap();
        reg.register(entry("mu", "alice.x")).unwrap();
        let ids: Vec<String> = reg.list().into_iter().map(|e| e.provider_id).collect();
        assert_eq!(ids, vec!["alpha", "mu", "zeta"]);
    }

    // ── AgentSessionEvent wire compatibility ────────────────────────────

    #[test]
    fn agent_session_event_text_roundtrips_from_idl_shape() {
        let json = serde_json::json!({"kind": "text", "text": "hi"});
        let ev: AgentSessionEvent = serde_json::from_value(json.clone()).unwrap();
        match &ev {
            AgentSessionEvent::Text { text } => assert_eq!(text, "hi"),
            other => panic!("expected Text, got {other:?}"),
        }
        // serialized form must match the JS shape so it can round-trip
        // back out of Rust without reshaping.
        let back = serde_json::to_value(&ev).unwrap();
        assert_eq!(back, json);
    }

    #[test]
    fn agent_session_event_done_decodes_with_optional_error() {
        let json = serde_json::json!({
            "kind": "done",
            "stopReason": "error",
            "errorMessage": "model timeout",
        });
        let ev: AgentSessionEvent = serde_json::from_value(json).unwrap();
        match ev {
            AgentSessionEvent::Done {
                stop_reason,
                error_message,
            } => {
                assert_eq!(stop_reason, "error");
                assert_eq!(error_message.as_deref(), Some("model timeout"));
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn agent_session_event_tool_call_decodes_input_as_value() {
        let json = serde_json::json!({
            "kind": "toolCall",
            "id": "t-1",
            "name": "shell",
            "input": {"cmd": ["ls", "/"]},
            "source": "cronymax.tool.shell",
            "status": "in_progress",
        });
        let ev: AgentSessionEvent = serde_json::from_value(json).unwrap();
        match ev {
            AgentSessionEvent::ToolCall {
                id, name, source, ..
            } => {
                assert_eq!(id, "t-1");
                assert_eq!(name, "shell");
                assert_eq!(source, "cronymax.tool.shell");
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    // ── AgentSessionRouter ──────────────────────────────────────────────

    #[tokio::test]
    async fn router_routes_event_to_registered_sink() {
        let router = AgentSessionRouter::new();
        let mut rx = router.register("s-1", "ext.a").unwrap();
        let delivered = router
            .route_event(
                "s-1",
                AgentSessionEvent::Text {
                    text: "hello".into(),
                },
            )
            .await
            .unwrap();
        assert!(delivered);
        match rx.recv().await {
            Some(AgentSessionMessage::Event(AgentSessionEvent::Text { text })) => {
                assert_eq!(text, "hello");
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[tokio::test]
    async fn router_route_event_on_unknown_session_returns_false() {
        let router = AgentSessionRouter::new();
        let delivered = router
            .route_event(
                "ghost",
                AgentSessionEvent::Done {
                    stop_reason: "end_turn".into(),
                    error_message: None,
                },
            )
            .await
            .unwrap();
        assert!(!delivered);
    }

    #[tokio::test]
    async fn router_route_turn_done_signals_dispatcher() {
        let router = AgentSessionRouter::new();
        let mut rx = router.register("s-2", "ext.a").unwrap();
        router.route_turn_done("s-2").await.unwrap();
        match rx.recv().await {
            Some(AgentSessionMessage::TurnDone) => {}
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn router_register_duplicate_session_id_rejected() {
        let router = AgentSessionRouter::new();
        let _rx = router.register("s-dup", "ext.a").unwrap();
        let err = router.register("s-dup", "ext.a").unwrap_err();
        assert!(matches!(err, ExtensionError::BadContribution { .. }));
    }

    #[tokio::test]
    async fn router_unregister_drops_sink() {
        let router = AgentSessionRouter::new();
        let _rx = router.register("s-3", "ext.a").unwrap();
        assert_eq!(router.len(), 1);
        assert!(router.unregister("s-3"));
        assert_eq!(router.len(), 0);
        // Subsequent send is a no-op (Ok(false)).
        let delivered = router
            .route_event(
                "s-3",
                AgentSessionEvent::Text {
                    text: "post".into(),
                },
            )
            .await
            .unwrap();
        assert!(!delivered);
    }

    #[tokio::test]
    async fn router_close_all_for_severs_only_that_extensions_sinks() {
        let router = AgentSessionRouter::new();
        let mut rx_a = router.register("s-a", "ext.a").unwrap();
        let mut rx_a2 = router.register("s-a2", "ext.a").unwrap();
        let mut rx_b = router.register("s-b", "ext.b").unwrap();
        assert_eq!(router.len(), 3);

        let closed = router.close_all_for("ext.a");
        assert_eq!(closed, 2, "both ext.a sinks severed");
        assert_eq!(router.len(), 1, "ext.b sink survives");

        // Severed receivers observe channel close (`None`) so the dispatcher
        // loops unwind instead of hanging.
        assert!(rx_a.recv().await.is_none());
        assert!(rx_a2.recv().await.is_none());

        // ext.b still routes normally.
        assert!(router.route_turn_done("s-b").await.unwrap());
        assert!(matches!(
            rx_b.recv().await,
            Some(AgentSessionMessage::TurnDone)
        ));
    }
}
