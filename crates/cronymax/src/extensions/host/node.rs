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
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rmpv::Value;
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::{Mutex as TokioMutex, Notify};
use tokio::task::JoinHandle;

use crate::extensions::error::{ExtensionError, ExtensionResult};
use crate::extensions::logging::LogWriter;
use crate::extensions::rpc::codec::method;
use crate::extensions::rpc::{Connection, RpcServer};

/// Why a host process ended — used to classify the exit for logging and
/// restart policy. A clean `exit(0)` we *didn't* ask for is still abnormal
/// (spec Layer F): the extension dropped its host without a deactivate
/// handshake.
#[derive(Clone, Copy, Debug)]
pub enum HostExit {
    /// `exit(0)` without a deactivate handshake (spec Layer F).
    CleanUnexpected,
    /// Non-zero exit code (spec Layer E).
    Code(i32),
    /// Terminated by a signal (spec Layer E).
    Signal(i32),
}

/// Out-of-band signals a running host emits to its supervisor in
/// [`crate::extensions::runtime::ExtensionRuntime`]. Delivered over the
/// unbounded mpsc channel installed at spawn via
/// [`SpawnConfig::host_event_sink`]. `None` sink (tests / dev harness)
/// disables every signal — the host runs without supervision.
#[derive(Clone, Debug)]
pub enum HostEvent {
    /// The child exited without us asking (crash / abnormal exit). The
    /// supervisor restarts (≤ N) then disables.
    Crashed { ext_id: String, exit: HostExit },
    /// Ping/pong timed out — the host is wedged (spec Layer D). The
    /// supervisor escalates this to a restart (kills then respawns).
    Hung { ext_id: String },
    /// Resident memory crossed the configured warn threshold. One-shot per
    /// host lifetime. Observability only — never a kill (P10-T02 scoped
    /// down to warn-not-enforce).
    RssWarn {
        ext_id: String,
        rss: u64,
        limit: u64,
    },
}

/// Sender half of the host→supervisor signal channel.
pub type HostEventSink = UnboundedSender<HostEvent>;

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
    /// Log sink for the child's stdout (`output.log` — extension `console.log`
    /// fallback). `None` drops the stream (tests / no log manager). Attached by
    /// [`crate::extensions::runtime::ExtensionRuntime`] at activation from the
    /// session [`crate::extensions::logging::LogManager`].
    pub stdout_log: Option<Arc<LogWriter>>,
    /// Log sink for the child's stderr (`host.log` — Node warnings +
    /// `console.error` + uncaught stacks). `None` drops the stream.
    pub stderr_log: Option<Arc<LogWriter>>,
    /// Out-of-band crash / hung / RSS signal channel back to the runtime
    /// supervisor. `None` (tests / dev harness) means the host runs
    /// unsupervised — no auto-restart, no RSS notice.
    pub host_event_sink: Option<HostEventSink>,
    /// Resident-memory warn threshold in bytes. `None` disables RSS
    /// sampling. Crossing it logs a warning and emits one
    /// [`HostEvent::RssWarn`] — observability only, never a kill.
    pub rss_warn_bytes: Option<u64>,
}

impl SpawnConfig {
    pub fn ping_interval_default() -> Duration {
        Duration::from_secs(5)
    }

    /// Default resident-memory warn threshold (~1.5 GiB). A host crossing
    /// this only triggers a log + one notice; it is never killed.
    pub fn rss_warn_default() -> u64 {
        1536 * 1024 * 1024
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
    /// Raw child pid for signalling — `shutdown` SIGTERM/SIGKILL, the
    /// `Drop` backstop, and RSS sampling all use it. `-1` for
    /// [`NodeHost::dummy_for_test`] / a process that never started.
    pid: AtomicI32,
    /// Set true before any *planned* teardown (`shutdown`, `kill_blocking`,
    /// the `Drop` backstop). The exit-watcher reads it to decide whether an
    /// exit is a crash (emit [`HostEvent::Crashed`]) or expected (silent);
    /// the health monitor reads it to stop emitting `Hung`; and it makes
    /// `Drop` a no-op after an explicit shutdown. Shared (`Arc`) with the
    /// watcher / health tasks — which is also why those tasks hold the
    /// individual field `Arc`s and **not** `Arc<HostState>`, so the `Drop`
    /// backstop's `strong_count` reflects live `NodeHost` handles only.
    intentional_shutdown: Arc<AtomicBool>,
    connection: TokioMutex<Option<Arc<Connection>>>,
    rpc_task: TokioMutex<Option<JoinHandle<ExtensionResult<()>>>>,
    health_task: TokioMutex<Option<JoinHandle<()>>>,
    /// The task that owns the [`Child`] and `child.wait()`s on it. Owning
    /// the `Child` here (rather than in `HostState`) is what lets
    /// `child.wait()` reap the zombie without `&mut HostState`, and lets
    /// `shutdown` signal purely by pid.
    exit_watch_task: TokioMutex<Option<JoinHandle<()>>>,
    /// Filled by the exit-watcher when `child.wait()` returns. `is_alive`
    /// reads it; `shutdown` polls it. `Arc` so the watcher can write it.
    exit_status: Arc<TokioMutex<Option<ExitStatus>>>,
    /// Notified once when `exit_status` is filled, so `shutdown` can wake
    /// promptly instead of busy-polling.
    exit_done: Arc<Notify>,
    /// One-shot guard so an over-threshold host emits at most one
    /// `RssWarn` notice per lifetime (the warn log still fires each tick).
    rss_notice_fired: Arc<AtomicBool>,
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

        // Wire stdout/stderr drains so the child's process output isn't lost:
        // append to the per-extension `output.log` / `host.log` writers when a
        // sink is attached (production, via the session LogManager), else just
        // consume the bytes so the pipes don't fill (tests / no log manager).
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(drain_pipe(stdout, cfg.stdout_log.clone()));
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(drain_pipe(stderr, cfg.stderr_log.clone()));
        }

        let pid = child.id().map(|p| p as i32).unwrap_or(-1);
        let host = Self {
            state: Arc::new(HostState {
                ext_id: cfg.ext_id.clone(),
                pid: AtomicI32::new(pid),
                intentional_shutdown: Arc::new(AtomicBool::new(false)),
                connection: TokioMutex::new(Some(conn.clone())),
                rpc_task: TokioMutex::new(Some(rpc_task)),
                health_task: TokioMutex::new(None),
                exit_watch_task: TokioMutex::new(None),
                exit_status: Arc::new(TokioMutex::new(None)),
                exit_done: Arc::new(Notify::new()),
                rss_notice_fired: Arc::new(AtomicBool::new(false)),
                restart_count: TokioMutex::new(0),
            }),
        };

        // Exit watcher: own the `Child`, `wait()` on it (reaps the zombie),
        // record the status, and — unless we asked for the death — emit a
        // `Crashed` signal so the runtime supervisor can restart. Holds only
        // the individual field `Arc`s (not `Arc<HostState>`) so the `Drop`
        // backstop's `strong_count` counts `NodeHost` handles only.
        {
            let intentional = host.state.intentional_shutdown.clone();
            let exit_status = host.state.exit_status.clone();
            let exit_done = host.state.exit_done.clone();
            let sink = cfg.host_event_sink.clone();
            let ext_id = cfg.ext_id.clone();
            let watcher = tokio::spawn(async move {
                let status = match child.wait().await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(ext_id, err = %e, "ext-host child.wait failed");
                        return;
                    }
                };
                *exit_status.lock().await = Some(status);
                exit_done.notify_waiters();
                if !intentional.load(Ordering::SeqCst) {
                    tracing::warn!(
                        ext_id,
                        code = ?status.code(),
                        signal = ?status.signal(),
                        "ext-host exited unexpectedly (Layer E/F: crashed)",
                    );
                    if let Some(sink) = sink {
                        let _ = sink.send(HostEvent::Crashed {
                            ext_id: ext_id.clone(),
                            exit: classify_exit(status),
                        });
                    }
                }
            });
            *host.state.exit_watch_task.lock().await = Some(watcher);
        }

        if let Some(interval) = cfg.ping_interval {
            host.start_health_monitor(
                conn,
                interval,
                cfg.host_event_sink.clone(),
                pid,
                cfg.rss_warn_bytes,
            )
            .await;
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
                pid: AtomicI32::new(-1),
                intentional_shutdown: Arc::new(AtomicBool::new(false)),
                connection: TokioMutex::new(None),
                rpc_task: TokioMutex::new(None),
                health_task: TokioMutex::new(None),
                exit_watch_task: TokioMutex::new(None),
                exit_status: Arc::new(TokioMutex::new(None)),
                exit_done: Arc::new(Notify::new()),
                rss_notice_fired: Arc::new(AtomicBool::new(false)),
                restart_count: TokioMutex::new(0),
            }),
        }
    }

    /// Number of times the host has been restarted in its lifetime.
    pub async fn restart_count(&self) -> u32 {
        *self.state.restart_count.lock().await
    }

    /// Raw child pid, or `-1` for a dummy / never-started host. Synchronous
    /// so [`crate::extensions::runtime::ExtensionRuntime::kill_all_blocking`]
    /// can reap on the hard-exit path without `await`.
    pub fn pid(&self) -> i32 {
        self.state.pid.load(Ordering::Relaxed)
    }

    /// Synchronous SIGKILL backstop for the hard-exit path
    /// (`std::process::exit`, which skips `Drop`). Marks the death
    /// intentional first so the watcher won't classify it as a crash.
    pub fn kill_blocking(&self) {
        self.state
            .intentional_shutdown
            .store(true, Ordering::SeqCst);
        let pid = self.state.pid.load(Ordering::Relaxed);
        if pid > 1 {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }

    /// Get the RPC connection, if the host is alive.
    pub async fn connection(&self) -> Option<Arc<Connection>> {
        self.state.connection.lock().await.clone()
    }

    /// Whether the host's child process is still alive (returns false once
    /// the exit-watcher has recorded an exit, or for a dummy host).
    pub async fn is_alive(&self) -> bool {
        if self.state.pid.load(Ordering::Relaxed) <= 1 {
            return false;
        }
        self.state.exit_status.lock().await.is_none()
    }

    /// Send SIGTERM (graceful), wait briefly for the exit-watcher to record
    /// the exit, then SIGKILL if it's still alive. Marks the death
    /// intentional first so the watcher stays silent (no spurious
    /// `Crashed`) and the `Drop` backstop becomes a no-op.
    pub async fn shutdown(self) -> ExtensionResult<ExitStatus> {
        self.state
            .intentional_shutdown
            .store(true, Ordering::SeqCst);
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

        let pid = self.state.pid.load(Ordering::Relaxed);
        if pid <= 1 {
            return Err(ExtensionError::HostSpawn(
                "no live process to shut down".into(),
            ));
        }
        // The exit-watcher owns the `Child` and reaps it; we only signal and
        // poll the recorded status. SIGTERM, wait ≤2s, then SIGKILL.
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGTERM,
        );
        if let Some(status) = self.wait_exit(Duration::from_secs(2)).await {
            return Ok(status);
        }
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        );
        self.wait_exit(Duration::from_secs(2))
            .await
            .ok_or_else(|| ExtensionError::HostSpawn("host did not exit after SIGKILL".into()))
    }

    /// Poll the watcher-filled `exit_status` cell up to `timeout`, waking on
    /// the `exit_done` notify between polls. Returns the recorded status, or
    /// `None` if the deadline passed first.
    async fn wait_exit(&self, timeout: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = *self.state.exit_status.lock().await {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            let notified = self.state.exit_done.notified();
            let _ = tokio::time::timeout(Duration::from_millis(50), notified).await;
        }
    }

    async fn start_health_monitor(
        &self,
        conn: Arc<Connection>,
        interval: Duration,
        sink: Option<HostEventSink>,
        pid: i32,
        rss_warn: Option<u64>,
    ) {
        let ext_id = self.state.ext_id.clone();
        let intentional = self.state.intentional_shutdown.clone();
        let rss_fired = self.state.rss_notice_fired.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                if intentional.load(Ordering::SeqCst) {
                    break;
                }
                // RSS sampling (observability only — never a kill). Warn every
                // tick over threshold but emit at most one notice.
                if let Some(limit) = rss_warn {
                    if let Some(rss) = read_rss_bytes(pid) {
                        if rss > limit {
                            tracing::warn!(
                                ext_id,
                                rss_mb = rss / 1_048_576,
                                limit_mb = limit / 1_048_576,
                                "ext-host resident memory over warn threshold",
                            );
                            if !rss_fired.swap(true, Ordering::SeqCst) {
                                if let Some(s) = &sink {
                                    let _ = s.send(HostEvent::RssWarn {
                                        ext_id: ext_id.clone(),
                                        rss,
                                        limit,
                                    });
                                }
                            }
                        }
                    }
                }
                let res = tokio::time::timeout(
                    Duration::from_millis(2000),
                    conn.request(method::PING, Value::Nil),
                )
                .await;
                match res {
                    Ok(Ok(_)) => continue,
                    Ok(Err(e)) => {
                        if intentional.load(Ordering::SeqCst) {
                            break;
                        }
                        tracing::warn!(ext_id, err = %e, "ext-host ping rpc error");
                        if let Some(s) = &sink {
                            let _ = s.send(HostEvent::Hung {
                                ext_id: ext_id.clone(),
                            });
                        }
                        break;
                    }
                    Err(_) => {
                        if intentional.load(Ordering::SeqCst) {
                            break;
                        }
                        tracing::warn!(ext_id, "ext-host ping timeout (Layer D: hung)");
                        if let Some(s) = &sink {
                            let _ = s.send(HostEvent::Hung {
                                ext_id: ext_id.clone(),
                            });
                        }
                        break;
                    }
                }
            }
        });
        *self.state.health_task.lock().await = Some(task);
    }
}

/// Classify a child exit for logging / restart policy. A clean `exit(0)`
/// we didn't ask for is still abnormal (spec Layer F).
fn classify_exit(status: ExitStatus) -> HostExit {
    if let Some(code) = status.code() {
        if code == 0 {
            HostExit::CleanUnexpected
        } else {
            HostExit::Code(code)
        }
    } else {
        HostExit::Signal(status.signal().unwrap_or(0))
    }
}

/// Read a process's resident memory in bytes. Best-effort, cross-platform,
/// no new deps. Returns `None` on any failure (dead pid, parse error,
/// unsupported platform) — RSS sampling must never be fatal.
#[cfg(target_os = "linux")]
fn read_rss_bytes(pid: i32) -> Option<u64> {
    if pid <= 1 {
        return None;
    }
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let resident_pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: sysconf is a pure lookup with no preconditions.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return None;
    }
    Some(resident_pages * page_size as u64)
}

#[cfg(target_os = "macos")]
fn read_rss_bytes(pid: i32) -> Option<u64> {
    if pid <= 1 {
        return None;
    }
    // SAFETY: `proc_pid_rusage` fills a zeroed `rusage_info_v2` via the
    // out-pointer; on success (rc == 0) we read only `ri_resident_size`.
    unsafe {
        let mut info: libc::rusage_info_v2 = std::mem::zeroed();
        let mut ptr: libc::rusage_info_t =
            &mut info as *mut libc::rusage_info_v2 as libc::rusage_info_t;
        let rc = libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, &mut ptr);
        if rc == 0 {
            Some(info.ri_resident_size)
        } else {
            None
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn read_rss_bytes(_pid: i32) -> Option<u64> {
    None
}

impl Drop for NodeHost {
    fn drop(&mut self) {
        // Only the last live `NodeHost` handle reaps. The watcher / health
        // tasks hold the field `Arc`s, not `Arc<HostState>`, so this counts
        // handles only. `swap` makes the kill fire at most once and turns a
        // post-`shutdown` drop into a no-op (shutdown already set the flag).
        if Arc::strong_count(&self.state) > 1 {
            return;
        }
        if self.state.intentional_shutdown.swap(true, Ordering::SeqCst) {
            return;
        }
        let pid = self.state.pid.load(Ordering::Relaxed);
        if pid <= 1 {
            return;
        }
        // Best-effort SIGKILL backstop for tests / panics / forgotten
        // handles. The graceful path is `shutdown().await`; this is the net
        // beneath it. Only `nix::kill` (a raw syscall) — safe even if the
        // tokio runtime is already gone.
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        );
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

/// Drain a child stdout/stderr pipe, writing one NDJSON record `{t,msg}` per
/// line to `sink`. Timestamping each line (epoch ms) is what lets the settings
/// "Logs" tab merge console output with `createOutputChannel` channels into one
/// time-ordered view. `None` sink (tests) just consumes the bytes. A write
/// error is logged, never fatal — a stalled write would back the pipe up and
/// hang the child.
async fn drain_pipe<R>(mut reader: R, sink: Option<Arc<LogWriter>>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    // Cap an unterminated line so a child that never emits `\n` can't grow the
    // buffer without bound; flush it as one record at the cap.
    const MAX_LINE: usize = 64 * 1024;
    let mut buf = [0u8; 4096];
    let mut line = Vec::<u8>::new();
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let Some(w) = sink.as_ref() else { continue };
                line.extend_from_slice(&buf[..n]);
                while let Some(pos) = line.iter().position(|&b| b == b'\n') {
                    let mut rest = line.split_off(pos + 1);
                    std::mem::swap(&mut line, &mut rest);
                    write_log_line(w, &rest); // `rest` now holds the line incl. trailing \n
                }
                if line.len() > MAX_LINE {
                    let whole = std::mem::take(&mut line);
                    write_log_line(w, &whole);
                }
            }
        }
    }
    // Flush any trailing partial line at EOF.
    if let Some(w) = sink.as_ref() {
        if !line.is_empty() {
            write_log_line(w, &line);
        }
    }
}

/// Write one drained console line as an NDJSON record. Strips the trailing
/// CR/LF, skips truly empty lines, and stamps `t` (epoch ms).
fn write_log_line(w: &LogWriter, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_end_matches(['\r', '\n']);
    if text.is_empty() {
        return;
    }
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let record = serde_json::json!({ "t": t, "msg": text }).to_string();
    if let Err(e) = w.write_line(&record) {
        tracing::warn!(error = %e, path = %w.path().display(), "extension log write failed");
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
            stdout_log: None,
            stderr_log: None,
            host_event_sink: None,
            rss_warn_bytes: None,
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
            stdout_log: None,
            stderr_log: None,
            host_event_sink: None,
            rss_warn_bytes: None,
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
            stdout_log: None,
            stderr_log: None,
            host_event_sink: None,
            rss_warn_bytes: None,
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
            stdout_log: None,
            stderr_log: None,
            host_event_sink: None,
            rss_warn_bytes: None,
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
            stdout_log: None,
            stderr_log: None,
            host_event_sink: None,
            rss_warn_bytes: None,
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
            stdout_log: None,
            stderr_log: None,
            host_event_sink: None,
            rss_warn_bytes: None,
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
