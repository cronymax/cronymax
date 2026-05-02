## Context

The build cache already has `CRONYMAX_BUILD_APP=ON` and `CEF_ROOT=third_party/cef` — proven to configure successfully (helper plists exist in `build/`). The only missing piece is `CMAKE_EXPORT_COMPILE_COMMANDS`. Both `ms-vscode.cpptools` and `vscode-clangd` are installed. Two clangd binaries exist: Apple clangd 21.0 at `/usr/bin/clangd` and LLVM-18 at `~/.local/llvm-18/bin/clangd`. The codebase has two structurally distinct zones: native modules (`src/common/`, `src/sandbox/`, `src/workspace/`, `src/terminal/`, `src/agent/`) and the CEF app layer (`src/app/`), which has `-fno-rtti`, `-fno-exceptions`, and uses `CefRefPtr` + `IMPLEMENT_REFCOUNTING`.

## Goals / Non-Goals

**Goals:**

- Generate `build/compile_commands.json` covering all targets including the CEF app and all helper executables.
- Configure clangd to use that database with zero manual symlinking.
- Eliminate the cpptools/clangd IntelliSense conflict.
- Enable clang-tidy as-you-type with check profiles appropriate to each code zone.

**Non-Goals:**

- CI tidy integration.
- Configuring clang-format.
- TypeScript/web tooling.
- Fixing the exceptions/RTTI ABI mismatch between native modules and CEF targets.

## Decisions

### D1. Use Apple clangd 21.0 (`/usr/bin/clangd`), not LLVM-18

**Decision:** Pin `clangd.path` to `/usr/bin/clangd` in `.vscode/settings.json`.

**Alternatives considered:**

- `~/.local/llvm-18/bin/clangd` — already in `$PATH`, version 18.1.8.

**Rationale:** The project compiles with Apple's `c++` (`/usr/bin/c++`). Apple clangd 21.0 shares the same SDK knowledge, header search paths, and platform ABI assumptions as the compiler. LLVM-18 is a separate toolchain install that can produce header-lookup mismatches for Apple SDK headers and Objective-C++ (`.mm`) files. Apple clangd is also newer (21 vs 18). LLVM-18 remains available for other tools (`clang-format`, analysis CLI) but is not the editor LSP.

### D2. Compilation database via `.clangd` directive, not symlink

**Decision:** Set `CompileFlags: CompilationDatabase: build` in `.clangd`. Do not create a `compile_commands.json` symlink at the project root.

**Alternatives considered:**

- Symlink `compile_commands.json → build/compile_commands.json` at project root — clangd finds it automatically.
- `--compile-commands-dir` argument in VS Code settings.

**Rationale:** Symlinks add git noise (`.gitignore` entry or committed dangling symlink risk). The `.clangd` `CompilationDatabase` key is explicit, self-documenting, and requires no filesystem artifact beyond the config file itself. The `--compile-commands-dir` argument in VS Code settings is redundant once `.clangd` is authoritative, and the `.clangd` file travels with the repo for all contributors.

### D3. Full CMake configure with `CRONYMAX_BUILD_APP=ON`

**Decision:** Generate `compile_commands.json` via a full configure that includes the app targets. The existing cache (`build/CMakeCache.txt`) already has `CRONYMAX_BUILD_APP=ON` and `CEF_ROOT=third_party/cef`. Adding `CMAKE_EXPORT_COMPILE_COMMANDS=ON` to `CMakeLists.txt` is sufficient; no extra flags needed at reconfigure time.

**Alternatives considered:**

- Configure with `CRONYMAX_BUILD_APP=OFF` and add fallback `.clangd` flags for `src/app/` — simpler but gives ~85% coverage; CEF-injected compiler defines (`__STDC_CONSTANT_MACROS`, `__STDC_FORMAT_MACROS`, `CEF_USE_SANDBOX`) would be missing from app sources.

**Rationale:** The configure has already succeeded with `CRONYMAX_BUILD_APP=ON` (helper plists, helper app dirs, libcef_dll_wrapper build dir all exist). The risk of configure failure is zero. Full coverage is strictly better: clangd sees the exact flags each file was built with, including CEF-injected defines and architecture flags.

### D4. Two-file `.clang-tidy` hierarchy, no `InheritParentConfig`

**Decision:** Two standalone `.clang-tidy` files:

- `/project/.clang-tidy` — for native modules; broader modernize + bugprone + performance checks.
- `/project/src/app/.clang-tidy` — for the CEF layer; conservative subset that excludes checks conflicting with CEF patterns.

Neither file sets `InheritParentConfig: true`.

**Alternatives considered:**

- Single `.clang-tidy` in `.clangd` inline — simpler but can't vary checks by subdirectory, not usable from CLI.
- `InheritParentConfig: true` in `src/app/.clang-tidy` with removals — less duplication but merging order surprises and less legible intent.
- Single uniform config everywhere — generates false positives in `src/app/` (`modernize-use-smart-pointers` fires on `CefRefPtr`; `cppcoreguidelines-owning-memory` fires on CEF `new` patterns).

**Rationale:** The two code zones are genuinely different in their constraints (RTTI off, exceptions off, intrusive refcounting in CEF land). Standalone per-directory configs are explicit, independent, and work identically whether driven by clangd or the `clang-tidy` CLI. Excluding `clang-analyzer-*` from both keeps type-time latency acceptable (path-sensitive analysis is 500ms–2s per file).

### D5. Disable `ms-vscode.cpptools` IntelliSense engine

**Decision:** Set `"C_Cpp.intelliSenseEngine": "disabled"` in `.vscode/settings.json`.

**Rationale:** cpptools and clangd both provide hover, completions, go-to-def, and diagnostics. Running both produces duplicated and conflicting results. clangd is strictly superior here given `compile_commands.json`. The cpptools extension itself can remain installed for its debugger (`ms-vscode.cpptools` provides the `cppdbg` debug adapter); only its IntelliSense engine is disabled.

### D6. No `clang-analyzer-*` checks at type-time

**Decision:** Explicitly exclude `clang-analyzer-*` from both `.clang-tidy` files.

**Rationale:** clang-analyzer checks are path-sensitive (inter-procedural data-flow), costing 500ms–2s per file even on small translation units. Type-time tidy runs on every edit. The performance cost is prohibitive. Analyzer checks belong in CI via `scan-build` or a dedicated `run-clang-tidy` pass, not in the editor feedback loop.

### D7. Check selection for native modules

Root `.clang-tidy` includes:

- `bugprone-integer-overflow`, `bugprone-signed-char-misuse`, `bugprone-use-after-move` — relevant to POSIX code and move-heavy modern C++.
- `modernize-use-override`, `modernize-use-nullptr`, `modernize-loop-convert` — style consistency.
- `performance-unnecessary-copy-initialization`, `performance-move-const-arg`, `performance-for-range-copy` — relevant to the string/vector-heavy agent and workspace code.
- `readability-container-size-empty` — catches `.size() == 0` patterns.

Excluded: `modernize-use-trailing-return-type` (controversial style), `readability-magic-numbers` (too noisy in SQLite row-index code), all `clang-analyzer-*`.

### D8. Check selection for CEF app layer

`src/app/.clang-tidy` includes only:

- `modernize-use-override` — high value in CEF subclass-heavy code.
- `bugprone-use-after-move` — safe, fast, applicable.
- `readability-container-size-empty` — safe, fast.

Excluded vs. root: `modernize-use-smart-pointers` (`CefRefPtr` IS the smart pointer; this check would fire on every `CefRefPtr<>` return), `cppcoreguidelines-owning-memory` (`new ClientHandler()` is the canonical CEF pattern), all performance checks (CEF objects are refcounted, not moved).
