## Context

The desktop shell currently hard-codes a single dark palette across the C++ chrome (`MainWindow::BuildTitleBar` paints `#14141a`), the macOS window background helper (`mac_view_style.mm` sets the same color), and the renderer tokens in `web/src/styles/theme.css`. Each surface picks the color independently, which is fragile and locks the app to dark mode.

Three concrete UX gaps motivate this change:

- The reference screenshot shows the title bar and sidebar painted as a single continuous chrome that frames a recessed content card. Today the title bar/sidebar already share `#14141a`, but the content area is flush — no border, no rounding — so the chrome reads as flat instead of card-like.
- Settings opens as an in-tab `SettingsOverlay` modal inside the agent panel (`web/src/panels/agent/App.tsx`) whose visibility flag also lives in the agent store. The title-bar gear button currently activates the agent singleton tab to surface that overlay, mixing two concerns into one tab.
- Sidebar rows for `Terminal N` and `Chat N` are populated from a separate `terminals` array (driven by `terminal.list`) and a localStorage-only `chats` array. Their click handlers `dispatch({ type: "setPanel" })` and call `terminal.switch`, neither of which tells the host to activate the corresponding tab card. Meanwhile every actual tab — including each terminal/chat tab opened from the title bar — already exists in the unified `shell.tabs_list` snapshot with a `kind` field. The sidebar is showing two parallel realities and the wrong one is wired to clicks.

## Goals / Non-Goals

**Goals:**

- Single source of truth for theme tokens: a `web/src/styles/theme.css` `:root` block (Light) and a `[data-theme="dark"]` (or `@media (prefers-color-scheme: dark)`) block (Dark) consumed by every renderer panel.
- C++ chrome (title bar, sidebar background fill, content frame border, NSWindow background) reads its colors from a small `ThemeChrome` struct that is updated whenever the renderer's resolved theme changes.
- The shell respects `prefers-color-scheme` by default; the user can pin Light or Dark or System via the Settings popover; the choice persists to the active Space's `space.kv` table under key `ui.theme`.
- The content panel renders as a 12 px-rounded, 1 px bordered card inset 8 px from sidebar / title bar / window edges, on every theme.
- The Settings UI moves into a CEF popover (the same docked-popover infra `OpenPopover` already uses) anchored under the title-bar gear button.
- Every sidebar row activates a real native tab card via `shell.tab_switch { id }`.

**Non-Goals:**

- No new accent colors / no rebrand; we keep the existing accent (`#7c5cff`) and just add a Light counterpart.
- No animated theme transitions — switching the theme repaints everything in one frame, no fade.
- No per-Space theme override; theme is a global user preference (stored on whichever Space is "default" but read globally).
- No Windows port work for the chrome color path (the existing macOS-only `mac_view_style.mm` is the only platform binding touched).
- No redesign of individual panel content (chat bubbles, terminal output, agent trace) beyond what the new tokens automatically pull through.
- The `panel: "browser" | "terminal" | "agent" | "graph" | "chat" | "config"` enum in the sidebar store is deleted, not refactored — there is exactly one source of truth for "what is mounted" (the unified tab list with `kind`).

## Decisions

### Decision 1 — Theme model: tokens in CSS, resolved value mirrored to C++

CSS custom properties on the document root are the source of truth at runtime. We keep the existing Tailwind v4 `@theme` block but split colors into two sets: a `:root` block holding the Light palette and a `[data-theme="dark"]` block (plus a `@media (prefers-color-scheme: dark)` fallback for `data-theme="system"`) overriding them with the Dark palette. A small `useTheme` hook resolves the user preference (`system | light | dark`) into a concrete mode (`light | dark`), sets `data-theme` on `<html>`, and pushes the resolved mode + the four chrome colors (`bg`, `surface`, `border`, `fg`) over a new `theme.changed` bridge event so the C++ side can repaint.

**Alternatives considered:**

- _Single C++-controlled theme that pushes CSS strings to the renderer._ Rejected: keeps the C++/web coupling but forces all token math (e.g. `surface-2`) into C++. CSS-first inverts the cost: the renderer is where the math already lives.
- _No system-follow; only explicit Light/Dark._ Rejected: the explicit ask is "follow the system theme."

### Decision 2 — Theme persistence: `space.kv` with key `ui.theme`

The active Space's `SpaceStore` already exposes a key/value scratch table; we reuse it under `ui.theme = "system" | "light" | "dark"`. On startup `MainWindow` reads it, pushes the resolved value to the sidebar before first paint, and broadcasts on every change. This avoids a new schema migration and keeps the value travelling with the user's profile.

**Alternative:** a top-level `~/.cronymax/preferences.json`. Rejected for v1 — the Space DB is already the persistence boundary and we have no other top-level prefs file yet. Migration to a per-user file can come later if multi-Space inconsistency becomes a complaint.

### Decision 3 — Chrome color comes from a single `ThemeChrome` struct

`MainWindow` holds a `ThemeChrome { cef_color_t window_bg; cef_color_t border; cef_color_t fg; cef_color_t fg_muted; }` member updated by a new `ApplyThemeChrome(const ThemeChrome&)` method. `BuildTitleBar` and `BuildChrome` ask for the current value instead of using `kTitleBarBg` / `kSidebarBg` constants. The `theme.changed` bridge event re-runs `ApplyThemeChrome` and re-issues `SetBackgroundColor` on the title-bar panel, the content-frame panel, and the macOS window helper.

**Alternative:** Re-create the chrome on theme change. Rejected — would tear down browser views and reset state.

### Decision 4 — Rounded content frame is a CEF panel, not CSS

The content card needs to clip per-tab `BrowserView`s, which are platform NSViews on macOS. CSS `border-radius` on a parent does not clip a child NSView. We instead wrap `content_panel_` in a `content_frame_` `CefPanel` with insets `(8, 8, 8, 8)` and use the `mac_view_style.mm` helper to set `cornerRadius = 12` + `masksToBounds = YES` on the frame's underlying NSView. The same helper paints the 1 px border via a `CALayer` with `borderWidth = 1` and `borderColor` from the current theme.

**Alternative:** Pure CSS rounding by giving each tab card an HTML wrapper. Rejected — web tab cards host a real `BrowserView`, not a `<div>`.

### Decision 5 — Settings is a CEF popover, not an HTML popover

`MainWindow` already implements a docked CEF popover (`OpenPopover` / `popover_view_`) with a chrome strip and centred layout. We reuse it for Settings: a new `panels/settings/` Vite entry hosts the existing `SettingsOverlay` content (lifted out of the agent tab). The title-bar gear button now calls `OpenPopover(ResourceUrl("panels/settings/index.html"))` instead of activating the agent singleton tab. `LayoutPopover` already centres under the title bar.

**Alternative:** A floating HTML popover inside the sidebar's BrowserView. Rejected — the popover needs to overlay the content area, which sits in a different BrowserView.

### Decision 6 — Sidebar rows unified onto `shell.tabs_list`

The sidebar drops the parallel `terminals` / `chats` collections and the `panel` enum. The single `tabs` array is the new `TabSummary[]` from the unified `shell.tabs_list` snapshot (which already includes `kind`). Row icons are picked from `kind`. Click sends `shell.tab_switch { id }`; close sends `shell.tab_close_str { id }`. The standalone `terminal.list`/`terminal.created`/`terminal.removed`/`terminal.switched` events are no longer consumed by the sidebar (the terminal panel still uses them for its own input routing).

This also fixes the symptom in the screenshot: every visible row is a real tab id, so clicking it activates the right card.

**Alternative:** Keep the parallel collections and add a new "navigate the host" action that calls `shell.tab_switch` for the correct id. Rejected — leaves two sources of truth and means we still have to map `chat-id` → `tab-id` somewhere.

## Risks / Trade-offs

- [The C++ chrome can paint stale colors briefly on startup if the renderer hasn't yet pushed its first `theme.changed`.] → On startup `MainWindow` reads `ui.theme` from `space.kv` synchronously and applies the corresponding default `ThemeChrome` before showing the window. Renderer's first push merely confirms the same value.
- [`prefers-color-scheme` changes can race with explicit user choice.] → The renderer treats `system` as "subscribe to the media query and re-resolve"; explicit `light` / `dark` takes precedence and ignores the query.
- [Lifting `SettingsOverlay` out of the agent tab can break in-flight LLM Settings save/cancel state.] → The lift moves the same component to a new entry point with the same store; the diff is mechanical (props-in, dispatch-out), and the agent tab keeps its own store untouched apart from removing the `settingsOpen` slice.
- [Rounded NSView clipping interacts badly with `BrowserView` resizing during animations.] → We do not animate the frame; corner radius is set once at creation. Resize triggers a normal re-layout, not a CALayer mask change.
- [Sidebar's parallel `chats` localStorage history is dropped.] → Acceptable: those rows were dead-end UI (clicks did nothing). No persisted chat _content_ lives in that array; the underlying chat tab persistence is unaffected.

## Migration Plan

1. Land C++ + renderer side-by-side in one change. The `theme.*` channels and the unified sidebar store are mutually consistent; mixing old + new is not supported because of the broken click behavior we are fixing.
2. On first launch after upgrade, `space.kv` has no `ui.theme` row — the renderer falls back to `system` and writes it back, so subsequent launches start from a persisted value.
3. Rollback is a single git revert; no schema change to undo.

## Open Questions

- Should the gear button live in the title bar (current spot) or in the sidebar's space header? _Decision deferred — keep the title-bar position introduced by `native-title-bar` for v1._
- Do we want a per-Space theme override later? _Out of scope; the `ui.theme` row is global enough for now since most users run a single Space._
