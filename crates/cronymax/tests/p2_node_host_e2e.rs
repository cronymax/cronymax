//! Phase 2 end-to-end host integration: spawn the bundled Node 26 with
//! the real `extension-host-bootstrap.js`, drive a tiny test extension,
//! and assert the `$/ready` handshake + `extension/activate` round-trip
//! work. Permission-denial / audit-notify tests were dropped together
//! with the v1-alpha permission model.
//!
//! This test is **skipped** unless the bundled Node 26 binary is present
//! (run `scripts/fetch-node26.sh` once). That keeps `cargo test` green on
//! fresh checkouts without forcing a 50 MB download.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rmpv::Value;
use tempfile::TempDir;
use tokio::sync::Mutex;

use cronymax::extensions::host::node::{NodeHost, SpawnConfig};
use cronymax::extensions::rpc::RpcServer;

fn repo_root() -> PathBuf {
    // crates/cronymax/tests/p2_node_host_e2e.rs → ../../..
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
            "skipping p2_node_host_e2e: bundled Node 26 not present at {} \
             (run `scripts/fetch-node26.sh` first)",
            bundled_node().display(),
        );
        return true;
    }
    if !bundled_bootstrap().is_file() {
        eprintln!(
            "skipping p2_node_host_e2e: bootstrap.js missing at {}",
            bundled_bootstrap().display(),
        );
        return true;
    }
    false
}

/// Build a tiny extension on the fly. The extension's `activate(ctx)`
/// pushes a flag we can detect indirectly via audit events.
fn write_test_extension(root: &std::path::Path, ext_id: &str) -> PathBuf {
    let dist = root.join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(
        dist.join("main.js"),
        r#"exports.activate = async (ctx) => {
            ctx.subscriptions.push({ dispose: () => {} });
            return { hello: "from-test-ext" };
        };
        exports.deactivate = async () => {};
        "#,
    )
    .unwrap();
    let manifest = root.join("cronymax-extension.json");
    let raw = serde_json::json!({
        "id": ext_id,
        "name": "Test Ext",
        "version": "0.1.0",
        "publisher": ext_id.split('.').next().unwrap(),
        "engines": { "cronymax": "^1.0" },
        "main": "./dist/main.js",
        "activationEvents": ["onStartup"],
    });
    std::fs::write(&manifest, serde_json::to_string_pretty(&raw).unwrap()).unwrap();
    manifest
}

#[derive(Clone, Debug, Default)]
struct CapturedNotifies {
    ready: Arc<Mutex<Option<Value>>>,
}

fn capture_server(captured: CapturedNotifies) -> RpcServer {
    let ready = captured.ready.clone();
    RpcServer::builder()
        .on_notify("$/ready", move |params| {
            let ready = ready.clone();
            async move {
                *ready.lock().await = Some(params);
                Ok(())
            }
        })
        .build()
}

async fn wait_for<F>(deadline: Duration, mut pred: F) -> bool
where
    F: FnMut() -> bool,
{
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        if pred() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn bootstrap_emits_ready_notify_then_activates() {
    if skip_if_no_bundled_node() {
        return;
    }

    let td = TempDir::new().unwrap();
    let ext_dir = td.path().join("ext");
    std::fs::create_dir_all(&ext_dir).unwrap();
    let manifest = write_test_extension(&ext_dir, "alice.e2e");
    let storage = td.path().join("storage");
    let global = td.path().join("global");
    std::fs::create_dir_all(&storage).unwrap();
    std::fs::create_dir_all(&global).unwrap();

    let captured = CapturedNotifies::default();
    let server = capture_server(captured.clone());

    // v1 alpha dropped the Node 26 permission model — no `--permission`
    // or `--allow-*` flags. The host binary is invoked with full Node
    // API access; trust is at install-time.
    let _ = (&ext_dir, &storage, &global);
    let node_flags = vec!["--no-warnings".into()];
    let cfg = SpawnConfig {
        ext_id: "alice.e2e".into(),
        node_binary: bundled_node(),
        node_flags,
        bootstrap_js: bundled_bootstrap(),
        ext_dir: ext_dir.clone(),
        storage_dir: storage,
        global_storage_dir: global,
        workspace_dirs: Vec::new(),
        manifest_path: manifest,
        max_restarts: 0,
        ping_interval: None,
        stdout_log: None,
        stderr_log: None,
    };

    let host = match NodeHost::spawn(cfg, server).await {
        Ok(h) => h,
        Err(e) => panic!("spawn failed: {e}"),
    };

    // Wait up to 4 s for the bootstrap to send $/ready.
    let got_ready = wait_for(Duration::from_secs(4), || {
        captured
            .ready
            .try_lock()
            .map(|g| g.is_some())
            .unwrap_or(false)
    })
    .await;
    assert!(got_ready, "bootstrap never sent $/ready notify");

    // Send extension/activate and expect a response.
    let conn = host
        .connection()
        .await
        .expect("connection should be present after spawn");
    let resp = tokio::time::timeout(
        Duration::from_secs(4),
        conn.request("extension/activate", Value::Nil),
    )
    .await
    .expect("activate timed out")
    .expect("activate failed");

    // The test extension's activate() returns { hello: "from-test-ext" }.
    match resp {
        Value::Map(entries) => {
            let hello = entries
                .iter()
                .find_map(|(k, v)| k.as_str().filter(|&s| s == "hello").and(v.as_str()));
            assert_eq!(hello, Some("from-test-ext"));
        }
        other => panic!("expected map from activate, got {other:?}"),
    }

    // Clean shutdown.
    let _ = host.shutdown().await;
}

#[tokio::test]
async fn workspace_folders_array_reaches_extension() {
    // Extension activate() reflects globalThis.cronymax.workspace.workspaceFolders
    // back. Platform passes two roots; both should arrive (canonical), in
    // declaration order, and `rootUri` should match folders[0].
    if skip_if_no_bundled_node() {
        return;
    }

    let td = TempDir::new().unwrap();
    let ext_dir = td.path().join("ext");
    std::fs::create_dir_all(ext_dir.join("dist")).unwrap();
    std::fs::write(
        ext_dir.join("dist/main.js"),
        r#"exports.activate = async () => ({
            folders: globalThis.cronymax.workspace.workspaceFolders.map(f => f.uri),
            rootUri: globalThis.cronymax.workspace.rootUri ?? null,
        });
        "#,
    )
    .unwrap();
    std::fs::write(
        ext_dir.join("cronymax-extension.json"),
        r#"{ "id":"alice.ws","name":"X","version":"0.1.0","publisher":"alice",
             "engines":{"cronymax":"^1.0"},"main":"./dist/main.js","activationEvents":[] }"#,
    )
    .unwrap();
    let manifest = ext_dir.join("cronymax-extension.json");

    let ws_a = td.path().join("workspace_a");
    let ws_b = td.path().join("workspace_b");
    std::fs::create_dir_all(&ws_a).unwrap();
    std::fs::create_dir_all(&ws_b).unwrap();
    let storage = td.path().join("storage");
    let global = td.path().join("global");
    std::fs::create_dir_all(&storage).unwrap();
    std::fs::create_dir_all(&global).unwrap();

    let server = capture_server(CapturedNotifies::default());
    let cfg = SpawnConfig {
        ext_id: "alice.ws".into(),
        node_binary: bundled_node(),
        node_flags: vec!["--no-warnings".into()],
        bootstrap_js: bundled_bootstrap(),
        ext_dir: ext_dir.clone(),
        storage_dir: storage,
        global_storage_dir: global,
        workspace_dirs: vec![ws_a.clone(), ws_b.clone()],
        manifest_path: manifest,
        max_restarts: 0,
        ping_interval: None,
        stdout_log: None,
        stderr_log: None,
    };

    let host = NodeHost::spawn(cfg, server).await.unwrap();
    let conn = host.connection().await.unwrap();
    let resp = tokio::time::timeout(
        Duration::from_secs(4),
        conn.request("extension/activate", Value::Nil),
    )
    .await
    .unwrap()
    .unwrap();

    let canon_a = std::fs::canonicalize(&ws_a).unwrap();
    let canon_b = std::fs::canonicalize(&ws_b).unwrap();
    let want_a = format!("file://{}", canon_a.display());
    let want_b = format!("file://{}", canon_b.display());

    if let Value::Map(entries) = resp {
        let folders = entries
            .iter()
            .find_map(|(k, v)| k.as_str().filter(|&s| s == "folders").map(|_| v))
            .expect("folders key");
        let got: Vec<&str> = folders
            .as_array()
            .expect("folders is array")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(got, vec![want_a.as_str(), want_b.as_str()]);

        let root_uri = entries
            .iter()
            .find_map(|(k, v)| k.as_str().filter(|&s| s == "rootUri").map(|_| v))
            .expect("rootUri key");
        assert_eq!(root_uri.as_str(), Some(want_a.as_str()));
    } else {
        panic!("unexpected response: {resp:?}");
    }

    let _ = host.shutdown().await;
}

#[tokio::test]
async fn no_workspace_open_yields_empty_array_not_null() {
    if skip_if_no_bundled_node() {
        return;
    }

    let td = TempDir::new().unwrap();
    let ext_dir = td.path().join("ext");
    std::fs::create_dir_all(ext_dir.join("dist")).unwrap();
    std::fs::write(
        ext_dir.join("dist/main.js"),
        r#"exports.activate = async () => ({
            count: globalThis.cronymax.workspace.workspaceFolders.length,
            rootUri: globalThis.cronymax.workspace.rootUri ?? null,
            isArray: Array.isArray(globalThis.cronymax.workspace.workspaceFolders),
        });
        "#,
    )
    .unwrap();
    std::fs::write(
        ext_dir.join("cronymax-extension.json"),
        r#"{ "id":"alice.empty","name":"X","version":"0.1.0","publisher":"alice",
             "engines":{"cronymax":"^1.0"},"main":"./dist/main.js","activationEvents":[] }"#,
    )
    .unwrap();
    let manifest = ext_dir.join("cronymax-extension.json");
    let storage = td.path().join("storage");
    let global = td.path().join("global");
    std::fs::create_dir_all(&storage).unwrap();
    std::fs::create_dir_all(&global).unwrap();

    let server = capture_server(CapturedNotifies::default());
    let cfg = SpawnConfig {
        ext_id: "alice.empty".into(),
        node_binary: bundled_node(),
        node_flags: vec!["--no-warnings".into()],
        bootstrap_js: bundled_bootstrap(),
        ext_dir: ext_dir.clone(),
        storage_dir: storage,
        global_storage_dir: global,
        workspace_dirs: Vec::new(),
        manifest_path: manifest,
        max_restarts: 0,
        ping_interval: None,
        stdout_log: None,
        stderr_log: None,
    };

    let host = NodeHost::spawn(cfg, server).await.unwrap();
    let conn = host.connection().await.unwrap();
    let resp = tokio::time::timeout(
        Duration::from_secs(4),
        conn.request("extension/activate", Value::Nil),
    )
    .await
    .unwrap()
    .unwrap();

    if let Value::Map(entries) = resp {
        let count = entries
            .iter()
            .find_map(|(k, v)| k.as_str().filter(|&s| s == "count").and(v.as_u64()))
            .expect("count");
        let is_array = entries
            .iter()
            .find_map(|(k, v)| k.as_str().filter(|&s| s == "isArray").and(v.as_bool()))
            .expect("isArray");
        let root_uri = entries
            .iter()
            .find_map(|(k, v)| k.as_str().filter(|&s| s == "rootUri").map(|_| v))
            .expect("rootUri key");
        assert_eq!(count, 0);
        assert!(
            is_array,
            "workspaceFolders must be an array even when empty"
        );
        assert_eq!(*root_uri, Value::Nil);
    } else {
        panic!("unexpected response");
    }

    let _ = host.shutdown().await;
}
