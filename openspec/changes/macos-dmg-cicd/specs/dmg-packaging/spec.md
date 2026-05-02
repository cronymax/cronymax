## ADDED Requirements

### Requirement: Designed DMG installer window

The system SHALL produce a DMG with a designed installer window: a background image, the app icon positioned on the left, a drag-to-Applications symlink on the right, and a `README.txt` in the DMG root.

#### Scenario: DMG opens with designed layout

- **WHEN** a user opens the DMG in Finder
- **THEN** a window opens showing the background image, `cronymax.app` icon on the left, Applications alias on the right, and a `README.txt` file visible in the window

#### Scenario: Drag-to-Applications installs app

- **WHEN** a user drags `cronymax.app` onto the Applications alias inside the DMG
- **THEN** the app is copied to `/Applications/cronymax.app`

### Requirement: DMG volume icon set

The system SHALL set the DMG volume icon to `AppIcon.icns` so the mounted volume shows the app icon in the Finder sidebar.

#### Scenario: Volume icon visible in Finder sidebar

- **WHEN** the DMG is mounted
- **THEN** the volume appears in the Finder sidebar with the cronymax icon, not a generic disk image icon

### Requirement: Gatekeeper README included

The system SHALL include a `README.txt` inside the DMG root explaining how to open the app on macOS without a Developer ID signature.

#### Scenario: README visible in DMG

- **WHEN** a user opens the DMG
- **THEN** a `README.txt` is visible alongside the app icon, containing instructions to right-click → Open to bypass the Gatekeeper prompt

#### Scenario: README content is accurate

- **WHEN** a user follows the README instructions
- **THEN** they can open the app on macOS 14+ without moving it to the trash

### Requirement: DMG filenames encode version and architecture

The system SHALL name the output DMGs `cronymax-{version}-arm64.dmg` and `cronymax-{version}-x86_64.dmg` where `{version}` is the semver string from the triggering git tag.

#### Scenario: arm64 DMG filename

- **WHEN** the workflow runs for tag `v0.2.0` on the arm64 job
- **THEN** the output DMG is named `cronymax-0.2.0-arm64.dmg`

#### Scenario: x86_64 DMG filename

- **WHEN** the workflow runs for tag `v0.2.0` on the x86_64 job
- **THEN** the output DMG is named `cronymax-0.2.0-x86_64.dmg`

### Requirement: CI-resilient DMG creation

The system SHALL invoke `create-dmg` with retry and sleep flags to tolerate Finder/AppleScript timing issues on GitHub Actions macOS runners.

#### Scenario: AppleScript retry on "Can't get disk" error

- **WHEN** `create-dmg`'s AppleScript step encounters a Finder timing error
- **THEN** the tool retries up to the configured limit before failing the job

#### Scenario: DMG creation succeeds on clean runner

- **WHEN** the workflow runs on a fresh GitHub-hosted macOS runner
- **THEN** `create-dmg` completes without manual intervention
