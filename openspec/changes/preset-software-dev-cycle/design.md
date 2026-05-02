## Context

`agent-document-orchestration` ships the Flow engine, document lifecycle, reviewer pipeline, and a two-example preset set (`simple-prd-to-spec`, `bug-fix-loop`). The engine is complete but no opinionated multi-role workflow exists. This change adds the `software-dev-cycle` preset — a PM → RD → QA flow — and the three engine extensions it requires: agent re-invocation on approval, per-edge reviewer configuration, and a named multi-provider LLM registry.

The key constraint throughout: this change MUST NOT modify the existing `AgentRuntime` ReAct loop interface. All new behaviour is layered into `FlowRuntime` (orchestration) and `LlmProviderRegistry` (credentials). The agent YAML format change (structured `llm` field) is additive with a backwards-compat fallback.

**Existing constraints carried forward:**

- C++20, exceptions disabled; no nlohmann JSON (hand-written `JsonValue`).
- `yaml-cpp` already vendored for YAML parsing.
- CEF desktop app on macOS; `AgentRuntime` instances live in the renderer; `FlowRuntime` lives in the C++ host.
- OS Keychain (macOS Security framework) available for secret storage.
- `state.json` written atomically (tmp + `std::filesystem::rename` + POSIX `flock`).

**Stakeholders:** Preset users (copy-paste to start); engine consumers (`agent-document-orchestration` implementors); future Skills Marketplace (`test_runner.*` will migrate there).

## Goals / Non-Goals

**Goals:**

- Ship `software-dev-cycle` preset with PM, RD, QA, QA-Critic agents and all required doc types.
- Extend FlowRuntime with `on_approved_reschedule`, `max_cycles`, per-edge `reviewer_agents`, port-completion tracking in `state.json`, and InvocationContext envelope injection.
- Replace single global LLM config with a named multi-provider registry; add `github-copilot` device-flow OAuth and `openai-compat` API-key kinds.
- Add `test_runner.*` built-in tools to the agent-entity layer.

**Non-Goals:**

- Visual editor changes for new edge fields (deferred → `agent-orchestration-ui`).
- Skills Marketplace migration of `test_runner.*` (explicitly deferred).
- WYSIWYG `prototype` rendering with Mermaid (deferred → `document-wysiwyg`).
- Multi-human collaborative workflows.
- Windows / Linux support.

## Decisions

### Decision 1: Port completion state lives in state.json, not a separate file

**Chosen**: Extend the existing `state.json` with an `agents` map. Each agent entry has `ports` (map of port name → `PENDING | IN_REVIEW | APPROVED`) and `invocations` (ordered list with trigger metadata).

**Alternatives**:

- Separate `port-state.json` — extra file, extra lock, no benefit; `state.json` is already the Run's source of truth.
- In-memory only — breaks restart recovery; the "Run state survives restart" requirement already mandates persistence.

**Rationale**: One atomic write per transition. Port map doubles as a deduplication guard for `on_approved_reschedule` idempotency.

---

### Decision 2: Next pending port determined by flow.yaml declaration order

**Chosen**: When `on_approved_reschedule` fires, FlowRuntime finds the first port in the producing agent's `ports` map with status `PENDING`, where order is the order of edge declarations in `flow.yaml`.

**Alternatives**:

- Explicit `sequence:` field on the agent YAML — more expressive but adds a config surface that must be kept in sync with flow edges; fragile.
- LLM decides next task — Option A from exploration; rejected as non-deterministic and fragile.
- Human re-trigger between phases — Option C from exploration; valid for v1 but removes automation value of the preset.

**Rationale**: Declaration order is already meaningful (it's how users reason about the flow when editing YAML). No new config surface needed.

---

### Decision 3: InvocationContext injected as first system message

**Chosen**: FlowRuntime prepends a `{ role: "system", content: "<rendered context>" }` message to the agent's initial message history before passing it to `AgentRuntime`. The rendered content includes: what was approved/changed, the next task (next pending port), and a list of available approved docs in the Run.

**Alternatives**:

- New field on `AgentRuntime::start()` — cleaner API but requires changing the `AgentRuntime` interface, which we want to avoid.
- Tool call response — agent would need to call a `get_context` tool first; roundabout.

**Rationale**: System messages are the standard LLM mechanism for injecting authoritative context. Prepending to history preserves the `AgentRuntime` interface contract completely.

---

### Decision 4: LLM provider registry stored in ~/.cronymax/providers.json; secrets in OS keychain

**Chosen**: `~/.cronymax/providers.json` stores provider metadata (id, kind, base_url, default_provider flag). API keys and OAuth tokens stored exclusively in the macOS Security framework keychain under item name `cronymax-provider-<id>`. Registry loaded at app start; written atomically on any change.

**Alternatives**:

- SQLite — overkill for a flat config with <10 entries.
- Workspace-scoped config — providers contain user credentials; should not be committed to git.
- Env vars — not persistent; poor UX for desktop app.

**Rationale**: JSON file for non-secret config is editable by power users. OS keychain for secrets is the macOS standard; avoids ever writing keys to disk in plaintext. App-private path (`~/.cronymax/`) keeps providers consistent across workspaces.

---

### Decision 5: GitHub Copilot uses device flow, not redirect OAuth

**Chosen**: GitHub device flow (`POST /login/device/code` → poll `/login/oauth/access_token` → exchange for Copilot token).

**Alternatives**:

- Redirect OAuth with localhost callback server — requires managing a dynamic port, browser redirect, CSRF token; complex for a desktop app.
- Better Auth sidecar — overkill for scope (one OAuth provider + API keys); adds a Node.js process dependency.
- Personal access token (PAT) — no OAuth dance; user experience worse than device flow; PAT scopes are broader than needed.

**Rationale**: Device flow is the standard for desktop/CLI apps authenticating to GitHub. No browser popup, no port management. UX: show a code + URL in a settings modal → user approves in browser → modal auto-closes.

---

### Decision 6: test_runner.\* implemented as a thin shell-exec wrapper per runner

**Chosen**: `test_runner.run_suite` executes the appropriate test command (`npx jest --json`, `pytest --json`, `go test -json`, `vitest run --reporter=json`) as a sandboxed subprocess, parses the JSON reporter output, and maps it to the structured result schema. Each runner has a parser registered at compile time.

**Alternatives**:

- Single generic parser on raw stdout — fragile; test output formats are not standardised.
- Native test runner library bindings — impractical in C++; each runner is a different ecosystem.
- Delegate entirely to `terminal.execSandboxed` and let LLM parse — rejected; defeats the purpose of structured output.

**Rationale**: JSON reporter output is stable across minor runner versions. Thin per-runner parsers are easy to test and extend. `go test -json` and `pytest --json` are already machine-readable. Jest/Vitest have had `--json` for years.

---

### Decision 7: per-edge reviewer_agents is an override, not an additive list

**Chosen**: When `reviewer_agents:` is present on an edge, it completely replaces the Flow-level reviewer agent set for that edge. An empty list `[]` means no LLM reviewers for that edge.

**Alternatives**:

- Additive (merge with global) — surprising when trying to remove a global reviewer from a specific edge.
- Named exclude list — less readable.

**Rationale**: Override semantics are explicit and predictable. Users who want additive can list global + edge-specific reviewers explicitly.

## Risks / Trade-offs

- **State.json write contention** → Multiple agent completions in a parallel run could race on `state.json`. Mitigation: existing POSIX `flock` + atomic-rename pattern extended to port-state writes. Port map updates are small (single field change); lock hold time is negligible.

- **InvocationContext message length** → For long-running Runs with many approved docs, the available_docs list in the context message grows. Mitigation: list only doc path, type, and revision — not full content. Full content is injected via the existing document handoff mechanism.

- **Device flow UX timing** → GitHub polls require the user to act in a browser within 5 minutes. Mitigation: settings modal shows a countdown and re-launches the flow if the user misses the window.

- **test_runner JSON reporters not always available** → Older project setups may not have JSON reporters configured. Mitigation: `test_runner.discover` surfaces which reporters are available; `test_runner.run_suite` returns a clear `reporter_not_configured` error so the QA agent can fall back to `terminal.execSandboxed` and note limitations in the bug-report.

- **prototype doc type without WYSIWYG** → In v1 (before `document-wysiwyg`), Mermaid diagrams in prototype docs render as raw fenced code blocks. Not a blocker; the doc is still valid and reviewable in source view. The preset ships now; richer rendering comes with `document-wysiwyg`.

## Migration Plan

1. `assets/builtin-doc-types/`: add 5 new YAML files — no migration needed (additive).
2. `assets/examples/agents/` and `assets/examples/flows/`: add new files — no migration needed.
3. `agent.yaml` structured `llm` field: flat string form still accepted (backwards-compat); existing agent files continue to work using `default_provider`.
4. `state.json` schema extension: new `agents` map is optional at read time; existing Runs without it are treated as having all ports PENDING (safe for Runs that predate this change).
5. LLM provider config migration: on first launch after this change, the existing single global `{base_url, api_key}` config is read and automatically migrated into a named provider entry `default` in the new registry. API key is moved to the keychain. Old config key is removed.

## Open Questions

- None blocking implementation. The `test_runner` parser for `vitest` vs `jest` JSON output format differences should be verified during implementation — both claim `--json` but field names differ slightly.
