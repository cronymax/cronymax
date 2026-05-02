## ADDED Requirements

### Requirement: React Flow canvas

The system SHALL provide a visual editor at `web/orchestration/editor.html?flow=<flow_id>` rendered with `@xyflow/react` (React Flow v12). The canvas SHALL show one node per agent declared in `flow.yaml` and one edge per `FlowEdge`. The canvas SHALL support pan, zoom, drag-to-rearrange, and a fit-to-view control.

#### Scenario: Editor opens an existing Flow

- **WHEN** a user opens `editor.html?flow=simple-prd-to-spec`
- **THEN** the canvas reads `flow.yaml` via `flow.load`, renders one node per declared agent, draws every edge in `edges:`, and applies persisted positions from `flow.layout.json` if present, otherwise runs `dagre.layout()` and uses the result for that session

#### Scenario: First-render auto-layout is not persisted

- **WHEN** the editor auto-lays out a flow that has no `flow.layout.json`
- **THEN** the auto-layout result is applied for the session but is not written to disk; positions are persisted only when the user manually moves a node

---

### Requirement: Agent palette and side panel

The editor SHALL show an agent palette listing every agent in `.cronymax/agents/` for the active Space. Dragging a palette item onto the canvas SHALL add a node of that agent kind. Selecting a node or edge SHALL open a side panel that surfaces editable fields (agent: `kind`, `llm`, `system_prompt`; edge: `port`, `requires_human_approval`).

#### Scenario: Drag palette item onto canvas

- **WHEN** the user drags `architect` from the palette and drops it on the canvas
- **THEN** a new node appears at the drop position; the underlying `flow.yaml` is updated to include `architect` in `agents:` on next save; the node is selected and its side panel opens

#### Scenario: Side-panel edit writes through to YAML

- **WHEN** the user changes an edge's `requires_human_approval` checkbox in the side panel
- **THEN** the editor calls `flow.update` with the new YAML; on success the canvas re-renders from the new model; on failure the previous state is restored and a toast shows the error

---

### Requirement: Typed-port edge enforcement

The editor SHALL enforce typed-port compatibility at edge connection time. When the user drags an edge from agent A to agent B, the side panel's `port` dropdown SHALL be populated by the intersection of A's declared output doc-types and the doc-types declared in `.cronymax/doc_types/`. If the intersection is empty, the editor SHALL refuse the connection and surface a red toast naming the conflict.

#### Scenario: Compatible port allows connection

- **WHEN** A produces `prd` and `prd.doc-type.yaml` exists
- **THEN** dragging an edge from A to B exposes `prd` in the port dropdown and creates the edge once the user selects it

#### Scenario: No common port is rejected

- **WHEN** A produces only `tech-spec` and B has no declared input ports
- **THEN** the editor refuses the connection with a toast such as "no compatible doc-type between 'A' and 'B'" and no edge is added

---

### Requirement: Lossless YAML round-trip

The editor SHALL round-trip losslessly with `flow.yaml`: a canvas-load followed by a canvas-save with no user edit SHALL produce a byte-identical YAML file (modulo final trailing newline). Manual YAML edits made outside the editor SHALL re-render correctly when the editor is reopened. Layout-only changes SHALL write only `flow.layout.json` and SHALL NOT modify `flow.yaml`.

#### Scenario: Idempotent save

- **WHEN** the user opens the editor and immediately saves without editing
- **THEN** `flow.yaml`'s mtime is unchanged (no write occurred) or its bytes are identical to the previous version

#### Scenario: Hand-edit reflected on reopen

- **WHEN** the user closes the editor, hand-edits `flow.yaml` to add a new agent to `agents:`, and reopens the editor
- **THEN** the canvas shows a node for the new agent at an auto-laid-out position

---

### Requirement: Live-execution overlay

The editor SHALL support a `view-mode` toggle (`edit` / `run`). In `run` mode, the canvas SHALL subscribe to `events.subscribe` for the current Run id and SHALL: change a node's fill colour to reflect the latest `agent_status` for that agent; show an animated stroke on edges traversed by `handoff` events; show a small badge on the producing node when a `document_event` arrives. The same canvas geometry SHALL be used in both modes.

#### Scenario: Status colour updates on event

- **WHEN** an `agent_status` with `status: "thinking"` arrives in `run` mode for `agent_id: "product"`
- **THEN** the `product` node's fill changes within one animation frame (≤ 16 ms after the event broadcast)

#### Scenario: Run mode does not allow edits

- **WHEN** the editor is in `run` mode
- **THEN** node drag, edge connect, and side-panel writes are disabled; switching back to `edit` mode re-enables them
