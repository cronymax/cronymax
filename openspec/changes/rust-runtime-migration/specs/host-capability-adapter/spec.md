## ADDED Requirements

### Requirement: Host exposes privileged capabilities to the runtime
The desktop host SHALL expose privileged local capabilities to the Rust runtime through a capability adapter surface rather than through host-owned agent semantics. The capability surface SHALL include shell or PTY execution, browser or page inspection, notifications, filesystem mediation, and secret-management operations needed by the runtime.

#### Scenario: Runtime invokes shell capability
- **WHEN** the runtime needs to execute a sandboxed shell command
- **THEN** it invokes the host shell capability and receives a structured result rather than executing host-specific orchestration logic directly in the UI process

#### Scenario: Runtime invokes browser inspection capability
- **WHEN** the runtime needs the active page URL or extracted page content
- **THEN** the host returns that data through the capability adapter without taking ownership of the higher-level tool semantics

### Requirement: Host capability adapter enforces local policy
The host capability adapter SHALL enforce local policy for workspace scope, OS privilege boundaries, and user-granted permissions before completing a capability call.

#### Scenario: File access outside workspace is denied
- **WHEN** the runtime asks the host for filesystem access outside the active Space's allowed workspace scope
- **THEN** the host rejects the capability request with a structured permission error

#### Scenario: User approval gates sensitive capability
- **WHEN** the runtime requests a capability that requires explicit user approval
- **THEN** the host renders the prompt and returns the user's decision to the runtime as a capability or control response

### Requirement: Capability adapter returns structured results and errors
The host capability adapter SHALL return structured success and failure results so the Rust runtime can interpret capability outcomes without host-specific parsing.

#### Scenario: Structured terminal result
- **WHEN** a shell execution completes
- **THEN** the host returns exit code, stdout, stderr, and any policy metadata in a structured capability result

#### Scenario: Structured secret lookup failure
- **WHEN** a secret or token lookup fails
- **THEN** the host returns a structured error classification that allows the runtime to decide whether to retry, prompt, or fail the run

### Requirement: Capability adapter remains subordinate to runtime authority
The host capability adapter SHALL not create or mutate authoritative run or agent state as part of serving a capability call.

#### Scenario: Notification post does not mutate run state locally
- **WHEN** the runtime asks the host to post an OS notification for a run event
- **THEN** the host posts the notification and returns the result without changing local run status or trace state on its own

#### Scenario: Capability timeout is reported to runtime
- **WHEN** a capability call times out or the host cannot complete it
- **THEN** the host reports the timeout to the runtime and the runtime decides the resulting run transition