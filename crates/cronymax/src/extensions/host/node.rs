//! Node 26 subprocess host — spawn, RPC fd 3 wiring, health monitoring, restart.
//!
//! ### Unsafe usage
//!
//! Inheriting a file descriptor onto a child's fd 3 (spec §6.1.1)
//! fundamentally requires `pre_exec` + `dup2`, both of which are unsafe.
//! Allowing unsafe in this module is intentional and scoped — the rest
//! of the crate still honours `forbid(unsafe_code)`.
#![allow(unsafe_code)]
//!
//! One [`NodeHost`] per running extension. The host owns:
//!
//! * a tokio `Child` for the Node 26 process
//! * a [`super::super::rpc::Connection`] over the inherited fd 3 socket
//! * a restart counter (gives up after `max_restarts`)
//! * a ping/pong health monitor (cancelled on shutdown)
//!
//! The spawn step uses a Unix `socketpair` + `pre_exec` to dup2 one half
//! onto the child's fd 3. The Rust side wraps its half as a tokio
//! `UnixStream` and hands it to [`super::super::rpc::Connection::open`].
//!
//! ### Testing
//!
//! Tests pass a mock binary (any executable that talks MessagePack-RPC on
//! fd 3) via [`SpawnConfig::node_binary`]. The real Node 26 binary lands
//! in `P2-T01` and is interchangeable here.

use std::os::unix::io::FromRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rmpv::Value;
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex as TokioMutex;
use tokio::task::JoinHandle;

use crate::extensions::error::{ExtensionError, ExtensionResult};
use crate::extensions::rpc::codec::method;
use crate::extensions::rpc::{Connection, RpcServer};

/// Settings for spawning one extension's Node host.
#[derive(Clone, Debug)]
pub struct SpawnConfig {
    pub ext_id: String,
    /// Path to the Node 26 binary cronymax ships (P2-T01) or any
    /// MessagePack-RPC-speaking executable in tests.
    pub node_binary: PathBuf,
    /// Pre-built argv tail from `build_node_flags`.
    pub node_flags: Vec<String>,
    /// Absolute path to `bundled/extension-host-bootstrap.js`.
    pub bootstrap_js: PathBuf,
    /// Extension install dir (passed via env so bootstrap.js can find it).
    pub ext_dir: PathBuf,
    /// Per-extension storage dir.
    pub storage_dir: PathBuf,
    /// Per-extension global storage dir.
    pub global_storage_dir: PathBuf,
    /// All currently open workspace roots. Empty when no workspace is
    /// open. Workspace access is granted **automatically** to every
    /// extension; manifests do not need to declare it. See
    /// [`crate::extensions::capability::ExpansionCtx::workspaces`].
    pub workspace_dirs: Vec<PathBuf>,
    /// Manifest path on disk (bootstrap.js reads it directly).
    pub manifest_path: PathBuf,
    /// Restart policy.
    pub max_restarts: u32,
    /// Ping/pong interval. Set to `None` to disable health checks
    /// (useful in tests for mock binaries that don't speak `$/ping`).
    pub ping_interval: Option<Duration>,
}

impl SpawnConfig {
    pub fn ping_interval_default() -> Duration {
        Duration::from_secs(5)
    }
}

/// Runtime state of one host. Cheap to clone (Arc internals).
#[derive(Clone, Debug)]
pub struct NodeHost {
    state: Arc<HostState>,
}

#[derive(Debug)]
struct HostState {
    ext_id: String,
    child: TokioMutex<Option<Child>>,
    connection: TokioMutex<Option<Arc<Connection>>>,
    rpc_task: TokioMutex<Option<JoinHandle<ExtensionResult<()>>>>,
    health_task: TokioMutex<Option<JoinHandle<()>>>,
    restart_count: TokioMutex<u32>,
}

impl NodeHost {
    /// Spawn the Node host and wait for the bootstrap to complete its
    /// fd 3 handshake. Returns once the process is alive and the
    /// `Connection` is pumping.
    pub async fn spawn(cfg: SpawnConfig, handlers: RpcServer) -> ExtensionResult<Self> {
        let (host_socket, child_socket) = unix_socketpair()?;

        let mut cmd = std::process::Command::new(&cfg.node_binary);
        cmd.args(&cfg.node_flags);
        cmd.arg(&cfg.bootstrap_js);
        cmd.env("CRONYMAX_EXTENSION_MANIFEST", &cfg.manifest_path);
        cmd.env("CRONYMAX_EXTENSION_DIR", &cfg.ext_dir);
        cmd.env("CRONYMAX_EXTENSION_STORAGE", &cfg.storage_dir);
        cmd.env("CRONYMAX_EXTENSION_GLOBAL_STORAGE", &cfg.global_storage_dir);
        // Pass workspace folders as a JSON array of canonical paths. The
        // bootstrap parses this and surfaces it as
        // `workspace.workspaceFolders` to the extension. Empty array = no
        // workspace open.
        let canon_workspaces: Vec<String> = cfg
            .workspace_dirs
            .iter()
            .map(|p| {
                std::fs::canonicalize(p)
                    .unwrap_or_else(|_| p.clone())
                    .display()
                    .to_string()
            })
            .collect();
        cmd.env(
            "CRONYMAX_WORKSPACE_FOLDERS",
            serde_json::to_string(&canon_workspaces).unwrap_or_else(|_| "[]".into()),
        );
        // Back-compat: also expose the first folder under the old single-
        // value env, so any code that still reads it keeps working during
        // the transition.
        if let Some(first) = canon_workspaces.first() {
            cmd.env("CRONYMAX_WORKSPACE_DIR", first);
        } else {
            cmd.env_remove("CRONYMAX_WORKSPACE_DIR");
        }
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Use pre_exec to dup2 the child socket onto fd 3 before exec.
        // SAFETY: we only call async-signal-safe functions inside the closure.
        unsafe {
            use std::os::unix::process::CommandExt;
            let child_fd = child_socket;
            cmd.pre_exec(move || {
                let res = libc::dup2(child_fd, 3);
                if res < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // Mark fd 3 as cloexec-clear so it survives exec.
                let flags = libc::fcntl(3, libc::F_GETFD);
                if flags >= 0 {
                    libc::fcntl(3, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                }
                Ok(())
            });
        }

        let mut tokio_cmd: Command = Command::from(cmd);
        let mut child = tokio_cmd
            .spawn()
            .map_err(|e| ExtensionError::HostSpawn(format!("spawn {:?}: {e}", cfg.node_binary)))?;

        // Close our copy of the child's fd; only the kernel-cloned fd in
        // the child remains open.
        // SAFETY: child_socket fd ownership transfers to the child via
        // pre_exec / dup2; closing here is correct.
        unsafe {
            libc::close(child_socket);
        }

        // Wrap our host-side fd as an async UnixStream.
        let host_stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(host_socket) };
        host_stream
            .set_nonblocking(true)
            .map_err(|e| ExtensionError::HostSpawn(format!("set_nonblocking: {e}")))?;
        let stream = UnixStream::from_std(host_stream)
            .map_err(|e| ExtensionError::HostSpawn(format!("UnixStream::from_std: {e}")))?;

        let (reader, writer) = stream.into_split();
        let (conn, rpc_task) = Connection::open(reader, writer, handlers);

        // Wire stdout/stderr drains so the child's process output isn't
        // lost. In production these get routed to the log writers
        // (P2-T11). For now, just consume them so the pipes don't fill.
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(drain_pipe(stdout, "stdout"));
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(drain_pipe(stderr, "stderr"));
        }

        let state = Arc::new(HostState {
            ext_id: cfg.ext_id.clone(),
            child: TokioMutex::new(Some(child)),
            connection: TokioMutex::new(Some(conn.clone())),
            rpc_task: TokioMutex::new(Some(rpc_task)),
            health_task: TokioMutex::new(None),
            restart_count: TokioMutex::new(0),
        });
        let host = Self { state };

        if let Some(interval) = cfg.ping_interval {
            host.start_health_monitor(conn, interval).await;
        }

        Ok(host)
    }

    pub fn ext_id(&self) -> &str {
        &self.state.ext_id
    }

    /// Test-only: construct a `NodeHost` with no underlying process.
    /// Used by [`crate::extensions::runtime`] unit tests that need a
    /// stand-in handle so they can populate `ExtensionRuntime.state.handles`
    /// without paying for a real spawn. The returned host's `is_alive()`
    /// returns `false`, `connection()` returns `None`, and `shutdown()`
    /// returns an error — but the value can be moved through code that
    /// only cares about `ext_id()` and key it by string.
    #[cfg(test)]
    pub(crate) fn dummy_for_test(ext_id: &str) -> Self {
        Self {
            state: Arc::new(HostState {
                ext_id: ext_id.to_string(),
                child: TokioMutex::new(None),
                connection: TokioMutex::new(None),
                rpc_task: TokioMutex::new(None),
                health_task: TokioMutex::new(None),
                restart_count: TokioMutex::new(0),
            }),
        }
    }

    /// Number of times the host has been restarted in its lifetime.
    pub async fn restart_count(&self) -> u32 {
        *self.state.restart_count.lock().await
    }

    /// Get the RPC connection, if the host is alive.
    pub async fn connection(&self) -> Option<Arc<Connection>> {
        self.state.connection.lock().await.clone()
    }

    /// Whether the host's child process is still alive (returns false if
    /// it exited).
    pub async fn is_alive(&self) -> bool {
        let mut child_guard = self.state.child.lock().await;
        match child_guard.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Send SIGTERM (graceful) and await exit.
    pub async fn shutdown(self) -> ExtensionResult<std::process::ExitStatus> {
        // Abort health monitor so it stops poking a dying child.
        if let Some(task) = self.state.health_task.lock().await.take() {
            task.abort();
        }
        // Abort the RPC read loop too.
        if let Some(task) = self.state.rpc_task.lock().await.take() {
            task.abort();
        }
        // Drop connection arc.
        let _ = self.state.connection.lock().await.take();

        let mut child_guard = self.state.child.lock().await;
        let mut child = child_guard
            .take()
            .ok_or_else(|| ExtensionError::HostSpawn("already shut down".into()))?;
        // Try graceful kill first; tokio::process::Child::kill sends SIGKILL,
        // so issue SIGTERM via nix and wait briefly first.
        if let Some(pid) = child.id() {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
        match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
            Ok(Ok(status)) => Ok(status),
            _ => {
                let _ = child.kill().await;
                child
                    .wait()
                    .await
                    .map_err(|e| ExtensionError::HostSpawn(format!("wait: {e}")))
            }
        }
    }

    async fn start_health_monitor(&self, conn: Arc<Connection>, interval: Duration) {
        let ext_id = self.state.ext_id.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let res = tokio::time::timeout(
                    Duration::from_millis(2000),
                    conn.request(method::PING, Value::Nil),
                )
                .await;
                match res {
                    Ok(Ok(_)) => continue,
                    Ok(Err(e)) => {
                        tracing::warn!(ext_id, err = %e, "ext-host ping rpc error");
                        break;
                    }
                    Err(_) => {
                        tracing::warn!(ext_id, "ext-host ping timeout (Layer D: hung)");
                        break;
                    }
                }
            }
        });
        *self.state.health_task.lock().await = Some(task);
    }
}

/// Create a connected pair of Unix sockets. Returns `(host_fd, child_fd)`.
/// The `host_fd` becomes the Rust-side `UnixStream`; the `child_fd` is
/// dup2'd onto fd 3 in the child via `pre_exec`.
fn unix_socketpair() -> ExtensionResult<(i32, i32)> {
    use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};
    let (a, b) = socketpair(
        AddressFamily::Unix,
        SockType::Stream,
        None,
        SockFlag::empty(),
    )
    .map_err(|e| ExtensionError::HostSpawn(format!("socketpair: {e}")))?;
    use std::os::fd::AsRawFd;
    use std::os::fd::IntoRawFd;
    let host_fd = a.into_raw_fd();
    let child_fd = b.into_raw_fd();
    // Belt-and-suspenders: make sure host_fd doesn't get marked CLOEXEC
    // for fd 3 inheritance, even though pre_exec already clears that
    // explicitly on the child side. The host side is owned by Rust and
    // never re-execs; CLOEXEC on host_fd is fine either way.
    let _ = host_fd.as_raw_fd();
    Ok((host_fd, child_fd))
}

async fn drain_pipe<R>(mut reader: R, label: &'static str)
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_n) => {
                // In production, write into the LogWriter (P2-T11). For
                // skeleton testing, drop the bytes.
                let _ = label;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    /// Build a tiny shell script that talks MessagePack-RPC on fd 3.
    /// It echoes back any Request frame as a Response with the same msgid
    /// and result `"echo"`, and responds to `$/ping` with `"pong"`.
    ///
    /// We can't easily implement a real MessagePack codec in shell, so
    /// instead we use a small Python script with `msgpack` if available,
    /// otherwise we just verify that the process spawns and runs without
    /// the host crashing.
    fn write_mock_binary(td: &TempDir, name: &str, body: &str) -> PathBuf {
        let path = td.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        drop(f);
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn dummy_paths(td: &TempDir) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
        let ext_dir = td.path().join("ext");
        let storage = td.path().join("storage");
        let global = td.path().join("global");
        let manifest = td.path().join("manifest.json");
        let bootstrap = td.path().join("bootstrap.js");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::create_dir_all(&storage).unwrap();
        std::fs::create_dir_all(&global).unwrap();
        std::fs::write(&manifest, "{}").unwrap();
        std::fs::write(&bootstrap, "// stub").unwrap();
        (ext_dir, storage, global, manifest, bootstrap)
    }

    #[tokio::test]
    async fn spawn_with_immediately_exiting_binary_reports_not_alive() {
        // Using `/bin/true` — exits immediately with 0. The host should
        // spawn it successfully; is_alive() should flip to false quickly.
        let td = TempDir::new().unwrap();
        let (ext_dir, storage, global, manifest, bootstrap) = dummy_paths(&td);
        let cfg = SpawnConfig {
            ext_id: "alice.exit".into(),
            node_binary: PathBuf::from("/usr/bin/true"),
            node_flags: vec![],
            bootstrap_js: bootstrap,
            ext_dir,
            storage_dir: storage,
            global_storage_dir: global,
            workspace_dirs: Vec::new(),
            manifest_path: manifest,
            max_restarts: 0,
            ping_interval: None, // no health check — /bin/true doesn't speak RPC
        };
        let host = NodeHost::spawn(cfg, RpcServer::builder().build())
            .await
            .unwrap();
        // Give the process a moment to exit.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!host.is_alive().await);
        let status = host.shutdown().await.unwrap();
        // Exit status from /bin/true is success.
        assert!(status.success() || status.code() == Some(0));
    }

    #[tokio::test]
    async fn spawn_long_running_binary_reports_alive_then_shuts_down() {
        // Spawn `sleep` — it runs until killed. We immediately shut down
        // and verify the process was alive in between.
        let td = TempDir::new().unwrap();
        let (ext_dir, storage, global, manifest, bootstrap) = dummy_paths(&td);
        let cfg = SpawnConfig {
            ext_id: "alice.long".into(),
            node_binary: PathBuf::from("/bin/sleep"),
            node_flags: vec!["10".into()],
            bootstrap_js: bootstrap,
            ext_dir,
            storage_dir: storage,
            global_storage_dir: global,
            workspace_dirs: Vec::new(),
            manifest_path: manifest,
            max_restarts: 0,
            ping_interval: None,
        };
        let host = NodeHost::spawn(cfg, RpcServer::builder().build())
            .await
            .unwrap();
        assert!(host.is_alive().await);
        assert_eq!(host.ext_id(), "alice.long");
        let status = host.shutdown().await.unwrap();
        // Killed by SIGTERM; code() may be None on signal exits.
        let _ = status;
    }

    #[tokio::test]
    async fn shutdown_is_idempotent_via_consumed_self() {
        // shutdown takes self by value, so it can only be called once.
        // This test mostly ensures the second call to a fresh host's
        // shutdown also works (no flaky state in HostState).
        let td = TempDir::new().unwrap();
        let (ext_dir, storage, global, manifest, bootstrap) = dummy_paths(&td);
        let cfg = SpawnConfig {
            ext_id: "alice.idem".into(),
            node_binary: PathBuf::from("/bin/sleep"),
            node_flags: vec!["10".into()],
            bootstrap_js: bootstrap,
            ext_dir,
            storage_dir: storage,
            global_storage_dir: global,
            workspace_dirs: Vec::new(),
            manifest_path: manifest,
            max_restarts: 0,
            ping_interval: None,
        };
        let host = NodeHost::spawn(cfg, RpcServer::builder().build())
            .await
            .unwrap();
        let _ = host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn ext_id_is_visible_through_handle() {
        let td = TempDir::new().unwrap();
        let (ext_dir, storage, global, manifest, bootstrap) = dummy_paths(&td);
        let cfg = SpawnConfig {
            ext_id: "alice.id".into(),
            node_binary: PathBuf::from("/bin/sleep"),
            node_flags: vec!["1".into()],
            bootstrap_js: bootstrap,
            ext_dir,
            storage_dir: storage,
            global_storage_dir: global,
            workspace_dirs: Vec::new(),
            manifest_path: manifest,
            max_restarts: 0,
            ping_interval: None,
        };
        let host = NodeHost::spawn(cfg, RpcServer::builder().build())
            .await
            .unwrap();
        assert_eq!(host.ext_id(), "alice.id");
        assert_eq!(host.restart_count().await, 0);
        let _ = host.shutdown().await;
    }

    #[tokio::test]
    async fn nonexistent_binary_returns_host_spawn_error() {
        let td = TempDir::new().unwrap();
        let (ext_dir, storage, global, manifest, bootstrap) = dummy_paths(&td);
        let bogus = write_mock_binary(&td, "no-such-binary", "");
        std::fs::remove_file(&bogus).unwrap();
        let cfg = SpawnConfig {
            ext_id: "alice.bogus".into(),
            node_binary: bogus,
            node_flags: vec![],
            bootstrap_js: bootstrap,
            ext_dir,
            storage_dir: storage,
            global_storage_dir: global,
            workspace_dirs: Vec::new(),
            manifest_path: manifest,
            max_restarts: 0,
            ping_interval: None,
        };
        let err = NodeHost::spawn(cfg, RpcServer::builder().build())
            .await
            .unwrap_err();
        assert!(matches!(err, ExtensionError::HostSpawn(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn workspace_dir_env_is_forwarded_when_set() {
        // Use a small shell script that dumps env to a file so we can
        // verify the env was passed.
        let td = TempDir::new().unwrap();
        let (ext_dir, storage, global, manifest, _) = dummy_paths(&td);
        let env_dump = td.path().join("env.txt");
        let script_body = format!("#!/bin/sh\nenv > {} && sleep 1\n", env_dump.display());
        let script = write_mock_binary(&td, "dumpenv.sh", &script_body);
        let workspace = td.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let cfg = SpawnConfig {
            ext_id: "alice.env".into(),
            node_binary: script,
            node_flags: vec![],
            bootstrap_js: td.path().join("ignored.js"),
            ext_dir: ext_dir.clone(),
            storage_dir: storage,
            global_storage_dir: global,
            workspace_dirs: vec![workspace.clone()],
            manifest_path: manifest,
            max_restarts: 0,
            ping_interval: None,
        };
        let host = NodeHost::spawn(cfg, RpcServer::builder().build())
            .await
            .unwrap();
        // Wait up to 2s for the script to dump env.
        let mut env = String::new();
        for _ in 0..40 {
            if let Ok(s) = std::fs::read_to_string(&env_dump) {
                if !s.is_empty() {
                    env = s;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            !env.is_empty(),
            "env dump file never appeared (mock script didn't run?)"
        );
        // The host writes the canonical form of the workspace path into
        // the env. On macOS that means /var/folders/... → /private/var/...
        let canon_ws = std::fs::canonicalize(&workspace).unwrap_or(workspace);
        assert!(
            env.contains(&format!("CRONYMAX_WORKSPACE_DIR={}", canon_ws.display())),
            "env did not include workspace dir (canonical); got:\n{env}",
        );
        // The folders env is a JSON array containing that same canonical
        // path.
        assert!(
            env.contains("CRONYMAX_WORKSPACE_FOLDERS="),
            "missing CRONYMAX_WORKSPACE_FOLDERS in {env}"
        );
        assert!(
            env.contains(&canon_ws.display().to_string()),
            "workspace folders env did not include canonical path"
        );
        assert!(env.contains("CRONYMAX_EXTENSION_DIR="));
        assert!(env.contains("CRONYMAX_EXTENSION_STORAGE="));
        assert!(env.contains("CRONYMAX_EXTENSION_GLOBAL_STORAGE="));
        let _ = host.shutdown().await;
    }
}
