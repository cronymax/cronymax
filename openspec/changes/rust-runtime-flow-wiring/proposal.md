## Why

The `software-dev-cycle` preset flow and all multi-agent flow orchestration are entirely non-functional: when `agent.run` or `flow.run.start` is called, the Rust runtime spawns a plain `ReactLoop` with no tools (`EmptyDispatcher`) and no connection to `FlowRuntime`. Agents cannot submit documents, approval gates never fire, and `on_approved_reschedule` logic is never reached. The infrastructure exists but was never wired together after the `rust-runtime-cpp-cutover` change landed.

## What Changes

- Wire `HostCapabilityDispatcher` (with `submit_document`, `shell`, `filesystem`, `test_runner`, `notify` tools) into `ReactLoop` inside `RuntimeHandler::handle_control` — replacing `EmptyDispatcher`
- Add `submit_document` capability to `crates/cronymax/src/capability/` so the LLM can produce documents that route through `FlowRuntime`
- Connect `FlowRuntime` to `StartRun`: when a `flow_id` is present in the payload, delegate agent scheduling and document routing to `FlowRuntime` instead of a bare `ReactLoop`
- Add `flow.run.start` to `web/src/bridge_channels.ts` with typed Zod schema
- Add a "Start Run" trigger to the FlowEditor run-mode panel in the UI
- Wire `review.approve` / `review.request_changes` bridge calls to `RuntimeAuthority::resolve_review` in Rust (completing `rust-runtime-migration` tasks 8.1–8.3 for review channels)
- Add `flow.save` C++ bridge handler so the visual editor can write `flow.yaml` changes (unblocks `agent-orchestration-ui` tasks 6.3, 6.8)

## Capabilities

### New Capabilities

- `submit-document-tool`: The `submit_document` Rust capability adapter — receives a doc-type + Markdown body from the LLM, writes it to the workspace, updates `FlowRuntime` port state, and triggers downstream routing
- `flow-runtime-integration`: How `FlowRuntime` is driven from inside the Rust runtime process — `StartRun` → `FlowRuntime::start_run`, agent scheduling, invocation context injection, `on_approved_reschedule` callbacks, cycle enforcement
- `flow-run-ui`: UI surface for starting, monitoring, and approving flow runs — `flow.run.start` channel, run-mode Start button in FlowEditor, review-approve wiring to `ResolveReview`

### Modified Capabilities

- `multi-agent-orchestration`: Run lifecycle now owned by Rust `RuntimeAuthority` + `FlowRuntime`; approval decisions route through `ResolveReview` control request rather than legacy in-process `ReviewStore`

## Impact

- `crates/cronymax/src/capability/` — new `submit_document.rs`; updated `dispatcher.rs` to register it; updated `mod.rs`
- `crates/cronymax/src/runtime/handler.rs` — `StartRun` branch replaces `EmptyDispatcher` with `HostCapabilityDispatcher`; wires `FlowRuntime` when `flow_id` present
- `crates/cronymax/src/flow/runtime.rs` — public `FlowRuntime` entry points for use from `RuntimeHandler`
- `app/browser/bridge_handler.cc` — `review.approve` / `review.request_changes` forward to `RuntimeProxy::SendControl(ResolveReview{...})` before falling back to legacy `ReviewStore`; add `flow.save` handler
- `web/src/bridge_channels.ts` — add `flow.run.start`, `flow.save` channel definitions
- `web/src/components/FlowEditor/` — Start Run button in run-mode panel
