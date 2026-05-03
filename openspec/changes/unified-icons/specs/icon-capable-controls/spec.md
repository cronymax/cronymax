## ADDED Requirements

### Requirement: Native icon-only button factory

The system SHALL provide a free function `MakeIconButton(CefRefPtr<CefButtonDelegate>, IconId, std::string_view accessible_name)` in `app/browser/icon_registry.h` that creates a `CefLabelButton` with empty text, sets the normal/hovered/disabled images from the registry, and calls `SetAccessibleName(accessible_name)`. The returned button SHALL have no visible text.

#### Scenario: No visible text

- **WHEN** `MakeIconButton(delegate, IconId::kClose, "Close")` is called
- **THEN** the returned `CefLabelButton` displays only the icon image; no text glyph is visible

#### Scenario: Accessible name present

- **WHEN** the accessibility tree is inspected for a button created with `MakeIconButton`
- **THEN** the button's accessible name equals the string passed as `accessible_name`

---

### Requirement: Native icon+label button factory

The system SHALL provide a free function `MakeIconLabelButton(CefRefPtr<CefButtonDelegate>, IconId, std::string_view label, std::string_view accessible_name)` in `app/browser/icon_registry.h` that creates a `CefLabelButton` with visible text equal to `label`, sets images from the registry, and calls `SetAccessibleName(accessible_name)`.

#### Scenario: Text and icon both visible

- **WHEN** `MakeIconLabelButton(delegate, IconId::kTabTerminal, "Terminal 2", "Terminal 2")` is called
- **THEN** the returned button displays both the terminal icon and the text "Terminal 2"

---

### Requirement: Native icon-only button has tooltip

Every button produced by `MakeIconButton` SHALL have a tooltip set via `SetTooltipText()` equal to its `accessible_name`. Buttons produced by `MakeIconLabelButton` SHOULD also set a tooltip equal to the label, but it is not required if the label text is always visible.

#### Scenario: Tooltip on hover

- **WHEN** the user hovers the pointer over a native icon-only button
- **THEN** the operating system displays a tooltip with the button's accessible name

---

### Requirement: React `<Icon>` component

The system SHALL provide a `web/src/shared/components/Icon.tsx` component accepting a `name: IconName` prop and an optional `size?: number` prop (default 16). The component SHALL render an inline SVG element sourced from the Codicons sprite. The component SHALL accept all standard HTML attributes for `<svg>` including `aria-label`, `className`, and `style`.

#### Scenario: Renders SVG

- **WHEN** `<Icon name="arrow-left" />` is rendered
- **THEN** the DOM contains an `<svg>` element with a `<use>` reference to the `#arrow-left` symbol in the Codicons sprite

#### Scenario: Size prop respected

- **WHEN** `<Icon name="refresh" size={20} />` is rendered
- **THEN** the SVG element has `width="20"` and `height="20"` attributes

---

### Requirement: React `IconName` type

The system SHALL define a TypeScript string-union type `IconName` in `web/src/shared/icons.ts` enumerating all Codicon names used by the app. This type SHALL be the sole union used in `<Icon name>` props, action bar configs, and sidebar row configs. Adding a new icon SHALL require adding it to `IconName`.

#### Scenario: Type error on unknown name

- **WHEN** a developer writes `<Icon name="not-a-real-icon" />`
- **THEN** TypeScript reports a compile-time type error

---

### Requirement: Codicons sprite injection

The system SHALL inject the Codicons SVG sprite once per panel root via a `<IconSprite>` component rendered as a visually-hidden element. Each panel root component (sidebar, popover, settings, terminal, FlowEditor) SHALL include `<IconSprite>` at the top level.

#### Scenario: Sprite injected once

- **WHEN** any panel root renders
- **THEN** the DOM contains exactly one `<div aria-hidden="true">` element with the full Codicons SVG sprite as its content

#### Scenario: Icon resolves from sprite

- **WHEN** an `<Icon name="close">` is rendered in a panel that has `<IconSprite>` in its root
- **THEN** the `<use>` reference resolves to the sprite's `#close` symbol without a network request

---

### Requirement: Replace all ad-hoc glyphs in React panels

Every Unicode/emoji glyph used as an icon in React panel components SHALL be replaced with an `<Icon>` call. Affected locations: `sidebar/App.tsx` (`glyphFor()`), `popover/App.tsx` (action buttons), `settings/App.tsx` (close button), `terminal/App.tsx` (ActionBar), `FlowEditor/index.tsx` (toolbar and dialog close). Plain text that is not acting as an icon (button labels, model names, file names) SHALL NOT be changed.

#### Scenario: Sidebar tab rows use `<Icon>`

- **WHEN** the sidebar renders a terminal tab row
- **THEN** the row icon is rendered via `<Icon name="terminal" />`, not a `"⌨"` character

#### Scenario: Popover action buttons use `<Icon>`

- **WHEN** the popover panel renders the reload button
- **THEN** the button contains `<Icon name="refresh" />`, not `"↻"`
