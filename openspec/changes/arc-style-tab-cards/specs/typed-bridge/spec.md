## ADDED Requirements

### Requirement: New tab/toolbar/chrome channels

The typed bridge channel registry SHALL include the following new channels with Zod-validated payloads:

- `shell.tab_switch` (renderer → C++): `{ tabId: string }`.
- `shell.tab_open_singleton` (renderer → C++, awaitable): `{ kind: TabKind }` → `{ tabId: string, created: boolean }`.
- `shell.tab_close` (renderer → C++): `{ tabId: string }`.
- `shell.tabs_list` (C++ → renderer event): `{ tabs: TabSummary[] }` (full replacement).
- `shell.tab_activated` (C++ → renderer event): `{ tabId: string }`.
- `tab.set_toolbar_state` (renderer → C++): `{ tabId: string, state: ToolbarState }` where `ToolbarState` is a Zod discriminated union over `kind`.
- `tab.set_chrome_theme` (renderer → C++): `{ tabId: string, color: string | null }`.

`TabKind`, `TabSummary`, and `ToolbarState` SHALL be defined as Zod discriminated unions in `web/src/shared/types/index.ts` and consumed by the channel schemas in `web/src/shared/bridge_channels.ts`.

#### Scenario: Channels are registered

- **WHEN** the channel registry is enumerated
- **THEN** all seven channels above are present with valid Zod schemas

#### Scenario: Discriminated union narrows on kind

- **WHEN** a `tab.set_toolbar_state` payload arrives with `state.kind = "web"`
- **THEN** Zod narrows validation to the web variant's fields and rejects payloads carrying terminal/chat/agent/graph fields

## REMOVED Requirements

### Requirement: shell.show_panel channel

**Reason**: Replaced by `shell.tab_switch`. There is no longer a panel enum to switch between; the unit of switching is a tab id.

**Migration**: Replace any `bridge.send("shell.show_panel", { panel })` call with the appropriate `shell.tab_switch` or `shell.tab_open_singleton` + `shell.tab_switch` sequence.

#### Scenario: Channel is gone

- **WHEN** the channel registry is enumerated
- **THEN** no entry named `shell.show_panel` exists

---

### Requirement: topbar.url_changed and topbar.panel_changed events

**Reason**: With per-tab native toolbars, the toolbar reads URL state directly from its owning `CefBrowser` and reads identity directly from its `Tab`. There is no separate topbar renderer needing to be told what to display.

**Migration**: Subscribers MUST migrate to `shell.tab_activated` (for "which tab is now active") and `shell.tabs_list` (for tab metadata changes including title/URL).

#### Scenario: Channels are gone

- **WHEN** the channel registry is enumerated
- **THEN** no entries named `topbar.url_changed` or `topbar.panel_changed` exist

---

### Requirement: shell.set_drag_regions channel

**Reason**: Drag regions are now statically known on the C++ side (toolbar strip + sidebar top inset) and applied directly via `mac_view_style.mm`. JS-driven region pumping is no longer needed.

**Migration**: Remove all `bridge.send("shell.set_drag_regions", …)` calls and the `useDragRegions` hook that produced them.

#### Scenario: Channel is gone

- **WHEN** the channel registry is enumerated
- **THEN** no entry named `shell.set_drag_regions` exists
