## Why

The `space-agent-integration` change ships a working ReAct agent loop, but multi-agent collaboration is modeled as a workflow graph of LLM/Tool/Condition/Human nodes — a _workflow-engine_ paradigm. Real product teams collaborate by producing **Documents** (PRDs, Tech Specs, Test Cases) and reviewing each other's work. This change re-architects multi-agent orchestration around that organizational model: nodes are first-class **Agents** (LLM + Context + Memory + Skills + Loop), edges carry typed **Documents**, and handoffs gate on **Reviewer-in-the-Agent-Loop** approval. It is the spine of three follow-on changes (`agent-orchestration-ui`, `agent-skills-marketplace`, `document-wysiwyg`).

## What Changes

- **NEW** Agent as a first-class entity (`agent.yaml`): own LLM config, system prompt, attached skills, memory namespace. Reuses the existing `AgentRuntime` ReAct loop as its internal "thinking" engine.
- **NEW** Flow as a higher-level graph (`flow.yaml`): nodes are Agent instances declared in the Flow; edges are **Document handoffs** with typed input/output ports. Replaces the public Subgraph/Condition/Human node-kind UX.
- **NEW** Document as a versioned, on-disk markdown artifact stored under `<workspace>/.cronymax/flows/<flow>/docs/<doc>.md`. Git-trackable; revisions kept in a `.history/` sidecar; review state in a sidecar `reviews.json`.
- **NEW** Document type system: schema-defined doc types (e.g. `prd`, `tech-spec`, `test-plan`) with required sections and validators. Typed ports on agents check connection compatibility.
- **NEW** Reviewer agents: agents whose job is to comment on Documents (not produce new ones). Includes a built-in `Critic` reviewer plus a deterministic `Schema` validator. Per-doc `max_review_rounds` to prevent infinite revision loops.
- **NEW** Hybrid routing: typed-port edges (Flow-defined) provide the default path; `@agent` mentions inside a submitted Document add opt-in extra recipients (escape hatch for bug-fix-style backward routing).
- **NEW** Run lifecycle: a Flow Run is a logical execution with its own id, run directory (`.cronymax/flows/<flow>/runs/<id>/`), trace log, and inbox-emitting events.
- **MODIFIED** `multi-agent-orchestration` capability: deprecate Subgraph/Condition/Human as user-facing node kinds; the existing requirements describe an _Agent's internal loop_, not the Flow.
- **MODIFIED** `space-manager` capability: a Space owns a collection of Flows; Flow runtime resources live under the active Space.
- **NOT IN SCOPE** (deferred to follow-on changes): visual Flow editor, Slack-style channel UI, inbox + status dot UI, skills marketplace, WYSIWYG markdown editor. This change ships a **CLI-flavored** experience: hand-edited YAML, raw markdown, simple chat panel.

## Capabilities

### New Capabilities

- `flow-orchestration`: Flow definition (YAML), Flow Run lifecycle, typed-port routing, `@mention` escape hatch, Run trace.
- `document-collaboration`: Document FS layout, revision history, Document types/schemas, review threads (`reviews.json`), reviewer agents, `max_review_rounds`.
- `agent-entity`: Agent as first-class config (`agent.yaml`) — LLM provider/model, system prompt, attached skills, memory namespace; agent registry per Space.

### Modified Capabilities

- `multi-agent-orchestration`: scope narrows to "single Agent's internal ReAct loop." Public Subgraph/Human/Condition node kinds removed from the user-facing API; routing requirements move to `flow-orchestration`.
- `space-manager`: Space gains a `flows/` collection and an active-Flow-Run pointer.

## Impact

- **`src/agent/`**: `AgentRuntime` gains `agent_id` and `memory_namespace`; multiple instances per Space (one per declared Agent in the active Flow).
- **NEW `src/flow/`**: `flow_runtime.{h,cc}`, `flow_definition.{h,cc}` (YAML loader), `document_store.{h,cc}` (FS-backed), `review_thread.{h,cc}`.
- **NEW `web/flow/`**: minimal chat panel that renders Run events, lets the user post messages, and renders Document submissions as cards.
- **`src/cef_app/bridge_handler.{h,cc}`**: new channels `flow.*`, `document.*`, `review.*`, `agent.registry.*`.
- **`src/agent/agent_graph.h`**: keep as internal-only data model for the per-Agent loop; remove from public bridge surface.
- **Workspace FS contract**: this change introduces `.cronymax/{flows,agents,doc-types}/` as a stable on-disk layout. Treat as v1; future migrations will need a versioning strategy.
- **No new third-party deps** beyond what `space-agent-integration` already brings (SQLite, CEF). YAML parser: pick a header-only library (e.g. `yaml-cpp` from system or vendored).
