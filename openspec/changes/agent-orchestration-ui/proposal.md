## Why

`agent-document-orchestration` ships the Flow + Document + Review machinery with a minimal CLI-flavored UI: hand-edited YAML, raw markdown, a basic chat panel. The orchestration UX is the product's primary differentiator — users need to _see_ agents collaborating, _edit_ Flows visually, and _be notified_ asynchronously when something needs their attention. This change builds the three first-class UI surfaces that turn the foundation into a product: a Slack-style per-Flow channel view, a visual Flow graph editor, and an inbox + status-dot notification system.

## What Changes

- **NEW** Per-Flow channel view (Slack-style): each Flow gets a persistent channel showing typed messages (`text`, `agent_status`, `document_event`, `review_event`, `handoff`, `error`, `system`). Documents render as rich cards; reviews render as threaded replies under the doc card. The user is a member of every channel they own and can post free text or `@`-mention agents to trigger work.
- **NEW** Threading: every Document submission is a thread root; comments and revisions live in the thread. Channel timeline stays scannable.
- **NEW** Visual Flow editor (React Flow canvas): drag agents from a palette, draw typed-port edges, configure agents in a side panel. Document-type compatibility is enforced at edge connection time. Auto-layout (dagre/elk) on first render; manual positions preserved after.
- **NEW** Live execution overlay: while a Flow Run is active, nodes light up by status (idle / thinking / submitted / reviewing / blocked); same canvas, run-mode toggle.
- **NEW** Flow YAML ↔ canvas round-trip: canvas state serializes to the existing `flow.yaml` (source of truth from `agent-document-orchestration`); manual YAML edits re-render correctly. Layout sidecar `flow.layout.json` (committable; gitignore optional).
- **NEW** Notification inbox: typed events project into a unified inbox view (review requests, mentions, completions, failures, permission requests, cost-threshold alerts). Read/unread/snooze states; per-Flow and global filters.
- **NEW** Status bar dot: ambient indicator (gray idle / blue activity / orange needs-attention / red error). Hover shows summary; click opens inbox.
- **NEW** OS notifications: configurable per event type; default to "needs-action" events only. macOS dock badge mirrors unread count.
- **NEW** Event bus: a single typed-event stream is the single source of truth that the channel view, inbox, and status dot all project from.
- **MODIFIED** `flow-orchestration`: emits a stable typed-event stream (currently scoped to Run trace JSONL); UI subscribes via the bridge.
- **MODIFIED** `document-collaboration`: review thread events surface as channel threads; comment posting can originate from chat replies, not just the workbench.

## Capabilities

### New Capabilities

- `flow-channel-view`: per-Flow Slack-style channel — typed messages, threading, document/review cards, `@mention` from chat, run dividers.
- `flow-visual-editor`: React Flow-based canvas — agent palette, typed-port edges, side-panel config, auto-layout, YAML round-trip, live-run overlay.
- `notification-inbox`: typed-event projection — inbox panel with filters/snooze, status-bar dot, OS notifications, dock badge.
- `agent-event-bus`: single typed-event stream powering all UI projections; persists to SQLite for replay.

### Modified Capabilities

- `flow-orchestration`: formalizes the typed-event schema and persists it; adds `flow.layout.json` sidecar.
- `document-collaboration`: comments can be posted from channel threads as well as the workbench; both write to the same `reviews.json`.

## Impact

- **NEW `web/orchestration/`**: channel view, Flow editor canvas, inbox UI, status-dot component. React Flow becomes a frontend dependency.
- **NEW `src/event_bus/`**: typed event store (SQLite-backed), subscriptions, projection helpers.
- **`src/cef_app/bridge_handler.{h,cc}`**: new channels `events.subscribe`, `events.list`, `inbox.*`, `notifications.*`.
- **macOS integration**: `NSUserNotification` (or `UNUserNotificationCenter`) for OS notifications; dock badge via `NSDockTile`. Adds a thin Objective-C++ shim under `src/platform/macos/`.
- **Frontend deps added**: React Flow, dagre (or elk-js), date-fns (relative timestamps).
- **No new C++ third-party deps**.
- **Migration**: `flow-orchestration`'s previous unstable trace format becomes the typed-event schema; one-time conversion of existing runs.
