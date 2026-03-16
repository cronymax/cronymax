// ── Architecture Layers (bottom → top) ───────────────────────────────────────
//
// 6. AI/LLM       – Provider-agnostic streaming completions, agent runtime
// 5. Services      – Cross-session data (Memory, ScheduleTask, Block)
// 4. Channels      – Messaging platform bridges (Lark, Telegram, etc.)
// 3. UI            – In-app widgets (chat, browser, terminal, channel, settings)
// 2. Sessions      – Typed session data models (Chat, Browser, Terminal, Channel)
// 1. Profile       – Sandbox, Permissions, session lifecycle, profile CRUD
// 0. App           – Lifecycle, rendering, window management, daemon
//
// Cross-cutting: config, renderer (GPU + terminal PTY + webview)
//
// Dependency rule: each layer may depend on layers below it, never above.

pub mod ai;         // 6. AI / LLM providers + agent runtime
pub mod services;   // 5. Cross-session services (Memory, ScheduleTask, Block, Secret)
pub mod channels;   // 4. Messaging platform bridges
pub mod ui;         // 3. UI widget layer (+ overlay)
pub mod sessions;   // 2. Session data models
pub mod profile;    // 1. Profile, sandbox, permissions
pub mod app;        // 0. App lifecycle & rendering

// Cross-cutting modules
pub mod config;     // Application configuration
pub mod renderer;   // GPU/wgpu rendering, terminal PTY, webview management
