//! `cronygraph` — multi-agent orchestration primitives.
//!
//! This crate is intentionally **business-less** and **host-less**:
//!   * It must not depend on `crony` or any CEF / FFI surface.
//!   * It must not encode product policy (cronymax-specific agents,
//!     LLM providers, persistence layout, permission semantics, etc.).
//!   * It exposes graph and orchestration primitives that `cronymax`
//!     composes into the runtime authority.
//!
//! Only scaffolding lives here today. Concrete primitives are added in
//! the orchestration core tasks (3.x) of `rust-runtime-migration`.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod graph;
pub mod orchestration;

/// Crate version surfaced for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
