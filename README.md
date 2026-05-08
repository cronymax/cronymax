# cronymax

This repository is a CEF-first prototype for a macOS AI desktop shell. The
current implementation focuses on the thin vertical slice needed for a one-week
demo:

- CEF Views shell skeleton with sidebar, browser area, terminal drawer, and
  agent panel.
- Native macOS runtime for PTY sessions, local file access, command risk
  classification, permission checks, and Seatbelt profile generation.
- React + TypeScript + Tailwind v4 frontend (multi-entry Vite build, one HTML
  per CEF panel) with a typed Zod-validated bridge to the C++ host.
- **Flows**: a small graph of typed Agents that produce typed Documents and
  review each other's work. Hand-edit YAML under `.cronymax/{agents,doc_types,flows}/`
  and start a Run from the Flows sidebar — see
  [docs/flows_quickstart.md](docs/flows_quickstart.md). The
  Slack-style channel view, dagre-laid-out Flow editor, and unified inbox
  are documented in [docs/orchestration_ui.md](docs/orchestration_ui.md).
- **Document workbench**: a dedicated panel for authoring, reviewing, and
  diffing the Markdown documents a flow produces — Milkdown-based WYSIWYG
  mode, Monaco source mode, side-by-side / inline diff view, a block-anchored
  comment rail, and one-click suggested-edit acceptance that writes a new
  revision. See [docs/document_workbench.md](docs/document_workbench.md).
- A `native_probe` CLI `<comment>`that can validate the native runtime `</comment>` without CEF.

## Getting Started

### Prerequisites

| Tool      | Version            | Notes                                                                |
| --------- | ------------------ | -------------------------------------------------------------------- |
| macOS     | 13+ (arm64 tested) | Linux/Windows not yet wired up                                       |
| Xcode CLT | latest             | `xcode-select --install`                                             |
| CMake     | 3.21+              | `brew install cmake`                                                 |
| Ninja     | optional           | `brew install ninja` (faster builds)                                 |
| bun       | 1.1+               | `brew install bun` — required when `CRONYMAX_BUILD_WEB=ON` (default) |

### Clone

```sh
git clone --recurse-submodules <repo-url> cronymax
cd cronymax
# if you forgot --recurse-submodules:
git submodule update --init
```

### Install Frontend Dependencies

The CMake build will run `bun install --frozen-lockfile` automatically as part
of the `cronymax_web` target, but you can prime the cache up-front:

```sh
bun install
```

### Configure & Build the Full App

CEF is downloaded automatically on first configure (cached in `.cef-cache/`,
gitignored). No manual download or env vars required.

```sh
cmake -S . -B build \
  -DCRONYMAX_BUILD_APP=ON \
  -DCRONYMAX_BUILD_WEB=ON \
  -DCMAKE_BUILD_TYPE=Debug
cmake --build build --target cronymax_app -j8
```

The `cronymax_app` target depends on `cronymax_web`, which runs
`bun install --frozen-lockfile && bun run build` in `web/` and produces
`web/dist/<panel>/index.html` for every React panel. A separate
`cronymax_web_sync` target re-copies `web/dist/` into the bundle on every
build, so frontend-only edits don't require a C++ relink:

```sh
cmake --build build --target cronymax_web_sync -j8
```

### Run

```sh
open build/cronymax.app
# or
./build/cronymax.app/Contents/MacOS/cronymax
# Add `--use-mock-keychain` to avoid keychain password prompt on mac
./build/cronymax.app/Contents/MacOS/cronymax --use-mock-keychain
```

### Build Native Runtime Only (no CEF)

For native-only iteration, skip the CEF shell:

```sh
cmake -S . -B build -DCRONYMAX_BUILD_APP=OFF
cmake --build build
```

Useful probes:

```sh
./build/native_probe policy .
./build/native_probe agent . "/read README.md"
./build/native_probe agent . "/write scratch/demo.txt hello"
./build/native_probe agent . "/exec pwd"
```

The sandboxed exec path uses `/usr/bin/sandbox-exec`, which is suitable for a
prototype but should be revisited before production hardening.

## Frontend Development Workflow

The `web/` directory is a bun workspace package. Common commands (run from
`web/`):

```sh
bun dev          # start Vite dev server on http://localhost:5173
bun run build    # tsc -b && vite build → web/dist/
bun run typecheck  # tsc -b --noEmit
bun run lint     # eslint src/
bun run preview  # serve web/dist/ for sanity checks
```

### Hot Reload Inside CEF

To point the CEF shell at the Vite dev server (instead of the bundled
`web/dist/`), set `CRONYMAX_DEV=1` before launching:

```sh
cd web && bun dev &            # leaves Vite running on :5173
cd ..
CRONYMAX_DEV=1 ./build/cronymax.app/Contents/MacOS/cronymax
```

When `CRONYMAX_DEV` is set, `main_window.cc` rewrites every `ResourceUrl()`
call to `http://localhost:5173/<relative_path>`, so React Fast Refresh applies
across all panels without a full bundle rebuild.

### Skip the Frontend Build

If bun is unavailable or you want to iterate on C++ without rebuilding the
React bundle, pass `-DCRONYMAX_BUILD_WEB=OFF`:

```sh
cmake -S . -B build \
  -DCRONYMAX_BUILD_APP=ON \
  -DCRONYMAX_BUILD_WEB=OFF
cmake --build build --target cronymax_app -j8
```

The bundle will still include any previously-built `web/dist/` (and the
remaining vanilla legacy panels in `web/{shell,terminal,agent,chat,shared}/`).

## VS Code / clangd Setup

The repo ships a complete clangd configuration. Once you have a configured
build, all C++ navigation, completions, hover, and as-you-type diagnostics work
out of the box.

**Required extensions**

| Extension                               | Purpose                                         |
| --------------------------------------- | ----------------------------------------------- |
| `llvm-vs-code-extensions.vscode-clangd` | C++ LSP (IntelliSense, go-to-def, tidy)         |
| `ms-vscode.cpptools`                    | Debugger only — IntelliSense engine is disabled |

**One-time setup**

After cloning and installing the extensions, run a CMake configure once to
generate `build/compile_commands.json`:

```sh
cmake -S . -B build \
  -DCRONYMAX_BUILD_APP=ON \
  -DCMAKE_BUILD_TYPE=Debug
```

`CMAKE_EXPORT_COMPILE_COMMANDS` is already enabled in `CMakeLists.txt`.
The configure covers all targets — native modules **and** the CEF app layer —
so every header resolves correctly in the editor.

Then reload the VS Code window (`Cmd+Shift+P` → **Developer: Reload Window**).
The clangd status bar item should show indexing progress and then go idle.

**What the config does**

| File                      | Purpose                                                                                                                          |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `.clangd`                 | Points clangd at `build/compile_commands.json`; enables background indexing, inlay hints, and clang-tidy                         |
| `.clang-tidy`             | Root tidy profile for native modules (`app/common/`, `app/sandbox/`, `app/workspace/`, `app/terminal/`, `app/agent/`)            |
| `app/browser/.clang-tidy` | Conservative CEF-safe tidy profile (`app/browser/`) — excludes checks that conflict with `CefRefPtr` and `IMPLEMENT_REFCOUNTING` |
| `.vscode/settings.json`   | Pins `/usr/bin/clangd` (Apple 21.0), disables cpptools IntelliSense engine, sets clangd as the C++ formatter                     |

## Project Layout

```
app/                 C++ sources (flat `cronymax::` namespace)
  browser/             CEF shell, BrowserViews, bridge handler
  sandbox/, workspace/, terminal/, common/, agent/
web/                 Frontend monorepo (bun + Vite)
  src/panels/<name>/   React tree for each CEF panel
  src/shared/          Bridge, hooks, design tokens, primitives
  <panel>/index.html   Vite entry per panel (Shape A multi-entry)
  shell/, terminal/, …   Legacy vanilla panels (being migrated)
cmake/               CronymaxApp.cmake (CEF + app + web targets)
openspec/            Active spec-driven changes (see openspec/changes/)
cef/         Upstream CEF source (git submodule, chromiumembedded/cef)
.cef-cache/  Cached CEF binary archive download (gitignored)
tools/               native_probe and other CLI utilities
```

## Prototype Boundaries

Implemented now:

- Native PTY lifecycle API.
- Local workspace file broker.
- Command risk classifier.
- Permission broker.
- Seatbelt profile compiler.
- Sandboxed command launcher.
- Single prototype agent runtime with `/read`, `/write`, and `/exec`.
- Agent Graph data model and validation skeleton.
- CEF app skeleton and browser-side bridge.
- Typed React + Tailwind v4 frontend with Zod-validated bridge (topbar
  panel migrated; sidebar, terminal, agent, chat, popover still vanilla).

Next implementation slices:

- Migrate remaining panels (popover, sidebar, terminal, agent, chat, graph)
  off the legacy vanilla code per `openspec/changes/react-frontend-migration/`.
- Wire browser context extraction into `browser.getActivePage`.
- Replace terminal-lite with xterm.js once dependency vendoring is decided.
- Add command block markers for shell integration.
- Persist tabs, workspaces, terminal blocks, and agent traces.
- Add permission UI instead of CLI-only allow/deny decisions.
