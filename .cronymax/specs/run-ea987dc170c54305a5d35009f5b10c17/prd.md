---
title: Flow Thread Sessions — PRD
doc_type: prd
---

# Flow Thread Sessions — Product Requirements Document

## Goal

Transform every flow run started from the Chat panel into a transparent, interactive **inline thread** — a live Slack-style conversation feed that surfaces per-agent streaming activity, tool calls, and document reviews in real time. Users should be able to follow exactly what each agent is doing, understand why a document turned out the way it did, and (in a subsequent phase) directly message a specific agent node mid-run using `@node-name` routing.

## Users

**Primary:** Developers and technical PMs using Cronymax to run multi-agent flows (PM → RD → QA cycles) from the Chat panel. They currently approve or reject documents with no visibility into the reasoning that produced them.

**Secondary:** Any Cronymax user who triggers an agent flow and wants to audit or intervene in the run without switching to a separate Workbench panel.

## User Stories

1. **As a user starting a flow run**, I want to see a summary card appear in the chat timeline so that I know the run has started and can monitor progress without leaving the conversation.
2. **As a user monitoring a flow**, I want to click "View thread" and see a chronological feed of every agent's messages and tool calls so that I understand what each node is doing in real time.
3. **As a user reviewing a document**, I want pending document reviews to appear inline in the thread view so that I don't have to switch to the Workbench panel to approve or reject.
4. **As a user who wants context**, I want to click "← Back" and return to the main chat timeline with the trajectory strip still visible so that I can continue other conversations without losing flow state.
5. **As a user in Chunk 2**, I want to type `@rd-impl please add soft deletes` in the thread prompt and have that node re-invoked with my feedback so that I can course-correct without restarting the entire flow.

## Acceptance Criteria

1. When a flow run starts, a `FlowThreadSummary` card appears beneath the triggering message within **1 second** of the first `run_status` event arriving.
2. The `FlowThreadSummary` card displays a compact trajectory strip showing per-node status chips (✓ done, ◉ running, ○ pending) and a live event count that increments as node events arrive.
3. Clicking **"View thread"** hides the block timeline and renders the thread view with a `← Back` header and a node-status strip at the top.
4. The thread view shows at least **one event per node** that has started, rendered in arrival order with the agent name label and text content or tool call details.
5. Tool call entries display the tool name, arguments summary, and a resolved status (✓ / running… / ✗) with elapsed time when completed.
6. Pending `FlowDocReviewPanel` cards are embedded in the thread view — no separate panel switch is required to act on a review.
7. Clicking **"← Back"** returns to the main chat timeline; the `FlowThreadSummary` fork block remains visible with the latest trajectory strip state.
8. While the user is in the main chat view (thread not open), the sidebar shows `FlowDocReviewPanel` with `showHistory={false}` — pending reviews only — so reviews are never silently missed.
9. The thread view is read-only with the prompt editor disabled until Chunk 2 is delivered; the input field is visible but shows a placeholder explaining the limitation.
10. *(Chunk 2)* Flow node sub-runs are persisted in a child session (`child_session_id`) separate from the parent chat session; the parent session does not contain node-level token or tool events.
11. *(Chunk 2)* An `@node-name message` sent from the thread prompt re-invokes the named node with `trigger.kind = "human_feedback"` and `review_comments` populated with the message text.
12. *(Chunk 2)* The `@` picker in the thread prompt is pre-populated with all node names from the active flow topology.
13. *(Chunk 2)* Messages sent without an `@` prefix are routed to the `__chat__` orchestrator in the child session, not to any specific node.

## Non-Goals

- Inline document editing inside the thread view (editing remains in the Workbench panel).
- Diff view between document revisions within the thread.
- Auto-scroll to the currently active node bubble.
- Interrupting a live running node mid-stream (only done/pending nodes can be addressed in Chunk 2).
- Node-to-node direct messaging.
- Historical thread replay from prior sessions (only the current live run is shown).
- Any changes to the Workbench panel's existing review or diff functionality.
