## Why

VS Code C++ development on this codebase currently suffers from three overlapping problems:

**Extension conflict.** Both `ms-vscode.cpptools` (IntelliSense engine active) and `vscode-clangd` are installed and running simultaneously. Both attempt to provide hover, completions, go-to-definition, and diagnostics on every `.cc`/`.h` file. The result is duplicate results, conflicting diagnostics, and degraded editor performance.

**No compilation database.** `CMAKE_EXPORT_COMPILE_COMMANDS` is unset in the CMake cache. Without `compile_commands.json`, clangd guesses at include paths and gets them wrong — cross-module headers (`"workspace/file_broker.h"` from `src/agent/`), SQLite3 headers, and CEF headers all fail to resolve. Navigation, completions, and diagnostics are unreliable.

**CEF include path gap.** `src/app/` sources use `#include "include/cef_*.h"` — a pattern that resolves only when `-Ithird_party/cef` is in the compile flags, which only happens when `CRONYMAX_BUILD_APP=ON`. Without a proper compilation database capturing these flags, clangd cannot resolve any CEF symbols in the app sources.

## What Changes

- **MODIFIED** `CMakeLists.txt`: add `set(CMAKE_EXPORT_COMPILE_COMMANDS ON)` so every CMake configure generates `build/compile_commands.json`.
- **NEW** `.clangd`: project-level clangd configuration pointing at `build/` for the compilation database, enabling background indexing, inlay hints, and inline clang-tidy.
- **NEW** `.clang-tidy`: root-level tidy config covering `src/common/`, `src/sandbox/`, `src/workspace/`, `src/terminal/`, `src/agent/` — modern C++20 checks, bugprone checks, performance checks. No clang-analyzer (too slow for type-time).
- **NEW** `src/app/.clang-tidy`: CEF-specific tidy config scoped to `src/app/`. Conservative — excludes checks that conflict with CEF patterns (`CefRefPtr`, `IMPLEMENT_REFCOUNTING`, `new`-based CEF object construction). Inherits nothing from root (standalone).
- **NEW** `.vscode/settings.json`: disables `ms-vscode.cpptools` IntelliSense engine, pins clangd to `/usr/bin/clangd` (Apple 21.0 — matches the compiler), sets clangd as the C++ formatter, configures useful clangd arguments.

## Capabilities

No new runtime capabilities. This is a developer tooling change only.

## Impact

- `CMakeLists.txt`: one-line addition.
- `.clangd`, `.clang-tidy`, `src/app/.clang-tidy`, `.vscode/settings.json`: new files, no build impact.
- A CMake reconfigure is required after the CMakeLists change to generate `build/compile_commands.json`. The existing cache already has `CRONYMAX_BUILD_APP=ON` and `CEF_ROOT=third_party/cef`, so reconfigure is a clean `cmake -B build .` with no extra flags.
- Developers need to reload their VS Code window once after the settings are in place.

## Non-goals

- Installing or managing the clangd extension itself (already installed).
- Configuring clangd for the `web/` TypeScript codebase (separate toolchain).
- Setting up `clang-format` (handled by the existing `xaver.clang-format` extension and any future `.clang-format` file).
- CI clang-tidy integration (separate concern; `run-clang-tidy` will work out of the box once `compile_commands.json` exists).
- Fixing the underlying CEF ABI mismatch between native modules (exceptions on) and CEF targets (exceptions off) — that is a build concern, not a clangd concern.
