## ADDED Requirements

### Requirement: Flow definition file

The system SHALL load a Flow from a YAML file at `<workspace>/.cronymax/flows/<flow-name>/flow.yaml`. A Flow definition SHALL declare a name, the participating agents (by file basename in `.cronymax/agents/`), and the directed edges between them with typed Document ports.

#### Scenario: Load a valid Flow

- **WHEN** the system reads a syntactically valid `flow.yaml` referencing only declared agents and known doc types
- **THEN** the Flow is registered with the active Space and made available for execution

#### Scenario: Flow references an undeclared agent

- **WHEN** an edge in `flow.yaml` references an agent name not listed under the Flow's `agents:` field
- **THEN** the load fails with a validation error identifying the missing agent and the Flow is not registered

#### Scenario: Flow references an unknown doc type

- **WHEN** an edge port references a doc type that has no schema under `.cronymax/doc-types/`
- **THEN** the load fails with a validation error identifying the unknown doc type

---

### Requirement: Flow Run lifecycle

The system SHALL create a Flow Run when a Flow is started. Each Run SHALL have a unique id, a run directory at `.cronymax/flows/<flow>/runs/<run-id>/`, a `state.json` file persisted on every status transition, and a status transitioning through `PENDING → RUNNING → (PAUSED | COMPLETED | FAILED | CANCELLED)`.

#### Scenario: Start a Flow Run

- **WHEN** the user starts a Flow with an initial input message
- **THEN** the system creates a run directory, writes `state.json` with status `RUNNING`, instantiates the entry Agent(s), and emits a `flow.run.started` event

#### Scenario: Run state survives app restart

- **WHEN** the app restarts while a Flow Run was in `PAUSED` status (awaiting human review)
- **THEN** on launch the Run is rehydrated from `state.json` and remains in `PAUSED`, ready to resume when the user acts

#### Scenario: Cancel a Run

- **WHEN** the user cancels a Run
- **THEN** all in-flight Agent loops in the Run are aborted, in-flight LLM requests are cancelled, status transitions to `CANCELLED`, and `state.json` is persisted

---

### Requirement: Typed-port routing (default route)

The system SHALL route a submitted Document to all Agents reachable via outgoing edges from the producing Agent whose port type matches the submitted Document's type. Routing SHALL happen automatically when the Document transitions to `APPROVED`.

#### Scenario: Single matching downstream

- **WHEN** ProductAgent submits a Document of type `prd` and the Flow has one edge `product:prd → architect:prd`
- **THEN** ArchitectAgent is scheduled with the approved PRD as input

#### Scenario: Multiple matching downstreams (fan-out)

- **WHEN** an Agent submits a Document type matching outgoing edges to two different downstream Agents
- **THEN** both downstream Agents are scheduled in parallel with the same Document

#### Scenario: No matching port

- **WHEN** an Agent submits a Document type that has no outgoing edge of that type
- **THEN** the Document remains in `APPROVED` status with no further routing and the Run completes if no other Agents are pending

---

### Requirement: @-mention escape-hatch routing

The system SHALL parse `@<AgentName>` mentions in the body of a submitted Document and SHALL additionally schedule each mentioned Agent in addition to the typed-port default route. `@mention` routing SHALL be additive, never replacing the default route.

#### Scenario: Mention adds a recipient

- **WHEN** a submitted PRD contains `@SecReview` in its body and the Flow's typed ports route the PRD to ArchitectAgent
- **THEN** both ArchitectAgent (default route) and SecReviewAgent (mention) are scheduled with the PRD

#### Scenario: Mention of an undeclared agent

- **WHEN** a Document mentions `@UnknownAgent` not declared in the Flow
- **THEN** the system emits a warning event and does not schedule the unknown agent; default routing proceeds normally

#### Scenario: Mention inside a fenced code block is ignored

- **WHEN** an `@AgentName` token appears only inside a triple-backtick fenced code block
- **THEN** it is not parsed as a mention and no routing occurs

#### Scenario: Mention enables backward routing

- **WHEN** a CoderAgent's submitted Document mentions `@ProductAgent` to request clarification
- **THEN** ProductAgent is scheduled even though the Flow has no `coder → product` edge

---

### Requirement: Run trace event stream

The system SHALL emit typed events during Run execution and persist them to `runs/<run-id>/trace.jsonl` (one JSON object per line). Event types SHALL include at minimum: `flow.run.started`, `flow.run.completed`, `flow.run.failed`, `flow.run.cancelled`, `agent.scheduled`, `agent.thinking`, `agent.tool_call`, `document.submitted`, `document.handed_off`, `review.comment_added`, `review.approved`, `review.changes_requested`, `mention.parsed`, `error`.

#### Scenario: Event written for document submission

- **WHEN** an Agent submits a Document
- **THEN** a `document.submitted` event is appended to `trace.jsonl` with the document path, revision number, submitter agent id, and timestamp

#### Scenario: Trace replay on UI subscription

- **WHEN** a UI client subscribes to a Run's events mid-execution
- **THEN** the system streams the existing `trace.jsonl` content followed by live events, with no duplication or gaps

---

### Requirement: Bridge surface for Flow execution

The system SHALL expose the following bridge channels for the renderer: `flow.list`, `flow.load`, `flow.run.start`, `flow.run.status`, `flow.run.cancel`. All payloads SHALL be JSON. Server-push events SHALL be delivered on a single `event` channel with a `type` discriminator field.

#### Scenario: List Flows in active Space

- **WHEN** the renderer sends `flow.list` with the active Space id
- **THEN** the system responds with a JSON array of Flow names, statuses of any active Runs, and last-modified timestamps
