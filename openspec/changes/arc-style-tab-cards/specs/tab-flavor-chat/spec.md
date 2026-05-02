## ADDED Requirements

### Requirement: Chat tab toolbar layout

A chat tab's toolbar SHALL populate slots as follows:

- **leading**: a chat glyph icon plus the conversation's display name.
- **middle**: the model name (e.g., `claude-3-5-sonnet`) and the message count (e.g., `12 msg`).
- **trailing**: clear-conversation button (`🗑` / `Clear`), settings button (`⚙`).

#### Scenario: Layout

- **WHEN** a chat tab is constructed
- **THEN** its toolbar's leading slot contains an icon + name, middle contains model + message count, trailing contains clear + settings

---

### Requirement: Chat tab state push

A chat tab's renderer SHALL push `tab.set_toolbar_state` with `kind: "chat"` whenever its name, model, or message count changes. The payload schema SHALL be `{ name: string, model: string, messageCount: number }`.

#### Scenario: Message arrives

- **WHEN** the user sends a new message in a chat tab
- **THEN** within one debounce window, the renderer pushes `tab.set_toolbar_state` with the incremented `messageCount`

#### Scenario: Model swap

- **WHEN** the user changes the model in a chat tab
- **THEN** the renderer pushes `tab.set_toolbar_state` with the new `model` value

---

### Requirement: Clear conversation

Clicking the clear-conversation button SHALL prompt for confirmation. On confirmation, the chat history SHALL be cleared, the message count SHALL reset to zero, and the renderer SHALL push the updated toolbar state. The tab id SHALL remain unchanged.

#### Scenario: Confirmed clear

- **WHEN** the user clicks Clear and confirms
- **THEN** the chat history is empty, `messageCount` is 0 in the next push, and the tab remains in the sidebar at the same position

#### Scenario: Cancelled clear

- **WHEN** the user clicks Clear and cancels
- **THEN** nothing changes

---

### Requirement: Chat tab pushes chrome theme

A chat tab's renderer SHALL inject the chrome theme sampler defined in `tab-chrome-theme` and push `tab.set_chrome_theme` per its precedence rules.

#### Scenario: Sampler injected

- **WHEN** a chat tab loads
- **THEN** the renderer injects the theme sampler script
