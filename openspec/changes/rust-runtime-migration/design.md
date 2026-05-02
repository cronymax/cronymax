## Context

The current runtime is split across three execution domains: renderer JavaScript owns the active ReAct loop, native C++ owns bridge dispatch and a subset of orchestration state, and workspace/app storage is shared between UI-facing code paths and runtime-facing code paths. That split makes it difficult to identify a single source of truth for run state, agent state, tool routing, permission state, and trace emission.

This migration intentionally changes the authority model rather than just porting code. A standalone Rust process becomes the only runtime authority for runs and agents. The CEF host remains responsible for UI, native resources, and privileged OS integrations, but it no longer owns orchestration semantics. The migration also needs to account for current OpenSpec assumptions that still treat the renderer as the agent-loop owner and that stage future sidecar work around a Node-hosted skill runtime.

Constraints:

- Workspace-authored artifacts such as `flow.yaml`, `agent.yaml`, documents, and review sidecars remain the product-facing contract and MUST stay readable and git-trackable.
- The desktop app still needs privileged access to PTYs, browser state, notifications, filesystem mediation, and secrets on behalf of the runtime.
- Migration cannot require a flag-day rewrite of every panel at once; host/UI compatibility shims are needed while runtime ownership moves.
- The user selected GIPS as the IPC substrate and expects full host migration rather than only moving the ReAct loop.

## Goals / Non-Goals

**Goals:**

- Establish the standalone Rust runtime as the sole authority for run lifecycle, agent lifecycle, orchestration, memory, permission state, and runtime event emission.
- Define a GIPS protocol that covers runtime control, live event streaming, and host capability invocation with clear ownership and failure semantics.
- Split the Rust implementation into crates with explicit responsibilities so CEF integration, runtime implementation, and business-agnostic orchestration can evolve independently.
- Reduce the CEF/C++ host to a capability adapter and UI shell rather than a peer orchestration engine.
- Preserve authored workspace artifacts while moving operational runtime state and behavior behind the Rust boundary.
- Provide a migration path that removes renderer-owned agent execution and in-process `AgentRuntime` semantics without breaking the product surface during rollout.
- Prevent future sidecar work from reintroducing competing orchestration authorities.

**Non-Goals:**

- Redesigning Flow YAML, document schemas, or the review lifecycle introduced by earlier changes.
- Defining a final plugin or skill sandbox model beyond the requirement that future skill execution cannot become a peer authority to the Rust runtime.
- Removing CEF, replacing the desktop UI stack, or changing the current workspace artifact layout.
- Solving Linux/Windows packaging details in this change beyond protocol and authority boundaries.

## Decisions

### Decision 1: One Rust runtime process is the authority for all Spaces in an app session

The app launches one standalone Rust runtime process per desktop app session. That runtime owns all run and agent state across all Spaces, including run creation, cancellation, pause/resume semantics, agent loop execution, tool routing, memory namespace state, permission grants, and runtime event production.

Authored artifacts remain split by responsibility:

- Workspace-facing truth: `flow.yaml`, `agent.yaml`, documents, `.history`, `reviews.json`
- Runtime-facing truth: run state, live agent state, runtime event log, memory indexes, permission grants, host capability leases, provider session state

Alternatives considered:

- One runtime per Space: simpler isolation, but more process churn, harder cross-Space coordination, and duplicated provider/session resources.
- One runtime per run: strongest isolation, but operationally expensive and poor fit for long-lived inbox/event projections.
- Keep runtime truth partly in C++: preserves today's split-brain problem and weakens portability.

Rationale: one runtime per app session gives a single authority without requiring a daemon model. It also keeps the host's responsibility narrow: spawn, supervise, and proxy.

### Decision 2: GIPS is the only runtime protocol boundary

All runtime communication crosses a GIPS boundary. The protocol is divided into three logical surfaces:

- `control`: start/cancel/resume runs, post user input, query state, mutate review decisions
- `events`: replay and live streaming of runtime events, token streaming, run status changes, permission requests
- `capabilities`: runtime requests host-executed privileged operations and receives correlated results

Every request has a stable correlation id. Event messages are append-only facts emitted by the Rust runtime. Capability calls are explicit requests from Rust to the host, never implicit side effects of host-owned orchestration.

Alternatives considered:

- Continue using bridge channels as the runtime contract: tightly coupled to CEF and unsuitable for a portable runtime.
- JSON-RPC over stdio: workable, but weaker transport semantics than GIPS and redundant given the user's chosen in-house IPC stack.
- gRPC or WebSocket transport: overbuilt for a local desktop boundary and less aligned with host credential checks.

Rationale: GIPS gives message boundaries, platform-native transport, credential inspection, and a clean path to shared-memory transfer for larger payloads later.

### Decision 3: Rust workspace is split into `crony/`, `crates/cronymax`, and `crates/cronygraph`

The Rust side is organized into three layers:

- `crony/`: CEF-facing wrapper crate that exposes the FFI or shared-library boundary needed by the native host and owns standalone runtime process lifecycle management, startup configuration, health checks, and protocol bootstrap.
- `crates/cronymax`: main runtime crate that owns process-local state, persistence, protocol handlers, provider integration, memory, permissions, tool routing, and host capability mediation.
- `crates/cronygraph`: business-agnostic orchestration crate that owns reusable graph and multi-agent orchestration primitives without desktop-specific policy or product-facing integration concerns.

Dependency direction is one-way:

- `crony/` depends on `crates/cronymax`
- `crates/cronymax` depends on `crates/cronygraph`
- `crates/cronygraph` depends on neither desktop host bindings nor product-specific runtime glue

Alternatives considered:

- Single runtime crate: simpler initially, but FFI, lifecycle management, orchestration primitives, and product-specific runtime concerns become entangled quickly.
- Put orchestration primitives inside `crony/`: couples business-less graph logic to host integration details.
- Put lifecycle management inside `crates/cronymax`: works mechanically, but muddies the intended distinction between host boundary glue and runtime core.

Rationale: this split matches the intended ownership boundaries. `crony/` is the integration shell, `crates/cronymax` is the runtime product core, and `crates/cronygraph` is the reusable orchestration engine.

### Decision 4: The host becomes a capability adapter, not a runtime peer

The C++ host keeps ownership of UI and privileged local resources, but all agent-facing tool semantics are defined in Rust. The host exports capability providers such as:

- PTY/shell execution
- browser/page inspection
- notifications and dock/status integration
- filesystem mediation under workspace/root policies
- secret/keychain access

The runtime invokes those capabilities over GIPS. The host does not decide run transitions, tool policy, or trace semantics. Permission prompts originate from the Rust runtime and are rendered by the host/UI as a response surface.

Alternatives considered:

- Let C++ keep built-in tool semantics while Rust owns orchestration: still creates two semantic authorities.
- Move all privileged integrations into Rust immediately: possible long-term, but not required to achieve portable runtime authority.

Rationale: this cut keeps platform-specific code local while making the runtime portable and headless-capable.

### Decision 5: Rust owns operational persistence; host stores only shell/UI metadata

Operational runtime state moves behind the Rust boundary. The Rust runtime owns persistence for run state, event history, memory indexes, permission state, and other operational metadata. The host may continue storing shell-facing metadata such as tab layout or window/UI state, but it no longer persists semantic run or agent truth.

Workspace-authored documents remain on disk in the workspace as before. The runtime reads and writes them as part of the authored contract, but host-side stores such as `SpaceStore` or host event tables are no longer the canonical source for runtime state.

Alternatives considered:

- Keep SQLite ownership in C++ and proxy semantic operations into it: preserves tight host/runtime coupling and complicates testing.
- Store everything in workspace files: makes operational state noisy, slower to query, and harder to treat as app-private.

Rationale: portable runtime authority requires portable persistence authority for operational data.

### Decision 6: Renderer execution paths become compatibility shims and are removed in stages

The migration proceeds by replacing runtime ownership in layers:

1. Add the Rust runtime process and GIPS supervision.
2. Move LLM calls, tool-call parsing, and the ReAct loop out of renderer JavaScript.
3. Move run lifecycle, event production, and review/control mutations behind Rust.
4. Replace in-process native `AgentRuntime` and direct bridge-owned orchestration with host capability adapters.
5. Retire legacy bridge surfaces once the UI consumes runtime facts rather than generating them.

During the migration, bridge handlers may proxy to the runtime for compatibility, but they MUST NOT remain an alternate orchestration path.

Alternatives considered:

- Rewrite everything behind one feature flag and switch at once: too risky for a cross-cutting product subsystem.
- Leave renderer execution alive indefinitely as a fallback path: undermines the authority model and doubles validation surface.

Rationale: staged migration is safer, but it still converges on a single authority.

### Decision 7: Future skill execution is subordinate to Rust runtime authority

This change supersedes assumptions that a future Node sidecar is a peer runtime authority for skills or tool execution. Any future skill runtime, plugin host, or sandbox layer MUST be subordinate to Rust runtime authority. Rust defines the tool namespace, permissions model, and invocation semantics even if some tool implementations execute in another sandbox.

Alternatives considered:

- Keep a peer Node sidecar for skill authority: recreates competing control planes.
- Freeze all future skill work until the migration is complete: unnecessary, as long as the authority boundary is explicit.

Rationale: the migration only pays off if future extensibility does not reintroduce split ownership.

## Risks / Trade-offs

- [Runtime crash or deadlock cuts off all Spaces at once] → Mitigation: host supervision, explicit restart semantics, and state rehydration owned by Rust.
- [IPC latency degrades token streaming or tool round-trips] → Mitigation: separate control, event, and capability channels; reserve GIPS shared memory for large payloads.
- [Host and runtime drift on capability contracts] → Mitigation: version the protocol, validate message schemas on both ends, and keep the host surface narrow.
- [Migration leaves shadow orchestration paths in bridge handlers] → Mitigation: treat any host-side orchestration logic as temporary compatibility code with explicit removal tasks.
- [Current Stage 3 skill design assumes a peer Node sidecar] → Mitigation: update dependent OpenSpec artifacts to make any future skill runtime subordinate to Rust authority.
- [Operational persistence split across Rust and host causes reconciliation bugs] → Mitigation: make Rust canonical for semantic state; host stores only shell/UI metadata.

## Migration Plan

1. Create the Rust workspace structure with `crony/`, `crates/cronymax`, and `crates/cronygraph`, and wire it into host packaging and local development flows.
2. Implement runtime supervision and protocol bootstrap in `crony/`, including startup, shutdown, health checks, and GIPS connection setup.
3. Define and implement the initial GIPS protocol for control, events, and capabilities, with schema/version negotiation, in `crates/cronymax` and its `crony/` integration layer.
4. Move orchestration primitives and graph execution logic into `crates/cronygraph`, keeping it free of CEF-specific or product-specific boundary glue.
5. Move LLM request execution, streaming assembly, tool-call parsing, and the ReAct loop into `crates/cronymax` while the host proxies existing UI entrypoints.
6. Move run lifecycle, trace/event emission, and runtime persistence into `crates/cronymax`; convert UI panels to subscribe to runtime-emitted events instead of renderer-generated traces.
7. Replace native in-process tool/orchestration paths with host capability adapters and Rust-defined tool semantics.
8. Remove or deprecate legacy bridge/runtime code paths, including renderer-owned agent loop execution and host-owned semantic trace generation.
9. Update dependent design assumptions for skills/runtime extension so no later work creates a competing authority.

Rollback strategy:

- During phased rollout, keep a gated compatibility path that proxies the old UI entrypoints into the Rust runtime rather than reviving independent orchestration.
- If a release must roll back, disable runtime startup and restore the prior in-process paths as a temporary product fallback, but do not attempt mixed-state execution between the old and new authorities for the same run.

## Open Questions

- Should the Rust runtime own the existing per-Space SQLite database directly, or should it use a new runtime-owned store and leave `SpaceStore` as shell/UI metadata only?
- What is the long-term hosting model for skills: Rust-native extensions, WASM guests, or subordinate subprocess sandboxes?
- Which runtime events need durable replay guarantees versus best-effort live delivery only?
- Do we want one global runtime process name/service per app session or a host-spawned private runtime instance with ephemeral addressing only?
- How much legacy bridge surface do we preserve for one release cycle versus cutting directly to runtime-backed UI entrypoints?
