---
title: runtime/handler.rs — Architectural Split Prototype
doc_type: prototype
---

# `runtime/handler.rs` — Architectural Split Prototype

## Problem Analysis

`handler.rs` is currently **~1,550 lines** and conflates at least **twelve distinct concerns** inside a single `match` arm dispatch function:

| Responsibility | Approx. lines | ControlRequest variants |
|---|---|---|
| Struct declaration & constructors | ~90 | — |
| Pure helper utilities | ~80 | — |
| Transport lifecycle (connect/disconnect/capability) | ~70 | — |
| Subscription fan-out | ~50 | `Subscribe`, `Unsubscribe` |
| **Run start (orchestration)** | **~480** | `StartRun` |
| Run operations | ~200 | `ResumeRun`, `Cancel`, `Pause`, `PostInput`, `ResolveReview`, `SwapMemory` |
| Workspace / file I/O | ~40 | `WorkspaceLayout`, `FileRead`, `FileWrite` |
| Flow CRUD | ~180 | `FlowList`, `FlowLoad`, `FlowSave` |
| Agent registry | ~110 | `AgentRegistryList/Load/Save/Delete` |
| Doc-type registry | ~70 | `DocTypeList/Load/Save/Delete` |
| Terminal PTY | ~90 | `TerminalStart/Input/Resize/Stop` |
| Document store | ~110 | `DocumentList/Read/Submit/SuggestionApply`, `MentionParse` |
| Flow-run review | ~200 | `FlowRunGetPendingReviews`, `FlowRunApprove`, `FlowRunRequestChanges`, `GetSessionPendingActions` |
| Session / space | ~90 | `GetSpaceSnapshot`, `SessionList`, `SessionThreadInspect`, `ListProviderModels` |

---

## Guiding Principles

1. **`RuntimeHandler` becomes a thin router.** Its only job is to hold shared state and forward each `ControlRequest` variant to the correct sub-handler method.  
2. **Sub-handlers are `impl RuntimeHandler` blocks in separate files.** No new traits or indirection layers — Rust's `mod` system lets us put `impl RuntimeHandler` anywhere, keeping the struct owned in one place.  
3. **Pure utilities live in a dedicated `helpers` module.** Zero state, fully testable.  
4. **Files map 1-to-1 to a cohesive responsibility.** A developer looking for "how does FlowRunApprove work" opens `review_ops.rs` without reading 1,500 lines.  
5. **`mod.rs` stays as the DI / wiring layer** — struct definition, constructors, `Handler` trait impl (thin routing only).

---

## Target Module Tree

```
crates/cronymax/src/runtime/handler/
│
├── mod.rs            ← RuntimeHandler struct + constructors + Handler trait impl
│                       (handle_control is a 30-line match that delegates to methods below)
│
├── helpers.rs        ← Pure functions with no state:
│                         parse_run / parse_space / parse_review
│                         authority_err_to_control
│                         base64_encode
│                         build_workspace_injection_block
│                         apply_anthropic_effort_override
│                         build_middleware_chain
│                         default_chat_system_prompt
│
├── subscription.rs   ← handle_subscribe / handle_unsubscribe
│                       Fan-out task spawning helper
│
├── run_start.rs      ← handle_start_run (the 480-line mega-handler)
│                       LlmParams (private struct for payload extraction)
│                       spawn_supervision_task (extracted helper)
│
├── run_ops.rs        ← handle_resume_run
│                       handle_cancel_run / handle_pause_run
│                       handle_post_input
│                       handle_resolve_review
│                       handle_swap_memory
│
├── workspace_ops.rs  ← handle_workspace_layout
│                       handle_file_read / handle_file_write
│
├── flow_ops.rs       ← handle_flow_list / handle_flow_load / handle_flow_save
│
├── registry_ops.rs   ← handle_agent_registry_{list,load,save,delete}
│                       handle_doc_type_{list,load,save,delete}
│
├── terminal_ops.rs   ← handle_terminal_{start,input,resize,stop}
│
├── document_ops.rs   ← handle_document_{list,read,submit,suggestion_apply}
│                       handle_mention_parse
│
├── review_ops.rs     ← handle_flow_run_get_pending_reviews
│                       handle_get_session_pending_actions
│                       handle_flow_run_approve
│                       handle_flow_run_request_changes
│
└── session_ops.rs    ← handle_get_space_snapshot
                        handle_session_list / handle_session_thread_inspect
                        handle_list_provider_models
```

---

## Key Design Details

### `mod.rs` — the thin router

```rust
// runtime/handler/mod.rs

mod helpers;
mod subscription;
mod run_start;
mod run_ops;
mod workspace_ops;
mod flow_ops;
mod registry_ops;
mod terminal_ops;
mod document_ops;
mod review_ops;
mod session_ops;

pub use helpers::{build_middleware_chain}; // only re-export what external callers need

pub struct RuntimeHandler {
    pub(crate) authority: RuntimeAuthority,
    pub(crate) services: Arc<RuntimeServices>,
    pub(crate) agent_runner: AgentRunner,
    pub(crate) workspace_roots: Vec<PathBuf>,
    pub(crate) workspace_cache_dir: Option<PathBuf>,
    pub(crate) sandbox_policy: Option<Arc<SandboxPolicy>>,
    pub(crate) sink: Mutex<Option<ResponseSink>>,
    pub(crate) fanout: Mutex<HashMap<SubscriptionId, JoinHandle<()>>>,
    pub(crate) flow_contexts: Mutex<HashMap<String, RunContext>>,
    pub(crate) flow_run_to_agent_run: Mutex<HashMap<String, RunId>>,
    pub(crate) pending_capabilities: Mutex<HashMap<CorrelationId, oneshot::Sender<CapabilityResponse>>>,
}

// All constructors stay here — single source of truth for DI wiring.
impl RuntimeHandler {
    pub fn from_services(...) -> Self { ... }
    #[deprecated] pub fn new(...) -> Self { ... }
    #[deprecated] pub fn with_policy(...) -> Self { ... }
    #[deprecated] pub fn with_policy_and_managers(...) -> Self { ... }
    pub fn set_workspace_cache_dir(&mut self, dir: PathBuf) { ... }
    pub fn authority(&self) -> &RuntimeAuthority { &self.authority }
    pub async fn call_capability(self: &Arc<Self>, ...) -> anyhow::Result<CapabilityResponse> { ... }
}

#[async_trait]
impl Handler for RuntimeHandler {
    async fn on_connected(&self, sink: ResponseSink) {
        *self.sink.lock() = Some(sink);
    }

    // The ONLY code that stays in this file is the routing table.
    // Each arm is a one-liner.
    async fn handle_control(&self, _id: CorrelationId, req: ControlRequest) -> ControlResponse {
        match req {
            ControlRequest::Ping                    => ControlResponse::Pong,
            ControlRequest::Subscribe { topic }     => self.handle_subscribe(topic),
            ControlRequest::Unsubscribe { subscription } => self.handle_unsubscribe(subscription),

            ControlRequest::StartRun { .. }         => self.handle_start_run(req).await,
            ControlRequest::ResumeRun { .. }        => self.handle_resume_run(req).await,
            ControlRequest::CancelRun { run_id }    => self.run_op(&run_id, |a, id| a.cancel_run(id)),
            ControlRequest::PauseRun { run_id }     => self.run_op(&run_id, |a, id| a.pause_run(id)),
            ControlRequest::PostInput { .. }        => self.handle_post_input(req),
            ControlRequest::ResolveReview { .. }    => self.handle_resolve_review(req).await,
            ControlRequest::SwapMemory { .. }       => self.handle_swap_memory(req),

            ControlRequest::WorkspaceLayout { .. }  => self.handle_workspace_layout(req).await,
            ControlRequest::FileRead { .. }         => self.handle_file_read(req).await,
            ControlRequest::FileWrite { .. }        => self.handle_file_write(req).await,

            ControlRequest::FlowList { .. }         => self.handle_flow_list(req).await,
            ControlRequest::FlowLoad { .. }         => self.handle_flow_load(req).await,
            ControlRequest::FlowSave { .. }         => self.handle_flow_save(req).await,

            ControlRequest::AgentRegistryList { .. }   => self.handle_agent_registry_list(req).await,
            ControlRequest::AgentRegistryLoad { .. }   => self.handle_agent_registry_load(req).await,
            ControlRequest::AgentRegistrySave { .. }   => self.handle_agent_registry_save(req).await,
            ControlRequest::AgentRegistryDelete { .. } => self.handle_agent_registry_delete(req).await,

            ControlRequest::DocTypeList { .. }      => self.handle_doc_type_list(req).await,
            ControlRequest::DocTypeLoad { .. }      => self.handle_doc_type_load(req).await,
            ControlRequest::DocTypeSave { .. }      => self.handle_doc_type_save(req).await,
            ControlRequest::DocTypeDelete { .. }    => self.handle_doc_type_delete(req).await,

            ControlRequest::TerminalStart { .. }    => self.handle_terminal_start(req).await,
            ControlRequest::TerminalInput { .. }    => self.handle_terminal_input(req).await,
            ControlRequest::TerminalResize { .. }   => self.handle_terminal_resize(req).await,
            ControlRequest::TerminalStop { .. }     => self.handle_terminal_stop(req).await,

            ControlRequest::DocumentList { .. }         => self.handle_document_list(req).await,
            ControlRequest::DocumentRead { .. }         => self.handle_document_read(req).await,
            ControlRequest::DocumentSubmit { .. }       => self.handle_document_submit(req).await,
            ControlRequest::DocumentSuggestionApply {..}=> self.handle_document_suggestion_apply(req).await,
            ControlRequest::MentionParse { .. }         => self.handle_mention_parse(req).await,

            ControlRequest::GetSpaceSnapshot { .. }         => self.handle_get_space_snapshot(req),
            ControlRequest::SessionList { .. }              => self.handle_session_list(req).await,
            ControlRequest::SessionThreadInspect { .. }     => self.handle_session_thread_inspect(req).await,
            ControlRequest::ListProviderModels { .. }       => self.handle_list_provider_models(req).await,

            ControlRequest::FlowRunGetPendingReviews { .. } => self.handle_flow_run_get_pending_reviews(req).await,
            ControlRequest::GetSessionPendingActions { .. } => self.handle_get_session_pending_actions(req).await,
            ControlRequest::FlowRunApprove { .. }           => self.handle_flow_run_approve(req).await,
            ControlRequest::FlowRunRequestChanges { .. }    => self.handle_flow_run_request_changes(req).await,
        }
    }

    async fn handle_capability_reply(&self, id: CorrelationId, response: CapabilityResponse) { ... }
    async fn on_disconnected(&self) { ... }
}

// run_op helper stays here because it uses self.authority directly.
impl RuntimeHandler {
    pub(crate) fn run_op<F>(&self, run_id_str: &str, op: F) -> ControlResponse
    where F: FnOnce(&RuntimeAuthority, RunId) -> Result<(), AuthorityError> { ... }
}
```

### `helpers.rs` — pure functions

```rust
// runtime/handler/helpers.rs
//
// Zero-dependency utility functions. No access to RuntimeHandler state.
// All functions are `pub(super)` — only the handler sub-modules need them.

pub(super) fn build_workspace_injection_block(path: &Path, tool_names: &[&str]) -> String { ... }
pub(super) fn apply_anthropic_effort_override(cfg: Option<ThinkingConfig>, effort: Option<&str>) -> Option<ThinkingConfig> { ... }
pub(super) fn build_middleware_chain(authority: RuntimeAuthority) -> Arc<MiddlewareChain> { ... }
pub(super) fn default_chat_system_prompt() -> String { ... }
pub(super) fn base64_encode(data: &[u8]) -> String { ... }
pub(super) fn parse_run(s: &str) -> Result<RunId, ControlResponse> { ... }
pub(super) fn parse_space(s: &str) -> Result<SpaceId, ControlResponse> { ... }
pub(super) fn parse_review(s: &str) -> Result<ReviewId, ControlResponse> { ... }
pub(super) fn authority_err_to_control(e: AuthorityError, space_id: Option<&str>, run_id: Option<&str>) -> ControlError { ... }
```

### `run_start.rs` — the 480-line orchestrator, now isolated

The biggest improvement. `handle_start_run` is extracted wholesale, but we also extract two private helpers that make the function readable:

```rust
// runtime/handler/run_start.rs

impl RuntimeHandler {
    pub(super) async fn handle_start_run(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::StartRun { space_id, payload, session_id, session_name, agent_id } = req
            else { unreachable!() };

        let space  = try_ctrl!(parse_space(&space_id));
        let params = LlmParams::from_payload(&payload);         // ← extracted
        let (maybe_session_id, prior_thread) = self.resolve_session(&payload, &params, space, &session_id, &session_name).await;
        // ... rest of the handler
    }
}

/// Extracted: all LLM field extraction from the payload JSON lives here.
struct LlmParams {
    base_url: String,
    api_key: Option<String>,
    model: String,
    provider_kind: String,
    reasoning_effort: Option<String>,
    anthropic_effort: Option<String>,
}

impl LlmParams {
    fn from_payload(payload: &serde_json::Value) -> Self { ... }
}

/// Extracted: doc-channel supervision loop for flow runs.
fn spawn_supervision_task(
    flow_ctx: RunContext,
    services: Arc<RuntimeServices>,
    ar: AgentRunner,
    run_id: RunId,
    doc_rx: mpsc::Receiver<DocumentSubmitted>,
) { ... }

/// Extracted: supervision loop for non-flow (chat-originated) flow tool invocations.
fn spawn_chat_supervision_task(...) { ... }
```

### `review_ops.rs` — flow review in one place

```rust
// runtime/handler/review_ops.rs
//
// All four flow-run document review handlers. Previously scattered across
// hundreds of lines of handle_control, now trivially navigable.

impl RuntimeHandler {
    pub(super) async fn handle_flow_run_get_pending_reviews(&self, req: ControlRequest) -> ControlResponse { ... }
    pub(super) async fn handle_get_session_pending_actions(&self, req: ControlRequest) -> ControlResponse { ... }
    pub(super) async fn handle_flow_run_approve(&self, req: ControlRequest) -> ControlResponse { ... }
    pub(super) async fn handle_flow_run_request_changes(&self, req: ControlRequest) -> ControlResponse { ... }
}
```

---

## Migration Strategy

The split is **purely mechanical** — no logic changes, no new abstractions, no trait objects. Execution is safe:

| Phase | Action | Risk |
|---|---|---|
| 1 | Create `handler/` directory, move `handler.rs` → `handler/mod.rs`. CI must pass (zero changes). | None |
| 2 | Extract `helpers.rs`. Move pure functions, fix call sites. Each function is `pub(super)`. | Compile-only |
| 3 | Extract `subscription.rs`, `workspace_ops.rs`, `flow_ops.rs`, `registry_ops.rs`, `terminal_ops.rs`, `document_ops.rs`, `session_ops.rs` (all stateless or near-stateless handlers). | Low — these are simple CRUD arms |
| 4 | Extract `run_ops.rs` (resume/cancel/pause/post_input/swap_memory/resolve_review). | Medium — shares helpers with run_start |
| 5 | Extract `review_ops.rs` (the four flow review arms). | Medium — shares RunContext lookup logic |
| 6 | Extract `run_start.rs` (the big one). Apply `LlmParams` + `spawn_supervision_task` extractions. | Medium — largest but now isolated |
| 7 | Thin `mod.rs` to the routing table + struct/constructors only. | Low |

Each phase is independently reviewable via a focused PR diff.

---

## Before / After at a Glance

```
Before                     After
────────────────────────   ────────────────────────────────────────
handler.rs   ~1,550 lines  handler/mod.rs         ~160 lines  (struct + router)
                           handler/helpers.rs       ~80 lines  (pure utils)
                           handler/subscription.rs  ~55 lines
                           handler/run_start.rs    ~480 lines  (isolated)
                           handler/run_ops.rs      ~210 lines
                           handler/workspace_ops.rs ~45 lines
                           handler/flow_ops.rs     ~185 lines
                           handler/registry_ops.rs ~185 lines
                           handler/terminal_ops.rs  ~95 lines
                           handler/document_ops.rs ~115 lines
                           handler/review_ops.rs   ~210 lines
                           handler/session_ops.rs   ~95 lines
                           ─────────────────────────────────────
                           Total                 ~1,915 lines
                           (overhead is module doc-comments)
```

> `run_start.rs` remains the largest file at ~480 lines. A future follow-up can extract the supervision task helpers into a dedicated `supervision.rs` once the immediate split lands and proves stable.

---

## What This Enables

- **Fearless parallel editing** — two engineers can modify `review_ops.rs` and `registry_ops.rs` simultaneously with zero merge conflicts.
- **Focused code review** — a PR touching only flow-review behaviour has a 200-line diff, not a 1,500-line one.
- **Test co-location** — each sub-module can carry its own `#[cfg(test)]` block testing only its concern. The existing integration tests in `mod.rs` stay untouched.
- **Incremental extraction** — because every file is an `impl RuntimeHandler` block, the struct's visibility, field access, and `self.run_op` helper are all naturally shared without any new indirection.
