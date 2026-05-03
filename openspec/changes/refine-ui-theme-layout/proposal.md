## Why

The shell chrome currently looks unfinished: the title bar and sidebar paint a hard-coded dark color (`#14141a`) that no longer matches the window's translucent background, the content area sits flush against the sidebar with no visual separation, settings opens as a modal overlay inside the agent tab, and the sidebar's `Terminal N / Chat N` rows do nothing when clicked because they only mutate a local renderer-side `panel` enum that the host ignores. There is also no support for the macOS Light appearance — the whole UI is locked to a dark palette.

We need to land an "appearance pass" that makes the chrome match the window background, gives the content area a proper card-style frame, surfaces settings as a lightweight popover, fixes the broken sidebar rows, and respects the system Light/Dark setting.

## What Changes

- Introduce a single source of truth for theme tokens (`web/src/styles/theme.css`) that defines a Light and a Dark palette, follows `prefers-color-scheme` by default, and lets the user override with an explicit Light / Dark / System choice persisted to disk.
- Plumb the active theme to the C++ shell so `MainWindow` can repaint the title bar, sidebar background, and content frame in step with the renderer (no more hard-coded `kTitleBarBg = 0xFF14141A`).
- Sidebar background, title bar background, and the macOS window background SHALL paint the exact same color (the "window chrome color"), so the chrome reads as one continuous surface (the blue-box area in the reference screenshot).
- The main content panel SHALL render as a rounded card: a 1 px border in the theme's border color, 12 px corner radius, and a small inset from the sidebar / title bar / window edges. Active tab cards mount inside the rounded frame.
- The Settings entry SHALL open as a CEF popover anchored under the title-bar gear button, hosting the existing LLM Settings UI. The inline `SettingsOverlay` inside the agent tab and the "open agent singleton" code path used by the title-bar gear button SHALL be removed.
- Sidebar `Terminal N` / `Chat N` / web tab rows SHALL be backed by real `TabId`s from the unified `shell.tabs_list` snapshot. Clicking a row SHALL send `shell.tab_switch { id }` so the host activates the corresponding tab card. The legacy `terminals` / `chats` arrays in the sidebar store and the `terminal.switch` activation path are removed.
- **BREAKING (renderer-only):** `web/src/panels/sidebar/store.ts` drops `terminals`, `activeTerminalId`, `chats`, `activeChatId`, `panel`, the `loadChats`/`saveChats` localStorage helpers, and every action that mutated them. The single tab list (with `kind`) drives every row.
- **BREAKING (renderer-only):** `web/src/panels/agent/App.tsx` no longer renders `SettingsOverlay`; the agent tab becomes pure agent UI. The `settingsOpen`/`openSettings`/`closeSettings` slice of the agent store is removed.

## Capabilities

### New Capabilities

- `app-theme`: design-token system with Light + Dark palettes, system-follow default, explicit user override persisted to `space.kv` (key `ui.theme = light|dark|system`), a `theme.get` / `theme.set` bridge channel pair, and a `theme.changed` push event so every panel and the C++ chrome repaint together.
- `chrome-layout`: visual layout of the shell — title bar background, sidebar background, and macOS window background paint the same theme-driven color; the content host is wrapped in a rounded (12 px) bordered frame inset by 8 px from the sidebar / title bar / window edges.
- `settings-popover`: Settings UI hosted in a CEF popover anchored beneath the title-bar gear button, replacing the inline `SettingsOverlay` inside the agent tab. Backed by a dedicated `panels/settings/` renderer entry and a new `shell.settings_popover_open` channel.

### Modified Capabilities

- `sidebar-tabs`: every row in the sidebar — web, terminal, chat — is bound to a real `TabId` from `shell.tabs_list` and activates via `shell.tab_switch { id }`. The store no longer carries a separate `terminals` / `chats` collection or a renderer-side `panel` enum.

## Impact

- **C++:** `app/browser/main_window.{h,cc}` (theme-aware title-bar / sidebar / content-frame colors, rounded-corner content frame, settings popover open path, gear button rewired); `app/browser/mac_view_style.{h,mm}` (window background follows theme); `app/browser/bridge_handler.{h,cc}` (`theme.get` / `theme.set` / `theme.changed`, `shell.settings_popover_open`); `app/workspace/space_store.*` (read/write `ui.theme` kv).
- **Web:** `web/src/styles/theme.css` (Light + Dark token sets via `:root` / `[data-theme="dark"]` plus `@media (prefers-color-scheme)`); `web/src/hooks/useTheme.ts` (new); `web/src/bridge_channels.ts` + `web/src/types/index.ts` (`theme.*`, `shell.settings_popover_open`); `web/src/panels/sidebar/{App.tsx,store.ts}` (unified tab rows, drop `terminals`/`chats` slices); `web/src/panels/agent/{App.tsx,store.ts}` (drop `SettingsOverlay`); new `web/src/panels/settings/` entry (App.tsx, main.tsx, public/panels/settings/index.html, vite config update).
- **Build:** Vite multi-page config gains the `panels/settings` entry.
- **No new runtime dependencies.**
