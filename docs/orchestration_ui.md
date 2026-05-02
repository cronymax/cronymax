# Cronymax Orchestration UI

This document describes the **agent-orchestration-ui** surfaces introduced in
the change set of the same name. It covers three React panels that consume the
typed `EventBus` API exposed by the C++ host.

## Surfaces

### 1. Channel — `web/orchestration/channel.html?flow=<id>[&run=<id>]`

A scoped, real-time view of every `AppEvent` for one Flow (and optionally
one Run). Renders typed events with kind-aware components:

| Kind             | Component                                       |
| ---------------- | ----------------------------------------------- |
| `text`           | `<TextBubble>` with `@mention` chips            |
| `document_event` | `<DocumentCard>` with Approve / Request-changes |
| `review_event`   | `<ReviewReply>` (compact log line)              |
| `agent_status`   | inline log line                                 |
| `handoff`        | centered transition line                        |
| `system`         | `<RunDivider>` (run started / paused / etc.)    |
| `error`          | `<ErrorBanners>` (dismissible, sessionStorage)  |

**Composer**: parses `@agent` mentions and posts via `events.append`
(kind=text). Cmd/Ctrl+Enter submits. Unknown mentions render with a red
ring (visual hint only — server still accepts them).

**Header pill**: derived from latest `system.subkind`. When the run is
active (`run_started` or `run_paused`), a `Cancel` button appears wired to
`flow.run.cancel`.

### 2. Editor — `web/orchestration/editor.html?flow=<id>[&mode=run]`

React Flow v12 graph viewer with `dagre` auto-layout. Loads the flow
definition via `flow.load`, places agent nodes left-to-right, and renders
labelled edges. Two modes:

- **Edit** (default): nodes are draggable but changes are not persisted —
  C++ side does not yet expose `flow.save` or `flow.layout.save`. The
  editor is currently read-only with respect to the on-disk flow YAML.
- **Run**: subscribes to `events.subscribe` and projects:
  - `agent_status` → node fill (`thinking`/`blocked`/`done`)
  - `handoff` → animated emerald edge for ~1.5 s
  - `document_event` → blue badge on producer node

### 3. Inbox — `web/orchestration/inbox.html`

Lists rows from `inbox.list` (filterable by `unread` / `read` / `snoozed`
/ `all`). Auto-refreshes when relevant `event` broadcasts arrive
(`review_event`, `error`, `handoff`).

Per-row actions:

- **Acknowledge** → `inbox.read`
- **Snooze** dropdown (1h / 4h / Tomorrow / Custom hours) → `inbox.snooze`

## Status dot

`useStatusDotState()` (in `web/src/shared/hooks/useStatusDotState.tsx`)
returns `off | activity | attention | error` based on inbox counts and
event broadcasts. The companion `<StatusDot>` component renders a 6 px
circle suitable for embedding in a panel title bar. (Wiring into every
panel template is deferred — the hook and component are ready to use.)

## Bridge channels (added)

| Channel                       | Direction | Purpose                                |
| ----------------------------- | --------- | -------------------------------------- |
| `events.list`                 | request   | Paginated history (newest-first)       |
| `events.subscribe`            | request   | Begin live broadcast on `event`        |
| `events.append`               | request   | Post a `text` event (with `@mentions`) |
| `inbox.list`                  | request   | Triaged needs-action rows              |
| `inbox.read` / `unread`       | request   | Toggle row state                       |
| `inbox.snooze`                | request   | Set `snooze_until` (unix ms)           |
| `notifications.get_prefs`     | request   | List enabled kinds                     |
| `notifications.set_kind_pref` | request   | Toggle one kind's notification         |
| `event` (broadcast)           | event     | One AppEvent envelope per `Append`     |

The legacy `event.subscribe` channel is **deprecated** and now returns
HTTP 410 with a one-shot console warning. Renderer code must migrate to
`events.subscribe` (note the plural). See
[bridge channel registry](../web/src/shared/bridge_channels.ts) and the
[AppEvent schema](../web/src/shared/types/events.ts) for full type
definitions.

## macOS native

`app/platform/macos/notifications.{h,mm}` exposes a thin C++ API around
`UNUserNotificationCenter` and `NSDockTile.badgeLabel`. `SpaceManager`
subscribes to its `EventBus` and:

- Posts a banner notification for needs-action events when the kind is
  enabled in `notification_prefs` and the user has granted authorization.
- Refreshes the dock-tile badge with the unread count after every
  `Append`.

A `notifications_stub.cc` provides no-op implementations on non-Apple
platforms.
