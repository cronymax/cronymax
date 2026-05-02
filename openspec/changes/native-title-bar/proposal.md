## Why

The shell ships with no title bar of its own. Today the macOS traffic lights float over an empty translucent strip and the only `+Tab` / `+Terminal` / `+Chat` affordances live at the bottom of the sidebar — far from where users expect "open a new thing" to be (top of the window, Arc/Chrome muscle memory). At the same time, Phase 14.5 of `arc-style-tab-cards` left terminal/chat/agent/graph as singletons, which conflicts with the diagram the title bar implies (`Tab2: terminal / Tab3: chat` — multiple of each).

This change introduces a real, native CEF-Views title bar with the new-tab actions on it, and pulls the multi-instance flip forward so the buttons mean what they appear to mean.

## What Changes

- Add a native title bar (`CefPanel` with `CefBoxLayout`, height ≈ 38 px) above the existing `[sidebar | content]` body. Root window layout flips from horizontal to vertical.
- Title bar contents, left → right:
  - 78 px reservation for the macOS traffic lights (no widget, just spacing).
  - Flex spacer (drag region in v1).
  - Three `CefLabelButton` actions with tooltips: `+ Web`, `+ Terminal`, `+ Chat` (icon + glyph).
  - 0 px window-controls slot on macOS, reserved for the eventual Windows port.
- Make terminal and chat tab kinds **multi-instance**: each click of `+ Terminal` or `+ Chat` creates a fresh tab (`Terminal 1`, `Terminal 2`, …). Agent and graph stay singletons for now.
- Remove the bottom `+ Tab / + Terminal / + Chat` row from the HTML sidebar; the title bar is the canonical action surface.
- Add a macOS-only AppKit drag overlay sitting above the title bar's flex spacer so window dragging works from that region (drag from button hit-rects is suppressed). No drag from the lights area or the buttons themselves in v1.
- Update `LayoutPopover` to centre under the new title-bar height (not the deleted topbar height).
- **BREAKING (renderer-only):** Sidebar React panel no longer ships its own bottom action row; the corresponding store actions are removed.

## Capabilities

### New Capabilities

- `native-title-bar`: the title-bar panel itself — its layout, the new-tab action buttons, the traffic-light reservation, the macOS drag overlay, the Windows-controls stub, and the `shell.tab_new_kind { kind }` channel that backs the buttons.

### Modified Capabilities

- `tab-system`: terminal and chat kinds become multi-instance — `RegisterSingletonKind(kTerminal/kChat)` is replaced by a "new instance per request" path; display names are auto-numbered (`Terminal N`, `Chat N`); the `shell.tab_open_singleton` contract continues to apply only to kinds that remain singletons.
- `sidebar-tabs`: the bottom action row (`+ Tab / + Terminal / + Chat`) is removed; the sidebar React store drops the corresponding `newTab` / `newTerminal` / `newChat` actions and the `shell.tab_open_singleton` calls for terminal/chat.

## Impact

- **C++:** `MainWindow` (root layout flip, new `titlebar_panel_` + `BuildTitleBar`); `mac_view_style.{h,mm}` (AppKit `mouseDownCanMoveWindow` overlay helper); `bridge_handler.{h,cc}` + `ShellCallbacks` (new `shell.tab_new_kind`); `tab_manager.{h,cc}` (un-singleton terminal/chat, auto-number display names); `LayoutPopover` constants.
- **Web:** `web/src/panels/sidebar/App.tsx` + `store.ts` (drop bottom action row); `web/src/shared/bridge_channels.ts` + `types/index.ts` (add `shell.tab_new_kind` schema).
- **No new dependencies.**
