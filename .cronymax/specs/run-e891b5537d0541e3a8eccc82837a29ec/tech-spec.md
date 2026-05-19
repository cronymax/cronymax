---
title: Tech Spec: Software Dev Cycle Preset Flow (PM → RD → QA)
doc_type: tech-spec
---

# Tech Spec: Software Dev Cycle Preset Flow (PM → RD → QA)

## Summary

This document describes the full technical implementation plan for the **software-dev-cycle** preset flow, a multi-agent PM → RD → QA pipeline built on top of cronymax's Flow engine. The preset ships as the canonical end-to-end demonstration of agentic orchestration: PM produces a prototype + PRD; RD produces a tech-spec, code-description, and submit-for-testing; QA runs test cases, files bug reports, receives patches, and emits a final test-report. All phase transitions are governed by per-port reviewer pipelines with AND-join gates and automatic agent re-invocation.

Areas of work span four layers:
1. **Runtime (C++/Rust)** — flow execution engine: re-invocation, AND-join, cycle counting, state persistence
2. **Agent context injection** — `InvocationContext` system message composition
3. **Frontend (TypeScript/React)** — Channel view event kinds, Flow Editor live highlighting, Document Workbench review actions
4. **Tooling** — `test_runner.*` tool suite for the QA agent

---

## Approach

### 1. Runtime: FlowRunner Engine

#### 1.1 Data Model

A flow run is persisted to `{workspace}/flows/{flow_id}/runs/{run_id}/state.json` with the following schema:

```json
{
  "run_id": "...",
  "flow_id": "software-dev-cycle",
  "status": "running",          // running | paused | completed | cancelled | needs_human
  "created_at": 1709000000000,
  "ports": {
    "pm-design:prototype": { "status": "approved", "doc_id": "...", "doc_path": "..." },
    "pm-design:prd":       { "status": "pending",  "doc_id": null },
    ...
  },
  "cycles": {
    "qa-testing:bug-report": 2
  }
}
```

`status` values per port: `waiting` | `in_review` | `approved` | `rejected` | `exhausted`.

`state.json` is written atomically (write-to-temp → rename) on every state transition. On startup the `FlowRunner` scans all `state.json` files and resumes any run whose `status == "running"` and has ports with `status == "waiting"` that have all upstream dependencies met.

#### 1.2 Agent Re-invocation

When a document port is approved:
1. Mark the port `approved` in `state.json`.
2. Check the owning node's next pending output port (in YAML declaration order).
3. If a next port exists, compose an `InvocationContext` (see §2) and enqueue a new agent task within **5 seconds** (configurable as `reinvocation_delay_ms`).
4. Emit a `handoff` event with `reason: "typed_port"`.

#### 1.3 AND-Join Gate

`qa-testing` has two upstream dependencies: `tech-spec` (from `rd-design`) and `submit-for-testing` (from `rd-design`). The runtime evaluates the AND condition before dispatching the QA agent:

```
gate(qa-testing) = ports["rd-design:tech-spec"].status == "approved"
               AND ports["rd-design:submit-for-testing"].status == "approved"
```

Both the `tech-spec` and `submit-for-testing` paths write `state.json` independently. The second one to become `approved` triggers the QA dispatch.

#### 1.4 Reviewer Pipeline

Each port declares an ordered list of reviewers in `flow.yaml`. The runtime processes them as an **ordered gate sequence**:
- **human**: waits for a `flow.run.approve` or `flow.run.request_changes` bridge call.
- **agent (critic / qa-critic / rd)**: the named agent is invoked with a `reviewer` role prompt; it calls `flow.run.approve` or `flow.run.request_changes` internally via the tool bridge.

A port advances to `approved` only when all declared reviewers have approved in sequence. `request_changes` from any reviewer increments the round counter and re-queues the producing agent with the reviewer's comments merged into `InvocationContext`. If `round > max_review_rounds` (default 3) the run halts with `status: needs_human`.

#### 1.5 Bug-Report Cycle Counting

`qa-testing:bug-report` is wired to `rd-patch` with `max_cycles: 5`. The runtime tracks `cycles["qa-testing:bug-report"]` in `state.json`. After `rd-patch` emits a `patch-note` that routes back to `qa-testing`, the cycle counter increments. When `cycles >= max_cycles` the port receives `status: exhausted` and the run transitions to `needs_human`.

#### 1.6 Run Lifecycle Events

| Transition | AppEvent emitted |
|---|---|
| Run started | `system { subkind: "run_started" }` |
| Run paused (awaiting human review) | `system { subkind: "run_paused", cause: "human_approval" }` |
| Run completed | `system { subkind: "run_completed" }` |
| Run cancelled | `system { subkind: "run_cancelled" }` |
| Escalated to human | `system { subkind: "run_paused", cause: "escalated" }` |
| Agent re-invoked | `agent_status { status: "thinking" }` then `handoff` |

---

### 2. InvocationContext Injection

Every time an agent is re-invoked within a run, the runtime prepends a structured system message:

```markdown
## FlowRuntime: Invocation Context

All required inputs for node `{node_id}` have been approved. Last approval: `{last_port}` from node `{source_node}`.

### Your Next Task
Submit a document of type: **{next_port}**

### Your Pending Ports (in order)
  1. {port_1}
  2. {port_2}
  ...

### Available Approved Documents
{list of: - `{doc_type}` from `{node_id}` — path: `{doc_path}` (revision {n})}

### Review Comments (if any)
{structured comments from prior reviewers}
```

This message is injected as the **first `system` message** in the agent's context window, after the agent's own system prompt. The agent tool `submit_document` reads its `doc_type` against `next_port` and fails fast if mismatched.

---

### 3. Frontend Changes

#### 3.1 Channel View — `error` Event Kind

The Channel `App.tsx` already handles all existing event kinds. One missing kind: **`error`** events are stored in `state.errors` but not rendered in the timeline. Add:

```tsx
if (e.kind === "error") {
  return <ErrorCard key={e.id} event={e} />;
}
```

`ErrorCard` shows code + message in a red-bordered card with an expand toggle for details.

#### 3.2 Channel View — Agent Status Pill

The current `agent_status` renderer is a plain text line. Replace with a `StatusBubble` component that:
- Shows a spinner animation for `thinking`
- Shows a blocked badge for `blocked`
- Shows a checkmark for `done`
- Includes agent name and elapsed time

#### 3.3 Flow Editor — Live Node Highlighting

`store.ts` already carries `runningId` and `doneId`. Wire them to the runtime event stream:

- Subscribe to `agent_status` events via the bridge using the `run_id` returned by `flowRun.start`.
- On `agent_status { status: "thinking" }`: dispatch `setHighlight({ running: nodeIdForAgent(agentId) })`.
- On `agent_status { status: "done" }`: dispatch `setHighlight({ done: nodeIdForAgent(agentId) })`.
- On `handoff` events: briefly animate the matching edge (CSS `animate-pulse` on the SVG path) for 2 seconds.

`nodeIdForAgent(agentId)` looks up the canvas `GraphNode` whose `config.agent_name` matches `agentId`.

Edge animation requires storing an `animatingEdgeIndex` in state, set on `handoff` event, cleared after a timeout.

#### 3.4 Flow Editor — Run Status in Toolbar

When `activeRunId` is set, display beside the Cancel button:
- A green "●  Running" pill when `system.subkind == "run_started"`.
- An amber "⏸  Paused" pill when `system.subkind == "run_paused"`.

#### 3.5 Document Workbench — Review Integration

`workbench/App.tsx` must wire the Approve / Request-Changes actions to `flowRun.approve` / `flowRun.requestChanges`. The Workbench URL params include `flow_run_id`, `node_id`, and `port` (passed by the Document Card's "Open in Workbench" link).

```ts
// workbench/url.ts additions
export function getReviewParams() {
  const p = new URLSearchParams(window.location.search);
  return {
    flow_run_id: p.get("flow_run_id") ?? "",
    node_id: p.get("node_id") ?? "",
    port: p.get("port") ?? "",
  };
}
```

The Workbench toolbar shows an **Approve** button (green) and a **Request Changes** button (amber) when review params are present. Clicking either calls `flowRun.approve` / `flowRun.requestChanges` and emits a toast. The Channel view reflects the resulting `review_event` within 2 seconds via the existing event subscription.

#### 3.6 Document Card — "Open in Workbench" Deep-link

`DocumentCard` already exists. Add a button that opens the workbench panel URL with `?doc_path=…&flow_run_id=…&node_id=…&port=…`. The `thread` prop carries `doc_path`, and the `flowId` / `runId` props carry the run context.

---

### 4. `test_runner` Tool Suite

Implemented as native tools registered in the agent entity layer (not via the Skills Marketplace per the PRD non-goal).

#### 4.1 `test_runner.discover`

Scans the workspace root for:
- `package.json` → check `scripts.test` for `jest` / `vitest`
- `vitest.config.*` → Vitest
- `pytest.ini` / `pyproject.toml [tool.pytest]` → pytest
- `go.mod` → Go test

Returns:
```json
{ "runners": ["vitest", "pytest"] }
```

#### 4.2 `test_runner.run_suite`

Parameters: `runner` (string, from discover output), `args` (optional string[]).

Execution:
1. For `vitest`: run `npx vitest run --reporter=json --outputFile=.cronymax/test-results/vitest.json`
2. For `jest`: run `npx jest --json --outputFile=.cronymax/test-results/jest.json`
3. For `pytest`: run `pytest --json-report --json-report-file=.cronymax/test-results/pytest.json`
4. For `go test`: run `go test ./... -json > .cronymax/test-results/gotest.jsonl`

If the JSON reporter config is absent (exit code 1 with no output file), return:
```json
{ "status": "reporter_not_configured", "runner": "vitest" }
```

Otherwise parse and return a `TestRunResult`:
```json
{
  "runner": "vitest",
  "passed": 42,
  "failed": 1,
  "skipped": 3,
  "duration_ms": 1234,
  "failures": [
    { "name": "AuthService > login fails with wrong password", "message": "expected false to be true" }
  ]
}
```

#### 4.3 `test_runner.get_last_report`

Returns the most recent `TestRunResult` written to `.cronymax/test-results/` by scanning modification timestamps. Returns `null` if no report exists.

---

### 5. LLM Provider Keychain Migration

The `ProvidersTab` currently stores `api_key` in the workspace SQLite KV store (per the caption text). Per AC-5 and the PRD, API keys must be stored in the macOS Keychain. This requires:

1. A new bridge command `llm.providers.get_secret(provider_id)` / `llm.providers.set_secret(provider_id, secret)` that reads/writes Keychain via `SecItemAdd` / `SecItemCopyMatching` with `kSecAttrService = "cronymax.llm"` and `kSecAttrAccount = provider_id`.
2. `providers.json` (persisted to disk) stores all fields **except** `api_key`. The `api_key` field is populated at runtime from Keychain on load and stripped before save.
3. On first launch after update, `legacy_importer` detects any existing `api_key` fields in the KV store, migrates them to Keychain, and wipes the KV record (idempotent: checks Keychain first).

Frontend changes to `ProvidersTab`: replace direct `api_key` field in the persist payload with a separate `shells.browser.llm.providers.setSecret({ provider_id, api_key })` call on Save.

---

## Key Decisions

| # | Decision | Rationale |
|---|---|---|
| 1 | `state.json` written atomically to `{workspace}/flows/{flow_id}/runs/{run_id}/` | Survives app restarts; isolated per run; atomic rename avoids corrupt reads |
| 2 | Agent reviewer pipeline is **sequential**, not parallel | Prevents reviewer contention; earlier reviewers (human) set context for later ones (Critic) |
| 3 | InvocationContext injected as **system message** before user turn | Keeps agent system prompt stable; context is always fresh and per-invocation |
| 4 | AND-join implemented at the dispatch site (not a separate graph node) | The flow.yaml already captures the topology; a separate join node would add YAML verbosity with no user benefit |
| 5 | `test_runner` tools are agent-layer tools, not Skills Marketplace | Explicit PRD non-goal; avoids migration complexity |
| 6 | Keychain stores only the secret; `providers.json` stores all other fields | Secrets never on disk; human-readable config file is diffable and portable |
| 7 | Edge animation uses CSS `animate-pulse` on SVG path, cleared after 2s | Lightweight; no animation library needed |
| 8 | QA agent `test_runner` usage is optional / graceful | `reporter_not_configured` status allows QA agent to proceed with manual test documentation if automation is unavailable |

---

## Testing Strategy

### Unit Tests

- **FlowRunner state machine**: table-driven tests covering all port status transitions, AND-join triggering, cycle counting, and exhaustion escalation.
- **InvocationContext builder**: snapshot tests verifying correct system message for each phase (prototype, prd, tech-spec, code-description, submit-for-testing, bug-report, patch-note).
- **test_runner.discover**: mock workspace fixtures with various config file combos.
- **test_runner.run_suite**: mock subprocess execution; test JSON parse for each runner type; test `reporter_not_configured` path.
- **Keychain migration**: mock SecItem API; verify idempotency on double-run.

### Integration Tests (Rust runtime)

- Simulate a full PM → RD → QA run using mock agents that auto-approve every port.
- Assert `state.json` reflects correct port statuses after each phase.
- Assert QA is dispatched only after both `tech-spec` AND `submit-for-testing` are approved.
- Assert bug-report cycle is capped at 5 and run transitions to `needs_human`.
- Kill the process mid-run and restart; assert the run resumes from the correct port.

### Component Tests (React Testing Library)

- `Channel App.tsx`: render all 10 event kinds and assert correct components are mounted.
- `DocumentCard`: assert "Open in Workbench" link carries correct params.
- `ProvidersTab`: assert `setSecret` is called on Save for GitHub Copilot providers; assert `api_key` absent from `raw` JSON.
- `FlowEditor`: assert `animate-pulse` class appears on correct edge SVG after injecting a mock `handoff` event.

### E2E (Playwright / Tauri WebDriver)

- Launch the app, navigate to the Flows panel, start a `software-dev-cycle` run with mock agents.
- Verify Channel view shows `run_started` system event.
- Verify Flow Editor shows animated node for the active agent.
- Approve the prototype document via the Document Card; verify PM agent is re-invoked for PRD.
- Cancel the run; verify `run_cancelled` system event and run status badge updates.

### Manual Acceptance Checklist (QA gate)

- All AC-1 through AC-9 items from the PRD exercised on a fresh workspace.

---

## Migration Plan

### State Schema Migration

No existing `state.json` files exist in production (feature is new). No migration needed.

### LLM Provider Keychain Migration

1. On first launch after update, `LegacyImporter::MigrateProviderSecrets()` runs synchronously before the UI renders.
2. For each provider in the KV store whose `api_key` is non-empty:
   a. Check if Keychain already has a secret for `provider_id` → skip if present (idempotency).
   b. Write secret to Keychain via `SecItemAdd`.
   c. Set `api_key = ""` in the KV store record.
3. Log migration results (count migrated, count skipped) to the app log at `INFO` level.
4. On failure (Keychain permission denied): leave the KV record untouched; surface a toast prompting the user to re-enter the API key in Settings.

### Flow YAML Compatibility

The `software-dev-cycle/flow.yaml` already exists in the repository. The `FlowEditor` init code already seeds `SEED_SOFTWARE_DEV_CYCLE_FLOW` into `localStorage` for new installs. For existing installs that already have a `software-dev-cycle` key in `localStorage`, the seed step is skipped (non-destructive).
