## ADDED Requirements

### Requirement: Canonical semantic theme taxonomy

The system SHALL define one canonical semantic theme taxonomy for both Light and Dark modes. The taxonomy SHALL include at minimum: `primary`, `secondary`, `bg_body`, `bg_base`, `bg_float`, `bg_mask`, `fill_active`, `fill_disabled`, `fill_focus`, `fill_hover`, `fill_pressed`, `fill_selected`, `fill_tag`, `text_title`, `text_caption`, `text_disabled`, `text_placeholder`, `border`, `divider`, `info`, `success`, `warning`, and `error`.

#### Scenario: Every token exists in Light mode

- **WHEN** the Light theme is resolved
- **THEN** every token in the semantic taxonomy resolves to a non-empty value

#### Scenario: Every token exists in Dark mode

- **WHEN** the Dark theme is resolved
- **THEN** every token in the semantic taxonomy resolves to a non-empty value

### Requirement: Teal-mint brand palette

The system SHALL define the brand axis with teal as `primary` and mint as `secondary` in both Light and Dark themes. Interactive state tokens derived from the brand axis SHALL remain teal-dominant so active and focused states read as intentional controls rather than decorative highlights.

#### Scenario: Active controls use teal-dominant emphasis

- **WHEN** a control enters an active or focused state
- **THEN** the applied `fill_active` or `fill_focus` value is derived from the `primary` teal axis rather than the `secondary` mint axis

#### Scenario: Secondary accent remains supportive

- **WHEN** the interface renders a supportive accent such as a tag or soft highlight
- **THEN** the system may use `secondary` or `fill_tag` without replacing the `primary` action color

### Requirement: Semantic status colors remain distinct from the brand axis

The system SHALL define `info`, `success`, `warning`, and `error` as semantic function colors separate from `primary` and `secondary`. Success SHALL NOT reuse the same teal-dominant values used for primary brand actions.

#### Scenario: Success is visually distinct from primary action

- **WHEN** a success state and a primary action are displayed side by side
- **THEN** the success color is distinguishable from `primary` without relying on labels alone

#### Scenario: Brand tokens do not replace semantic tokens

- **WHEN** a component needs to represent informational, warning, or error status
- **THEN** it uses `info`, `warning`, or `error` rather than `primary` or `secondary`

### Requirement: Legacy token compatibility during migration

The system SHALL provide a compatibility mapping from legacy renderer token names to the semantic taxonomy during migration. The compatibility mapping SHALL preserve existing visual output closely enough that panels can migrate incrementally.

#### Scenario: Legacy token consumer resolves semantic value

- **WHEN** a panel still references a legacy `cronymax-*` token during the migration window
- **THEN** that token resolves through the compatibility mapping to the corresponding semantic token value

#### Scenario: New panels consume semantic tokens directly

- **WHEN** a new or migrated panel is authored against the theme system
- **THEN** it uses the semantic taxonomy instead of introducing additional local color names
