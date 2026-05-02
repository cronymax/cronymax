## ADDED Requirements

### Requirement: Terminal and chat tabs are multi-instance

The system SHALL allow multiple concurrent tabs of kind `kTerminal` and multiple concurrent tabs of kind `kChat`. `TabManager::RegisterSingletonKind` SHALL NOT be called for these kinds. Each `Open(kTerminal, ...)` or `Open(kChat, ...)` call SHALL create a new, independent tab.

#### Scenario: Multiple terminals coexist

- **WHEN** the user clicks "+ Terminal" three times in the title bar
- **THEN** three terminal tabs exist with three distinct tab ids and three independent PTY/behavior instances

#### Scenario: Singleton open is rejected for multi-instance kinds

- **WHEN** the renderer sends `shell.tab_open_singleton { kind: "terminal" }` or `shell.tab_open_singleton { kind: "chat" }`
- **THEN** the dispatcher returns a failure response (the channel is reserved for kinds explicitly registered as singletons)

---

### Requirement: Auto-numbered display names

When `TabManager::Open(kind, params)` is called for a kind in the auto-numbered set (`{kTerminal, kChat}`) and `params.display_name` is empty, the manager SHALL assign `"<KindDisplayName> N"` where `N = (max numeric suffix among existing tabs of that kind) + 1`. The numeric suffix counter SHALL NOT reuse numbers vacated by closed tabs.

#### Scenario: First instance is numbered 1

- **WHEN** there are no terminal tabs and the user creates one
- **THEN** the new tab's display name is `Terminal 1`

#### Scenario: Numbers do not reuse closed slots

- **WHEN** the user creates `Terminal 1`, `Terminal 2`, `Terminal 3`, then closes `Terminal 2`, then creates another terminal
- **THEN** the new terminal is named `Terminal 4` (not `Terminal 2`)

#### Scenario: Caller-supplied name wins

- **WHEN** `TabManager::Open(kTerminal, params)` is called with `params.display_name = "Build Logs"`
- **THEN** the auto-numbering rule does not apply and the new tab's display name is `Build Logs`
