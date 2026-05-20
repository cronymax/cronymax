---
title: Flow Chat Orchestrator — PRD
doc_type: prd
---

# Flow Chat Orchestrator — PRD

## Goal

Transform the `__chat__` session from a one-shot flow-seed into a persistent, multi-turn orchestrator so that users can start, monitor, and review entire flow runs without ever leaving the chat panel. The FlowEditor is demoted to a read-only topology viewer. Every human-required action — approving documents, requesting changes, handling cycle exhaustion, and confirming run completion — is surfaced as a contextual, actionable card in the conversation thread.

---

## Users

| Persona | Description |
|---|---|
| **Developer / Technical User** | Runs `software-dev-cycle` and `bug-fix-loop` flows day-to-day; wants tight feedback loops and clear cycle/reviewer status without switching context. |
| **Product Manager / Designer** | Starts flows from a natural-language brief; reviews and approves PM-produced documents (prototype, PRD) directly in chat. |
| **Power User** | Already familiar with the Flows sidebar and FlowEditor; will tolerate losing the Start Run button in exchange for a more transparent orchestration experience. |

---

## User Stories

1. **As a user**, I want to start a flow run by typing a natural-language brief in chat, so that I don't need to navigate to the Flows sidebar or FlowEditor.
2. **As a user**, I want to see a live Flow Instances Bar above the chat composer that shows the name, run number, and current status of every active run in this session, so that I always know what's happening without scrolling up.
3. **As a user**, I want to expand a run row in the Flow Instances Bar to see every port and its `PortStatus` badge (approved, in-review, pending, failed), so that I can drill down without opening the FlowEditor.
4. **As a user**, I want the chat thread to automatically inject a Document Card when a document is ready for my review, so that I can approve or request changes without navigating away from chat.
5. **As a user**, I want to approve a document directly from a Document Card with a single click, so that the flow continues immediately and I can stay in context.
6. **As a user**, I want to request changes via an inline structured-comment composer attached to a Document Card, so that my feedback is unambiguous and automatically routed back to the responsible agent.
7. **As a user**, I want automated reviewer verdicts (Critic, QA-Critic) to be surfaced in the chat thread as a readable summary — including cycle counts and specific change requests — so that I have full situational awareness without reading raw logs.
8. **As a user**, I want to be notified in chat when a bug-fix loop exceeds its max cycles, and presented with actionable options (resume with extra cycle, ship as-is, halt), so that I can resolve the escalation without hunting for it elsewhere.
9. **As a user**, I want to see a completion summary in chat when all ports reach `APPROVED` and the run finishes, so that I get a clear signal the flow is done and what the outcome was.
10. **As a user**, I want the FlowEditor to show a live read-only topology with per-node status colours and a "← Back to Chat" button, so that I can inspect the graph without being confused by a Start Run button that no longer applies.

---

## Acceptance Criteria

- [ ] Typing a flow brief in chat and sending it calls `flow.start` on the `__chat__` agent and produces a confirmation turn in the thread containing the `flow_run_id` and flow name.
- [ ] A **Flow Instances Bar** renders between the message list and the composer once at least one flow run is associated with the current session; it is hidden when no runs exist.
- [ ] Each row in the Flow Instances Bar displays: flow display name, run number, and a `StatusDot` icon that reflects the current aggregate run state (`thinking`, `in review`, `done`, `failed`, `paused`).
- [ ] Clicking a row in the Flow Instances Bar expands an inline port list showing all ports and their individual `PortStatus` badges (`✓ approved`, `⏳ in-review`, `○ pending`, `✕ failed`); clicking again collapses it.
- [ ] When a document is submitted and all automated reviewers have passed, `__chat__` injects a **Document Card** turn into the conversation containing: document type and revision number, a prose preview snippet (≥ 2 sentences), a "Read full doc ↗" link to the Document Workbench, an **Approve** button, and a **Request Changes** button.
- [ ] Clicking **Approve** on a Document Card calls `flow.approve`, shows a spinner on the button, replaces it with "✓ Approved" for 1.5 s, then collapses the card; a follow-up assistant turn confirms the next agent has been scheduled.
- [ ] Clicking **Request Changes** on a Document Card opens an **inline feedback composer** anchored below the card, supporting: free-text comment field, severity selector (Error / Warning / Info), and an "Add another comment" control for multiple structured comments.
- [ ] Submitting the inline feedback composer calls `flow.request_changes` with the structured comments, collapses the composer, and triggers a chat turn confirming "Requested changes on `<doc>` (revision N). `<Agent>` will revise. (Cycle M of K)".
- [ ] When automated reviewers run, a summary turn is posted in chat listing each reviewer, their verdict (✓ Approved / ⚠ Changes requested), and any specific change-request bullets; the summary also shows the current cycle count.
- [ ] When `on_cycle_exhausted: escalate_to_human` fires, the run transitions to `PAUSED`, the Flow Instances Bar icon for that run changes to an amber pulse, and `__chat__` injects an escalation card listing the open bugs/items and offering three actions: **Resume with extra cycle**, **Ship as-is**, **Halt run**.
- [ ] When a run reaches `COMPLETED`, `__chat__` injects a completion turn summarising outcome (all docs approved, final test-report headline metrics if present); the Flow Instances Bar row updates to `✓ done` and auto-collapses after 5 seconds.
- [ ] The **FlowEditor** no longer contains a Start Run button. It renders a live read-only dagre topology where node fill colour reflects `PortStatus` and edges animate for ~1.5 s on `handoff` events. A **← Back to Chat** button is the only primary CTA.
- [ ] All Document Card buttons use the existing `<Button variant="outline">` (Request Changes) and `<Button variant="primary">` (Approve) components from the design system.
- [ ] The Flow Instances Bar uses `h-9` height in its collapsed per-run state, renders on `surface-1` background, and is separated from the message list by a `border-b-1` divider.
- [ ] Status dots use the shared `StatusDot` component with states mapped as: `activity` → thinking, `attention` → in review / paused, `off` → pending, `error` → failed.

---

## Non-Goals

- **Mobile or web targets**: this feature is scoped to the macOS CEF desktop shell only.
- **Inline document editing within the Document Card**: users who wish to edit a document before approving must open the Document Workbench. Quick-edit in the card is out of scope for this iteration.
- **Starting flows from outside chat**: the Flows sidebar retains its existing start-run path for power users; cross-posting run notifications to a `__chat__` session when no session is open is not required.
- **Persisting chat history across app restarts**: durable conversation history and re-hydration of Document Cards after a restart are out of scope.
- **Multi-session run management**: a single run is associated with the session that started it; broadcasting run events to other open sessions is not required.
- **Changing flow configuration from chat**: users may not edit YAML flow definitions, agent parameters, or `max_cycles` from within the chat interface.
