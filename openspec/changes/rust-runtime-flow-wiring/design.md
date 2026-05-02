## Context

The `rust-runtime-cpp-cutover` change successfully moved the agent execution engine from JavaScript (CEF renderer) to a Rust process. The protocol layer (`RuntimeAuthority`, `RuntimeProxy`, `GIPS`) is fully wired. However, two subsystems that were implemented independently were never connected:

1. `HostCapabilityDispatcher` — builds the tool list for a `ReactLoop` (shell, filesystem, notify, test_runner). It was implemented during `rust-runtime-migration` but the `StartRun` handler in `handler.rs` still passes `Arc::new(EmptyDispatcher)`.
2. `FlowRuntime` — manages multi-agent scheduling, typed-port routing, `@mention` routing, approval gates, and cycle limits. It was implemented during `agent-document-orchestration` but `StartRun` spawns a bare `ReactLoop` without involving `FlowRuntime` at all.

Additionally, three UI/bridge gaps block the user from triggering or reviewing runs:

- No `flow.run.start` channel in `bridge_channels.ts` — the C++ side handles it, but TypeScript has no typed definition.
- `review.approve` in `bridge_handler.cc` still calls the legacy in-process `ReviewStore`; it never reaches `RuntimeAuthority::resolve_review`.
- No `flow.save` handler — the visual FlowEditor is read-only.

## Goals / Non-Goals

**Goals:**

- Enable a `software-dev-cycle` flow to execute fully end-to-end: start → agent runs with tools → document submitted → next agent scheduled → approval gate → resume → run complete
- Wire `HostCapabilityDispatcher` so the LLM has access to `submit_document`, `shell`, `filesystem`, `test_runner`, and `notify`
- Connect `FlowRuntime` to the `StartRun` path when `flow_id` is present
- Add `flow.run.start` channel and Start Run button so users can trigger a run from the UI
- Route `review.approve` through `ResolveReview` to unblock approval gates
- Add `flow.save` so the visual editor can persist changes

**Non-Goals:**

- Rewriting `FlowRuntime` or `ReactLoop` internals
- Adding new agent node types or tool categories
- Migrating the legacy in-process agent path off `ReviewStore` entirely (fallback retained)
- Inbox "Approve" inline action (tracked separately in `agent-orchestration-ui`)

## Decisions

### D1: `FlowRuntime` as an owned field of `RuntimeHandler`, not `RuntimeAuthority`

**Decision:** `RuntimeHandler` will own an `Arc<tokio::sync::Mutex<FlowRuntime>>` instance per space, created lazily on first `StartRun` with a `flow_id`.

**Rationale:** `RuntimeAuthority` is the protocol-level state machine (run IDs, event fanout, cancel tokens). `FlowRuntime` is the scheduling oracle. Keeping them at the same layer in `RuntimeHandler` avoids tight coupling and allows `FlowRuntime` to be mocked in tests.

**Alternative considered:** Embedding `FlowRuntime` inside `RuntimeAuthority`. Rejected because `RuntimeAuthority` is already complex and `FlowRuntime` needs access to filesystem state that belongs at the handler level.

---

### D2: `submit_document` communicates back to `FlowRuntime` via a `tokio::sync::mpsc` channel, not a direct method call

**Decision:** The `submit_document` capability adapter will send a `DocumentSubmitted { run_id, doc_type, document_id }` message on an `mpsc::Sender` that the `RuntimeHandler` created and passed into the dispatcher. The `RuntimeHandler` run-supervision task receives these messages and forwards them to `FlowRuntime`.

**Rationale:** `ReactLoop` runs in a separate `tokio::task`. Direct synchronous callbacks from inside the loop back to `FlowRuntime` would require holding a mutex across an `await` point (unsound) or adding complex lifetime constraints. The channel pattern is idiomatic async Rust and decouples the loop from the scheduler cleanly.

**Alternative considered:** Passing `Arc<Mutex<FlowRuntime>>` directly into the dispatcher and calling it inline. Rejected due to deadlock risk — `FlowRuntime` may need to spawn the next agent loop while the current one is still holding the mutex.

---

### D3: Review approval in C++ will try Rust-runtime path first, fall back to legacy

**Decision:** In `bridge_handler.cc`, `review.approve` will call `RuntimeProxy::SendControl(ResolveReview{...})`. If `RuntimeProxy` returns a "no such run" error, the handler falls back to the existing `ReviewStore::approve()`.

**Rationale:** There are existing completed runs in the legacy path; a hard cutover would break them. The fallback ensures backward compatibility while new runs go through the Rust path.

**Alternative considered:** Single-path cutover with a feature flag. Rejected as premature — the legacy path still needs to serve runs started before this change.

---

### D4: `flow.save` writes atomically via a temp file + rename

**Decision:** The `flow.save` C++ handler will write the YAML to `<space_dir>/flow.yaml.tmp` then `rename()` it to `flow.yaml`. No locking is needed because all bridge callbacks are serialised on the browser process IPC thread.

**Rationale:** Avoids partial-write corruption if the process is killed during a save. The rename is atomic on all supported POSIX targets.

## Risks / Trade-offs

- **FlowRuntime per-space lazy init**: If `StartRun` with `flow_id` arrives for a space that has no `FlowRuntime` yet, we create one. If a second `StartRun` arrives before the first completes, we need to decide: queue, reject, or allow parallel runs. **Mitigation:** Reject with `run_already_active` error for now; parallel runs are a future feature.
- **mpsc channel backpressure**: If a run submits documents faster than the supervision task drains them, the channel buffer could fill. **Mitigation:** Use a bounded channel with capacity 64; `submit_document` returns an error if the channel is full, which the LLM can retry.
- **Legacy ReviewStore divergence**: Approval decisions that go to Rust runtime are not reflected in `ReviewStore`, so any UI that reads `ReviewStore` for run status will be stale. **Mitigation:** Document as known limitation; UI run status should read from runtime events, not ReviewStore.

## Migration Plan

1. Add `submit_document.rs` capability + register in `dispatcher.rs`
2. Wire `HostCapabilityDispatcher` in `handler.rs` (replaces `EmptyDispatcher`)
3. Add `DocumentSubmitted` channel + supervision loop in `RuntimeHandler`
4. Wire `FlowRuntime::start_run()` from `StartRun` when `flow_id` present
5. Add `flow.run.start` Zod channel definition
6. Add `flow.save` C++ handler
7. Update `review.approve` / `review.request_changes` in `bridge_handler.cc`
8. Add Start Run button in `FlowEditor` run-mode panel

Each step is independently deployable. Steps 1–2 can ship without steps 3–4 (standalone agent runs get tools; flow runs still bypass `FlowRuntime`).

## Open Questions

- Should `FlowRuntime` support concurrent runs per space (different `flow_id`s), or is one active run per space the limit for this change?
- Does the `submit_document` tool need to support doc-type validation against the space's registered doc types at write time, or is that a follow-up?
- Should `flow.save` also call `RuntimeProxy` to notify the runtime of graph changes, or is a file-level save sufficient?
