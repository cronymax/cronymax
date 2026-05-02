## ADDED Requirements

### Requirement: IconId vocabulary

The system SHALL define an `enum class IconId` in `app/browser/icon_registry.h` covering all icon roles used across native controls: `kBack`, `kForward`, `kRefresh`, `kStop`, `kNewTab`, `kClose`, `kSettings`, `kTabTerminal`, `kTabChat`, `kTabAgent`, `kTabGraph`, `kTabWeb`, `kRestart`, `kCount` (sentinel). Every `IconId` value SHALL have a corresponding SVG file in `assets/icons/` and a registry entry mapping it to a `CefImage`.

#### Scenario: Full coverage at startup

- **WHEN** `IconRegistry::Init()` completes
- **THEN** `IconRegistry::GetImage(id)` returns a non-null `CefRefPtr<CefImage>` for every `IconId` value except `kCount`

#### Scenario: Missing asset is fatal at startup

- **WHEN** an expected SVG file is absent from the bundle's `Resources/icons/` directory
- **THEN** `IconRegistry::Init()` logs a fatal error and the app fails to launch with a descriptive message

---

### Requirement: Startup rasterisation via macOS Core Graphics

`IconRegistry::Init()` SHALL rasterise each SVG at two logical sizes (16×16 and 20×20 pixels) using `NSImage` at the main display's device pixel ratio. The resulting pixel data SHALL be stored in a `CefImage` via `CefImage::AddBitmap()`. Rasterisation SHALL complete before `MainWindow::CreateControls()` is called.

#### Scenario: HiDPI rasterisation

- **WHEN** `IconRegistry::Init()` runs on a Retina display (device pixel ratio = 2.0)
- **THEN** each stored `CefImage` contains a 32×32-pixel bitmap at scale factor 2.0 for the 16-logical-pixel slot

#### Scenario: Init before window creation

- **WHEN** the app starts
- **THEN** `IconRegistry::Init()` is called inside `DesktopApp::OnContextInitialized()` before any `CefWindow::CreateTopLevelWindow()` call

---

### Requirement: Image retrieval API

`IconRegistry` SHALL expose a static `GetImage(IconId id, int logical_size = 16)` method returning a `CefRefPtr<CefImage>`. Callers SHALL NOT cache the returned pointer beyond a single button-construction site; the registry owns the lifetime.

#### Scenario: Retrieve 20px image

- **WHEN** `IconRegistry::GetImage(IconId::kSettings, 20)` is called
- **THEN** it returns the CefImage rasterised at logical size 20

#### Scenario: Unsupported size falls back to 16

- **WHEN** `IconRegistry::GetImage(IconId::kBack, 24)` is called with a size that was not rasterised
- **THEN** it returns the 16px image and logs a warning

---

### Requirement: Asset vendoring

SVG source files for all `IconId` values SHALL be stored in `assets/icons/` at a pinned version of VS Code Codicons. An `assets/icons/README.md` SHALL document the Codicons version pinned, the list of icon files and their corresponding `IconId` values, and the procedure for updating the set.

#### Scenario: README present

- **WHEN** the `assets/icons/` directory is inspected
- **THEN** a `README.md` file is present containing the pinned Codicons version string

#### Scenario: Icon file naming convention

- **WHEN** the `assets/icons/` directory is listed
- **THEN** each file is named `<codicon-name>.svg` matching the Codicons filename convention (e.g., `arrow-left.svg`, `refresh.svg`, `settings-gear.svg`)
