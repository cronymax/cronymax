## Why

The current agent system splits execution across the renderer, native C++, and bridge handlers, which makes runs, agent state, tool routing, and event ownership hard to reason about and difficult to reuse outside the desktop UI. This change consolidates runtime truth into a standalone Rust process so the agent runtime becomes portable, testable, and authoritative before more orchestration and skills work compounds the current split-brain design.

## What Changes

- **BREAKING** Move run lifecycle, agent lifecycle, orchestration, tool routing, memory, and runtime event emission out of the renderer/native in-process path into a standalone Rust runtime process.
- **BREAKING** Adopt a GIPS-based IPC boundary between the CEF host and the Rust runtime for control messages, event streams, and capability invocation.
- Implement the Rust side as a multi-crate workspace: `crony/` provides the CEF-facing wrapper/shared-library boundary and standalone process lifecycle management, `crates/cronymax` owns the runtime implementation, and `crates/cronygraph` owns business-agnostic multi-agent orchestration primitives.
- Introduce a host capability adapter boundary: the desktop host exposes privileged capabilities such as PTY execution, browser/page inspection, OS notifications, filesystem mediation, and keychain access to the Rust runtime instead of owning agent semantics directly.
- Make the Rust runtime the source of truth for run state, agent state, trace/event production, permission state, and memory namespaces across all Spaces.
- Preserve workspace-authored artifacts such as `flow.yaml`, `agent.yaml`, documents, and review sidecars as the durable project-facing contract while moving runtime metadata and behavior out of the UI process.
- Replace the current mixed tool-execution model with Rust-defined tools that may call host capabilities over IPC.
- Establish a migration path away from renderer-owned agent loop execution and away from future sidecar plans that assume skills or orchestration are centered outside the Rust runtime.

## Capabilities

### New Capabilities

- `rust-runtime-authority`: standalone Rust runtime that owns runs, agents, orchestration, memory, permissions, and runtime event emission.
- `runtime-host-protocol`: GIPS protocol and lifecycle for control requests, event streaming, and capability calls between the desktop host and the Rust runtime.
- `host-capability-adapter`: host-provided privileged capability surface that the Rust runtime invokes for shell, browser, notification, filesystem, and secret-management operations.

### Modified Capabilities

- `multi-agent-orchestration`: execution authority moves from renderer JavaScript to the Rust runtime, and the runtime becomes the source of truth for agent loop state and trace production.
- `space-manager`: Spaces no longer own in-process agent runtimes; they bind UI/browser/terminal resources to runtime-managed runs and agents.
- `llm-provider`: model calls, streaming, and tool-call parsing move behind the Rust runtime boundary instead of being initiated by renderer-side logic.

## Impact

- Affects `app/agent/`, `app/flow/`, `app/browser/`, `app/event_bus/`, `app/workspace/`, and the renderer agent runtime under `web/src/agent_runtime/`.
- Introduces a Rust workspace split across `crony/`, `crates/cronymax`, and `crates/cronygraph`, plus a GIPS-based IPC contract shared between the app host and the runtime.
- Changes how trace events, permission prompts, tool execution, and run persistence are produced and consumed.
- Invalidates or supersedes current assumptions that the renderer owns the agent loop or that future skill/runtime sidecars are the primary authority for orchestration.
