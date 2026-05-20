---
title: Flow Trajectory Diagram — Prototype
doc_type: prototype
---

# Flow Trajectory Diagram — Prototype

## Overview

A real-time agent-trajectory strip that floats above the prompt editor in the chat-bound Flow panel. When a flow is selected and has active or recently-completed runs in the current session, the diagram renders as a compact, horizontally-scrollable chip strip showing each agent node in left-to-right order with live status indicators.

---

## Placement

The diagram lives inside the existing `relative` wrapper that anchors the picker popover and review panels:

```tsx
<div className="absolute bottom-full left-0 right-0 z-40 mb-1 flex flex-col gap-1">
  {state.selectedFlow && (
    <FlowTrajectoryDiagram
      selectedFlow={state.selectedFlow}
      sessionId={state.activeChatId}
    />
  )}
  <FlowDocReviewPanel sessionId={state.activeChatId} />
  <ReviewsPanel sessionId={state.activeChatId} />
</div>
```

**`bottom-full`** pins the bottom edge of the stack to the top edge of the editor card; **`z-40`** keeps it above the timeline but below the Radix portal layer (`z-50`). The diagram only mounts when `selectedFlow` is non-empty **and** at least one flow run is visible — so it takes no space during normal (non-flow) conversations.

---

## Architecture

### Topology loading (`loadFlowTopology`)

Nodes and edges are read from `localStorage["flows"]`, the same JSON object the FlowEditor writes on every save. Nodes are sorted by their canvas `x` position to produce a stable left-to-right ordering that matches the visual layout the user designed. Back-edges (edges where `from.x ≥ to.x`, i.e. right-to-left arcs representing cycles such as QA ↔ RD-patch) are flagged in a `Set<string>` keyed `"${from_id}→${to_id}"`.

A `storage` event listener keeps the topology live: if the user edits the flow graph while a run is in progress the strip immediately reorders to match.

### Run tracking

**On mount** the component calls `shells.browser.activity.snapshot()` and scans the `runs` array for entries whose `session_id` matches the current chat tab and whose `flow_run_id` is non-null. This recovers runs started before the component mounted (e.g. page reload mid-run).

**Live updates** arrive via `browser.on("event")` filtered to `kind === "run_status"`. Each event carries:

| field | usage |
|---|---|
| `flow_run_id` | groups sub-runs into a single trajectory |
| `run_id` | identifies the individual agent sub-run |
| `agent_id` | matches against node `agentName` for status derivation |
| `status` | `pending \| running \| awaiting_review \| succeeded \| failed \| cancelled` |
| `session_id` | optional — used to accept events even before the flow run is seen in the snapshot |

Sub-runs are stored in `Map<flowRunId, Map<runId, SubRun>>` held in a ref (never causing re-renders on mutation); `rebuildFlowRuns()` flattens them into React state only when a meaningful change occurs.

### Status aggregation

For each display node, all sub-runs whose `agentId` matches the node's `agentName` are collected and their statuses are reduced by priority:

```
running > awaiting_review > failed > succeeded > pending
```

This means a node lights up green only once **all** of its sub-runs have succeeded, and stays amber if any one is waiting on a review.

### Topology fallback

When `localStorage["flows"]` has no entry for the selected flow (YAML-only flows, or flows that have never been opened in the editor), the component falls back to deriving nodes from the unique `agent_id` values seen in temporal (first-observed) order. The chip strip is still fully functional; it just lacks the explicit edge arrows and back-edge cycle indicators.

---

## Visual Design

### Node chip

```
┌───────────────────────────┐
│  ●  pm-design             │  ← running (pulsing primary dot)
└───────────────────────────┘
```

Four colour variants driven by status:
- **pending** — muted border, grey dot
- **running** — primary border + background, pulsing blue dot
- **awaiting_review** — amber border + background, pulsing amber dot
- **succeeded** — green-tinted border + background, green check SVG
- **failed** — red-tinted border + background, red × SVG

### Arrow separator

A 20×12 SVG arrow (`M1 6 L16 6 M11 1 L17 6 L11 11`) sits between chips. When the edge between two adjacent nodes is a back-edge, a dashed quadratic curve appears below the arrow to signal a cycle without breaking the linear layout.

### Header bar

```
⚡ my-flow                    #1  #2   a3f9b0c2  ⌄
```

- Flow name (truncated) on the left
- Run-selector pills (`#1`, `#2`, …) when multiple flow runs exist for the session — clicking snaps the strip to that run
- Short flow-run ID (first 8 chars of UUID) in monospace
- Collapse/expand toggle (ChevronDown / ChevronRight)

The component auto-advances `activeRunIndex` to the newest run whenever a brand-new `flow_run_id` appears, so the user always sees the current run without manual action.

### Cycle legend

When the loaded topology contains at least one back-edge, a one-line legend appears below the strip:

```
⌒ (dashed)  back-edge (cycle)
```

---

## Component API

```tsx
interface Props {
  /** Name of the selected flow (matches keys in localStorage["flows"]). */
  selectedFlow: string;
  /** activeChatId from the chat store — used to filter runs. */
  sessionId: string | null | undefined;
}

export function FlowTrajectoryDiagram({ selectedFlow, sessionId }: Props): JSX.Element | null
```

Returns `null` when no flow runs are available, keeping the composer layout compact by default.

---

## File locations

| File | Role |
|---|---|
| `web/src/panels/chat/FlowTrajectoryDiagram.tsx` | New component (full implementation) |
| `web/src/panels/chat/App.tsx` | Integration — imports and renders inside `absolute bottom-full` zone |

---

## Known limitations / future work

1. **Agent-name matching is case-sensitive** — `agentName` from the stored flow spec must exactly match `agent_id` emitted by the runtime. A normalisation pass (lowercase, trim) would improve robustness.
2. **No per-node click-through** — clicking a chip could open the relevant trace entries or the document review panel filtered to that node. Deferred to a follow-up.
3. **Topology from YAML** — flows defined purely in `.cronymax/flows/*.yaml` without an FlowEditor save have no stored `x` positions. A topological sort (Kahn's algorithm) over the YAML edges would give a deterministic order without requiring the editor.
4. **Run retention** — completed runs persist in the strip for the lifetime of the page session. A "clear completed" button or a time-based eviction policy would keep the header tidy in long-running sessions.
