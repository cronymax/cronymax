## 1. Bridge channel skeleton

- [x] 1.1 Add `ShellNewTabKindPayloadSchema` and `ShellNewTabKindResponseSchema` (`{kind:'web'|'terminal'|'chat'}` → `{tabId:string, kind:string}`) to `web/src/shared/types/index.ts`.
- [x] 1.2 Register `shell.tab_new_kind` channel in `web/src/shared/bridge_channels.ts` (req/res).
- [x] 1.3 Add `ShellCallbacks::new_tab_kind` (`std::function<std::string(const std::string&)>`) to `src/app/bridge_handler.h`.
- [x] 1.4 Add the `shell.tab_new_kind` dispatcher branch in `src/app/bridge_handler.cc` (returns the JSON the callback produces, or `{}` if missing).
- [x] 1.5 Build green (`cmake --build build --target cronymax_app -j 8` + `cmake --build build --target cronymax_web -j 8`).

## 2. Multi-instance flip (tab-system)

- [x] 2.1 In `src/app/main_window.cc`, stop calling `tabs_->RegisterSingletonKind(TabKind::kTerminal)` and `…(kChat)` in `OnWindowCreated`. Keep the `SetKindContentUrl` calls.
- [x] 2.2 In `src/app/tab_manager.{h,cc}` add an internal "auto-numbered kinds" set (`{kTerminal, kChat}`) and an auto-numbering pass inside `Open(kind, params)`: when `params.display_name` is empty AND kind is in the set, assign `<KindDisplayName> N` where N = max-existing-suffix + 1.
- [x] 2.3 Make `shell.tab_open_singleton` dispatcher return a failure when called with a kind not registered as a singleton (so leftover renderer code logs loudly).
- [x] 2.4 Wire `sh.new_tab_kind` in `MainWindow::OnWindowCreated`: handles `web` (current `OpenWebTab` path), `terminal`/`chat` (`tabs_->Open(kind, {})` then activate), broadcasts `shell.tab_created` with the existing numeric-id JSON shape.
- [x] 2.5 Build green.

## 3. Native title bar (CefPanel)

- [x] 3.1 Add `BuildTitleBar()` in `MainWindow` that returns a horizontal `CefPanel` with the slot order `lights_pad_ | spacer_ | btn_web_ | btn_term_ | btn_chat_ | win_pad_`. Use `SizedPanelDelegate` for `lights_pad_(78,26)` and `win_pad_(0,1)` on macOS.
- [x] 3.2 Add `MainWindow::titlebar_panel_`, `spacer_`, `btn_web_`, `btn_term_`, `btn_chat_` (and `lights_pad_`, `win_pad_`) members in `main_window.h`.
- [x] 3.3 Wire each button's click via the existing `FnButtonDelegate` pattern (defer-to-UI-tick if needed) so it resolves to a `bridge`-equivalent C++ call: `MainWindow::OpenNewTabKind("web"|"terminal"|"chat")`. Set tooltips via `SetTooltipText`.
- [x] 3.4 Set the title-bar background to the existing chrome dark (`#0E0E10`) and set `kTitleBarH = 38` as a layout constant.
- [x] 3.5 Flip the root layout in `BuildChrome`: `window->SetToBoxLayout({.horizontal=false})`, add `titlebar_panel_` (flex=0), then a `body_panel_` (`SetToBoxLayout({.horizontal=true})`, flex=1) that re-parents the existing `[sidebar_view_ | content_outer]` children.
- [x] 3.6 Build green; confirm the title bar appears, the body sits below it, and tabs still mount/activate correctly.

## 4. macOS drag overlay

- [x] 4.1 In `src/app/mac_view_style.h` declare `void InstallTitleBarDragOverlay(CefWindowHandle, CefRect spacer_rect_window_coords)`.
- [x] 4.2 In `src/app/mac_view_style.mm` implement a private `CronymaxDragView : NSView` with `mouseDownCanMoveWindow = YES`, lazily attached to the NSWindow's `contentView`. The function (re)frames the singleton drag view to `spacer_rect_window_coords` (converting from CEF's top-down to NSView's bottom-up).
- [x] 4.3 In `MainWindow`, add `RefreshTitleBarDragRegion()`: query `spacer_->GetBounds()`, convert to window coords, call `InstallTitleBarDragOverlay`. Call it once after the initial `BuildChrome` (post-`Layout()`) and again from `OnWindowBoundsChanged`.
- [x] 4.4 Build green; verify drag-from-spacer moves the window and clicks on the buttons still work.

## 5. Popover constants

- [x] 5.1 In `MainWindow::LayoutPopover` change `kTopbarH=44` to `kTitleBarH=38` (and rename for clarity).
- [x] 5.2 Verify the popover still centres correctly under the title bar.

## 6. Sidebar bottom-row removal

- [x] 6.1 In `web/src/panels/sidebar/App.tsx` remove the `<div>` (or footer) hosting the `+ Tab / + Terminal / + Chat` row, and the `newTab` / `newTerminal` / `newChat` `useCallback` definitions.
- [x] 6.2 In `web/src/panels/sidebar/store.ts` (and any related selector module) remove store actions that exclusively backed those callbacks (if any). Leave the dock-item activate path untouched.
- [x] 6.3 Build the web bundle (`cmake --build build --target cronymax_web -j 8`); fix TypeScript errors from now-unused imports.

## 7. Validate + smoke

- [x] 7.1 Run `openspec validate native-title-bar` → must pass.
- [ ] 7.2 Manual smoke: launch the app, confirm the title bar appears with three buttons + tooltips; click each button repeatedly to verify multi-instance terminals and chats; drag from the spacer; resize the window and re-drag; open a popover and verify centring; verify sidebar no longer shows the bottom action row.
- [x] 7.3 Update `docs/architecture.md` with the new root-layout shape (`window VBOX → titlebar | body HBOX → sidebar | content`).
