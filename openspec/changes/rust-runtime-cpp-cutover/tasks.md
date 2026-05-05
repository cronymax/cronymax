## 1. Runtime Packaging And Supervision

- [x] 1.1 Add a `crony-cabi` C ABI surface in the `crony` crate (`crony_client_new`/`send`/`recv`/`close`, opaque handle, `CRONY_ABI_VERSION`) that wraps the existing `GipsTransport` and is built as a library the host can link.
- [x] 1.2 Add a `cronymax-runtime` binary target in `crony/bin/` that boots the existing runtime and uses the same gips transport configuration as the embedded path.
- [x] 1.3 Add CMake and bundle steps that build the `crony` C ABI library and the `cronymax-runtime` binary, copy them into the app's packaged runtime location for dev and app-bundle builds, and expose the C ABI header to the host.
- [x] 1.4 Create `app/runtime_bridge/` with runtime binary discovery, child-process launch, shutdown, and Hello/Welcome handshake plumbing on top of the C ABI.
- [x] 1.5 Implement supervised restart behavior and closed-failure reporting when the runtime is unavailable or the handshake fails.

## 2. Runtime Proxy And Event Transport

- [x] 2.1 Implement `RuntimeProxy` request and reply translation for the runtime control, event, review, inbox, permission, and document envelopes on top of the C ABI handle.
- [x] 2.2 Implement runtime event subscription fanout from the C ABI recv pump to existing bridge subscriber callbacks.
- [x] 2.3 Implement the host capability adapter boundary so runtime-originated approval and protected capability requests flow through the existing permission broker and return results to the runtime.

## 3. Bridge Handler Rewiring

- [x] 3.1 Rewire `agent.*` and `flow.*` bridge handlers to forward through `RuntimeProxy` instead of `AgentRuntime` or `FlowRuntime`.
- [x] 3.2 Rewire `review.*`, `events.*`, and `inbox.*` bridge handlers to use runtime-backed request and subscription paths.
- [x] 3.3 Rewire `permission.*` and `document.*` handlers to use runtime-backed state and remove legacy in-process orchestration calls.

## 4. SpaceManager Integration

- [x] 4.1 Remove `Space::agent_runtime` and `Space::flow_runtime` ownership and replace it with runtime binding and subscription state needed for the active Space.
- [x] 4.2 Update Space switching to reconnect browser and agent-facing panels to runtime-backed state while preserving browser tabs and terminal sessions.
- [x] 4.3 Update tool-scope enforcement so runtime-originated capability calls are validated against the owning Space `workspace_root`.

## 5. Persistence Cutover

- [x] 5.1 Implement one-shot import of legacy workspace run state files into the runtime persistence store before runtime-backed bridge traffic is served.
- [x] 5.2 Persist and honor an import-complete marker so legacy snapshots are not re-imported on every launch.
- [x] 5.3 Remove host semantic event and inbox persistence paths in favor of runtime-backed reads and subscriptions.

## 6. Legacy Path Removal

- [x] 6.1 Delete `app/agent/agent_runtime.*`, `app/flow/flow_runtime.*`, and `app/flow/trace_writer.*` after forwarded handlers cover their behavior.
- [x] 6.2 Delete `app/document/reviewer_pipeline.*` and `app/document/review_store.*` after review flow is fully runtime-backed.
- [x] 6.3 Delete `web/src/agent_runtime/` and rewire the affected renderer panels to act as pure projections of runtime events.

## 7. Validation And Cleanup

- [x] 7.1 Add or update end-to-end coverage that boots the runtime child, drives a renderer-shaped bridge workflow, and asserts request, event, and review flows.
- [x] 7.2 Remove obsolete C++ and JS tests that covered the deleted in-process runtime path and replace them with runtime-bridge focused coverage where needed.
- [x] 7.3 Run OpenSpec validation plus targeted build and test commands for the Rust runtime, host bridge, and renderer surfaces touched by the cutover.
