//! Tracing subscriber bootstrap shared by FFI and the standalone binary.

use std::sync::Once;

use tracing_subscriber::EnvFilter;

static INIT: Once = Once::new();

/// Install a global `tracing` subscriber. Idempotent — subsequent calls
/// are no-ops so the host can call this freely.
///
/// `filter` overrides `RUST_LOG` when `Some`. `None` falls back to
/// `RUST_LOG`, then to `info` for cronymax / crony crates.
pub fn install(filter: Option<&str>) {
    INIT.call_once(|| {
        let env = match filter {
            Some(directive) => EnvFilter::try_new(directive)
                .unwrap_or_else(|_| EnvFilter::new("info")),
            None => EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,cronymax=info,crony=info")),
        };
        let _ = tracing_subscriber::fmt()
            .with_env_filter(env)
            .with_target(true)
            .try_init();
    });
}
