# Architecture

## Overview

cronymax is a single Cargo package (no workspace) structured as a modular Rust application. The main event loop is powered by winit, with GPU rendering via wgpu and UI widgets via egui.

## Module Structure

```text
src/
├── main.rs              # Entry point
├── lib.rs               # Crate root, module declarations
├── config.rs            # TOML configuration loading
├── app/                 # Application lifecycle & event loop
│   ├── mod.rs           # App struct, ApplicationHandler impl, run()
│   ├── state.rs         # AppState — all mutable application state
│   ├── render.rs        # about_to_wait — event-driven render loop
│   ├── app_events.rs    # AppEvent routing (user events from proxy)
│   ├── commands.rs      # UI action dispatch
│   ├── keybinds.rs      # Keyboard shortcut processing
│   ├── cmd/             # UI action command handlers
│   │   ├── channel.rs   # Channel commands
│   │   ├── onboard.rs   # Onboarding commands
│   │   ├── scheduler.rs # Task scheduler commands
│   │   ├── settings.rs  # Settings commands
│   │   ├── split.rs     # Pane split commands
│   │   └── webview.rs   # Webview commands
│   ├── events/          # Application event handlers
│   │   ├── channel.rs   # Channel events
│   │   ├── llm.rs       # LLM response events
│   │   ├── misc.rs      # Miscellaneous events
│   │   └── onboard.rs   # Onboarding events
│   ├── draw/            # Frame rendering
│   │   ├── mod.rs       # Main redraw orchestration
│   │   ├── overlays.rs  # Overlay rendering (settings, tooltips)
│   │   ├── post.rs      # Post-frame processing
│   │   ├── term.rs      # Terminal pane rendering (quads + text)
│   │   └── webviews.rs  # Webview overlay positioning
│   ├── lifecycle/       # Application lifecycle
│   │   ├── mod.rs       # Window creation, GPU init, handle_resumed
│   │   ├── init.rs      # Extended initialization
│   │   └── llm.rs       # LLM provider auto-detection
│   └── window/          # Window event handling
│       ├── events.rs    # WindowEvent handling (input, resize, etc.)
│       └── misc.rs      # Resize, focus, theme, scale changes
├── ai/                  # AI integration
│   ├── client.rs        # LLM API client (async-openai)
│   ├── chat.rs          # Chat session management
│   ├── agent.rs         # Tool-use agent execution
│   ├── context.rs       # Context window management
│   ├── db.rs            # SQLite conversation storage
│   └── stream.rs        # AppEvent enum, streaming response types
├── renderer/            # GPU rendering pipeline
│   ├── mod.rs           # Renderer struct, frame orchestration
│   ├── scheduler.rs     # RenderSchedule trait, FrameScheduler impl
│   ├── atlas.rs         # Glyph atlas texture management
│   ├── cursor.rs        # Terminal cursor shapes & rendering
│   ├── quad.rs          # Quad vertex buffer & draw calls
│   ├── text.rs          # glyphon text buffer construction
│   └── platform/        # Platform-specific GPU setup
├── terminal/            # PTY session management
│   └── mod.rs           # TerminalSession — spawn, read, write, resize
├── ui/                  # UI widget system
│   ├── mod.rs           # PanelWidget trait, draw_all orchestration
│   ├── types.rs         # Shared UI types and state structs
│   ├── styles/          # Theme, colors, text cursor styles
│   ├── tiles/           # Pane layout and tile management
│   ├── settings/        # Settings panels (general, providers, etc.)
│   └── ...              # Chat panel, completion, titlebar, etc.
├── webview/             # Embedded browser
│   ├── mod.rs           # WebviewPanel lifecycle
│   └── manager.rs       # Multi-webview management
├── channel/             # Messaging channel integration
├── service/             # Platform services
├── sandbox/             # Sandboxing and security policies
├── secret/              # Credential management (OS keychain)
└── profile/             # User profile storage
```

## Widget Trait Hierarchy

All UI panels implement the `PanelWidget` trait, enabling uniform rendering orchestration:

```rust
pub trait PanelWidget {
    fn name(&self) -> &str;
    fn draw(&mut self, ctx: &mut WidgetCtx<'_>) -> WidgetResponse;
}
```

**`WidgetCtx`** bundles references to egui context, application state, and configuration needed by widgets during drawing.

**`WidgetResponse`** carries signals back from widgets to the orchestrator (e.g., request focus, trigger navigation).

The `draw_all()` function in `src/ui/mod.rs` iterates over all active panels, calling `draw()` on each and collecting responses for post-draw processing.

## Renderer Pipeline

1. **Atlas management** — Glyphs are rasterized via glyphon and cached in a GPU texture atlas
2. **Quad generation** — Terminal cells, cursor, and selection are converted to colored quads
3. **Text rendering** — glyphon `TextBuffer` is built per-pane with terminal content
4. **GPU submission** — wgpu render pass composites quads and text in a single frame
5. **egui overlay** — egui UI (panels, settings, chat) is rendered on top via a custom wgpu egui renderer

## Event-Driven Render Model

The application uses an event-driven architecture instead of continuous polling:

```text
                    ┌─────────────┐
                    │  winit loop  │
                    │ Wait/WaitUntil│
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
        WindowEvent   UserEvent    about_to_wait
        (input, resize) (PTY data,  (render decision)
                        repaint)
                                        │
                              ┌─────────┴─────────┐
                              │  FrameScheduler    │
                              │  .poll(now)        │
                              │  dirty flag +      │
                              │  timer deadlines   │
                              └─────────┬─────────┘
                                        │
                              request_redraw() or Wait
```

Key design decisions:

- **ControlFlow::Wait** — The event loop sleeps until an event arrives, achieving < 3% idle CPU
- **RenderSchedule trait** — Abstracts frame scheduling behind `mark_dirty()`, `schedule_at()`, and `poll()` methods. The `FrameScheduler` implementation handles frame coalescing (~250fps cap via 4ms minimum interval)
- **EventLoopProxy** — PTY reader threads and egui repaint callbacks wake the main thread via proxy events
- **Timer scheduling** — Cursor blink (530ms) and deferred egui repaints use `schedule_at()` to set `WaitUntil` deadlines
