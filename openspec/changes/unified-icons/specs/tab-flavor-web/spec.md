## MODIFIED Requirements

### Requirement: Web tab toolbar layout

A web tab's toolbar SHALL populate slots as follows:

- **leading**: back button (icon `kBack`, accessible name `"Back"`), forward button (icon `kForward`, accessible name `"Forward"`), refresh button (icon `kRefresh`, accessible name `"Refresh"`).
- **middle**: an editable URL pill (`CefTextfield`) showing the current URL.
- **trailing**: a "new tab" button (icon `kNewTab`, accessible name `"New Tab"`).

Back/forward enabled state SHALL be derived from the owning `CefBrowser`'s navigation history. While loading, the refresh button's image SHALL be swapped to icon `kStop` (accessible name `"Stop"`) and restored to `kRefresh` when loading completes. All toolbar buttons SHALL be produced via `MakeIconButton` from `icon_registry.h`. No Unicode glyph characters (`◀`, `▶`, `⟳`, `✕`, `⊕`) SHALL appear as button text.

#### Scenario: Layout

- **WHEN** a web tab is constructed
- **THEN** its toolbar's leading slot contains back/forward/refresh buttons each with an icon image and no visible text; middle contains a URL textfield; trailing contains the new-tab icon button

#### Scenario: Back/forward reflect history

- **WHEN** a web tab can go back in its navigation history
- **THEN** the back button (`kBack`) is enabled; otherwise its disabled-state image is shown

#### Scenario: Refresh becomes stop while loading

- **WHEN** the web tab begins loading
- **THEN** the refresh button's image is swapped to `IconRegistry::GetImage(IconId::kStop)` and clicking it stops the load; when loading finishes the image reverts to `IconRegistry::GetImage(IconId::kRefresh)`

#### Scenario: New-tab button accessible name

- **WHEN** the accessibility tree is inspected for the trailing new-tab button
- **THEN** the accessible name is `"New Tab"`
