## 1. Foundation: build system & dependencies

- [x] 1.1 Add `yaml-cpp` via CMake `FetchContent` (pinned to a tagged release); verify static link size impact
- [x] 1.2 Create `src/flow/` module with `CMakeLists.txt` wiring into `cronymax_app`
- [x] 1.3 Create `src/document/` module with `CMakeLists.txt` wiring into `cronymax_app`
- [x] 1.4 Add `~/.cronymax/builtin-doc-types/` install step to the macOS bundle (CMake install rule copying `assets/builtin-doc-types/`)
- [x] 1.5 Author 5 built-in doc-type YAML files: `prd.yaml`, `tech-spec.yaml`, `test-plan.yaml`, `code-description.yaml`, `freeform.yaml`

## 2. Workspace FS layout & migration scaffolding

- [x] 2.1 Define `WorkspaceLayout` helper that resolves `<root>/.cronymax/{flows,agents,doc-types,conflicts}/` paths
- [x] 2.2 First-touch initializer: create `.cronymax/` skeleton on Space activation if missing; write a `version: 1` marker file
- [x] 2.3 Add `.gitignore` defaults helper that suggests appending `runs/*/trace.jsonl` (opt-in dialog, never auto-edits user gitignore)
- [x] 2.4 Bridge channel `workspace.layout` returning the resolved paths for the renderer

## 3. YAML loaders & validators

- [x] 3.1 `DocTypeSchema` loader (`doc_type_schema.{h,cc}`): parse YAML, validate grammar, produce immutable schema object
- [x] 3.2 `AgentDefinition` loader (`agent_definition.{h,cc}`): parse `<agent>.agent.yaml`, validate required fields (`name`, `kind`, `llm`, `system_prompt`)
- [x] 3.3 `FlowDefinition` loader (`flow_definition.{h,cc}`): parse `flow.yaml`, validate agent references and port types against doc types
- [x] 3.4 Friendly error reporting: each loader returns `Result<T, LoadError>` with file path + line number when possible
- [x] 3.5 Unit tests for each loader: valid file passes, each missing-field case fails with a clear error

## 4. Per-Space registries

- [x] 4.1 `AgentRegistry` (`agent_registry.{h,cc}`): in-memory map keyed by agent name; populated from `.cronymax/agents/`
- [x] 4.2 `FlowRegistry` (`flow_registry.{h,cc}`): in-memory map keyed by flow name; populated from `.cronymax/flows/`
- [x] 4.3 `DocTypeRegistry` (`doc_type_registry.{h,cc}`): merges built-in types with workspace overrides
- [x] 4.4 Filesystem watcher (per Space): watch the three directories with `kqueue`/`FSEvents`; refresh registries on change with debounce
- [x] 4.5 Wire `SpaceManager::ActivateSpace()` to instantiate the three registries for the new Space and dispose for the old
- [x] 4.6 Bridge channels: `agent.registry.list`, `agent.registry.load`, `flow.list`, `flow.load`, `doc_type.list`

## 5. Document store

- [x] 5.1 `DocumentStore` (`document_store.{h,cc}`): write/read APIs over `.cronymax/flows/<flow>/docs/`
- [x] 5.2 Revision snapshot writer: every write also produces `docs/.history/<doc>.<rev>.md`; SHA recorded
- [x] 5.3 POSIX `flock`-based per-doc write lock; spike first to confirm APFS reliability (Decision/Open Q from `design.md`)
- [x] 5.4 Conflict diversion: if a write lock is held by an agent, external file changes detected by the watcher route to `.cronymax/conflicts/<doc>.<timestamp>.md`
- [x] 5.5 Bridge channels: `document.read`, `document.list`, `document.subscribe` (server-push on changes)
- [x] 5.6 Unit tests: revision integrity, lock contention, conflict diversion

## 6. Reviews subsystem

- [x] 6.1 `ReviewsState` data model + JSON (de)serializer matching the schema in `document-collaboration` spec
- [x] 6.2 `ReviewStore` (`review_store.{h,cc}`): atomic read-modify-write of `runs/<id>/reviews.json` under a per-file lock
- [x] 6.3 Schema validator reviewer (`schema_reviewer.{h,cc}`): deterministic, blocking; checks required sections / front-matter against `DocTypeSchema`
- [x] 6.4 Critic reviewer agent: ship a built-in `agents/critic.agent.yaml` + dedicated system prompt + structured JSON output schema (`{ comments: [...] }`)
- [x] 6.5 Reviewer pipeline runner (`reviewer_pipeline.{h,cc}`): validators → reviewer agents (parallel, with `reviewer_timeout_secs` AbortController) → optional human gate
- [x] 6.6 `max_review_rounds` enforcement with `on_review_exhausted: approve | halt`
- [x] 6.7 Bridge channels: `review.list`, `review.comment`, `review.approve`, `review.request_changes`
- [x] 6.8 Unit tests: validator failure short-circuits, reviewer timeout doesn't block, max_review_rounds ceiling honored

## 7. AgentRuntime evolution

- [x] 7.1 Add `agent_id`, `flow_run_id`, `memory_namespace` fields to `AgentRuntime`; thread through constructor
- [x] 7.2 Allow multiple `AgentRuntime` instances per Space (refactor any singleton assumptions); keyed by `(flow_run_id, agent_id)`
- [x] 7.3 Per-Agent message history isolation: confirm no shared state between instances
- [x] 7.4 Memory namespace plumbing: scope memory tool calls to `<agent-name>` namespace by default
- [x] 7.5 New built-in tool `submit_document(type, content, mentions?)`: validates port match, writes via `DocumentStore`, transitions doc to `IN_REVIEW`, ends loop
- [x] 7.6 Update `web/agent/loop.js` to recognize `submit_document` as a terminal tool
- [x] 7.7 Reject raw `fs.write` to paths under `.cronymax/{flows,agents,doc-types}/` and `runs/*/reviews.json` (enforces `space-manager` MODIFIED requirement)
- [x] 7.8 Remove deprecated bridge channels `agent.graph.*` from `BridgeHandler`; update renderer to not call them
- [x] 7.9 Update `docs/architecture.md` notes that `AgentGraph` is internal-only

## 8. FlowRuntime

- [x] 8.1 `FlowRunState` data model: id, status, started_at, agents_in_flight, documents{}, persisted to `runs/<id>/state.json`
- [x] 8.2 `FlowRuntime` (`flow_runtime.{h,cc}`): manages active Runs per Space; spawns/owns `AgentRuntime` instances for each declared Agent
- [x] 8.3 Run start: create run dir, write `state.json`, instantiate entry Agent(s) seeded with the user's initial input
- [x] 8.4 Run state persistence: write `state.json` after every status transition
- [x] 8.5 Run cancellation: abort all in-flight Agent loops + LLM requests; transition to `CANCELLED`
- [x] 8.6 Run rehydration on app start: scan all Spaces' `runs/`, find `PAUSED` runs, restore registry/document state, leave Agent loops un-restarted (v1 limitation)
- [x] 8.7 Bridge channels: `flow.run.start`, `flow.run.status`, `flow.run.cancel`
- [x] 8.8 Active-run pointer persisted in SQLite alongside Space metadata
- [x] 8.9 Integration test: start a single-Agent Flow, see it produce a doc, see Run complete

## 9. Routing engine

- [x] 9.1 `MentionParser` (`mention_parser.{h,cc}`): line-anchored `@\w+` parser; ignores fenced code blocks; returns positions + agent names
- [x] 9.2 Typed-port resolver: given producing Agent + submitted doc type, return list of downstream Agents per Flow edges
- [x] 9.3 Mention resolver: filter parsed mentions against the Flow's declared agents; emit warning event for unknown mentions
- [x] 9.4 `Router` (`router.{h,cc}`): combines typed-port + mention results; schedules downstream Agents in `FlowRuntime`
- [x] 9.5 Backward-routing support: ensure Router does not enforce DAG; an `@upstream` mention is allowed
- [x] 9.6 Unit tests: fan-out, no-match, mention-only adds extra recipient, code-block mention ignored, unknown mention warns

## 10. Trace event stream

- [x] 10.1 `TraceEvent` typed-event schema (one type per event listed in `flow-orchestration` spec)
- [x] 10.2 `TraceWriter`: appends to `runs/<id>/trace.jsonl` on a background thread with a write queue (so high-frequency events don't stall main)
- [x] 10.3 `TraceSubscriber`: bridge channel `event` (server-push) with replay-then-live semantics for late subscribers
- [x] 10.4 Wire all subsystems (FlowRuntime, AgentRuntime, ReviewerPipeline, Router) to emit their events
- [x] 10.5 Tool-call trace events tagged with Space id, Run id, Agent id, tool name, args (per `space-manager` ADDED requirement)

## 11. Renderer: minimal chat panel

- [x] 11.1 `web/flow/chat.html` + `chat.js`: subscribe to event stream, render typed events
- [x] 11.2 Document submission renders as a card (title, type, status, revision, comment count)
- [x] 11.3 User text input box; messages are emitted as `mention.user_input` bridge calls
- [x] 11.4 Server-side: parse `mention.user_input` for `@AgentName` and route to the Agent in the active Run
- [x] 11.5 Approve / Request-changes buttons on document cards (calls `review.approve` / `review.request_changes`)
- [x] 11.6 Comment list rendering for a doc (read-only in this change; comments come from reviewer agents)
- [x] 11.7 Run status header (running / paused-awaiting-review / completed / failed / cancelled) + cancel button

## 12. Examples & documentation

- [x] 12.1 Ship 2 example Flows under `assets/examples/flows/`: `simple-prd-to-spec` and `bug-fix-loop` (the latter demonstrates `@mention` backward routing)
- [x] 12.2 Ship example agents: `product.agent.yaml`, `architect.agent.yaml`, `coder.agent.yaml`, `critic.agent.yaml`
- [x] 12.3 First-Flow walkthrough doc at `docs/flows_quickstart.md` (hand-edit YAML, start Run, watch chat)
- [x] 12.4 Update `docs/multi_agent_orchestration.md` with the implemented YAML schemas (link to spec)
- [x] 12.5 Update `README.md` with a one-paragraph mention of Flows

## 13. Testing & validation

- [x] 13.1 Integration test: 2-Agent Flow (Product → Architect) with Schema reviewer, Run completes
- [x] 13.2 Integration test: 3-Agent Flow with Critic reviewer + human approval gate; verify pause / resume
- [x] 13.3 Integration test: `@mention` backward routing (Coder → @Product) reaches Product
- [x] 13.4 Integration test: `max_review_rounds` exhaustion both modes (`approve` and `halt`)
- [x] 13.5 Integration test: Run rehydration after app restart for a `PAUSED` Run
- [x] 13.6 Integration test: concurrent comment writes don't lose data (`reviews.json` lock)
- [x] 13.7 Manual QA: hand-edit a Flow YAML while a Run is paused; resume; verify no corruption
- [x] 13.8 Run `openspec validate agent-document-orchestration --strict` and address any findings

## 14. Cleanup & migration

- [x] 14.1 Remove `agent.graph.*` bridge channel handlers and any renderer call sites
- [x] 14.2 Update `space-agent-integration` archive notes to flag the multi-agent scope narrowing (if archived after this change starts)
- [x] 14.3 Confirm `cronymax_app` builds clean with `-Werror` after refactor
- [x] 14.4 Smoke test on a fresh workspace: app launches, creates `.cronymax/` skeleton, example Flow runs end-to-end
