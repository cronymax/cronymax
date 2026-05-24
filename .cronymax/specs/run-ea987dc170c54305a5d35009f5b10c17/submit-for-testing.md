---
title: Flow Thread Sessions — Submit for Testing
doc_type: submit-for-testing
---

# Flow Thread Sessions — Submit for Testing

## What Was Built

This delivers **Chunk 1** of the Flow Thread Sessions feature. Every flow run started from the Chat panel now creates an **inline conversation thread** — a live, Slack-style feed embedded directly in the chat timeline.

### User-facing behaviour
- When a flow run starts, a **`FlowThreadSummary` card** appears beneath the triggering message showing a trajectory strip (per-node status chips), a live event count badge, and a "running…" indicator.
- Clicking **"View thread →"** navigates into a dedicated thread view that replaces the chat timeline with:
  - A fixed header with a **← Back** button, the short `flowRunId` prefix, and the trajectory strip.
  - An **inline `FlowDocReviewPanel`** for any pending document reviews.
  - A **`FlowTrajectoryDiagram`** (appears once the `flowRunId` is backfilled from the first `run_status` event).
  - A **per-node event feed** grouped by agent, rendered in arrival order.
- Clicking **← Back** restores the main chat timeline with the `FlowThreadSummary` card still visible.
- The main-view sidebar shows **pending reviews only** (`showHistory={false}`); resolved review history is suppressed in the sidebar (visible only inside the thread view).
- The thread prompt area is **read-only** — thread messaging (Chunk 2 `@node-name` routing) is not yet wired.

### Protocol changes (Rust + frontend)
- `child_session_id` plumbing added end-to-end: the frontend allocates a UUID, passes it in `StartRun`, and the Rust runtime upserts a real child session linked to the parent via `parent_session_id` + `fork_point`.
- `RunStatus` events now carry `agent_id` and `flow_run_id` fields so the frontend can fan events into the correct thread.
- `AgentRunOptions` in `runtime.ts` accepts the new `child_session_id` field.

---

## Test Setup

### Build
```bash
cd web && bun run build
# Must exit 0 with no TypeScript errors
```

### Prerequisites
- A running Cronymax backend (Rust) built from the latest index — `cargo build` in `crates/cronymax`.
- At least one multi-node flow available (e.g. `software-dev-cycle` or `pm-rd-qa-cycle`) registered in the flows registry.
- Chat panel open in a browser pointed at the local dev server.

### Run the dev server
```bash
cd web && bun run dev
```

---

## In Scope

| # | Acceptance Criterion | How to Verify |
|---|---|---|
| AC1 | `FlowThreadSummary` card appears within ~1 s of flow start | Start a flow run; confirm the card is visible beneath the message before the first node completes |
| AC2 | Live event count badge increments as events arrive | Watch the badge number grow during a run |
| AC3 | "View thread →" button navigates into thread view | Click "View thread"; confirm the chat timeline is replaced by the thread layout |
| AC4 | Trajectory strip in card shows per-node status | Observe node chips with correct status dots (pending / running / done) |
| AC5 | `flowRunId` prefix appears in thread header once run is confirmed | Short UUID prefix visible in "← Back … [runId]" header line |
| AC6 | Pending document reviews appear inline in thread view | When a node produces a review document, open the thread and confirm the review card is present |
| AC7 | ← Back restores main timeline with summary card still visible | Click ← Back; confirm original block list is shown with `FlowThreadSummary` card intact |
| AC8 | Main-view sidebar shows pending reviews only (no resolved history) | In main view, approve a review; confirm the resolved entry disappears from the sidebar (still visible in thread view) |
| AC9 | Thread prompt area is disabled / read-only | In thread view, confirm the input area is absent or disabled with an appropriate placeholder |

---

## Known Limitations

- **Chunk 1 only — no thread messaging.** The `@node-name` re-invocation prompt (Chunk 2) is not implemented. The thread view has no active prompt editor.
- **Events are session-lived.** `FlowThread` data is not persisted to `localStorage`. If the app is refreshed mid-run, the thread event feed will be empty until new events arrive on the live session.
- **Child session isolation is partial.** The child session is upserted by the Rust runtime, but in certain edge cases early `pending` events from very fast sub-runs may arrive before the child session subscription is fully registered. The eager subscription inside `onRun` mitigates this but cannot guarantee zero-loss across all race conditions.
- **`FlowTrajectoryDiagram` requires `flowRunId`.** The diagram renders only after `setFlowThreadRunId` fires (first `run_status` event). There is a short window at start where the diagram is not yet visible.
- **No test coverage for new components yet.** Unit and integration tests described in the tech spec (component tests for `FlowThreadSummary`, reducer unit tests, E2E navigation tests) have not been written. Manual smoke testing is the primary verification path for this submission.
