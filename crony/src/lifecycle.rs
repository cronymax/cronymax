//! In-process lifecycle helpers used by both the FFI surface and the
//! standalone `cronymax-runtime` binary.

use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};

use cronymax::{Runtime, RuntimeConfig, RuntimeHandle};

/// Shared handle to a started runtime. `None` until `boot` runs.
static RUNTIME: OnceLock<Arc<RuntimeBundle>> = OnceLock::new();

#[derive(Debug)]
pub struct RuntimeBundle {
    pub runtime: Runtime,
    pub handle: RuntimeHandle,
}

/// Boot the runtime in-process. Idempotent: subsequent calls return the
/// existing bundle without re-initialising.
pub fn boot(config: RuntimeConfig) -> Result<Arc<RuntimeBundle>> {
    if let Some(existing) = RUNTIME.get() {
        return Ok(Arc::clone(existing));
    }
    let runtime = Runtime::new(config).context("constructing cronymax runtime")?;
    let handle = runtime.start().context("starting cronymax runtime")?;
    let bundle = Arc::new(RuntimeBundle { runtime, handle });
    // Ignore the race-loser case; both bundles are equivalent.
    let _ = RUNTIME.set(Arc::clone(&bundle));
    Ok(RUNTIME.get().cloned().unwrap_or(bundle))
}

/// Returns the running bundle if `boot` has succeeded.
pub fn current() -> Option<Arc<RuntimeBundle>> {
    RUNTIME.get().cloned()
}

/// Cleanly stop the runtime. Safe to call without a prior `boot`.
pub fn shutdown() {
    if let Some(bundle) = RUNTIME.get() {
        bundle.runtime.shutdown();
    }
}

/// Liveness probe used by host-side health checks (task 1.3).
pub fn is_healthy() -> bool {
    RUNTIME
        .get()
        .map(|bundle| bundle.runtime.is_running())
        .unwrap_or(false)
}
