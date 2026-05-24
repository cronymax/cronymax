//! Cronymax extension platform — Rust side.
//!
//! Implements the runtime that hosts third-party extensions described by the
//! IDL in [`cep-idl/v1`](./cep-idl/v1/). One extension == one Node 26
//! subprocess with `--permission` and a hand-picked set of `--allow-*` flags
//! derived from its `cronymax-extension.json` manifest.
//!
//! See `docs/extensions/spec-v0.3.md` for the design and
//! `docs/extensions/implementation-plan-v0.3.md` for the rollout.
//!
//! ### Module layout
//!
//! * [`manifest`] — `cronymax-extension.json` schema, validation,
//!   canonicalization. (Phase 1)
//! * [`registry`] — installed-extension index and enable/disable state.
//!   (Phase 1)
//! * [`activation`] — `activationEvents` matching engine. (Phase 1)
//! * [`capability`] — `Manifest → Node --allow-* flags` mapping; nothing
//!   else. Permission enforcement is Node-VM-only. (Phase 2)
//! * [`host`] — Node 26 subprocess lifecycle: spawn, health, restart,
//!   per-extension inherited fd 3 (see spec §6.1.1). (Phase 2)
//! * [`rpc`] — MessagePack-RPC server: request/response/notify with
//!   cancellation. (Phase 2)
//! * [`contributions`] — single registry that every L2 EP consumes. No
//!   per-EP handler files; new EPs register here. (Phase 4)
//! * [`events`] — L1.5 platform-event bus. (Phase 5)
//! * [`api`] — Rust implementations of the IDL-v1 API surface that
//!   extensions call into. (Phases 2–3)
//! * [`error`] — shared `ExtensionError` enum.
//!
//! ### Invariants (see plan §2 — every PR self-checks)
//!
//! 1. No specific extension id (`coco`, `mermaid`, etc.) appears anywhere
//!    under this module tree. Dogfood extensions live in their own crates.
//! 2. Every L2 EP goes through [`contributions`]. No per-EP handler module.
//! 3. Every L1 API call passes through Node's Permission Model. No internal
//!    fast paths that skip it.
//! 4. The `AgentProvider` surface is identical for the chat panel and the
//!    flow runtime — they consume the same registry.
//! 5. Before adding a built-in EP, try implementing as a pure SDK extension.

pub mod activation;
pub mod api;
pub mod capability;
pub mod contributions;
pub mod error;
pub mod events;
pub mod host;
pub mod logging;
pub mod manifest;
pub mod paths;
pub mod registry;
pub mod rpc;
pub mod runtime;

pub use error::ExtensionError;
pub use manifest::Manifest;
pub use paths::{default_bundled_bootstrap, default_bundled_dir, default_bundled_node};
pub use registry::{default_registry_root, ExtensionRegistry};
pub use runtime::ExtensionRuntime;
