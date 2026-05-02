## Why

The web layer (`web/`) is ~5 KLOC of vanilla HTML/JS/CSS spread across six panels (sidebar, topbar, terminal, agent, chat, graph) plus a popover chrome. Each panel hand-rolls DOM updates, duplicates design tokens across CSS files, and talks to C++ through `window.cefQuery` returning raw JSON strings. This produced concrete pain in the prototype:

- **Untyped bridge → silent payload drift**: `terminal.new` returned `{id, name}` but the sidebar treated it as the bare row; recently fixed by hand. The boundary that ate the bug has no schema.
- **Broadcast-driven local state → race conditions**: panels reconstruct UI from broadcast events instead of owning a reducer, so one missed event leaves the view stale. The recent New-Terminal regression was exactly this class of bug.
- **No build step → no static checks, no HMR**: every change requires a full app rebuild (POST_BUILD copies `web/`) or a manual `cp` into the bundle. Iteration on Arc-style polish (popover chrome, sidebar interactions) is 10-30s per visual tweak.
- **Design tokens duplicated**: `sidebar.css` 300 LOC, `topbar.css` 139 LOC, `terminal.css` 304 LOC, `agent.css` 292 LOC — colors, spacing, radii copy-pasted with drift.

Migrating to a small modern toolchain (React + TypeScript + Tailwind + Vite + pnpm) with a typed bridge eliminates the boundary class of bugs, gives sub-second iteration via Vite dev server, and unifies the design system without rewriting C++.

## What Changes

- **NEW** `web/` becomes a pnpm + Vite project with multi-page build (one entry per panel — Shape A: panels stay isolated CEF BrowserViews, each gets its own React tree).
- **NEW** TypeScript strict mode across all frontend code; shared types module for bridge payloads.
- **NEW** Typed bridge layer: `bridge.ts` exposes `send<C>(channel, payload)` with a discriminated-union channel registry and Zod schemas validating every inbound and outbound payload at runtime.
- **NEW** React-hooks-only state management: per-panel `useReducer` + `useContext` store, plus two shared hooks (`useBridgeEvent`, `useBridgeQuery`). No third-party state libraries.
- **NEW** Tailwind v4 with a single shared `@theme` block defining cronymax design tokens (Arc-style palette, spacing, radii, shadows). All panels consume the same tokens.
- **NEW** Dev mode: a `CRONYMAX_DEV=1` env var (or `--dev` CLI flag) makes `MainWindow::ResourceUrl()` return `http://localhost:5173/<panel>/` instead of the bundled `file://` path. Vite dev server with HMR survives across panel reloads.
- **NEW** Production build: `pnpm --filter web build` emits `web/dist/<panel>/index.html` + hashed assets; CMake POST_BUILD copies `web/dist/` into `cronymax.app/Contents/Resources/web/`.
- **MODIFIED** `cmake/CronymaxApp.cmake`: POST_BUILD step copies `web/dist/` instead of `web/`. A `cronymax_web` custom target invokes `pnpm build` (gated by a CMake option `CRONYMAX_BUILD_WEB=ON` so C++-only iterations skip it).
- **MODIFIED** `src/app/main_window.cc`: `ResourceUrl()` swaps base URL based on dev/prod mode. No other C++ changes required — bridge channel names and payload shapes stay identical.
- **MODIFIED** All 7 panel entry HTMLs migrate from `<script src="…js">` to `<script type="module" src="/src/panels/<panel>/main.tsx">` per Vite convention.
- **REMOVED** Hand-rolled DOM construction in `web/shell/sidebar.js`, `web/shell/topbar.js`, `web/terminal/terminal.js`, `web/agent/{agent,loop,llm,tools,graph}.js`, `web/chat/chat.js`, `web/shell/popover_chrome.html` script blocks.

## Capabilities

### New Capabilities

- `web-frontend`: React + TypeScript + Tailwind frontend convention — multi-entry Vite build (Shape A), one React tree per CEF panel, hooks-only state, Tailwind v4 design tokens, dev/prod URL switching.
- `typed-bridge`: TypeScript bridge layer with discriminated-union channel registry, Zod runtime validation on both directions, optimistic-update pattern for `terminal.new`-class flows.

### Modified Capabilities

_(none — existing capability specs from `space-agent-integration` describe channel semantics and stay correct; this change describes how the JS side consumes them)_

## Non-goals

- **No new product features.** This is a like-for-like migration. Sidebar shows the same items, terminal renders the same blocks, agent panel runs the same loop.
- **No bridge channel renames.** `terminal.new` stays `terminal.new`; payload shapes stay byte-identical. C++ does not need to change.
- **No state library.** No Redux, Zustand, Jotai, MobX, Recoil, or `useSyncExternalStore`-based shared store. Hooks only.
- **No SPA collapse (Shape B).** Each panel keeps its own CEF BrowserView and own React tree. C++ continues to own window layout.
- **Not touching `src/`** beyond `ResourceUrl()` (one function in `main_window.cc`). No agent/sandbox/workspace changes.
- **Not addressing libghostty or native sidebar/topbar** — those are separate explorations.

## Impact

- **`web/`**: full restructure to `web/{package.json,pnpm-lock.yaml,tsconfig.json,vite.config.ts,src/{shared,panels}}`; existing `.js`/`.html`/`.css` files migrate piecemeal during the rollout.
- **`cmake/CronymaxApp.cmake`**: new `cronymax_web` custom target running `pnpm install && pnpm build`; POST_BUILD copies `web/dist/` instead of `web/`. New CMake option `CRONYMAX_BUILD_WEB` (default `ON`) to gate the pnpm step.
- **`src/app/main_window.cc`**: `ResourceUrl()` reads `CRONYMAX_DEV` env var and routes to either `http://localhost:5173/<panel>/` (dev) or the bundled `file://…/web/<panel>/index.html` (prod).
- **`.gitignore`**: add `web/node_modules/`, `web/dist/`, `web/.vite/`.
- **New tooling dependency**: `pnpm` (Node ≥ 20). Documented in README.
- **New runtime dependencies in `web/package.json`**: `react`, `react-dom`, `zod`. Dev deps: `typescript`, `vite`, `@vitejs/plugin-react`, `tailwindcss@4`, `@types/react`, `@types/react-dom`, `eslint`, `@typescript-eslint/*`.
- **Bundle size**: prod build expected ~150-200 KB gzipped per panel after React + shared chunk; acceptable for desktop-app context (loaded from `file://`, no network).
- **Build time**: cold `pnpm install` ~30s; cold `pnpm build` ~5-10s for all panels. Skippable with `-DCRONYMAX_BUILD_WEB=OFF` for C++-only edits.
- **No C++ namespace, module, or capability changes.**
