## ADDED Requirements

### Requirement: Title bar, sidebar, and window share one chrome color

The title bar background, the sidebar background, and the macOS window background SHALL paint the exact same `cef_color_t` value, sourced from the active theme's chrome `window_bg` token. No layer in this set SHALL hold a hard-coded color literal.

#### Scenario: Same color across surfaces

- **WHEN** the app is rendered in any theme
- **THEN** the pixel color of the title bar, the sidebar, and the macOS window background match exactly (sampled outside any child widget)

#### Scenario: Theme switch repaints all three together

- **WHEN** `theme.changed` is broadcast
- **THEN** the title bar, the sidebar, and the macOS window background all repaint to the new `window_bg` within the same frame

#### Scenario: No color literals remain

- **WHEN** the C++ source is searched
- **THEN** no `0xFF14141A` (or any other hex color literal) appears in `BuildTitleBar`, `BuildChrome`, or the macOS window background helper for the chrome color path

---

### Requirement: Content panel rounded card frame

The content area SHALL be wrapped in a `content_frame_` `CefPanel` that paints a 1 px border in the theme's `border` color, has a 12 px corner radius, and is inset 8 px from the sidebar (left), the title bar (top), and the window edges (right, bottom). Active tab cards mount inside the frame.

#### Scenario: Visible border and rounding

- **WHEN** the window is shown at any size in any theme
- **THEN** the content area presents a 12 px-rounded card with a 1 px border in the theme's `border` color, separated from the sidebar / title bar / window edges by 8 px on each side

#### Scenario: Tab card clipped to rounded frame

- **WHEN** a web tab is active
- **THEN** the tab card's BrowserView is clipped to the rounded frame on macOS (no square corners poking past the rounded edge)

#### Scenario: Frame border tracks theme

- **WHEN** the theme changes from Dark to Light
- **THEN** the frame's border color updates to the Light theme's `border` token without re-creating the frame view or any tab card

---

### Requirement: Chrome state is centralized in MainWindow

`MainWindow` SHALL hold a `ThemeChrome` struct (`window_bg`, `border`, `fg`, `fg_muted`) and expose `ApplyThemeChrome(const ThemeChrome&)` as the single entry point that updates the title bar, sidebar background, content frame border, and macOS window background. Construction SHALL apply an initial chrome derived from the persisted `ui.theme` value before the window is shown.

#### Scenario: ApplyThemeChrome is the only mutator

- **WHEN** the chrome color path changes
- **THEN** every call site that touches title-bar / sidebar / window background colors goes through `ApplyThemeChrome`

#### Scenario: Initial chrome before first paint

- **WHEN** the window is constructed and `ui.theme` is `"dark"` in `space.kv`
- **THEN** `ApplyThemeChrome` is called with the Dark chrome before `window->Show()`, so the user never sees a Light-flash on a dark-mode launch
