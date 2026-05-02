## ADDED Requirements

### Requirement: Multi-entry Vite build with one React tree per panel

The system SHALL build the frontend as a multi-entry Vite project where each cronymax panel (sidebar, topbar, terminal, agent, chat, graph, popover) has its own HTML entry, its own `main.tsx`, and its own React tree. Each panel SHALL continue to be loaded by C++ as a separate CEF BrowserView (Shape A — no SPA collapse).

#### Scenario: Each panel has its own entry

- **WHEN** Vite builds the frontend
- **THEN** `web/dist/` contains one directory per panel (`sidebar/`, `topbar/`, `terminal/`, `agent/`, `chat/`, `graph/`, `popover/`), each with its own `index.html` and chunked JS

#### Scenario: Shared code is chunked, not duplicated

- **WHEN** two panels both import `src/shared/bridge.ts` and `src/shared/hooks/useBridgeEvent.ts`
- **THEN** Vite emits the shared module to a single chunk loaded by both panels' HTML

#### Scenario: A panel can be migrated independently

- **WHEN** the sidebar panel has been migrated to React but the agent panel has not
- **THEN** the build still succeeds, the bundle still launches, the sidebar panel renders via React, and the agent panel renders from its existing vanilla `agent/index.html`

---

### Requirement: React-hooks-only state management

Each panel SHALL manage local state with React's built-in primitives (`useState`, `useReducer`, `useContext`, `useEffect`). The frontend SHALL NOT depend on Redux, Zustand, Jotai, MobX, Recoil, or any other third-party state management library.

#### Scenario: Panel store uses useReducer

- **WHEN** a panel needs to manage non-trivial state (multiple fields, derived values, or actions)
- **THEN** the panel defines a single root reducer and exposes its `[state, dispatch]` pair via Context

#### Scenario: Cross-panel coordination uses bridge events

- **WHEN** an action in one panel must be reflected in another panel
- **THEN** C++ remains the single source of truth, and each panel subscribes to the relevant bridge event via `useBridgeEvent`

---

### Requirement: Tailwind v4 with shared theme tokens

The frontend SHALL use Tailwind CSS v4 with cronymax design tokens (colors, spacing, radii, shadows, typography) declared in a single shared `@theme` block. All panels SHALL consume the same tokens. Panel-specific CSS files duplicating tokens SHALL NOT exist.

#### Scenario: Single source of design tokens

- **WHEN** a developer needs to change the cronymax accent color
- **THEN** they edit one variable in `web/src/shared/design/theme.css` and all panels reflect the change after rebuild

#### Scenario: All panels import the shared theme

- **WHEN** any panel's `main.tsx` is the entry being built
- **THEN** the resulting CSS includes the shared `@theme` tokens before any panel-specific styles

---

### Requirement: Dev-mode hot module replacement via env var

When the `CRONYMAX_DEV` environment variable is set, the application SHALL load each panel from the Vite dev server at `http://localhost:5173/<panel>/` instead of from the bundled `file://…/Resources/web/<panel>/index.html`. When `CRONYMAX_DEV` is not set, the application SHALL load panels from the bundled `file://` paths offline.

#### Scenario: Dev launch hits Vite dev server

- **WHEN** `pnpm dev` is running and the application is launched with `CRONYMAX_DEV=1`
- **THEN** each panel BrowserView loads from `http://localhost:5173/<panel>/` and code edits trigger HMR without an app rebuild

#### Scenario: Production launch loads from bundle

- **WHEN** the application is launched without `CRONYMAX_DEV` set
- **THEN** each panel BrowserView loads from `file://…/cronymax.app/Contents/Resources/web/<panel>/index.html` and the app functions with no network connection

---

### Requirement: pnpm as the package manager

The frontend SHALL use pnpm (version 8 or later) as its package manager. The project SHALL ship a `pnpm-lock.yaml` and SHALL NOT ship `package-lock.json` or `yarn.lock`.

#### Scenario: Lockfile is committed

- **WHEN** the repository is cloned fresh
- **THEN** `web/pnpm-lock.yaml` exists and `pnpm install --frozen-lockfile` reproduces the documented dependency tree

---

### Requirement: TypeScript strict mode

The frontend SHALL use TypeScript with `"strict": true` in `tsconfig.json`. All panel code, shared modules, and config files (where TS is supported) SHALL be `.ts` or `.tsx`. Vanilla `.js` files SHALL NOT be added to `web/src/`.

#### Scenario: Strict mode catches missing fields

- **WHEN** a developer writes `bridge.send("terminal.new")` and forgets a required payload field
- **THEN** TypeScript reports a type error at build time

---

### Requirement: CMake `cronymax_web` build target

The CMake build SHALL include a `cronymax_web` custom target that runs `pnpm install --frozen-lockfile` followed by `pnpm build` in `web/`. The target SHALL be gated by a CMake option `CRONYMAX_BUILD_WEB` (default `ON`). When `CRONYMAX_BUILD_WEB=OFF`, the C++ build SHALL skip the pnpm step and the POST_BUILD copy SHALL still copy whatever `web/dist/` contains.

#### Scenario: Default build includes web

- **WHEN** the user runs `cmake --build build --target cronymax_app` with default options
- **THEN** `pnpm build` runs first and `web/dist/` is copied into the bundle

#### Scenario: C++-only iteration skips web build

- **WHEN** the user configures with `-DCRONYMAX_BUILD_WEB=OFF` and rebuilds `cronymax_app`
- **THEN** the pnpm step is skipped and the existing `web/dist/` (if any) is copied unchanged

---

### Requirement: POST_BUILD copies `web/dist/` to bundle resources

The CMake POST_BUILD step on the `cronymax_app` target SHALL copy `web/dist/` (the Vite build output) into `cronymax.app/Contents/Resources/web/`. The legacy behavior of copying raw `web/` source files SHALL be removed.

#### Scenario: Bundle contains built assets

- **WHEN** the build completes
- **THEN** `cronymax.app/Contents/Resources/web/sidebar/index.html` (and equivalent for each migrated panel) exists and references the hashed JS/CSS chunks emitted by Vite

---

### Requirement: Build artifacts excluded from source control

The repository SHALL exclude `web/node_modules/`, `web/dist/`, and `web/.vite/` from source control via `.gitignore`.

#### Scenario: Fresh clone is clean

- **WHEN** the repository is cloned and `git status` is inspected after `pnpm install && pnpm build`
- **THEN** none of `web/node_modules/`, `web/dist/`, or `web/.vite/` appears as untracked or modified
