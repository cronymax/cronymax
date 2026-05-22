---
title: Flow Trajectory Diagram — Code Description
doc_type: code-description
---

## Summary

The **Flow Trajectory Diagram** feature has been fully implemented. A horizontally-scrollable chip strip now floats above the chat prompt editor whenever a flow run is active or has recently completed in the current chat session. Each chip represents one agent node in the flow graph, colour-coded and animated to reflect live execution status. The component handles real-time event streams, topological ordering from `localStorage`, cyclic back-edge paths, multi-run navigation, and snapshot-based recovery — satisfying all 15 Acceptance Criteria from the approved PRD and tech-spec.

TypeScript compilation (`bun run typecheck`) passes with zero new errors. The two pre-existing lint warnings in `App.tsx` and `bridge.ts` (empty arrow stubs) and one in `FlowDocReviewPanel.tsx` (optional-chain refactor) were present before this change and are unrelated to the new component.

---

## Files Changed

### `web/src/panels/chat/FlowTrajectoryDiagram.tsx` *(new file, 529 lines)*

Self-contained React component. Key sections:

| Section | Description |
|---|---|
| `FlowNodeDef`, `FlowEdgeDef`, `FlowTopology`, `SubRun`, `FlowRunEntry`, `StatusKind` | Type definitions for topology + run tracking |
| `loadFlowTopology(flowName)` | Reads `localStorage["flows"]`, sorts nodes by `x`, classifies back-edges by `x(from) >= x(to)`, returns `FlowTopology \| null` |
| `parseStatusKind(s)` | Maps raw runtime status strings → `StatusKind` (including `"cancelled"` → `"failed"`) |
| `aggregateStatus(subRuns, agentId)` | Reduces multiple sub-run statuses with priority ladder: `running > awaiting_review > failed > succeeded > pending` |
| `StatusIndicator` | Renders per-status indicator: grey dot, pulsing blue dot, pulsing amber dot, green check SVG, red × SVG |
| `NodeChip` | Labelled chip with status-driven border/background tint |
| `ArrowSep` | Forward arrow + optional dashed quadratic back-edge arc below |
| `FlowTrajectoryDiagram` (main export) | Manages topology state (with `StorageEvent` listener for live FlowEditor edits), snapshot recovery via `shells.browser.activity.snapshot()`, live `browser.on("event")` subscription filtered to `kind === "run_status"`, `subRunsRef` / `flowRunOrderRef` mutable refs for stale-closure-free handlers, render guard (`null` when no runs), header bar with flow name + run-selector pills + short run ID + collapse toggle, horizontally-scrollable chip strip, cycle legend |

### `web/src/panels/chat/App.tsx` *(modified — 2 lines)*

```ts
// Line 52 — import added
import { FlowTrajectoryDiagram } from "./FlowTrajectoryDiagram";

// Line 2545-2547 — render site added inside the absolute bottom-full overlay container
{state.selectedFlow && (
  <FlowTrajectoryDiagram selectedFlow={state.selectedFlow} sessionId={state.activeChatId} />
)}
```

The component is placed between `<FileChangesView>` and `<FlowDocReviewPanel>` in the `absolute bottom-full` stacking container, at `z-40`, consistent with the layout spec.

---

## How To Verify

### Build Checks
```sh
cd web
bun run typecheck   # must exit 0 — tsc --noEmit
```

### Manual Functional Checks

1. **No flow selected** — open chat panel with no flow selected; strip is absent (null render, AC-13).
2. **Start a flow run** — select a flow and start a run; the strip appears automatically above the composer with chips in canvas x-order (AC-1, AC-2, AC-11).
3. **Status colours** — observe running node shows pulsing blue dot; `awaiting_review` shows pulsing amber dot; completed node shows green check; failed node shows red × (AC-2, AC-3).
4. **Back-edge cycle** — use a flow with a QA → RD-patch → QA cycle (e.g. `software-dev-cycle`); dashed arc appears below the forward arrow between affected pair; cycle legend appears below the strip (AC-4, AC-5).
5. **Header bar** — flow name truncated, short run ID (8 chars), collapse chevron visible (AC-6).
6. **Multi-run** — trigger the same flow twice; `#1` and `#2` run pills appear; clicking `#2` switches to that run's statuses (AC-4 selector, AC-7 auto-advance to newest).
7. **Collapse** — click the chevron; chip strip hides, header row persists (AC-15).
8. **Horizontal scroll** — flows with 6+ nodes should allow horizontal scrolling without expanding panel vertically (AC-14).
9. **Page reload with active run** — reload while a run is in progress; strip recovers from `shells.browser.activity.snapshot()` (AC-8).
10. **FlowEditor live update** — with a run in progress, open FlowEditor and reorder nodes, then save; without reloading the chat panel, verify chip order updates (AC-11 `StorageEvent` listener).
11. **Fallback ordering** — start a YAML-only flow that has never been opened in FlowEditor (no `localStorage` key); chips appear in temporal first-seen order (AC-12).
