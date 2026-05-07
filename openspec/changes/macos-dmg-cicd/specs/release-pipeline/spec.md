## ADDED Requirements

### Requirement: Tag-triggered release workflow
The system SHALL provide a GitHub Actions workflow that triggers on pushes of tags matching `v[0-9]+.[0-9]+.[0-9]+` and produces a GitHub Release with DMG artifacts for both arm64 and x86_64.

#### Scenario: Tag push triggers release
- **WHEN** a git tag matching `v*.*.*` is pushed to the repository
- **THEN** the release workflow starts and runs web-build, native-arm64, native-x86_64, and release jobs

#### Scenario: Release publishes both DMGs
- **WHEN** both native build jobs complete successfully
- **THEN** a GitHub Release is created at the tag with `cronymax-{version}-arm64.dmg` and `cronymax-{version}-x86_64.dmg` as release assets

#### Scenario: Version derived from tag
- **WHEN** the workflow runs for tag `v0.2.0`
- **THEN** both DMG filenames contain `0.2.0` and the app's `CFBundleShortVersionString` reads `0.2.0`

### Requirement: Web build runs once on Linux
The system SHALL build the web frontend exactly once per release on an `ubuntu-latest` runner and share the output with both native macOS jobs via a workflow artifact.

#### Scenario: Web artifact shared to native jobs
- **WHEN** the `web-build` job completes
- **THEN** `web/dist/` is available as a downloadable artifact that both `native-arm64` and `native-x86_64` jobs consume

#### Scenario: macOS jobs skip pnpm
- **WHEN** a native macOS job runs
- **THEN** it does NOT install Node.js or run pnpm; it uses `-DCRONYMAX_BUILD_WEB=OFF` and relies on the downloaded `web/dist/`

### Requirement: CEF URLs committed to repository
The system SHALL store CEF binary distribution URLs and SHA256 checksums for both architectures in `cmake/cef-version.env` in the repository root, parseable as key=value pairs.

#### Scenario: Workflow reads CEF config
- **WHEN** a native job runs on any machine
- **THEN** it sources `cmake/cef-version.env` to obtain `CEF_ARM64_URL`, `CEF_ARM64_SHA256`, `CEF_X86_64_URL`, `CEF_X86_64_SHA256` and passes the appropriate pair to CMake

#### Scenario: CEF download is cached across runs
- **WHEN** the same CEF URL was used in a previous run
- **THEN** the GitHub Actions cache restores the archive without re-downloading it

### Requirement: Cargo dependency cache across runs
The system SHALL cache Cargo registry and build artifacts between workflow runs, keyed on `Cargo.lock`.

#### Scenario: Cargo cache hit reduces build time
- **WHEN** `Cargo.lock` has not changed since the last successful run
- **THEN** the cargo registry is restored from cache and `cargo build` skips downloading crates

### Requirement: Release job creates GitHub Release from artifacts
The system SHALL have a dedicated `release` job that depends on both native jobs, downloads their DMG artifacts, and publishes a GitHub Release using the GitHub CLI.

#### Scenario: Release job waits for both arches
- **WHEN** one native job fails
- **THEN** the release job does not run and no GitHub Release is created

#### Scenario: Release notes from tag
- **WHEN** the release is created
- **THEN** release notes are populated from the annotated tag message or auto-generated from commits since the previous tag
