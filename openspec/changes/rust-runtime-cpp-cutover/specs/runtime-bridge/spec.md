## ADDED Requirements

### Requirement: Runtime child process supervision

The host SHALL launch a standalone `cronymax-runtime` child process during app startup, establish the GIPS handshake before exposing runtime-backed bridge operations, restart the child after unexpected termination, and stop the child during app shutdown.

#### Scenario: Startup handshake succeeds

- **WHEN** the application creates the main window for a session with runtime-backed orchestration enabled
- **THEN** the host starts `cronymax-runtime`, completes the Hello/Welcome protocol handshake, and marks the runtime bridge available before processing `agent.*`, `flow.*`, `review.*`, `events.*`, `inbox.*`, `permission.*`, or `document.*` requests

#### Scenario: Runtime child crashes

- **WHEN** the supervised runtime child exits unexpectedly while the application remains open
- **THEN** the host starts a replacement child, re-establishes the handshake, and resumes accepting new bridge requests without re-enabling the legacy in-process orchestration path

### Requirement: Bridge request forwarding

The host SHALL translate each supported browser bridge request for orchestration, reviews, inbox, permissions, and document actions into the corresponding runtime protocol envelope, send it over GIPS, and return the runtime reply without executing the legacy in-process orchestration code.

#### Scenario: Forward a run control request

- **WHEN** the renderer invokes a supported `agent.*` or `flow.*` bridge method
- **THEN** the host forwards the request through `RuntimeProxy`, waits for the runtime reply, and returns the runtime-produced payload to the renderer

#### Scenario: Runtime unavailable request fails closed

- **WHEN** a supported bridge request arrives before the runtime bridge becomes available or while restart is still in progress
- **THEN** the host rejects the request with a runtime-unavailable error and does not fall back to the removed in-process runtime

### Requirement: Runtime event subscription fanout

The host SHALL maintain a runtime event subscription over GIPS and fan out subscribed runtime events to renderer clients using the existing bridge subscription channel shape.

#### Scenario: Event stream reaches subscribed renderer client

- **WHEN** the runtime emits a run-history, review, inbox, tool, or completion event for a subscribed Space or Run
- **THEN** the host forwards the event to each matching renderer subscriber in arrival order using the bridge event subscription channel

### Requirement: Host capability adapter boundary

The host SHALL service runtime-initiated capability and approval requests through explicit adapters, including the permission broker, and SHALL return adapter results back to the runtime over GIPS.

#### Scenario: Runtime requests approval for a protected capability

- **WHEN** the runtime issues a capability request that requires user approval
- **THEN** the host consults the permission broker, returns the approval decision to the runtime, and does not expose a renderer-direct path that can bypass the runtime-originated request

### Requirement: Legacy state import on first cutover launch

The host SHALL import legacy per-run state snapshots from workspace-owned `.cronymax/flows/.../state.json` files into the runtime persistence store once, then treat the runtime persistence file as the single semantic source of truth.

#### Scenario: First launch with legacy run state

- **WHEN** the cutover build starts against a workspace that contains legacy host-managed run state files
- **THEN** the host imports those snapshots into the runtime before serving runtime-backed requests and records completion so subsequent launches do not re-import unchanged legacy state
