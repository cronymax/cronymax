## ADDED Requirements

### Requirement: Rust runtime owns run and agent authority
The system SHALL run a standalone Rust runtime process that is the sole authority for run lifecycle and agent lifecycle across all Spaces in an app session. Run creation, cancellation, pause/resume, agent state mutation, memory namespace state, and permission state SHALL be created and mutated only by the Rust runtime.

#### Scenario: Start a run through the runtime authority
- **WHEN** the user starts a Flow run from the desktop UI
- **THEN** the host forwards the request to the Rust runtime and the Rust runtime creates the run id, initializes agent state, and returns the authoritative run state

#### Scenario: Host cannot mutate run state directly
- **WHEN** the desktop host receives a UI request that would change a run or agent state
- **THEN** the host proxies the request to the Rust runtime instead of mutating semantic runtime state locally

### Requirement: Rust runtime executes agent loops
The system SHALL execute the agent loop, including LLM turns, tool-call routing, observation handling, and terminal conditions, inside the Rust runtime rather than in renderer JavaScript or host-owned orchestration code.

#### Scenario: Tool-driven ReAct loop runs in Rust
- **WHEN** an agent response contains one or more tool calls
- **THEN** the Rust runtime decides the next loop step, invokes tools through runtime-defined routing, appends observations, and continues execution without renderer-owned loop logic

#### Scenario: Terminal tool ends the loop in Rust
- **WHEN** an agent completes work by invoking a terminal runtime tool such as document submission
- **THEN** the Rust runtime records the terminal transition and stops the loop without requiring renderer-side control flow

### Requirement: Rust runtime emits authoritative runtime events
The system SHALL treat runtime-emitted events from the Rust process as the authoritative source for run status, trace events, permission requests, and token streaming. The host and renderer SHALL project those events but SHALL NOT synthesize competing authoritative runtime facts.

#### Scenario: Trace event flows from runtime to UI
- **WHEN** the runtime executes a tool call
- **THEN** the Rust runtime emits the corresponding trace event and the host forwards that event to the UI for rendering

#### Scenario: Permission request originates in runtime
- **WHEN** the runtime reaches a step that requires explicit user approval
- **THEN** the Rust runtime emits a permission request event and awaits a correlated user response instead of the host inventing the request locally

### Requirement: Runtime authority survives host restarts through rehydration
The system SHALL persist runtime-owned operational state so the Rust runtime can rehydrate runs and agent state after a process restart without relying on renderer-owned execution history.

#### Scenario: Runtime rehydrates paused work
- **WHEN** the runtime restarts while a run was paused or awaiting approval
- **THEN** the runtime restores the run's authoritative state and reports it to the host on reconnect

#### Scenario: Runtime restart invalidates stale host projections
- **WHEN** the host reconnects to a restarted runtime
- **THEN** the host replaces its local projections with runtime-reported authoritative state before accepting further UI mutations