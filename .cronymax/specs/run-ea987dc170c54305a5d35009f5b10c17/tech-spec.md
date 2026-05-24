---
title: Flow Thread Sessions — Tech Spec
doc_type: tech-spec
---

# Flow Thread Sessions — Technical Specification

## Summary

This feature transforms every flow run started from the Chat panel into an **inline conversation thread** — a live, Slack-style feed of per-agent streaming activity, tool calls, and document reviews embedded directly in the chat timeline.

Delivery is split into two chunks:

- **Chunk 1 (frontend-only, this spec):** Read-only thread view using the existing shared-session event subscription. Zero Rust changes required. Ships the complete visual shell.
- **Chunk 2 (full child session):** Rust protocol changes to isolate flow node sub-runs in a dedicated child session and enable `@node-name` re-invocation from the thread prompt. Scoped to a follow-on spec.

---

## Approach

### 1. State Model (`store.ts`)

Three new types are added:

```ts
interface FlowThreadEvent {
  agentId: string;
  kind: "token" | "tool_call" | "trace" | "status";
  segment?: ContentSegment;  // for token/tool_call kinds
  entry?: TraceEntry;        // for trace kind
  seqNum: number;
  ts: number;
}

interface FlowThread {
  flowRunId: string;          // "" until first run_status confirms it
  childSessionId: string;     // crypto.randomUUID() before agentRun() is called
  events: FlowThreadEvent[];
}

// ActiveView already exists; its "thread" variant already has blockId + threadId fields
```

`ConversationBlock` already carries an optional `flowThread?: FlowThread` field (currently used for future extension). No new field additions needed.

Three new reducer cases (already scaffolded in `store.ts`):

| Action | Effect |
|---|---|
| `attachFlowThread` | Attach `FlowThread` to a `ConversationBlock` when the flow run starts |
| `setFlowThreadRunId` | Backfill `flowRunId` when first `run_status` event arrives |
| `appendFlowThreadEvent` | Append a new `FlowThreadEvent` into `block.flowThread.events` |

### 2. Event Subscription (`App.tsx`)

When a flow run is launched:

1. `childSessionId = crypto.randomUUID()` is generated before `agentRun()` is called.
2. `dispatch(attachFlowThread { id: blockId, flowThread: { flowRunId: "", childSessionId, events: [] } })` is called immediately.
3. `runtime.on(`session:${chatSessionId}`)` already exists for the parent session — **we reuse it** for Chunk 1. The existing `FlowTrajectoryDiagram`/`FlowNodeConversationsPanel` hooks already pull `NodeConversation` data from `run_status` events on the parent session. In Chunk 1 the `appendFlowThreadEvent` reducer is fed from the **same parent session** events, filtered by `agent_id` and `flow_run_id`.

Specifically, inside the parent session subscription:

```ts
if (agentId && flowRunId && flowRunId === block.flowThread?.flowRunId) {
  // route token / tool / trace events into appendFlowThreadEvent
}
```

The first `run_status` event with `flow_run_id` populated triggers `setFlowThreadRunId`.

### 3. `FlowThreadSummary` Card

A new sub-component rendered inside `ConversationBlockView` when `block.flowThread` is defined:

```
┌──────────────────────────────────────────────────┐
│  ⑂ Flow thread                                    │
│  [pm-design ✓] → [rd-impl ◉] → [qa-critic ○]    │
│  12 events   running…         [View thread →]    │
└──────────────────────────────────────────────────┘
```

Props:
- `flowThread: FlowThread`
- `flowTopology: FlowTopology | null` (loaded from localStorage)
- `onViewThread: () => void` — dispatches `setActiveView({ kind: "thread", blockId, threadId: flowThread.childSessionId })`

The trajectory strip reuses the existing `NodeChip` primitive from `FlowTrajectoryDiagram.tsx` (extract into a small shared helper if needed). Node status is derived from the latest `FlowThreadEvent` per `agentId` whose kind is `"status"`.

**Event count badge** shows `block.flowThread.events.length`.

**Running indicator** is shown while any node's last-known status is `"running"`.

### 4. Thread View Layout (`App.tsx`)

When `state.activeView.kind === "thread"`:

- The block timeline is replaced by the thread view.
- Layout:

```
┌────────────────────────────────────────────────┐
│ ← Back   [pm-design ✓][rd-impl ◉][qa-critic ○]│  ← fixed header
├────────────────────────────────────────────────┤
│                                                │
│  [pm-design]  "Here is the PRD…"              │  ← scrollable event feed
│               tool: write_document  ✓ 1.2s   │
│  ─────────────────────────────────────────    │
│  [rd-impl]    "Starting implementation…"      │
│                                                │
│  ┌── FlowDocReviewPanel ──────────────────┐   │  ← inline review cards
│  │  Review: prd  [Approve] [Req Changes]  │   │
│  └────────────────────────────────────────┘   │
│                                                │
├────────────────────────────────────────────────┤
│  @ Message flow thread…  (disabled — Chunk 2) │  ← read-only prompt
└────────────────────────────────────────────────┘
```

**Event feed rendering rules:**

- Events are rendered in `seqNum` order (arrival order).
- Consecutive events from the same `agentId` are visually grouped under one agent bubble (no repeated name header until the `agentId` changes).
- `kind === "token"` → render `segment` through a lightweight `ContentStreamView` instance.
- `kind === "tool_call"` → render a `ToolCallCard` (reuse existing component).
- `kind === "trace"` → render a compact `TraceViewer` row.
- `kind === "status"` → render a status transition pill (e.g. "rd-impl awaiting review").
- `FlowDocReviewPanel` is rendered between the event feed and the read-only prompt, using the parent `sessionId` prop and `showHistory={false}` — consistent with main-view sidebar behaviour.

**← Back button:** dispatches `setActiveView({ kind: "main" })`. The `FlowThreadSummary` card in the timeline remains visible, preserving trajectory strip state.

### 5. Main-View Sidebar (unchanged logic, clarified contract)

When `activeView.kind === "main"` the existing `FlowDocReviewPanel` in the sidebar already renders. The only explicit change: confirm `showHistory={false}` is passed in the sidebar mount so pending reviews are never silently missed (AC #8).

### 6. Read-Only Prompt in Thread View

The `<Textarea>` prompt editor is rendered but has `disabled` attribute and a placeholder:

> `"Thread messaging is coming soon — flow node @-routing will be available in a future update."`

The `<ArrowUp>` submit button is also disabled.

---

## Key Decisions

### KD-1: Chunk 1 uses parent session, not child session

The PRD requirement for a real child session (AC #10–#13) is Chunk 2. In Chunk 1 we fan `FlowThreadEvent` entries from the **existing parent session subscription** (`session:<chatSessionId>`). This avoids any Rust protocol changes and ships the complete visual shell immediately.

The `childSessionId` field is still allocated (via `crypto.randomUUID()`) so that Chunk 2 can transparently swap in the real child session subscription without a reducer schema change.

### KD-2: `FlowThread.events` is a flat append-only log, not a Map per agent

Interleaving agent events chronologically is simpler with a flat array keyed by `seqNum`. Grouping by `agentId` for rendering is a pure rendering concern, handled in the thread view's render pass.

### KD-3: `NodeChip` extracted as a shared primitive

Both `FlowThreadSummary` and the thread view header need per-node status chips. The chip component in `FlowTrajectoryDiagram.tsx` is extracted to `web/src/panels/chat/NodeChip.tsx` (≈20 lines) to avoid duplication.

### KD-4: No new localStorage keys for Chunk 1

`FlowThread` data is session-lived (populated at runtime from event subscriptions). It is intentionally **not** persisted to localStorage — if the app restarts mid-run the thread is empty and will re-populate as events arrive in the live session. Persisting events would require additional storage budget and migration work with low payoff (reviewed in the PRD non-goals: "Historical thread replay from prior sessions").

### KD-5: `FlowDocReviewPanel` is rendered in both views — same component, different props

- In main view (sidebar): `showHistory={false}` (pending only).
- In thread view (inline): `showHistory={false}` (pending only; history not needed inline).
- Only in the full app where a second mount is appropriate: `showHistory={true}` is used as before.

---

## File Changes

| File | Change |
|---|---|
| `web/src/panels/chat/store.ts` | `FlowThread`, `FlowThreadEvent` types finalized; `attachFlowThread`, `setFlowThreadRunId`, `appendFlowThreadEvent` reducer cases; `ActiveView` "thread" variant (already present) |
| `web/src/panels/chat/App.tsx` | Add `FlowThreadSummary` component; add thread view layout; add child `runtime.on(session:...)` fan-out for `appendFlowThreadEvent`; wire `FlowDocReviewPanel` with `showHistory={false}` in sidebar |
| `web/src/panels/chat/NodeChip.tsx` | **New file.** Extracted `NodeChip` primitive (status dot + label) shared by `FlowThreadSummary` and thread view header |
| `web/src/panels/chat/FlowTrajectoryDiagram.tsx` | Import `NodeChip` from new file; minor refactor (chip rendering delegated) |
| `web/src/panels/chat/FlowDocReviewPanel.tsx` | No changes; `showHistory` prop already in place |

---

## Testing Strategy

### Unit / Component Tests

- **`FlowThreadSummary` rendering:** Given a `FlowThread` with 3 events spanning 2 agents, confirm the event count badge shows `3`, the trajectory strip renders 2 chips, and the "View thread" button is present.
- **`FlowThreadSummary` — no thread:** Given a `ConversationBlock` with `flowThread = undefined`, confirm no summary card renders.
- **Thread view event grouping:** Given 4 events (A, A, B, A), confirm 3 agent bubbles (A-group, B, A).
- **`appendFlowThreadEvent` reducer:** Verify events are appended in order and `seqNum` is preserved.
- **`setFlowThreadRunId` reducer:** Verify `flowRunId` backfills correctly; other blocks are untouched.

### Integration / E2E Tests

- **Flow run → summary card appears within 1 s of first `run_status` event** (AC #1): Mock `runtime.on` to emit a synthetic `run_status` event after dispatch of `attachFlowThread`; assert `FlowThreadSummary` appears and event count > 0.
- **View thread / Back navigation** (AC #3, #7): Simulate `onViewThread` click; assert active view switches; assert `← Back` restores main view; assert `FlowThreadSummary` still visible.
- **Pending review inline in thread view** (AC #6): With a pending review in `FlowDocReviewPanel`, open thread view; assert `ReviewCard` is present inside the thread layout.
- **Read-only prompt** (AC #9): Assert the thread view textarea is `disabled` and shows the expected placeholder.

### Manual Smoke Tests

1. Start a `software-dev-cycle` flow from the Chat panel.
2. Within 1 second, confirm `FlowThreadSummary` card appears with trajectory strip.
3. Click "View thread" — confirm thread view with node events.
4. Click "← Back" — confirm main timeline is restored with the summary card intact.
5. If a pending review is active, confirm it appears inline in the thread view.
6. Confirm sidebar shows pending-only reviews (`showHistory={false}`) while in main view.

---

## Migration Plan

No data migration is required for Chunk 1. The `FlowThread` field on `ConversationBlock` is optional and defaults to `undefined`. Persisted v4 chat data with no `flowThread` field loads without modification (the `loadChat` reducer already handles missing optional fields gracefully).

When Chunk 2 ships, the `childSessionId` field will be populated with a real session ID that the Rust runtime recognizes. Existing chat history with a `childSessionId` that points to a no-longer-existing session will silently render an empty thread — acceptable per the non-goal of "historical thread replay from prior sessions."
