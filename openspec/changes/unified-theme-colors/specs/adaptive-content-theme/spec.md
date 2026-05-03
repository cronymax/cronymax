## ADDED Requirements

### Requirement: Content adaptation respects shell boundaries

The system SHALL allow webpage color signals to influence the active content presentation without allowing those signals to repaint shell chrome. The title bar, sidebar, window background, and outer content frame remain token-driven.

#### Scenario: Webpage color does not repaint outer chrome

- **WHEN** an active webpage exposes a strong `theme-color` or body background color
- **THEN** the title bar, sidebar, and native window background remain unchanged by that webpage color

#### Scenario: Adaptation is confined to content presentation

- **WHEN** a webpage color is accepted for adaptation
- **THEN** only content-local presentation such as tab-local chrome tint or an inner content surface may reflect that color

### Requirement: Deterministic page-color precedence

The system SHALL derive webpage color for adaptation using the following precedence order: first `meta[name="theme-color"]`, then a non-transparent computed `body` background color, and finally no override if neither yields a usable color.

#### Scenario: Theme-color metadata wins

- **WHEN** a page provides both `meta[name="theme-color"]` and a non-transparent body background
- **THEN** the adaptation source is the `theme-color` value

#### Scenario: Body background is fallback

- **WHEN** a page has no usable `theme-color` metadata and has a non-transparent body background
- **THEN** the adaptation source is the body background color

#### Scenario: No usable page color falls back to app theme

- **WHEN** neither `theme-color` nor body background yields a usable color
- **THEN** content adaptation falls back to the app's token-driven neutral presentation

### Requirement: Adapted content color is constrained for readability

The system SHALL clamp or reject sampled webpage colors that would reduce readability or erase the visual separation between adapted content and shell surfaces.

#### Scenario: Low-contrast sample is rejected

- **WHEN** a sampled webpage color would make content text or controls insufficiently readable
- **THEN** the system rejects or adjusts that color before applying it

#### Scenario: Extreme sample is normalized

- **WHEN** a sampled webpage color is extremely bright, dark, or saturated
- **THEN** the system normalizes the color to remain visually compatible with the surrounding token-driven shell

### Requirement: Adaptation updates as the active page theme changes

The system SHALL re-evaluate the active page's adaptation color when page theme metadata or body background changes and SHALL update the active content presentation accordingly.

#### Scenario: Page theme change updates active content treatment

- **WHEN** the active page changes its `theme-color` metadata or body background at runtime
- **THEN** the system recomputes the adaptation color and updates the active content presentation

#### Scenario: Inactive pages do not repaint active content

- **WHEN** a background tab changes its page color signals
- **THEN** the active content presentation does not repaint until that tab becomes active
