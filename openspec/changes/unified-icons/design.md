## Context

The Cronymax desktop app has three disjoint icon systems: (1) Unicode/emoji string literals in native `CefLabelButton` labels (`main_window.cc`, `web_tab_behavior.cc`, `simple_tab_behavior.cc`), (2) emoji/Unicode constants inline in React panel components (`sidebar/App.tsx`, `popover/App.tsx`, `settings/App.tsx`, `terminal/App.tsx`, `FlowEditor/index.tsx`), and (3) dynamically-fetched Google Favicon API thumbnails for web tabs. There is no shared vocabulary, no recolouring capability, no theme awareness, and emoji glyphs have inconsistent rendering on macOS. The app's native layer already uses `CefLabelButton` throughout and `CefLabelButton::SetImage()` already exists but has never been exercised.

**Constraints:**
- Must not add a native windowing dependency (no Qt, GTK, or similar) for SVG rendering
- Must not require a network request for any icon other than site favicons
- Must remain macOS-only for native code during this change (cross-platform native icons are out of scope)
- Must not change the CEF Views control hierarchy (TabToolbar layout, title bar panel structure)

## Goals / Non-Goals

**Goals:**
- Single semantic icon vocabulary (`IconId` C++ enum / kebab-case string in React) shared across both layers
- Native toolbar buttons display crisp vector-sourced icons at every DPI via `CefLabelButton::SetImage()`
- React panel components use a typed `<Icon name>` component instead of inline glyphs
- VS Code Codicon set as the icon source — consistent visual language and MIT licensed
- Accessibility: every icon-only native button has `SetAccessibleName()` set; every React `<Icon>` carries an `aria-label`
- The three existing specs that mandate glyph literals (`sidebar-tabs`, `tab-flavor-web`, `tab-toolbar`) are updated so future tasks implement the right thing

**Non-Goals:**
- Theme-aware colour variants for native icons (e.g., accent-tinted `kNewTab` button) — deferred to a future tokens change
- Cross-platform native icon support (Linux/Windows) — macOS only for now
- Replacing site favicons with any kind of icon registry entry — favicons are identity, not action icons
- Animated or state-transition icons
- Changing the `TabToolbar` three-slot layout, tab card sizing, or any visual metric other than the glyph-to-image swap

## Decisions

### D1: Use existing `CefLabelButton::SetImage()` — no new control type

`CefLabelButton` already has `SetImage(cef_button_state_t, CefRefPtr<CefImage>)` for each button state (normal, hovered, pressed, disabled, focused). Setting text to an empty string and setting an image gives a pure icon button. Setting text to a non-empty string gives icon + label. No wrapper class is needed; the existing `CefLabelButton` pointer type is used everywhere. Two small factory helpers (`MakeIconButton`, `MakeIconLabelButton`) in `icon_registry.h` are sufficient.

**Alternative considered:** A new `IconButton` C++ class inheriting `CefLabelButton`. Rejected because `CefLabelButton` is a CEF-managed ref-counted type; subclassing it does not work cleanly with `CreateLabelButton` factory semantics and CEF's internal lifecycle.

### D2: SVG → PNG rasterisation via macOS Core Graphics at startup

On macOS, `NSImage` natively loads SVG data. At startup `IconRegistry::Init()` iterates over all `IconId` values, reads the corresponding `.svg` file from the app bundle's `Resources/icons/` directory, constructs an `NSImage`, renders it at the main screen's device pixel ratio into a bitmap context, and wraps the result in a `CefImage` via `CefImage::AddBitmap()`. Only two sizes per icon are rasterised: 16×16 and 20×20 logical pixels (the two sizes used in toolbars and sidebar rows).

**Alternative considered:** Pre-rasterised PNG files committed to the repository (build-time script). Rejected because it adds binary assets to the repo and diverges from the SVG source of truth whenever icons are updated.

**Alternative considered:** Off-screen `CefBrowserView` HTML canvas render. Rejected as too heavy — a dedicated invisible browser view just for icon rasterisation is architecturally unclean.

### D3: VS Code Codicons as icon vocabulary

Codicons are MIT licensed, ship as individual SVG files in the `@vscode/codicons` npm package, cover all required action icons (arrow-left, arrow-right, refresh, close, add, settings-gear, terminal, comment-discussion, circuit-board, type-hierarchy), and have a consistent 16×16 design grid that matches the existing toolbar height of 40 px. The SVGs are vendored into `assets/icons/` at a pinned Codicons version so the app is self-contained at runtime.

**Alternative considered:** Google Material Symbols, Heroicons, Lucide. All would work but Codicons are already the VS Code visual idiom and require no design decisions about style (outlined vs filled, weight, grade).

### D4: Semantic `IconId` enum as shared vocabulary

The C++ layer defines `enum class IconId` in `app/browser/icon_registry.h`. The React layer uses a `type IconName = "back" | "forward" | ...` string union in `web/src/shared/icons.ts`, mirroring the same identifiers. A static mapping table in `IconRegistry` maps each `IconId` to a filename and each name in React maps to the same filename. The two layers stay in sync by convention; no codegen is required because the set of action icons is small and stable.

### D5: React `<Icon>` is a thin SVG inline component

`web/src/shared/components/Icon.tsx` renders a `<svg>` element sourced from the vendored Codicons SVG sprite (`@vscode/codicons/dist/codicon.svg`). It uses a `<use href="#<name>">` reference into the sprite, which is injected once as a hidden `<div>` in each panel's root via a `<IconSprite>` component. Size defaults to 16×16 and inherits `currentColor` so it responds to CSS colour tokens automatically.

**Alternative considered:** `<img src="path/to/icon.svg">` per icon. Rejected because each `<img>` causes a separate resource load and the icons cannot be recoloured via `color` CSS property.

### D6: Sequential replacement, no coexistence period

The glyph-literal approach is removed entirely in the implementation tasks for this change. No "new system on, old system still works" compatibility layer is needed because the affected files are completely within the Cronymax repo (no public API). The `IconRegistry` must be initialised before `MainWindow::CreateControls()` is called; the call order in `desktop_app.cc` enforces this.

## Risks / Trade-offs

- **[Risk] macOS SVG rasterisation startup cost** — Rasterising ~15 icons at startup via `NSImage` adds a small but non-zero init cost.
  → **Mitigation**: Benchmark at startup. The set is tiny; expect < 5 ms total. If profiling shows otherwise, switch to pre-rasterised PNGs.

- **[Risk] Codicon SVG version drift** — If the vendored SVG files are not updated when the app's design language evolves, icons will look stale.
  → **Mitigation**: Pin a specific `@vscode/codicons` release in `package.json` and document the sync process in `assets/icons/README.md`.

- **[Risk] Dark / light mode icon colour** — Rasterising at startup captures pixels at a single colour. If the OS switches appearance, icon images may look wrong until the next restart.
  → **Mitigation**: Register for `NSAppearance` change notifications and call `IconRegistry::Reinit()` to re-rasterise. Accepted complexity; deferred to a follow-up task if needed.

- **[Risk] Sidebar favicon fallback divergence** — Web tabs in the React sidebar currently fall back to an inline `🌐` emoji if the favicon fails. Replacing this with `<Icon name="globe" />` changes the visual treatment without an explicit favicon-failed state in the bridge.
  → **Mitigation**: The `faviconFor()` function already signals URL → favicon URL; the `<Icon>` globe fallback is rendered by the `onError` handler in the `<img>` favicon element. No bridge change required.

## Migration Plan

1. **Vendor assets**: Copy Codicons SVGs (from `@vscode/codicons/dist/icons/`) for the required icon names into `assets/icons/`. Add `assets/icons/README.md` with the version pin.
2. **Native registry**: Add `app/browser/icon_registry.{h,cc}`, call `IconRegistry::Init()` from `DesktopApp::OnContextInitialized()` before window creation. No existing file is modified in this step.
3. **Native title bar**: Replace glyph-text `CefLabelButton` in `main_window.cc` with `MakeIconLabelButton` calls. Verify visually.
4. **Native tab toolbars**: Replace glyph-text buttons in `web_tab_behavior.cc` and `simple_tab_behavior.cc` with `MakeIconButton` or `MakeIconLabelButton` calls.
5. **React `<Icon>` component**: Add `web/src/shared/icons.ts` and `web/src/shared/components/Icon.tsx`. Add `<IconSprite>` to each panel root.
6. **React panels**: Replace all ad-hoc glyph/emoji usages in sidebar, popover, settings, terminal, FlowEditor with `<Icon name="...">`.
7. **Spec verification**: Confirm `sidebar-tabs`, `tab-flavor-web`, and `tab-toolbar` delta specs in this change supersede glyph references.

**Rollback**: All changes are local to the repo. Rolling back is a git revert of the implementation commits. The `IconRegistry::Init()` call can be gated behind an `#ifdef UNIFIED_ICONS` flag as a safety net during development but removed before merge.

## Open Questions

- Should `MakeIconButton` accept an `IconId` pair (`icon_normal`, `icon_hovered`) for explicit per-state override, or should the registry always handle state by adjusting opacity internally? **Lean towards registry-managed opacity for simplicity.**
- For the sidebar "identity" icons (terminal, chat, agent, graph), should the icon be a standalone button/image view or combined with the text label in a single `CefLabelButton`? The current `simple_tab_behavior.cc` uses one button for `"⌨ Terminal 2"`. An image view + separate label view in the leading slot may be cleaner — **resolve in icon-capable-controls spec.**
