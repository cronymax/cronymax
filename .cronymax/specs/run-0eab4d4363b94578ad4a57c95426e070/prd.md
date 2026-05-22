---
title: Cronymax — Product Requirements Document
doc_type: prd
---

# Cronymax — Product Requirements Document

---

## Goal

Deliver a macOS AI desktop shell that lets individual developers and small engineering teams run multi-agent automation workflows directly on their local machine, without leaving their codebase. The shell combines a CEF-based browser surface, a native PTY/sandbox runtime, and a typed Flows orchestration engine so that AI agents can read, write, and execute in a workspace while humans retain clear visibility and control over every decision.

---

## Users

| Persona | Description |
|---|---|
| **Solo developer** | Runs one-person projects; wants AI to automate boilerplate, code review, and documentation but needs to stay in control of what gets committed. |
| **Small team lead** | Coordinates 2–6 engineers; wants to encode team process as reusable Flows (PM → RD → QA) and route agent output through structured human-approval gates. |
| **DevOps / platform engineer** | Cares about sandboxing, permission grants, and auditability; needs to see exactly what shell commands an agent ran and at what risk level. |

All three personas work on macOS 13+ (arm64) and are comfortable with terminal tooling.

---

## User Stories

### Flows & Orchestration

1. **As a developer**, I want to pick a preset Flow (e.g. `simple-prd-to-spec`) from a sidebar dropdown and start a run with one click, so I can kick off a multi-agent pipeline without writing any YAML.

2. **As a team lead**, I want to hand-edit agent and flow YAML files in `.cronymax/` and have the app hot-reload them automatically, so I can iterate on team workflows without restarting the app.

3. **As a team lead**, I want to see a DAG visualization of my Flow with live node status (thinking / blocked / done) during a run, so I can immediately spot which agent is the bottleneck.

4. **As a developer**, I want to post `@mention` messages in the Channel view to nudge or redirect a specific agent mid-run, so I don't have to cancel and restart when requirements shift.

5. **As a developer**, I want a Cancel button in the Channel header when a run is active, so I can abort a runaway pipeline instantly.

6. **As a team lead**, I want to configure `max_review_rounds` and `on_review_exhausted` per flow edge, so the system never loops forever and I control whether it auto-approves or halts.

### Document Workbench

7. **As a developer**, I want to open agent-produced Markdown documents in a WYSIWYG editor, save edits with ⌘S, and have the result stored as a new numbered revision, so I can polish AI output without touching raw Markdown.

8. **As a reviewer**, I want to switch to a side-by-side diff view between any two revisions of a document, so I can quickly see what changed between agent drafts.

9. **As a reviewer**, I want to leave block-anchored comments with optional suggested text replacements, so I can give the next agent (or a human) precise, actionable feedback.

10. **As a reviewer**, I want to click **Accept** on a suggested edit and have the document updated atomically (new revision, comment resolved), so accepting feedback is a single action rather than a copy-paste workflow.

11. **As a developer**, I want to deep-link to a specific document block via `#block-<id>` so I can share a URL that jumps straight to the relevant paragraph in the workbench.

### Inbox & Notifications

12. **As a developer**, I want a unified Inbox that lists every item needing my attention (document approvals, review requests, agent errors) with filter tabs (Unread / All / Snoozed), so I never miss a required human gate.

13. **As a developer**, I want to snooze an inbox item for 1 h / 4 h / Tomorrow / Custom, so low-priority items don't clutter my attention when I'm in flow.

14. **As a developer**, I want macOS banner notifications for needs-action events and a Dock badge showing the unread count, so I'm alerted even when the app is in the background.

### Native Runtime & Security

15. **As a DevOps engineer**, I want every shell command the agent runs to be classified by risk level (safe / moderate / dangerous) before execution, so I can configure automatic allow/deny policies per workspace.

16. **As a developer**, I want a permission broker UI that shows me what a sandboxed agent is requesting and lets me allow or deny it inline (not via a CLI prompt), so I can make quick decisions without switching context.

17. **As a developer**, I want PTY sessions and agent command traces persisted to disk so I can audit exactly what ran in a previous session even after a restart.

### General Shell

18. **As a developer**, I want hot-reload (React Fast Refresh) when `CRONYMAX_DEV=1` is set so frontend iterations are instant during local development.

19. **As a developer**, I want workspace data stored under `~/Library/Application Support/app.cronymax/` in a versioned layout (with automatic migration) so upgrading the app never loses my run history.

---

## Acceptance Criteria

### AC-1 — Flow Lifecycle
- A user can start, pause, resume, and cancel a Flow Run entirely from the UI.
- The Channel panel shows every AppEvent (text, document, review, handoff, error, system) with correct kind-specific rendering within 500 ms of the event being appended.
- The DAG editor correctly reflects `agent_status` transitions (idle → thinking → blocked → done) during a run without a page reload.

### AC-2 — Human Approval Gates
- When a flow edge sets `requires_human_approval: true`, the run halts and surfaces a `DocumentCard` with **Approve** / **Request changes** buttons in the Channel view.
- Clicking **Approve** unblocks the run in ≤ 1 s.
- Clicking **Request changes** posts a `review_event` and the run stays paused.

### AC-3 — Document Workbench
- WYSIWYG, Source, and Diff modes are all reachable via the mode toggle without losing `flow`, `doc`, and `run_id` URL state.
- ⌘S in WYSIWYG and Source modes writes a new revision within 2 s and surfaces the new revision number in the UI.
- Diff mode renders correctly for any two valid revision numbers (`from`/`to` params).
- Block IDs are auto-assigned on save for any top-level block that lacks one.
- A `#block-<id>` deep-link scrolls the target block into view and applies a 1.5 s pulse highlight on mount.

### AC-4 — Suggested Edits
- **Accept** on a suggested edit produces a new revision, resolves the comment (`resolved_in_rev` set), and refreshes the rail without a manual page reload.
- Accepting a suggestion against a stale revision returns a clear inline `stale_revision` error banner (not a silent failure or unhandled exception).
- **Dismiss** appends a `(suggestion dismissed)` audit comment.

### AC-5 — Inbox & Notifications
- The Inbox lists all unread items on load and auto-refreshes on `review_event`, `error`, and `handoff` broadcasts.
- Snooze options (1 h / 4 h / Tomorrow / Custom) correctly set `snooze_until` and move the row to the Snoozed filter.
- macOS banner notifications fire for needs-action events when the user has granted notification authorization and the kind is enabled in `notification_prefs`.
- Dock badge count matches the inbox unread count after every event append.

### AC-6 — Native Sandbox
- Every command run by an agent is classified by the risk classifier before execution; the classification result is logged to the run trace.
- Sandboxed command execution uses the Seatbelt profile generated by the profile compiler; direct access to paths outside the declared workspace is denied at the OS level.
- PTY history is persisted to `$userDataDir/cronymax/profiles/<id>/workspaces/<ws>/pty/` and survives an app restart.

### AC-7 — Data Persistence & Migration
- On first launch after an upgrade, `LayoutMigrator` runs all pending version steps in order (V0 → V4) without deleting source data until migration succeeds.
- Each migration step is idempotent (sentinel files prevent re-runs).
- Run state (`state.json`) enables a paused Run to survive an app restart and resume correctly.

### AC-8 — Dev Tooling
- `CRONYMAX_DEV=1` with a running Vite dev server enables React Fast Refresh across all panels with no full bundle rebuild.
- `native_probe` CLI correctly exercises `policy`, `agent /read`, `agent /write`, and `agent /exec` subcommands without CEF.

---

## Non-Goals

- **Windows / Linux support** — the runtime relies on macOS-only APIs (Seatbelt, `UNUserNotificationCenter`, PTY); other platforms are explicitly out of scope for this release.
- **Remote / cloud agents** — all agent execution is local; no hosted inference infrastructure is included.
- **Multi-user collaboration** — concurrent editing by multiple humans on the same run is not supported; the comment rail and approval model assume a single reviewer at a time.
- **Flow YAML GUI editor** — the DAG editor is read-only in this release; structural flow edits are done by hand in YAML files.
- **LLM provider management UI** — selecting and configuring the underlying model/API key is out of scope; configuration is handled via environment variables or config files.
- **Mobile or tablet form factors** — the product targets macOS desktop only.
- **Replacing Xterm.js in this release** — the terminal-lite implementation remains; full Xterm.js integration is deferred pending dependency vendoring decisions.
