---
title: Trajectory Tracking Diagram — Floating Agent-Flow Overlay (Prototype)
doc_type: prototype
---

# Trajectory Tracking Diagram — Floating Agent-Flow Overlay

**Feature:** Live agent-trajectory strip that floats above the prompt editor in the chat-bound Flow panel, showing every node in the selected flow, its current execution status, the document ports that connect nodes, and cycle indicators — all derived directly from the flow's YAML definition rather than requiring the FlowEditor to have saved a canvas spec.

---

## 1. Problem Statement

The `FlowTrajectoryDiagram` component already exists in the codebase (`web/src/panels/chat/FlowTrajectoryDiagram.tsx`) and is mounted above the prompt editor in `App.tsx`. However it has four critical gaps that make it effectively invisible in normal use:

| # | Gap | Impact |
|---|-----|--------|
| 1 | Topology is sourced only from `localStorage("flows")` (FlowEditor canvas JSON). YAML-only flows — the primary kind — produce no topology at all. | Diagram never renders for `software-dev-cycle`, `bug-fix-loop`, or `simple-prd-to-spec`. |
| 2 | Component returns `null` until `flowRuns.length > 0`, so the diagram is hidden before the first run even starts. | Users get no spatial overview when choosing a flow. |
| 3 | Document-port labels (e.g. `prd`, `tech-spec`, `submit-for-testing`) are not shown on edges. | No indication of what document is being handed off between agents. |
| 4 | AND-join gates, `max_cycles` constraints, and `requires_human_approval` flags are invisible. | Complex flows look identical to trivial linear ones. |

---

## 2. Design Goals

1. **Always visible** — as soon as a flow is selected in the header dropdown, the diagram renders with all nodes in "pending" state (no run required).
2. **YAML-first topology** — parse the active flow's `flow.yaml` via the runtime bridge; fall back to the FlowEditor canvas JSON; fall back to temporal agent order.
3. **Port labels on edges** — each arrow carries the document type it routes.
4. **Structural annotations** — `[👤]` human approval gate, `[🔁 ×N]` max-cycle badge, `[∧]` AND-join indicator.
5. **Live status overlay** — status dots/icons update in real time as sub-runs emit `run_status` events, exactly as today.
6. **Compact, collapsible** — single horizontal strip; collapses to a one-line header on click; never taller than ~80 px expanded.

---

## 3. Visual Spec (ASCII Wireframe)

### 3a. Expanded — `software-dev-cycle` (mid-run, RD designing)

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ ◉ software-dev-cycle                                           #1 · a3f8b2d1 ∨│
├──────────────────────────────────────────────────────────────────────────────┤
│  ✓ pm-design  ──prd──▶  ● rd-design  ──tech-spec──▶  ○ qa-testing  ──╮      │
│    [👤]                   [👤 critic]   [👤 critic    ∧ submit──▶──╯  │      │
│                                          qa-critic]                          │
│                           ╰──patch-note──╮  ○ rd-patch  ◀──bug-report[🔁×5]─╯│
└──────────────────────────────────────────────────────────────────────────────┘
```

### 3b. Collapsed

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ ◉ software-dev-cycle   ✓ pm-design  ● rd-design  ○ qa-testing  ○ rd-patch  ›│
└──────────────────────────────────────────────────────────────────────────────┘
```

### 3c. No-run (flow just selected, no runs yet)

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ ○ software-dev-cycle                                                       ∨  │
├──────────────────────────────────────────────────────────────────────────────┤
│  ○ pm-design  ──prd──▶  ○ rd-design  ──tech-spec──▶  ○ qa-testing           │
│                                                         ∧ submit-for-testing  │
│                           ╰──patch-note──╮  ○ rd-patch  ◀──bug-report[🔁×5]─╯│
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## 4. Status Dot Semantics

| Symbol | CSS class | Meaning |
|--------|-----------|---------|
| `○` hollow grey | `bg-muted-foreground/20 border border-muted-foreground/30` | Pending / not yet started |
| `●` pulsing primary | `bg-primary animate-pulse` | Currently running |
| `◎` pulsing amber | `bg-amber-400 animate-pulse` | Awaiting human review |
| `✓` green check | `text-green-400` SVG | Succeeded |
| `✗` red X | `text-red-400` SVG | Failed / cancelled |

The "flow status" indicator in the header follows the same priority ladder: `running > awaiting_review > failed > succeeded > pending`.

---

## 5. Topology Resolution (Priority Order)

```
resolveTopology(flowName):
  1. Try shells.browser.workspace.flow.get(flowName)
       → returns parsed YAML (flow.yaml)
       → extract nodes[], edges[], port labels, reviewers[], max_cycles
  2. Fallback: localStorage("flows")[flowName]
       → FlowEditor canvas JSON (current behaviour)
  3. Fallback: temporal order from sub-run agent_ids
       → no edge/port info, no structural annotations
```

### 5a. YAML → Topology Mapping

For the `nodes:`/`edges:` style flows (e.g. `software-dev-cycle`):

```
node.id          → NodeDef.id
node.owner       → NodeDef.agentName
node.outputs[].port  → EdgeDef.port
node.outputs[].routes_to → EdgeDef.to_id
node.outputs[].reviewers → NodeDef.approvalFlags
node.outputs[].max_cycles → EdgeDef.maxCycles
```

For the `agents:`/`edges:` style flows (e.g. `bug-fix-loop`):

```
agents[]         → one NodeDef per agent, ordered by forward-edge traversal
edges[].from     → EdgeDef.from_id (agent name)
edges[].to       → EdgeDef.to_id
edges[].port     → EdgeDef.port
edges[].requires_human_approval → EdgeDef.humanApproval = true
```

AND-join nodes: when a node appears as `to_id` in two or more distinct edges that originate from the **same** source node (same owner, different ports), it carries an `∧` annotation.

---

## 6. Component Interface (TypeScript)

```tsx
// ── Public Props ─────────────────────────────────────────────────────
interface FlowTrajectoryDiagramProps {
  /** Name of the selected flow (from store.selectedFlow). */
  selectedFlow: string;
  /** Session id for run-status event filtering. */
  sessionId: string | null | undefined;
}

// ── Extended internal types ───────────────────────────────────────────

interface NodeDef {
  id: string;            // flow node id (stable across runs)
  agentName: string;     // owner agent id
  displayName: string;   // node.id or node.name
  /** Reviewer list derived from any output port's reviewers array. */
  approvalFlags: ApprovalFlag[];
  /** True when this node is the target of 2+ edges (AND-join). */
  isAndJoin: boolean;
}

type ApprovalFlag = "human" | "critic" | "qa-critic" | string;

interface EdgeDef {
  fromId: string;
  toId: string;
  port: string;           // document type label
  isBackEdge: boolean;    // right→left arc (cycle)
  maxCycles?: number;     // from max_cycles field
  humanApproval?: boolean;
}

interface ResolvedTopology {
  nodes: NodeDef[];       // topological order (back-edges excluded from sort)
  edges: EdgeDef[];
  source: "yaml" | "editor" | "runtime";
}
```

---

## 7. Layout Algorithm

1. **Forward nodes** (those reachable by only forward edges) are laid out left-to-right in topological order.
2. **Back-edge targets** (nodes that are the `from` of a back-edge) emit a dashed loop below the strip — the loop connects from the *from-node* column back to the *to-node* column via a curved SVG arc.
3. **AND-join nodes** display an `∧` badge and two incoming arrows merge into one inbound arrow using a simple Y-merge glyph.
4. The entire strip is `overflow-x: auto` so deep flows scroll horizontally without wrapping.

---

## 8. Interaction Model

| Action | Result |
|--------|--------|
| Click collapse chevron | Toggle collapsed ↔ expanded |
| Click run index badge (`#1`, `#2` …) | Switch the "active run" whose status is overlaid |
| Hover node chip | Tooltip: agent name, run id, last status change timestamp |
| Hover edge arrow | Tooltip: port/document type, reviewers, max_cycles |
| Hover `[👤]` badge | Tooltip: "Requires human approval" |
| Hover `[🔁×N]` badge | Tooltip: "Max N cycles before escalation" |

The diagram is **read-only** from the chat panel — clicking nodes does not navigate or trigger anything.

---

## 9. File Changes Required

| File | Change |
|------|--------|
| `web/src/panels/chat/FlowTrajectoryDiagram.tsx` | Full rewrite of `loadFlowTopology()` to add YAML resolution path; extend `NodeDef`/`EdgeDef` with approval flags, max_cycles, AND-join; add `ApprovalBadge`, `EdgeLabel`, `AndJoinIndicator` sub-components; show diagram even when `flowRuns.length === 0` (all-pending state). |
| `web/src/shells/bridge.ts` *(or equivalent)* | Verify / expose `shells.browser.workspace.flow.get(name)` API that returns parsed YAML. If absent, read via `shells.browser.workspace.file.read(".cronymax/flows/<name>/flow.yaml")` and parse client-side with a small YAML parser (already available via `js-yaml` or inline). |
| `web/src/panels/chat/App.tsx` | No structural change needed — the `<FlowTrajectoryDiagram>` mount point is already correct. One adjustment: render the component unconditionally when `selectedFlow` is non-empty (currently wrapped in `state.selectedFlow && ...` which is already correct). |

---

## 10. Acceptance Criteria

- [ ] Selecting `software-dev-cycle` from the flow dropdown immediately renders the 4-node strip (`pm-design → rd-design → qa-testing / rd-patch`) with all nodes in "pending" state, before any run is triggered.
- [ ] Node chips display correct reviewer badges: `pm-design` shows `[👤]`, `rd-design` shows `[👤 critic]`, `qa-testing` shows `[👤 critic qa-critic]` on the `test-report` output.
- [ ] The `bug-report` back-edge from `qa-testing` to `rd-patch` renders as a dashed arc below the strip with `[🔁×5]` label.
- [ ] When a run is in progress, the currently-executing node's chip shows a pulsing primary dot; all preceding nodes that have completed show a green ✓.
- [ ] Selecting `bug-fix-loop` shows `product → architect → coder` with the `code-patch` back-edge (coder → product) shown as a dashed arc.
- [ ] Selecting `simple-prd-to-spec` shows `product → architect` with `[👤]` on both edges.
- [ ] Collapsing the diagram remembers state across the session (sessionStorage).
- [ ] The diagram never exceeds 80 px in expanded height on a 1280 px wide viewport.
- [ ] No horizontal layout overflow on a 480 px wide viewport — the strip scrolls.
- [ ] All existing `FlowTrajectoryDiagram` unit tests pass with the updated interface.

---

## 11. Non-Goals (Out of Scope for this Iteration)

- Clicking a node chip to open the agent's last run log.
- Dragging nodes to reorder them (that's the FlowEditor's domain).
- Animated token/document-packet transitions along edges.
- Showing per-port document previews inline.

---

## 12. Open Questions for RD

1. Does `shells.browser.workspace.flow.get(name)` already exist, or do we need to read the YAML file raw and parse client-side?  
2. Is `js-yaml` (or equivalent) already in the web bundle, or should we ship a tiny inline YAML-to-JSON parser for the subset we need?  
3. Should the diagram persist its collapsed state per-flow (so each flow remembers independently) or globally?
