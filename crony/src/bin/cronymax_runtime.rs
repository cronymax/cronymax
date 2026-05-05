//! Standalone runtime binary.
//!
//! Reads a JSON-encoded `cronymax::RuntimeConfig` from stdin, boots the
//! runtime, binds the GIPS service so the CEF host can connect, and waits
//! for SIGINT/SIGTERM before shutting down.

use std::io::{self, Read};

use anyhow::{Context, Result};
use crony::boundary::GipsTransport;
use cronymax::RuntimeConfig;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .context("reading runtime config from stdin")?;

    let config: RuntimeConfig =
        serde_json::from_str(&buf).context("parsing runtime config json")?;

    crony::logging::install(config.logging.filter.as_deref());

    let bundle = crony::lifecycle::boot(config).context("booting runtime")?;

    // Bind the GIPS service so the CEF host can connect. This registers the
    // Mach bootstrap service (macOS) / named socket (Linux) that the host
    // discovers via crony_client_new("ai.cronymax.runtime", ...).
    let transport = GipsTransport::bind_default().context("binding GIPS transport")?;

    // Attach the transport to the runtime authority. The returned JoinHandle
    // drives the dispatch loop for the lifetime of this variable.
    let _session = bundle.runtime.attach_transport(transport);

    tracing::info!(
        version = crony::CRATE_VERSION,
        protocol = %cronymax::PROTOCOL_VERSION,
        "cronymax-runtime up; awaiting shutdown signal"
    );

    wait_for_shutdown().await;

    crony::lifecycle::shutdown();
    tracing::info!("cronymax-runtime exited cleanly");
    Ok(())
}

#[cfg(unix)]
async fn wait_for_shutdown() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = sigint.recv() => tracing::info!("SIGINT received"),
        _ = sigterm.recv() => tracing::info!("SIGTERM received"),
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("ctrl-c received");
}
