## ADDED Requirements

### Requirement: Unified shell background color

The system SHALL paint the title bar, sidebar, and native window background from the same resolved `bg_body` token in every supported theme mode.

#### Scenario: Title bar and sidebar match

- **WHEN** the app is rendered in Light or Dark mode
- **THEN** the title bar and sidebar backgrounds match the same `bg_body` value

#### Scenario: Native window background follows shell token

- **WHEN** the resolved theme changes
- **THEN** the native window background repaints to the same `bg_body` value used by the title bar and sidebar

### Requirement: Layered shell surfaces

The system SHALL assign shell surfaces by role: `bg_body` for outer chrome, `bg_base` for the content frame, `bg_float` for floating surfaces such as popovers and menus, and `bg_mask` for overlay scrims.

#### Scenario: Content frame uses base surface

- **WHEN** the content frame is displayed inside the shell
- **THEN** its background derives from `bg_base` rather than `bg_body`

#### Scenario: Popover uses floating surface

- **WHEN** a floating settings or menu surface is shown
- **THEN** that surface uses `bg_float` and remains visually distinct from the underlying `bg_base` or `bg_body`

#### Scenario: Overlay uses mask surface

- **WHEN** a modal or scrim layer is displayed
- **THEN** the overlay derives from `bg_mask`

### Requirement: Text and line roles are stable across shell surfaces

The system SHALL use `text_title` and `text_caption` for shell text hierarchy and `border` and `divider` for shell structural lines. Shell surfaces SHALL NOT introduce hard-coded local text or line colors outside the token system.

#### Scenario: Shell heading uses title text role

- **WHEN** a primary shell label such as a title-bar button label or active section label is rendered
- **THEN** it uses `text_title`

#### Scenario: Shell separators use line roles

- **WHEN** the content frame border or a shell divider is painted
- **THEN** the system uses `border` or `divider` from the token system rather than a hard-coded literal

### Requirement: Theme mirror to native shell subset

The system SHALL mirror the shell-relevant subset of the semantic theme tokens from the renderer to native code so native surfaces and web-rendered shell surfaces remain synchronized.

#### Scenario: Native shell receives resolved subset

- **WHEN** the resolved theme is applied
- **THEN** native code receives the token values required to paint `bg_body`, `bg_base`, `bg_float`, `bg_mask`, `border`, `text_title`, and `text_caption`

#### Scenario: Renderer and native shell stay in sync

- **WHEN** the user switches theme mode
- **THEN** the native shell and renderer shell surfaces repaint from the same resolved token set without diverging surface roles
