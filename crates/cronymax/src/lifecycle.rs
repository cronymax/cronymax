//! Runtime lifecycle scaffold.
//!
//! Real run/agent/permission state lands in tasks 4.x. This module
//! exposes the minimum surface `crony` needs in tasks 1.3 / 1.4:
//!
//! * Construct a `Runtime` from a validated `RuntimeConfig`.
//! * Start it (idempotent — returns a `RuntimeHandle`).
//! * Shut it down cleanly.
//! * Check liveness for host-side health checks.

use std::sync::Arc;

use parking_lot::Mutex;
use thiserror::Error;
use tokio::task::JoinHandle;
use tracing::info;

use crate::config::RuntimeConfig;
use crate::protocol::dispatch::DispatchError;
use crate::protocol::session;
use crate::protocol::transport::Transport;
use crate::protocol::{ProtocolVersion, PROTOCOL_VERSION};
use crate::runtime::{JsonFilePersistence, RuntimeAuthority, RuntimeHandler};
use crate::sandbox::policy::SandboxPolicy;

/// Errors surfaced from runtime lifecycle operations.
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("protocol version mismatch: host={host}, runtime={runtime}")]
    ProtocolMismatch {
        host: ProtocolVersion,
        runtime: ProtocolVersion,
    },

    #[error("runtime already started")]
    AlreadyStarted,

    #[error("runtime is not running")]
    NotRunning,

    #[error("runtime initialization failed: {0}")]
    Init(String),
}

#[derive(Debug, Default)]
struct RuntimeState {
    started: bool,
}

/// Owned runtime instance. Constructed by `crony` once per process.
#[derive(Debug)]
pub struct Runtime {
    config: RuntimeConfig,
    state: Arc<Mutex<RuntimeState>>,
    authority: RuntimeAuthority,
    /// Shared PTY session managers, passed to every RuntimeHandler so that
    /// sessions created via the browser transport are visible to the renderer
    /// transport (and vice-versa).
    terminal_managers: Arc<
        parking_lot::Mutex<
            std::collections::HashMap<String, crate::terminal::SharedPtySessionManager>,
        >,
    >,
}

/// Handle returned from `Runtime::start`. Today it carries no resources
/// of its own; tasks 2.x give it a transport handle and tasks 4.x give
/// it a runtime authority handle.
#[derive(Clone, Debug)]
pub struct RuntimeHandle {
    state: Arc<Mutex<RuntimeState>>,
}

impl Runtime {
    /// Validate the supplied configuration and build a runtime instance.
    pub fn new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        if !PROTOCOL_VERSION.is_compatible_with(config.host_protocol) {
            return Err(RuntimeError::ProtocolMismatch {
                host: config.host_protocol,
                runtime: PROTOCOL_VERSION,
            });
        }
        // Build the persistence backend from the configured app data
        // dir and rehydrate the authority. Failure here is fatal: the
        // runtime cannot honour its authority contract without state.
        let persistence: Arc<dyn crate::runtime::Persistence> = Arc::new(
            JsonFilePersistence::under_app_data_dir(&config.storage.app_data_dir),
        );
        let authority = RuntimeAuthority::rehydrate(persistence)
            .map_err(|e| RuntimeError::Init(e.to_string()))?;
        Ok(Self {
            config,
            state: Arc::new(Mutex::new(RuntimeState::default())),
            authority,
            terminal_managers: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        })
    }

    /// Start the runtime. Idempotent within a single process — calling
    /// twice without `shutdown` returns `AlreadyStarted`.
    ///
    /// The transport / dispatch loops are wired up in task 2.3.
    pub fn start(&self) -> Result<RuntimeHandle, RuntimeError> {
        let mut state = self.state.lock();
        if state.started {
            return Err(RuntimeError::AlreadyStarted);
        }
        state.started = true;
        info!(
            protocol = %PROTOCOL_VERSION,
            app_data_dir = ?self.config.storage.app_data_dir,
            "cronymax runtime started"
        );
        Ok(RuntimeHandle {
            state: Arc::clone(&self.state),
        })
    }

    /// Cleanly stop the runtime. Safe to call multiple times.
    pub fn shutdown(&self) {
        let mut state = self.state.lock();
        if state.started {
            state.started = false;
            info!("cronymax runtime stopped");
        }
    }

    /// Liveness check used by host health probes.
    pub fn is_running(&self) -> bool {
        self.state.lock().started
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    /// Attach a [`Transport`] to the runtime and drive a dispatch
    /// session against it. Returns a join handle the caller can await
    /// for graceful exit.
    ///
    /// The session is wired to the [`RuntimeAuthority`] this `Runtime`
    /// owns, so control requests mutate authoritative state and event
    /// subscriptions stream real runtime events.
    pub fn attach_transport<T: Transport>(
        &self,
        transport: T,
    ) -> JoinHandle<Result<(), DispatchError>> {
        // Build sandbox policy from the optional `sandbox` section of
        // the RuntimeConfig (task 6.1).
        let sandbox_policy: Option<SandboxPolicy> = self.config.sandbox.as_ref().map(|sc| {
            let mut policy = SandboxPolicy::default_for_workspace(&sc.workspace_root);
            policy.set_allow_network(sc.allow_network);
            for p in &sc.extra_read_paths {
                policy.add_read_path(p);
            }
            for p in &sc.extra_write_paths {
                policy.add_write_path(p);
            }
            for p in &sc.extra_deny_paths {
                policy.add_deny_path(p);
            }
            policy
        });

        let handler = Arc::new(RuntimeHandler::with_policy_and_managers(
            self.authority.clone(),
            self.config.storage.workspace_roots.clone(),
            sandbox_policy,
            Some(Arc::clone(&self.terminal_managers)),
        ));
        session::spawn_session(transport, handler)
    }

    /// Borrow the runtime authority. `crony` uses this to seed initial
    /// Spaces / Agents from host configuration before connecting.
    pub fn authority(&self) -> &RuntimeAuthority {
        &self.authority
    }
}

impl RuntimeHandle {
    pub fn is_running(&self) -> bool {
        self.state.lock().started
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::{LogConfig, StoragePaths};

    fn cfg(host: ProtocolVersion) -> RuntimeConfig {
        RuntimeConfig {
            storage: StoragePaths {
                workspace_roots: vec![PathBuf::from("/tmp/ws")],
                app_data_dir: PathBuf::from("/tmp/app"),
                cache_dir: PathBuf::from("/tmp/cache"),
            },
            logging: LogConfig {
                log_dir: PathBuf::from("/tmp/log"),
                filter: None,
            },
            host_protocol: host,
            sandbox: None,
        }
    }

    #[test]
    fn protocol_mismatch_is_detected() {
        let bad = ProtocolVersion::new(PROTOCOL_VERSION.major.wrapping_add(1), 0, 0);
        let err = Runtime::new(cfg(bad)).unwrap_err();
        assert!(matches!(err, RuntimeError::ProtocolMismatch { .. }));
    }

    #[test]
    fn start_is_idempotent_only_after_shutdown() {
        let rt = Runtime::new(cfg(PROTOCOL_VERSION)).unwrap();
        let _h = rt.start().unwrap();
        assert!(rt.is_running());
        assert!(matches!(
            rt.start().unwrap_err(),
            RuntimeError::AlreadyStarted
        ));
        rt.shutdown();
        assert!(!rt.is_running());
        let _h2 = rt.start().unwrap();
        assert!(rt.is_running());
    }
}
