---
title: Flow Thread Sessions — Prototype
doc_type: prototype
---

# Prototype: Flow Thread Sessions

## Problem Statement

When a user starts a flow run from the Chat tab, all agent activity disappears into a black box. The only visible output is the final document review card and a static trajectory diagram. Users cannot:

- **See what agents are doing** in real time (what they're writing, what tools they're calling)
- **Understand why** a document came out the way it did (no provenance)
- **Intervene mid-run** by messaging a specific node (e.g. "@pm-design please include a section on X")
- **Replay the conversation** that led to a given approval decision

This creates friction in the review loop and forces the user to either blindly approve or reject without context.

---

## Proposed Change

Transform every flow run into a **Slack-style inline thread** — a `ConversationBlock` in the chat timeline that acts as the fork point, embedding a live trajectory diagram and a "View thread →" entry point. Clicking it opens a thread view that shows all node agent messages interleaved chronologically.

The thread is backed by a **real child session**: a sibling session in the runtime that all flow node sub-runs write into, separate from the parent chat session. The user can post messages into the thread with `@node-name` routing.

### Before / After

| | Before | After |
|---|---|---|
| Flow activity visibility | None during run | Per-node streaming in thread view |
| Context for review | Must open Workbench panel | Full conversation in inline thread |
| User can address a node | Not possible | `@pm-design revise section 3` |
| Session topology | All runs share parent session | Flow runs isolated in child session |

---

## UI Behaviour

### 1. Fork Block (main chat timeline)

When the user starts a flow run, the `ConversationBlock` that triggered it gains a **`FlowThreadSummary`** card beneath the message:

```
┌──────────────────────────────────────────────┐
│  Flow thread                                  │
│  [pm-design ✓] → [rd-impl ◉] → [qa-critic ○] │
│  3 events    running…           [View thread] │
└──────────────────────────────────────────────┘
```

- **Trajectory strip** — compact version of `FlowTrajectoryDiagram` showing per-node status chips.
- **Event count** — increments as flow node events arrive in the child session.
- **"running…"** — shown while any sub-run is active.
- **"View thread"** — dispatches `setActiveView({ kind: "thread", blockId, threadId })`.

### 2. Thread View

Replaces the block timeline in the chat feed:

```
┌──────────────────────────────────────────────────────────┐
│  ← Back to chat   [pm-design ✓][rd-impl ◉][qa-critic ○] │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  [pm-design]  "Here is the PRD…"                        │
│               tool: write_document(prd.md)  ✓ 1.2s      │
│                                                          │
│  [rd-impl]    "Starting implementation…"                │
│               tool: read_file(src/main.rs)  (running…)  │
│                                                          │
│  [you] → rd-impl:  please add soft deletes to schema    │
│                                                          │
│  ┌──────────────────────────────────────────────────┐   │
│  │  @  Message flow thread…                         │   │
│  └──────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────┘
```

- **← Back** — returns to `activeView.kind === "main"`.
- **Node bubbles** — consecutive events from the same agent (token stream + tool calls) collapse into one bubble.
- **Document reviews** — pending `FlowDocReviewPanel` is embedded in the thread view.
- **Prompt editor** — initially read-only (Chunk 1); enabled with `@` routing in Chunk 2.

### 3. Main-view sidebar fallback

When `activeView.kind === "main"`, the sidebar shows `FlowDocReviewPanel` with `showHistory={false}` (pending reviews only, no history) so nothing is missed while the user isn't in the thread view.

---

## Session Identity Design

Child session identity follows the **frontend-owns-session-identity** principle already established in the runtime:

```
t=0  crypto.randomUUID() → flowThreadId = "a1b2c3d4-…"
t=0  agentRun() called with child_session_id: flowThreadId
t=0  Frontend subscribes to session:a1b2c3d4 — ready immediately

t=?  First run_status event arrives — UI transitions "starting" → "live"
```

`child_session_id` is an optional field on `ControlRequest::StartRun`. Old callers that omit it get today's behaviour unchanged.

---

## Data Flow

```
App.tsx
  │  crypto.randomUUID() → flowThreadId (before agentRun)
  │  dispatch(attachFlowThread { flowRunId: "", childSessionId: flowThreadId })
  │  agentRun(..., { child_session_id: flowThreadId })
  │
  │  runtime.on(`session:${flowThreadId}`) ──► appendFlowThreadEvent / setFlowThreadRunId
  │
  ▼
run_start.rs (Rust)
  │  upsert child session (flowThreadId)
  │  set_session_fork_point(child_sid, parent_sid, ForkPoint { message_idx, run_id, created_at_ms })
  │  attach flow_run_id to child session
  │  route all flow node sub-runs → child session (flow_session_id)
  │
  ▼
authority.rs
  │  RunStatus events now carry agent_id + flow_run_id
  │  (populated from run.spec["agent_name"] for flow sub-runs)
  │
  ▼
FlowTrajectoryDiagram.tsx
  │  useFlowNodeConversations(sessionId, flowRuns, topology)
  │    ├── session-level subscription discovers new sub-runs via agent_id in RunStatus
  │    ├── per-run subscriptions (run:{runId}) process token / trace events
  │    └── returns Map<agentId, NodeConversation>
  │
  ▼
Thread view (App.tsx)
     Renders FlowThreadEvent[] interleaved from all nodes
```

---

## Affected Files

| File | Change |
|---|---|
| `crates/cronymax/src/protocol/control.rs` | `child_session_id: Option<String>` on `StartRun` |
| `crates/cronymax/src/protocol/events.rs` | `agent_id` + `flow_run_id` on `RunStatus` event |
| `crates/cronymax/src/runtime/authority.rs` | `set_session_fork_point()` helper; enrich RunStatus events with `agent_id`/`flow_run_id` |
| `crates/cronymax/src/runtime/agent_runner.rs` | Store `agent_name` in run spec for identity lookup |
| `crates/cronymax/src/runtime/handler/run_start.rs` | Upsert child session + fork point; route flow sub-runs to `flow_session_id` |
| `crates/cronymax/src/flow/runtime.rs` | Document `"human_feedback"` trigger kind |
| `web/src/shells/runtime.ts` | `child_session_id?: string` on `AgentRunOptions` |
| `web/src/panels/chat/store.ts` | `FlowThread`, `FlowThreadEvent`, `NodeConversation`, `StatusKind` types; `ActiveView`; three new reducers |
| `web/src/panels/chat/App.tsx` | `FlowThreadSummary`, thread view layout, `onViewThread`, child session subscriptions, `attachFlowThread` dispatch |
| `web/src/panels/chat/FlowTrajectoryDiagram.tsx` | `useFlowNodeConversations` hook; clickable `NodeChip`; session-scoped event subscription |
| `web/src/panels/chat/FlowNodeConversationsPanel.tsx` | New component: per-node tab strip + `ContentStreamView` + `TraceViewer` |
| `web/src/panels/chat/FlowDocReviewPanel.tsx` | `showHistory?: boolean` prop to suppress history in main-view sidebar |
| `docs/flow-thread-sessions.md` | Full design doc (session topology, `@` routing state machine, work breakdown) |
| `docs/architecture.md` | One-line entry for `flow-thread-sessions` |

---

## Implementation Phases

### Chunk 1 — Frontend only (read-only thread view)

Works with the current shared-session architecture. Zero Rust changes:

1. `FlowThread` / `FlowThreadEvent` / `NodeConversation` types in `store.ts`
2. `attachFlowThread` / `setFlowThreadRunId` / `appendFlowThreadEvent` reducers
3. `FlowThreadSummary` card in `ConversationBlockView`
4. `activeView` switching in `App.tsx` (main ↔ thread)
5. Thread view layout with `← Back`, trajectory strip, event list
6. `useFlowNodeConversations` hook in `FlowTrajectoryDiagram.tsx`
7. Session-level subscription for child session events in `App.tsx`
8. `FlowDocReviewPanel` `showHistory` prop; main-view shows pending-only

### Chunk 2 — Real child session (Rust + frontend protocol)

Full interactive thread:

9. `child_session_id` on `ControlRequest::StartRun` + `AgentRunOptions`
10. `handle_start_run`: upsert child session, set `parent_session_id` + `fork_point`
11. Flow node runs routed to `flow_session_id` (child when present, parent otherwise)
12. `agent_id` + `flow_run_id` on `RunStatus` events; `agent_name` in run spec
13. `"human_feedback"` trigger kind on `InvocationContext`
14. Thread view prompt editor enabled; `@node-name` re-invokes done/pending nodes

---

## `@` Routing State Machine (Chunk 2)

| Node state | Behaviour on `@node-name message` |
|---|---|
| **done** | Re-invoke: new `InvocationContext` with `trigger.kind = "human_feedback"`, `review_comments = [{ message }]` |
| **pending** | Pre-inject: queue message into the node's first invocation context |
| **running** | Queue: hold message, inject on next invocation (v2) |
| **no `@`** | Send to `__chat__` orchestrator in the child session |

---

## Scope

| In scope | Out of scope |
|---|---|
| Read-only thread view of all node events (Chunk 1) | Inline document editing inside thread |
| Optimistic thread block attached before first event | Diff view between document revisions |
| Lazy subscription to child session events | Auto-scroll to active node bubble |
| `@node-name` re-invocation of done nodes (Chunk 2) | Interrupting a live running node mid-stream |
| `@` picker populated with flow node names | Node-to-node messaging |
| Thread view shows pending document reviews | Historical thread replay from prior sessions |

---

## Acceptance Criteria (Preview)

1. When a flow run starts, a `FlowThreadSummary` card appears beneath the triggering message within 1 second of the first `run_status` event.
2. Clicking "View thread" hides the block timeline and shows the thread view with a `← Back` header.
3. The thread view shows at least one event per node that has started, in arrival order.
4. Each node event displays the agent name label and its text content or tool call.
5. Pending document reviews appear in the thread view without requiring a panel switch.
6. `← Back` returns to the main chat timeline; the fork block still shows the trajectory strip.
7. *(Chunk 2)* Flow node sub-runs appear in the child session, not the parent session, in the runtime state snapshot.
8. *(Chunk 2)* `@pm-design revise X` sent from thread view spawns a new invocation of `pm-design` with `trigger.kind = "human_feedback"`.
