## 1. Runtime Scaffold

- [x] 1.1 Create the Rust workspace layout with `crony/`, `crates/cronymax`, and `crates/cronygraph` and wire it into the top-level build.
- [x] 1.2 Define crate responsibilities and dependency direction so `crony/` depends on `crates/cronymax` and `crates/cronymax` depends on `crates/cronygraph`.
- [x] 1.3 Implement the `crony/` integration layer that owns runtime startup, shutdown, health checks, and standalone process lifecycle management.
- [x] 1.4 Define the runtime configuration contract for workspace roots, app-private storage paths, logging, and protocol version negotiation.

## 2. GIPS Protocol

- [x] 2.1 Define the initial GIPS message schemas for `control`, `events`, and `capabilities`, including correlation ids and version fields.
- [x] 2.2 Implement the `crony/` boundary code needed to connect the C++ app to the Rust runtime over GIPS.
- [x] 2.3 Implement `crates/cronymax` transport setup and dispatch loops for control requests, event subscriptions, and capability calls.
- [x] 2.4 Add protocol compatibility checks so startup fails cleanly on version mismatch.

## 3. Orchestration Core

- [x] 3.1 Implement `crates/cronygraph` graph and multi-agent orchestration primitives with no CEF-specific or business-specific glue.
- [x] 3.2 Move reusable execution and routing abstractions into `crates/cronygraph` while keeping product policies outside the crate.
- [x] 3.3 Add focused tests for `crates/cronygraph` covering graph traversal, orchestration flow, and terminal conditions independent of host integration.

## 4. Runtime Authority Core

- [x] 4.1 Implement `crates/cronymax` runtime-owned data models for Spaces, runs, agents, memory namespaces, and permission state.
- [x] 4.2 Implement `crates/cronymax` run lifecycle operations for start, cancel, pause, resume, and review-related state transitions.
- [x] 4.3 Implement `crates/cronymax` event emission for run changes, trace events, token streaming, and permission requests.
- [x] 4.4 Add runtime rehydration so paused or awaiting-approval work survives runtime restart.

## 5. LLM and Agent Loop Migration

- [x] 5.1 Port the renderer-owned ReAct loop into `crates/cronymax`, including message history updates, tool-call routing, and terminal conditions.
- [x] 5.2 Move OpenAI-compatible LLM request execution, streaming assembly, and tool-call parsing into `crates/cronymax`.
- [x] 5.3 Route existing UI agent entrypoints through `crony/` to `crates/cronymax` without leaving a parallel renderer-owned execution path.
- [x] 5.4 Add focused tests for runtime-owned LLM streaming, tool-call loops, and human-approval pauses.

## 6. Host Capability Adapters

- [x] 6.1 Implement host capability adapters for sandboxed shell or PTY execution and return structured results over GIPS.
- [x] 6.2 Implement host capability adapters for browser or page inspection and wire them to the active Space context.
- [x] 6.3 Implement host capability adapters for filesystem mediation, workspace scope enforcement, and secret or keychain access.
- [x] 6.4 Implement host capability adapters for notifications, dock/status integration, and user approval prompts.

## 7. Persistence and State Ownership

- [x] 7.1 Move operational runtime persistence for runs, events, memory indexes, and permission grants into `crates/cronymax`.
- [x] 7.2 Reduce host-owned persistence to shell or UI metadata such as Space metadata, tabs, and window-facing state.
- [x] 7.3 Migrate event replay and runtime-state restoration so UI surfaces consume runtime authority instead of host-generated semantic state.
- [x] 7.4 Add migration or compatibility handling for previously persisted in-process runtime state.

## 8. UI and Bridge Rewiring

- [ ] 8.1 Update bridge handlers to proxy runtime control requests and event subscriptions through `crony/` instead of invoking in-process orchestration.
- [ ] 8.2 Rewire agent, flow, inbox, and review UI surfaces to render runtime-emitted events and state projections.
- [ ] 8.3 Update permission and review actions so the UI returns user decisions to `crates/cronymax` through the host boundary.
- [ ] 8.4 Add compatibility shims only where needed for rollout and mark every shim with an explicit removal path.

## 9. Space Manager Integration

- [ ] 9.1 Remove in-process `AgentRuntime` ownership from `Space` and replace it with runtime-backed bindings for the active Space.
- [ ] 9.2 Update Space switching and activation flows to bind UI resources to runtime-managed runs and agents.
- [ ] 9.3 Preserve existing browser and terminal resource switching while moving semantic agent state ownership out of `SpaceManager`.

## 10. Legacy Path Removal

- [ ] 10.1 Remove renderer-owned agent loop execution once the Rust runtime path is feature-complete.
- [ ] 10.2 Remove host-owned semantic trace generation and in-process orchestration paths that bypass runtime authority.
- [ ] 10.3 Delete or deprecate legacy bridge and runtime code paths that would allow mixed execution authorities.

## 11. Dependent Design Cleanup

- [x] 11.1 Update dependent OpenSpec changes and docs that assume renderer-owned execution or a peer Node skill sidecar authority.
- [x] 11.2 Document that future skill or plugin runtimes must be subordinate to Rust runtime authority.
- [x] 11.3 Update architecture and migration docs to reflect the crate split, GIPS protocol, and host-capability boundary.

## 12. Validation

- [x] 12.1 Add end-to-end tests covering run startup, tool execution, event streaming, permission prompts, and restart rehydration through the Rust runtime.
- [x] 12.2 Add failure-mode tests for runtime crash, protocol mismatch, capability timeout, and host reconnect behavior.
- [x] 12.3 Run `openspec validate rust-runtime-migration --strict` and fix any artifact issues before implementation begins.
