## 1. Repository Prerequisites

- [x] 1.1 Find the x86_64 CEF binary URL matching the current arm64 URL version (`cef_binary_147.0.10+gd58e84d+chromium-147.0.7727.118_macosx64.tar.bz2`) and compute its SHA256
- [x] 1.2 Create `cmake/cef-version.env` with `CEF_ARM64_URL`, `CEF_ARM64_SHA256`, `CEF_X86_64_URL`, `CEF_X86_64_SHA256`
- [x] 1.3 Add `rust-toolchain.toml` at repo root pinning `channel = "stable"` with targets `aarch64-apple-darwin` and `x86_64-apple-darwin`

## 2. App Icon

- [x] 2.1 Create or source `AppIcon.icns` (placeholder or final design) and place at `assets/installer/AppIcon.icns`
- [x] 2.2 Add `CFBundleIconFile` key pointing to `AppIcon.icns` in `app/browser/mac/Info.plist.in`
- [x] 2.3 Add `POST_BUILD` copy of `assets/installer/AppIcon.icns` → `$<TARGET_BUNDLE_CONTENT_DIR:cronymax_app>/Resources/AppIcon.icns` in `cmake/CronymaxApp.cmake`
- [ ] 2.4 Verify built `cronymax.app` shows the custom icon in Finder and Dock

## 3. Version Injection

- [x] 3.1 Add `if(DEFINED CRONYMAX_VERSION) set(PROJECT_VERSION "${CRONYMAX_VERSION}") endif()` block in `CMakeLists.txt` (after the `project()` call)
- [ ] 3.2 Verify that `cmake -DCRONYMAX_VERSION=9.9.9 ...` + build produces an Info.plist with `CFBundleShortVersionString = 9.9.9`

## 4. DMG Design Assets

- [x] 4.1 Create `assets/installer/dmg-background.png` (800×500 px recommended; placeholder or final design)
- [x] 4.2 Write `assets/installer/README.txt` with Gatekeeper unblock instructions (right-click → Open, or `xattr -d com.apple.quarantine`)

## 5. GitHub Actions Workflow

- [x] 5.1 Create `.github/workflows/release.yml` with trigger `on: push: tags: ['v[0-9]+.[0-9]+.[0-9]+']`
- [x] 5.2 Add `web-build` job: `ubuntu-latest`, `pnpm install --frozen-lockfile`, `pnpm --filter cronymax-web typecheck`, `pnpm --filter cronymax-web lint`, `pnpm --filter cronymax-web build`, upload `web/dist/` as artifact `web-dist`
- [x] 5.3 Add `native-arm64` job: `macos-15`, needs `web-build`, restore CEF cache keyed on `CEF_ARM64_URL`, restore cargo cache keyed on `Cargo.lock`, download `web-dist` artifact to `web/dist/`, cmake configure + build, `brew install create-dmg`, invoke `create-dmg` to produce `cronymax-{v}-arm64.dmg`, upload DMG artifact
- [x] 5.4 Add `native-x86_64` job: `macos-13`, mirror of `native-arm64` using x86_64 CEF URL and `x86_64-apple-darwin` Rust target, output `cronymax-{v}-x86_64.dmg`
- [x] 5.5 Add `release` job: `ubuntu-latest`, needs `[native-arm64, native-x86_64]`, download both DMG artifacts, `gh release create`
- [x] 5.6 Add `permissions: contents: write` scoped to the `release` job (not global)

## 6. Cache Configuration

- [x] 6.1 Configure GitHub Actions cache for CEF archive in `native-arm64` job: key = `cef-arm64-${{ hashFiles('cmake/cef-version.env') }}`, path = `~/.cef-cache`
- [x] 6.2 Configure GitHub Actions cache for CEF archive in `native-x86_64` job: key = `cef-x86_64-${{ hashFiles('cmake/cef-version.env') }}`, path = `~/.cef-cache`
- [x] 6.3 Configure GitHub Actions cache for Cargo registry in both native jobs: key = hash of `Cargo.lock`, paths = `~/.cargo/registry` and `~/.cargo/git`

## 7. Verification

- [ ] 7.1 Push a test tag (e.g. `v0.1.1-rc1`) to a fork or branch and confirm all four jobs run
- [ ] 7.2 Verify arm64 DMG: mounts, shows designed window, `cronymax.app` icon correct, drag-to-Applications works, README.txt visible
- [ ] 7.3 Verify x86_64 DMG: same checks on an Intel Mac or Rosetta environment
- [ ] 7.4 Verify GitHub Release is created with both DMGs attached and correct version in filenames
- [ ] 7.5 Verify app `CFBundleShortVersionString` matches the tag version in both DMGs
- [ ] 7.6 Confirm Gatekeeper prompt appears on first open and the README instructions resolve it
