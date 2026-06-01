//! P10 end-to-end: extension host lifecycle robustness against real Node 26.
//!
//! Skipped (with a stderr breadcrumb) when the bundled Node 26 binary is
//! missing — same convention as `p2`/`p4`/`p6`.
//!
//! What it proves:
//!
//! 1. **Auto-restart** — SIGKILL a live host and the supervisor respawns it:
//!    the extension's `activate()` runs again (a new pid, a second activation
//!    marker) and the extension stays `is_activated`.
//! 2. **Crash storm → disable + notice** — with a budget of 1, the second
//!    crash exhausts it: the extension is deactivated, the registry persists
//!    `enabled=false` + `disabled_reason=Crash`, and an `Error` notice fires.
//! 3. **`shutdown_all` leaves no orphans** — two live hosts are both reaped
//!    (the leak this whole change fixes).
//!
//! The fixture extension self-reports from inside `activate()` (writes its
//! `process.pid` and appends an activation marker to its storage dir) so the
//! test orchestrates purely through public API + files — no internal
//! accessors, and the SIGKILL targets the real child via `kill(1)`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempfile::TempDir;

use cronymax::extensions::host::node::SpawnConfig;
use cronymax::extensions::registry::{DisabledReason, ExtensionRegistry};
use cronymax::extensions::runtime::SpawnConfigBuilder;
use cronymax::extensions::{ExtensionRuntime, Manifest};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn bundled_node() -> PathBuf {
    repo_root().join("crates/cronymax/bundled/node/bin/node")
}

fn bundled_bootstrap() -> PathBuf {
    repo_root().join("crates/cronymax/bundled/extension-host-bootstrap.js")
}

fn skip_if_no_bundled_node() -> bool {
    if !bundled_node().is_file() {
        eprintln!(
            "skipping p10_host_lifecycle_e2e: bundled Node 26 not present at {} \
             (run `scripts/fetch-node26.sh` first)",
            bundled_node().display(),
        );
        return true;
    }
    if !bundled_bootstrap().is_file() {
        eprintln!(
            "skipping p10_host_lifecycle_e2e: bootstrap.js missing at {}",
            bundled_bootstrap().display(),
        );
        return true;
    }
    false
}

async fn wait_until<F>(deadline: Duration, mut pred: F) -> bool
where
    F: FnMut() -> bool,
{
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        if pred() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// A minimal host-backed extension whose `activate()` records its pid and
/// appends one line to `acts` per activation (so a restart is observable as
/// the line count growing + the pid changing).
fn write_host_extension(ext_dir: &Path, id: &str, publisher: &str) {
    let dist = ext_dir.join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(
        dist.join("main.js"),
        r#"
const fs = require("fs");
const path = require("path");
exports.activate = async () => {
  const dir = process.env.CRONYMAX_EXTENSION_STORAGE;
  try {
    fs.writeFileSync(path.join(dir, "pid"), String(process.pid));
    fs.appendFileSync(path.join(dir, "acts"), "x\n");
  } catch (_e) {}
};
exports.deactivate = () => {};
"#,
    )
    .unwrap();
    let manifest = serde_json::json!({
        "id": id,
        "name": id,
        "version": "0.0.1",
        "publisher": publisher,
        "engines": { "cronymax": "^1.0" },
        "main": "dist/main.js",
        "activationEvents": ["*"],
    });
    std::fs::write(
        ext_dir.join("cronymax-extension.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

fn make_builder(storage_root: PathBuf, global_root: PathBuf) -> SpawnConfigBuilder {
    let node = bundled_node();
    let bootstrap = bundled_bootstrap();
    Arc::new(move |id: &str, _m: &Manifest, ext_dir: &Path| {
        let storage_dir = storage_root.join(id);
        let global_storage_dir = global_root.join(id);
        let _ = std::fs::create_dir_all(&storage_dir);
        let _ = std::fs::create_dir_all(&global_storage_dir);
        SpawnConfig {
            ext_id: id.to_string(),
            node_binary: node.clone(),
            node_flags: vec!["--no-warnings".into()],
            bootstrap_js: bootstrap.clone(),
            ext_dir: ext_dir.to_path_buf(),
            storage_dir,
            global_storage_dir,
            workspace_dirs: Vec::new(),
            manifest_path: ext_dir.join("cronymax-extension.json"),
            max_restarts: 0,
            ping_interval: Some(SpawnConfig::ping_interval_default()),
            stdout_log: None,
            stderr_log: None,
            // Left None: `ExtensionRuntime` installs the crash-signal sink +
            // RSS threshold itself in `spawn_and_handshake`.
            host_event_sink: None,
            rss_warn_bytes: None,
        }
    })
}

fn read_pid(storage_root: &Path, id: &str) -> Option<i32> {
    std::fs::read_to_string(storage_root.join(id).join("pid"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn acts_count(storage_root: &Path, id: &str) -> usize {
    std::fs::read_to_string(storage_root.join(id).join("acts"))
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

fn pid_alive(pid: i32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn sigkill(pid: i32) {
    let _ = std::process::Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .status();
}

fn install_ext(reg_root: &Path, id: &str, publisher: &str) -> ExtensionRegistry {
    let ext_dir = reg_root.join(id);
    std::fs::create_dir_all(&ext_dir).unwrap();
    write_host_extension(&ext_dir, id, publisher);
    let mut registry = ExtensionRegistry::new(reg_root);
    registry.refresh().expect("registry refresh");
    registry
}

#[tokio::test]
async fn kill_host_auto_restarts() {
    if skip_if_no_bundled_node() {
        return;
    }
    let td = TempDir::new().unwrap();
    let reg_root = td.path().join("registry");
    std::fs::create_dir_all(&reg_root).unwrap();
    let registry = install_ext(&reg_root, "test.restart", "test");

    let runtime = ExtensionRuntime::new(registry);
    let storage_root = td.path().join("storage");
    runtime.set_spawn_config_builder(make_builder(storage_root.clone(), td.path().join("global")));

    runtime
        .activate_default("test.restart")
        .await
        .expect("activate");
    assert!(
        wait_until(Duration::from_secs(10), || acts_count(
            &storage_root,
            "test.restart"
        ) >= 1)
        .await,
        "first activation should record an activation marker",
    );
    let pid1 = read_pid(&storage_root, "test.restart").expect("pid after first activate");
    assert!(pid_alive(pid1), "host should be alive after activate");

    // Kill the live host out from under the runtime.
    sigkill(pid1);

    // The supervisor should respawn: a second activation marker + a new pid.
    let restarted = wait_until(Duration::from_secs(20), || {
        acts_count(&storage_root, "test.restart") >= 2
            && read_pid(&storage_root, "test.restart").is_some_and(|p| p != pid1)
    })
    .await;
    assert!(restarted, "host should auto-restart after SIGKILL");
    assert!(
        runtime.is_activated("test.restart"),
        "extension stays activated across a restart",
    );
    let pid2 = read_pid(&storage_root, "test.restart").unwrap();
    assert_ne!(pid1, pid2, "restart should be a fresh process");
    assert!(pid_alive(pid2), "the restarted host should be alive");

    runtime.shutdown_all().await;
}

#[tokio::test]
async fn crash_storm_disables_and_notifies() {
    if skip_if_no_bundled_node() {
        return;
    }
    let td = TempDir::new().unwrap();
    let reg_root = td.path().join("registry");
    std::fs::create_dir_all(&reg_root).unwrap();
    let registry = install_ext(&reg_root, "test.storm", "test");

    let runtime = ExtensionRuntime::new(registry);
    // Budget of 1: the 1st crash restarts, the 2nd exhausts the budget.
    runtime.set_max_restarts(1);

    let notices: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let sink = notices.clone();
        runtime.set_notice_emitter(Arc::new(move |notice| {
            sink.lock()
                .unwrap()
                .push((format!("{:?}", notice.level), notice.message));
        }));
    }
    let storage_root = td.path().join("storage");
    runtime.set_spawn_config_builder(make_builder(storage_root.clone(), td.path().join("global")));

    runtime
        .activate_default("test.storm")
        .await
        .expect("activate");
    assert!(
        wait_until(Duration::from_secs(10), || acts_count(
            &storage_root,
            "test.storm"
        ) >= 1)
        .await,
        "first activation",
    );
    let pid1 = read_pid(&storage_root, "test.storm").expect("pid1");

    // Crash #1 → restart (count 1 ≤ budget 1).
    sigkill(pid1);
    assert!(
        wait_until(Duration::from_secs(20), || {
            acts_count(&storage_root, "test.storm") >= 2
                && read_pid(&storage_root, "test.storm").is_some_and(|p| p != pid1)
        })
        .await,
        "first crash should restart",
    );
    let pid2 = read_pid(&storage_root, "test.storm").expect("pid2");

    // Crash #2 → count 2 > budget 1 → disable.
    sigkill(pid2);
    assert!(
        wait_until(Duration::from_secs(20), || !runtime
            .is_activated("test.storm"))
        .await,
        "exceeding the restart budget should disable the extension",
    );

    // Registry persisted the crash-disable.
    let info = runtime
        .list_installed()
        .into_iter()
        .find(|e| e.id == "test.storm")
        .expect("ext still listed");
    assert!(!info.enabled, "crash-disabled extension is not enabled");
    assert_eq!(
        info.disabled_reason,
        Some(DisabledReason::Crash),
        "disabled_reason should record the crash",
    );

    // Both hosts are already dead (we killed them); nothing live to reap.
    runtime.shutdown_all().await;

    // An Error notice surfaced to the user.
    let got = notices.lock().unwrap();
    assert!(
        got.iter()
            .any(|(lvl, msg)| lvl == "Error" && msg.contains("test.storm")),
        "expected a crash-disable Error notice; got {got:?}",
    );
}

#[tokio::test]
async fn shutdown_all_reaps_every_host() {
    if skip_if_no_bundled_node() {
        return;
    }
    let td = TempDir::new().unwrap();
    let reg_root = td.path().join("registry");
    std::fs::create_dir_all(&reg_root).unwrap();
    // Two extensions in one registry.
    write_host_extension(&reg_root.join("test.one"), "test.one", "test");
    write_host_extension(&reg_root.join("test.two"), "test.two", "test");
    let mut registry = ExtensionRegistry::new(&reg_root);
    registry.refresh().unwrap();

    let runtime = ExtensionRuntime::new(registry);
    let storage_root = td.path().join("storage");
    runtime.set_spawn_config_builder(make_builder(storage_root.clone(), td.path().join("global")));

    runtime
        .activate_default("test.one")
        .await
        .expect("activate one");
    runtime
        .activate_default("test.two")
        .await
        .expect("activate two");
    assert!(
        wait_until(Duration::from_secs(10), || {
            acts_count(&storage_root, "test.one") >= 1 && acts_count(&storage_root, "test.two") >= 1
        })
        .await,
        "both hosts should activate",
    );
    let p1 = read_pid(&storage_root, "test.one").expect("pid one");
    let p2 = read_pid(&storage_root, "test.two").expect("pid two");
    assert!(
        pid_alive(p1) && pid_alive(p2),
        "both hosts alive before shutdown"
    );

    runtime.shutdown_all().await;

    assert!(
        wait_until(Duration::from_secs(5), || !pid_alive(p1) && !pid_alive(p2)).await,
        "shutdown_all must leave no orphaned Node hosts (p1={p1} p2={p2})",
    );
}
