## 0. Toolchain Spike

- [x] 0.1 Add `web/package.json` with `react`, `react-dom`, `zod` as deps and `vite`, `@vitejs/plugin-react`, `typescript`, `tailwindcss@4`, `@types/react`, `@types/react-dom`, `eslint`, `@typescript-eslint/parser`, `@typescript-eslint/eslint-plugin` as devDeps
- [x] 0.2 Add `web/tsconfig.json` (strict mode, bundler resolution, JSX react-jsx, target ES2022) and `web/tsconfig.node.json` for `vite.config.ts`
- [x] 0.3 Add `web/vite.config.ts` with `@vitejs/plugin-react` and `build.rollupOptions.input` listing all panel entries (initially just a `playground` entry)
- [x] 0.4 Add `web/src/shared/design/theme.css` with `@import "tailwindcss"` and an `@theme` block containing initial cronymax tokens (bg, fg, accent, popover radius, popover shadow)
- [x] 0.5 Add `web/src/panels/playground/{index.html,main.tsx}` rendering a `<h1>` plus a `useBridgeQuery("space.list")` smoke check
- [x] 0.6 Update `.gitignore` to exclude `web/node_modules/`, `web/dist/`, `web/.vite/`
- [x] 0.7 Add CMake option `CRONYMAX_BUILD_WEB` (default `ON`) in `CMakeLists.txt`
- [x] 0.8 Add `cronymax_web` custom target in `cmake/CronymaxApp.cmake` that runs `pnpm install --frozen-lockfile` and `pnpm --filter . build` in `web/`, gated by `CRONYMAX_BUILD_WEB`
- [x] 0.9 Modify the POST_BUILD step in `cmake/CronymaxApp.cmake` to copy `web/dist/` instead of `web/` into `cronymax.app/Contents/Resources/web/`
- [x] 0.10 Add `ResourceUrl()` dev-mode branch in `src/app/main_window.cc` reading `getenv("CRONYMAX_DEV")` and returning `http://localhost:5173/<relative>` when set
- [ ] 0.11 Verify: `cd web && pnpm dev` then `CRONYMAX_DEV=1 ./build/cronymax.app/Contents/MacOS/cronymax` shows the playground panel via HMR
- [x] 0.12 Verify: `cmake --build build --target cronymax_app` produces a bundle with the playground panel loadable offline from `file://`
- [x] 0.13 If CEF blocks `http://localhost:5173/` due to web-security, document the dev-only flag (`--disable-web-security` for the dev launch) in README

## 1. Shared Layer

- [x] 1.1 Add `web/src/shared/bridge.ts` exposing `bridge.send<C>(channel, payload?)` and `bridge.on<C>(channel, handler)` backed by `window.cefQuery` and `window.__aiDesktopDispatch`
- [x] 1.2 Add `web/src/shared/bridge_channels.ts` with the discriminated-union channel registry (start with the channels touched by the first panel migration; expand per panel)
- [x] 1.3 Add Zod schemas in `web/src/shared/types/` mirroring the C++ payload shapes used by initial channels (`SpaceSchema`, `TerminalRowSchema`, `BrowserTabSchema`, etc.)
- [x] 1.4 Add `web/src/shared/hooks/useBridgeEvent.ts` — typed subscription hook with auto-cleanup and inbound-payload Zod validation
- [x] 1.5 Add `web/src/shared/hooks/useBridgeQuery.ts` — promise wrapper around `bridge.send` returning `{data, error, loading, send}`, with outbound and inbound Zod validation
- [x] 1.6 Add `web/src/shared/hooks/usePanelStore.ts` — small helper that wires `useReducer` + Context with a typed dispatch
- [x] 1.7 Add `web/src/shared/components/` primitive set: `<Button>`, `<IconButton>`, `<Pill>`, `<ScrollArea>`, `<Surface>` — all Tailwind-styled
- [x] 1.8 Add `web/src/shared/components/ErrorBoundary.tsx` with a dev-mode overlay showing Zod validation errors clearly
- [x] 1.9 Document the bridge/store patterns inline (JSDoc on the public APIs)

## 2. Panel Migration: popover

- [x] 2.1 Add channel entries for `shell.popover_refresh`, `shell.popover_open_as_tab`, `shell.popover_close`, `popover.url_changed` to `bridge_channels.ts`
- [x] 2.2 Add `web/src/panels/popover/{index.html,main.tsx,App.tsx,store.ts,components/}`
- [x] 2.3 Port URL pill (🔒/🌐 + monospace url) and three SVG action buttons (refresh, open-tab, close) to React components
- [x] 2.4 Wire `useBridgeEvent("popover.url_changed", …)` to update displayed URL
- [x] 2.5 Delete `web/shell/popover_chrome.html` (vanilla version)
- [x] 2.6 Update `MainWindow::OpenPopover()` URL to load `popover/index.html` from the new path
- [ ] 2.7 Smoke test: open a tab → inspect popover from agent → confirm refresh/close/open-as-tab all dispatch correctly

## 3. Panel Migration: topbar

- [x] 3.1 Add channels touched by topbar to `bridge_channels.ts`
- [x] 3.2 Add `web/src/panels/topbar/{index.html,main.tsx,App.tsx,store.ts,components/}`
- [x] 3.3 Port URL bar, navigation buttons, panel-switcher to React components
- [x] 3.4 Delete `web/shell/topbar.{html,js,css}`
- [ ] 3.5 Smoke test: switch panels, navigate URLs, confirm tab state reflects in topbar

## 4. Panel Migration: sidebar

- [x] 4.1 Add channels touched by sidebar (`terminal.list`, `terminal.new`, `terminal.switch`, `terminal.close`, `terminal.created`, `terminal.removed`, `terminal.switched`, `space.list`, `space.switch`, `space.create`, `shell.show_panel`) to `bridge_channels.ts`
- [x] 4.2 Add `web/src/panels/sidebar/{index.html,main.tsx,App.tsx,store.ts,components/}`
- [x] 4.3 Port spaces dock, tab list (pinned + session sections), terminals dock, panel-switcher dock to React components
- [x] 4.4 Implement optimistic-update pattern for `terminal.new` (dispatch optimistic placeholder → commit on success → revert on failure)
- [x] 4.5 Wire `useBridgeEvent` for all relevant broadcast channels with deduping by id
- [x] 4.6 Delete `web/shell/sidebar.{html,js,css}`
- [ ] 4.7 Smoke test: New Terminal click adds row immediately; switching/closing/creating spaces all consistent

## 5. Panel Migration: terminal

- [x] 5.1 Add channels touched by terminal (`terminal.start`, `terminal.stop`, `terminal.input`, `terminal.output`, `terminal.exit`, `terminal.blocks_load`, `terminal.restart_requested`, `agent.task_from_command`) to `bridge_channels.ts`
- [x] 5.2 Decide per-channel fast-path: list `terminal.output` as a non-Zod-validated channel (raw passthrough) and document why
- [x] 5.3 Add `web/src/panels/terminal/{index.html,main.tsx,App.tsx,store.ts,components/}`
- [x] 5.4 Port pane lifecycle (`ensurePane`, `startTerminal`) and per-submit block UI (`.cmd-block.running` → `.ok|.fail`) to React components
- [x] 5.5 Port action bars (✨ Explain / 🔧 Fix / ↻ Retry / Copy cmd / Copy out) to React components
- [x] 5.6 Port OSC 133 parsing into a pure reducer that takes terminal output chunks and emits block updates
- [x] 5.7 Delete `web/terminal/{index.html,terminal.js,terminal.css}`
- [ ] 5.8 Smoke test: run `ls`, `false`, `cd /tmp`, `vim` (alt-screen no-op), confirm blocks render with correct status and actions

## 6. Panel Migration: chat

- [x] 6.1 Add channels touched by chat to `bridge_channels.ts`
- [x] 6.2 Add `web/src/panels/chat/{index.html,main.tsx,App.tsx,store.ts,components/}`
- [x] 6.3 Port chat surface to React components
- [x] 6.4 Delete `web/chat/`
- [ ] 6.5 Smoke test

## 7. Panel Migration: agent

- [x] 7.1 Add channels touched by agent (`agent.task`, `agent.llm.stream`, `tool.exec`, `permission.request`, `permission.respond`, `llm.config.get`, `llm.config.set`) to `bridge_channels.ts`
- [~] 7.2 Decide fast-path for `agent.llm.stream` (likely raw passthrough) — N/A: legacy `llm.js` streams via direct `fetch()` to the OpenAI-compatible endpoint; no bridge channel involved
- [x] 7.3 Add `web/src/panels/agent/{index.html,main.tsx,App.tsx,store.ts,components/}`
- [~] 7.4 Port `loop.js`, `llm.js`, `tools.js` behavior into typed React reducers + components — kept as ESM-loaded globals (typed via `web/src/shared/agent_runtime.d.ts`); UI shell migrated to React. A future change can fully port the runtime
- [x] 7.5 Delete `web/agent/{index.html,agent.js,agent.css}` (graph files stay until Phase 8; runtime engines `loop.js`/`llm.js`/`tools.js` retained per 7.4)
- [ ] 7.6 Smoke test: full agent loop with at least one tool call and one permission prompt

## 8. Panel Migration: graph

- [~] 8.1 Add channels touched by graph to `bridge_channels.ts` — N/A: graph editor persists flows via `localStorage` only; no bridge channels involved
- [x] 8.2 Add `web/src/panels/graph/{index.html,main.tsx,App.tsx,store.ts,components/}` (entry HTML kept at `web/agent/graph.html` to preserve the C++ ResourceUrl)
- [x] 8.3 Port graph rendering — keep current SVG approach; flag whether to adopt a viz lib later in a follow-up
- [x] 8.4 Delete `web/agent/{graph.html,graph.js,graph.css}` (graph.html replaced by minimal React entry; graph.js/graph.css removed)
- [ ] 8.5 Smoke test: open agent graph, edit nodes, save

## 9. Cleanup

- [x] 9.1 Confirm `web/` no longer contains any non-`src/`, non-config files outside `dist/` and `node_modules/` (panel HTML entries `web/<panel>/index.html` and `web/agent/{index,graph}.html` retained as Vite multi-entry inputs; legacy `web/shared/`, `web/agent/{llm,tools,loop}.js`, and per-panel JS/CSS removed; runtime engines moved to `web/src/shared/agent_runtime/`)
- [x] 9.2 Remove the playground entry from `vite.config.ts` and `web/src/panels/playground/`
- [x] 9.3 Confirm POST_BUILD copies only `web/dist/`
- [x] 9.4 Update README with `pnpm` setup steps, `CRONYMAX_DEV` dev workflow, and the `-DCRONYMAX_BUILD_WEB=OFF` escape hatch
- [x] 9.5 Add `pnpm lint` and `pnpm typecheck` as part of `cronymax_web` (or as a separate `cronymax_web_check` target) so CI catches regressions
- [ ] 9.6 Final manual end-to-end test: create a Space, open tabs, run terminal commands, run an agent task with a tool call and a permission prompt
