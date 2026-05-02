## MODIFIED Requirements

### Requirement: Agent graph execution

The system SHALL execute an `AgentGraph` by traversing its nodes and edges starting from `entry_node_id`. The execution engine SHALL run in the CEF renderer process (JavaScript). Execution SHALL support LLM and Tool node kinds. The `AgentGraph` data model SHALL be **internal-only** — used to define a single Agent's ReAct loop, not user-facing multi-agent orchestration. Multi-agent orchestration is governed by the `flow-orchestration` capability.

#### Scenario: Execute a single LLM node graph

- **WHEN** a graph with one LLM node and no tool calls is executed with an initial task
- **THEN** the LLM is called, the response is returned as the final output, and execution completes

#### Scenario: Graph respects max_iterations

- **WHEN** a graph's agent loop reaches `max_iterations` without a terminal condition
- **THEN** execution halts and returns an error result indicating the iteration limit was exceeded

---

### Requirement: Agent loop (ReAct pattern)

The system SHALL implement a ReAct agent loop within an LLM node: call LLM → parse tool calls → execute tools → append results → call LLM again, until the LLM produces a response with no tool calls, calls the `submit_document` terminal tool, or a terminal condition is reached.

#### Scenario: Tool call round trip

- **WHEN** the LLM response contains one or more tool calls
- **THEN** the engine executes each tool call via the tool bridge, appends the results to the message history, and calls the LLM again

#### Scenario: Terminal condition via submit_document

- **WHEN** the LLM response calls the `submit_document` tool
- **THEN** the loop terminates after the tool returns success and the Agent's Run participation completes

#### Scenario: Terminal condition via no tool calls

- **WHEN** the LLM response contains no tool calls
- **THEN** the loop terminates and the response content is returned as the Agent's output

---

### Requirement: Agent trace streaming

The system SHALL emit trace events during execution (`agent.thinking`, `agent.tool_call`, `agent.completed`) to the active Flow Run's `trace.jsonl` and to subscribed UI clients in real time. Each event SHALL include a timestamp, agent id, and JSON payload.

#### Scenario: Trace event delivery

- **WHEN** an agent loop executes a tool call
- **THEN** an `agent.tool_call` trace event with the tool name, input, and output is appended to `runs/<run-id>/trace.jsonl` and dispatched to subscribed UI clients before the next LLM call begins

---

### Requirement: Parallel agents in a Flow Run

Each Agent declared in a Flow SHALL run as an independent instance with its own ReAct loop. Multiple Agents in the same Flow Run MAY have active loops running concurrently. Each Agent's loop SHALL be isolated to its own message history, memory namespace, and tool scope.

#### Scenario: Two agents run concurrently in one Run

- **WHEN** a Flow fans out a Document to two downstream Agents
- **THEN** both Agents' ReAct loops execute concurrently without interfering with each other's tool calls or message history

## REMOVED Requirements

### Requirement: Conditional edge routing

**Reason**: Replaced by typed-port routing in `flow-orchestration`. Conditional edges within a single Agent's internal graph are no longer a public concept.

**Migration**: Use Flow-level typed ports (`flow-orchestration`'s "Typed-port routing" requirement) for inter-Agent routing. Conditional control flow within a single Agent's reasoning is handled by the LLM's natural language and tool-calling, not by graph topology.

---

### Requirement: Human-in-the-loop node

**Reason**: Replaced by the Reviewer pipeline in `document-collaboration`. Humans participate as Reviewers on Documents, not as workflow nodes.

**Migration**: Add `requires_human_approval: true` to a Flow edge to gate transitions on human review (`document-collaboration`'s "Reviewer pipeline" requirement). The user's approval surface is the chat panel and the Document workbench, not a dedicated graph node.

---

### Requirement: Subgraph execution

**Reason**: User-facing subgraph composition is replaced by Flow composition in a future change. For now, multi-Agent orchestration is flat within a single Flow.

**Migration**: Decompose previous subgraph use cases into separate Flows triggered manually, or wait for the planned `flow-composition` capability in a follow-on change.

---

### Requirement: Command-block to agent task

**Reason**: Moved out of `multi-agent-orchestration` scope. Command-block dispatch is now a Flow trigger surface (the user starts a Flow Run from a failed terminal block); it belongs to `warp-terminal` integration with `flow-orchestration`.

**Migration**: The "Fix" action on a failed command block now starts a built-in `bug-fix` Flow Run with the command context as the initial input message, rather than directly invoking an Agent loop.
