## Context

The `space-agent-integration` change is at 55/60 tasks and ships:
- A C++ `AgentRuntime` per Space hosting a JS-driven ReAct loop in the CEF renderer (`web/agent/loop.js`).
- A `BridgeHandler` with `space.*`, `agent.*`, `tool.*` JSON channels.
- `AgentGraph` C++ data model with `kLlm | kTool | kCondition | kHuman | kSubgraph` node kinds.
- SQLite-backed `SpaceStore` for spaces, tabs, terminal blocks, and traces.

Today, multi-agent collaboration would happen *inside* one `AgentGraph` by stringing together LLM/Tool/Human nodes — the workflow-engine paradigm. Real product-team workflows (PRD → review → tech spec → review → code) don't fit that shape: they revolve around **Documents** that move between specialists, with explicit review gates.

This change introduces a new layer above the existing engine: **Flows** of **Agents** that hand off **Documents**, with humans and other agents acting as **Reviewers**. It does NOT rewrite the ReAct loop; that loop becomes the *internal* loop of one Agent. Three follow-on changes (`agent-orchestration-ui`, `agent-skills-marketplace`, `document-wysiwyg`) build the polished UI, extensibility, and editing experience.

This is a CLI-flavored slice intentionally: hand-edited YAML, raw markdown, a basic chat panel. UI polish is deferred so the data model and runtime can stabilize first.

## Goals / Non-Goals

**Goals:**

- Define the on-disk Workspace contract for Flows, Agents, Documents, Document Types, and Reviews — git-trackable as the source of truth.
- Implement a `FlowRuntime` that orchestrates Agents via Document handoffs, with both typed-port (Flow-defined) and `@mention` (producer-driven) routing.
- Define the Document lifecycle (`DRAFT → IN_REVIEW → CHANGES_REQUESTED → APPROVED → HANDED_OFF`) and persist revisions in a `.history/` sidecar.
- Implement reviewer agents (LLM `Critic`, deterministic `Schema` validator) with per-doc `max_review_rounds` ceiling.
- Demote `kSubgraph | kCondition | kHuman` from public node kinds; keep `AgentGraph` as the internal per-Agent loop only.
- Ship a basic chat panel showing Run events and rendering Document submissions as cards.

**Non-Goals:**

- Visual Flow editor (deferred → `agent-orchestration-ui`).
- Slack-style channel UI, threading, inbox, status dot (deferred → `agent-orchestration-ui`).
- WYSIWYG markdown editor, block-anchored comments (deferred → `document-wysiwyg`).
- Skill marketplace, Node sidecar (deferred → `agent-skills-marketplace`).
- Real-time collaborative document editing (always out of scope; single-writer per doc).
- Multi-user / multi-human workflows (always out of scope for now).
- Windows / Linux support (mac-only this change).

## Decisions

### Decision 1: Documents are `.md` files in the workspace, not SQLite blobs

**Chosen**: Documents live at `<workspace>/.cronymax/flows/<flow>/docs/<doc>.md`. Revisions live at `.cronymax/flows/<flow>/docs/.history/<doc>.<rev>.md`. Reviews live at `.cronymax/flows/<flow>/runs/<run-id>/reviews.json`. All committable.

**Alternatives**:
- SQLite blobs — invisible to git, breaks the "agents produce real artifacts you can git-grep" pitch.
- Git commits per revision — pollutes commit history with `Architect: v1`, `Architect: v2 addressed comments`. Better to surface only the user-approved final commit.

**Rationale**: Documents *are* the deliverable. Treating them as workspace artifacts makes the agent team observable to existing dev tools (git, GitHub, VS Code, grep). The `.history/` sidecar is the per-revision audit log; the user controls when to git-commit the approved doc.

---

### Decision 2: Layer-on-top — keep the existing ReAct engine intact

**Chosen**: `AgentRuntime` and `web/agent/loop.js` are unmodified in their interface. A new `FlowRuntime` instantiates *N* `AgentRuntime` instances per Flow Run, one per declared Agent. Each `AgentRuntime` is given the input Document(s) as initial context and a `submit_document(type, content)` tool to produce its output.

**Alternatives**:
- Replace `GraphEngine` with a Flow engine — wastes the just-shipped ReAct work; conflates two abstraction levels.
- Single `AgentRuntime` shared across all agents in a Flow — destroys per-Agent context/memory isolation, kills the "first-class Agent identity" goal.

**Rationale**: The ReAct loop is the right abstraction for *one* Agent's thinking. Flow handoffs are a different abstraction. Stacking them is cleaner than merging.

---

### Decision 3: `AgentGraph` node kinds demoted to internal-only

**Chosen**: `AgentNodeKind { kLlm, kTool, kCondition, kHuman, kSubgraph }` stays in `src/agent/agent_graph.h` but is documented as **internal data model for the per-Agent loop**. The bridge surface no longer exposes graph editing channels for these kinds; `agent.graph.*` channels will be removed in this change.

**Alternatives**:
- Delete `AgentGraph` entirely — would require rewriting the existing ReAct loop dispatch in `loop.js`.
- Rename to avoid confusion (e.g. `AgentInternalGraph`) — churn for marginal clarity; comments suffice.

**Rationale**: Minimum disruption to the working engine. Public concept space simplifies to {Flow, Agent, Document, Review}.

---

### Decision 4: Hybrid routing — typed ports + `@mention`, additive not exclusive

**Chosen**: A Flow declares typed ports between Agents (`product.out:prd → architect.in:prd`). On Document submission, the FlowRuntime:
1. Finds all outgoing edges from the producing Agent matching the submitted doc's port → schedules those Agents (the **default route**).
2. Parses `@<AgentName>` mentions in the doc body → schedules those Agents *additionally* (the **escape hatch**).
3. Mentioned Agents must be declared in the Flow; unknown mentions are warnings, not errors.

**Alternatives**:
- Producer-driven only (LLM picks `@mention` for everyone): unpredictable, debug-hostile.
- Flow-defined only: blocks the bug-fix-clarification scenario where Coder needs to ping Product.
- Override mode (mention replaces default): surprising; users expect the declared Flow to always run.

**Rationale**: Default route is *guaranteed* (debuggable, predictable); `@mention` is *opt-in extra* (flexible). Predictable + flexible.

---

### Decision 5: Document type schemas are YAML files in the workspace

**Chosen**: `<workspace>/.cronymax/doc-types/<type>.yaml` declares required headings, optional sections, and lightweight validators. The `Schema` reviewer validates a submitted doc against its type's schema deterministically (no LLM).

**Example** (`prd.yaml`):

```yaml
name: prd
display_name: Product Requirements Doc
required_sections:
  - { heading: "Goal", min_words: 20 }
  - { heading: "Acceptance Criteria", min_items: 1, kind: list }
  - { heading: "Non-Goals" }
optional_sections:
  - { heading: "Open Questions" }
front_matter_required: [author, owner_agent]
```

**Alternatives**:
- JSON Schema over a parsed AST — overkill, doesn't match how users think about markdown.
- No schema (freeform) — Reviewer agents can't give structured feedback; doc-type ports become meaningless.
- Hard-coded built-in types only — kills extensibility.

**Rationale**: The minimum machine-readable contract that supports both port typing and deterministic linting. Built-in types ship with the app under `~/.cronymax/builtin-doc-types/`; user types override per workspace.

---

### Decision 6: Reviewer pipeline is fixed-order (validators → reviewer agents → human)

**Chosen**: On `submit_document`:

```
   ┌─────────────┐  fail  ┌────────────────────────────┐
   │ Validators  │───────▶│ status=CHANGES_REQUESTED   │
   │ (Schema)    │        │ (back to author)           │
   └─────┬───────┘        └────────────────────────────┘
         │ pass
         ▼
   ┌──────────────────────┐
   │ Reviewer Agents      │ ← run in parallel; emit comments
   │ (Critic + custom)    │   never block; configurable timeout
   └─────┬────────────────┘
         │ all complete (or timeout)
         ▼
   ┌──────────────────┐  no human  ┌──────────┐
   │ Human Reviewer?  │───────────▶│ APPROVED │
   └────┬─────────────┘            └──────────┘
        │ yes
        ▼
   ┌──────────────────────────────────┐
   │ status=IN_REVIEW (await user)    │
   └──────────────────────────────────┘
```

- Validators block (fail = changes requested, no human/agent involvement).
- Reviewer agents only emit comments; they *never* block. They have a per-Flow `reviewer_timeout_secs` (default 60).
- Human reviewer (declared on the Flow edge as `requires_human_approval: true`) gates final transition to APPROVED.
- `max_review_rounds` (per-doc, default 3) caps the validator+reviewer cycle. After the cap, the doc auto-transitions to APPROVED with a `review_exhausted: true` warning, OR halts the Flow Run (configurable).

**Alternatives**:
- Reviewer agents block on critique severity — produces infinite loops; requires Critic to know what's "good enough."
- Always require human approval — defeats automation; Flows can't run unattended overnight.

**Rationale**: Deterministic gates only block; LLM critique is advisory. Humans gate when explicitly configured.

---

### Decision 7: `reviews.json` is the per-Run review state; comments are NOT in the doc

**Chosen**: Each Run directory has one `reviews.json`:

```json
{
  "run_id": "2026-05-01-1234",
  "documents": {
    "docs/prd.md": {
      "current_revision": 2,
      "status": "APPROVED",
      "round_count": 1,
      "revisions": [
        { "rev": 1, "submitted_at": "...", "submitted_by": "ProductAgent", "sha": "..." },
        { "rev": 2, "submitted_at": "...", "submitted_by": "ProductAgent", "sha": "..." }
      ],
      "comments": [
        {
          "id": "c-001",
          "author": "CriticAgent",
          "kind": "comment",
          "anchor": { "kind": "line_range", "rev": 1, "start": 14, "end": 18 },
          "body": "Acceptance criteria missing for empty-state UX.",
          "resolved_in_rev": 2
        }
      ]
    }
  }
}
```

**Alternatives**:
- Inline HTML comments in the doc — pollutes the artifact, breaks WYSIWYG round-trip.
- Per-doc sidecar (`<doc>.reviews.json`) — multiplies files; cross-doc queries need to scan many files.
- Sidecar branch / PR — heavyweight; out of scope.

**Rationale**: Single source per Run. Comment anchoring is `line_range` for v1; future change `document-wysiwyg` migrates to block-ID anchors with a `block_id` field added (line range kept as `legacy_anchor`).

---

### Decision 8: Bridge surface is JSON over the existing `channel\npayload` framing

**Chosen**: Reuse the `channel\n{json}` framing from `space-agent-integration`. New channels:

- `flow.list`, `flow.load`, `flow.run.start`, `flow.run.status`, `flow.run.cancel`
- `document.read`, `document.list`, `document.subscribe` (server-push events)
- `review.list`, `review.comment`, `review.approve`, `review.request_changes`
- `agent.registry.list`, `agent.registry.load`
- `mention.user_input` (user posts a chat message that may contain `@mentions`)

Server-push events use a single `event` channel with a `type` discriminator (`flow.run.event`, `document.changed`, `review.comment_added`, etc.) so the renderer subscribes once.

**Alternatives**:
- JSON-RPC 2.0 — bigger change, breaks existing handlers.
- gRPC over WebSocket — overkill for in-process bridge.

**Rationale**: No protocol change needed. Standardizes the event-stream pattern for the next change's UI work.

---

### Decision 9: YAML parser — vendor `yaml-cpp` (single header alternative not viable for round-trip)

**Chosen**: Add `yaml-cpp` (single static lib) as a CMake `FetchContent` dependency for parsing `flow.yaml`, `agent.yaml`, and doc-type schemas. Round-trip writing is **not** required in this change (canvas editor in next change handles round-trip; this change only reads YAML).

**Alternatives**:
- Single-header `mini-yaml` — incomplete YAML 1.2 support; struggles with anchors.
- Hand-roll a tiny subset parser — bug magnet.
- JSON only — non-negotiable; YAML is friendlier for hand-editing.

**Rationale**: `yaml-cpp` is well-maintained, MIT-licensed, statically linked, ~150KB. Round-trip preservation is the next change's problem.

---

### Decision 10: `agent.yaml` is per-workspace; `flow.yaml` references agents by name

**Chosen**:

```
.cronymax/
├── agents/
│   ├── product.agent.yaml
│   ├── architect.agent.yaml
│   ├── coder.agent.yaml
│   └── critic.agent.yaml          # reviewer agent
└── flows/
    └── feature-x/
        └── flow.yaml              # references agents by file basename
```

A Flow declares which agents participate (`agents: [product, architect, coder, critic]`). Edges reference declared agents only. `@mention` resolution is restricted to declared agents.

**Alternatives**:
- Inline agent config in `flow.yaml` — duplicates agent definitions across Flows; no agent reuse.
- Global agent registry (`~/.cronymax/agents/`) — couples agents to the user, not the workspace; loses portability.

**Rationale**: Agents are reusable workspace assets. Flows compose them.

## Risks / Trade-offs

- **YAML hand-editing as the only authoring path** → Steep onboarding. *Mitigation*: ship 2 example Flows + commented YAML schemas; users copy-paste-modify. The visual editor in the next change is the real fix.

- **Filesystem as the document store** → No transactions. Two reviewers writing `reviews.json` concurrently can clobber. *Mitigation*: single-writer lock per `reviews.json` file (POSIX `flock`); writes are append-mostly so contention is low. SQLite-backed alternative is a future change if pain is real.

- **Reviewer agent cost** → Every doc submission triggers N LLM-driven reviewers. With paid APIs this stacks fast. *Mitigation*: per-Flow `reviewer_enabled: false` flag; per-reviewer-agent cost cap; cost surfacing in trace events (consumed by inbox in next change).

- **`max_review_rounds` ceiling can hide real problems** → Doc approves with `review_exhausted: true` after 3 rounds even if Critic still has valid concerns. *Mitigation*: surface `review_exhausted` prominently in the chat panel and in `reviews.json`; offer a "raise the ceiling for this run" UI affordance later.

- **`@mention` parsing is brittle** → False positives in code blocks (`@types/node`), email addresses, etc. *Mitigation*: strict parser — `@mention` must be at line start, preceded by whitespace, followed by space/punctuation; ignore inside fenced code blocks.

- **`AgentGraph` demotion is technically a breaking change for any existing graph editor UI** → No such UI shipped publicly yet, but `agent.graph.*` channels existed in `space-agent-integration`. *Mitigation*: this change explicitly removes them in the modified `multi-agent-orchestration` spec; no migration since no production users.

- **Flow Runs are long-lived** → A Flow can pause for hours waiting on human review. The ReAct loop and renderer process must survive app restarts. *Mitigation*: Flow Run state is persisted to `runs/<id>/state.json` after every status transition; on app start, `FlowRuntime` rehydrates `IN_REVIEW` runs. Live agent execution does NOT survive restart in v1 (the loop is killed on quit and the agent's work-in-progress message buffer is lost) — surfaced as a known limitation.

- **Document concurrency between human-edits and agent-edits** → A human edits `prd.md` in their editor while ProductAgent is producing v2. *Mitigation*: doc has a `locked_by: agent` flag while an agent is actively writing; human edits during that window go to a `.cronymax/conflicts/` directory with a notification. Acceptable v1 friction.

## Migration Plan

This change layers on top of in-progress `space-agent-integration`. Migration steps:

1. **Land `space-agent-integration` first** (5 tasks remaining; assumed shipped before this change starts).
2. **Add `flow-orchestration`, `document-collaboration`, `agent-entity` capabilities** (new specs; no migration).
3. **Modify `multi-agent-orchestration` spec**: REMOVE Subgraph/Condition/Human public requirements (keep them as internal implementation notes); add a "scope: per-Agent ReAct loop" preamble.
4. **Modify `space-manager` spec**: ADD Flow registry per-Space, active-Run pointer.
5. **Bridge breaking change**: remove `agent.graph.*` channels from the public surface. No external consumers; renderer code adjusted in lockstep.
6. **No data migration needed**: existing Spaces have no Flows; Flows are opt-in workspace artifacts.
7. **Rollback strategy**: revert the change. Existing `space-agent-integration` continues to work since the layer-on-top design preserves all its surfaces.

## Open Questions

- **POSIX `flock` on macOS APFS** — is it reliable for our concurrency pattern? *Spike before implementation*. If not, fall back to a small SQLite table for review-state mutations and keep `reviews.json` as a derived export.
- **Should `flow.yaml` allow inline agent overrides?** (e.g. "use product-agent but with a different system prompt for this Flow") — defer; needs a real motivating use case.
- **Should APPROVED documents auto-commit to git?** — defer; UX question for the next change. Safer default: never auto-commit; surface a "commit this doc" button in the workbench later.
- **Is `Critic` agent's prompt shipped in-repo or pulled from skills marketplace?** — for v1, in-repo as a built-in. The `agent-skills-marketplace` change can refactor it into a skill bundle.
- **Cancellation semantics during reviewer fan-out** — if user cancels a Run while 3 reviewer agents are mid-LLM-call, do we kill all in-flight requests? *Decide in implementation*: yes, `AbortController` style; partial reviews discarded.
