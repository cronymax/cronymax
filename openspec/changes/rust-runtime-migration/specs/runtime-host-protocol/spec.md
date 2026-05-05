## ADDED Requirements

### Requirement: Runtime protocol uses GIPS
The system SHALL use GIPS as the only IPC transport between the desktop host and the standalone Rust runtime. Control requests, event streams, and host capability invocations SHALL all cross the runtime boundary over GIPS.

#### Scenario: Host connects to runtime over GIPS
- **WHEN** the desktop app starts the runtime process
- **THEN** the host establishes GIPS connections for runtime control and event delivery before enabling runtime-backed UI actions

#### Scenario: No alternate runtime transport
- **WHEN** the host communicates with the runtime for orchestration or capability purposes
- **THEN** it uses the GIPS protocol surface instead of direct bridge-channel semantics or ad hoc subprocess pipes

### Requirement: Protocol supports correlated requests and streaming events
The runtime protocol SHALL support correlated request/response messages for control and capability calls and SHALL support replayable plus live event streaming from the runtime to the host.

#### Scenario: Correlated capability response
- **WHEN** the runtime asks the host to perform a privileged capability call
- **THEN** the host returns the result using the same correlation identifier so the runtime can resume the waiting operation deterministically

#### Scenario: Host subscribes to live runtime events
- **WHEN** a UI surface subscribes to a run's activity
- **THEN** the host obtains replayable and live runtime events from the runtime protocol and forwards them without inventing synthetic trace order

### Requirement: Protocol surfaces runtime lifecycle explicitly
The protocol SHALL expose runtime availability, version compatibility, and reconnect behavior so the host can present runtime health accurately and recover from restarts.

#### Scenario: Version mismatch blocks startup
- **WHEN** the host and runtime protocol versions are incompatible
- **THEN** the host rejects runtime-backed actions and surfaces a protocol compatibility error instead of attempting partial operation

#### Scenario: Runtime restart triggers resubscription
- **WHEN** the runtime process restarts during an app session
- **THEN** the host reconnects, re-establishes event subscriptions, and refreshes projections from runtime authority before resuming normal UI actions

### Requirement: Protocol separates control from capability execution
The runtime protocol SHALL distinguish semantic control requests from host capability execution so capability latency or failure does not redefine runtime ownership.

#### Scenario: Capability failure does not transfer authority
- **WHEN** a host capability call fails
- **THEN** the runtime records the failure as part of runtime state and decides the next transition instead of the host applying orchestration logic locally

#### Scenario: Control mutation never bypasses runtime
- **WHEN** the user approves a review or cancels a run
- **THEN** the host sends a control mutation to the runtime rather than mutating local storage and broadcasting the result as authoritative