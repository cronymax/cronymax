//! Composition root for the cronymax runtime.
//!
//! [`RuntimeServices`] groups every shared service required by
//! [`super::handler::RuntimeHandler`] and (eventually) [`super::agent_runner::AgentRunner`].
//! It is constructed once per process and shared via `Arc<RuntimeServices>`.
//!
//! Having a single composition root means:
//! * the telescoping `RuntimeHandler::with_*/with_all` constructors can be collapsed
//!   to `RuntimeHandler::new(Arc<RuntimeServices>)`;
//! * integration tests substitute `MockLlmFactory` / `FakeCapabilityFactory` without
//!   touching any real infrastructure.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::capability::factory::{CapabilityFactory, DefaultCapabilityFactory};
use crate::config::RuntimeConfig;
use crate::flow::{FlowRuntimeOnCreate, FlowRuntimeRegistry};
use crate::llm::factory::{DefaultLlmProviderFactory, LlmProviderFactory};
use crate::memory::MemoryManager;
use crate::protocol::events::RuntimeEventPayload;
use crate::runtime::authority::{flow_run_topics, RuntimeAuthority};
use crate::terminal::SharedPtySessionManager;

// ── RuntimeServices ───────────────────────────────────────────────────────────

/// Shared services injected into every runtime component.
///
/// Construct via [`RuntimeServices::new`] for production, or build manually in
/// tests by supplying mock/fake implementations via the public fields.
pub struct RuntimeServices {
    /// Authority: the single source of truth for runs, agents, permissions, etc.
    pub authority: RuntimeAuthority,

    /// Registry of lazily-initialised [`crate::flow::runtime::FlowRuntime`] instances.
    pub flow_registry: Arc<FlowRuntimeRegistry>,

    /// Factory that creates an [`crate::llm::provider::LlmProvider`] for a given
    /// [`crate::llm::config::LlmConfig`].
    pub llm_factory: Arc<dyn LlmProviderFactory>,

    /// Factory that assembles a `DispatcherBuilder` with tier-appropriate shell
    /// and filesystem capabilities.
    pub capability_factory: Arc<dyn CapabilityFactory>,

    /// Shared PTY session managers — keyed by workspace root string so that
    /// sessions created via the browser transport are visible to the renderer
    /// transport (and vice-versa).
    pub terminal_managers: Arc<Mutex<HashMap<String, SharedPtySessionManager>>>,

    /// Optional semantic-memory manager (present when embedding is configured).
    pub memory_manager: Option<Arc<MemoryManager>>,
}

impl RuntimeServices {
    /// Construct production [`RuntimeServices`] from `config`.
    ///
    /// Builds `DefaultLlmProviderFactory` (with a fresh `CopilotTokenCache`) and
    /// `DefaultCapabilityFactory`, plus a new `FlowRuntimeRegistry`.
    ///
    /// Pass the pre-constructed `authority` (from [`RuntimeAuthority::rehydrate`]),
    /// the shared `terminal_managers` map, and an optional `memory_manager`.
    pub fn new(
        _config: &RuntimeConfig,
        authority: RuntimeAuthority,
        terminal_managers: Arc<Mutex<HashMap<String, SharedPtySessionManager>>>,
        memory_manager: Option<Arc<MemoryManager>>,
    ) -> Arc<Self> {
        let llm_factory: Arc<dyn LlmProviderFactory> = Arc::new(DefaultLlmProviderFactory::new());
        let capability_factory: Arc<dyn CapabilityFactory> = Arc::new(DefaultCapabilityFactory);

        // Wire the FlowRuntime event emitter to the authority at composition root.
        // Emits to ["flow:{event}", "flow_run:{flow_run_id}"] and also to
        // "session:{sid}" when the flow run has an originating session.
        let auth_for_registry = authority.clone();
        let on_create: FlowRuntimeOnCreate = Arc::new(move |rt| {
            let auth = auth_for_registry.clone();
            rt.set_event_emitter(Box::new(move |event, json_payload| {
                // Extract flow_run_id from the payload JSON ({"run_id": "run-..."}).
                let flow_run_id: String = serde_json::from_str::<serde_json::Value>(json_payload)
                    .ok()
                    .and_then(|v| v.get("run_id").and_then(|r| r.as_str()).map(str::to_owned))
                    .unwrap_or_default();
                let session_id = if flow_run_id.is_empty() {
                    None
                } else {
                    auth.resolve_session(&flow_run_id)
                };
                let mut topics = flow_run_topics(&flow_run_id, session_id.as_deref());
                // Preserve the legacy flat topic for backward compatibility during migration.
                topics.push(format!("flow:{event}"));
                let data = serde_json::json!({ "event": event, "payload": json_payload });
                auth.emit_many(&topics, RuntimeEventPayload::Raw { data });
            }));
        });
        let flow_registry = Arc::new(FlowRuntimeRegistry::with_on_create(on_create));

        Arc::new(Self {
            authority,
            flow_registry,
            llm_factory,
            capability_factory,
            terminal_managers,
            memory_manager,
        })
    }

    /// Minimal constructor for legacy code paths that don't have a `RuntimeConfig`.
    /// Uses default factories (same as `new`) but skips config-dependent wiring.
    pub fn new_minimal(
        authority: RuntimeAuthority,
        terminal_managers: Arc<Mutex<HashMap<String, SharedPtySessionManager>>>,
    ) -> Arc<Self> {
        let llm_factory: Arc<dyn LlmProviderFactory> = Arc::new(DefaultLlmProviderFactory::new());
        let capability_factory: Arc<dyn CapabilityFactory> = Arc::new(DefaultCapabilityFactory);
        let flow_registry = Arc::new(FlowRuntimeRegistry::default());
        Arc::new(Self {
            authority,
            flow_registry,
            llm_factory,
            capability_factory,
            terminal_managers,
            memory_manager: None,
        })
    }
}
