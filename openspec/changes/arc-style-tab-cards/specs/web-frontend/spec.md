## REMOVED Requirements

### Requirement: Topbar panel entry

**Reason**: The topbar is replaced by per-tab native CEF Views toolbars (see `tab-toolbar` capability). There is no longer a top-level topbar panel.

**Migration**:

- The `topbar/` entry is removed from `web/vite.config.ts`'s `rollupOptions.input`.
- The `web/src/panels/topbar/` source tree is deleted.
- The C++ side stops creating a topbar `CefBrowserView`; toolbar widgets are constructed natively per tab by `TabToolbar` and behaviors.
- Any consumer of `topbar.url_changed` or `topbar.panel_changed` events MUST migrate to reading state from the active tab's behavior or to subscribing to `shell.tab_activated` and `shell.tabs_list`.

#### Scenario: Topbar entry is gone

- **WHEN** the Vite build runs
- **THEN** `web/dist/` contains no `topbar/` directory and no `topbar/index.html`

#### Scenario: Topbar source is gone

- **WHEN** the repository is searched
- **THEN** `web/src/panels/topbar/` does not exist

---

### Requirement: Drag region pump from JS

**Reason**: With native toolbars, drag regions are statically known on the C++ side. The JS-driven `useDragRegions` hook (`ResizeObserver` + `MutationObserver` pumping `shell.set_drag_regions`) is no longer needed.

**Migration**:

- The `useDragRegions` hook is removed from `web/src/shared/hooks/`.
- The `shell.set_drag_regions` channel is removed from the channel registry (see `typed-bridge` change).
- The `app-drag` / `no-drag` CSS class convention is removed from sidebar and any other panels.
- C++ applies a single fixed draggable strip to the toolbar's `CefPanel` NSView and to the sidebar's top inset (~28 pt). Implemented in `mac_view_style.mm` (simplified `ApplyDraggableRegions`).

#### Scenario: Hook is gone

- **WHEN** the TypeScript build runs
- **THEN** `web/src/shared/hooks/useDragRegions.ts` does not exist

#### Scenario: Carve-out classes are gone

- **WHEN** the repository is searched
- **THEN** the class names `app-drag` and `no-drag` are not used in any TSX file
