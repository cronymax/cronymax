## ADDED Requirements

### Requirement: Terminal tab toolbar layout

A terminal tab's toolbar SHALL populate slots as follows:

- **leading**: a terminal glyph icon (`▶_`) plus the terminal's display name.
- **middle**: the current working directory (truncated with ellipsis as needed) plus a state indicator (`idle` | `running` | `exited`).
- **trailing**: shell name label (e.g., `zsh`), restart button (`⟳`), config button (`⚙`).

#### Scenario: Layout

- **WHEN** a terminal tab is constructed
- **THEN** its toolbar's leading slot contains an icon + name, middle contains cwd + state, trailing contains shell + restart + config

---

### Requirement: Terminal tab state push

A terminal tab's renderer SHALL push `tab.set_toolbar_state` with `kind: "terminal"` whenever its name, cwd, state, or shell changes. The payload schema SHALL be `{ name: string, cwd?: string, state: "idle" | "running" | "exited", shell: string }`.

#### Scenario: State change pushed

- **WHEN** a terminal's state transitions from `idle` to `running`
- **THEN** the renderer pushes `tab.set_toolbar_state` with the updated state

#### Scenario: Cwd may be omitted

- **WHEN** the terminal does not yet know its cwd
- **THEN** the push payload omits the `cwd` field and the toolbar middle slot shows "—" or similar placeholder

---

### Requirement: Terminal restart

Clicking the restart button SHALL terminate the current shell process and respawn it in the same tab, preserving the tab id and the tab's position in the sidebar. The terminal's transcript SHALL be cleared on restart.

#### Scenario: Restart preserves identity

- **WHEN** the user clicks restart on a terminal tab
- **THEN** the same tab id remains in the sidebar at the same position, the shell process is replaced, and the transcript is cleared

---

### Requirement: Terminal chrome is fixed dark

A terminal tab SHALL NOT inject the chrome theme sampler. Its chrome SHALL always be the dark fallback.

#### Scenario: No sampler injection

- **WHEN** a terminal tab is constructed
- **THEN** the renderer does NOT inject `tab.set_chrome_theme` machinery and the chrome remains `#0E0E10`
