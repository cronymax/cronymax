## MODIFIED Requirements

### Requirement: Agent graph execution

The system SHALL execute an `AgentGraph` by traversing its nodes and edges starting from `entry_node_id`. The execution engine SHALL run inside the standalone Rust runtime process and SHALL be invoked from the host and renderer through the runtime bridge. Execution SHALL support LLM, Tool, Condition, Human, and Subgraph node types.

#### Scenario: Execute a single LLM node graph

- **WHEN** a graph with one LLM node and no tool calls is executed with an initial task
- **THEN** the host forwards the request to the runtime, the runtime calls the LLM, and the runtime response is returned as the final output

#### Scenario: Graph respects max_iterations

- **WHEN** a graph's agent loop reaches `max_iterations` without a terminal condition
- **THEN** the runtime halts execution and returns an error result indicating the iteration limit was exceeded

### Requirement: Agent loop (ReAct pattern)

The system SHALL implement a ReAct agent loop within an LLM node inside the Rust runtime: call LLM, parse tool calls, dispatch tools through runtime-owned capability adapters, append results, and call the LLM again until the LLM produces a response with no tool calls or a terminal condition is reached.

#### Scenario: Tool call round trip

- **WHEN** the LLM response contains one or more tool calls
- **THEN** the runtime executes each tool call through its capability dispatch boundary, appends the results to the message history, and calls the LLM again

#### Scenario: Terminal condition

- **WHEN** the LLM response contains no tool calls
- **THEN** the runtime loop terminates and the response content is returned as the final output

### Requirement: Human-in-the-loop node

The system SHALL pause graph execution when a `kHuman` node is reached and emit a review or approval event from the runtime. Execution SHALL resume only after the user provides an explicit allow or deny response through the host bridge and the runtime receives the decision.

#### Scenario: Pause at human node

- **WHEN** graph execution reaches a node of kind `kHuman`
- **THEN** the runtime emits a pending review event and execution is suspended until a bridge-supplied decision is received

#### Scenario: Resume after allow

- **WHEN** the user approves a runtime-issued permission or review request
- **THEN** the host forwards the approval to the runtime and graph execution resumes from the node immediately following the `kHuman` node

#### Scenario: Abort after deny

- **WHEN** the user denies a runtime-issued permission or review request
- **THEN** the host forwards the denial to the runtime and graph execution halts with a user-cancelled result

### Requirement: Parallel agents across Spaces

Each Space SHALL remain independently addressable for runtime-managed runs. Multiple Spaces MAY have active runs concurrently, and each run SHALL execute with the workspace root, permissions, and subscriptions associated with its owning Space rather than a host-owned per-Space agent runtime instance.

#### Scenario: Two Spaces run agents simultaneously

- **WHEN** a run is active for Space A and the user starts another run for Space B
- **THEN** both runs execute concurrently in the runtime without interfering with each other's tool scope, message history, or approvals

### Requirement: Agent trace streaming

The system SHALL emit runtime execution events during agent execution and deliver them to the UI in real time through the runtime event subscription. Each event SHALL include a timestamp and JSON payload.

#### Scenario: Trace event delivery

- **WHEN** an agent loop executes a tool call
- **THEN** the runtime emits a `tool_call` event with the tool name, input, and output and the host forwards that event to subscribed UI clients before the next LLM call begins

### Requirement: Command-block to agent task

The system SHALL allow the user to dispatch a failed command block as an agent task through the runtime bridge. The task payload SHALL include the command string, full output, exit code, cwd, and Space id.

#### Scenario: Dispatch Fix from failed block

- **WHEN** a command block has exit code not equal to `0` and the user activates the Fix action
- **THEN** the host forwards the command block context to the runtime, the agent panel becomes active, and the runtime begins executing the task
