## Why

The shell currently expresses every icon as a Unicode or emoji string literal scattered across native C++ (`CefLabelButton` labels in `main_window.cc`, `web_tab_behavior.cc`, `simple_tab_behavior.cc`) and React renderer panels (emoji in the sidebar's `glyphFor()`, Unicode in settings/popover close buttons, emoji in the FlowEditor toolbar). There is no shared vocabulary, no single place to change an icon, and the existing OpenSpec requirements for sidebar rows and tab toolbars are written in terms of specific glyph characters that can never be made consistent with VS Code SVG icons without revising every spec. Fixing this now is the right moment because `arc-style-tab-cards` and `native-title-bar` have stabilised the native control surface and the React migration has established the shared `src/shared/` primitive layer — there is now a clean extension point on both sides.

## What Changes

- Introduce a small **native icon registry** (`IconRegistry`) that maps semantic `IconId` enums to `CefImage` objects rasterised at startup from a set of SVG sources bundled with the app. The registry owns all per-state (normal / hovered / disabled) images and is the single place to update a native icon.
- Replace every `CefLabelButton` whose label was carrying a Unicode glyph as icon (◀ ▶ ↻ ✕ ⊕ ⚙ ⊕ ⌨ 💬) with an image-first `CefLabelButton` that calls `SetImage(...)` from the registry and sets an explicit accessible name via `SetAccessibleName(...)`. Text labels that carry real identity (e.g. `Terminal 2`, `Chat 1`) are **kept**; only the glyph-as-icon part is replaced.
- Introduce a shared **React `<Icon>` component** (`src/shared/components/Icon.tsx`) that renders a named VS Code SVG icon. Replace all ad-hoc emoji and Unicode glyphs in React panels with `<Icon name="..." />` calls.
- **BREAKING (spec language)**: Revise requirements in `sidebar-tabs`, `tab-flavor-web`, and `tab-toolbar` that reference specific glyph characters. Requirements SHALL now reference semantic action roles (`back`, `forward`, `refresh`, `stop`, `new-tab`, `close`, `settings`, `tab.terminal`, `tab.chat`, `tab.agent`, `tab.graph`, `tab.web`) instead of literal characters.

## Capabilities

### New Capabilities

- `icon-registry`: Semantic icon ID enum + native C++ registry that rasterises SVG sources into `CefImage` objects at startup. Covers the full set of icon IDs used across title bar, tab toolbars, and sidebar. Covers per-state image assignment, HiDPI scale factors, and theme-aware recolouring via tinting.
- `icon-capable-controls`: Conventions and helpers for building icon-bearing native controls (`MakeIconButton`, `MakeIconLabelButton`) and the shared React `<Icon>` component. Covers icon-only vs icon+label usage rules and accessibility requirements.

### Modified Capabilities

- `sidebar-tabs`: Row icon requirement changes from "keyboard/speech-bubble/cog/graph/globe glyph characters" to semantic icon IDs from the registry. Web rows keep real favicons with a semantic globe-icon fallback.
- `tab-flavor-web`: Toolbar button requirements change from glyph literals (`◀`, `▶`, `⟳`, `✕`, `⊕`) to semantic icon roles (`back`, `forward`, `refresh`, `stop`, `new-tab`).
- `tab-toolbar`: Toolbar slot requirements change from accepting glyph-text buttons to requiring icon-capable controls (`SetImage`-bearing `CefLabelButton` or icon-only controls with accessible names).

## Impact

- **C++**: `app/browser/main_window.cc` (title-bar buttons), `app/browser/tab_behaviors/web_tab_behavior.cc` (nav buttons), `app/browser/tab_behaviors/simple_tab_behavior.cc` (leading icon+name label). New files: `app/browser/icon_registry.{h,cc}`.
- **Assets**: New `assets/icons/` directory containing the SVG sources for every `IconId`. At startup/build, SVGs are rasterised (or bundled pre-rasterised PNGs) and loaded into the registry.
- **Web**: `web/src/shared/components/Icon.tsx` (new). Updates to `web/src/panels/sidebar/App.tsx`, `web/src/panels/settings/App.tsx`, `web/src/panels/popover/App.tsx`, `web/src/components/FlowEditor/index.tsx`, `web/src/panels/terminal/App.tsx`.
- **No new npm dependencies required**: VS Code Codicon SVGs can be vendored directly or sourced via the existing `@vscode/codicons` package.
- **No bridge changes**: Icon system is a purely presentational layer.
- **Existing OpenSpec artifacts in `arc-style-tab-cards` and `refine-ui-theme-layout`**: The glyph-character requirement language in those spec files is superseded; delta specs in this change apply on top.
