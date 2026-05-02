## ADDED Requirements

### Requirement: App icon wired into macOS bundle

The system SHALL include an `AppIcon.icns` file in the app bundle's `Resources/` directory and declare it as `CFBundleIconFile` in `Info.plist` so macOS displays the custom icon in the Dock, Finder, and window title bar.

#### Scenario: App shows custom icon in Dock

- **WHEN** `cronymax.app` is running
- **THEN** the Dock shows the custom cronymax icon, not the generic macOS app icon

#### Scenario: App shows custom icon in Finder

- **WHEN** a user navigates to `cronymax.app` in Finder
- **THEN** the app's icon is the custom cronymax icon

### Requirement: App icon source asset committed to repository

The system SHALL store the canonical `AppIcon.icns` file at `assets/installer/AppIcon.icns` in the repository, used by both the CMake build (bundle Resources) and DMG packaging (volume icon).

#### Scenario: Single source of truth for icon

- **WHEN** `AppIcon.icns` is updated at `assets/installer/AppIcon.icns`
- **THEN** the next build automatically uses the updated icon in both the app bundle and the DMG volume

### Requirement: Icon copied into bundle at build time

The system SHALL copy `assets/installer/AppIcon.icns` into `cronymax.app/Contents/Resources/` as a `POST_BUILD` step in `cmake/CronymaxApp.cmake`.

#### Scenario: Icon present in built bundle

- **WHEN** `cmake --build` completes with `CRONYMAX_BUILD_APP=ON`
- **THEN** `cronymax.app/Contents/Resources/AppIcon.icns` exists

### Requirement: Rust toolchain pinned to stable

The system SHALL include a `rust-toolchain.toml` at the repository root pinning the Rust channel to `stable` with `aarch64-apple-darwin` and `x86_64-apple-darwin` targets declared.

#### Scenario: CI uses correct toolchain

- **WHEN** a GitHub Actions job runs `cargo build` on any macOS runner
- **THEN** `rustup` reads `rust-toolchain.toml` and uses the stable channel without additional setup steps

#### Scenario: Local dev gets consistent toolchain

- **WHEN** a developer runs `cargo build` locally after cloning
- **THEN** `rustup` installs and activates the pinned stable toolchain automatically
