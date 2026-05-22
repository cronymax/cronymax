---
title: Flow Chat Orchestrator — UX Prototype
doc_type: prototype
---

# Flow Chat Orchestrator — UX Prototype

## Overview

**Feature:** Flow Chat Orchestrator  
**Goal:** Transform the `__chat__` session from a one-shot flow-seed into a persistent, multi-turn orchestrator. Users start, monitor, and review flow runs entirely through the chat panel. The FlowEditor becomes a read-only topology viewer.

---

## Key Screens

### Screen 1 — Chat Panel (Idle, Pre-Run)

```
┌──────────────────────────────────────────────────────────────────┐
│  cronymax                              [sidebar] [+new chat]      │
├──────────────┬───────────────────────────────────────────────────┤
│  SPACES      │                                                   │
│  ─────────   │   ┌── Chat ──────────────────────────────────┐   │
│  Playground  │   │                                           │   │
│              │   │  ┌─────────────────────────────────────┐ │   │
│  FLOWS       │   │  │  👤  You                 10:32 AM    │ │   │
│  ─────────   │   │  │  Start a software-dev-cycle flow     │ │   │
│  ▶ sdc       │   │  │  for the FlowInstancesBar feature.  │ │   │
│  ▶ prd-spec  │   │  └─────────────────────────────────────┘ │   │
│              │   │                                           │   │
│              │   │  ┌─────────────────────────────────────┐ │   │
│              │   │  │  🤖  Assistant            10:32 AM   │ │   │
│              │   │  │  Starting **software-dev-cycle**     │ │   │
│              │   │  │  run #3 for the FlowInstancesBar     │ │   │
│              │   │  │  feature. I'll keep you updated as   │ │   │
│              │   │  │  agents complete each phase.         │ │   │
│              │   │  │                                      │ │   │
│              │   │  │  Run started ✓  `r-1779292561-ab3c`  │ │   │
│              │   │  └─────────────────────────────────────┘ │   │
│              │   │                                           │   │
│              │   │  ────────────────── Flow Instances ────── │   │
│              │   │  ● software-dev-cycle #3  [● thinking]    │   │
│              │   │    pm-design › prototype                  │   │
│              │   │                                           │   │
│              │   │  ┌─────────────────────────────────────┐ │   │
│              │   │  │  Type a message…           [⬆ Send] │ │   │
│              │   │  └─────────────────────────────────────┘ │   │
│              │   └───────────────────────────────────────────┘   │
└──────────────┴───────────────────────────────────────────────────┘
```

**Interaction notes:**
- User types a natural-language brief into the chat input; `__chat__` calls `flow.start` and replies with a confirmation message in-thread.
- A thin **Flow Instances Bar** appears above the composer after a run starts. It shows one row per active run: flow name, run number, and a live status icon.
- The status icon cycles: `● thinking` (LLM active) → `⏳ in review` (waiting on human) → `✓ done` / `✕ failed`.

---

### Screen 2 — Flow Instances Bar (Expanded)

```
  ──────────────────────────────────────── Flow Instances ──────
  ▼ software-dev-cycle #3                            [⏳ review]
    ├─ pm-design     prototype       ✓ approved
    ├─ pm-design     prd             ⏳ awaiting your review
    ├─ rd-design     tech-spec       ○ pending
    ├─ rd-design     code-desc       ○ pending
    └─ qa-testing    test-report     ○ pending
  ─────────────────────────────────────────────────────────────
  ▶ simple-prd-to-spec #1                              [✓ done]
```

**Interaction notes:**
- Clicking the chevron (▼/▶) on any run row expands it to show all ports and their `PortStatus` badges.
- Badge colours: ✓ green (approved), ⏳ amber (in-review), ○ grey (pending), ✕ red (failed/exhausted).
- The bar is hidden entirely when no runs are associated with the current session.

---

### Screen 3 — Review Notification in Chat (Document Ready)

```
  ┌────────────────────────────────────────────────────────┐
  │  🤖  Assistant                              10:47 AM    │
  │                                                        │
  │  PM has submitted the **PRD** for your review.         │
  │  All automated reviewers (Critic) passed ✓             │
  │                                                        │
  │  ┌── 📄 prd · revision 1 ─────────────────────────┐   │
  │  │  **FlowInstancesBar Feature PRD**               │   │
  │  │  Goal: Surface live run status above the chat   │   │
  │  │  prompt so users always know which flows are    │   │
  │  │  active…  [Read full doc ↗]                     │   │
  │  │                                                 │   │
  │  │  [✓ Approve]   [✎ Request Changes]              │   │
  │  └─────────────────────────────────────────────────┘   │
  └────────────────────────────────────────────────────────┘
```

**Interaction notes:**
- When a document reaches human-review stage, `__chat__` injects a turn into the conversation with a **Document Card**: a preview snippet, a link to open the full Workbench, and **Approve** / **Request Changes** inline buttons.
- Clicking **Approve** calls `flow.approve`; the flow continues immediately without navigating away from chat.
- Clicking **Request Changes** opens an inline **feedback composer** (see Screen 4).

---

### Screen 4 — Request Changes Composer (Inline)

```
  ┌────────────────────────────────────────────────────────┐
  │  ✎ Request changes on: prd · revision 1               │
  │  ──────────────────────────────────────────────────    │
  │  ┌──────────────────────────────────────────────────┐  │
  │  │  The "Non-Goals" section is missing. Please add  │  │
  │  │  explicit exclusions for mobile and web targets. │  │
  │  └──────────────────────────────────────────────────┘  │
  │  Severity: [● Error  ○ Warning  ○ Info]                │
  │                                                        │
  │  [+ Add another comment]           [Cancel] [Submit ↗] │
  └────────────────────────────────────────────────────────┘
```

**Interaction notes:**
- The composer supports multiple structured comments (severity + message + optional suggestion).
- Submitting calls `flow.request_changes`; PM is re-invoked automatically with feedback injected into its `InvocationContext`.
- After submission, the chat thread shows a confirmation message: *"Requested changes on prd. PM will revise and resubmit."*

---

### Screen 5 — Reviewer Loop Visible in Chat

```
  ┌───────────────────────────────────────────────────────��┐
  │  🤖  Assistant                              11:02 AM    │
  │                                                        │
  │  RD submitted **tech-spec** (revision 1). Running      │
  │  automated reviewers…                                  │
  │                                                        │
  │    Critic       — ✓ Approved                          │
  │    QA-Critic    — ⚠ Changes requested                  │
  │      · "Missing test environment preconditions"        │
  │      · "Add API contract table for bridge channels"    │
  │                                                        │
  │  RD is being re-invoked with reviewer feedback.        │
  │  (Cycle 1 of 3)                                        │
  └────────────────────────────────────────────────────────┘
```

**Interaction notes:**
- Automated reviewer verdicts are surfaced in chat as a live summary, not hidden in trace logs.
- When all agent reviewers approve, the summary changes to "All reviewers approved. Awaiting your sign-off." and presents the Document Card with Approve/Request Changes.
- Cycle count (n of max_cycles) is shown inline to give the user situational awareness.

---

### Screen 6 — Cycle Exhausted (Bug-Fix Loop)

```
  ┌────────────────────────────────────────────────────────┐
  │  ⚠  Assistant                               02:15 PM    │
  │                                                        │
  │  The bug-fix loop between QA and RD has reached its    │
  │  maximum of **5 cycles** without full resolution.      │
  │                                                        │
  │  3 bugs remain open:                                   │
  │    · #BUG-4  FlowInstancesBar flickers on re-render    │
  │    · #BUG-7  session lock not acquired for fast turns  │
  │    · #BUG-9  `flow.approve` returns 404 on restart     │
  │                                                        │
  │  Run is **paused** — your decision needed:             │
  │  [▶ Resume with extra cycle]  [✓ Ship as-is]           │
  │  [✕ Halt run]                                          │
  └────────────────────────────────────────────────────────┘
```

**Interaction notes:**
- When `on_cycle_exhausted: escalate_to_human` fires, the run transitions to `PAUSED` and `__chat__` surfaces this escalation card.
- User can grant one extra cycle, mark the run as complete despite open bugs, or halt entirely.
- The Flow Instances Bar updates the run's icon to `⚠` to indicate a paused state requiring attention.

---

### Screen 7 — FlowEditor (Demoted to Topology Viewer)

```
┌──────────────────────────────────────────────────────────────────┐
│  Flow: software-dev-cycle                  [← Back to Chat]      │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│   ┌───────────┐    prd     ┌───────────┐  tech-spec  ┌────────┐ │
│   │  pm-design │ ────────► │ rd-design  │ ──────────► │  qa-   │ │
│   │  ✓ done   │◄─ ─ ─ ─ ─ │  ● active  │ ◄─── ─ ─ ─ │testing │ │
│   └───────────┘  (note:    └───────────┘  bug-report  └────────┘ │
│                   no Start                                        │
│                   Run btn)                                        │
│                                                                  │
│  Node status legend:  ○ pending  ● active  ✓ done  ⏳ in review  │
│                                                                  │
│  [View run state.json ↗]                                         │
└──────────────────────────────────────────────────────────────────┘
```

**Interaction notes:**
- The **Start Run** button is removed. The only call-to-action is **← Back to Chat**, reinforcing that `__chat__` is the control surface.
- Node colours reflect live `PortStatus` from the current run.
- The editor is purely informational; no document approval/rejection cards appear here.

---

## User Flows

### Flow A — Starting a Run

1. User opens a chat session in the Playground space.
2. User types: *"Run a software-dev-cycle flow to build the FlowInstancesBar feature."*
3. `__chat__` recognises the intent, calls `flow.list` (internal), then `flow.start("software-dev-cycle", brief)`.
4. Chat replies with a run-started confirmation and the `flow_run_id`.
5. Flow Instances Bar appears above the composer showing the new run at `● thinking`.
6. PM agent begins producing the prototype; status updates flow into the bar in real-time.

### Flow B — Approving a Document

1. PM submits `prd`; Critic agent runs automatically.
2. Critic approves → `__chat__` receives a runtime-initiated turn.
3. Chat shows a Document Card with snippet, Workbench link, and Approve / Request Changes.
4. User clicks **Approve** → `flow.approve` called → RD agent is scheduled immediately.
5. Chat replies: *"PRD approved. RD is now writing the tech-spec."*

### Flow C — Requesting Changes

1. User reviews the tech-spec in the Workbench, finds an issue.
2. Returns to chat, clicks **Request Changes** on the tech-spec Document Card.
3. Fills in one or more structured comments in the inline composer.
4. Submits → RD is re-invoked with `review_comments` in `InvocationContext`.
5. Chat shows: *"Requested changes on tech-spec (revision 1). RD will revise. (Cycle 1 of 3)"*

### Flow D — Run Completion

1. QA submits `test-report`; Critic approves; human approves.
2. All ports reach `APPROVED` state; flow run transitions to `COMPLETED`.
3. `__chat__` receives a completion notification turn.
4. Chat displays: *"software-dev-cycle #3 complete ✓. All documents approved. Test report: 97% pass rate, ship recommended."*
5. Flow Instances Bar row updates icon to `✓` and collapses after 5 seconds.

---

## Interactions & Micro-UX Details

| Interaction | Behaviour |
|---|---|
| **Hover on Flow Instances Bar row** | Expand chevron highlights; cursor becomes pointer |
| **Click run row** | Expands port list inline; click again to collapse |
| **Approve via Document Card** | Button shows spinner → swaps to "✓ Approved" for 1.5 s → card collapses |
| **Request Changes submit** | Composer collapses; chat shows inline acknowledgement bubble |
| **Run paused (cycle exhausted)** | Bar icon pulses amber; escalation card auto-scrolls into view |
| **FlowEditor topology** | Node colours animate on `agent_status` events; edges animate for ~1.5 s on `handoff` events |

---

## Design Tokens & Style Notes

- Document Cards use `surface-2` background, `border-1` outline, `radius-lg` corners — consistent with the existing `ConversationBlockView` card style.
- Status icons use the shared `StatusDot` component (`useStatusDotState` hook: `off | activity | attention | error`).
- Flow Instances Bar is a single-line `h-9` strip in `surface-1` with `border-b-1` separator; expands to auto height when a run is clicked.
- Reviewer verdict summaries are rendered as compact `<ul>` lists with `text-sm` and severity-coloured left-border accents.
- All Document Card buttons use existing `<Button variant="outline">` and `<Button variant="primary">` components.

---

## Open UX Questions

1. **Auto-dismiss for completed runs** — Should the Flow Instances Bar auto-collapse completed runs after N seconds, or keep them visible until the user explicitly dismisses? Proposed: auto-collapse after 5 s with a brief `✓ done` flash.
2. **Multiple pending reviews** — If three documents are simultaneously awaiting human review, should `__chat__` inject one turn per document, or bundle them into a single "N documents need your review" card? Proposed: bundle, with expandable per-document sections.
3. **Chat-initiated vs. sidebar-initiated flows** — When a flow is started from the Flows sidebar (legacy path, for power users), should it still post a notification turn to the most recent `__chat__` session? Proposed: yes, if a session is open; otherwise, no-op (flow runs as before).
4. **Inline vs. Workbench approval** — Should users be able to edit a document directly in the Document Card (quick inline markdown tweak) before approving? Proposed: out of scope for this iteration; the Workbench link covers this.
