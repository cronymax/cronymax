## ADDED Requirements

### Requirement: on_approved_reschedule edge trigger

The system SHALL support an `on_approved_reschedule: true` field on Flow edges. When a Document on that edge's port transitions to `APPROVED`, FlowRuntime SHALL re-invoke the producing Agent with a system-injected `InvocationContext` message identifying the approved document and the next pending output port (by declaration order in `flow.yaml`). This trigger SHALL fire exactly once per port per Run.

#### Scenario: RD re-invoked after tech-spec approval

- **WHEN** a `tech-spec` document reaches `APPROVED` on an edge declared with `on_approved_reschedule: true`
- **THEN** FlowRuntime schedules a new AgentRuntime invocation for the producing agent with `trigger.kind: document_approved` and `trigger.port: tech-spec` injected as the first system message

#### Scenario: Re-invoke fires exactly once

- **WHEN** `on_approved_reschedule` fires for a given port and the agent is re-invoked
- **THEN** a subsequent re-evaluation of the same port's APPROVED state does not schedule a duplicate invocation (idempotent via port completion map in `state.json`)

#### Scenario: No re-invoke when all ports complete

- **WHEN** `on_approved_reschedule` fires and the port completion map shows no remaining PENDING ports for this agent
- **THEN** the agent transitions to DONE status and no new invocation is scheduled

---

### Requirement: Per-edge reviewer_agents override

The system SHALL support a `reviewer_agents:` list on individual Flow edges. When present, this list SHALL override the Flow-level reviewer agent set for documents submitted on that port. The per-edge list SHALL reference agents by name; names not declared in the Flow SHALL produce a validation error at load time.

#### Scenario: Per-edge reviewer overrides global set

- **WHEN** an edge declares `reviewer_agents: [critic, qa-critic]` and the Flow also has `reviewer_enabled: true`
- **THEN** only `critic` and `qa-critic` are scheduled as reviewers for documents on that edge, not the global reviewer set

#### Scenario: Empty per-edge list disables reviewer agents for that edge

- **WHEN** an edge declares `reviewer_agents: []`
- **THEN** no LLM reviewer agents are run for documents on that edge; validators and human review still apply

---

### Requirement: max_cycles edge cap on peer cycles

The system SHALL support a `max_cycles: <int>` field on Flow edges. When declared, FlowRuntime SHALL count the number of times a document has been submitted on that edge within the current Run. When the count reaches `max_cycles`, the configured `on_cycle_exhausted` behaviour SHALL be triggered instead of routing the document.

#### Scenario: Bug-report edge respects max_cycles

- **WHEN** a `bug-report` document is submitted on an edge with `max_cycles: 5` for the fifth time in the current Run
- **THEN** the document is NOT routed to the downstream agent; the `on_cycle_exhausted` action is triggered instead

#### Scenario: on_cycle_exhausted: escalate_to_human pauses Run

- **WHEN** `max_cycles` is reached on an edge with `on_cycle_exhausted: escalate_to_human`
- **THEN** the Run transitions to `PAUSED`, a `flow.run.cycle_exhausted` event is emitted, and the user receives an inbox notification to intervene

#### Scenario: on_cycle_exhausted: halt fails the Run

- **WHEN** `max_cycles` is reached on an edge with `on_cycle_exhausted: halt`
- **THEN** the Run transitions to `FAILED` with reason `cycle_exhausted`

---

### Requirement: InvocationContext envelope

The system SHALL construct a typed InvocationContext for every AgentRuntime invocation. FlowRuntime SHALL inject it as a system-role message prepended to the agent's initial message history. The context SHALL include: `trigger.kind` (`flow_started` | `document_approved` | `changes_requested` | `mention_received`), `trigger.port` (when applicable), `trigger.doc` (path), `available_docs` (all APPROVED doc paths in the Run with type and revision), `pending_ports` (remaining output ports in declaration order), and a rendered human-readable `system_message` summarising the task.

#### Scenario: First invocation context

- **WHEN** an agent is invoked for the first time in a Run (trigger: `flow_started`)
- **THEN** the injected system message states the initial input documents and lists all declared output ports as pending

#### Scenario: Re-invoke context after approval

- **WHEN** an agent is re-invoked with `trigger.kind: document_approved`
- **THEN** the injected system message states which doc was approved, names the next task (`produce <next-port>`), and lists all currently approved docs in the Run as available context

#### Scenario: Context is first message in history

- **WHEN** the AgentRuntime constructs the initial message history for an invocation
- **THEN** the InvocationContext system message is the first entry, before any user-provided task message

---

### Requirement: Per-agent port completion tracking in state.json

The system SHALL extend `state.json` with an `agents` map tracking each declared agent's port completion state and invocation history within the Run. For each agent, the map SHALL record each output port's status (`PENDING` | `IN_REVIEW` | `APPROVED`) and an ordered list of invocations with id, trigger metadata, and status.

#### Scenario: Port marked APPROVED after document approval

- **WHEN** a document on port `tech-spec` transitions to `APPROVED`
- **THEN** `state.json` is updated to set `agents.rd.ports.tech-spec: APPROVED` before any re-invocation is scheduled

#### Scenario: Run state survives restart with port map intact

- **WHEN** the app restarts while a Run is `PAUSED` awaiting human review
- **THEN** `state.json` is rehydrated including the full port completion map, and FlowRuntime resumes from the correct state without re-running completed ports
