//! Standalone runtime binary.
//!
//! Reads a JSON-encoded `cronymax::RuntimeConfig` from stdin, boots the
//! runtime, binds the GIPS service so the CEF host can connect, and waits
//! for SIGINT/SIGTERM before shutting down.

use std::io::{self, Read};

use anyhow::{Context, Result};
use crony::boundary::{GipsTransport, RENDERER_SERVICE_NAME};
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

    // Bind a second service dedicated to renderer-process clients (built-in
    // pages). Keeping a separate service avoids contention on the single
    // ReturnPath slot inside GipsTransport when both the browser process and
    // renderer process are connected simultaneously.
    let renderer_transport = GipsTransport::bind(RENDERER_SERVICE_NAME)
        .context("binding renderer GIPS transport")?;

    // Attach both transports to the shared RuntimeAuthority.  Each call
    // to attach_transport spawns an independent dispatch session; they share
    // the same subscription bus via the Arc<RuntimeAuthority>.
    let browser_session = bundle.runtime.attach_transport(transport);
    let _renderer_session = bundle.runtime.attach_transport(renderer_transport);

    tracing::info!(
        version = crony::CRATE_VERSION,
        protocol = %cronymax::PROTOCOL_VERSION,
        "crony up; awaiting shutdown signal"
    );

    // Race the shutdown signal against the browser transport session.
    // When the session exits (e.g. idle timeout → host assumed disconnected)
    // we exit immediately so the C++ supervisor can respawn a fresh crony.
    tokio::select! {
        _ = wait_for_shutdown() => {
            tracing::info!("shutdown signal received");
        }
        result = browser_session => {
            match result {
                Ok(Ok(())) => tracing::warn!("browser transport session ended cleanly; exiting"),
                Ok(Err(e)) => tracing::warn!("browser transport session error: {e}; exiting"),
                Err(e) => tracing::warn!("browser transport task panicked: {e}; exiting"),
            }
            // Force-exit immediately so background threads (PTY I/O, spawn_blocking
            // tasks, etc.) cannot delay process termination.  The C++ supervisor
            // detects the exit via waitpid and respawns a fresh crony.
            // stdout/stderr are flushed by libc's atexit handlers.
            std::process::exit(0);
        }
    }

    crony::lifecycle::shutdown();
    tracing::info!("crony exited cleanly");
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
