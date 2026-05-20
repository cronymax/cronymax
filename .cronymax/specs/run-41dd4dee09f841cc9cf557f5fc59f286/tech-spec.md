---
title: Flow Chat Orchestrator — Tech Spec
doc_type: tech-spec
---

# Flow Chat Orchestrator — Tech Spec

## Summary

Transform the `__chat__` session from a one-shot flow-seed into a persistent, multi-turn orchestration surface. This spec covers the frontend changes (React/TypeScript in `web/src/`) required to meet every acceptance criterion in the approved PRD. The backend runtime already exposes the necessary APIs (`flow.run.*`, `start.run`, session events, `get.session.pending.actions`). All work is confined to the Web tier (Tier 1).

Key deliverables:
1. **Flow Instances Bar** — live collapsible strip above the chat composer showing per-port `PortStatus` badges (promoted from sub-run granularity to flow-port granularity).
2. **Document Cards** — chat-thread-injected cards with Approve / Request Changes for every pending human review, replacing the current floating `FlowDocReviewPanel`.
3. **Reviewer Verdict Summaries** — inline chat turns showing automated reviewer outcomes.
4. **Cycle-Exhaustion Escalation Card** — amber-pulsing escalation card with three action buttons when `on_cycle_exhausted: escalate_to_human` fires.
5. **Run Completion Summary** — completion chat turn with auto-collapse of the run row in the Instances Bar.
6. **FlowEditor Demotion** — remove the Start Run button; add a `← Back to Chat` primary CTA; animate node fills on `PortStatus` changes.

---

## Approach

### 1 — Data Model Extensions (`store.ts`)

#### New block type: `DocReviewBlock`
```ts
export interface DocReviewBlock {
  kind: "doc-review";
  id: string;
  flow_run_id: string;
  node_id: string;
  port: string;
  doc_type: string;
  revision: number;
  preview: string;          // First ~2 sentences extracted from `content`
  content: string | null;   // Full markdown body
  status: "pending" | "approved" | "changes_requested";
  ts: number;
  comments: Comment[];
}
```

Add `DocReviewBlock` to the `Block` union and add corresponding actions:
- `createDocReviewBlock`
- `resolveDocReviewBlock` — sets `status` to `"approved"` or `"changes_requested"`

#### New block type: `ReviewerSummaryBlock`
```ts
export interface ReviewerSummaryBlock {
  kind: "reviewer-summary";
  id: string;
  flow_run_id: string;
  node_id: string;
  port: string;
  doc_type: string;
  revision: number;
  verdicts: Array<{
    reviewer: string;
    verdict: "approved" | "changes_requested";
    comments: string[];
  }>;
  cycleCount: number;
  maxCycles: number;
  ts: number;
  comments: Comment[];
}
```

#### New block type: `CycleEscalationBlock`
```ts
export interface CycleEscalationBlock {
  kind: "cycle-escalation";
  id: string;
  flow_run_id: string;
  run_id: string;
  openIssues: string[];
  status: "pending" | "resumed" | "shipped" | "halted";
  ts: number;
  comments: Comment[];
}
```

#### Extend `FlowInstancesBar` data model
Replace `SubRunEntry` with `PortEntry` sourced from the `flow.run.get_ports` endpoint (to be added to `runtime.ts`). Each port has:
```ts
interface PortEntry {
  node_id: string;
  port: string;
  status: "approved" | "in_review" | "pending" | "failed";
}
```
The bar's aggregate status becomes `"paused"` when a cycle-exhaustion event arrives.

#### State additions
```ts
interface State {
  // ... existing fields
  /** Maps flow_run_id → display name (e.g. "software-dev-cycle #3") */
  flowRunNames: Record<string, string>;
}
```

---

### 2 — Event Routing (`App.tsx`)

The chat panel already subscribes to `runtime.on("session:{id}", ...)`. Extend the handler to:

| Event kind | Action |
|---|---|
| `doc_review_ready` | Inject `DocReviewBlock` into the timeline |
| `reviewer_verdict` | Inject `ReviewerSummaryBlock` |
| `cycle_exhausted` | Inject `CycleEscalationBlock`; mark run as `paused` in `FlowInstancesBar` state |
| `flow_run_completed` | Inject `FlowNotificationBlock` (variant `"success"`) with completion summary; schedule auto-collapse after 5 s |
| `run_status` (awaiting_review) | Trigger `flowRun.getSessionPendingActions(sessionId)` refresh (existing path, kept for tool approvals) |

The C++ runtime already emits `run_status`; the new event kinds (`doc_review_ready`, `reviewer_verdict`, `cycle_exhausted`, `flow_run_completed`) will be delivered as `{ kind: "raw", data: { event: "...", ... } }` payloads over the existing session subscription channel — no new bridge surface is needed.

---

### 3 — New Components

#### `DocReviewCard.tsx`
- Renders a bordered card inside the chat timeline (not floating above composer).
- Shows: doc_type badge, revision number, 2-sentence preview, "Read full doc ↗" link (opens Workbench panel), **Approve** (`variant="primary"`) and **Request Changes** (`variant="outline"`) buttons.
- **Approve** flow: spinner → "✓ Approved" for 1.5 s → calls `dispatch(resolveDocReviewBlock, "approved")`.
- **Request Changes** flow: expands inline `FeedbackComposer` (see below).
- Calls `flowRun.approve / flowRun.requestChanges` on the bridge.

#### `FeedbackComposer.tsx`
- Embedded inside `DocReviewCard`.
- Props: `onSubmit(comments: FlowReviewComment[]) => void`, `onCancel() => void`.
- UI: free-text field, severity selector (`Error | Warning | Info` radio group), "Add another comment" button.
- Validates at least one non-empty comment before enabling Submit.

#### `ReviewerSummaryCard.tsx`
- Renders a compact unordered list of reviewer verdicts with severity-coloured left-border accents.
- Shows cycle count as `(Cycle N of M)` in `text-muted-foreground`.
- Read-only; no actions.

#### `CycleEscalationCard.tsx`
- Alert-style card with amber `ShieldAlert` icon.
- Lists open issues as a `<ul>`.
- Three action buttons:
  - **Resume with extra cycle** → calls `flowRun.resume(run_id)`.
  - **Ship as-is** → calls `flowRun.postInput(run_id, { decision: "ship" })`.
  - **Halt run** → calls `flowRun.cancel(run_id)`.
- After any action, sets `status` field and disables all buttons.

---

### 4 — `FlowInstancesBar.tsx` Revision

Replace sub-run-granularity rendering with port-granularity:

```
▼ software-dev-cycle #3            [⏳ in review]
  ├─ pm-design · prototype         ✓ approved
  ├─ pm-design · prd               ⏳ in-review
  ├─ rd-design · tech-spec         ○ pending
  └─ qa-testing · test-report      ○ pending
```

- Port status badges map to the four states: `✓ approved` (green), `⏳ in-review` (amber), `○ pending` (grey), `✕ failed` (red).
- Aggregate status `"paused"` renders as `⚠` amber-pulse icon.
- Auto-collapse completed runs after 5 s using a `setTimeout` cleared on unmount.
- Height spec: `h-9` collapsed row, `surface-1` background, `border-b-1` separator above message list.
- Port data fetched via a new `flowRun.getPorts(flow_run_id)` runtime call; refreshed on `flow.run.changed` events.

---

### 5 — `FlowEditor` Demotion

File: `web/src/components/FlowEditor/index.tsx`

Changes:
- **Remove** the Start Run button (`<Button onClick={handleStartRun}>`) and all associated `handleStartRun` logic.
- **Add** a `← Back to Chat` button as the sole primary CTA in the header. On click, it calls `shells.browser.panels.focus("chat")` (or the existing IPC method used to switch active panel).
- **Add** a `runState` prop (or read from a context) that maps node IDs to statuses. Node `className` is dynamically derived: `pending` → `bg-muted/20`, `active` → `bg-primary/15 border-primary/40` (existing), `done` → `bg-green-900/20 border-green-700/40`, `in_review` → `bg-amber-900/20 border-amber-600/40`.
- Edge SVG paths animate for ~1.5 s after a `handoff` event by toggling a CSS class with a keyframe animation.

---

### 6 — `runtime.ts` Additions

```ts
export const flowRun = {
  // ... existing methods
  async getPorts(flow_run_id: string): Promise<{ ports: Array<{ node_id: string; port: string; status: string }> }> {
    return (await runtimeSend("flow.run.get_ports", { flow_run_id })) as { ports: ... };
  },
};
```

The `get_ports` handler is to be added on the Rust side; if not yet available, the frontend falls back to deriving port status from existing `pending_reviews` and `run_status` events.

---

### 7 — Chat Composer Placement

`FlowDocReviewPanel` (the floating panel above the composer) is **removed**. Document reviews are surfaced exclusively as `DocReviewBlock` items in the chat timeline. The `FlowInstancesBar` is promoted to render between the message list and the composer, replacing the area currently occupied by `FlowDocReviewPanel`.

`App.tsx` layout (simplified):
```
<div className="flex flex-col h-full">
  <MessageList />                    {/* scrollable timeline */}
  <FlowInstancesBar sessionId={...} />   {/* new position */}
  <ComposerArea />
</div>
```

---

## Key Decisions

| Decision | Rationale |
|---|---|
| Inject review cards into the timeline instead of floating panel | Matches PRD acceptance criteria; keeps review context co-located with conversation history |
| Reuse existing `session:{id}` subscription channel for new events | No new bridge API surface; events are wrapped as `kind:"raw"` payloads already |
| Port-granularity rather than sub-run-granularity in Instances Bar | Aligns with PRD's "port and its PortStatus badge" requirement |
| Remove `FlowDocReviewPanel` rather than keeping both | Avoids duplicate review UI; simpler state management |
| `DocReviewBlock` stored in chat timeline (localStorage) | Consistent with all other block types; review history is preserved across soft reloads |
| Fall back to existing `pending_reviews` API if `get_ports` unavailable | Incrementally deployable without blocking on Rust-side changes |
| Auto-collapse completed runs after 5 s | Directly specified in PRD acceptance criteria |
| Inline `FeedbackComposer` with severity selector | PRD specifies structured comments; re-uses the existing `FlowReviewComment` type already in `runtime.ts` |

---

## Testing Strategy

### Unit Tests (`web/test/`)
- `DocReviewCard.test.tsx` — render with pending/approved/changes_requested status; button interactions call correct bridge methods.
- `FeedbackComposer.test.tsx` — validates submit is disabled with empty comments; multiple comments; severity toggle.
- `CycleEscalationCard.test.tsx` — three button paths (resume, ship, halt); buttons disable after action.
- `FlowInstancesBar.test.tsx` — expand/collapse; aggregate status derivation; paused state; auto-collapse timer.
- `store.reducer.test.ts` — new action cases for `createDocReviewBlock`, `resolveDocReviewBlock`, etc.

### Integration / E2E (manual, documented in submit-for-testing)
- Start a `software-dev-cycle` flow from chat; verify Flow Instances Bar appears.
- Expand a run row; verify port list with correct badges.
- Approve a document via Document Card; verify flow progresses.
- Request changes; verify feedback composer and structured comment submission.
- Simulate cycle exhaustion; verify escalation card with three options.
- Confirm run completion message and auto-collapse.
- Open FlowEditor; confirm no Start Run button; confirm `← Back to Chat` navigates back.

---

## Migration Plan

1. **Schema** — `DocReviewBlock`, `ReviewerSummaryBlock`, `CycleEscalationBlock` are new block `kind` values. The `loadChatData` migration path in `store.ts` already handles unknown block kinds gracefully (they fall through the `sanitizedBlocks.map` without modification). No explicit v5 migration key is needed unless we decide to purge stale `FlowDocReviewPanel` artifacts — those are held only in component memory, not localStorage.

2. **`FlowDocReviewPanel` removal** — once `DocReviewCard` is in the timeline, `FlowDocReviewPanel` import in `App.tsx` is deleted. No localStorage data is affected.

3. **FlowEditor** — the removal of the Start Run button is a UI-only change. Existing saved flow graphs and YAML definitions are untouched.

4. **Rollout order**:
   a. Add new block types to `store.ts` + reducer (no visible change).
   b. Implement `DocReviewCard` + `FeedbackComposer` and wire into `App.tsx` event handler.
   c. Revise `FlowInstancesBar` to port-granularity.
   d. Add `ReviewerSummaryCard` and `CycleEscalationCard`.
   e. Demote `FlowEditor` (remove Start Run button, add Back to Chat).
   f. Remove `FlowDocReviewPanel`.
