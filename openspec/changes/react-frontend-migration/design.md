## Context

The cronymax frontend is currently vanilla HTML/JS/CSS — seven CEF BrowserViews each loading a static `index.html` with hand-rolled DOM. This worked for prototyping but produced two recurring failure modes:

1. **Untyped JSON bridge**: payload shape drift between C++ producer and JS consumer goes undetected until UI breaks.
2. **Broadcast-driven local state**: panels rebuild UI from `__aiDesktopDispatch` events without owning a reducer, so a missed/reordered event leaves the view inconsistent.

We want React + TypeScript + Tailwind + Vite + pnpm to fix both classes systemically. The core question this design answers is **how to introduce a build pipeline and a typed bridge without disturbing C++ ownership of window layout, panels, or message routing**.

## Goals / Non-Goals

### Goals

- Eliminate untyped-JSON drift at the bridge boundary.
- Replace broadcast-driven view rebuilds with reducer-owned local state.
- Sub-second visual iteration via Vite HMR in dev mode.
- Single source of design tokens (Tailwind v4 `@theme`).
- Zero C++ logic changes; one C++ helper (`ResourceUrl()`) gains dev/prod branching.
- Preserve current panel isolation: each panel = one CEF BrowserView = one React tree.

### Non-Goals

- No third-party state library (Redux/Zustand/Jotai/etc.).
- No SPA collapse (Shape B). Panels remain independent CEF BrowserViews.
- No bridge channel renames or payload changes.
- No new product features.
- No migration to Chromium native UI for sidebar/topbar.
- No libghostty integration.

## Decisions

### Decision 1: Shape A (multi-entry) over Shape B (SPA)

**What**: Each existing panel keeps its own CEF BrowserView and gets its own Vite entry, its own React tree, its own root store. Vite's `build.rollupOptions.input` lists all seven entry HTML files; shared code (bridge, hooks, design tokens, primitive components) lives in `src/shared/` and Rollup naturally chunks it.

**Why**:

- C++ already owns layout via CefBoxLayout — that code is debugged and works. SPA collapse would require rewriting `MainWindow::CreateMainWindow()` and `LayoutPopover()`.
- Browser tabs are real CefBrowserViews and stay native. An SPA would have to fake them with iframes (loses CDP, popups, devtools per tab).
- Panel isolation matches the security model: a bug in `terminal/` cannot poison the `agent/` V8 context.
- Migration can be incremental: convert one panel at a time, others stay vanilla until reached.

**Alternatives considered**:

- **Shape B (single SPA)**: rejected — too much C++ surgery, loses native browser tabs, no incremental migration path.
- **Shape A but with a single shared store via BroadcastChannel/SharedWorker**: rejected — over-engineered; C++ is already the source of truth and bridge events already do cross-panel notification.

### Decision 2: React hooks only — no state library

**What**: Each panel defines a single root reducer and exposes it via Context. Components consume slices via `useContext(PanelStore)`. No Zustand, Redux, Jotai, MobX, Recoil, `useSyncExternalStore`-based shared store, or signals library.

**Why**:

- Per-panel state is small (sidebar ≈ 50 fields, terminal ≈ 30 + per-block records, agent ≈ 40). `useReducer` handles this comfortably.
- A library would mostly add devtools we already get from React DevTools.
- Cross-panel coordination is C++'s job (single source of truth in `SpaceManager`/`BrowserManager`); panels subscribe to bridge events for the slices they care about.
- One fewer dependency, one fewer mental model, one fewer thing to teach a future contributor.

**Patterns**:

- `useBridgeEvent(channel, handler)` — auto-subscribes, auto-unsubscribes on unmount, types `handler` payload from the channel registry.
- `useBridgeQuery(channel, payload?)` — promise wrapper around `bridge.send`, returns `{data, error, loading, send}`.
- Optimistic updates: dispatch optimistic action with temp id; on bridge resolve, dispatch `commit(tempId, realId)`; on reject, dispatch `revert(tempId)`. Pure reducer, no special primitive needed.

**Alternatives considered**:

- **Zustand**: smaller than Redux, but still adds a dep and a second mental model alongside hooks.
- **`useSyncExternalStore` shared store**: appropriate when multiple components in the same tree need to subscribe to an external source — but our external source is the bridge, and `useBridgeEvent` covers that case directly.

### Decision 3: Typed bridge with Zod runtime validation

**What**: `src/shared/bridge.ts` defines a single channel registry as a discriminated union:

```ts
// One entry per channel — request and response Zod schemas
const Channels = {
  "terminal.new": { req: z.object({}), res: TerminalRowSchema },
  "terminal.list": { req: z.object({}), res: z.array(TerminalRowSchema) },
  "terminal.input": {
    req: z.object({ id: z.string(), data: z.string() }),
    res: z.void(),
  },
  "space.create": {
    req: z.object({ name: z.string(), root_path: z.string() }),
    res: SpaceSchema,
  },
  // …all ~25 channels
} as const;
```

`bridge.send(channel, payload)` validates `payload` against `Channels[channel].req` before serializing, and validates the JSON response against `Channels[channel].res` before resolving. `useBridgeEvent(channel, handler)` validates inbound event payloads before invoking the handler.

**Why**:

- TS types alone catch dev-time mismatches; Zod catches runtime drift between C++ producer and JS consumer (the actual failure mode that bit us).
- A single source of truth for "what does this channel look like?" — comments on each entry double as docs.
- Validation failures are explicit (a Zod error thrown into the panel's error boundary) rather than silent (`undefined.id`).
- C++ can later be regenerated from this registry if we want; for now it's the canonical reference.

**Cost**: Zod is ~13 KB gzipped, shared across all panels via Vite chunking. Runtime cost per call is negligible (~µs for our payload sizes).

**Alternatives considered**:

- **TS types only**: cheaper but loses runtime validation — exactly the gap that produced the `terminal.new` bug.
- **Protobuf / FlatBuffers**: overkill for ~25 channels; adds codegen step.
- **JSON Schema + ajv**: equivalent to Zod but with worse DX (schemas as JSON literals).

### Decision 4: Tailwind v4 with `@theme` (no JS config file)

**What**: Tailwind v4 (released 2024) supports configuration in CSS via the `@theme` directive. We use one `src/shared/design/theme.css`:

```css
@import "tailwindcss";

@theme {
  --color-cronymax: #0d0e10;
  --color-cronymax-fg: #e8e8ea;
  --color-cronymax-accent: #7c5cff;
  --radius-popover: 12px;
  --shadow-popover: 0 -10px 28px rgba(0, 0, 0, 0.35);
  /* …Arc-style palette */
}
```

Each panel imports `theme.css` once. All Tailwind classes consume the tokens.

**Why**:

- One file owns the design system. Today's drift across `sidebar.css`/`topbar.css`/etc. goes away.
- Co-located with CSS — no JS config file to keep in sync.
- v4's JIT is fast enough that we don't need a watcher in dev (Vite handles it).

**Alternatives considered**:

- **CSS Modules**: ergonomic but doesn't solve token sharing.
- **vanilla-extract / panda-css**: more typed, but heavier toolchain.
- **Tailwind v3 with `tailwind.config.js`**: works but we'd be adopting an obsolete config style.

### Decision 5: Dev/prod URL switching via env var, not CMake

**What**: `MainWindow::ResourceUrl(relative)` checks `getenv("CRONYMAX_DEV")`. If set, returns `http://localhost:5173/<panel>/<relative>`; otherwise returns the existing `file://…/Resources/web/<relative>` path.

**Usage**:

```bash
# Dev: terminal A
cd web && pnpm dev          # Vite dev server on :5173
# Dev: terminal B
CRONYMAX_DEV=1 ./build/cronymax.app/Contents/MacOS/cronymax
```

HMR works because each panel BrowserView's WebSocket back to Vite survives panel-show/hide.

**Why**:

- No build needed for JS edits in dev — Vite dev server serves on demand.
- No CMake reconfigure to toggle modes.
- Production build path is unchanged: bundled `file://` URLs work offline.

**Alternatives considered**:

- **CMake-time toggle**: requires reconfigure for mode switch — too coarse.
- **Always run Vite dev server in background**: leaks a process; surprising in prod-like testing.

### Decision 6: pnpm with single root, no workspaces yet

**What**: `web/package.json` is a single package. No `pnpm-workspace.yaml`, no per-panel sub-packages.

**Why**:

- Seven panels share enough code (bridge, hooks, design, primitives) that splitting them creates more import-path noise than it saves.
- We can always promote `src/shared/` to a workspace package later if a panel needs to be independently versioned.

**Alternatives considered**:

- **pnpm workspace per panel**: useful only if panels diverge in deps or get versioned independently. Premature.

### Decision 7: Migration order — sidebar first, agent last

**Order**:

1. `popover/` (smallest, isolated, recently rewritten — low risk)
2. `topbar/` (small, mostly cosmetic)
3. `sidebar/` (medium, exercises terminal-create flow that broke recently — proves the typed bridge value)
4. `terminal/` (largest interactive surface; block UI is the most rewarding to type)
5. `chat/` (medium)
6. `agent/` (largest with most state)
7. `graph/` (visualization-heavy; may benefit from a viz lib later)

**Why this order**:

- Start with the smallest/safest panel to shake out the toolchain (Vite config, dev URL switching, CMake POST_BUILD wiring) before committing to the bigger ones.
- `sidebar` third because it's the panel where the typed bridge directly addresses a felt bug.
- `agent` and `graph` last because they're the largest and most likely to surface React-pattern questions that benefit from earlier panels' lessons.

**Each panel migration is one PR** that includes: vanilla files removed, React panel added, snapshot test of broadcast-event handling, manual smoke test. The mixed state (some panels React, some vanilla) is supported throughout — Vite's multi-input build emits only the entries that exist.

## Risks / Trade-offs

| Risk                                                                                 | Mitigation                                                                                                                                         |
| ------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| Vite HMR over `http://localhost:5173/` from CEF may have CSP or mixed-content issues | Spike during Phase 0; fall back to disabling web-security in dev mode (CEF flag `--disable-web-security`) if needed                                |
| `pnpm install` in CI doubles cold build time                                         | Cache `web/node_modules/` between CI runs; gate via `CRONYMAX_BUILD_WEB=OFF` for C++-only PRs                                                      |
| Bundle size grows from ~5 KB raw JS per panel to ~150-200 KB gzipped                 | Acceptable for a desktop app loading from `file://`; no network cost                                                                               |
| Broadcast events arrive before React commits, causing dropped updates                | `useBridgeEvent` queues events received during initial render and flushes after first commit                                                       |
| Panel iframe-style isolation means we can't share React state cheaply                | Intentional — cross-panel sync stays in C++ as today                                                                                               |
| Tailwind v4 is newer; tooling ecosystem still catching up                            | Keep an escape hatch to v3 documented in design.md if blockers emerge                                                                              |
| Mixed JS/TS state during rollout                                                     | Per-panel migration is atomic; the registry only adds an entry when its panel migrates                                                             |
| Zod adds runtime cost on hot paths (e.g., terminal output stream)                    | Streaming output channels (`terminal.output`) use a hand-written parser path, not full Zod, when payload size justifies it; documented per-channel |

## Migration Plan

**Phase 0 — Toolchain spike (no panel migration yet)**

- Stand up `web/package.json`, `vite.config.ts`, `tsconfig.json`, `tailwind.config` (or `@theme` block), and a single throwaway `playground/` entry that mounts `<h1>hello</h1>`.
- Wire CMake `cronymax_web` target + POST_BUILD copy of `dist/`.
- Implement `CRONYMAX_DEV` switching in `ResourceUrl()`.
- Verify dev-mode HMR works inside a CEF BrowserView.
- Verify prod build loads from bundled `file://`.

**Phase 1 — Shared layer**

- Build `src/shared/bridge.ts` with the channel registry (start with channels touched by the first panel to migrate).
- Build `useBridgeEvent`, `useBridgeQuery`, `usePanelStore`.
- Build `src/shared/design/` with Tailwind theme + a small primitive set (`<Button>`, `<Pill>`, `<IconButton>`, `<ScrollArea>`).

**Phase 2-8 — Panel migrations** (one panel per phase, in the order from Decision 7)

For each panel:

- Add Vite entry (`src/panels/<panel>/{index.html,main.tsx,App.tsx,store.ts,components/}`).
- Port behavior from existing `web/<panel>/*.js` to React + reducer.
- Add channel entries to bridge registry as needed.
- Remove old `web/<panel>/` files in the same commit that adds the new entry.
- Manual smoke test against the running app.

**Phase 9 — Cleanup**

- Delete any unused vanilla files.
- Confirm POST_BUILD copies only `web/dist/`.
- Update README with `pnpm` setup steps and `CRONYMAX_DEV` dev workflow.
- Add a `web/CONTRIBUTING.md` (if desired) for the bridge/store patterns.

## Open Questions

- **Error boundaries**: should each panel have a single root error boundary, or per-region? Recommendation: single root + a developer overlay in dev mode. Confirm during Phase 1.
- **Chrome DevTools Protocol per panel**: do we expose `--remote-debugging-port` only in dev mode? Recommendation: yes, gated by `CRONYMAX_DEV`. Confirm in Phase 0.
- **Tailwind v4 vs v3**: any blockers on v4 in CEF Chromium 147? No known issues — v4 emits standard CSS. Verify in Phase 0 spike.
- **Streaming channels**: `terminal.output` and `agent.llm.stream` are high-frequency. Per-channel opt-out from full Zod parsing is described in Risks; the exact predicate ("payload.length > N" vs "channel listed in fast-path set") to be decided when those panels migrate.
