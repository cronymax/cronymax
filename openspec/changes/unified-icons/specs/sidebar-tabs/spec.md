## MODIFIED Requirements

### Requirement: Row icons keyed by tab kind

Row icons SHALL be picked from the tab's `kind` field using semantic icon IDs from the icon registry: `web` → site favicon (with `<Icon name="globe">` as fallback), `terminal` → `<Icon name="terminal">`, `chat` → `<Icon name="comment-discussion">`, `agent` → `<Icon name="settings-gear">`, `graph` → `<Icon name="type-hierarchy">`. Icon glyph characters (keyboard emoji, speech-bubble emoji, cog emoji, graph emoji) SHALL NOT be used. Every `<Icon>` used in a sidebar row SHALL carry an `aria-label` equal to the tab kind.

#### Scenario: Web row uses favicon with icon fallback

- **WHEN** a web tab has a non-empty `url`
- **THEN** the row icon is the host's favicon loaded via `<img src={faviconUrl} onError={...}>` and on error the `onError` handler renders `<Icon name="globe" aria-label="web" />` instead

#### Scenario: Terminal row uses semantic icon

- **WHEN** a row has `kind = "terminal"`
- **THEN** the row icon is rendered as `<Icon name="terminal" aria-label="terminal" />`, not the `"⌨"` character

#### Scenario: Chat row uses semantic icon

- **WHEN** a row has `kind = "chat"`
- **THEN** the row icon is rendered as `<Icon name="comment-discussion" aria-label="chat" />`, not the `"💬"` character

#### Scenario: Agent row uses semantic icon

- **WHEN** a row has `kind = "agent"`
- **THEN** the row icon is rendered as `<Icon name="settings-gear" aria-label="agent" />`, not the `"⚙"` character

#### Scenario: Graph row uses semantic icon

- **WHEN** a row has `kind = "graph"`
- **THEN** the row icon is rendered as `<Icon name="type-hierarchy" aria-label="graph" />`, not the `"▦"` character
