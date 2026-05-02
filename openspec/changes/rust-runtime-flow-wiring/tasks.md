## Tasks

### 1. submit_document capability (Rust)

- [x] 1.1 Create `crates/cronymax/src/capability/submit_document.rs` — implement `SubmitDocumentCapability` that writes a `.md` file to `<space_dir>/documents/<id>.md` and returns `{ document_id }` as a JSON tool result
- [x] 1.2 Register `submit_document` in `HostCapabilityDispatcher::build()` in `crates/cronymax/src/capability/dispatcher.rs`
- [x] 1.3 Expose `submit_document` module in `crates/cronymax/src/capability/mod.rs`

### 2. Wire HostCapabilityDispatcher to ReactLoop (Rust)

- [x] 2.1 In `crates/cronymax/src/runtime/handler.rs`, replace `Arc::new(EmptyDispatcher)` with `HostCapabilityDispatcher::build(space_ctx)` before spawning `ReactLoop`
- [x] 2.2 Add a `tokio::sync::mpsc` bounded channel (capacity 64) for `DocumentSubmitted { run_id, doc_type, document_id }` messages; pass `Sender` into `HostCapabilityDispatcher` so `submit_document` can signal submissions
- [x] 2.3 Add a supervision task in `RuntimeHandler` that drains the `DocumentSubmitted` receiver and calls `FlowRuntime::on_document_submitted()` when a flow run is active

### 3. Connect FlowRuntime to StartRun (Rust)

- [x] 3.1 In `RuntimeHandler::handle_control`, when `StartRun { payload }` contains a `flow_id`, load `FlowRuntime` for the space (create lazily if absent) and call `FlowRuntime::start_run(run_id, flow_id, initial_input)`
- [x] 3.2 Have `FlowRuntime::start_run` schedule the entry agent by sending a `StartAgentInvocation` message to the supervision loop, which then spawns the `ReactLoop` with an injected `InvocationContext`
- [x] 3.3 Implement `FlowRuntime::on_document_submitted()` — update port state, call `Router::route()`, and schedule downstream agents whose input ports are now satisfied
- [x] 3.4 Implement `FlowRuntime::on_approved_reschedule(run_id, review_id)` — resume the pending downstream agent after an Approved `ResolveReview` is received
- [x] 3.5 Wire `ResolveReview` control requests in `RuntimeHandler::handle_control` to call `FlowRuntime::on_approved_reschedule()` or `FlowRuntime::on_rejected_requeue()`
- [x] 3.6 Enforce cycle limit: halt run and emit `run_failed` event when `FlowRuntime` cycle counter exceeds `max_cycles`

### 4. flow.run.start bridge channel (TypeScript)

- [x] 4.1 Add `flow.run.start` channel definition to `web/src/bridge_channels.ts` — request `{ space_id: string; flow_id: string; initial_input?: string }`, response `{ run_id: string }`
- [x] 4.2 Add `flow.save` channel definition to `web/src/bridge_channels.ts` — request `{ space_id: string; graph: FlowGraph }`, response `{ ok: boolean; error?: string }`

### 5. Start Run button in FlowEditor (TypeScript/React)

- [x] 5.1 Add a run-mode panel component to `web/src/components/FlowEditor/` that shows a "Start Run" button when no active run exists for the current flow
- [x] 5.2 Wire the button to call `flow.run.start` via the bridge channel and transition panel to the running state on success
- [x] 5.3 Show active `run_id` and a Cancel button once a run is started; wire Cancel to `flow.run.cancel`

### 6. flow.save C++ bridge handler

- [x] 6.1 Add `flow.save` handler in `app/browser/bridge_handler.cc` that deserialises the `FlowGraph` payload and writes it atomically to `<space_dir>/flow.yaml` using a temp-file + rename strategy
- [x] 6.2 Return `{ ok: true }` on success or `{ ok: false, error: "<reason>" }` on serialisation failure without overwriting the existing file

### 7. Wire review.approve / review.request_changes to Rust runtime (C++)

- [x] 7.1 In `app/browser/bridge_handler.cc`, update the `review.approve` handler to first call `RuntimeProxy::SendControl(ResolveReview { run_id, review_id, decision: Approved, notes })`
- [x] 7.2 If `RuntimeProxy` returns a "no such run" / "run not in Rust runtime" response, fall through to the existing legacy `ReviewStore::approve()` path
- [x] 7.3 Apply the same dual-path logic to `review.request_changes` → `ResolveReview { decision: RequestChanges }`
