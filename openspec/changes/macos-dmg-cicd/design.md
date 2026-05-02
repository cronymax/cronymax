## Context

cronymax is a CEF-based desktop app with a multi-layer build: TypeScript/React frontend (Vite/pnpm), a Rust runtime (`crony`, compiled via `cargo`), and a C++ CEF shell (CMake). On macOS the output is a `.app` bundle with several helper apps, the Chromium Embedded Framework, and the Rust `crony` binary nested inside `Contents/Frameworks/`.

Currently the app builds and runs locally but:

- The CEF binary URL (`CRONYMAX_CEF_DIST_URL`) lives only in the local CMake cache, not in the repo
- There is no `rust-toolchain.toml`; developers run nightly Rust locally but the code requires only stable 1.85+
- The app has no `.icns` icon — `Info.plist` has no `CFBundleIconFile` key
- There are no CI workflows at all
- Ad-hoc signing (linker-signed) is used; no Developer ID cert exists

Distribution target: GitHub Releases, macOS 14.5+, two separate DMGs (arm64 and x86_64).

## Goals / Non-Goals

**Goals:**

- Automated, tag-triggered release pipeline publishing arm64 and x86_64 DMGs to GitHub Releases
- Repeatable CMake build: CEF URLs + SHA256s committed to the repo
- Designed DMG installer window (background, drag-to-Applications, README)
- App icon wired into bundle + DMG volume
- Version derived from git tag, injected into `Info.plist`
- Rust toolchain pinned in-repo (stable, both macOS targets)

**Non-Goals:**

- Apple Developer ID signing or notarization (no cert; users use right-click → Open)
- Universal binary (fat Mach-O with both arches); two separate arch-specific DMGs instead
- Linux or Windows builds
- Homebrew cask or other distribution channels
- Automated testing in CI beyond typecheck + lint on the web frontend
- Self-hosted runners

## Decisions

### D1: Two separate DMGs (arm64 + x86_64), not one universal binary

**Chosen**: Ship `cronymax-{v}-arm64.dmg` and `cronymax-{v}-x86_64.dmg` as separate release artifacts.

**Why not universal**: CEF has no universal binary distribution — two separate ~270 MB tarballs (arm64 and x86_64). Building a fat universal bundle requires downloading both, building both C++ and Rust for both arches, then `lipo`-ing ~20 binaries across the CEF framework, helpers, and `crony`. The lipo assembly script is significant engineering for limited user-facing benefit: users on macOS have been picking "Apple Silicon" vs "Intel" downloads for years (Electron, VS Code, Firefox all do this). Universal binary can be added later as a separate change.

**Alternatives**: Single universal job with `CMAKE_OSX_ARCHITECTURES=arm64;x86_64` — rejected because CMake's arch flag doesn't propagate to `cargo` (`RustRuntime.cmake` calls `cargo` directly), and CEF has no pre-built universal distribution.

### D2: Web build runs once on ubuntu-latest, shared as a workflow artifact

**Chosen**: A `web-build` job on `ubuntu-latest` runs `pnpm install + typecheck + lint + build`, uploads `web/dist/` as an artifact. Both native macOS jobs download this artifact and place it at `web/dist/` before running CMake with `-DCRONYMAX_BUILD_WEB=OFF`.

**Why**: `web/dist/` is platform-agnostic (pure JS/CSS/HTML). Running pnpm on ubuntu is faster and cheaper than on macOS runners (10× billing multiplier). The `CRONYMAX_BUILD_WEB=OFF` flag suppresses CMake's `cronymax_web` target while the `POST_BUILD copy_directory` step still copies `web/dist/` into the bundle — which is exactly what we want.

**Risk**: If someone changes `cmake/CronymaxApp.cmake` so that web is no longer a separate gate, this coupling could silently break. Mitigated by the web job being a required dependency of both native jobs.

### D3: `cmake/cef-version.env` for CEF URLs

**Chosen**: A plain key=value `.env` file at `cmake/cef-version.env`:

```
CEF_ARM64_URL=https://cef-builds.spotifycdn.com/...macosarm64.tar.bz2
CEF_ARM64_SHA256=<sha>
CEF_X86_64_URL=https://cef-builds.spotifycdn.com/...macosx64.tar.bz2
CEF_X86_64_SHA256=<sha>
```

**Why not `cmake/cef-version.cmake`**: The `.env` format is trivially parseable in bash (`source cmake/cef-version.env`) by the workflow, avoids CMake invocation just to extract strings, and keeps the CI layer decoupled from the CMake layer. CMake already accepts the URL as a `-D` flag; the workflow just reads the file and passes it.

**Why not hardcode in workflow YAML**: Harder to maintain, CEF bumps happen independently of workflow logic changes.

### D4: Version injection via `-DCRONYMAX_VERSION`

**Chosen**: CI extracts the semver from the git tag (`${GITHUB_REF_NAME#v}`) and passes `-DCRONYMAX_VERSION=<ver>` to CMake. `CMakeLists.txt` (or `cmake/CronymaxApp.cmake`) adds:

```cmake
if(DEFINED CRONYMAX_VERSION)
  set(PROJECT_VERSION "${CRONYMAX_VERSION}")
endif()
```

**Why**: `PROJECT_VERSION` is set by `project()` and not directly overridable by `-DPROJECT_VERSION`. The `CRONYMAX_VERSION` indirection is the cleanest in-tree pattern — doesn't require changing the `project()` call, works alongside local builds (where `CRONYMAX_VERSION` is unset and `0.1.0` is used), and is explicit about intent.

**DMG filename**: Constructed in the workflow shell from `$GITHUB_REF_NAME`, independent of CMake.

### D5: `create-dmg` (not `hdiutil`) with retry flags

**Chosen**: `brew install create-dmg` in the CI step, then invoke with `--applescript-sleep-duration 10 --hdiutil-retries 10`.

**Why**: User wants a designed DMG window from day one (background image, drag-to-Applications, icon placement). `hdiutil` alone produces a bare volume with no Finder layout. `create-dmg` uses AppleScript to configure the Finder window — this works on GitHub macOS runners (full GUI session), but is occasionally flaky. The retry/sleep flags mitigate the two known failure modes: "Can't get disk" (-1728) and "Resource busy" on `hdiutil attach`.

**Sandbox mode**: `--sandbox-safe` skips AppleScript entirely — useful if the Finder approach proves unreliable on future runner versions. Design assets still render as icons; the window just won't have the custom layout. This is a fallback escape hatch.

### D6: Ad-hoc signing, README.txt for Gatekeeper

**Chosen**: No `codesign --sign` with a Developer ID. The bundle remains linker-signed (ad-hoc). A `README.txt` is placed in the DMG root explaining how to right-click → Open to bypass Gatekeeper on first launch.

**Why**: No Developer ID Application cert exists. Notarization requires one. The target audience (developers/technical users) can follow the README instructions. When a cert is obtained, `create-dmg --codesign <identity>` and `--notarize` can be added in a single-line change to the workflow.

### D7: `rust-toolchain.toml` pinned to stable

**Chosen**: Add `rust-toolchain.toml` at repo root:

```toml
[toolchain]
channel = "stable"
targets = ["aarch64-apple-darwin", "x86_64-apple-darwin"]
```

**Why**: The code uses no nightly features (confirmed: no `#![feature(...)]` in source). Developers run nightly locally but the workspace declares `rust-version = "1.85"`. Pinning to stable makes CI predictable and surfaces any accidental nightly-only usage immediately.

## Risks / Trade-offs

**[Risk] `macos-13` runner deprecation** → GitHub is the last Intel-native runner being `macos-13`. If GitHub deprecates it before Universal binary support is added, the x86_64 DMG drops from releases. Mitigation: monitor GitHub runner changelog; Universal binary change is a known future upgrade path.

**[Risk] `create-dmg` AppleScript flakiness on CI** → Occasional "Can't get disk" errors cause workflow failures unrelated to code changes. Mitigation: `--applescript-sleep-duration 10 --hdiutil-retries 10`. Fallback: switch to `--sandbox-safe` if flakiness persists; last resort, switch to `dmgbuild` (Python, no Finder/AppleScript).

**[Risk] CEF URL goes stale** → CEF version in `cmake/cef-version.env` must stay in sync with the `cef/` submodule. If the submodule is bumped without updating the `.env`, CI will download a mismatched framework and the build will fail at configure time (CEF's `FindCEF.cmake` version check). Mitigation: document the update procedure in the `.env` file header.

**[Risk] No app icon design assets yet** → `AppIcon.icns` and `dmg-background.png` don't exist. CI can use a placeholder (e.g., a simple generated icns) until design assets are ready. The `CFBundleIconFile` key and copy step must still be wired even if the placeholder is ugly.

**[Trade-off] Two DMGs vs Universal** → Intel users must consciously download the x86_64 variant. "Apple Silicon" / "Intel" labels on GitHub Releases are a well-understood convention; this is the same pattern used by Electron, VS Code, and Firefox.

**[Trade-off] No notarization** → Gatekeeper prompt on first open for all users. Not bypassed by any workflow step. Acceptable for a technical audience; unacceptable for a mainstream distribution. When a Developer ID cert is obtained, notarization can be added as a 2-line workflow addition.

## Open Questions

- **x86_64 CEF SHA256**: The x86_64 equivalent of the current arm64 URL needs to be fetched and its SHA256 verified before `cmake/cef-version.env` can be committed. (Low effort, manual step.)
- **App icon design**: Who creates `AppIcon.icns` and `dmg-background.png`? These are design artifacts, not engineering. A placeholder can unblock CI; final assets can be swapped in without any code changes.
- **Tag format**: Workflow assumes `v{major}.{minor}.{patch}` tags (e.g. `v0.2.0`). Is this the agreed format?
