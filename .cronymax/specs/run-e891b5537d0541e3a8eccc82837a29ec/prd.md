---
title: PRD: Software Dev Cycle Preset Flow (PM → RD → QA)
doc_type: prd
---

# PRD: Software Dev Cycle Preset Flow (PM → RD → QA)

## Goal

Provide an opinionated, ready-to-run multi-agent workflow — the **software-dev-cycle preset** — that takes a product idea from initial UX sketch all the way through implementation and verified testing, with no hand-authored YAML required. The preset ships inside cronymax's Flow engine and is the canonical demonstration of the platform's end-to-end agentic capabilities.

---

## Users

| Persona | Description |
|---|---|
| **Solo developer / indie hacker** | Wants to move fast from idea to shipped code; happy to delegate PM and QA work to agents but wants final control over each gate. |
| **Small engineering team** | Two to five engineers who want to assign PM and QA roles to agents and focus human attention on code review and architecture. |
| **Platform evaluator** | Evaluating cronymax as an agentic platform; needs a compelling end-to-end demo that runs out of the box. |
| **Flow author (power user)** | Uses the preset as a starting template and customises agent prompts, reviewer assignments, and LLM providers without rewriting the engine. |

---

## User Stories

### Flow Setup & Launch

1. **As a developer**, I want to start the `software-dev-cycle` flow from the Flows sidebar with a single click, so I don't have to write any YAML to begin a structured PM → RD → QA cycle.
2. **As a flow author**, I want to configure each agent's LLM provider (OpenAI-compatible endpoint or GitHub Copilot) in a settings panel, so I can route different agents to different models without editing environment variables.
3. **As a developer**, I want GitHub Copilot authentication to use the standard device-flow (show code → approve in browser → auto-close modal), so I don't need to manage personal access tokens or OAuth redirect servers.

### PM Phase

4. **As a PM agent**, I want to produce a `prototype` document (UX sketches, key screens, user flows) as my first output, so stakeholders can review the design intent before any engineering begins.
5. **As a human reviewer**, I want to approve or request changes on the `prototype` document in the Channel view, so the PM agent can iterate before the PRD is written.
6. **As a PM agent**, I want to be automatically re-invoked after my prototype is approved, with an `InvocationContext` message telling me the next pending port is `prd`, so I continue the cycle without human re-triggering.
7. **As a human reviewer**, I want the `prd` document to pass both human approval and the Critic agent before it routes to RD, so only well-formed requirements move forward.

### RD Phase

8. **As an RD agent**, I want to receive the approved PRD and produce a `tech-spec` (reviewed by human + Critic + QA-Critic), a `code-description` (reviewed by human + Critic), and a `submit-for-testing` handoff to QA, all from a single flow node invocation cycle.
9. **As a human reviewer**, I want the `tech-spec` to be reviewed by the QA-Critic agent before approval, so testability gaps are caught before implementation.
10. **As an RD agent**, I want to be re-invoked with full InvocationContext when my prior output is approved, so I always know which port (tech-spec → code-description → submit-for-testing) to produce next.

### QA Phase

11. **As a QA agent**, I want to receive both the approved `tech-spec` and the `submit-for-testing` handoff (AND-join gate) before starting my testing cycle, so I never test against an incomplete spec.
12. **As a QA agent**, I want to produce `test-cases` (reviewed by RD + Critic), file `bug-reports` back to RD, and ultimately produce a `test-report` (reviewed by human + Critic), all within the same flow node.
13. **As a developer**, I want the bug-report → patch-note loop to be capped at five cycles with escalation to a human if exhausted, so runaway automated cycles cannot occur.
14. **As an RD agent** (in patch mode), I want to receive a `bug-report` at a dedicated `rd-patch` node and return a `patch-note` that routes back to QA, so defect resolution is traceable and isolated from initial implementation.

### Document Workbench

15. **As a reviewer**, I want to view any flow-produced document (prototype, PRD, tech-spec, etc.) in the Document Workbench with WYSIWYG and source modes, inline diff against prior revisions, and a comment rail, so I can give precise, anchored feedback.
16. **As a reviewer**, I want to accept suggested edits with one click, automatically creating a new document revision, so the review round-trip is fast.

### Observability & Control

17. **As a developer**, I want the Flows Channel view to show every agent event (status, document produced, reviewer decision, handoff) in chronological order with kind-aware components, so I always know where the run stands.
18. **As a developer**, I want the Flow Editor to highlight active nodes (thinking / blocked / done) and animate edge transitions during a run, so I get a live visual of the current pipeline state.
19. **As a developer**, I want to cancel an active run from the Channel header pill, so I can abort a runaway cycle without killing the app.
20. **As a developer**, I want the run state (port completion, invocation history) to survive an app restart, so I can resume a long-running cycle after rebooting.

---

## Acceptance Criteria

### AC-1: Preset availability
- The `software-dev-cycle` flow appears in the Flows sidebar on first launch; no manual YAML authoring is required.
- The flow contains four agent nodes: `pm-design`, `rd-design`, `qa-testing`, `rd-patch`.

### AC-2: Agent re-invocation
- After a port's document is approved, the producing agent is automatically re-invoked within 5 seconds with an `InvocationContext` system message identifying the next pending port and listing all approved documents in the run.
- Port completion state persists in `state.json` and survives an app restart; ports with status `PENDING` at restart are resumed correctly.

### AC-3: Reviewer pipeline
- `prototype`: human approval only.
- `prd`: human + Critic; does not route to RD until both approve.
- `tech-spec`: human + Critic + QA-Critic; does not route to QA until all three approve.
- `code-description`: human + Critic.
- `submit-for-testing`: no reviewer; routes directly to `qa-testing`.
- `test-cases`: reviewed by RD + Critic.
- `bug-report`: routes to `rd-patch`; cycle capped at 5; on exhaustion the run is escalated to human (status `needs_human`).
- `test-report`: human + Critic.

### AC-4: AND-join gate
- `qa-testing` does not start until **both** `tech-spec` and `submit-for-testing` are present and approved in the run.

### AC-5: LLM provider registry
- Users can add, edit, and remove named LLM providers from a Settings panel.
- API keys are stored exclusively in the macOS Keychain; they never appear in `providers.json` or any log file.
- GitHub Copilot authentication uses device flow; the settings modal shows the user code and URL, polls automatically, and closes on success or shows an error on timeout.
- Existing single-global-config workspaces are automatically migrated to the new registry on first launch after update; migration is idempotent.

### AC-6: test_runner tools
- `test_runner.discover` returns the list of detected test runners for the current workspace (Jest, Vitest, pytest, Go test).
- `test_runner.run_suite` executes the appropriate runner, parses structured JSON reporter output, and returns a typed result object.
- `test_runner.get_last_report` returns the most recent `test_runner.run_suite` result for the session.
- If a JSON reporter is not configured, `test_runner.run_suite` returns `reporter_not_configured` rather than failing silently.

### AC-7: Document Workbench integration
- All five document types (`prototype`, `prd`, `tech-spec`, `bug-report`, `test-report`) open correctly in the Document Workbench.
- WYSIWYG and source (Monaco) modes are available for each type.
- Side-by-side diff is available when more than one revision exists.
- Approving or requesting changes from the Workbench is reflected in the Channel view within 2 seconds.

### AC-8: Run observability
- Channel view renders all event kinds (`text`, `document_event`, `review_event`, `agent_status`, `handoff`, `system`, `error`) with correct kind-aware components.
- Flow Editor highlights the currently active node and animates handoff edges during a live run.
- Cancelling a run from the Channel header pill stops all in-flight agent tasks and marks the run `cancelled` in the sidebar.

### AC-9: Review limits
- A single document port undergoes at most 3 review rounds (`max_review_rounds: 3`); on exhaustion the run is halted and flagged to the human.

---

## Non-Goals

- **Visual flow editor persistence**: Dragging nodes in the Editor does not persist layout to disk in this version (`flow.save` / `flow.layout.save` not implemented).
- **WYSIWYG Mermaid rendering for prototype docs**: Mermaid diagrams in `prototype` documents render as fenced code blocks until the `document-wysiwyg` change ships.
- **Multi-human collaborative review**: Only one human reviewer seat per review gate; concurrent multi-user approval is out of scope.
- **Skills Marketplace migration for test_runner**: `test_runner.*` tools ship in the agent-entity layer; marketplace migration is explicitly deferred.
- **Windows / Linux support**: The LLM provider registry and Keychain integration are macOS-only in this version.
- **Custom flow topologies via UI**: The preset flow YAML is the only supported topology; a drag-to-connect editor is deferred to `agent-orchestration-ui`.
- **Real-time multi-workspace event fan-out**: Events are scoped to a single active workspace session.
