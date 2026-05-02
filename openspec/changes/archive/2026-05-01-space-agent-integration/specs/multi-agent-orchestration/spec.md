## ADDED Requirements

### Requirement: Agent graph execution

The system SHALL execute an `AgentGraph` by traversing its nodes and edges starting from `entry_node_id`. The execution engine SHALL run in the CEF renderer process (JavaScript). Execution SHALL support LLM, Tool, Condition, Human, and Subgraph node types.

#### Scenario: Execute a single LLM node graph

- **WHEN** a graph with one LLM node and no tool calls is executed with an initial task
- **THEN** the LLM is called, the response is returned as the final output, and execution completes

#### Scenario: Graph respects max_iterations

- **WHEN** a graph's agent loop reaches `max_iterations` without a terminal condition
- **THEN** execution halts and returns an error result indicating the iteration limit was exceeded

---

### Requirement: Agent loop (ReAct pattern)

The system SHALL implement a ReAct agent loop within an LLM node: call LLM → parse tool calls → execute tools → append results → call LLM again, until the LLM produces a response with no tool calls or a terminal condition is reached.

#### Scenario: Tool call round trip

- **WHEN** the LLM response contains one or more tool calls
- **THEN** the engine executes each tool call via the tool bridge, appends the results to the message history, and calls the LLM again

#### Scenario: Terminal condition

- **WHEN** the LLM response contains no tool calls
- **THEN** the loop terminates and the response content is returned as the final output

---

### Requirement: Conditional edge routing

The system SHALL route execution to the next node by evaluating the `condition` field on outgoing edges. An edge with an empty condition SHALL act as an unconditional default.

#### Scenario: Route on has_tool_calls condition

- **WHEN** an LLM node completes and an outgoing edge has `condition: "has_tool_calls"` and the response contained tool calls
- **THEN** execution follows that edge to the next node

#### Scenario: Default edge

- **WHEN** no conditional edge matches and a default (empty condition) edge exists
- **THEN** execution follows the default edge

---

### Requirement: Human-in-the-loop node

The system SHALL pause graph execution when a `kHuman` node is reached and emit a permission request event. Execution SHALL resume only after the user provides an explicit allow or deny response via the permission UI.

#### Scenario: Pause at human node

- **WHEN** graph execution reaches a node of kind `kHuman`
- **THEN** the agent panel shows a pending permission prompt and execution is suspended

#### Scenario: Resume after allow

- **WHEN** the user approves a permission request triggered by a `kHuman` node
- **THEN** graph execution resumes from the node immediately following the `kHuman` node

#### Scenario: Abort after deny

- **WHEN** the user denies a permission request triggered by a `kHuman` node
- **THEN** graph execution halts and returns a result indicating user-cancelled

---

### Requirement: Subgraph execution

The system SHALL support nested graph execution via `kSubgraph` nodes. A subgraph node SHALL execute a full `AgentGraph` as a step and return its output to the parent graph.

#### Scenario: Subgraph completes and returns output

- **WHEN** execution reaches a `kSubgraph` node
- **THEN** the referenced subgraph is executed to completion, and its final output is injected as a message in the parent graph's context before continuing

---

### Requirement: Parallel agents across Spaces

Each Space SHALL maintain an independent agent runtime. Multiple Spaces MAY have active agent loops running concurrently. Each agent loop SHALL be isolated to its Space's tool scope.

#### Scenario: Two Spaces run agents simultaneously

- **WHEN** an agent loop is running in Space A and the user starts an agent loop in Space B
- **THEN** both loops execute concurrently without interfering with each other's tool calls or message history

---

### Requirement: Agent trace streaming

The system SHALL emit trace events during execution (llm_call, tool_call, human_input, done) to the agent panel UI in real time. Each event SHALL include a timestamp and JSON payload.

#### Scenario: Trace event delivery

- **WHEN** an agent loop executes a tool call
- **THEN** a `tool_call` trace event with the tool name, input, and output is dispatched to the agent panel before the next LLM call begins

---

### Requirement: Command-block to agent task

The system SHALL allow the user to dispatch a failed command block as an agent task. The task payload SHALL include the command string, full output, exit code, cwd, and Space id.

#### Scenario: Dispatch Fix from failed block

- **WHEN** a command block has exit code ≠ 0 and the user activates the Fix action
- **THEN** an agent task is created with the command block's context, the agent panel becomes active, and the agent loop begins executing
