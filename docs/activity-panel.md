# Activity Panel

The Activity Panel is a persistent titlebar popover that provides a cross-session
live view of all agent runs in the active space. It supports inline approval of
pending tool-use reviews without leaving the current context.

---

## Anatomy

```
┌──────────────────────────────────────────────┐
│  Activity                                  × │
│──────────────────────────────────────────────│
│  All   Live   Needs Review (2)               │
│──────────────────────────────────────────────│
│  💬 Chat                                     │
│    session:abc3  · 2 runs                    │
│      8f3a1c2e  [done]  4 turns  3.2k tok 12s │
│      a1b2c3d4  [●]  2 turns  1.1k tok …      │
│                                              │
│  🔀 Flows                                    │
│    Flow run abcd1234  · 3 agents             │
│      99aabb01  [done]  5 turns  8k tok  1m   │
│      12345678  [awaiting review]             │
│        ┌──────────────────────────────────┐  │
│        │ bash · rm -rf /tmp/old-data      │  │
│        │          [Allow]  [Deny]         │  │
│        └──────────────────────────────────┘  │
└──────────────────────────────────────────────┘
```

---

## Data Flow

### 1. Snapshot Hydration (on panel open)

```
App.tsx
  └─ useActivityFeed("all")
       └─ browser.send("shell.spaces_list")      → active space id
       └─ browser.send("activity.snapshot")
            ↓  BridgeHandler::HandleActivitySnapshot
            ↓  RuntimeProxy::SendControl({ kind: "get_space_snapshot", space_id })
            ↓  RuntimeHandler::handle_control(GetSpaceSnapshot { space_id })
            ↓  RuntimeAuthority::get_space_snapshot(space_id)
               → filters snapshot.runs by space_id
               → filters snapshot.reviews by run_ids in that space
            ← ControlResponse::SpaceSnapshot { runs, pending_reviews }
       → hydrates Map<RunId, RunEntry> + Map<ReviewId, ReviewEntry>
```

### 2. Live Updates (while panel is open)

```
runtime.subscribe("*", handler)
  on run_status       → update RunEntry.status
  on assistant_turn   → update turn_count, tokens, duration
  on permission_request → upsert ReviewEntry (state: "pending")
  on review_resolved  → remove ReviewEntry
```

Subscription is torn down in the `useEffect` cleanup when the panel unmounts.

### 3. Group Computation

`computeGroups(runs, reviews, activeSpaceId, filter)` → `ActivityGroups`:

| Group key      | Map key       | Description                               |
| -------------- | ------------- | ----------------------------------------- |
| `chatGroups`   | `session_id`  | Runs without `flow_run_id`                |
| `flowGroups`   | `flow_run_id` | Runs spawned from a flow node             |
| `pendingCount` | –             | Count of `RunEntry` with a pending review |

Filter modes:

- `"all"` — all runs in active space
- `"live"` — only `status === "running"`
- `"needs_review"` — only `status === "awaiting_review"`

---

## Run Grouping: `flow_run_id`

`Run` in the Rust runtime carries an optional `flow_run_id: Option<String>`.
The field is populated by `RuntimeHandler::spawn_agent_loop` immediately after
`authority.start_run` succeeds, via `authority.set_run_flow_id(run_id, flow_run_id)`.

Runs **without** a `flow_run_id` are shown under the Chat heading (grouped by
`session_id`). Runs **with** a `flow_run_id` are shown under the Flows heading
(grouped by that id).

---

## Control Protocol Extensions

| Added                                  | Location                                   |
| -------------------------------------- | ------------------------------------------ |
| `ControlRequest::GetSpaceSnapshot`     | `crates/cronymax/src/protocol/control.rs`  |
| `ControlResponse::SpaceSnapshot`       | `crates/cronymax/src/protocol/control.rs`  |
| `RuntimeAuthority::get_space_snapshot` | `crates/cronymax/src/runtime/authority.rs` |
| `RuntimeHandler` dispatch arm          | `crates/cronymax/src/runtime/handler.rs`   |

---

## Bridge Channel

| Channel             | Request payload | Response payload                                |
| ------------------- | --------------- | ----------------------------------------------- |
| `activity.snapshot` | `{}`            | `{ runs: object[], pending_reviews: object[] }` |

Implemented in `BridgeHandler::HandleActivitySnapshot` (`app/browser/bridge_handler.cc`).

---

## Frontend Files

| File                                         | Purpose                                 |
| -------------------------------------------- | --------------------------------------- |
| `web/src/panels/activity/index.html`         | HTML entry point                        |
| `web/src/panels/activity/main.tsx`           | React bootstrap                         |
| `web/src/panels/activity/App.tsx`            | Root component; header, filter tabs     |
| `web/src/panels/activity/useActivityFeed.ts` | Snapshot hydration + live event merging |
| `web/src/panels/activity/ActivityTree.tsx`   | Chat/Flows tree layout                  |
| `web/src/panels/activity/RunRow.tsx`         | Single run row + inline ApprovalCard    |

---

## Relationship to Other Panels

- **Flows panel** (`panels/flows`) — shows the graph view of a specific flow
  definition. Activity panel shows **runtime run instances** across all flows.
- **Chat panel** (`panels/chat`) — hosts per-session chat UI with `ApprovalCard`
  inline in the assistant message stream. Activity panel surfaces the same
  `ApprovalCard` for runs in any session without switching tabs.
- **Inbox** (`panels/inbox`) — stores persistent notifications about completed
  runs. Activity panel is live-only; it is not a historical audit log.
