## 1. Theme tokens (renderer)

- [x] 1.1 In `web/src/styles/theme.css`, split the existing `@theme` block into a Light token set on `:root` and a Dark token set on `[data-theme="dark"]`. Cover at minimum: `--color-cronymax`, `-surface`, `-surface-2`, `-border`, `-fg`, `-fg-muted`, `-accent`, `-accent-soft`, `-success`, `-danger`, `-warning`.
- [x] 1.2 Add a `@media (prefers-color-scheme: dark) { :root:not([data-theme]) { ... } }` block that mirrors the Dark set so `data-theme` absent (initial paint) still respects the OS in `system` mode.
- [x] 1.3 Pick the Light palette: `bg #f6f6f8`, `surface #ffffff`, `surface-2 #efeff3`, `border #d8d8de`, `fg #18181c`, `fg-muted #5f6470`. Keep the existing Dark palette as is for the Dark set.
- [ ] 1.4 Build the web bundle (`cmake --build build --target cronymax_web -j 8`); verify both palettes render by manually toggling `data-theme` in DevTools.

## 2. Theme bridge channels

- [x] 2.1 Add to `web/src/types/index.ts`: `ThemeModeSchema = z.enum(["system","light","dark"])`, `ThemeGetResponseSchema = { mode, resolved }`, `ThemeSetPayloadSchema = { mode }`, `ThemeChangedPayloadSchema = { mode, resolved, chrome: { window_bg, border, fg, fg_muted } }`.
- [x] 2.2 Register `theme.get`, `theme.set` (req/res) and `theme.changed` (event) in `web/src/bridge_channels.ts`.
- [x] 2.3 Add `ThemeCallbacks { get_mode, set_mode }` to `app/browser/bridge_handler.h` (and mirror in `client_handler.h` if that is where shell callbacks are wired).
- [x] 2.4 Add the `theme.get` and `theme.set` dispatcher branches in `app/browser/bridge_handler.cc`. `theme.set` writes `space.kv["ui.theme"]`, then triggers a broadcast (via the new `MainWindow::OnThemeChanged`).
- [x] 2.5 Add a `BroadcastToAllPanels("theme.changed", payload)` helper invocation from `MainWindow::OnThemeChanged(ResolvedMode)`.

## 3. Theme persistence + initial sync

- [x] 3.1 In `app/workspace/space_store.*`, expose `GetKv(key)` / `SetKv(key, value)` if not already present (the schema already has the table per `space-manager` spec).
- [x] 3.2 In `MainWindow::OnWindowCreated`, before `BuildChrome`, read `ui.theme` from `space.kv` (default `"system"`) and resolve to a concrete `light|dark` mode using a new `ResolveSystemAppearance()` helper (macOS: `[NSApp.effectiveAppearance bestMatchFromAppearancesWithNames:@[Aqua, DarkAqua]]`).
- [x] 3.3 Compute the initial `ThemeChrome` from the resolved mode and call `ApplyThemeChrome(initial)` before `window->Show()`.
- [x] 3.4 Listen to `NSApp.effectiveAppearance` KVO (macOS) so when in `system` mode, OS appearance changes trigger `OnThemeChanged`.

## 4. useTheme hook

- [x] 4.1 Create `web/src/hooks/useTheme.ts` exporting `useTheme()` returning `{ mode, resolved, setMode }`. On mount: call `bridge.send("theme.get")`, set `data-theme` on `<html>`, subscribe to `theme.changed`. When `mode === "system"`, also subscribe to `window.matchMedia('(prefers-color-scheme: dark)')`.
- [x] 4.2 Replace the current eager dark assumption in any panel root component by mounting `useTheme()` once per panel root (sidebar, chat, terminal, agent, settings) so `data-theme` is always set before first paint.

## 5. ThemeChrome in MainWindow

- [x] 5.1 Add `struct ThemeChrome { cef_color_t window_bg; cef_color_t border; cef_color_t fg; cef_color_t fg_muted; };` to `app/browser/main_window.h` and a `ThemeChrome current_chrome_;` member plus `void ApplyThemeChrome(const ThemeChrome&);`.
- [x] 5.2 Refactor `BuildTitleBar` to call `panel->SetBackgroundColor(current_chrome_.window_bg)` instead of `kTitleBarBg`. Remove the `kTitleBarBg = 0xFF14141A` constant.
- [x] 5.3 Refactor the sidebar background path: drop the inline `style={{ backgroundColor: "#14141a" }}` in `web/src/panels/sidebar/App.tsx` and replace with `bg-cronymax`. Set the `BrowserView`'s `background_color` to a transparent color so the underlying panel paints through.
- [x] 5.4 In `app/browser/mac_view_style.mm`, change `StyleMainWindowTranslucent` to read the chrome `window_bg` from a thread-local-or-passed value rather than the hard-coded `#14141a`. Add `void SetMainWindowBackgroundColor(NSWindow*, cef_color_t)`.
- [x] 5.5 `ApplyThemeChrome` SHALL: call `titlebar_panel_->SetBackgroundColor`, `body_panel_->SetBackgroundColor`, the macOS window background helper, and update the content frame's border CALayer color (see Section 6).

## 6. Rounded content frame

- [x] 6.1 In `MainWindow::BuildChrome`, wrap `content_panel_` in a `content_frame_` `CefPanel` (`SetToFillLayout`), inset 8 px from the body's left/top/right/bottom via `body_layout->SetFlexForView(content_frame_, 1)` plus a non-zero `inside_border_insets` on `body_box`.
- [x] 6.2 Add `void InstallRoundedFrame(CefWindowHandle, double radius, cef_color_t border)` to `mac_view_style.{h,mm}`. Implementation sets `cornerRadius = 12`, `masksToBounds = YES`, attaches a 1 px-border `CALayer` colored with `border`.
- [x] 6.3 Call `InstallRoundedFrame` once after the content frame's NSView is realized, and re-set the border color from `ApplyThemeChrome` on subsequent theme changes.
- [ ] 6.4 Verify a web tab card's BrowserView is clipped to the rounded frame on macOS (no square corner overlap).

## 7. Settings popover

- [x] 7.1 Add `web/src/panels/settings/{App.tsx,main.tsx}` and `web/public/panels/settings/index.html`. Move the existing `SettingsOverlay` body from `web/src/panels/agent/App.tsx` into `App.tsx` here; keep its store interactions identical (LLM provider/key/model save/cancel).
- [x] 7.2 Add a `Theme` section to the Settings App: three radio choices (`System`, `Light`, `Dark`) wired to `useTheme().setMode`.
- [x] 7.3 Add the `panels/settings` entry to `web/vite.config.ts` (mirrors the existing per-panel entries).
- [x] 7.4 Register `shell.settings_popover_open` (req/res) in `web/src/bridge_channels.ts` and `web/src/types/index.ts`.
- [x] 7.5 Add the `shell.settings_popover_open` dispatcher branch in `app/browser/bridge_handler.cc`; it calls `MainWindow::OpenPopover(ResourceUrl("panels/settings/index.html"))` and is idempotent (no-op if already open).
- [x] 7.6 Rewire the title-bar gear button in `MainWindow::BuildTitleBar` to call the same `OpenPopover` path instead of activating the agent singleton tab.
- [x] 7.7 Remove `SettingsOverlay`, the `settingsOpen` slice, and the `openSettings`/`closeSettings` actions from `web/src/panels/agent/App.tsx` and `store.ts`.

## 8. Sidebar unification

- [x] 8.1 In `web/src/panels/sidebar/store.ts`, replace the `tabs` field with the unified `TabSummary[]` shape (`{ kind, id, displayName }`); remove `terminals`, `activeTerminalId`, `chats`, `activeChatId`, `panel`. Add `activeTabId: string | null`.
- [x] 8.2 Replace per-row reducers (`addTab`, `closeTab`, `addTerminal`, …) with a single `setSnapshot(tabs, activeTabId)` reducer driven by `shell.tabs_list`.
- [x] 8.3 In `web/src/panels/sidebar/App.tsx`, drop `useBridgeEvent("terminal.*")` subscriptions, drop `loadChats`/`saveChats`. Subscribe to `shell.tabs_list` and `shell.tab_activated`. Build rows from the snapshot, keyed by `kind`.
- [x] 8.4 Row click → `bridge.send("shell.tab_switch", { id })`. Row close → `bridge.send("shell.tab_close_str", { id })` (or whichever channel the C++ already exposes for string ids). No-op if the row is already active.
- [x] 8.5 Picture/glyph picker by `kind`: web → favicon + globe fallback, terminal → `⌨`, chat → `💬`, agent → `⚙`, graph → `▦`.

## 9. Validation + smoke

- [x] 9.1 `openspec validate refine-ui-theme-layout` must pass.
- [x] 9.2 Build green: `cmake --build build --target cronymax -j 8` and `cmake --build build --target cronymax_web -j 8`.
- [ ] 9.3 Manual smoke (Dark): launch with system in Dark — chrome and content frame paint Dark; verify the rounded 12 px frame and 8 px inset; click Settings gear → popover opens with theme controls; click Terminal 1 / Chat 1 / Chat 2 rows → each activates the matching tab card.
- [ ] 9.4 Manual smoke (Light): switch macOS to Light — entire chrome and content repaints to Light; pin to `Dark` in Settings → stays Dark when OS toggles back; relaunch — persisted choice restored.
- [x] 9.5 Update `docs/architecture.md` chrome diagram to include the rounded content frame and the Settings popover replacing the agent overlay.
