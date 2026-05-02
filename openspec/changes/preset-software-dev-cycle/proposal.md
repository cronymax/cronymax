## Why

The `agent-document-orchestration` change ships a generic Flow engine but no opinionated starting point for the most common agentic use case: a full software development cycle (design → implementation → testing). Teams need a ready-to-run PM → RD → QA preset flow with role-appropriate agents, document types, and review gates — so they can start collaborating with agents immediately without hand-authoring YAML from scratch.

## What Changes

- **NEW** `software-dev-cycle` preset flow (`assets/examples/flows/software-dev-cycle/flow.yaml`) with PM, RD, QA, and QA-Critic agents covering the full design-to-ship cycle.
- **NEW** Four preset agent YAML files (`pm.agent.yaml`, `rd.agent.yaml`, `qa.agent.yaml`, `qa-critic.agent.yaml`) in `assets/examples/agents/` with role-specific system prompts and tool assignments.
- **NEW** Five built-in document types: `prototype`, `submit-for-testing`, `bug-report`, `test-report` (new); `test-cases` promoted to a first-class built-in (replaces the example-only `test-plan`).
- **NEW** `test_runner.*` built-in tool family (`test_runner.discover`, `test_runner.run_suite`, `test_runner.get_last_report`) available to agents with `kind: producer`; outputs structured JSON results for test-report generation.
- **MODIFIED** Flow edge schema: adds `on_approved_reschedule`, `max_cycles`, `on_cycle_exhausted`, and per-edge `reviewer_agents` fields.
- **MODIFIED** Run `state.json`: extends to track per-agent port completion state and invocation history, enabling the FlowRuntime to inject deterministic task context on agent re-invocation.
- **MODIFIED** Agent YAML schema: `llm` field changes from a flat model-name string to a structured `{provider, model}` object referencing the provider registry.
- **MODIFIED** LLM provider configuration: replaces the single global `{base_url, api_key}` config with a named multi-provider registry supporting `openai-compat`, `github-copilot` (OAuth device flow), and `none` (local) auth kinds.

## Capabilities

### New Capabilities

- `dev-cycle-flow-preset`: The `software-dev-cycle` preset flow definition, all four role agents (PM, RD, QA, QA-Critic), and the five new document types that wire them together.
- `test-runner-tools`: The `test_runner.*` built-in tool family for QA agents — suite discovery, structured execution results, and last-report retrieval. Defined in `agent-entity`; migrates to the Skills Marketplace in a future change.

### Modified Capabilities

- `flow-orchestration`: Adds `on_approved_reschedule` (re-invoke producer after approval), `max_cycles` / `on_cycle_exhausted` (peer-cycle loop cap on an edge pair), per-edge `reviewer_agents` override, port completion tracking in `state.json`, and typed `InvocationContext` envelope injected by FlowRuntime on each agent re-schedule.
- `agent-entity`: Adds `test_runner.*` tool declarations; changes `llm:` field to structured `{provider, model}` shape.
- `llm-provider`: Replaces single global config with a named provider registry; adds `github-copilot` (GitHub device-flow OAuth → token exchange) and `openai-compat` auth kinds; API keys stored in OS keychain by provider ID.

## Impact

- **`assets/builtin-doc-types/`**: Add `prototype.yaml`, `submit-for-testing.yaml`, `bug-report.yaml`, `test-report.yaml`, `test-cases.yaml`.
- **`assets/examples/agents/`**: Add `pm.agent.yaml`, `rd.agent.yaml`, `qa.agent.yaml`, `qa-critic.agent.yaml`.
- **`assets/examples/flows/software-dev-cycle/`**: New directory with `flow.yaml`.
- **`src/flow/flow_definition.{h,cc}`**: Parse new edge fields (`on_approved_reschedule`, `max_cycles`, `on_cycle_exhausted`, `reviewer_agents`); parse structured `llm.provider` + `llm.model` on agent YAML.
- **`src/flow/flow_runtime.{h,cc}`**: Port completion tracking; `on_approved_reschedule` trigger; `max_cycles` enforcement on edge pairs; `InvocationContext` construction and injection.
- **`src/flow/run_state.{h,cc}`** (new): Serialise/deserialise the extended `state.json` schema including per-agent port maps and invocation history.
- **`src/agent/agent_runtime.{h,cc}`**: Accept `InvocationContext` as initial message; thread `llm.provider` lookup through to the LLM client.
- **`src/llm/llm_provider_registry.{h,cc}`** (new): Named provider registry; `github-copilot` device-flow OAuth + token exchange; OS keychain integration for API keys.
- **No new third-party C++ deps** — `yaml-cpp` already vendored; keychain access via existing macOS Security framework.
- **Frontend**: Settings UI gains a provider-management panel (list, add, authenticate). Minimal scope — no React Flow canvas changes.
