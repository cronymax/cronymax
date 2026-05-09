# Multi-Agent Orchestration — Design & Decisions

> Captured from the `/opsx:explore` discovery session that produced the staged change set:
> [agent-document-orchestration](../openspec/changes/agent-document-orchestration/proposal.md) →
> [agent-orchestration-ui](../openspec/changes/agent-orchestration-ui/proposal.md) →
> [agent-skills-marketplace](../openspec/changes/agent-skills-marketplace/proposal.md) →
> [document-wysiwyg](../openspec/changes/document-wysiwyg/proposal.md).

## 1. Paradigm shift

The current `space-agent-integration` change models multi-agent as a **workflow graph** (LLM / Tool / Condition / Human / Subgraph nodes). The new model is **organization-shaped**: one node kind (Agent), and Documents are the medium of collaboration.

| Dimension          | Old (workflow-engine)                     | New (organization)                                          |
| ------------------ | ----------------------------------------- | ----------------------------------------------------------- |
| Node kinds         | LLM / Tool / Condition / Human / Subgraph | **Agent only**                                              |
| Agent identity     | Graph node config                         | First-class entity: own LLM, Context, Memory, Skills, Loop  |
| Inter-node payload | Implicit message-history mutation         | **Markdown Document** (PRD, Tech Spec, Test Cases, …)       |
| Human-in-the-loop  | A node kind that pauses for allow/deny    | **Reviewer-in-the-Agent-Loop**: review/comment on Documents |
| Handoff trigger    | Edge traversal (auto)                     | Typed port + `@downstream-agent` mention                    |
| Skills             | `tools[]` array on the node               | **Marketplace** — discoverable, installable                 |
| LLM provider       | OpenAI-compatible only                    | Multi-provider; **GitHub Copilot first**                    |

## 2. Mental model

```
   ┌──────────────────────────────────────────────────────────┐
   │                       FLOW (org chart)                   │
   │                                                          │
   │   ┌─────────┐    Doc:PRD    ┌──────────┐   Doc:TechSpec │
   │   │ Product │  ──────────▶  │ Architect│  ────────┐     │
   │   │  Agent  │   ▲           │  Agent   │          │     │
   │   └─────────┘   │           └──────────┘          ▼     │
   │        ▲   ┌────┴─────┐          ▲          ┌──────────┐│
   │        │   │ Reviewer │          │          │   Coder  ││
   │        │   │ (human)  │     ┌────┴─────┐    │   Agent  ││
   │        │   └──────────┘     │ Reviewer │    └──────────┘│
   │        │                    │ (Critic  │          │     │
   │        │                    │  Agent)  │          ▼     │
   │        │                    └──────────┘    Doc:Code+   │
   │        │                                    Description │
   │        │                                          │     │
   │        └──────────  @ProductAgent  ◀──────────────┘     │
   │                     "found ambiguity in PRD"            │
   └──────────────────────────────────────────────────────────┘

   Each AGENT, zoomed in:
   ┌──────────────────────────────────────────────────────────┐
   │   Agent:Coder                                            │
   │   ┌───────────┐   ┌─────────┐   ┌──────────┐  ┌───────┐ │
   │   │ LLM       │   │ Context │   │ Memory   │  │Skills │ │
   │   │ (Copilot/ │   │ (window)│   │(long-term│  │ • git │ │
   │   │  GPT/...) │   │         │   │  vec/kv) │  │ • npm │ │
   │   └───────────┘   └─────────┘   └──────────┘  │ • test│ │
   │                                                └───────┘ │
   │         ┌─────────── Agent Loop ──────────┐              │
   │         │  perceive → plan → tool_call →  │              │
   │         │  observe → write_doc → submit   │              │
   │         └─────────────────────────────────┘              │
   └──────────────────────────────────────────────────────────┘
```

## 3. Locked decisions

```
┌──────────────────────────────────────────────────────────────────┐
│  1. Strategy:    Layer on top of space-agent-integration         │
│  2. Doc storage: .md files in workspace (git-trackable)          │
│  3. Routing:     hybrid — typed ports + @mention escape hatch    │
│  4. Reviewer:    Agent reviewers shipped in v1                   │
│  5. Skills:      Marketplace in v1 scope (curated GitHub repo)   │
│  6. Copilot:     Public API via GitHub OAuth / gh CLI            │
│  7. Chat:        Per-Flow channel (Slack-style)                  │
│  8. Flow editor: Visual graph editor (React Flow)                │
│  9. Workbench:   WYSIWYG (Milkdown) + Monaco revision diff       │
│ 10. Notify:      Inbox + status-bar dot + OS notifications       │
└──────────────────────────────────────────────────────────────────┘
```

## 4. Layer-on-top: the existing engine becomes the inner loop

```
   ┌──────────────────────────────────────────────────────────────┐
   │  NEW LAYER  (this change set)                                │
   │  ┌──────────┐  ┌─────────┐  ┌──────────┐  ┌──────────────┐  │
   │  │ Flow     │  │ Document│  │ Review   │  │  Skills      │  │
   │  │ runtime  │  │ store   │  │ thread   │  │  marketplace │  │
   │  └────┬─────┘  └────┬────┘  └────┬─────┘  └──────┬───────┘  │
   │       │             │            │                │          │
   │       └─────────────┴────────────┴────────────────┘          │
   │                            │ uses                            │
   ├────────────────────────────▼─────────────────────────────────┤
   │  EXISTING  (space-agent-integration)                         │
   │  ┌──────────────────────────────────────────────────────┐    │
   │  │  AgentRuntime  →  ReAct loop  →  Tools  →  LLM       │    │
   │  │      (one of these per Agent in the Flow)            │    │
   │  └──────────────────────────────────────────────────────┘    │
   └──────────────────────────────────────────────────────────────┘
```

The existing `AgentRuntime` + ReAct loop becomes one Agent's _internal_ loop. `AgentGraph` node-kinds (LLM/Tool/Condition/Human/Subgraph) demote from public concepts to internal implementation details.

> **rust-runtime-migration update.** The "internal Agent loop"
> referenced above is being moved out of the C++ host and out of the
> renderer into `crates/cronymax`. Once the migration completes, the
> Rust runtime is the sole authority for run lifecycle, tool dispatch,
> reviewer-in-the-loop pauses, and persistence; UI panels here describe
> projections of runtime-emitted events rather than renderer-owned
> state. Any future skill or plugin runtime described in this document
> (including the Skills Marketplace) is **subordinate** to the Rust
> runtime: skills run as capability adapters or as runtime-launched
> child processes that participate in the runtime's permission and
> review flow. There is no peer Node skill sidecar with independent
> orchestration authority.

## 5. Document lifecycle

```
   ┌────────┐  agent.submit  ┌──────────┐  reviewer.comment  ┌────────────┐
   │ DRAFT  │───────────────▶│IN_REVIEW │───────────────────▶│CHANGES_REQ │
   │ (v_n)  │                │  (v_n)   │                    │  (v_n)     │
   └────────┘                └────┬─────┘                    └─────┬──────┘
        ▲                         │ approve                        │ revise
        │                         ▼                                ▼
        │                    ┌────────┐                       ┌────────┐
        └────────────────────│APPROVED│                       │ DRAFT  │
            handoff returned │ (v_n)  │                       │(v_n+1) │
                             └───┬────┘                       └───┬────┘
                                 │ @downstream OR              ───┘
                                 │ flow auto-routes
                                 ▼
                            ┌────────┐
                            │HANDED  │
                            │  OFF   │
                            └────────┘
```

## 6. On-disk layout (git-trackable)

```
   <workspace>/
   ├── .cronymax/
   │   ├── flows/
   │   │   └── feature-x/
   │   │       ├── flow.yaml              # Flow definition (source of truth)
   │   │       ├── flow.layout.json       # canvas positions (committable)
   │   │       ├── runs/
   │   │       │   └── 2026-05-01-1234/
   │   │       │       ├── trace.jsonl    # agent traces (gitignored)
   │   │       │       └── reviews.json   # review threads (committed)
   │   │       └── docs/
   │   │           ├── prd.md             # ◀ document, committed
   │   │           ├── tech-spec.md       # ◀ document, committed
   │   │           ├── test-plan.md
   │   │           └── .history/          # revisions (committed or ignored)
   │   ├── agents/
   │   │   └── coder.agent.yaml           # agent config, committed
   │   └── doc-types/
   │       └── prd.yaml                   # required sections / validators
   └── .gitignore
       └── .cronymax/flows/*/runs/*/trace.jsonl
```

Storage split rule:

- **Workspace FS** (git): artifacts humans care about — flow defs, agents, doc types, the documents themselves, review sidecars.
- **SQLite** (per Space, app-private): channel messages, inbox state, run history index, agent traces.
- **`~/.cronymax/`** (app-private): installed skills, cached models, Keychain refs.

## 7. Hybrid routing

```
   FLOW DEFINITION (typed ports — predictable path)
   ┌─────────┐ doc:PRD ┌──────────┐ doc:TechSpec ┌─────────┐
   │ Product │────────▶│ Architect│─────────────▶│ Coder   │
   └─────────┘         └──────────┘              └─────────┘
                            │
                            │ doc:RFC (rare)
                            ▼
                       ┌──────────┐
                       │ SecReview│  ← not on default path
                       └──────────┘

   PRODUCER-DRIVEN OVERRIDE (escape hatch)
   In a doc, agent writes:
       "## Handoff
        @Architect — please draft a tech spec.
        @SecReview — flagged: this touches auth, please consult."

   The Flow runtime parses @mentions on submit and dispatches in
   addition to the typed-port default route.
```

Rules:

- Typed ports = guaranteed forward edges; enforced doc-type compatibility.
- `@mentions` = additive opt-in; allows backward routing (e.g. Coder → @Product for clarification).
- Only Agents declared in the Flow are mentionable.

## 8. Reviewer pipeline

```
   Document submitted
        │
        ▼
   ┌─────────────────┐
   │ Validators      │ ← deterministic, fast, cheap. Block on failure.
   │ (schema, lint)  │
   └────────┬────────┘
            │ pass
            ▼
   ┌─────────────────┐
   │ Reviewer Agents │ ← LLM, slow, expensive. Run in parallel.
   │ (Critic, Sec)   │   Produce comments, not pass/fail.
   └────────┬────────┘
            │ comments emitted
            ▼
   ┌─────────────────┐
   │ Human Reviewer  │ ← gates approval, sees agent comments as
   │ (optional)      │   "AI suggestions" they can accept/dismiss.
   └─────────────────┘
```

Guardrail: per-doc `max_review_rounds` (default 3) prevents Critic→revise→Critic loops.

## 9. Layered architecture

```
   ┌──────────────────────────────────────────────────────────────┐
   │  PRESENTATION                                                │
   │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐        │
   │  │ Channel  │ │   Flow   │ │ Document │ │ Inbox +  │        │
   │  │  view    │ │  editor  │ │workbench │ │ status   │        │
   │  │ (Slack)  │ │ (graph)  │ │(Milkdown)│ │  dot     │        │
   │  └─────┬────┘ └─────┬────┘ └─────┬────┘ └─────┬────┘        │
   ├────────┼────────────┼────────────┼────────────┼─────────────┤
   │  EVENT BUS  (single stream of typed events)                  │
   │  ──────────────────────────────────────────────────────────  │
   │  msg.posted / doc.submitted / doc.commented / flow.started   │
   │  agent.thinking / handoff / mention / permission.requested   │
   ├──────────────────────────────────────────────────────────────┤
   │  ORCHESTRATION                                               │
   │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐        │
   │  │   Flow   │ │ Document │ │  Review  │ │   Skill  │        │
   │  │ runtime  │ │  store   │ │  threads │ │ runtime  │        │
   │  └──────────┘ └──────────┘ └──────────┘ └──────────┘        │
   ├──────────────────────────────────────────────────────────────┤
   │  AGENT LAYER (existing space-agent-integration)              │
   │  ┌──────────────────────────────────────────────────────┐    │
   │  │  AgentRuntime · ReAct loop · Tools · LLM provider    │    │
   │  └──────────────────────────────────────────────────────┘    │
   ├──────────────────────────────────────────────────────────────┤
   │  PLATFORM                                                    │
   │  CEF · SQLite (channel msgs, inbox) · Workspace FS (docs)    │
   └──────────────────────────────────────────────────────────────┘
```

The **event bus** is the spine. Channel view, inbox, and status dot are all _projections_ of the same typed-event stream.

## 10. UI surfaces

### 10.1 Per-Flow channel (Slack-style)

```
   ┌──────────────────────────────────────────────────────────────┐
   │  Cronymax                                                    │
   ├──────────┬───────────────────────────────────────────────────┤
   │ SPACE:   │  # feature-x-flow                                 │
   │ acme     │  ─────────────────────────────────────────────    │
   │          │                                                   │
   │ FLOWS    │  @user                              10:02 AM      │
   │ #feat-x  │  Build a tab-grouping feature.                    │
   │ #bugfix  │  ─────────────────────────────────────────────    │
   │ #refac   │                                                   │
   │          │  @ProductAgent                      10:02 AM      │
   │ DMs      │  ✏️ Drafting prd.md ...                          │
   │ Product  │  📄 prd.md  [v1 · IN_REVIEW · 2 reviewers]       │
   │ Coder    │     ↳ 3 comments • 1 unresolved                  │
   │          │  ─────────────────────────────────────────────    │
   │ AGENTS   │                                                   │
   │ ProdAg   │  @CriticAgent                       10:03 AM      │
   │ ArchAg   │  💬 left 3 comments on prd.md                    │
   │ CritAg   │     ↳ "Acceptance criteria missing for…"          │
   │ CoderAg  │       (open thread →)                             │
   │          │  ─────────────────────────────────────────────    │
   │          │                                                   │
   │          │  @ProductAgent                      10:05 AM      │
   │          │  📄 prd.md  [v2 · APPROVED]                      │
   │          │  Handed off → @ArchitectAgent                     │
   │          │  ─────────────────────────────────────────────    │
   │          │  💬 [type message... @mention to trigger agent]   │
   └──────────┴───────────────────────────────────────────────────┘
```

Message types: `text`, `agent_status`, `document_event`, `review_event`, `handoff`, `error`, `system`. Each Document submission is a thread root; comments and revisions live in the thread.

### 10.2 Visual Flow editor (node)

```
   ┌────────────────────────────────┐
   │  ●  Architect                  │  ← agent name, status dot
   │  ────────────────────────────  │
   │  🤖 GPT-4o (via Copilot)       │  ← LLM badge
   │  🧠 architecture-skills v1.2   │  ← skill bundles
   │  ────────────────────────────  │
   │  IN:                           │
   │  ⬤ prd  ──────────              │  ← typed input port
   │                                │
   │  OUT:                          │
   │              tech-spec ⬤  ──── │  ← typed output port
   │              rfc ⬤             │
   │  ────────────────────────────  │
   │  Reviewers: Critic, Human      │
   └────────────────────────────────┘
```

Notes:

- React Flow canvas; YAML is source-of-truth (`flow.yaml`), layout sidecar is `flow.layout.json`.
- Edge connection validates Document-type compatibility at the port level (correct-by-construction).
- Live execution overlay reuses the same canvas in run-mode (idle/thinking/submitted/reviewing/blocked colours).
- Multiple output ports per Agent — agent declares which port via `submit_document(type=…)`.

### 10.3 Document workbench

```
   ┌───────────────────────────────────────────────────────┐
   │  📄 tech-spec.md   [WYSIWYG | Source | Diff v1↔v2]    │
   ├───────────────────────────────────────────────────────┤
   │                                            │ COMMENTS │
   │   # Tech Spec                              │          │
   │                                            │ Critic   │
   │   ## Overview                              │ "missing │
   │   The system shall...                      │  perf    │
   │                                            │  budget" │
   │   ## API                              ◀━━━━│  thread→ │
   │   ▸ POST /widgets                          │          │
   │     - Body: {...}                          │ Human    │
   │                                            │ "+1, agree"
   └───────────────────────────────────────────────────────┘
```

- WYSIWYG via **Milkdown** (ProseMirror, markdown-native, deterministic serialization).
- Source toggle always available; diff mode uses **Monaco DiffEditor** over markdown source.
- Comments anchor to **stable block IDs** (UUID assigned on first save, persisted via `data-block-id`), not line numbers — survive edits.
- Two diff concepts kept distinct:
  - **Authoring diff** (in workbench, between revisions) — _required_ for review UX.
  - **External git diff** — _nice to have_; falls out of stable serialization.

### 10.4 Notification inbox

```
   ┌────────────────────────────────────────────────────────┐
   │  📬 Inbox                              [unread: 3] [⚙] │
   ├────────────────────────────────────────────────────────┤
   │  🔵 Architect needs review on tech-spec.md   2m ago   │
   │     #feature-x · v1                                    │
   │     [Review] [Snooze] [Mark read]                      │
   │  ────────────────────────────────────────              │
   │  🔵 @user mentioned in #bugfix-auth          5m ago   │
   │     CoderAgent: "Need clarification on…"               │
   │     [Open] [Snooze]                                    │
   │  ────────────────────────────────────────              │
   │  ⚪ Flow #refactor completed                  1h ago   │
   │     12 docs, 3 reviews, 0 errors                       │
   │  ────────────────────────────────────────              │
   │  Filters: [All] [Mentions] [Reviews] [Failures]        │
   └────────────────────────────────────────────────────────┘
```

| Event                         | Default routing | Configurable |
| ----------------------------- | --------------- | ------------ |
| Doc submitted for your review | Inbox + OS      | ✓            |
| Agent @-mentioned you         | Inbox + OS      | ✓            |
| Doc you authored got comments | Inbox           | ✓            |
| Flow run completed (success)  | Status dot      | ✓            |
| Flow run failed               | Inbox + OS      | ✓            |
| Permission request from skill | Modal + OS      | ✗ (always)   |
| Cost threshold exceeded       | Inbox + OS      | ✓            |
| Agent paused (waiting input)  | Status dot      | ✓            |
| Reviewer agent left comments  | Inbox           | ✓            |

Status-bar dot: gray idle / blue activity / orange needs-attention / red error. macOS dock badge mirrors unread count.

## 11. Skills marketplace (v1: deliberately minimal)

```
   ┌──────────────────────────────────────────────────────────────┐
   │  v1 Marketplace                                              │
   ├──────────────────────────────────────────────────────────────┤
   │  Source:    A GitHub repo (e.g. cronymax/skills-registry)    │
   │  Manifest:  registry.json listing skill packages             │
   │  Package:   tarball with manifest.json + tools/ + prompts/   │
   │  Install:   download to ~/.cronymax/skills/<name>/           │
   │  Discovery: in-app browse + install UI fetches registry.json │
   │  Update:    manual "check for updates" button                │
   │  Search:    client-side filter on registry.json (no backend) │
   │  Auth:      none (public repo, signed releases later)        │
   └──────────────────────────────────────────────────────────────┘
```

Skill package shape:

```
   my-skill/
   ├── manifest.json        # name, version, agent_compatibility, permissions
   ├── README.md
   ├── prompts/
   │   ├── system.md        # appended to agent system prompt
   │   └── examples/        # few-shot
   ├── tools/
   │   ├── index.js         # tool implementations (Node.js)
   │   └── package.json     # node deps
   └── schemas/             # JSON schemas for tool inputs/outputs
```

Runtime: **Node.js sidecar** (single process) with per-skill `vm.Context` isolation. Permissions declared in manifest (`fs.read:<pat>`, `fs.write:<pat>`, `net.fetch:<host>`, `shell.exec`); enforced at the bridge layer, prompted on first use per Space (browser-style), persisted thereafter. Enforcement MUST land in the same release as the API.

Deno was considered for the smaller permission surface; deferred to keep the runtime story conservative. Bundling Node is also deferred — detect host install, surface friendly error.

## 12. Copilot integration

```
   Auth flow options (v1 supports either, in priority order):

   ┌─────────────────────────────────────────────────────────────┐
   │ Path 1 (preferred when available): gh CLI                   │
   │   exec("gh auth token") → use Copilot token directly.       │
   │   Zero UI. Trusts user's existing gh setup.                 │
   ├─────────────────────────────────────────────────────────────┤
   │ Path 2 (fallback): GitHub Device Flow OAuth                 │
   │   1. POST /login/device/code → user_code, verification_uri  │
   │   2. Show user_code, open verification_uri in browser       │
   │   3. Poll /login/oauth/access_token until granted           │
   │   4. Store token in Keychain (macOS), refresh as needed     │
   │   5. Exchange for Copilot session token                     │
   └─────────────────────────────────────────────────────────────┘

   Endpoint:    https://api.githubcopilot.com/chat/completions
   Headers:     Authorization: Bearer <token>
                Editor-Version: cronymax/0.x
                Copilot-Integration-Id: <reg-id-needed>
```

Provider adapter sketch (replaces OpenAI-only `llm.js`):

```ts
interface LlmProvider {
  listModels(): Promise<Model[]>;
  chat(req: ChatRequest, opts: { stream }): AsyncIterable<Chunk>;
  countTokens(text: string): number;
  auth: { kind: "apikey" | "oauth" | "session" };
}
// Built-ins: CopilotProvider, OpenAIProvider, AnthropicProvider, OllamaProvider.
// Per-Agent config picks {provider, model}.
```

Posture caveats:

- The Copilot Chat endpoint is technically internal; using it requires editor-impersonating headers. Several OSS projects do this — accept the TOS-grey posture, or restrict to the public GitHub Models API.
- Subscription required; first 401 → friendly error.
- Tokens stored via macOS Keychain (Security framework); never in plaintext.

## 13. Risks tracked

- **Latency stacking** — multi-agent + reviewer cycles can run minutes to hours. Async UX (notifications, inbox) must be honest about this; not a synchronous chat product.
- **Reviewer infinite loops** — `max_review_rounds` ceiling + escalation to human.
- **Document concurrency on FS** — single-writer lock per doc; reviewers comment, never edit the doc itself.
- **Comment anchor drift** — block-ID anchoring (UUIDs persisted in markdown) survives most edits.
- **Skill sandbox reality** — permissions only matter if enforced; ship enforcement with the API, not later.
- **Copilot endpoint stability** — internal API may change; isolate behind the provider adapter.
- **WYSIWYG ↔ markdown round-trip** — golden tests on serialization stability; restrict Milkdown schema to the safe subset.
- **YAML round-trip vs canvas edits** — normalize on save; YAML comments lost on round-trip (accepted).
- **Tab-of-truth confusion** — `flow.yaml` is the source of truth; `flow.layout.json` is a cosmetic sidecar.

## 14. Staged change set

| Stage | Change                                                                                       | Ships                                                                                                                                       |
| ----- | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| 1     | [agent-document-orchestration](../openspec/changes/agent-document-orchestration/proposal.md) | Foundation — Agent entity, Flow YAML, on-disk Documents, reviewer agents, hybrid routing. Hand-edited YAML, raw markdown, basic chat panel. |
| 2     | [agent-orchestration-ui](../openspec/changes/agent-orchestration-ui/proposal.md)             | Slack-style channel, visual Flow editor (React Flow), inbox + status dot, OS notifications, typed event bus.                                |
| 3     | [agent-skills-marketplace](../openspec/changes/agent-skills-marketplace/proposal.md)         | Skill package format, Node.js sidecar with `vm.Context` isolation, permission model, in-app marketplace from a GitHub registry.             |
| 4     | [document-wysiwyg](../openspec/changes/document-wysiwyg/proposal.md)                         | Milkdown WYSIWYG, block-anchored comments, Monaco revision diff, suggested-edits flow.                                                      |

Each stage is independently demoable. Stage 1 alone is a credible CLI-flavored product; later stages turn it into a polished platform.

## 15. Implemented YAML schemas (Stage 1)

Stage 1 ships the YAML loaders and validators below. See
[`docs/flows_quickstart.md`](flows_quickstart.md) for a hand-on walkthrough
and [`.cronymax/`](.cronymax/) for working samples.

### `<name>.agent.yaml`

| Field              | Required | Notes                                                       |
| ------------------ | -------- | ----------------------------------------------------------- |
| `name`             | yes      | Stable identifier; referenced from Flows.                   |
| `kind`             | no       | `worker` (default) or `reviewer`.                           |
| `llm`              | yes      | OpenAI-compatible model id (routed through `model_router`). |
| `system_prompt`    | yes      | Free-form Markdown.                                         |
| `memory_namespace` | no       | Defaults to `name`.                                         |
| `tools`            | no       | List of tool names; empty means Space defaults.             |

### `<name>.doc-type.yaml`

| Field                   | Required | Notes                                                  |
| ----------------------- | -------- | ------------------------------------------------------ |
| `name`                  | yes      | Used as the typed-port identifier on Flow edges.       |
| `display_name`          | no       | Shown in the chat panel.                               |
| `description`           | no       | Free text.                                             |
| `required_sections`     | no       | List of `{heading, [min_words], [min_items], [kind]}`. |
| `optional_sections`     | no       | Same shape as `required_sections`.                     |
| `front_matter_required` | no       | List of YAML keys expected in the doc's front-matter.  |

`kind: list` makes a section a Markdown list and enables `min_items`.
`min_words` applies to prose sections.

### `flow.yaml`

| Field                   | Required | Default | Notes                                          |
| ----------------------- | -------- | ------- | ---------------------------------------------- |
| `name`                  | yes      |         | Folder is `.cronymax/flows/<name>/`.           |
| `description`           | no       |         |                                                |
| `agents`                | yes      |         | List of agent names declared in this flow.     |
| `edges[]`               | no       |         | `{from, to, port, [requires_human_approval]}`. |
| `max_review_rounds`     | no       | `3`     | `0` disables the reviewer pipeline.            |
| `on_review_exhausted`   | no       | `halt`  | `halt` or `approve`.                           |
| `reviewer_timeout_secs` | no       | `60`    | Per-reviewer timeout.                          |
| `reviewer_enabled`      | no       | `true`  | Set false to skip reviewers entirely.          |

The first agent in `agents` is the entry point. `port` must match a
declared doc-type name; the producer's `submit_document` payload is
type-checked against the doc-type schema.

### Bridge channels (Stage 1)

| Channel                  | Direction | Payload / Reply                                         |
| ------------------------ | --------- | ------------------------------------------------------- |
| `flow.run.start`         | req       | `{flow_id, [initial_input]}` → `{run_id}`               |
| `flow.run.cancel`        | req       | `{run_id}` → `{ok}`                                     |
| `flow.run.status`        | req       | `{run_id}` → FlowRunState JSON                          |
| `flow.run.list`          | req       | → `{runs:[FlowRunState]}`                               |
| `flow.run.changed`       | event     | FlowRunState (state-transition broadcast)               |
| `event.subscribe`        | req       | `{run_id}` → `{ok}` (replay-then-live trace stream)     |
| `flow.event`             | event     | `TraceEvent` JSON line                                  |
| `mention.user_input`     | req       | `{flow_id, text}` → `{mentions:[name], unknown:[name]}` |
| `review.approve`         | req       | `{flow, run_id, name}` → `{ok}`                         |
| `review.request_changes` | req       | `{flow, run_id, name, [comment]}` → `{ok}`              |
| `review.comment`         | req       | `{flow, run_id, name, body}` → `{ok}`                   |
| `review.list`            | req       | `{flow, run_id, name}` → `{verdicts, comments}`         |
| `review.changed`         | event     | broadcast on every reviews.json mutation                |
| `document.list`          | req       | `{flow}` → `{docs:[{name, latest_revision}]}`           |
| `document.read`          | req       | `{flow, name, [revision]}` → `{content, revision}`      |
