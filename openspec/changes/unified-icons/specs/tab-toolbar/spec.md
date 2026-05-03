## MODIFIED Requirements

### Requirement: Native CEF Views toolbar per tab

Each tab SHALL render a toolbar built from CEF Views primitives (`CefPanel`, `CefBoxLayout`, `CefTextfield`, `CefLabelButton`, `CefImageView`). The toolbar SHALL NOT be implemented as an HTML page in a separate `CefBrowserView`. Each tab SHALL own its own toolbar instance; no toolbar SHALL be shared across tabs.

Action buttons (back, forward, refresh, stop, new-tab, close, restart, config) in toolbars SHALL be created via `MakeIconButton(delegate, IconId, accessible_name)` from `icon_registry.h`. Identity controls that show both an icon and a text label (e.g., the leading "icon + Terminal 2" slot in `simple_tab_behavior`) SHALL be created via `MakeIconLabelButton(delegate, IconId, label, accessible_name)`. No toolbar button SHALL use a Unicode or emoji glyph character as its sole visual icon representation.

#### Scenario: Toolbar is native

- **WHEN** any tab is constructed
- **THEN** its toolbar is a `CefPanel` populated with CEF Views widgets, NOT a `CefBrowserView` loading an HTML document

#### Scenario: Per-tab ownership

- **WHEN** the user switches between two tabs
- **THEN** each tab's toolbar widget tree is preserved across switches; no toolbar state is lost or recomputed

#### Scenario: Action buttons use SetImage

- **WHEN** any action button (back, forward, refresh, stop, close, new-tab, restart, config) is inspected in the native view tree
- **THEN** the button has a non-null `CefImage` set via `SetImage(CEF_BUTTON_STATE_NORMAL, ...)` and its visible text label is empty

#### Scenario: Identity buttons retain text label

- **WHEN** the leading slot of a terminal tab toolbar is inspected
- **THEN** the button has both a non-null `CefImage` (the terminal icon) and a non-empty text label (e.g., "Terminal 2")
