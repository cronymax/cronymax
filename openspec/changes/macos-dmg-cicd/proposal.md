## Why

cronymax.app builds successfully but has no distribution story — releasing means manually copying a raw `.app` off a developer machine with no versioning, no packaging, and no repeatable process. This change adds a tag-triggered GitHub Actions pipeline that builds arch-specific signed DMG installers and publishes them to GitHub Releases automatically.

## What Changes

- Add `.github/workflows/release.yml` — tag-triggered workflow that builds and publishes two DMGs (arm64 + x86_64) to GitHub Releases
- Add `cmake/cef-version.env` — commit the CEF binary distribution URLs and SHA256s for both architectures so CI can resolve them without the local CMake cache
- Add `rust-toolchain.toml` — pin the Rust toolchain to stable with both macOS targets declared, making CI and local dev consistent
- Add `assets/installer/` directory — DMG design assets: `AppIcon.icns`, `dmg-background.png`, `README.txt` (Gatekeeper unblock instructions)
- Modify `app/browser/mac/Info.plist.in` — add `CFBundleIconFile` pointing to `AppIcon.icns`
- Modify `cmake/CronymaxApp.cmake` — copy `AppIcon.icns` into bundle `Resources/`, accept `-DCRONYMAX_VERSION` to override `PROJECT_VERSION` so the tag version flows into `Info.plist`

## Capabilities

### New Capabilities

- `release-pipeline`: Tag-triggered GitHub Actions workflow that builds arm64 + x86_64 DMGs in parallel and publishes them to a GitHub Release
- `dmg-packaging`: Scripted DMG creation using `create-dmg` with a designed installer window (background image, drag-to-Applications link, README for Gatekeeper)
- `app-icon`: App icon (`.icns`) wired into the macOS bundle and the DMG volume

### Modified Capabilities

_(none — no existing specs change)_

## Impact

- **New files**: `.github/workflows/release.yml`, `cmake/cef-version.env`, `rust-toolchain.toml`, `assets/installer/{AppIcon.icns,dmg-background.png,README.txt}`
- **Modified files**: `app/browser/mac/Info.plist.in`, `cmake/CronymaxApp.cmake`, `CMakeLists.txt` (or `cmake/CronymaxApp.cmake` — `CRONYMAX_VERSION` override)
- **CI dependencies**: `create-dmg` (via `brew install`), `pnpm` on ubuntu runner
- **No notarization** — ad-hoc signing only; Gatekeeper will prompt on first open. Users are guided by the bundled `README.txt`.
- **No universal binary** — two separate DMGs, arm64 and x86_64. Users pick based on their hardware.
- **Existing local builds unaffected** — all changes are additive or conditional on the new `-DCRONYMAX_VERSION` flag
