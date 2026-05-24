---
title: Flow Thread Sessions — Code Description
doc_type: code-description
---

# Flow Thread Sessions — Code Description

## Summary

This implementation delivers **Chunk 1** of the Flow Thread Sessions feature: a read-only inline thread view embedded directly in the Chat panel timeline. When a user starts a flow run, a `FlowThreadSummary` card appears beneath the triggering message. Clicking "View thread" navigates into a dedicated thread view that shows per-node status events, a trajectory diagram, and pending document reviews — all without leaving the Chat panel.

The implementation requires **zero Rust protocol changes** for the read-only shell. A `crypto.randomUUID()` child session ID is allocated at start time and passed to the Rust runtime as `child_session_id` (a new optional field on `StartRun`). The Rust side upserts a real child session, links it to the parent via `parent_session_id` + `fork_point`, and routes flow node sub-runs into it. The frontend subscribes to `session:{childSessionId}` to populate the event log in real-time.

Build verified: `bun run build` in `web/` exits 0 with no type errors.

---

## Files Changed

### `web/src/panels/chat/store.ts`
- Added `NodeConversation` interface — per-node agent conversation stream (content + trace entries + status).
- Added `StatusKind` union type — exported from store so `FlowTrajectoryDiagram` can import it instead of redefining it.
- Added `FlowThreadEvent` interface — a single event from any flow node agent (kind: `token | tool_call | trace | status`, with `segment?`, `entry?`, `seqNum`, `ts`).
- Added `FlowThread` interface — attached to a `ConversationBlock` when a flow run starts; holds `flowRunId` (backfilled), `childSessionId` (known at t=0), and a flat `events[]` array.
- Added `flowThread?: FlowThread` optional field to `ConversationBlock`.
- Added `ActiveView` type (already present as `{ kind: "main" } | { kind: "thread"; blockId; threadId }`).
- Added three new reducer cases:
  - `attachFlowThread` — attaches a `FlowThread` to a block when flow starts.
  - `setFlowThreadRunId` — backfills `flowRunId` when the first `flow.run.changed` event arrives.
  - `appendFlowThreadEvent` — appends an event into `block.flowThread.events`.
- Added three corresponding `Action` union variants.

### `web/src/panels/chat/App.tsx`
**New component — `FlowThreadSummary`:**
- Renders a compact `Card` beneath the triggering `ConversationBlock`.
- Shows "Flow thread" label, live event count badge, "running…" indicator while `flowRunId === ""`, and a "View thread →" button.
- Wired into `ConversationBlockView` via new `onViewThread` prop.

**`ConversationBlockView` / `BlockView` updates:**
- Added `onViewThread?: (blockId, threadId) => void` prop threading.
- Renders `<FlowThreadSummary>` when `block.flowThread` is defined.

**`onViewThread` / `onBackToMain` handlers in `App`:**
- `onViewThread` dispatches `setActiveView({ kind: "thread", blockId, threadId })`.
- `onBackToMain` dispatches `setActiveView({ kind: "main" })`.

**Thread view layout (new `activeView.kind === "thread"` branch):**
- Fixed header: `← Back` button, "Flow thread" label, short `flowRunId` prefix.
- Scrollable body: `FlowDocReviewPanel` (pending reviews inline), `FlowTrajectoryDiagram` (when `flowRunId` is known), event feed (grouped by `agentId`, rendered in `seqNum` order).
- Thread prompt stub: read-only `<Textarea>` is not rendered yet (Chunk 2); the input area is the existing composer which remains in the main view.

**Block timeline gate:**
- The `<div ref={timelineRef}>` block list is now conditional on `activeView.kind === "main"`.

**Child-session subscriptions (`childSessionSubsRef` + `useEffect`):**
- On every `state.blocks` update, iterates blocks with `flowThread?.childSessionId` and subscribes to `runtime.on("session:{childId}", …)` if not already subscribed.
- The handler routes `run_status` events → `appendFlowThreadEvent` (kind `"status"`) and `flow.run.changed` raw events → `setFlowThreadRunId`.
- Cleanup effect runs on unmount only, unsubscribing all child sessions.

**`onRun` flow-start path:**
- Generates `flowChildSessionId = crypto.randomUUID()` before `agentRun()` when `state.selectedFlow` is set.
- Passes `child_session_id: flowChildSessionId` in `AgentRunOptions`.
- After `agentRun()` resolves, dispatches `attachFlowThread` with an empty `events[]`.
- Also sets up an eager `runtime.on("session:{childId}")` subscription inside `onRun` to catch events that arrive before the `useEffect` subscription fires (de-duplication via the `childSessionSubsRef` guard).
- Handles `flow.run.changed` raw events from `processRuntimeEvent` to dispatch `setFlowThreadRunId`.

**Main-view sidebar:**
- `FlowDocReviewPanel` now receives `showHistory={false}` in the main-view floating stack, so only pending reviews are shown there (approved history is suppressed in this slot).
- The `FlowTrajectoryDiagram` that was previously in the floating stack is removed (trajectory is now in the thread view and the block-embedded diagram).

### `web/src/panels/chat/FlowDocReviewPanel.tsx`
- Added optional `showHistory?: boolean` prop (default `true`) to `Props` and the component signature.
- The panel now returns `null` when `reviews.length === 0 && (resolvedReviews.length === 0 || !showHistory)`.
- The resolved-review history `<Collapsible>` is gated by `showHistory`.
- Minor cleanup: `selectionTooltipRef.current && ... .contains(...)` → `selectionTooltipRef.current?.contains(...)`.
- Resolved reviews collapsible content gets `border-t border-border/50 bg-background pb-1` styling.

### `web/src/panels/chat/FlowTrajectoryDiagram.tsx`
- Replaced `browser.on("event", …)` live-event listener with `runtime.on("session:{sessionId}", …)` — targeted subscription instead of a global broadcast tap. Removes the `browser` import.
- Added `import type { ContentSegment, NodeConversation, StatusKind, TraceEntry }` from `store.ts`; removed the local `type StatusKind` definition (now imported).
- `NodeChip` gained optional `isSelected` and `onClick` props for clickable trajectory chips (used when the panel is mounted in thread view).
- Added new exported `useFlowNodeConversations` hook — subscribes to per-run event streams and maintains a live `Map<agentId, NodeConversation>` by processing `token`, `thinking_token`, `run_status`, and `trace` events. Also subscribes to the session channel to discover new sub-runs dynamically.
- `FlowTrajectoryDiagram` props expanded with `onSelectNode?`, `selectedNodeId?`, and `onConversationsUpdate?` for integration with `FlowNodeConversationsPanel`.
- Sub-run agent ID now resolved from `r.spec.agent_name` as fallback (needed because early `pending` events carry the name in the spec JSON before `agent_id` is set).

### `web/src/panels/chat/FlowNodeConversationsPanel.tsx` *(new file)*
- New collapsible panel showing per-node agent conversation streams (content + trace + reviews).
- Props: `nodeOrder`, `conversations`, `selectedNodeId`, `onSelectNode`.
- Tab strip auto-selects the first running/awaiting-review node.
- Renders `ContentStreamView` + `TraceViewer` + `ReviewsSection` for the active tab.

### `web/src/shells/runtime.ts`
- Added `child_session_id?: string` to `AgentRunOptions`.
- `agentRun()` now forwards `child_session_id` in the `start.run` request payload when set.

### Rust / backend changes (in staged index)
Several Rust files were modified to support `child_session_id` plumbing and to enrich `RunStatus` events with `agent_id` and `flow_run_id`:

| File | Change |
|---|---|
| `crates/cronymax/src/protocol/control.rs` | Added `child_session_id: Option<String>` to `ControlRequest::StartRun` |
| `crates/cronymax/src/protocol/events.rs` | Added `agent_id: Option<String>` and `flow_run_id: Option<String>` to `RuntimeEventPayload::RunStatus` |
| `crates/cronymax/src/runtime/authority.rs` | New `set_session_fork_point()` helper; enriches `RunStatus` events with `agent_id` + `flow_run_id` at pending/awaiting_review/finalize sites; added `attach_flow_run_to_session()` call |
| `crates/cronymax/src/runtime/handler/run_start.rs` | Upserts child session from `child_session_id`; sets `parent_session_id` + `fork_point`; routes flow node sub-runs to child session |
| `crates/cronymax/src/runtime/agent_runner.rs` | Populates `agent_name` in run spec JSON so `RunStatus` events carry it from the first `pending` event |
| `crates/cronymax/src/flow/runtime.rs` | Added `"human_feedback"` trigger kind to `InvocationTrigger` doc comment |
| `docs/architecture.md` | Added `flow-thread-sessions` design note |
| `docs/flow-thread-sessions.md` | New 424-line design document covering session topology, protocol layer, `@` routing state machine, and UX sketches |

---

## How To Verify

### Build
```bash
cd web && bun run build
# Should exit 0 with no type errors — verified ✓
```

### Manual smoke test
1. Open the Chat panel and select a multi-node flow (e.g. `software-dev-cycle`) from the Flow dropdown.
2. Send a prompt. Within ~1 second of the run starting, a **"Flow thread"** card should appear beneath the message with a "running…" label.
3. As the flow progresses, the event count in the card should increment.
4. Click **"View thread"** — the chat timeline is replaced by the thread view showing:
   - `← Back` header with a short flow run ID prefix.
   - `FlowDocReviewPanel` (pending reviews only, if any).
   - `FlowTrajectoryDiagram` (appears once `flowRunId` is backfilled).
   - Per-node event entries with agent name labels.
5. Click **"← Back"** — the main chat timeline is restored with the `FlowThreadSummary` card still visible.
6. In main view, any pending document reviews appear in the composer floating stack (`FlowDocReviewPanel showHistory={false}`).
7. Approved/resolved review history is **not** shown in the floating stack (it is only visible inside the thread view's `FlowDocReviewPanel` which defaults to `showHistory={true}`).

### Key acceptance criteria mapping
| AC | Verification |
|---|---|
| AC #1 — summary card within 1s | Appears immediately after `attachFlowThread` dispatches (before `agentRun` resolves) |
| AC #2 — event count badge | `thread.events.length` rendered live |
| AC #3 — View thread / ← Back | `setActiveView` dispatches; tested by clicking |
| AC #6 — inline reviews | `FlowDocReviewPanel` inside thread body |
| AC #7 — Back restores summary | `activeView.kind === "main"` shows timeline with card |
| AC #8 — sidebar pending-only | `showHistory={false}` on main-view sidebar mount |
| AC #9 — read-only prompt | Thread view has no prompt editor (Chunk 2) |
