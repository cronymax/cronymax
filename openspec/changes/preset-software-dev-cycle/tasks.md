## 1. Built-in Document Types

- [x] 1.1 Add `assets/builtin-doc-types/prototype.yaml` (required sections: Screens, User Flows, Interactions; optional: Design Tokens, Open UX Questions, Mockups; front_matter: author, owner_agent, source_prd)
- [x] 1.2 Add `assets/builtin-doc-types/submit-for-testing.yaml` (required sections: What Was Built, Test Setup, In Scope, Known Limitations; front_matter: author, owner_agent, source_code_description)
- [x] 1.3 Add `assets/builtin-doc-types/bug-report.yaml` (required sections: Steps to Reproduce, Expected, Actual, Severity; optional: Reproduction Rate, Affected Tests; front_matter: author, owner_agent)
- [x] 1.4 Add `assets/builtin-doc-types/patch-note.yaml` (required sections: What Changed, Files Modified, How to Verify; front_matter: author, owner_agent, source_bug_report)
- [x] 1.5 Add `assets/builtin-doc-types/test-report.yaml` (required sections: Summary, Pass Rate, Open Bugs, Ship Recommendation; front_matter: author, owner_agent)
- [x] 1.6 Register all five new types in the built-in doc-type loader so they're available in any workspace without a `doc-types/` directory

## 2. Preset Agent YAML Files

- [x] 2.1 Add `assets/examples/agents/pm.agent.yaml` (kind: producer; llm: `{provider: copilot, model: gpt-4o}`; tools: submit_document, mention; system prompt: prototype → PRD → awaits approval between phases)
- [x] 2.2 Add `assets/examples/agents/rd.agent.yaml` (kind: producer; llm: `{provider: copilot, model: gpt-4o}`; tools: submit_document, mention, terminal.execSandboxed; system prompt: tech-spec → implement → code-description → submit-for-testing → patch-note on bug-report)
- [x] 2.3 Add `assets/examples/agents/qa.agent.yaml` (kind: producer; llm: `{provider: openai, model: gpt-4o-mini}`; tools: submit_document, mention, test_runner.discover, test_runner.run_suite, test_runner.get_last_report; system prompt: test-cases from tech-spec → run tests → bug-report or test-report → re-test on patch-note)
- [x] 2.4 Add `assets/examples/agents/qa-critic.agent.yaml` (kind: reviewer; llm: `{provider: openai, model: gpt-4o-mini}`; system prompt: review for testability gaps, respond with structured JSON comments)

## 3. Preset Flow YAML

- [x] 3.1 Create `assets/examples/flows/software-dev-cycle/` directory
- [x] 3.2 Write `flow.yaml` with all Phase 1 edges (pm → prototype [no downstream, human approval], pm → rd via prd [human approval, on_approved_reschedule])
- [x] 3.3 Add Phase 2 edges (rd → qa via tech-spec [human approval, reviewer_agents: [critic, qa-critic], on_approved_reschedule]; rd → code-description [human approval, reviewer_agents: [critic], on_approved_reschedule]; rd → qa via submit-for-testing [no review gate])
- [x] 3.4 Add Phase 3 edges (qa → test-cases [reviewer_agents: [rd, critic]]; qa → rd via bug-report [max_cycles: 5, on_cycle_exhausted: escalate_to_human]; rd → qa via patch-note; qa → test-report [human approval, reviewer_agents: [critic]])
- [x] 3.5 Validate `software-dev-cycle/flow.yaml` loads without errors in the FlowRuntime (write a manual test or loading integration test)

## 4. Flow Edge Schema Extensions (flow_definition.h/cc)

- [x] 4.1 Add `on_approved_reschedule: bool` field to `FlowEdge` struct; parse from YAML
- [x] 4.2 Add `reviewer_agents: []string` field to `FlowEdge` struct; parse from YAML; validate all referenced agents are declared in the Flow
- [x] 4.3 Add `max_cycles: int` and `on_cycle_exhausted: enum {escalate_to_human, halt}` fields to `FlowEdge` struct; parse from YAML
- [x] 4.4 Update `FlowDefinition::Validate()` to check per-edge reviewer names are declared agents

## 5. Port Completion Tracking (run_state.h/cc — new files)

- [x] 5.1 Define `RunAgentState` struct: `ports` map (port name → `PortStatus {PENDING, IN_REVIEW, APPROVED}`), `invocations` list (id, trigger kind/port/doc, status)
- [x] 5.2 Define extended `RunState` struct: existing fields + `agents` map (agent name → `RunAgentState`)
- [x] 5.3 Implement `RunState::Serialize()` / `RunState::Deserialize()` using hand-written `JsonValue`
- [x] 5.4 Add `RunState::MarkPortStatus(agent, port, status)` with atomic `state.json` write (flock + rename)
- [x] 5.5 Add `RunState::RecordInvocation(agent, InvocationTrigger)` appending to invocation history with atomic write
- [x] 5.6 Write unit tests for `RunState` serialization round-trip and port-status idempotency

## 6. InvocationContext Envelope (flow_runtime.h/cc)

- [x] 6.1 Define `InvocationContext` struct: `trigger` (kind enum + port + doc path), `available_docs` list (path, type, rev), `pending_ports` list (ordered), `system_message` string
- [x] 6.2 Implement `FlowRuntime::BuildInvocationContext(agent, trigger, run_state)` that constructs the context and renders the `system_message`
- [x] 6.3 Modify `FlowRuntime::ScheduleAgent()` to prepend the InvocationContext as a system-role message in the `AgentRuntime` initial message list
- [x] 6.4 Emit `agent.scheduled` trace event with trigger metadata and pending_ports list

## 7. on_approved_reschedule Logic (flow_runtime.h/cc)

- [x] 7.1 Implement `FlowRuntime::OnDocumentApproved(doc, port, producing_agent)` handler
- [x] 7.2 In `OnDocumentApproved`: update port map to APPROVED; check `on_approved_reschedule` flag
- [x] 7.3 Add idempotency guard: if port is already APPROVED in `state.json` when `OnDocumentApproved` fires, skip scheduling
- [x] 7.4 Write integration test: simulate tech-spec approval → assert RD is re-scheduled with correct InvocationContext

## 8. max_cycles Enforcement (flow_runtime.h/cc)

- [x] 8.1 Implement per-edge cycle counter in RunState
- [x] 8.2 In `FlowRuntime::RouteDocument()`: before routing on an edge with `max_cycles`, increment and check the counter
- [x] 8.3 Implement `escalate_to_human` action: transition Run to `PAUSED`, emit `flow.run.cycle_exhausted` event, push inbox notification
- [x] 8.4 Write unit test: submit bug-report 5 times → assert Run transitions to PAUSED on the 5th

## 9. Per-edge reviewer_agents (flow_runtime.h/cc)

- [x] 9.1 In `FlowRuntime::StartReviewPipeline()`: if the triggering edge has a non-empty `reviewer_agents` list, use it instead of the Flow-level reviewer set
- [x] 9.2 If `reviewer_agents` is an empty list `[]` on the edge, skip LLM reviewer scheduling entirely
- [x] 9.3 Write unit test: submit doc on edge with `reviewer_agents: [qa-critic]` → assert only `qa-critic` is scheduled

## 10. Agent YAML — Structured llm Field

- [x] 10.1 Update `AgentDefinition` parser (`src/flow/flow_definition.cc`) to accept both `llm: <string>` (flat, legacy) and `llm: {provider: <id>, model: <name>}` (structured)
- [x] 10.2 For flat string form, substitute `default_provider` from the provider registry at agent load time
- [x] 10.3 Thread `provider_id` through to `AgentRuntime` so it can select the right provider for LLM calls

## 11. LLM Provider Registry (llm_provider_registry.h/cc — new files)

- [x] 11.1 Define `LlmProvider` struct: id, kind (openai_compat | github_copilot | none), base_url, model_override (optional)
- [x] 11.2 Implement `LlmProviderRegistry`: load from `~/.cronymax/providers.json`; support add, update, remove; persist atomically; maintain `default_provider` pointer
- [x] 11.3 Implement `LlmProviderRegistry::GetToken(provider_id)` → retrieves API key / OAuth token from macOS Security framework keychain (SecKeychainItemRef or `SecItemCopyMatching`)
- [x] 11.4 Implement `LlmProviderRegistry::StoreToken(provider_id, token)` → stores to keychain under item `cronymax-provider-<id>`
- [x] 11.5 Wire `LlmProviderRegistry` into the existing LLM client so it resolves `base_url` + token per call

## 12. GitHub Copilot OAuth (llm_provider_registry.h/cc)

- [x] 12.1 Implement `StartDeviceFlow(client_id)` → `POST /login/device/code`; return device_code, user_code, verification_uri, expires_in, interval
- [x] 12.2 Implement `PollDeviceFlow(device_code, interval)` → poll `POST /login/oauth/access_token` until granted or expired; return GitHub access token
- [x] 12.3 Implement `ExchangeForCopilotToken(github_token)` → POST to Copilot token exchange endpoint; return Copilot API token + expiry
- [x] 12.4 Store Copilot token in keychain; store GitHub token separately for refresh use
- [x] 12.5 Implement transparent token refresh: before each LLM call with `github-copilot` provider, check expiry; if expired, call `ExchangeForCopilotToken` with stored GitHub token; update keychain

## 13. Legacy Config Migration

- [x] 13.1 On app launch, check for existence of old single-key LLM config (`{base_url, api_key}` in user prefs or SQLite)
- [x] 13.2 If found: create a provider entry named `default` in the new registry; store API key in keychain; set it as `default_provider`; delete old config entry
- [x] 13.3 Migration is idempotent: if provider `default` already exists, skip
- [x] 13.4 Write migration test: populate old config → run migration → assert registry has `default` provider and keychain entry exists

## 14. test_runner.\* Built-in Tools

- [x] 14.1 Define `TestRunnerResult` struct in C++ (or as a typed JSON schema in the bridge layer): total, passed, failed, skipped, duration_ms, failures[], coverage?
- [x] 14.2 Implement `test_runner.discover` tool: scan workspace for `package.json` (jest/vitest), `setup.cfg`/`pyproject.toml` (pytest), `*_test.go` (go test); return typed suite list
- [x] 14.3 Implement `test_runner.run_suite` for Jest: exec `npx jest --json --testPathPattern=<filter>`; parse JSON reporter output into `TestRunnerResult`
- [x] 14.4 Implement `test_runner.run_suite` for Vitest: exec `npx vitest run --reporter=json`; parse output (note: field name differences from Jest — verify during impl)
- [x] 14.5 Implement `test_runner.run_suite` for Pytest: exec `pytest --json-report --json-report-file=-`; parse output
- [x] 14.6 Implement `test_runner.run_suite` for Go test: exec `go test -json ./...`; parse streaming JSON lines
- [x] 14.7 Implement `test_runner.get_last_report`: retrieve the most recent `TestRunnerResult` stored in-memory per Flow Run; return null if no prior run
- [x] 14.8 Register all `test_runner.*` tools in the tool registry; enforce producer-only restriction (skip registration for reviewer agents with a logged warning)
- [x] 14.9 Write unit tests for each runner parser using fixture JSON outputs

## 15. Settings UI — Provider Management Panel

- [x] 15.1 Add a "LLM Providers" section to the settings panel (React, minimal scope)
- [x] 15.2 List configured providers with name, kind, and status (`authenticated` | `configured` | `unconfigured`)
- [x] 15.3 "Add Provider" flow: form for name, kind selector, base_url (for openai-compat), API key entry (masked); on save, call `llm.provider.add` bridge channel
- [x] 15.4 "Connect Copilot" flow: button triggers device flow; show user_code + verification_uri in modal; poll for completion; auto-close modal on success
- [x] 15.5 "Remove Provider" action with confirmation dialog
- [x] 15.6 Expose bridge channels: `llm.provider.list`, `llm.provider.add`, `llm.provider.remove`, `llm.provider.auth_start` (device flow), `llm.provider.auth_status`
