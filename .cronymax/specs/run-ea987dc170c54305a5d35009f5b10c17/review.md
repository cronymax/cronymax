---
title: Review: test-cases — REJECTED (wrong feature scope)
doc_type: review
---

# Review Verdict: **REJECT**

## Summary

The submitted `test-cases.md` document does **not** cover the feature described in the approved tech-spec and code-description (*Flow Thread Sessions*). It instead describes test coverage for three unrelated subsystems:

1. **C++ `LayoutMigrator`** — v0–v4 profile/flat layout migration and persistence.
2. **Rust Cronymax runtime** — authority, agent sessions, compaction, token estimates.
3. **FFI / GIPS protocol** — C ABI round-trips, Hello/Welcome, Ping/Pong, cancel-safety.

## Required Corrections

The test cases must be re-written to cover the **Flow Thread Sessions** feature as specified. At minimum they must include:

### Component / Unit Tests
- `FlowThreadSummary` renders correctly given a `FlowThread` with N events (event-count badge, trajectory strip chips, "View thread" button).
- `FlowThreadSummary` renders nothing when `block.flowThread` is `undefined`.
- Thread view event grouping: given events `[A, A, B, A]` confirm three agent bubbles are rendered.
- `appendFlowThreadEvent` reducer: events are appended in `seqNum` order; state is otherwise unchanged.
- `setFlowThreadRunId` reducer: `flowRunId` is backfilled on the correct block; other blocks are untouched.
- `attachFlowThread` reducer: `FlowThread` is attached with empty `events[]`; `childSessionId` is preserved.

### Integration / AC Tests
- **AC #1** — `FlowThreadSummary` card appears within 1 s of the first `run_status` event after `attachFlowThread` dispatches.
- **AC #2** — Event count badge reflects `block.flowThread.events.length` live.
- **AC #3 / #7** — "View thread" button dispatches `setActiveView({ kind: "thread", … })`; "← Back" restores `{ kind: "main" }` and summary card is still visible.
- **AC #6** — `FlowDocReviewPanel` is rendered inline inside the thread view body.
- **AC #8** — Main-view sidebar mounts `FlowDocReviewPanel` with `showHistory={false}`; resolved history is not shown.
- **AC #9** — Thread view has no active prompt editor (textarea is absent or disabled).

### Edge / Regression Cases
- Block with `flowThread.flowRunId === ""` shows "running…" indicator until `setFlowThreadRunId` fires.
- Child-session subscription is de-duplicated (subscribing twice to the same `childSessionId` emits no duplicate events).
- `FlowDocReviewPanel` with `showHistory={false}` and zero pending reviews returns `null` (no empty panel rendered).

## References
- Approved tech-spec §Testing Strategy
- Approved code-description §Files Changed, §How To Verify — AC table
