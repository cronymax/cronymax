## MODIFIED Requirements

### Requirement: Agent graph execution
The system SHALL execute an `AgentGraph` by traversing its nodes and edges starting from `entry_node_id`. The execution engine SHALL run in the standalone Rust runtime process. Execution SHALL support LLM, Tool, Condition, Human, and Subgraph node types.

#### Scenario: Execute a single LLM node graph
- **WHEN** a graph with one LLM node and no tool calls is executed with an initial task
- **THEN** the Rust runtime calls the LLM, returns the final output, and marks execution complete

#### Scenario: Graph respects max_iterations
- **WHEN** a graph's agent loop reaches `max_iterations` without a terminal condition
- **THEN** the Rust runtime halts execution and returns an error result indicating the iteration limit was exceeded

### Requirement: Agent loop (ReAct pattern)
The system SHALL implement a ReAct agent loop within an LLM node in the standalone Rust runtime: call LLM, parse tool calls, execute tools, append results, and call the LLM again until the LLM produces a response with no tool calls or a terminal condition is reached.

#### Scenario: Tool call round trip
- **WHEN** the LLM response contains one or more tool calls
- **THEN** the Rust runtime executes each tool call via its tool routing layer, appends the results to the message history, and calls the LLM again

#### Scenario: Terminal condition
- **WHEN** the LLM response contains no tool calls
- **THEN** the Rust runtime terminates the loop and returns the response content as the final output

### Requirement: Human-in-the-loop node
The system SHALL pause graph execution when a `kHuman` node is reached and emit a permission request from the Rust runtime through the host/UI boundary. Execution SHALL resume only after the user provides an explicit allow or deny response that is returned to the Rust runtime.

#### Scenario: Pause at human node
- **WHEN** graph execution reaches a node of kind `kHuman`
- **THEN** the runtime emits a pending permission request and execution is suspended until the host returns the user's decision

#### Scenario: Resume after allow
- **WHEN** the user approves a permission request triggered by a `kHuman` node
- **THEN** the Rust runtime resumes graph execution from the node immediately following the `kHuman` node

#### Scenario: Abort after deny
- **WHEN** the user denies a permission request triggered by a `kHuman` node
- **THEN** the Rust runtime halts execution and returns a result indicating user-cancelled

### Requirement: Parallel agents across Spaces
The standalone Rust runtime SHALL maintain independent agent contexts for each Space. Multiple Spaces MAY have active agent loops running concurrently, and each loop SHALL remain isolated to its Space's tool scope and runtime state.

#### Scenario: Two Spaces run agents simultaneously
- **WHEN** an agent loop is running in Space A and the user starts an agent loop in Space B
- **THEN** the Rust runtime executes both loops concurrently without interfering with each other's tool calls, memory namespaces, or message history

### Requirement: Agent trace streaming
The system SHALL emit trace events during execution from the Rust runtime in real time. Each event SHALL include a timestamp and structured payload, and the host SHALL forward those runtime events to the UI.

#### Scenario: Trace event delivery
- **WHEN** an agent loop executes a tool call
- **THEN** the Rust runtime emits a `tool_call` trace event with the tool name, input, and output before the next LLM call begins

### Requirement: Command-block to agent task
The system SHALL allow the user to dispatch a failed command block as an agent task. The host SHALL forward the command string, full output, exit code, cwd, and Space id to the Rust runtime, and the runtime SHALL begin executing the corresponding agent task.

#### Scenario: Dispatch Fix from failed block
- **WHEN** a command block has exit code not equal to 0 and the user activates the Fix action
- **THEN** the command block context is sent to the Rust runtime, the agent panel becomes active, and runtime-owned execution begins