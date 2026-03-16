# cronymax

**cronymax** is a WebGPU-native terminal emulator with integrated AI assistant and webview panels, built entirely in Rust.

## Feature Highlights

- **GPU-accelerated text rendering** — Uses wgpu 28 + glyphon for hardware-accelerated glyph rasterization and compositing at 60fps+
- **Integrated webview** — Embed web pages directly in the terminal window via wry, with bidirectional terminal ↔ webview messaging
- **AI chat & agent** — Built-in LLM integration with streaming responses, tool execution, and conversation history stored in SQLite
- **Split panes & tabs** — Multiple terminal sessions with horizontal/vertical splits and tab navigation
- **Event-driven architecture** — Idle CPU < 3% through WaitUntil-based event loop with frame coalescing
- **Configurable** — TOML-based configuration for fonts, themes, keybindings, and AI providers

## Tech Stack

| Component        | Technology                                |
| ---------------- | ----------------------------------------- |
| GPU rendering    | wgpu 28, glyphon 0.10                     |
| Windowing        | winit 0.30                                |
| UI widgets       | egui 0.31 (custom wgpu renderer)          |
| Terminal backend | alacritty_terminal 0.25, portable-pty 0.9 |
| Webview          | wry 0.54                                  |
| Async runtime    | tokio (multi-thread)                      |
| AI client        | async-openai                              |
| Storage          | SQLite, TOML config, OS keychain          |

## Supported Platforms

- macOS ARM64 (Apple Silicon) — primary
- macOS x86_64 (Intel)
- Linux x86_64
