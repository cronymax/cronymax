## MODIFIED Requirements

### Requirement: Agent graph execution

The system SHALL execute an `AgentGraph` by traversing its nodes and edges starting from `entry_node_id`. The execution engine SHALL run in the **Rust runtime process** using a `ReactLoop`. For flows (graphs with a `flow_id`), scheduling SHALL be owned by `FlowRuntime`; for standalone agent runs, a single `ReactLoop` SHALL be spawned directly. Execution SHALL support LLM, Tool, Condition, Human, and Subgraph node types.

#### Scenario: Execute a single LLM node graph

- **WHEN** a graph with one LLM node and no tool calls is executed with an initial task
- **THEN** the LLM is called, the response is returned as the final output, and execution completes

#### Scenario: Graph respects max_iterations

- **WHEN** a graph's agent loop reaches `max_iterations` without a terminal condition
- **THEN** execution halts and returns an error result indicating the iteration limit was exceeded

---

### Requirement: Human-in-the-loop node

The system SHALL pause graph execution when a `kHuman` node is reached **or when an agent node is configured with `requires_approval: true`** and emit a review-request event. Execution SHALL resume only after an explicit allow or deny response is received via `ResolveReview` control request routed through `RuntimeProxy`.

#### Scenario: Pause at human node

- **WHEN** graph execution reaches a node of kind `kHuman`
- **THEN** the agent panel shows a pending permission prompt and execution is suspended

#### Scenario: Resume after allow

- **WHEN** the user approves via `review.approve` which dispatches `ResolveReview { decision: Approved }`
- **THEN** graph execution resumes from the node immediately following the paused node

#### Scenario: Abort after deny

- **WHEN** the user denies via `review.request_changes` which dispatches `ResolveReview { decision: RequestChanges }`
- **THEN** graph execution halts or the originating agent is re-invoked with reviewer notes
