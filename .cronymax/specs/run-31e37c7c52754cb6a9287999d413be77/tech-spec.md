---
title: Flow Trajectory Diagram — Tech Spec
doc_type: tech-spec
---

# Flow Trajectory Diagram — Technical Specification

## Summary

The **Flow Trajectory Diagram** is a compact, horizontally-scrollable chip strip that renders above the chat prompt editor whenever a flow run is in progress or has recently completed in the current chat session. Each chip represents one agent node in the flow's topology, colour-coded and animated to reflect its live execution status. The component must handle real-time event streams, topological ordering from a `localStorage`-backed flow spec, cyclic (back-edge) graph paths, and multi-run navigation without full re-renders on every event.

The implementation lives in a single React component file at `web/src/panels/chat/FlowTrajectoryDiagram.tsx` and is consumed by the Chat `App.tsx`. A reference implementation already exists in the codebase; this spec documents the complete design so it can be verified, extended, or replaced.

---

## Approach

### Component Boundary

| Concern | Responsibility |
|---|---|
| **Topology** | Read canvas positions from `localStorage["flows"]` keyed by flow name; re-read on `StorageEvent` |
| **Run tracking** | Snapshot existing sub-runs from `shells.browser.activity.snapshot()` on mount; subscribe to live `run_status` events via `browser.on("event")` |
| **Status aggregation** | Per-node reduce across all matching sub-runs with priority ladder |
| **Rendering** | Collapsible Radix `<Collapsible>` with scrollable inner chip strip, SVG back-edge indicators, and run-selector pills |

### Data Flow

```
localStorage["flows"]        →  loadFlowTopology()  →  FlowTopology
                                                          orderedNodes (sorted by x)
                                                          edges, backEdges

shells.browser.activity
  .snapshot()  ─────────────────────────────────────┐
browser.on("event")                                  ↓
  kind === "run_status"  →  subRunsRef (Map)  →  flowRuns[]  →  React state
                             flowRunOrderRef              ↑
                                                  rebuildFlowRuns()
```

### State

| State | Type | Notes |
|---|---|---|
| `topology` | `FlowTopology \| null` | Updated from localStorage; null ⇒ fallback ordering |
| `flowRuns` | `FlowRunEntry[]` | Sorted by sequential insertion order |
| `activeRunIndex` | `number` | Auto-advances to newest; user-overridable |
| `collapsed` | `boolean` | Persisted in component state, not localStorage |

Two mutable refs avoid stale closure bugs in event handlers:
- `subRunsRef`: `Map<flowRunId, Map<runId, SubRun>>`
- `flowRunOrderRef`: `Map<flowRunId, seqNumber>`

### Topology Loading (`loadFlowTopology`)

1. Read `localStorage["flows"]` as `Record<string, StoredFlowSpec>`.
2. Look up `spec[flowName]`.
3. Sort `spec.nodes` by ascending `x` position.
4. For each edge, classify as a **back-edge** if `x(from) >= x(to)` (right-to-left arc).
5. Return `{ orderedNodes, edges, backEdges: Set<"fromId→toId"> }`.
6. Return `null` if key is absent, spec is missing, or `nodes` is empty.

Fallback ordering (no stored topology): unique `agent_id` values in first-observed temporal order from sub-run events.

### Status Aggregation

For a given `agentName` across all sub-runs of the active flow run, apply priority:

```
running > awaiting_review > failed > succeeded > pending
```

A node only reaches `succeeded` (green check) if **all** matching sub-runs have status `succeeded`. "cancelled" maps to `failed`.

### Event Subscription

```
browser.on("event", handler) → filter ev.tag === "event"
                             → filter pl.kind === "run_status"
                             → match session (pl.session_id === sessionId, or
                               flowRunId already tracked)
                             → upsert into subRunsRef
                             → rebuildFlowRuns()
```

The handler is registered once per `sessionId` change and cleaned up via the returned teardown function.

### Snapshot Recovery

On `sessionId` change:
1. Clear `subRunsRef` and `flowRunOrderRef`.
2. Call `shells.browser.activity.snapshot()`.
3. Walk `resp.runs`, skip entries where `session_id !== sessionId` or `flow_run_id` is null.
4. Build initial `subRunsRef` and `flowRunOrderRef` from the snapshot.
5. Call `rebuildFlowRuns()`.

### Rendering Details

**Header bar** (always visible, acts as `CollapsibleTrigger`):
- `<Activity />` icon (3×3 size)
- Flow name (truncated)
- Run-selector pills `#N` — only rendered when `flowRuns.length > 1`; active run highlighted with `bg-primary/20 text-primary`
- Short run ID (first 8 chars of UUID, monospace)
- `<ChevronUp />` / `<ChevronDown />` toggle

**Chip strip** (inside `CollapsibleContent`, `overflow-x-auto`):
- `min-w-max` inner container prevents wrapping
- Per-node `<NodeChip>` with status-coloured border/background
- `<ArrowSep>` between consecutive chips; `hasBackEdge` prop renders a dashed quadratic curve below the forward arrow when the pair is connected by a back-edge in either direction
- Cycle legend row (`⌒ back-edge (cycle)`) when `topology.backEdges.size > 0`

**Status indicators** inside each chip:
| Status | Indicator |
|---|---|
| `pending` | Muted grey dot (border) |
| `running` | Pulsing primary dot |
| `awaiting_review` | Pulsing amber dot |
| `succeeded` | Green polyline checkmark SVG |
| `failed` | Red × SVG |

**Null render**: Return `null` when `!selectedFlow || flowRuns.length === 0`. No layout space consumed.

### Placement in the Chat Panel

The component renders inside an `absolute bottom-full` stacking container above the prompt editor card:

```tsx
<div className="absolute bottom-full left-0 right-0 z-40 mb-1 flex flex-col gap-1">
  <FlowInstancesBar … />
  <FileChangesView … />
  {state.selectedFlow && (
    <FlowTrajectoryDiagram selectedFlow={state.selectedFlow} sessionId={state.activeChatId} />
  )}
  <FlowDocReviewPanel … />
  <ReviewsPanel … />
</div>
```

The container uses `z-40`; Radix portals use higher z-indices and are unaffected.

---

## Key Decisions

### 1. Mutable refs for event handler data
`subRunsRef` and `flowRunOrderRef` are React refs rather than state, so the `browser.on("event")` handler never goes stale and does not re-subscribe on every intermediate update. `setFlowRuns` is called only once per event after all mutations, keeping re-render cost proportional to discrete events, not token streams.

### 2. x-position ordering over edge-based topological sort
Canvas x-positions from the Flow Editor's stored JSON provide a stable, visually consistent ordering with zero graph-traversal cost. A Kahn's algorithm pass over YAML edges is deferred as a non-goal for this iteration.

### 3. Back-edge detection by coordinate comparison
An edge is a back-edge if `x(from) >= x(to)`. This is a heuristic that works correctly for any flow laid out left-to-right in the canvas editor and requires no DFS cycle detection.

### 4. Session-scoped run retention
Flow runs are retained in component state for the lifetime of the page session. No eviction policy. A `clear completed` action is out of scope.

### 5. Case-sensitive `agent_id` matching
`agentName` in the stored flow spec must exactly match the `agent_id` emitted by the runtime. Case normalisation is deferred.

### 6. Collapsible via Radix `<Collapsible>`
Using the existing shadcn component ensures keyboard-accessible, animated collapse/expand behaviour consistent with the rest of the UI without custom ARIA management.

---

## Testing Strategy

### Unit Tests (`web/test/`)

| Test | Coverage |
|---|---|
| `loadFlowTopology` — well-formed spec | Returns nodes sorted by `x`, edges, correct `backEdges` |
| `loadFlowTopology` — missing key | Returns `null` |
| `loadFlowTopology` — empty nodes array | Returns `null` |
| `loadFlowTopology` — back-edge detection | Edge where `x(from) >= x(to)` is in `backEdges` |
| `aggregateStatus` — priority ladder | `running` wins over `succeeded`; `failed` wins over `succeeded` |
| `aggregateStatus` — no matching sub-runs | Returns `"pending"` |
| `aggregateStatus` — all succeeded | Returns `"succeeded"` |
| `parseStatusKind` — all valid strings | Correct mapping including `"cancelled"` → `"failed"` |

### Component Tests (React Testing Library or Playwright component)

| Scenario | Assertions |
|---|---|
| No flow runs → null render | Component returns nothing; no DOM nodes rendered |
| Single flow run with nodes from localStorage | Chips rendered in x-sorted order; correct status colours |
| Fallback (no localStorage) | Chips in temporal order from sub-runs |
| Back-edge present | Dashed arc SVG visible; cycle legend shown |
| Collapse toggle | Strip hidden after chevron click; header still visible |
| Multi-run navigation | Clicking `#2` pill switches `activeRunIndex` |
| Auto-advance to newest run | `activeRunIndex` advances when a new `flow_run_id` arrives |

### Integration / E2E (Playwright against the CEF shell or dev server)

| Scenario | Assertions |
|---|---|
| Start a flow run from chat panel | Strip appears; first node shows pulsing dot |
| Run completes | All nodes green; strip persists |
| Edit flow in FlowEditor (same page session) | Strip node order updates without reload |
| Reload page while run is active | `snapshot()` recovery restores strip with correct statuses |
| No flow selected | Strip absent |

### Build Verification

```sh
cd web && bun run typecheck   # tsc --noEmit across all panels
cd web && bun run lint        # Biome format + lint
```

These must pass with zero new errors or warnings introduced by this component.

---

## Migration Plan

No database or persisted schema migrations are required.

**localStorage contract** (existing, unchanged):

```jsonc
localStorage["flows"] = {
  "<flowName>": {
    "nodes": [{ "id": 1, "name": "pm-design", "x": 120, "config": { "agent_name": "pm.agent" } }],
    "edges": [{ "from_id": 1, "to_id": 2 }]
  }
}
```

The component reads this structure read-only. The Flow Editor continues to be the sole writer.

**Bridge / runtime events** (existing, no change): `run_status` events on the `browser.on("event")` channel and `shells.browser.activity.snapshot()` are already used by `FlowInstancesBar`; this component reuses the same APIs without adding new C++ channels.

**Deprecation of `FlowInstancesBar`**: The `FlowTrajectoryDiagram` overlaps conceptually with `FlowInstancesBar`. The two coexist in this iteration; consolidation is deferred to a future cleanup pass.
