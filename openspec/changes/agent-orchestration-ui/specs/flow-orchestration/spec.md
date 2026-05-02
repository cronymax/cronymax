## MODIFIED Requirements

### Requirement: Run trace event stream

The system SHALL emit typed events conforming to the `AppEvent` schema (see `agent-event-bus`) during Run execution and persist them through `EventBus::Append`. `EventBus::Append` SHALL — within a single mutex — write the event to (a) the per-Space SQLite `events` table and (b) `runs/<run-id>/trace.jsonl` (one JSON object per line). Event kinds SHALL be drawn exclusively from the `AppEvent` closed enum (`text`, `agent_status`, `document_event`, `review_event`, `handoff`, `error`, `system`); kind-specific subkinds (e.g. `system.subkind = "run_started"`, `agent_status.status = "thinking"`) SHALL replace the previous flat string types.

#### Scenario: Event written for document submission

- **WHEN** an Agent submits a Document
- **THEN** a `document_event` with `payload.revision`, `payload.doc_path`, `payload.producer`, `payload.sha256_prefix` is appended; both the SQLite row and the `trace.jsonl` line are present after the call returns

#### Scenario: Trace replay on UI subscription

- **WHEN** a UI client subscribes to a Run's events mid-execution via `events.subscribe {run_id}`
- **THEN** the system streams every persisted event for that `run_id` in event-id order followed by live events, with no duplication or gaps

#### Scenario: Crash-consistency between table and JSONL

- **WHEN** the process is killed between the SQL commit and the JSONL fsync
- **THEN** on next start the SQLite row is the source of truth; a recovery step rebuilds `trace.jsonl` from the `events` table for any run whose JSONL is shorter than its row count

---

### Requirement: Bridge surface for Flow execution

The system SHALL expose the following bridge channels for the renderer: `flow.list`, `flow.load`, `flow.update`, `flow.run.start`, `flow.run.status`, `flow.run.cancel`. All payloads SHALL be JSON. Server-push events for Flow execution SHALL be delivered through the `event` broadcast defined in `agent-event-bus`. The legacy `event.subscribe` channel (run-scoped only) SHALL remain as an alias for `events.subscribe` with `run_id` set, with a deprecation warning logged on first use per process; it SHALL be removed in the `agent-skills-marketplace` change.

#### Scenario: List Flows in active Space

- **WHEN** the renderer sends `flow.list` with the active Space id
- **THEN** the system responds with a JSON array of Flow names, statuses of any active Runs, and last-modified timestamps

#### Scenario: flow.update writes through to YAML

- **WHEN** the renderer sends `flow.update` with a new YAML body produced by the visual editor
- **THEN** the system validates the YAML against the Flow schema, writes it atomically (.tmp + rename) to `flow.yaml`, and broadcasts a `system` event with `subkind: "flow_updated"`

#### Scenario: Legacy event.subscribe still works

- **WHEN** an old renderer calls `event.subscribe {run_id}`
- **THEN** the engine treats the call as `events.subscribe {run_id}`; a deprecation warning is logged exactly once per process

## ADDED Requirements

### Requirement: Flow layout sidecar

The system SHALL persist visual-editor node positions to `flow.layout.json` next to `flow.yaml`. The sidecar SHALL be optional — its absence SHALL trigger first-render auto-layout via dagre. The sidecar SHALL contain only cosmetic data (per-node `{x, y}` and viewport `{x, y, zoom}`); it SHALL NOT carry any semantic Flow state. The default `.gitignore` SHALL list `flow.layout.json` so cosmetic positions do not pollute version control unless the user explicitly opts in.

#### Scenario: Sidecar absence is non-fatal

- **WHEN** the visual editor opens a Flow whose directory contains no `flow.layout.json`
- **THEN** the editor renders correctly with auto-laid-out positions and no error toast

#### Scenario: Sidecar saved only on manual move

- **WHEN** the user drags a node to a new position
- **THEN** `flow.layout.json` is written atomically; opening the editor again restores that position

#### Scenario: Hand-edit of YAML does not break sidecar

- **WHEN** a user adds a new agent to `flow.yaml` by hand and reopens the editor
- **THEN** existing nodes keep their persisted positions; the new agent's node is auto-laid-out; `flow.layout.json` is updated to include the new node only after the user manually moves it
