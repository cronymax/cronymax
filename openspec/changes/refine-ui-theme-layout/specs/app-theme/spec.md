## ADDED Requirements

### Requirement: Token sets for Light and Dark

The renderer SHALL define two complete sets of design tokens — one for Light, one for Dark — covering at minimum: window background (`bg`), surface (`surface`), elevated surface (`surface-2`), border (`border`), foreground (`fg`), muted foreground (`fg-muted`), accent (`accent`), success, danger, warning. Both sets SHALL be defined in `web/src/styles/theme.css` and exposed as CSS custom properties on the document root.

#### Scenario: Light tokens applied

- **WHEN** the resolved theme mode is `light`
- **THEN** `getComputedStyle(document.documentElement).getPropertyValue('--color-cronymax')` returns the Light `bg` value, and Tailwind utilities such as `bg-cronymax` paint that color

#### Scenario: Dark tokens applied

- **WHEN** the resolved theme mode is `dark`
- **THEN** the same custom properties resolve to the Dark token values

#### Scenario: All required tokens exist in both modes

- **WHEN** the renderer style sheet is loaded
- **THEN** every token name listed above resolves to a non-empty value in both Light and Dark modes (no `undefined`/empty fallbacks)

---

### Requirement: System-follow is the default mode

The system SHALL default to `system` mode, which resolves to `light` or `dark` according to `prefers-color-scheme`. The resolved mode SHALL update live when the OS appearance changes.

#### Scenario: First launch with no stored preference

- **WHEN** the app launches and `space.kv` has no `ui.theme` row
- **THEN** the resolved mode equals `prefers-color-scheme: dark ? 'dark' : 'light'`, and `ui.theme` is written back as `"system"`

#### Scenario: OS appearance change while in system mode

- **WHEN** the user is in `system` mode and the OS switches from Light to Dark (or vice versa)
- **THEN** the renderer re-resolves the mode within one repaint and broadcasts `theme.changed` so the C++ chrome repaints in the same frame

---

### Requirement: Explicit user override

The user SHALL be able to set the theme to `light`, `dark`, or `system` from the Settings popover. The choice SHALL be persisted to `space.kv` under key `ui.theme` and SHALL take effect immediately across every panel and the C++ chrome.

#### Scenario: User picks Light

- **WHEN** the user selects "Light" in Settings
- **THEN** the renderer sends `theme.set { mode: "light" }`, `space.kv["ui.theme"]` becomes `"light"`, every panel repaints in Light, and the C++ title bar / sidebar / content frame repaint in Light

#### Scenario: Explicit choice ignores system preference

- **WHEN** the user has chosen `light` and the OS switches to Dark
- **THEN** the resolved mode stays `light` and no repaint is triggered

#### Scenario: Persisted choice restored on relaunch

- **WHEN** the app relaunches and `space.kv["ui.theme"]` is `"dark"`
- **THEN** the initial resolved mode is `dark` before the first frame is shown to the user

---

### Requirement: theme.get / theme.set / theme.changed channels

The system SHALL expose three bridge surfaces for theme:

- `theme.get` (renderer → C++): no payload, returns `{ mode: "system"|"light"|"dark", resolved: "light"|"dark" }`.
- `theme.set` (renderer → C++): payload `{ mode: "system"|"light"|"dark" }`, returns `{}`. Persists to `space.kv["ui.theme"]` and triggers a `theme.changed` broadcast.
- `theme.changed` (C++ → all panels): payload `{ mode, resolved, chrome: { window_bg, border, fg, fg_muted } }` (colors as `#RRGGBB`).

#### Scenario: Initial sync on panel load

- **WHEN** any panel mounts and calls `theme.get`
- **THEN** the response reflects the value currently persisted in `space.kv` and the resolved mode

#### Scenario: Set broadcasts to every panel

- **WHEN** one panel sends `theme.set { mode: "dark" }`
- **THEN** every other panel (sidebar, terminal, chat, agent, settings popover) receives `theme.changed` with `resolved: "dark"`

#### Scenario: Chrome colors included in broadcast

- **WHEN** `theme.changed` is dispatched
- **THEN** the payload's `chrome` object contains the four chrome colors as `#RRGGBB` strings, suitable for direct use by the C++ side

---

### Requirement: useTheme hook

The renderer SHALL provide a `useTheme()` React hook that returns `{ mode, resolved, setMode(mode) }`. The hook SHALL apply `data-theme="<resolved>"` to `<html>`, subscribe to `theme.changed`, and (when in `system` mode) subscribe to the `prefers-color-scheme` media query.

#### Scenario: data-theme attribute reflects resolved mode

- **WHEN** the resolved mode is `dark`
- **THEN** `document.documentElement.getAttribute('data-theme')` is `"dark"`

#### Scenario: setMode round-trips

- **WHEN** a component calls `setMode("light")`
- **THEN** the hook sends `theme.set { mode: "light" }`, the next render returns `{ mode: "light", resolved: "light" }`, and other components receive the matching `theme.changed`
