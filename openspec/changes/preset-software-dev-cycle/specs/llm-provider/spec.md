## MODIFIED Requirements

### Requirement: LLM provider configuration

The system SHALL maintain a named provider registry mapping provider IDs to provider configurations. Each provider entry SHALL declare `kind` (`openai-compat` | `github-copilot` | `none`), and kind-specific fields: `base_url` and `api_key` for `openai-compat`; no extra fields for `github-copilot` (token managed via OAuth device flow); `base_url` only for `none`. A `default_provider` SHALL be configured and used when an agent's `llm.provider` is omitted. API keys and OAuth tokens SHALL be stored in the OS keychain keyed by provider ID; the registry file at `~/.cronymax/providers.json` SHALL store only non-secret metadata. The registry SHALL be persisted and restored across app restarts.

#### Scenario: Named provider used for agent LLM call

- **WHEN** an agent is configured with `llm: { provider: openai, model: gpt-4o-mini }` and a provider named `openai` exists in the registry
- **THEN** the LLM call uses that provider's `base_url` and retrieves its `api_key` from the OS keychain

#### Scenario: Default provider used when agent omits provider

- **WHEN** an agent YAML uses the legacy flat `llm: gpt-4o` form or omits `provider`
- **THEN** the LLM call uses the registry's `default_provider` entry with the specified model

#### Scenario: Configuration survives restart

- **WHEN** the application restarts after provider registry has been saved
- **THEN** all configured providers (excluding keychain secrets) are restored from `~/.cronymax/providers.json` and LLM calls resume without re-authentication

---

## ADDED Requirements

### Requirement: GitHub Copilot OAuth provider

The system SHALL support a provider of kind `github-copilot`. Authenticating this provider SHALL use the GitHub device-flow OAuth protocol: the system displays a device code and verification URL to the user, polls for GitHub token grant, then exchanges the GitHub token for a Copilot API token at the Copilot token-exchange endpoint. The resulting Copilot token SHALL be stored in the OS keychain and refreshed automatically when expired.

#### Scenario: User authenticates Copilot provider

- **WHEN** the user initiates authentication for a `github-copilot` provider in the settings UI
- **THEN** the system starts the GitHub device flow, presents the user code and URL, and polls for grant completion

#### Scenario: Device flow completes and token is stored

- **WHEN** the user completes the GitHub device flow authorisation in their browser
- **THEN** the system receives the GitHub access token, exchanges it for a Copilot API token, stores the Copilot token in the OS keychain, and the provider status transitions to `authenticated`

#### Scenario: Expired token is refreshed transparently

- **WHEN** an LLM call is made with a `github-copilot` provider and the stored token has expired
- **THEN** the system re-runs the token exchange (using the stored GitHub token) to obtain a fresh Copilot token before retrying the LLM call, without prompting the user

---

### Requirement: API key provider management

The system SHALL allow users to add, update, and remove `openai-compat` providers via the settings UI. API keys SHALL be stored exclusively in the OS keychain (macOS Security framework; Windows Credential Store on Windows). The settings UI SHALL display provider status (`configured` | `unconfigured`) but SHALL NOT display the key value.

#### Scenario: User adds OpenAI provider

- **WHEN** the user enters a provider name, base URL (`https://api.openai.com/v1`), and API key in the settings UI and saves
- **THEN** the provider is added to the registry with status `configured` and the API key is stored in the OS keychain under `cronymax-provider-<name>`

#### Scenario: API key never written to disk in plaintext

- **WHEN** a provider with an API key is saved
- **THEN** `~/.cronymax/providers.json` contains no `api_key` field; the key is retrievable only via the OS keychain

---

### Requirement: Per-provider Ollama / local model support

The system SHALL support `none` auth kind for providers with a local base URL and no authentication. A provider of kind `none` SHALL be usable without any keychain entry or OAuth flow.

#### Scenario: Ollama provider works without authentication

- **WHEN** a provider is configured with `kind: none` and `base_url: http://localhost:11434/v1`
- **THEN** LLM calls to that provider succeed without any API key header or OAuth token
