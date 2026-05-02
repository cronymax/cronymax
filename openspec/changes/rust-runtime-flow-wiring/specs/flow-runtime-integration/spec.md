## ADDED Requirements

### Requirement: StartRun routes through FlowRuntime when flow_id is present

When a `StartRun` control request includes a `flow_id`, the runtime SHALL delegate the run to `FlowRuntime::start_run()` rather than spawning a bare `ReactLoop`. `FlowRuntime` SHALL inject an `InvocationContext` (initial documents, port state, run configuration) into each agent invocation.

#### Scenario: Flow run with flow_id starts FlowRuntime

- **WHEN** `StartRun { space_id, payload: { flow_id, ... } }` is received
- **THEN** `FlowRuntime::start_run()` is called, the router schedules the entry agent, and the first `ReactLoop` receives an `InvocationContext` describing its input documents and expected output ports

#### Scenario: Agent-only run without flow_id bypasses FlowRuntime

- **WHEN** `StartRun { payload }` has no `flow_id`
- **THEN** a standalone `ReactLoop` is spawned with `HostCapabilityDispatcher` and no flow routing logic is involved

---

### Requirement: Document submission triggers FlowRuntime routing

When a `submit_document` tool call completes inside a `ReactLoop` that is owned by a `FlowRuntime` run, `FlowRuntime` SHALL receive the submitted document, update port state for the producing agent, and schedule downstream agents whose input ports are now satisfied.

#### Scenario: Downstream agent scheduled after document submitted

- **WHEN** agent A submits a document matching the type expected on an outgoing typed-port edge
- **THEN** `FlowRuntime` marks agent A's output port complete, evaluates the router, and schedules agent B whose input port is now satisfied

#### Scenario: @mention routing delivers to named agent

- **WHEN** a submitted document body contains `@AgentName` and no typed-port edge matches
- **THEN** `FlowRuntime` routes the document to the `@AgentName` agent's inbox

---

### Requirement: FlowRuntime enforces approval gates

If an agent node is configured with `requires_approval: true`, `FlowRuntime` SHALL pause the run after the agent submits its output document and emit a `review_requested` event. The run SHALL not schedule the downstream agent until a `ResolveReview` control request is received with `decision: Approved`.

#### Scenario: Run pauses at approval gate

- **WHEN** an agent with `requires_approval: true` submits its output document
- **THEN** the `FlowRuntime` emits a `review_requested` event and does not schedule the next agent

#### Scenario: Run resumes after approval

- **WHEN** `ResolveReview { run_id, review_id, decision: Approved, notes }` is received
- **THEN** `FlowRuntime` calls `on_approved_reschedule`, schedules the pending downstream agent, and the run continues

#### Scenario: Run aborted after rejection

- **WHEN** `ResolveReview { decision: RequestChanges }` is received
- **THEN** `FlowRuntime` re-enqueues the originating agent with reviewer notes injected into its `InvocationContext` and the agent re-runs

---

### Requirement: FlowRuntime enforces cycle limit

`FlowRuntime` SHALL track a per-run cycle counter. If the total number of agent invocations exceeds `max_cycles` (default 20), the run SHALL be halted with a `cycle_limit_exceeded` error event.

#### Scenario: Run halts at cycle limit

- **WHEN** a `FlowRuntime` run has scheduled `max_cycles` agent invocations
- **THEN** no further agents are scheduled, a `run_failed` event is emitted with reason `cycle_limit_exceeded`, and all active `ReactLoop` tasks are cancelled
