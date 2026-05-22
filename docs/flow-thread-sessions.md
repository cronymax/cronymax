# Flow Thread Sessions — Design & Decisions

> Captured from the `/opsx:explore` discovery session that refined the
> `flow-node-conversations` change away from a floating panel toward
> Slack-style inline threads backed by real child sessions.

---

## 1. The Core Concept

Every flow run creates a **multi-agent chat room** — a child session where
multiple agents are participants and the user can join.

```
Main chat (session "chat-123")
  │
  │  User: "build me a feature X"
  │
  ├─ ConversationBlock "blk-abc"   ←── fork point
  │    ┌──────────────────────────────────────────────────────┐
  │    │  FlowTrajectoryDiagram (embedded)                    │
  │    │  ┌─────────┐  ┌─────────┐  ┌─────────┐              │
  │    │  │pm-design│→ │ rd-impl │→ │qa-critic│              │
  │    │  │  ✓ done │  │ running │  │ pending │              │
  │    │  └─────────┘  └─────────┘  └─────────┘              │
  │    │                                                      │
  │    │  Last turn previews (per node, collapsed)            │
  │    │  "3 replies · click to open thread →"                │
  │    └──────────────────────────────────────────────────────┘
  │
  └─ Thread view (activeView = { kind: "thread", blockId: "blk-abc" })
       ┌──────────────────────────────────────────────────────┐
       │  ← Back to chat     [compact trajectory strip]       │
       │                                                      │
       │  pm-design   "Here is the PRD..."                    │
       │  rd-impl     "Starting implementation..."            │
       │  pm-design   "Note: I revised section 3"             │
       │  qa-critic   "Tests: 14/14 passing"                  │
       │  [you]       @rd-impl please add soft deletes        │
       │                                                      │
       │  ┌──────────────────────────────────────────────┐   │
       │  │  @  Message flow thread...                   │   │
       │  └──────────────────────────────────────────────┘   │
       └──────────────────────────────────────────────────────┘
```

**Participants in the thread session:**

- `pm-design`, `rd-impl`, `qa-critic` — flow node agents (invoked by FlowRuntime)
- `__chat__` — orchestrator (always present)
- `[user]` — human participant via `@` routing

---

## 2. Session Identity Design Decision

### The chosen model: Option B — frontend generates `child_session_id`

The existing runtime already establishes the principle:

```rust
// state.rs — the comment that captures the design principle
/// Session identity is a caller-supplied string (the frontend's
/// `cronymax_chat_tab_id`) so no UUID generation is needed on the
/// Rust side.
pub struct SessionId(pub String);
```

This principle must hold for **both** parent and child sessions.
Frontend generates the ID before calling `agentRun`, enabling optimistic
UI and idempotent retries.

### Why not Option A (flow_run_id = child_session_id)?

`flow_run_id` is backend-generated: `"run-{uuid_simple}"`. Using it directly
as a session ID would:

1. Break the frontend-owns-session-identity principle for a special case
2. Force the frontend to wait for the first `run_status` event to know
   what session to subscribe to (a latency gap exists)
3. Create an implicit naming convention that everyone must know
4. Make it harder to restart a flow in the same thread (different
   `flow_run_id`, but same `child_session_id` intent)

### Why not Option C (backend generates a new UUID)?

- Same latency gap problem as Option A
- Frontend must wait to receive the child session ID before subscribing
- Breaks the "frontend owns identity" principle in a different way
- Adds a new field to the `agentRun` response without providing Option B's benefits

### Option B: the right long-term direction

```
t=0  crypto.randomUUID() → flowThreadId = "a1b2c3d4-..."
t=0  agentRun() called with child_session_id: flowThreadId
t=0  Frontend: renders thread block, sets up session:a1b2c3d4 subscription
t=0  UI shows "starting thread..." optimistically

     (no gap — UI is ready before the first event arrives)

t=?  First run_status event arrives
t=?  UI transitions from "starting" to "live"
```

vs. Option A:

```
t=0  agentRun() called
t=1  Rust generates flow_run_id = "run-abc123"
t=2  First run_status event arrives at frontend
t=3  Frontend learns child_session_id = "run-abc123"
t=4  Frontend renders thread block, sets up subscriptions

     (gap between t=0 and t=3 where frontend knows nothing)
```

### Option B properties

| Property                      | Option B                                              |
| ----------------------------- | ----------------------------------------------------- |
| Frontend-owns-identity        | ✓ preserved                                           |
| Optimistic UI possible        | ✓ yes — ID known at t=0                               |
| Idempotent retries            | ✓ same UUID resent on retry                           |
| No new response fields needed | ✓ backend doesn't need to echo back                   |
| Backward compatible           | ✓ `child_session_id` is optional; old calls unchanged |
| Protocol surface added        | +1 optional field on `StartRun`                       |

---

## 3. Protocol Layer Changes

The `child_session_id` travels through the full stack:

```
┌────────────────────────────┬──────────────────────────────────────────────┐
│ Layer                      │ Change                                        │
├────────────────────────────┼──────────────────────────────────────────────┤
│ App.tsx                    │ crypto.randomUUID() before agentRun() when    │
│                            │ flow_id is present; pass as child_session_id; │
│                            │ store in state for thread routing              │
├────────────────────────────┼──────────────────────────────────────────────┤
│ runtime.ts                 │ Add child_session_id?: string to              │
│                            │ AgentRunOptions; pass in agentRun() request   │
├────────────────────────────┼──────────────────────────────────────────────┤
│ bridge_handler.cc          │ No change — unknown JSON fields pass through  │
│                            │ (serde #[default] handles on Rust side)       │
├────────────────────────────┼──────────────────────────────────────────────┤
│ control.rs                 │ Add child_session_id: Option<String> to       │
│                            │ ControlRequest::StartRun                      │
├────────────────────────────┼──────────────────────────────────────────────┤
│ run_start.rs               │ When flow_id present and child_session_id     │
│                            │ present: upsert child session, set            │
│                            │ parent_session_id + fork_point, route all     │
│                            │ flow node runs to child session               │
├────────────────────────────┼──────────────────────────────────────────────┤
│ authority.rs               │ New set_session_parent() helper (or inline    │
│                            │ in run_start.rs); fills in the two fields on  │
│                            │ Session that already exist but are never set  │
└────────────────────────────┴──────────────────────────────────────────────┘
```

### `ControlRequest::StartRun` — before and after

```rust
// Before
StartRun {
    space_id: String,
    payload: serde_json::Value,
    session_id: Option<String>,      // parent chat session
    session_name: Option<String>,
    agent_id: Option<String>,
}

// After
StartRun {
    space_id: String,
    payload: serde_json::Value,
    session_id: Option<String>,      // parent chat session (unchanged)
    session_name: Option<String>,
    agent_id: Option<String>,
    child_session_id: Option<String>, // NEW: frontend-generated flow thread ID
}
```

### `Session` fields (already exist, never set)

```rust
// state.rs — these fields exist today, populated for the first time here
pub struct Session {
    pub id: SessionId,
    pub parent_session_id: Option<SessionId>,  // ← set by run_start.rs
    pub fork_point: Option<ForkPoint>,          // ← set by run_start.rs
    // ...
}

pub struct ForkPoint {
    pub message_idx: usize,    // = prior_thread.len() at fork time
    pub run_id: Option<RunId>, // = the orchestrator run just created
    pub created_at_ms: i64,    // = now_ms()
}
```

The `fork_point.message_idx` value is `prior_thread.len()` — available in
`handle_start_run` without any additional queries because `prior_thread` is
already loaded from ChatStore at that point.

---

## 4. The `@` Routing State Machine

When the user types `@pm-design revise the schema` in the thread view,
the behavior depends on that node's current state:

```
@pm-design sent when pm-design is...

  ┌─────────────┬──────────────────────────────────────────────────────┐
  │  done       │ Re-invoke: spawn a new InvocationContext with         │
  │  (finished) │ trigger.kind = "human_feedback",                     │
  │             │ review_comments = [{ message: user's text }].        │
  │             │                                                       │
  │             │ This is identical to "rejected_requeue" but           │
  │             │ human-initiated rather than reviewer-agent-initiated. │
  │             │ ← most common case, most implementable first         │
  ├─────────────┼──────────────────────────────────────────────────────┤
  │  pending    │ Pre-inject: queue message to be included in the       │
  │  (not run)  │ InvocationContext before the node's first invocation. │
  │             │ ← moderate complexity                                │
  ├─────────────┼──────────────────────────────────────────────────────┤
  │  running    │ Queue: hold the message, inject on next invocation.  │
  │  (active)   │ Interrupting a live agent turn mid-stream is hard.    │
  │             │ ← defer to v2                                        │
  └─────────────┴──────────────────────────────────────────────────────┘
```

### The `InvocationContext` already carries this pattern

The existing `InvocationContext` struct handles reviewer feedback via
`review_comments`. A `"human_feedback"` trigger is structurally identical:

```rust
pub struct InvocationContext {
    pub node_id: String,
    pub owner: String,
    pub trigger: InvocationTrigger,   // new variant: human_feedback
    pub available_docs: Vec<AvailableDoc>,
    pub pending_ports: Vec<String>,
    pub review_comments: Option<Vec<ReviewComment>>,  // reused for user message
}
```

The rejected_requeue → human_feedback diff is:

- Trigger kind changes
- Source of `review_comments` is the human message, not a reviewer agent
- The machinery for spawning the agent is identical

### Without `@` — default recipient

Message goes to `__chat__` (the orchestrator) in the child session.
This is a new `agentRun` call with:

- `session_id: flowThreadId`
- `agent_id: "__chat__"` (or unset, which defaults to `__chat__`)

The `@` picker in thread view is populated with flow node names instead of
registered global agents. `parseMention()` already handles this — only the
list of agent names passed to it changes.

---

## 5. Runtime Session Topology

```
Before (current):
  Session "chat-123"
    └── Run "r-orchestrator"  (flow_run_id = "fr-xyz")
    └── Run "r-pm-design"     (flow_run_id = "fr-xyz")
    └── Run "r-rd-impl"       (flow_run_id = "fr-xyz")
    └── Run "r-qa-critic"     (flow_run_id = "fr-xyz")

    All runs share the same session. Node output
    streams into the same conversation history.

─────────────────────────────────────────────────────────────

After (new design):
  Session "chat-123"                     (parent)
    └── Run "r-orchestrator"  ──────▶  Session "a1b2c3d4" (child)
                                          parent_session_id: "chat-123"
                                          fork_point: { message_idx: 2,
                                                        run_id: r-orch,
                                                        created_at_ms: ... }
                                          └── Run "r-pm-design"
                                          └── Run "r-rd-impl"
                                          └── Run "r-qa-critic"
```

---

## 6. Frontend View Architecture

```
store.ts
  activeView: { kind: "main" }
           | { kind: "thread", blockId: string, threadId: string }

  ConversationBlock {
    id, role, content, ...
    flowThread?: {
      flowRunId: string         // "fr-xyz" — links to FlowRuntime
      childSessionId: string    // "a1b2c3d4" — the child session to subscribe to
      events: FlowThreadEvent[] // chronological interleave across all nodes
    }
  }

  FlowThreadEvent {
    agentId: string
    kind: "token" | "trace" | "run_started" | "run_done" | ...
    segment?: string     // content token
    entry?: TraceEntry   // tool call / tool result
    seqNum: number       // global ordering across all nodes
    ts: number           // wall clock
  }
```

### Main view — fork block

The `ConversationBlock` that triggered the flow shows:

```
┌──────────────────────────────────────────────────────────┐
│  FlowTrajectoryDiagram                                   │
│  [pm-design ✓] → [rd-impl ◉] → [qa-critic ○]           │
│                                                          │
│  pm-design    "Here is the product requirements..."      │
│               (last full turn, collapsed)                │
│  rd-impl      streaming...                               │
│                                                          │
│  Reviews: 0 pending · 2 approved                        │
│                                                          │
│  3 messages · click to open thread →                    │
└──────────────────────────────────────────────────────────┘
```

`onExpand` dispatches `setActiveView({ kind: "thread", blockId, threadId })`.
Today this is a `noop` in `ThreadSummary` — this is the wiring gap.

### Thread view

```
┌──────────────────────────────────────────────────────────┐
│  ← Back to chat   [pm-design ✓][rd-impl ◉][qa-critic ○] │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  [pm-design]  "Here is the PRD..."                      │
│               tool: write_document(prd.md)              │
│                                                          │
│  [rd-impl]    "Starting implementation..."              │
│               tool: read_file(src/main.rs)              │
│               (streaming — live tail)                   │
│                                                          │
│  [you] → rd-impl:  add soft deletes to the schema       │
│                                                          │
│  ┌──────────────────────────────────────────────────┐   │
│  │  @  Message flow thread...                       │   │
│  └──────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────┘
```

---

## 7. Work Breakdown

The implementation splits cleanly into two independent chunks:

### Chunk 1 — Frontend only (no Rust changes)

Read-only thread view. Works with the current shared session architecture:

- `FlowForkBlock` embedded in `ConversationBlock`
- `activeView` switching (main ↔ thread)
- `FlowThreadEvent[]` sorted by `seqNum` for chronological interleave
- Thread view with node message bubbles (consecutive same-agent events collapse)
- Compact `FlowTrajectoryDiagram` header in thread view
- `onExpand` wired in `ThreadSummary` (currently a noop)
- Fork block preview: last turn per node + trajectory strip
- No prompt editor in thread view yet (read-only)

### Chunk 2 — Real child session (Rust + frontend protocol)

Full interactive thread:

- `child_session_id: Option<String>` on `ControlRequest::StartRun`
- `AgentRunOptions.child_session_id` in runtime.ts
- `crypto.randomUUID()` generated before `agentRun()` in App.tsx
- `handle_start_run`: upsert child session, set `parent_session_id` + `fork_point`
- Flow node runs route to child session instead of parent
- `"human_feedback"` trigger kind on `InvocationContext`
- Thread view prompt editor enabled
- `@node-name` re-invokes done/pending nodes in child session
- No `@` → sends to `__chat__` in child session

---

## 8. Open Questions

1. **`FlowTrajectoryDiagram` scoping**: Should the embedded fork block version
   be a new component scoped to one `flow_run_id`, or the same component
   reused with a filter? The current version shows all flow runs.

2. **Default recipient with no `@` in thread**: Goes to `__chat__`. But if
   `__chat__` completed its turn, does a new user message in the child session
   trigger a new `__chat__` run, or is it queued? This is equivalent to
   sending a new message in any chat session — same behavior, different framing.

3. **What happens to `FlowNodeConversationsPanel`**: The floating panel
   created for the old design. Delete entirely or reuse parts (e.g. `ReviewsSection`)?

4. **Node message grouping**: In the thread timeline, do consecutive turns
   from the same agent collapse into one bubble (like Slack), or are they
   always separate? Recommendation: collapse within a single invocation
   (tool calls + output = one bubble), separate invocations are always separate.
