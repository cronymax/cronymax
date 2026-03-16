# Contributing to cronymax

## Development Prerequisites

- **Rust nightly** — Install via [rustup](https://rustup.rs/). The project uses `rust-toolchain.toml` to pin the nightly channel with clippy and rustfmt components.
- **macOS** — Xcode command line tools (`xcode-select --install`)
- **Linux** — System dependencies:
  ```bash
  sudo apt-get install -y \
    libwebkit2gtk-4.1-dev libgtk-3-dev pkg-config \
    libssl-dev libdbus-1-dev libxdo-dev \
    libxkbcommon-dev libvulkan-dev
  ```
- **Windows** — MSVC Build Tools: Install [Visual Studio 2022](https://visualstudio.microsoft.com/) or [Build Tools for Visual Studio 2022](https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022) with the "Desktop development with C++" workload.

## Building

```bash
cargo build          # Debug build
cargo build --release  # Release build
cargo test           # Run all tests
```

## Code Style Rules

### File size limit: 500 lines

No `.rs` file in `src/` may exceed 500 lines. This is enforced by CI via `scripts/check-line-count.sh`.

When a file approaches the limit, decompose it:

1. Create a subdirectory with the same name as the module (e.g., `foo.rs` → `foo/mod.rs`)
2. Extract logical groups into sibling files (e.g., `foo/input.rs`, `foo/rendering.rs`)
3. Use `pub(in crate::app)` or `pub(super)` for internal APIs and re-export public items from `mod.rs`
4. Use `use crate::app::*;` in nested modules (not `use super::*;`)

### Nested module conventions (`src/app/`)

The `src/app/` module uses nested directories to group related files:

| Directory            | Contents                                                    |
| -------------------- | ----------------------------------------------------------- |
| `src/app/cmd/`       | UI action command handlers (settings, webview, split, etc.) |
| `src/app/events/`    | Application event handlers (LLM, channel, misc, onboard)    |
| `src/app/draw/`      | Frame rendering, overlays, terminal panes, post-frame       |
| `src/app/lifecycle/` | Application init, LLM provider detection                    |
| `src/app/window/`    | Window event handling, resize, focus, scaling               |

Files in nested modules use `use crate::app::*;` to import the parent module's items.

### Function argument limit: 5 parameters

Functions should accept at most 5 parameters. Use struct parameters or builder patterns for more complex signatures.

### Clippy zero-warnings policy

```bash
cargo clippy -- -D warnings
```

All clippy warnings are treated as errors. CI will reject PRs that introduce any warnings.

### Formatting

```bash
cargo fmt --check  # Verify formatting
cargo fmt          # Auto-format
```

## Architecture Overview

The project is a single Cargo package (no workspace) with these top-level modules:

| Module          | Purpose                                                    |
| --------------- | ---------------------------------------------------------- |
| `src/app/`      | Application lifecycle, event loop, state management        |
| `src/ai/`       | AI chat, agent tool execution, LLM client                  |
| `src/renderer/` | wgpu pipeline, text atlas, cursor, quad rendering          |
| `src/terminal/` | PTY session management (alacritty_terminal + portable-pty) |
| `src/ui/`       | egui widget system, settings panels, completion, titlebar  |
| `src/webview/`  | wry-based embedded browser panels                          |
| `src/channel/`  | Messaging channel integration                              |
| `src/service/`  | Platform services                                          |
| `src/sandbox/`  | Sandboxing and security policies                           |
| `src/secret/`   | Credential/secret management                               |
| `src/profile/`  | User profile storage                                       |
| `src/config.rs` | TOML configuration loading                                 |

### Widget trait system

All UI panels implement the `PanelWidget` trait:

```rust
pub trait PanelWidget {
    fn name(&self) -> &str;
    fn draw(&mut self, ctx: &mut WidgetCtx<'_>) -> WidgetResponse;
}
```

### Event-driven rendering

The render loop uses the `RenderSchedule` trait (`src/renderer/scheduler.rs`) for frame scheduling. The `FrameScheduler` implementation manages dirty-flag tracking, timer deadlines, and frame coalescing (~250fps cap via 4ms minimum frame interval). Uses winit's `ControlFlow::Wait` with `WaitUntil` for timer-based events (cursor blink, deferred egui repaint).

## PR Process

1. Create a feature branch from `001-webgpu-terminal-app`
2. Make your changes following the code style rules above
3. Ensure all checks pass:
   ```bash
   cargo fmt --check
   cargo clippy -- -D warnings
   cargo test
   bash scripts/check-line-count.sh
   ```
4. Push and open a pull request
5. CI will automatically run the check matrix on Ubuntu and macOS

## Testing

Tests live in the `tests/` directory:

- `tests/unit/` — Unit tests for individual modules
- `tests/integration/` — Integration tests (PTY, rendering)
- `tests/fixtures/` — Test configuration files and data
