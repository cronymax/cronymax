---
title: Flow Trajectory Diagram — PRD
doc_type: prd
---

# Flow Trajectory Diagram — Product Requirements Document

## Goal

Give users real-time visibility into how an active (or recently completed) multi-agent flow is progressing without leaving the chat panel. A compact, horizontally-scrollable chip strip should appear automatically above the prompt editor whenever a flow run is in progress or has finished in the current chat session. Each chip represents one agent node in the flow graph, coloured and animated to reflect its live execution status. Users should be able to distinguish pending, running, waiting-for-review, succeeded, and failed states at a glance, navigate between multiple runs of the same flow, and understand cyclic (back-edge) paths in the graph.

---

## Users

| Persona | Context |
|---|---|
| **Flow author / power user** | Designs multi-agent flows in the Flow Editor and monitors them from the chat panel while a run is underway. |
| **Collaborator / reviewer** | Has a flow run that paused at an `awaiting_review` gate and needs to know which agent is waiting for their input. |
| **Developer / debugger** | Investigates a failed run and needs to pinpoint which agent node errored without opening a separate trace view. |

---

## User Stories

1. **Real-time status** — As a flow author, I want to see each agent node's live status (pending / running / awaiting review / succeeded / failed) as chips in the chat panel, so I don't have to switch to a separate monitoring view during a run.

2. **Automatic appearance** — As a user, I want the trajectory strip to appear automatically when a flow run starts and disappear when there are no relevant runs, so it never clutters the chat UI during ordinary (non-flow) conversations.

3. **Back-edge / cycle indication** — As a flow author, I want cycles in the graph (e.g. QA → RD-patch → QA) to be visually distinguished from linear edges, so I understand re-entrant loops without reading the YAML.

4. **Multi-run navigation** — As a user who triggered the same flow more than once in a session, I want to click run-selector pills (`#1`, `#2`, …) to switch between trajectories, so I can compare outcomes.

5. **Collapse / expand** — As a user, I want to collapse the trajectory strip to a single header bar when I don't need it, so it doesn't obscure the composer on small screens.

6. **Topology live-update** — As a flow author who edits the flow graph while a run is in progress, I want the chip order to update immediately to reflect the saved topology, so the strip stays consistent with the editor.

7. **Fallback ordering** — As a user running a YAML-only flow that has never been opened in the Flow Editor, I want chips to appear in temporal (first-observed) order, so the strip still works even without stored canvas positions.

---

## Acceptance Criteria

- [ ] **AC-1 Placement:** The `FlowTrajectoryDiagram` component renders inside an `absolute bottom-full` container above the prompt-editor card, below the Radix portal layer (`z-40`), only when `selectedFlow` is non-empty and at least one matching flow run exists.
- [ ] **AC-2 Node chips:** Each agent node renders as a labelled chip with a coloured status indicator: grey dot (pending), pulsing blue dot (running), pulsing amber dot (awaiting_review), green check (succeeded), red × (failed).
- [ ] **AC-3 Border & background:** Chip border and background tint match the status colour variant (primary / amber / green / red / muted).
- [ ] **AC-4 Arrow separators:** A right-pointing SVG arrow appears between adjacent chips. When the edge is a back-edge (right-to-left in canvas `x` coordinates), a dashed quadratic curve is rendered below the arrow.
- [ ] **AC-5 Cycle legend:** When the topology contains at least one back-edge, a legend line — `⌒ (dashed)  back-edge (cycle)` — is shown below the chip strip.
- [ ] **AC-6 Header bar:** The header displays the flow name (truncated), run-selector pills for each flow run in the session, the short run ID (first 8 chars of UUID), and a collapse/expand chevron toggle.
- [ ] **AC-7 Auto-advance to newest run:** When a new `flow_run_id` appears, `activeRunIndex` automatically advances to it so users always see the current run.
- [ ] **AC-8 Run recovery on mount:** On component mount, `shells.browser.activity.snapshot()` is called to recover runs started before the component was mounted (e.g. after page reload).
- [ ] **AC-9 Live event subscription:** The component subscribes to `browser.on("event")` filtered to `kind === "run_status"` and updates chip statuses without a full re-render for every intermediate event.
- [ ] **AC-10 Status aggregation priority:** When a node has multiple sub-runs, statuses are reduced by priority `running > awaiting_review > failed > succeeded > pending`; a node turns green only when all its sub-runs have succeeded.
- [ ] **AC-11 Topology from localStorage:** Nodes are ordered by canvas `x` position read from `localStorage["flows"]`. A `storage` event listener keeps the order live when the flow is edited during a run.
- [ ] **AC-12 Fallback ordering:** When no stored topology exists for the selected flow, nodes are ordered by first-observed `agent_id` from runtime events.
- [ ] **AC-13 Null render:** The component returns `null` (renders nothing, takes no layout space) when there are no flow runs for the current session.
- [ ] **AC-14 Horizontal scroll:** The chip strip is horizontally scrollable when the number of nodes exceeds the panel width, without expanding the panel vertically.
- [ ] **AC-15 Collapse:** Clicking the chevron toggle hides the chip strip and arrow layer, leaving only the header bar visible.

---

## Non-Goals

- **Click-through to trace view** — clicking a chip will not navigate to per-agent trace entries or filter the document review panel in this iteration.
- **YAML-based topological sort** — flows that lack canvas `x` positions will use temporal ordering; computing Kahn's algorithm over YAML edges is deferred.
- **Run retention / eviction policy** — completed runs are retained for the page session; a "clear completed" button or time-based eviction is out of scope.
- **Case-insensitive agent-name matching** — `agentName` in the stored flow spec must exactly match the `agent_id` emitted by the runtime; normalisation is deferred.
- **Mobile / non-macOS support** — the feature targets the cronymax macOS CEF shell; responsive layout for other form factors is not required now.
