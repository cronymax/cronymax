## Context

`rust-runtime-migration` already moved the authoritative orchestration model into Rust: `crony` owns the host boundary and GIPS transport, `crates/cronymax` owns runtime authority and persistence, and `crates/cronygraph` owns graph semantics. The remaining gap is on the C++ and renderer side, where `AgentRuntime`, `FlowRuntime`, renderer-side ReAct execution, host-owned semantic event storage, and JSONL trace writing still exist as a second implementation of the same behavior.

This cutover has three constraints:

1. The C++ host must remain the owner of UI resources, browser views, terminal sessions, and platform capability adapters such as the permission broker.
2. The Rust runtime must become the only owner of semantic state for runs, reviews, inbox items, memory, and execution history.
3. The migration cannot ship a mixed-authority mode. The existing `rust-runtime-migration` decision was explicit: no compatibility shim that leaves both runtimes active.

The change spans multiple modules: a new host-side runtime bridge, bridge-handler rewiring, `SpaceManager` integration, a one-time persistence import, build-system work to package the runtime executable, and deletion of the legacy orchestration path in both C++ and renderer JavaScript.

## Goals / Non-Goals

**Goals:**

- Start and supervise a standalone `cronymax-runtime` process from the host application.
- Forward orchestration-related bridge traffic through a C++ `RuntimeProxy` over GIPS instead of invoking in-process execution code.
- Preserve the existing renderer-visible bridge method shapes so the UI can be rewired behind stable entry points.
- Rebind Space activation around runtime-managed identifiers and subscriptions while keeping browser and terminal ownership in the host.
- Import existing workspace-owned semantic state into the runtime once, then make runtime persistence the single source of truth.
- Delete the legacy host and renderer execution paths after forwarding is in place.

**Non-Goals:**

- Replacing CEF windowing, browser view management, or terminal ownership.
- Redesigning the Rust protocol surface or runtime semantics already delivered by `rust-runtime-migration`.
- Shipping a dual-write or dual-read transitional mode.
- Moving authored workspace artifacts such as YAML definitions, documents, or browser-tab metadata into the runtime store.

## Decisions

### Decision: Add a dedicated host-side `RuntimeProxy` and supervisor

The host will add `app/runtime_bridge/` with two responsibilities:

- `RuntimeSupervisor` locates the packaged `cronymax-runtime` binary, starts it during app startup, performs the Hello/Welcome handshake, monitors the child, and restarts it after unexpected exit.
- `RuntimeProxy` owns the active GIPS client connection, request serialization, reply deserialization, and runtime event subscription fanout for bridge handlers.

This separates lifecycle concerns from per-request forwarding and keeps bridge handlers thin.

Alternatives considered:

- Launch the runtime lazily from the first bridge request: rejected because it complicates failure behavior and would make the first user action pay the handshake cost.
- Embed the Rust runtime back into the host process: rejected because it recreates the coupled architecture that the Rust migration intentionally removed.

### Decision: Keep bridge method shapes stable and swap the implementation behind them

Existing renderer-facing bridge methods for `agent.*`, `flow.*`, `review.*`, `events.*`, `inbox.*`, `permission.*`, and `document.*` will continue to exist with the same top-level call shapes where practical. The host rewires their implementation to forward runtime protocol messages and translate runtime replies back into the existing JSON payload contract.

This minimizes renderer churn during the cutover and allows the host rewrite and renderer cleanup to proceed in smaller PRs while still honoring the no-dual-runtime rule.

Alternatives considered:

- Replace the renderer bridge surface with a brand-new API: rejected because it would couple transport migration to a UI API rewrite and enlarge the blast radius.

### Decision: Make the runtime the sole owner of semantic state; host keeps only UI metadata

The runtime persistence file becomes the only semantic source of truth for runs, reviews, inbox state, memory, and execution history. Host-managed SQLite remains for Space metadata, browser tabs, and terminal blocks. Semantic tables and JSONL trace files are removed.

The host will expose read and subscription projections of runtime state to the renderer rather than persisting a second semantic copy locally.

Alternatives considered:

- Keep host-side mirrored semantic tables for query speed: rejected because dual persistence would recreate consistency bugs and require reconciliation rules.

### Decision: Bind Space activation to runtime context rather than host-owned runtimes

`SpaceManager` will stop owning `AgentRuntime` and `FlowRuntime` instances per Space. Instead it will maintain:

- stable Space metadata and workspace roots
- active browser and terminal resources
- runtime-facing bindings such as active Space id, subscription handles, and run selection

Tool-scope enforcement remains a host responsibility, but the host will apply it as a capability adapter for runtime-originated requests using the owning Space's `workspace_root`.

Alternatives considered:

- Keep one runtime child per Space: rejected because it duplicates process lifecycle, complicates subscriptions, and is unnecessary once the Rust runtime can isolate runs by identifiers.

### Decision: Perform one-shot legacy state import before serving runtime-backed traffic

On first launch after cutover, the host scans existing workspace-owned run state files and imports them into runtime persistence before bridge handlers begin serving runtime-backed orchestration traffic. The host records that the import completed so later launches do not re-import unchanged state.

This preserves in-progress user data without maintaining long-term compatibility code in the steady state.

Alternatives considered:

- Require users to discard legacy state: rejected because it would drop active work.
- Re-read legacy files on every launch: rejected because it would create ambiguous precedence between old files and runtime persistence.

### Decision: Delete the renderer ReAct loop and in-process C++ orchestration after bridge forwarding lands

The renderer becomes a projection layer that renders runtime events and sends user intents through the bridge. `app/agent/`, `app/flow/flow_runtime`, `app/flow/trace_writer`, `app/document/reviewer_pipeline`, `app/document/review_store`, and `web/src/agent_runtime/` are removed as part of the cutover sequence.

Alternatives considered:

- Leave dead-code compatibility layers in place temporarily: rejected because they would continue to shape architecture around a removed runtime model and make regressions harder to spot.

### Decision: Expose gips to the host via a thin C ABI from the `crony` crate

The Rust `gips` dependency has no C++ binding, and the host needs the same envelope, transport, and lifecycle semantics that `crony/src/boundary.rs` already implements over `gips`. The host will link against a thin C ABI surface exported from the `crony` crate (compiled as a `cdylib`/`staticlib` alongside the `cronymax-runtime` binary) rather than reimplementing the gips wire format and platform primitives in C++.

The ABI surface is intentionally narrow:

- opaque `crony_client_t` handle plus `crony_client_new`, `crony_client_close`
- blocking `crony_client_send(handle, bytes, len)` returning a status code
- blocking `crony_client_recv(handle, out_buf, out_len, timeout_ms)` for replies and events
- thread-safe; the C ABI layer wraps the same `GipsTransport` machinery `crony` already uses internally
- payload is the existing JSON-encoded `ClientToRuntime` / `RuntimeToClient` envelopes; no second wire format

`RuntimeProxy` then owns: handle lifetime, JSON serialization with `nlohmann::json`, request/response correlation, and a single recv pump thread that dispatches replies and runtime events to subscribers.

Alternatives considered:

- Reimplement gips natively in C++: rejected because gips already encapsulates Mach ports, SOCK_SEQPACKET, and named-pipe details and a parallel implementation would diverge.
- Use a generic IPC such as a local TCP or stdin/stdout JSON-RPC channel: rejected because it would walk back the explicit transport choice the Rust migration made and lose the security properties of the platform-specific primitives.
- Generate the C++ binding with `cbindgen` from the runtime crate: viable, but the surface is small enough that a hand-written FFI module in `crony` is simpler to audit and keeps `cbindgen` out of the build graph.

## Risks / Trade-offs

- [Runtime startup or handshake failure blocks orchestration] → Surface explicit runtime-unavailable errors, add startup logging around binary discovery and handshake, and keep the failure mode closed rather than falling back.
- [Child-process restart may drop in-flight subscriptions or requests] → Centralize subscription bookkeeping in `RuntimeProxy`, re-establish subscriptions after restart, and fail active requests so callers can retry cleanly.
- [Bridge payload drift between legacy JSON shapes and runtime protocol messages] → Reuse the existing bridge entry points, add focused bridge integration tests per handler family, and validate JSON translation against runtime e2e fixtures.
- [One-shot import can duplicate or overwrite state] → Make import idempotent, persist an import-complete marker, and define runtime persistence as authoritative once import succeeds.
- [Packaging the runtime binary into the app bundle can fail on one platform path] → Add explicit CMake packaging steps and a startup diagnostic that prints the resolved runtime binary path.
- [Deleting host semantic tables may break UI features that were implicitly querying SQLite] → Move those queries to runtime-backed read APIs before removing the tables and cover the affected panels with end-to-end checks.
- [C ABI surface drifts from the Rust gips client used by the runtime] → Keep the ABI minimal (open / send / recv / close), exercise it from a Rust unit test that links the same library the host links, and bump a `CRONY_ABI_VERSION` constant on every signature change.

## Migration Plan

1. Package `cronymax-runtime` and the `crony` C ABI library together with the application bundle, and add startup supervision plus handshake diagnostics.
2. Introduce `RuntimeProxy` (linking the C ABI) with request forwarding, event subscription, and capability-adapter plumbing.
3. Rewire bridge handlers by family, starting with a narrow end-to-end slice that can prove request/reply plus subscription behavior.
4. Update `SpaceManager` to replace per-Space runtime ownership with runtime bindings and subscription management.
5. Implement the one-shot legacy state import before enabling runtime-backed handlers in production builds.
6. Delete renderer ReAct code and legacy C++ orchestration modules once forwarded handlers cover the complete orchestration path.
7. Remove semantic SQLite tables, JSONL trace writing, and obsolete tests after runtime-backed UI projections are green.

Rollback strategy: if the cutover build regresses before release, revert the change set and return to the pre-cutover build. There is no supported runtime toggle within a single shipped build.

## Open Questions

- Where should the host persist the import-complete marker: in the runtime persistence file, in host SQLite metadata, or in a sidecar marker file?
- Should runtime child restart automatically resume renderer subscriptions for all Spaces or only for the currently active Space?
- Does the final packaged runtime binary and the `crony` C ABI library live under the app bundle resources directory or next to the helper executables, and what is the canonical lookup order across dev and packaged builds?
- Which existing renderer panels still read semantic SQLite rows directly and therefore need an explicit runtime query path before those tables are removed?
- Should the C ABI library be a `staticlib` linked into the host binary or a `cdylib` loaded at runtime — and does that choice differ between dev and packaged builds?
