## Why

The current theme system mixes hard-coded native colors, a limited renderer token set, and per-tab color overrides that stop short of a coherent shell-wide model. That makes it difficult to redesign the brand around teal-mint, keep title bar and sidebar visually unified, and define a predictable rule for how content surfaces should harmonize with page colors without destabilizing the app chrome.

## What Changes

- Introduce a unified theme token taxonomy that covers brand, backgrounds, fills, text, lines, and semantic function colors for both Light and Dark modes.
- Redefine the app's primary/secondary brand axis around a teal-mint palette intended to read as intelligent and flexible.
- Make the title bar, sidebar, and native window background derive from the same shell background token so the app chrome reads as one continuous surface.
- Split shell backgrounds into layered roles (`bg_body`, `bg_base`, `bg_float`, `bg_mask`) so the content frame, popovers, and overlays are visually consistent across native and web surfaces.
- Define how adaptive page color should work: web content may harmonize with page background/theme color inside the content frame, while shell chrome remains token-driven and stable.
- Replace the current minimal token naming with a generalized token suite that downstream panels can consume without re-inventing local color semantics.
- **BREAKING:** existing renderer tokens such as `cronymax-bg`, `cronymax-surface`, and related component-level assumptions will be remapped onto the new taxonomy, requiring panel styles to migrate to the new token names.

## Capabilities

### New Capabilities

- `theme-token-system`: Defines the canonical Light and Dark token taxonomy for theme, background, fill, text, line, and function colors.
- `shell-chrome-theme`: Defines how title bar, sidebar, window background, content frame, and floating surfaces derive from the unified token system.
- `adaptive-content-theme`: Defines how the content panel harmonizes with webpage background or `theme-color` signals without letting external pages override app chrome.

### Modified Capabilities

- None.

## Impact

- Web theme foundations in `web/src/styles/theme.css` and any panel styles that consume the current `cronymax-*` tokens.
- Native chrome color plumbing in `app/browser/main_window.*`, `app/browser/mac_view_style.*`, and bridge paths that synchronize resolved theme state.
- Renderer-native integration points for adaptive tab chrome and content-surface color handling.
- OpenSpec artifacts for the new token system, shell chrome behavior, and adaptive content-color behavior.
