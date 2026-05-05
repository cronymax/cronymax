//! Standalone-process supervision scaffold.
//!
//! Today this only documents the intended responsibilities. Real spawn
//! / restart logic lands once GIPS transport is in place (tasks 2.x)
//! and the host knows how to talk to the runtime over a socket.
//!
//! Responsibilities:
//!   * Spawn the `cronymax-runtime` binary alongside the CEF host.
//!   * Hand it a serialized `RuntimeConfig` over its stdin / a pre-
//!     opened GIPS endpoint.
//!   * Watch for unexpected exits and surface them through a host
//!     callback so the UI can show a reconnect / retry banner.
//!   * Drive an orderly shutdown when the host exits.
//!
//! Until that lands, callers use the in-process `lifecycle::boot` path
//! so the runtime can be exercised from C++ today without yet wiring
//! GIPS.

use std::fmt;

#[derive(Debug, Default)]
pub struct Supervisor {
    _private: (),
}

impl Supervisor {
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Display for Supervisor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("crony::Supervisor(scaffold)")
    }
}
