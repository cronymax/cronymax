# Runtime Event Scoping & Traffic Isolation

Design notes for scoped event routing in the Rust runtime — eliminating
cross-session event leakage and establishing a clean isolation model across
workspace, session, run, flow run, PTY, and thread scopes.

---

## Problem: Two Delivery Paths With Very Different Isolation

Today the runtime has two independent event delivery mechanisms that coexist
awkwardly:

```
┌─────────────────── Rust Runtime (Authority) ──────────────────┐
│                                                                │
│  emit("run:{id}", payload)                                     │
│  emit_for_run(id, payload)         ← run-scoped; well isolated │
│  emit("flow:{event}", payload)     ← flat; NOT run-scoped      │
│  emit("terminal:{id}", payload)    ← PTY; not session-scoped   │
│  emit_log(...)   →  "*"            ← goes to EVERYONE          │
│                                                                │
│  Subscriptions:                                                │
│    sub_A: topic = "run:abc-123"   ← targeted, exact-match      │
│    sub_B: topic = "*"             ← receives ALL events        │
└──────────────────────┬─────────────────────────────────────────┘
                       │ GIPS
          ┌────────────┴────────────────────┐
          │ C++ Bridge                       │
          │                                 │
          │ sub_B fan-out                   │
          │   └─► broadcast_event("event", ALL)   ← path 2      │
          │                                 │
          │ sub_A fan-out                   │
          │   └─► kMsgRuntimeEvent(sub_id)  ← path 1            │
          └────────────┬────────────────────┘
                       │ IPC (shared per Space, all tabs)
          ┌────────────┴────────────────────────────────┐
          │ TypeScript / Renderer                        │
          │                                              │
          │  browser.on("event", ...)                    │
          │    └─► runtimeWildcard handlers              │
          │          ALL events, ALL tabs, client-filter │
          │                                              │
          │  runtime.on("run:{id}", cb)                  │
          │    └─► targeted via sub_A; well-isolated     │
          └──────────────────────────────────────────────┘
```

**Path 1 — Targeted subscriptions** (`runtime.on("run:{id}", cb)`):

- Component sends `Subscribe { topic: "run:abc" }` via control channel
- Rust authority creates a dedicated `mpsc::UnboundedSender` for that topic
- Fan-out task pumps only matching events directly to that subscriber
- Well-isolated: only events on the exact topic are delivered

**Path 2 — Wildcard broadcast** (`runtime.on("*", cb)` / `browser.on("event", ...)`):

- `WireSpaceEventCallback` holds a `"*"` subscription in Rust
- Every event emitted anywhere crosses IPC to every renderer tab
- Clients then filter in JS by `scope.flow_id` / `scope.run_id`
- `useEventStream.ts` uses this path; all token deltas for all runs in the
  workspace arrive in every chat tab regardless of relevance

**Hidden cost:** an event on `"run:abc"` crosses IPC _twice_ if both paths are
active — once via targeted fan-out, once via wildcard broadcast. For
high-frequency events (token deltas, PTY output) this is significant overhead.

---

## The Split Registry Problem

Session-to-flow-run association is maintained via a secondary in-memory map
rather than the primary session data model:

```
snapshot.sessions        snapshot.runs           FlowRuntime  (separate)
┌────────────────┐       ┌─────────────────┐     ┌──────────────────────┐
│ session A      │       │ run-1            │     │ flow_run-X           │
│  run_ids: [    │──────►│  session_id: A   │     │  originating_session │
│    run-1,      │       │  flow_run_id: X  │     │  = "A"               │
│    run-2       │       ├─────────────────┤     │                      │
│  ]             │       │ run-2            │     │ NOT in snapshot      │
└────────────────┘       │  session_id: A   │     │ NOT in run_ids       │
                         └─────────────────┘     └──────────────────────┘
                                                         │
                                            RuntimeAuthority.flow_sessions
                                            HashMap<flow_run_id, session_id>
                                            (in-memory; re-seeded on restart)
```

`FlowRunState` lives in a separate `FlowRuntime` data structure, not in the
authority snapshot. This forces the `flow_sessions` HashMap workaround for
routing and means `Session.run_ids` is incomplete — it only lists agent runs,
not flow runs.

---

## The Scope Hierarchy

Session is the universal root. Every run belongs to a session:

```
workspace:{id}
│
├── session:{id}                   ← chat tab; root of user context
│   │
│   ├── run:{run_id}               ← agent LLM loop
│   ├── flow_run:{id}              ← flow execution (currently split registry)
│   ├── shell:{block_id}           ← embedded PTY in chat (future)
│   │
│   └── thread:{child_session_id}  ← forked child session (future)
│       ├── run:{run_id}
│       └── thread:{...}           ← recursively forkable
│
└── terminal:{pty_id}              ← workspace-level terminal (no session parent)
```

### Shell block vs. workspace terminal

Two PTY-backed things; semantically distinct:

|                      | Shell block                 | Workspace terminal                |
| -------------------- | --------------------------- | --------------------------------- |
| Parent scope         | `session:{id}`              | `workspace:{id}`                  |
| Lifecycle            | Tied to the conversation    | Survives session changes          |
| Created by           | Agent or user within a chat | User opens terminal tab           |
| Output relevance     | Part of the session context | Independent tool                  |
| Routing              | emits to `session:{sid}`    | emits to `terminal:{pty_id}` only |
| High-freq PTY output | Session subscribers see it  | No cross-contamination            |

### Background / programmatic runs

All runs should have a session. Options:

- **Synthetic session per workspace** (`session:system`): background runs are
  assigned here. Activity panel subscribes to it. Not surfaced as a chat tab.
- **Nullable with targeted subscription only**: runs without `session_id` emit
  only to `run:{id}`. Only panels that explicitly know the run ID can subscribe.

The system-session approach is preferred — it keeps the subscription model
uniform and gives the activity panel a clean aggregate topic.

---

## Thread as Forked Session

A thread is a child session forked from a parent at a specific point in the
conversation history:

```
Session A (chat tab)
│
│  [message 1]
│  [run: agent did X]  ─── run-1
│  [message 2]
│  ┌──────────────────────────────────────────────────┐
│  │ Thread  →  session B                              │
│  │   parent_session_id: A                            │
│  │   fork_point: { after_run_id: run-1 }             │
│  │                                                   │
│  │   [run: alternative approach Y]  ─── run-3        │
│  └──────────────────────────────────────────────────┘
│  [message 3]
```

Data model additions to `Session`:

```rust
pub struct Session {
    // existing
    pub id: SessionId,
    pub space_id: SpaceId,
    pub run_ids: Vec<RunId>,

    // new
    pub parent_session_id: Option<SessionId>,  // set on child thread sessions
    pub fork_point: Option<ForkPoint>,          // where in parent this branched
}

pub struct ForkPoint {
    pub after_run_id: Option<RunId>,     // fork after this run completed
    pub message_index: Option<usize>,    // or at this message position
}
```

`run_ids` handles discovery for threads identically to regular sessions — no
new machinery. Thread sessions are regular sessions with a `parent_session_id`.

**Subscription model for threads:**

- Parent chat panel holds `session:{parent_id}` subscription — receives
  summary/meta events from child threads (thread started, thread completed)
- Full event stream is only needed when user "enters" the thread — at which
  point the panel subscribes to `session:{child_id}` as its primary topic
- This avoids the parent panel receiving token-delta floods from child threads

---

## Target: Multi-Topic Emit

The cleanest routing implementation requires no changes to the subscription
filter language. Topics stay **exact-match**. Routing is achieved by emitting
to multiple topics simultaneously:

```
Current:
  emit_for_run(run_id, payload)
    → emits on "run:{id}" only

Proposed:
  emit_for_run_scoped(run_id, session_id, payload)
    → emits on "run:{id}"         ← targeted (activity panel, direct subscribers)
    → emits on "session:{sid}"    ← session aggregate (chat tab)

  emit_for_flow_run_scoped(flow_run_id, session_id, payload)
    → emits on "flow_run:{fid}"   ← targeted
    → emits on "session:{sid}"    ← session aggregate

  emit_for_shell_block(block_id, session_id, payload)  [future]
    → emits on "shell:{bid}"      ← targeted
    → emits on "session:{sid}"    ← session aggregate

  emit_for_terminal(pty_id, payload)
    → emits on "terminal:{pty_id}" only  ← intentionally NOT session-routed
```

### Efficient multi-topic fan-out

One lock + one subscription walk; deliver if topic matches any of the targets:

```rust
fn emit_to_many(
    inner: &mut AuthorityInner,
    topics: &[String],
    payload: RuntimeEventPayload,
) {
    let now = now_ms();
    let mut dropped = vec![];
    for (id, sub) in inner.subscriptions.iter_mut() {
        if topics.iter().any(|t| sub.matches(t)) {
            let event = RuntimeEvent { sequence: sub.next_seq, emitted_at_ms: now, payload: payload.clone() };
            sub.next_seq += 1;
            if sub.tx.send(event).is_err() {
                dropped.push(*id);
            }
        }
    }
    for id in dropped { inner.subscriptions.remove(&id); }
}
```

This replaces calling `emit_locked` twice (which would double the subscription
walk and incorrectly increment sequence numbers twice for a subscriber that
matches multiple topics in the list).

---

## Target Subscription Model (Frontend)

```
Chat tab (session A)
  subscribe("session:{sid_A}")
  → receives: run events, flow_run events, shell block output, session meta
  → does NOT receive: session B's events, workspace terminal output

Workspace terminal tab
  subscribe("terminal:{pty_id}")
  → receives: PTY stdout/stderr/resize for that terminal only
  → does NOT receive: any agent/session events

Activity panel
  subscribe("session:system")      ← background runs
  subscribe("session:{sid_A}")     ← for each open session
  subscribe("session:{sid_B}")     ← ...
  OR subscribe("workspace:{id}")   ← if a workspace-aggregate topic exists

Thread embed within session A
  subscribe("session:{child_sid}") ← only when expanded/entered
  parent receives meta via session A subscription
```

---

## What Needs to Change

### Rust runtime (`crates/cronymax/`)

| File                   | Change                                                                       |
| ---------------------- | ---------------------------------------------------------------------------- |
| `runtime/state.rs`     | Add `parent_session_id`, `fork_point` to `Session`                           |
| `runtime/state.rs`     | Ensure flow run IDs are tracked in `Session.run_ids` (or add `flow_run_ids`) |
| `runtime/authority.rs` | Add `emit_to_many(topics, payload)` replacing dual `emit_locked` calls       |
| `runtime/authority.rs` | `emit_for_run_scoped(run_id, session_id, payload)`                           |
| `runtime/handler.rs`   | Pass `session_id` through all emit call sites                                |
| `flow/runtime.rs`      | Route flow run events through `session:{sid}` topic                          |

### C++ bridge (`app/browser/`)

| File              | Change                                                                                                                                   |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `shells/space.cc` | Replace `"*"` subscription in `WireSpaceEventCallback` with targeted per-session subscriptions; or eliminate wildcard broadcast entirely |
| `space_manager.h` | Track per-tab subscription handles (session_id → sub_id)                                                                                 |

### TypeScript (`web/src/`)

| File                                     | Change                                                            |
| ---------------------------------------- | ----------------------------------------------------------------- |
| `shells/bridge.ts`                       | Deprecate `runtime.on("*", cb)` wildcard path                     |
| `panels/channel/hooks/useEventStream.ts` | Switch from `browser.on("event")` to `runtime.on("session:{id}")` |
| `panels/chat/FlowDocReviewPanel.tsx`     | Already uses targeted subscription (post session-routing fix)     |
| `hooks/useRuntimeEvent.ts`               | No change needed; already handles targeted topics                 |

---

## Topic Naming Convention

| Topic pattern       | Scope                           | Subscriber                       |
| ------------------- | ------------------------------- | -------------------------------- |
| `session:{id}`      | All events in a session         | Chat tab panel                   |
| `run:{id}`          | Single agent loop run           | Activity row, direct subscribers |
| `flow_run:{id}`     | Single flow execution           | Flow panel, direct subscribers   |
| `shell:{block_id}`  | Shell block PTY output          | Shell block component (future)   |
| `terminal:{pty_id}` | Workspace terminal PTY          | Terminal tab                     |
| `session:system`    | Background/programmatic runs    | Activity panel                   |
| `workspace:{id}`    | All workspace events (reserved) | Reserved; possibly unused        |

Topic filter language stays **exact-match**. No prefix matching, no wildcards
except `"*"` which should be eliminated from production use.

---

## Open Questions

1. **Unify flow runs into snapshot or keep separate registry?** The `flow_sessions`
   HashMap is a workaround for `FlowRunState` living outside the authority snapshot.
   Bringing flow runs into `snapshot.runs` as first-class `Run` objects would let
   `Session.run_ids` be the single source of truth for all run types.

2. **System session or nullable session_id?** Every run needs a session for the
   routing model to be uniform. Either always create a synthetic system session,
   or special-case "no session" runs to emit only to `run:{id}`.

3. **Activity panel subscription strategy.** Does it hold one subscription per
   open session (and update as sessions are opened/closed), or is a
   `workspace:{id}` aggregate topic cleaner?

4. **Thread meta-events on parent topic.** When a thread's run completes, should
   an event bubble up to the parent session's topic? If so, what shape does the
   meta-event have, and how does the parent panel know which thread it came from?
