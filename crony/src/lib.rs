//! `crony` — CEF / host integration wrapper for the cronymax runtime.
//!
//! `crony` is the **only** crate in the workspace allowed to expose a
//! C FFI surface or to depend on CEF-shaped concerns. Its job is to:
//!
//!   * Own the standalone runtime process lifecycle (spawn, supervise,
//!     restart on crash, propagate shutdown).
//!   * Construct a `cronymax::RuntimeConfig` from host inputs.
//!   * Boot the in-process runtime when running as the standalone
//!     binary (`bin/cronymax-runtime`).
//!   * Bridge GIPS messages between the C++ host and `cronymax` (wired
//!     in tasks 2.x).
//!
//! Concrete capability implementations (`LocalFilesystem`, `LocalShell`,
//! `classify_command`, `RiskLevel`) live in `cronymax::capability` so
//! they are available from any Rust host without a CEF dependency.
//!
//! Tasks 1.3 / 1.4 only require the lifecycle scaffold and config
//! contract; richer surfaces land in later task groups.

#![deny(missing_debug_implementations)]
// FFI surfaces below are intentionally `unsafe`.
#![allow(clippy::missing_safety_doc)]

pub mod boundary;
pub mod ffi;
pub mod lifecycle;
pub mod logging;
pub mod supervisor;

pub use cronymax;

/// Crony crate version surfaced for diagnostics.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Re-export of the C ABI version constant for Rust callers (e.g. the
/// integration test that exercises the `crony_client_*` surface).
pub use ffi::CRONY_ABI_VERSION;
