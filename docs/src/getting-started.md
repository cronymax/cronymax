# Getting Started

## Installation from source

### Prerequisites

- **Rust nightly toolchain** — The project uses `rust-toolchain.toml` to automatically select the correct nightly version with clippy and rustfmt components. Install Rust via [rustup](https://rustup.rs/).

- **macOS** — Xcode command line tools:

  ```bash
  xcode-select --install
  ```

- **Linux (Ubuntu/Debian)** — System libraries:

  ```bash
  sudo apt-get update
  sudo apt-get install -y \
    libwebkit2gtk-4.1-dev libgtk-3-dev pkg-config \
    libssl-dev libdbus-1-dev libxdo-dev \
    libxkbcommon-dev libvulkan-dev
  ```

- **Windows** — Install [Build Tools for Visual Studio 2022](https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022) with the "Desktop development with C++" workload.

### Build

```bash
git clone https://github.com/user/cronymax.git
cd cronymax
cargo build --release
```

The compiled binary will be at `target/release/cronymax`.

## Pre-built binaries

Download the latest release from the [Releases](https://github.com/user/cronymax/releases) page:

| Platform       | Artifact                        |
| -------------- | ------------------------------- |
| Linux x86_64   | `cronymax-linux-x86_64.tar.gz`  |
| macOS ARM64    | `cronymax-macos-aarch64.tar.gz` |
| macOS x86_64   | `cronymax-macos-x86_64.tar.gz`  |
| Windows x86_64 | `cronymax-windows-x86_64.zip`   |

Extract and run:

```bash
# Linux / macOS
tar xzf cronymax-*.tar.gz
./cronymax

# Windows (PowerShell)
Expand-Archive cronymax-windows-x86_64.zip -DestinationPath .
.\cronymax.exe
```

## First Run

Launch the terminal:

```bash
cronymax
```

On first run, cronymax creates a default configuration at `~/.config/cronymax/config.toml`. You can customize fonts, themes, keybindings, and AI providers by editing this file.

### Basic keybindings

| Action           | macOS         |
| ---------------- | ------------- |
| New tab          | `Cmd+T`       |
| Close tab        | `Cmd+W`       |
| Split horizontal | `Cmd+D`       |
| Split vertical   | `Cmd+Shift+D` |
| Toggle AI chat   | `Cmd+L`       |
| Open webview     | `Cmd+Shift+B` |
| Copy             | `Cmd+C`       |
| Paste            | `Cmd+V`       |

## Configuration

Edit `~/.config/cronymax/config.toml`:

```toml
[font]
family = "Berkeley Mono"
size = 14.0
line_height = 1.2

[terminal]
cursor_style = "block"   # block, underline, bar
scrollback = 10000

[theme]
name = "dark"
```
