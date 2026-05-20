---
title: Flow Chat Orchestrator — Submit for Testing
doc_type: submit-for-testing
---

# Flow Chat Orchestrator — Submit for Testing

## What Was Built

The **Flow Chat Orchestrator** feature transforms the `__chat__` session from a one-shot flow-seed into a persistent, multi-turn orchestration surface. The following changes were implemented:

### Flow Trajectory Diagram (`FlowTrajectoryDiagram.tsx`)
A new horizontally-scrollable chip strip that floats above the chat prompt editor whenever a flow run is active or recently completed. Each chip represents one agent node in the flow graph, colour-coded and animated to reflect live execution status. It supports:
- Real-time event streaming via `browser.on("event")` filtered to `kind === "run_status"`
- Topological ordering sourced from `localStorage` (FlowEditor canvas x-coordinates)
- Back-edge cycle detection with dashed quadratic arc visual and cycle legend
- Multi-run navigation with selectable run pills (auto-advances to newest run)
- Snapshot-based recovery from `shells.browser.activity.snapshot()` on page reload
- Live topology updates via `StorageEvent` listener when FlowEditor saves changes
- Collapse toggle (header persists, chip strip hides)

### Integration in `App.tsx`
- `FlowTrajectoryDiagram` imported and placed inside the `absolute bottom-full` stacking container at `z-40`, between `<FileChangesView>` and `<FlowDocReviewPanel>`
- Only rendered when `state.selectedFlow` is set

---

## Test Setup

### Prerequisites
- macOS desktop shell (CEF) with app running in development mode
- A flow definition present in `localStorage` (e.g. `software-dev-cycle`) — open the FlowEditor at least once so canvas x-positions are persisted
- A running backend runtime capable of emitting `run_status` events

### Build Verification
```sh
cd web
bun run typecheck   # must exit 0 — zero new type errors
```

### Manual Environment
1. Launch the app and open the Chat panel.
2. Select a flow (e.g. `software-dev-cycle`) from the flow selector.
3. Start a flow run to trigger active event streaming.

---

## In Scope

The following acceptance criteria (from the approved PRD and tech-spec) are in scope for this QA pass:

### Flow Trajectory Diagram (Chip Strip)
| # | Test Case | Expected Result |
|---|---|---|
| AC-1 | No flow selected / no runs | Strip is completely absent (null render) |
| AC-2 | Start a flow run with a flow selected | Chip strip appears above composer; chips ordered by canvas x-position |
| AC-3 | Node status colours | `pending` → grey dot; `running` → pulsing blue dot; `awaiting_review` → pulsing amber dot; `succeeded` → green check SVG; `failed` / `cancelled` → red × SVG |
| AC-4 | Back-edge cycle (QA → RD-patch → QA) | Dashed quadratic arc renders below the forward arrow; cycle legend visible below strip |
| AC-5 | Cycle legend visibility | Legend appears if and only if at least one back-edge exists in the topology |
| AC-6 | Header bar | Flow name (truncated), short run ID (8 chars), collapse chevron all visible |
| AC-7 | Multi-run: trigger same flow twice | `#1` and `#2` pills appear in header; clicking `#2` switches statuses; newest run selected automatically |
| AC-8 | Collapse toggle | Clicking chevron hides chip strip; header row persists; clicking again restores |
| AC-9 | Horizontal scroll | Flows with 6+ nodes scroll horizontally without expanding the panel vertically |
| AC-10 | Page reload during active run | Strip recovers from `shells.browser.activity.snapshot()` — nodes show last-known statuses |
| AC-11 | FlowEditor live update | With a run active, re-order nodes in FlowEditor and save; chip order updates in chat panel without page reload (via `StorageEvent`) |
| AC-12 | Fallback ordering (no localStorage) | YAML-only flows that have never been opened in FlowEditor show chips in temporal first-seen order |

### Build Integrity
| # | Test Case | Expected Result |
|---|---|---|
| B-1 | `bun run typecheck` | Exits 0 — zero new TypeScript errors introduced |
| B-2 | No regressions in existing panels | `App.tsx`, `FlowDocReviewPanel.tsx`, `ReviewsPanel.tsx`, `useActivityFeed.ts`, `bridge.ts`, `runtime.ts` continue to compile and function as before |

---

## Known Limitations

1. **Full Flow Chat Orchestrator spec is partially in flight** — This QA pass covers the *Flow Trajectory Diagram* component (Phase 1). The following items from the approved tech-spec are **not yet implemented** and are **out of scope** for this round:
   - `DocReviewCard.tsx` / `FeedbackComposer.tsx` — Document Cards injected into chat timeline
   - `ReviewerSummaryCard.tsx` — Automated reviewer verdict summaries
   - `CycleEscalationCard.tsx` — Cycle-exhaustion escalation card with Resume / Ship / Halt actions
   - `FlowInstancesBar.tsx` revision to port-granularity
   - `FlowEditor` demotion (removal of Start Run button, addition of `← Back to Chat`)
   - Removal of `FlowDocReviewPanel`

2. **Backend events `doc_review_ready`, `reviewer_verdict`, `cycle_exhausted`, `flow_run_completed`** — These new event kinds are not yet consumed; only `run_status` is handled in this increment.

3. **`flowRun.getPorts`** — The `flow.run.get_ports` runtime call and corresponding Rust handler are not yet available; the trajectory diagram uses `run_status` sub-run data only.

4. **Mobile / web targets** — Feature is scoped exclusively to the macOS CEF desktop shell. Do not test on other targets.

5. **Pre-existing lint warnings** — Two empty-arrow-stub warnings in `App.tsx` / `bridge.ts` and one optional-chain refactor warning in `FlowDocReviewPanel.tsx` pre-date this change and are unrelated to the new component.
