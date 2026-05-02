## ADDED Requirements

### Requirement: flow.run.start bridge channel

The system SHALL expose a typed `flow.run.start` IPC channel in `web/src/bridge_channels.ts`. The request payload SHALL include `space_id` (string), `flow_id` (string), and optional `initial_input` (string). The response SHALL be `{ run_id: string }`.

#### Scenario: Channel is type-safe

- **WHEN** TypeScript code calls `flow.run.start` via the bridge
- **THEN** the Zod schema validates the payload at runtime and the compiler enforces the type at build time

#### Scenario: Missing space_id is rejected

- **WHEN** `flow.run.start` is called without a `space_id`
- **THEN** the channel validation returns a Zod parse error before the message reaches the C++ layer

---

### Requirement: Start Run button in FlowEditor run-mode panel

The FlowEditor's run-mode panel SHALL display a "Start Run" button when no run is active for the current flow. Clicking it SHALL invoke `flow.run.start` with the current `space_id` and `flow_id` and transition the panel to the running state.

#### Scenario: Button visible when no run is active

- **WHEN** the FlowEditor is in run mode and no active run exists for the current flow
- **THEN** a "Start Run" button is visible and enabled

#### Scenario: Panel transitions to running state

- **WHEN** the user clicks "Start Run"
- **THEN** the button is replaced with a loading indicator, `flow.run.start` is called, and on success the panel shows the active run ID

---

### Requirement: Review approve/reject wired to Rust runtime

The `review.approve` and `review.request_changes` C++ bridge handlers SHALL forward the decision to `RuntimeProxy::SendControl(ResolveReview { run_id, review_id, decision, notes })` in addition to (or replacing) the legacy in-process `ReviewStore` call. The Rust runtime SHALL be authoritative for run lifecycle; the legacy path SHALL be retained only as a fallback when no matching `run_id` exists in the Rust layer.

#### Scenario: Approve reaches Rust runtime

- **WHEN** `review.approve` is called from the UI with a `run_id` that corresponds to an active Rust runtime run
- **THEN** `RuntimeProxy::SendControl(ResolveReview { decision: Approved })` is dispatched and the runtime unblocks the pending approval gate

#### Scenario: Fallback for legacy runs

- **WHEN** `review.approve` is called with a `run_id` that is not found in the Rust runtime
- **THEN** the legacy in-process `ReviewStore::approve()` is called and the run proceeds on the legacy path

---

### Requirement: flow.save bridge channel

The system SHALL expose a `flow.save` C++ bridge channel that accepts a serialised `FlowGraph` payload and writes it to `flow.yaml` for the current space. This unblocks visual flow editing.

#### Scenario: Save persists flow graph changes

- **WHEN** `flow.save` is called with a serialised `FlowGraph`
- **THEN** the C++ handler writes the updated `flow.yaml` to the space directory and returns `{ ok: true }`

#### Scenario: Save fails on invalid payload

- **WHEN** `flow.save` is called with a payload that fails YAML serialisation
- **THEN** the handler returns `{ ok: false, error: "<reason>" }` without overwriting the existing file
