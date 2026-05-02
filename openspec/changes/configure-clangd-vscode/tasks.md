## 1. Enable compile_commands.json generation

- [x] 1.1 In `CMakeLists.txt`, add `set(CMAKE_EXPORT_COMPILE_COMMANDS ON)` after the `cmake_minimum_required` / `project` header block, before any `add_library`/`add_executable` calls.
- [ ] 1.2 Reconfigure CMake: run `cmake -B build .` from the project root. The existing cache already has `CRONYMAX_BUILD_APP=ON` and `CEF_ROOT=third_party/cef`; no extra flags required.
- [ ] 1.3 Verify `build/compile_commands.json` exists and contains entries for both native module sources (e.g. `src/agent/agent_runtime.cc`) and app sources (e.g. `src/app/bridge_handler.cc`).

## 2. Create .clangd project config

- [x] 2.1 Create `.clangd` at the project root with the following sections:
  - `CompileFlags.CompilationDatabase: build` — directs clangd to `build/compile_commands.json` without a symlink.
  - `Diagnostics.UnusedIncludes: Strict` — warn on unused `#include`s.
  - `Diagnostics.ClangTidy.Add: ['*']` — defer all check selection to `.clang-tidy` files.
  - `Index.Background: Build` — build the project index in the background for cross-reference.
  - `InlayHints.Enabled: Yes`, `InlayHints.ParameterNames: Yes`, `InlayHints.DeducedTypes: Yes`.
- [ ] 2.2 Open a C++ source file in VS Code and confirm the clangd status bar item shows "clangd: idle" (not error) after indexing starts.

## 3. Create root .clang-tidy (native modules)

- [x] 3.1 Create `.clang-tidy` at the project root. Enable the following checks (no `InheritParentConfig`):
  - `bugprone-integer-overflow`
  - `bugprone-signed-char-misuse`
  - `bugprone-use-after-move`
  - `modernize-use-override`
  - `modernize-use-nullptr`
  - `modernize-loop-convert`
  - `performance-unnecessary-copy-initialization`
  - `performance-move-const-arg`
  - `performance-for-range-copy`
  - `readability-container-size-empty`
  - Disable: `clang-analyzer-*`, `modernize-use-trailing-return-type`, `readability-magic-numbers`.
- [ ] 3.2 Verify: open `src/agent/tool_registry.cc` in VS Code. Clangd tidy squiggles should appear only for actual issues — no spurious `CefRefPtr` or smart-pointer warnings.

## 4. Create src/app/.clang-tidy (CEF layer)

- [x] 4.1 Create `src/app/.clang-tidy` as a standalone config (no `InheritParentConfig: true`). Enable only CEF-safe checks:
  - `modernize-use-override`
  - `bugprone-use-after-move`
  - `readability-container-size-empty`
  - Disable: `clang-analyzer-*`, `modernize-use-smart-pointers`, `cppcoreguidelines-owning-memory`, all `performance-*`.
- [ ] 4.2 Verify: open `src/app/bridge_handler.cc` in VS Code. No false-positive smart-pointer or ownership warnings. `modernize-use-override` should still fire if any `override` keyword is missing.

## 5. Create .vscode/settings.json

- [x] 5.1 Create `.vscode/settings.json` with:
  - `"clangd.path": "/usr/bin/clangd"` — pin Apple clangd 21.0.
  - `"clangd.arguments": ["--background-index", "--clang-tidy", "--completion-style=detailed", "--header-insertion=iwyu"]`.
  - `"C_Cpp.intelliSenseEngine": "disabled"` — eliminate the cpptools/clangd conflict.
  - `"[cpp]": { "editor.defaultFormatter": "llvm-vs-code-extensions.vscode-clangd" }` — clangd as C++ formatter.
  - `"[c]": { "editor.defaultFormatter": "llvm-vs-code-extensions.vscode-clangd" }`.
- [ ] 5.2 Reload the VS Code window (`Developer: Reload Window`).
- [ ] 5.3 Verify: open `src/common/path_utils.h`. Hover over a type — should show clangd hover (not cpptools). Status bar should show clangd version.
- [ ] 5.4 Verify: open `src/app/main_window.h`. Hover over `CefWindowDelegate` — should resolve to the CEF header in `third_party/cef/include/views/cef_window.h`.

## 6. Smoke test cross-module navigation

- [ ] 6.1 In `src/agent/agent_runtime.cc`, go-to-definition on `FileBroker` — should jump to `src/workspace/file_broker.h`.
- [ ] 6.2 In `src/app/client_handler.cc`, go-to-definition on `CefClient` — should jump to `third_party/cef/include/cef_client.h`.
- [ ] 6.3 In `src/app/bridge_handler.cc`, go-to-definition on `CefMessageRouterBrowserSide` — should resolve (tests CEF wrapper headers).
- [ ] 6.4 Confirm no "file not found" diagnostics on any header in `src/` (clangd Problems panel should be clean of include errors).
