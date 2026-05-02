## 1. Asset Preparation

- [x] 1.1 Add `@vscode/codicons` to `package.json` devDependencies and run `pnpm install`
- [x] 1.2 Create `assets/icons/` directory and vendor the required SVG files from `@vscode/codicons/dist/icons/`: `arrow-left.svg`, `arrow-right.svg`, `refresh.svg`, `close.svg`, `add.svg`, `settings-gear.svg`, `terminal.svg`, `comment-discussion.svg`, `type-hierarchy.svg`, `globe.svg`, `debug-stop.svg`
- [x] 1.3 Create `assets/icons/README.md` documenting the Codicons version pinned, the icon-to-`IconId` mapping, and the update procedure

## 2. Native Icon Registry

- [x] 2.1 Create `app/browser/icon_registry.h`: define `enum class IconId` with all values (`kBack`, `kForward`, `kRefresh`, `kStop`, `kNewTab`, `kClose`, `kSettings`, `kTabTerminal`, `kTabChat`, `kTabAgent`, `kTabGraph`, `kTabWeb`, `kRestart`, `kCount`); declare `IconRegistry::Init()`, `IconRegistry::GetImage(IconId, int logical_size)`; declare `MakeIconButton` and `MakeIconLabelButton` factory functions
- [x] 2.2 Create `app/browser/icon_registry.mm`: implement `IconRegistry::Init()` using `NSImage` + `CoreGraphics` to rasterise each SVG from the app bundle's `Resources/icons/` at 16px and 20px at the main display's device pixel ratio; store results in `CefImage` via `AddBitmap()`
- [x] 2.3 Implement `IconRegistry::GetImage(IconId id, int logical_size = 16)`: return the stored `CefImage` for the requested size; fall back to 16px and log a warning for unsupported sizes; abort with a fatal log if `id >= kCount`
- [x] 2.4 Implement `MakeIconButton(CefRefPtr<CefButtonDelegate>, IconId, std::string_view accessible_name)`: create a `CefLabelButton` with empty text, call `SetImage` for normal/hovered/disabled states, call `SetAccessibleName`, call `SetTooltipText`
- [x] 2.5 Implement `MakeIconLabelButton(CefRefPtr<CefButtonDelegate>, IconId, std::string_view label, std::string_view accessible_name)`: same as `MakeIconButton` but with non-empty text label
- [x] 2.6 Add `icon_registry.mm` to `CMakeLists.txt` (app source list) and copy `assets/icons/*.svg` into the app bundle's `Resources/icons/` directory via the CMake install step
- [x] 2.7 Call `IconRegistry::Init()` inside `DesktopApp::OnContextInitialized()` before any `CefWindow::CreateTopLevelWindow()` call; verify no icons are missing at startup

## 3. Native Title Bar Buttons

- [x] 3.1 Replace the `⊕ Web` `CefLabelButton` in `main_window.cc` with `MakeIconLabelButton(delegate, IconId::kTabWeb, "Web", "New Web Tab")`
- [x] 3.2 Replace the `⌨ Terminal` button with `MakeIconLabelButton(delegate, IconId::kTabTerminal, "Terminal", "New Terminal Tab")`
- [x] 3.3 Replace the `💬 Chat` button with `MakeIconLabelButton(delegate, IconId::kTabChat, "Chat", "New Chat Tab")`
- [x] 3.4 Replace the `⚙ Settings` button with `MakeIconLabelButton(delegate, IconId::kSettings, "Settings", "Settings")`

## 4. Native Tab Toolbar Buttons — Web Tab

- [x] 4.1 In `web_tab_behavior.cc`, replace the `◀` back button with `MakeIconButton(delegate, IconId::kBack, "Back")`
- [x] 4.2 Replace the `▶` forward button with `MakeIconButton(delegate, IconId::kForward, "Forward")`
- [x] 4.3 Replace the `↻` refresh button with `MakeIconButton(delegate, IconId::kRefresh, "Refresh")`
- [x] 4.4 Replace the `⊕` new-tab button with `MakeIconButton(delegate, IconId::kNewTab, "New Tab")`
- [x] 4.5 Update `UpdateRefreshStopGlyph()` (or rename to `UpdateRefreshStopIcon()`) to call `refresh_btn_->SetImage(CEF_BUTTON_STATE_NORMAL, IconRegistry::GetImage(IconId::kStop))` while loading and restore `IconId::kRefresh` when done, instead of setting text

## 5. Native Tab Toolbar Buttons — Simple Tab (Terminal, Chat, Agent, Graph)

- [x] 5.1 In `simple_tab_behavior.cc`, replace the leading `icon_ + " " + display_name_` single `CefLabelButton` with `MakeIconLabelButton(delegate, icon_id_for_kind(kind_), display_name_, display_name_)` where `icon_id_for_kind()` maps tab kind to the appropriate `IconId`
- [x] 5.2 Add a private helper `IconId icon_id_for_kind(TabKind kind)` in `simple_tab_behavior.cc` covering `terminal → kTabTerminal`, `chat → kTabChat`, `agent → kTabAgent`, `graph → kTabGraph`

## 6. React Icon Infrastructure

- [x] 6.1 Add the `@vscode/codicons` npm package as a regular dependency (not devDependencies) so the sprite is available at runtime; run `pnpm install`
- [x] 6.2 Create `web/src/shared/icons.ts`: define and export `type IconName` as a string union of all Codicon names used by the app (`"arrow-left"`, `"arrow-right"`, `"refresh"`, `"close"`, `"add"`, `"settings-gear"`, `"terminal"`, `"comment-discussion"`, `"type-hierarchy"`, `"globe"`, `"debug-stop"`, `"sparkle"`, `"tools"`)
- [x] 6.3 Create `web/src/shared/components/Icon.tsx`: implement `<Icon name={IconName} size={number} aria-label={string} className={string}>` rendering an `<svg>` with a `<use href="#codicon-<name>">` reference; default size 16; forward all standard SVG props
- [x] 6.4 Create `web/src/shared/components/IconSprite.tsx`: render a visually-hidden `<div aria-hidden="true">` injecting the full Codicons SVG sprite (`@vscode/codicons/dist/codicon.svg` content)
- [x] 6.5 Add `<IconSprite>` to the root component of each panel: `sidebar/App.tsx`, `popover/App.tsx`, `settings/App.tsx`, `terminal/App.tsx`, `FlowEditor/index.tsx`

## 7. React Panel Icon Replacements

- [x] 7.1 In `sidebar/App.tsx`, replace `glyphFor(kind)` with `<Icon name={iconNameForKind(kind)}>` and implement `iconNameForKind` mapping; replace globe fallback in `faviconFor()` with `<Icon name="globe">`
- [x] 7.2 In `popover/App.tsx`, replace the `↻` reload button text with `<Icon name="refresh" aria-label="Reload">`, the `↗` open-as-tab button with `<Icon name="link-external" aria-label="Open as Tab">`, and the `✕` close button with `<Icon name="close" aria-label="Close">`
- [x] 7.3 In `settings/App.tsx`, replace the `✕` / `×` close button text with `<Icon name="close" aria-label="Close">`
- [x] 7.4 In `terminal/App.tsx`, replace the `✨ Explain` button with `<Icon name="sparkle" /> Explain`, the `🔧 Fix` button with `<Icon name="tools" /> Fix`, and the `↻ Retry` button with `<Icon name="refresh" /> Retry`
- [x] 7.5 In `FlowEditor/index.tsx`, replace the `💾 Save` toolbar button with `<Icon name="save" /> Save`, the `🗑 Delete` button with `<Icon name="trash" /> Delete`, and all `×` dialog close buttons with `<Icon name="close" aria-label="Close">`

## 8. Verification

- [x] 8.1 Build the app (`cmake --build build`) and confirm no compilation errors in `icon_registry.mm` or any modified `tab_behaviors/` file
- [ ] 8.2 Launch the app and visually confirm all title bar buttons show icons (not emoji/Unicode) with correct tooltips on hover
- [ ] 8.3 Open a web tab and confirm back/forward/refresh/stop/new-tab toolbar buttons show icons with no visible text
- [ ] 8.4 Open terminal, chat, agent, and graph tabs and confirm each leading slot shows the correct icon + text label
- [ ] 8.5 Open the sidebar and confirm each tab row shows the correct semantic icon (not emoji glyphs); confirm web rows show favicons with globe fallback
- [x] 8.6 Run TypeScript compilation (`pnpm --filter web tsc --noEmit`) and confirm no `IconName` type errors
- [ ] 8.7 Confirm accessibility: use the macOS Accessibility Inspector to verify accessible names on all icon-only native buttons
