# Multi-Agent Runtime Refactor — Design & Decisions

> Captured from a three-session `/opsx:explore` based on
> [multi-agent.wiki](https://multi-agent.wiki/) pattern research.
> Supersedes the workflow-engine model described in
> [`multi_agent_orchestration.md`](multi_agent_orchestration.md).
> Feeds into future OpenSpec changes for `cronygraph` async primitives,
> loop-level critic, and chat-as-supervisor.

---

## 1. Current runtime mapped to the wiki taxonomy

```
  Wiki Dimension            What We Have                  Where It Lives
  ─────────────────────────────────────────────────────────────────────
  Control structure     →   Graph/State Machine           FlowRuntime
                            (implicit Supervisor)          + flow.yaml

  Information flow      →   Blackboard-ish                flow documents
                            (untyped, unscoped)            (markdown files)

  Decision making       →   Sequential Pipeline           AND-join + port
                            (no LLM routing)              routing (static)

  Human-in-the-loop     →   PendingReview + Approval      RuntimeAuthority
                            (gate exists, strong)         + review.approve

  Generator-Critic      →   Implicit (reviewer agents)    flow.yaml roles
                            (no first-class type)

  Parallel Fan-out      →   AND-join gather (reactive)    FlowRuntime
                            (no active concurrency)

  Trace / Observe       →   TraceKind + middleware         TraceEmitter +
                            (two separate systems)         FlowTrace

  Protocol              →   GIPS (control/events/caps)    crony + handler
```

---

## 2. The fundamental tension — two orchestration engines

There are two orchestration engines that never talk to each other:

```
  cronygraph::Orchestrator            FlowRuntime
  ────────────────────────            ──────────────────────────
  synchronous Step trait    vs.       async tokio scheduling
  generic S, E types        vs.       FlowRunState concrete type
  owns graph traversal      vs.       owns graph traversal too
  Branch → sequential       vs.       AND-join → parallel (reactive)
  no persistence            vs.       JSON state per run on disk
  no trace                  vs.       TraceEvent emission
  no agent context          vs.       InvocationContext rich struct
  not used in production    vs.       this is what runs flows
```

`cronygraph` was built to be "the engine" but `FlowRuntime` was built because
`cronygraph`'s sync generic shape couldn't handle async agent execution. The
result is scaffolding that scaffolds nothing — all real work is in `FlowRuntime`
which reinvents the graph.

---

## 3. Five structural gaps (from the wiki production checklist)

```
  ┌─────────────────────────────────────────────────────────────────┐
  │  Production Runtime Checklist (multi-agent.wiki)                │
  │                                                                 │
  │  ✓  App Server / Session API        →  GIPS control surface     │
  │  ✓  Agent Registry                  →  agent.yaml + AgentRunner │
  │  ✓  Event Log / Trace               →  RuntimeAuthority emit    │
  │  ✓  Guardrails / HITL               →  PendingReview + resolve  │
  │                                                                 │
  │  ✗  Orchestrator (plan/route/sched)  →  SPLIT across two layers │
  │  ✗  Task Registry                   →  MISSING                  │
  │  ✗  Blackboard / Context Store      →  SIMULATED via doc files  │
  │  ✗  Workspace isolation per agent   →  MISSING                  │
  │  ✗  Protocol Gateway (MCP/A2A)      →  stub only               │
  └─────────────────────────────────────────────────────────────────┘
```

### Gap 1 — `cronygraph::Orchestrator` is synchronous in an async world

```rust
// Today — Step::execute can never be called for real agents
pub trait Step<S, E>: Send + Sync {
    fn execute(&self, state: S) -> Result<Transition<S>, E>;
//                                ^^^^^^^^^^^^^^^^^^^^^^^^^^^^
//                                Can't await LLM, can't pause for approval,
//                                can't spawn concurrent tasks
}
```

Every real agent step needs to await an LLM call, await tool dispatch, await
approval resolution, and optionally spawn concurrent child tasks. Because this
was impossible, `FlowRuntime` duplicated the graph walking logic in async Tokio,
making `cronygraph::Orchestrator` dead code.

### Gap 2 — No Task Registry → no work tree, no per-task retry

The wiki's production runtime requires a formal `Task` type:

```
Task { id, parentId, sessionId, assignedAgent, goal, status, input, output }
```

We have `Run` (top level) and `FlowRunState.node_states` (port bitmaps per node)
but nothing between that answers: _Which agent is working on which goal? If one
node failed, what was its parent task? Can we retry just that leaf?_

The activity panel and flow trajectory UI currently reconstruct this by piecing
together events — a symptom of the missing underlying model.

### Gap 3 — No Blackboard → agents share context too broadly

Current agent context flows via `InvocationContext { available_docs: Vec<AvailableDoc> }`.
All documents from the run are visible to every agent. The wiki says:
"context passed is the minimum required, not the full history."

A Coder agent can accidentally see the PRD that was only meant for the
Architect. There is no declaration of data dependencies, no access enforcement,
no provenance on who wrote what.

### Gap 4 — No formal Supervisor → flow graph IS the supervisor but it's static

`flow.yaml` is a pre-authored execution plan. This means:

- No dynamic re-routing after a node fails
- No LLM-driven plan decomposition (the wiki's Hierarchical Decomposition)
- No confidence-based retries

The `@mention` routing is an escape hatch from the static graph, not a
first-class planning capability.

### Gap 5 — Two trace systems with no shared span identity

```
crates/cronymax/src/flow/trace.rs
    TraceKind { NodeInvoked, NodeSubmitted, ReviewOpened, ... }
    → written to disk per run, flow-specific

crates/cronymax/src/runtime/middleware.rs
    TraceEmitterMiddleware → RuntimeAuthority::emit_for_run
    → inner-loop events (token stream, tool calls, LLM turns)
```

These don't share a schema, don't share span IDs, and can't be correlated into
a single trace tree. The wiki recommends `traceId + spanId + parentSpanId`
for tree rendering:

```
  Session
  └── FlowRun (flow_run_id)
       ├── Node: ProductAgent    → workflow.node.enter
       │    ├── ReactLoop turn 1
       │    │    ├── tool.call.started: read_file
       │    │    └── tool.call.completed: read_file
       │    └── agent.result.received
       └── Node: ArchitectAgent  (after AND-join)
            └── ...
```

---

## 4. Target architecture — the two-tier model

### Core decision: Supervisor lives only at the Chat level

```
  ┌─────────────────────────────────────────────────────────────────────────┐
  │  LEVEL 0: Chat (Supervisor)                                             │
  │    Model: user-chosen in the chat toolbar (already per-request)         │
  │    Tools: invoke_agent, invoke_flow  +  existing flow.* tools           │
  │    Behavior: adaptive, sequential, plans, synthesizes                   │
  │    Human: always watching — it's the chat UI                            │
  │                                                                         │
  │          ↓ invokes                          ↓ invokes                  │
  │                                                                         │
  │  LEVEL 1A: Agents                   LEVEL 1B: Flows                     │
  │    (leaf executors)                   (deterministic pipelines)          │
  │    agent.yaml defines role            flow.yaml defines graph            │
  │    ReactLoop + tools                  FlowRuntime + AND-join             │
  │    CriticConfig for docs              CriticConfig per-agent inside      │
  │    Model: from agent.yaml             Agents use their own agent.yaml    │
  │    No sub-invocation                  Parallel via AND-join              │
  │    Depth: 1                           Depth: 1                           │
  └─────────────────────────────────────────────────────────────────────────┘

  Maximum hierarchy depth = 2.
  Parallelism happens inside Flows (AND-join), not at the Chat level.
  Flows are always deterministic — no planning inside flow.yaml.
```

Rationale: no `kind: supervisor` in `flow.yaml` nodes. Flow stays deterministic
and testable. The chat session's `__chat__` agent IS the supervisor. The user
already picks its model in the model-selector toolbar, so "Supervisor model
choice" requires zero new UI — it's the existing chat model picker.

### Full execution diagram

```
  User: "Build the auth module"
      │
      ▼
  Chat Supervisor — claude-opus-4-5 (user-picked):
  ┌─────────────────────────────────────────────────────────────────────┐
  │ Turn 1: invoke_agent("planner", "design auth API schema")           │
  │   → AWAITS  (SpawnsAgent — see §5)                                  │
  │   → planner ReactLoop runs internally:                              │
  │       turns: read_file, reason, write_doc                           │
  │       [CriticPhase] for doc artifact: LLM reviews schema            │
  │       1 revision → schema passes                                    │
  │   → planner writes "api_schema" to Blackboard                       │
  │   → Supervisor resumes with: api_schema                             │
  │                                                                     │
  │ Turn 2: invoke_flow("implementation-pipeline", input=api_schema)    │
  │   → AWAITS  (same SpawnsAgent mechanism)                            │
  │   → flow runs internally in parallel:                               │
  │       coder  reads [api_schema] → writes [impl_code]  ──┐           │
  │       tester reads [api_schema] → writes [test_suite] ──┤           │
  │                                             AND-join ───┘           │
  │       reviewer reads [impl_code, test_suite] → approved             │
  │   → flow completes, Supervisor sees final output                    │
  │                                                                     │
  │ Turn 3: Supervisor synthesizes → "Auth module complete"             │
  └─────────────────────────────────────────────────────────────────────┘

  TaskTree for the above:
  chat-turn (root)
  ├── task-001: planner          (done, confidence 0.9)
  └── task-002: flow:impl-pipe   (done)
       ├── task-003: coder       (done)
       └── task-004: tester      (done)
```

---

## 5. SpawnsAgent — reusing the PendingReview await pattern

The key architectural insight: sub-agent invocation is structurally identical to
human approval. The same `PendingReview` await mechanism handles both.

```
  EXISTING: Human-in-the-loop (PendingReview)
  ─────────────────────────────────────────────
  ReactLoop hits risky tool
    → ToolDispatcher returns NeedsApproval
    → authority.open_review_with_completion()
    → ReviewHandle { completion: oneshot::Receiver<ReviewResolution> }
    → loop AWAITS the receiver
    → human clicks approve
    → authority.resolve_review() fires the oneshot
    → loop RESUMES with tool result

  NEW: Sub-agent invocation (SpawnsAgent)
  ─────────────────────────────────────────
  ReactLoop hits invoke_agent(coder, goal, reads=[...])
    → ToolDispatcher returns SpawnsAgent { task_id, completion: oneshot::Receiver<AgentResult> }
    → SAME await mechanism  ←─────────────────────────
    → AgentRunner spawns child ReactLoop
    → child finishes, fires the oneshot
    → loop RESUMES with agent output as tool result
```

New `ToolOutcome` variant:

```
  ToolOutcome variants:
    Output(String)                      ← tool ran, here's the result
    NeedsApproval(..)                   ← human must decide (existing)
    SpawnsAgent { task_id, completion } ← NEW: spawned child, awaiting
    Error(String)
    Terminal(String)
```

### Why blocking await (not fire-and-poll)

Five dispatch options were considered:

| Option                 | Shape                                     | Parallelism            | Infrastructure cost           |
| ---------------------- | ----------------------------------------- | ---------------------- | ----------------------------- |
| A. Blocking await      | invoke_agent → suspend until done         | None (sequential)      | Minimal — reuse PendingReview |
| B. Fire-and-poll       | invoke_agent → task_id; check_task later  | Yes, but wastes tokens | Low — extends flow.status     |
| C. Parallel batch      | invoke_agents_parallel([...]) → await all | Yes, explicit          | Medium — new batch tool       |
| D. Two-phase plan/exec | LLM emits plan; runtime executes it       | Max                    | High — new plan executor      |
| E. Event-driven        | invoke_agent → subscribe to events        | Yes                    | High — changes loop model     |

**Decision: Option A (blocking await)** for the Chat Supervisor.

Rationale:

- Supervisor is per-chat; the user watches every step sequentially
- Parallel execution happens inside Flows (AND-join gates) — not at the chat level
- Reuses the `PendingReview`/`ReviewHandle` infrastructure exactly
- LLM context coherence: Supervisor sees each result immediately in conversation
- Bounded by `max_turns` in the supervisor's `LoopConfig` (no extra depth guard needed)

If parallel agent dispatch becomes necessary later, Option C can be added as a
second tool (`invoke_agents_parallel`) without changing Option A semantics.

---

## 6. CriticConfig — loop-level Generator-Critic

The wiki's Generator-Critic pattern belongs inside the `ReactLoop`, not as a
separate flow node. Critique fires just before the loop emits a Terminal
(submit / stop).

### How Claude Code / Codex handle it — and what we already have

These systems use **execution-based critique**:

```
  Code agent loop:
    LLM generates code
    → run_tests → exit code, stderr
    → if failure: appended to conversation as tool result
    → LLM sees: "[test FAILED] assertion error at line 42"
    → LLM fixes code → run_tests again
    → if pass: submit
```

**We already have this.** `ToolOutcome::Error` feeds back as a tool result and
the loop continues. Test failures ARE the execution-based critic for code.

### Where a separate LLM critic is needed

Execution critique cannot cover non-executable artifacts:

```
  Executable artifacts:          Non-executable artifacts:
    code → run tests ✓             PRD → ?
    compile → errors ✓             Tech spec → ?
    lint → warnings ✓              Architecture proposal → ?
    typecheck → errors ✓           Review comments → ?

  Already loop back ←             No machine oracle → need LLM critic
```

### CriticConfig design

New field on `AgentDef` alongside `reflection: Option<ReflectionConfig>`:

```yaml
# agent.yaml
critic:
  # Only fires for these artifact kinds — not for code (tests handle that)
  for_artifact_kinds: [prd, tech_spec, proposal, review_comment]

  # Separate model avoids correlated errors from the same bias as the generator
  model: claude-opus-4-5 # or claude-3-7-sonnet, or copilot

  # Number of revision cycles before accepting output regardless
  max_revisions: 2

  # Trigger: before_submit (only at terminal) or every_n_turns
  trigger: before_submit
```

The loop-level CriticPhase:

```
  Turns 1–N (working):
    │
    ├── [every N turns] → ReflectionPass: self-assess (existing, same model)
    │
    └── [at terminal / submit]
         ├── artifact_kind in for_artifact_kinds? ──no──→ submit as normal
         │
         └── yes → CriticPhase:
               different LLM call (different model, critic persona prompt)
               no tool access (pure semantic review)
               structured output:
                 { passed: bool,
                   issues: [{ severity, desc, evidence, fix_suggestion }],
                   confidence: float }
               │
               ├── passed: true  → submit
               └── passed: false, revisions remaining:
                     append critique as structured user message
                     revisions_remaining -= 1
                     continue loop
               (max_revisions exhausted → submit with warning)
```

### Distinction from ReflectionConfig

| Aspect            | `ReflectionConfig` (existing) | `CriticConfig` (new)                        |
| ----------------- | ----------------------------- | ------------------------------------------- |
| Actor             | Same LLM, same model          | Different LLM, different model              |
| Trigger           | Every N turns, mid-stream     | At terminal/submit only                     |
| Purpose           | Self-check: "am I on track?"  | External verify: "is the output correct?"   |
| Output            | Prose appended to history     | Structured `{ passed, issues, confidence }` |
| Error correlation | Same bias as generator        | Avoids correlated errors                    |
| Tool access       | Full (same as generator)      | None (pure semantic review)                 |

Both can be active simultaneously and complement each other.

---

## 7. Blackboard — explicit data dependencies in flow.yaml

### Decision: flow author declares reads

Blackboard access is declared in `flow.yaml` by the flow author, not enforced
at runtime by schema inference.

### Proposed flow.yaml extension

```yaml
# flow.yaml — extended with Blackboard declarations
nodes:
  - id: product-brief
    owner: pm-agent
    # reads: []  ← implicit (entry node, nothing to read yet)
    outputs:
      - port: prd
        blackboard_key: prd # where output lands in Blackboard
        reviewers: [human, critic-agent]
        routes_to: architecture

  - id: architecture
    owner: arch-agent
    reads: [prd] # explicit data dependency
    outputs:
      - port: tech-spec
        blackboard_key: tech_spec
        routes_to: implementation

  - id: implementation
    owner: coder-agent # sees only tech_spec — not prd
    reads: [tech_spec]
    outputs:
      - port: code
        blackboard_key: impl_code
```

The `reads` declaration does three things at once:

1. **Execution gate** — node cannot activate until all declared keys are present in the Blackboard (replaces derived `required_inputs`)
2. **Blackboard filter** — `InvocationContext` is built from only the declared keys (context isolation)
3. **Static analysis** — `FlowGraph::build` can detect missing artifacts at load time, not runtime

### Migration — backward-compatible dual mode

```
  FlowGraph::build resolution:

    Node declares reads: [prd, tech_spec]
      → use reads as execution gate AND Blackboard filter
      → existing routes_to is still respected for successor notification

    Node has no reads:
      → derive required_inputs from routes_to (existing behavior, unchanged)
      → no Blackboard filtering (sees all documents, same as today)
```

Existing `flow.yaml` files continue to work with no changes. New flows opt into
explicit data dependencies by adding `reads:` and `blackboard_key:` fields.

The only new load-time check: if a node declares `reads: [prd]` but no upstream
node declares `blackboard_key: prd`, `FlowGraph::build` emits an error rather
than discovering the problem at runtime.

---

## 8. Structural changes required

The 15 changes fall into three independent groups:

### Group 1 — cronygraph async primitives (infrastructure, no user-visible behavior change)

| #   | Change                                     | Where                                |
| --- | ------------------------------------------ | ------------------------------------ |
| 1   | Make `Step` trait async (`async_trait`)    | `cronygraph/src/orchestration.rs`    |
| 2   | Rewrite `Orchestrator::run` as async       | `cronygraph/src/orchestration.rs`    |
| 3   | Add `TaskTree` (id/parentId/status/output) | `cronygraph/src/task_tree.rs` (new)  |
| 4   | Add `Blackboard` (scoped kv + artifacts)   | `cronygraph/src/blackboard.rs` (new) |

### Group 2 — loop-level critic (agent quality, independent of orchestration)

| #   | Change                                                | Where                                     |
| --- | ----------------------------------------------------- | ----------------------------------------- |
| 5   | Add `CriticConfig` struct                             | `cronymax/src/agent_loop/react.rs`        |
| 6   | Add `CriticPhase` to `ReactLoop`                      | `cronymax/src/agent_loop/react.rs`        |
| 7   | Extend `AgentDef` with `critic: Option<CriticConfig>` | `cronymax/src/capability/agent_loader.rs` |

### Group 3 — Chat Supervisor + Blackboard + hierarchy (the orchestration shift)

| #   | Change                                     | Where                                     |
| --- | ------------------------------------------ | ----------------------------------------- |
| 8   | Add `AgentKind::Supervisor`                | `cronymax/src/capability/agent_loader.rs` |
| 9   | Add `SpawnsAgent` variant to `ToolOutcome` | `cronymax/src/agent_loop/tools.rs`        |
| 10  | Add `invoke_agent` capability tool         | `cronymax/src/capability/` (new)          |
| 11  | Add `invoke_flow` capability tool          | `cronymax/src/capability/` (new)          |
| 12  | Register both tools for `__chat__` agent   | `cronymax/src/runtime/agent_runner.rs`    |
| 13  | `RuntimeAuthority` owns `TaskTree`         | `cronymax/src/runtime/authority.rs`       |
| 14  | `FlowNode` reads `reads:` field            | `cronymax/src/flow/definition.rs`         |
| 15  | `FlowGraph::build` uses `reads` as gate    | `cronymax/src/flow/definition.rs`         |

### Dependency order

```
  Group 1 ──────────────────────────────────────────────────────┐
    (cronygraph async)                                           │
         ↓ prerequisite for                                      │
  Group 3 (orchestration)          Group 2 can ship any time ◄──┘
    (chat supervisor + Blackboard)  (loop-level critic)
```

Group 2 can ship independently and provides value immediately (document artifact
critique for any agent, regardless of supervisor wiring).

---

## 9. Trace event alignment

Unified span schema spanning both flow events and inner-loop events:

```
  TraceSpan {
    trace_id:       Uuid,            // one per FlowRun or chat session
    span_id:        Uuid,            // one per Step / tool call / loop turn
    parent_span_id: Option<Uuid>,    // enables tree reconstruction
    kind:           TraceSpanKind,
    actor:          String,          // "pm-agent", "__chat__", "coder"
    timestamp:      SystemTime,
    payload:        serde_json::Value,
  }

  TraceSpanKind:
    WorkflowNodeEnter / WorkflowNodeExit
    AgentTaskAssigned / AgentResultReceived
    ToolCallStarted   / ToolCallCompleted
    ApprovalRequested / ApprovalGranted / ApprovalRejected
    HandoffRequested  / HandoffAccepted
    CriticPhaseStarted / CriticPhaseCompleted
    BudgetExceeded
```

The `TraceEmitterMiddleware` in `cronymax` emits these. The existing `TraceKind`
in `flow/trace.rs` maps to these. The activity panel and flow trajectory UI
can render the full tree via `parent_span_id`.

---

## 10. Locked decisions

```
  ┌────────────────────────────────────────────────────────────────────┐
  │  1. Supervisor lives only at the Chat level — never in flow.yaml   │
  │  2. Supervisor model = chat toolbar model picker (zero new UI)     │
  │  3. Maximum hierarchy depth = 2 (chat → flow/agent)               │
  │  4. invoke_agent uses blocking await via SpawnsAgent/PendingReview │
  │  5. Parallelism at level 1 only — via Flow AND-join, not at Chat   │
  │  6. CriticConfig fires only for document artifacts (not code)      │
  │  7. Code critique = existing tool feedback loop (tests, lint)      │
  │  8. reads: replaces required_inputs; backward-compat dual mode     │
  │  9. cronygraph::Step becomes async — prerequisite for all of above │
  └────────────────────────────────────────────────────────────────────┘
```

---

## 11. Open questions (not yet decided)

- **Blackboard persistence**: is the Blackboard part of the `Snapshot` that
  `RuntimeAuthority` persists, or is it ephemeral per run? Document files on
  disk already serve as persistent artifacts; the Blackboard might be a purely
  in-memory index over those files.

- **`reads` in this change or follow-on**: the Blackboard filter requires `reads`
  in `flow.yaml`. The execution gate replacement of `required_inputs` is simpler
  and could ship first (Group 3 above) while the Blackboard filter lands later.

- **CriticConfig model fallback**: if `critic.model` is unset, should it default
  to the same model as the generator (cheaper, risks correlated errors) or refuse
  to enable (safer)?

- **TaskTree in the activity panel**: the activity panel currently reconstructs
  the work tree from events. When `TaskTree` is owned by `RuntimeAuthority`, the
  panel should read from `GetSpaceSnapshot` directly. Migration path needs scoping.
