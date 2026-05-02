## ADDED Requirements

### Requirement: Chrome theme push channel

The system SHALL expose one bridge channel `tab.set_chrome_theme` (renderer → C++) accepting `{ tabId: string, color: string | null }` where `color` is a CSS color string (e.g., `"#1a1a2e"`, `"rgb(20,20,30)"`) or `null` to reset to the default dark fallback.

#### Scenario: Apply pushed color

- **WHEN** a renderer pushes `{ tabId, color: "#1a1a2e" }`
- **THEN** the addressed tab's toolbar background and card border tint update to that color

#### Scenario: Null resets to default

- **WHEN** a renderer pushes `{ tabId, color: null }`
- **THEN** the addressed tab's chrome reverts to the dark cronymax default `#0E0E10`

#### Scenario: Unparseable color rejected

- **WHEN** a renderer pushes `{ tabId, color: "not-a-color" }`
- **THEN** the C++ handler rejects the message, logs a warning, and the chrome is not updated

---

### Requirement: Color precedence rule

A renderer that wants its tab card to color-match its content SHALL determine the color in this order, picking the first that yields a parseable result:

1. The `content` attribute of the first `<meta name="theme-color">` element.
2. The computed `background-color` of `document.body`, when non-transparent.
3. `null` (= dark fallback).

The renderer SHALL re-evaluate on initial load, on `<meta>` mutation (via `MutationObserver`), and on `body` style mutation. Changes SHALL be debounced to ≤ 4 pushes per second per tab.

#### Scenario: Meta tag wins

- **WHEN** a page has `<meta name="theme-color" content="#abcdef">` AND a body background of `#123456`
- **THEN** the pushed color is `#abcdef`

#### Scenario: Body background as fallback

- **WHEN** a page has no `<meta name="theme-color">` AND its body background is `rgb(30, 41, 59)`
- **THEN** the pushed color is the body background

#### Scenario: Debounce

- **WHEN** a renderer's theme-color mutates 30 times in one second
- **THEN** the bridge receives at most 4 `tab.set_chrome_theme` messages from that tab in that second

---

### Requirement: Hold previous color across web navigation

For web tabs, the system SHALL hold the previously applied chrome color from `loadStart` until either `loadEnd` or the first `tab.set_chrome_theme` push for the navigation, whichever comes first. The hold SHALL expire after a maximum of 200 ms after `loadEnd`, after which the chrome reverts to the dark fallback if no push has occurred.

#### Scenario: No flash on navigation

- **WHEN** a web tab navigates from page A (color `#abcdef`) to page B
- **THEN** the chrome stays `#abcdef` until page B pushes its color OR 200 ms after `loadEnd`

#### Scenario: Hold expires

- **WHEN** a web tab navigates and the new page never pushes a chrome color
- **THEN** within 200 ms after `loadEnd`, the chrome reverts to the dark fallback

---

### Requirement: Fixed-chrome kinds

Terminal and Graph tabs SHALL use the dark fallback chrome unconditionally and SHALL NOT inject a theme-color sampler. Web, Chat, and Agent tabs MAY inject the sampler.

#### Scenario: Terminal chrome is fixed

- **WHEN** any terminal tab is active
- **THEN** its chrome is the dark fallback `#0E0E10` regardless of any content rendered inside the terminal

#### Scenario: Graph chrome is fixed

- **WHEN** any graph tab is active
- **THEN** its chrome is the dark fallback `#0E0E10`
