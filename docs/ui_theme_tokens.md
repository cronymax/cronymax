# UI theme tokens

This document describes the design-token system used by the Cronymax
renderer, including the naming conventions and how tokens flow from CSS
variables to Tailwind utility classes and native shell chrome.

## Token naming convention

All Cronymax-specific design tokens use the `cronymax` namespace prefix
to avoid collisions with Tailwind's own built-in tokens and any
third-party component libraries.

| Layer | Pattern | Example |
|---|---|---|
| CSS variable (color) | `--color-cronymax-<role>` | `--color-cronymax-bg-body` |
| CSS variable (radius) | `--radius-cronymax-<name>` | `--radius-cronymax-pill` |
| CSS variable (shadow) | `--shadow-cronymax-<name>` | `--shadow-cronymax-elev-2` |
| Tailwind utility | `<property>-cronymax-<role>` | `bg-cronymax-bg-body` |

### Why `--color-cronymax-*` instead of `--cronymax-*`

Tailwind v4 reads `@theme {}` and auto-generates utility classes only
from variables in specific namespaces.  A variable named
`--color-cronymax-bg-body` produces utilities `bg-cronymax-bg-body`,
`text-cronymax-bg-body`, `border-cronymax-bg-body`, etc., for free.
A variable named `--cronymax-bg-body` (without the `--color-` prefix)
would produce **no** utilities and would require manual `@utility`
blocks instead.

## Defined tokens

### Colors (`web/src/styles/theme.css`)

The `@theme {}` block defines the dark-mode defaults.  Per-theme
overrides live in `:root[data-theme="light"]`,
`:root[data-theme="dark"]`, and
`@media (prefers-color-scheme: dark) { :root:not([data-theme]) { … } }`.

| Token | Role |
|---|---|
| `--color-cronymax-bg-body` | Outermost window / sidebar background |
| `--color-cronymax-bg-base` | Default page / panel surface |
| `--color-cronymax-bg-float` | Elevated surface (hover states, cards) |
| `--color-cronymax-bg-mask` | Scrim / overlay backdrop |
| `--color-cronymax-border` | Default 1 px border / divider |
| `--color-cronymax-text-title` | Primary foreground text |
| `--color-cronymax-text-caption` | Secondary / muted text |
| `--color-cronymax-primary` | Accent / interactive primary |
| `--color-cronymax-primary-soft` | Muted accent background |
| `--color-cronymax-success` | Success state |
| `--color-cronymax-danger` | Error / destructive state |
| `--color-cronymax-warning` | Warning state |
| `--color-cronymax-info` | Informational state |

### Radii

| Token | Value | Use |
|---|---|---|
| `--radius-cronymax-pill` | `9999px` | Pill-shaped tags and badges |
| `--radius-cronymax-popover` | `12px` | Popovers and floating panels |

### Shadows

| Token | Use |
|---|---|
| `--shadow-cronymax-popover` | Popover drop shadow |
| `--shadow-cronymax-elev-1` | Subtle surface elevation |
| `--shadow-cronymax-elev-2` | Pronounced surface elevation |

## Native shell chrome bridge

The bridge payload `theme.changed` mirrors a subset of renderer tokens
into `MainWindow::ThemeChrome` (C++) so the native title bar, window
background, and content frame border stay in sync with the renderer
theme without an IPC round-trip per paint:

```cpp
struct ThemeChrome {
  cef_color_t bg_body;    // --color-cronymax-bg-body
  cef_color_t bg_base;    // --color-cronymax-bg-base
  cef_color_t bg_float;   // --color-cronymax-bg-float
  cef_color_t text_title; // --color-cronymax-text-title
  cef_color_t border;     // --color-cronymax-border
};
```

`theme_sampler.ts` reads these values from `document.documentElement`
computed styles and packages them into the `chrome` field of the
`theme.changed` event:

```ts
getComputedStyle(document.documentElement)
  .getPropertyValue("--color-cronymax-bg-base")
```

## Migration note (renamed from `--color-ui-*`)

Prior to this refactor all tokens used an `--color-ui-*` prefix.  The
rename to `--color-cronymax-*` was performed globally across:

* `web/src/styles/theme.css` — variable declarations in all four theme blocks
* `web/src/theme_sampler.ts` — `getPropertyValue` call sites
* All TSX/TS component files — Tailwind utility class names
  (`bg-ui-*` → `bg-cronymax-*`, `text-ui-*` → `text-cronymax-*`, etc.)

The `--color-` Tailwind namespace prefix was preserved throughout so
utility auto-generation continued to work without any `@utility` additions.
