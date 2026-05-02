## ADDED Requirements

### Requirement: Agent definition file

The system SHALL load Agents from YAML files at `<workspace>/.cronymax/agents/<agent-name>.agent.yaml`. An Agent definition SHALL declare a `name`, `kind` (`producer` | `reviewer`), `llm` (provider + model), `system_prompt` (inline or `system_prompt_file`), `memory_namespace`, and an optional `skills:` list.

#### Scenario: Load a valid Agent

- **WHEN** the system reads a syntactically valid `<agent>.agent.yaml`
- **THEN** the Agent is registered in the active Space's agent registry and addressable by its `name`

#### Scenario: Reviewer agents declare review-only

- **WHEN** an Agent's `kind` is `reviewer`
- **THEN** the Agent is restricted to emitting comments via the Reviewer pipeline and SHALL NOT be schedulable as a Document producer in any Flow edge

#### Scenario: Missing required field rejects the file

- **WHEN** an `<agent>.agent.yaml` omits `name`, `llm`, or `system_prompt`
- **THEN** the load fails with a validation error and the agent is not registered

---

### Requirement: Per-Space agent registry

The system SHALL maintain an in-memory agent registry per Space, populated by scanning `<workspace>/.cronymax/agents/*.agent.yaml` on Space activation. The registry SHALL refresh when files are added, removed, or modified.

#### Scenario: New agent file is auto-detected

- **WHEN** the user adds a new `<agent>.agent.yaml` to `.cronymax/agents/` while the Space is active
- **THEN** the registry detects the change and the new Agent becomes available for use in Flows without an app restart

#### Scenario: Registry scope isolation

- **WHEN** the user switches from Space A to Space B
- **THEN** the active agent registry reflects only Space B's `.cronymax/agents/` contents

---

### Requirement: Agent runtime instance per Flow Run

The system SHALL instantiate one `AgentRuntime` per declared Agent per active Flow Run. Each instance SHALL have its own message history, context window, memory namespace, and tool scope. Two Agents in the same Run SHALL NOT share message history.

#### Scenario: Two agents in same Run have isolated context

- **WHEN** ProductAgent and ArchitectAgent both run in the same Flow Run
- **THEN** ArchitectAgent's LLM call does not include ProductAgent's internal message history (only the handed-off Document is shared)

#### Scenario: Same Agent in two parallel Runs has separate state

- **WHEN** the user starts two Runs of the same Flow concurrently
- **THEN** each Run has its own `AgentRuntime` instance for each Agent with independent state

---

### Requirement: Document-producing tool

The system SHALL expose a `submit_document(type, content, [@mentions])` tool to producer Agents. Calling this tool SHALL transition the in-progress Document to `IN_REVIEW` and trigger the Reviewer pipeline. The tool SHALL be the canonical way an Agent's loop terminates with output.

#### Scenario: Submit transitions doc and ends the loop

- **WHEN** an Agent's LLM emits a `submit_document` tool call
- **THEN** the runtime writes the content to disk, schedules reviewers, and exits the Agent's ReAct loop with success

#### Scenario: Submit with unknown doc type

- **WHEN** the Agent submits a doc with a `type` not declared as an outgoing port for this Agent in the Flow
- **THEN** the tool returns an error to the Agent's loop, the doc is not persisted, and the loop continues

---

### Requirement: Memory namespace per Agent

Each Agent SHALL have a logical memory namespace (default: `<agent-name>`). Memory operations from the Agent's loop SHALL be scoped to its namespace and SHALL NOT read or write other Agents' memory.

#### Scenario: Memory isolation

- **WHEN** ProductAgent writes a memory entry with key `last_pivot`
- **THEN** ArchitectAgent reading `last_pivot` from its own namespace receives no value
