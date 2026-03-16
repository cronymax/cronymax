# cronymax

A WebGPU-native terminal emulator with integrated AI and webview panels, built in Rust.

## Features

- **GPU-accelerated rendering** — WebGPU text rendering pipeline via wgpu 28 + glyphon for smooth 60fps+ output
- **Integrated webview** — Open web pages inline within the terminal window using wry
- **AI chat & agent** — Built-in AI assistant with tool use, streaming responses, and conversation history
- **Split panes** — Horizontal/vertical terminal splits with keyboard navigation
- **Event-driven architecture** — Idle CPU < 3% with WaitUntil-based render loop and frame coalescing
- **Configurable** — TOML config for fonts, themes, keybindings, and AI providers
- **Cross-platform** — macOS (primary), Linux x86_64, Windows x86_64

## Screenshots

<!-- TODO: Add screenshots -->

## Installation

### From source

Prerequisites:

- Rust nightly toolchain (see `rust-toolchain.toml`)
- macOS: Xcode command line tools
- Linux: `libwebkit2gtk-4.1-dev libgtk-3-dev pkg-config libssl-dev libdbus-1-dev libxdo-dev libxkbcommon-dev libvulkan-dev`
- Windows: MSVC Build Tools (Visual Studio 2022 or Build Tools for Visual Studio 2022)

```bash
git clone https://github.com/user/cronymax.git
cd cronymax
cargo build --release
```

The binary will be at `target/release/cronymax`.

### Pre-built binaries

Download from the [Releases](https://github.com/user/cronymax/releases) page. Available for:

- Linux x86_64 (`.tar.gz`)
- macOS ARM64 / Apple Silicon (`.tar.gz`)
- macOS x86_64 / Intel (`.tar.gz`)
- Windows x86_64 (`.zip`)

## Usage

```bash
# Launch the terminal
cronymax

# With a specific config file
cronymax --config path/to/config.toml
```

### Keybindings

| Action           | Keybinding    |
| ---------------- | ------------- |
| New tab          | `Cmd+T`       |
| Close tab        | `Cmd+W`       |
| Split horizontal | `Cmd+D`       |
| Split vertical   | `Cmd+Shift+D` |
| Toggle AI chat   | `Cmd+L`       |
| Open webview     | `Cmd+Shift+B` |

### Configuration

Configuration is stored in `~/.config/cronymax/config.toml`:

```toml
[font]
family = "Berkeley Mono"
size = 14.0
line_height = 1.2

[terminal]
cursor_style = "block"
scrollback = 10000

[theme]
name = "dark"
```

## Architecture

See [docs/src/architecture.md](docs/src/architecture.md) for the full architecture overview.

Key components:

- **Renderer** (`src/renderer/`) — wgpu pipeline, atlas texture, quad rendering, render scheduling
- **Terminal** (`src/terminal/`) — PTY session management via alacritty_terminal + portable-pty
- **UI** (`src/ui/`) — egui-based widget system with trait-based composition
- **AI** (`src/ai/`) — LLM client, agent tool execution, streaming responses
- **Webview** (`src/webview/`) — wry-based embedded browser panels
- **App** (`src/app/`) — Application logic organized into nested modules:
  - `cmd/` — UI action command handlers
  - `events/` — Application event handlers
  - `draw/` — Frame rendering and overlay drawing
  - `lifecycle/` — Application initialization and lifecycle
  - `window/` — Window event handling and management

## Development

```bash
# Build
cargo build

# Run tests
cargo test

# Clippy (zero warnings policy)
cargo clippy -- -D warnings

# Format
cargo fmt
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for development guidelines.

## License

MIT
