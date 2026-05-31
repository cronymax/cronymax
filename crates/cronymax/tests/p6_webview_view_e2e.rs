//! End-to-end: WebviewViewProvider bidirectional messaging for operation
//! views, against a real Node 26 host + bootstrap.js.
//!
//! Skipped (with a stderr breadcrumb) when the bundled Node 26 binary is
//! missing — same convention as `p2_node_host_e2e` / `p4_extension_runtime_e2e`.
//!
//! What it proves, end to end through the real host + bootstrap SDK:
//!
//! 1. `window.registerWebviewViewProvider(viewId, provider)` from inside
//!    `activate(ctx)` lands the view in the sidebar-view registry (via the
//!    `sidebar/register` wire) so the platform knows the owner.
//! 2. `runtime.resolve_view(viewId)` drives the extension's
//!    `resolveWebviewView`, which posts a greeting — proving the
//!    extension → view direction (`webviewView/postMessage` →
//!    `WebviewEvent::Message` keyed by viewId).
//! 3. `runtime.forward_panel_message(viewId, payload)` (the iframe → ext
//!    bridge) reaches the provider's `webview.onDidReceiveMessage`, which
//!    echoes the payload back out as another `WebviewEvent::Message`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempfile::TempDir;

use cronymax::extensions::api::webview::WebviewEvent;
use cronymax::extensions::host::node::SpawnConfig;
use cronymax::extensions::registry::ExtensionRegistry;
use cronymax::extensions::ExtensionRuntime;

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
            "skipping p6_webview_view_e2e: bundled Node 26 not present at {} \
             (run `scripts/fetch-node26.sh` first)",
            bundled_node().display(),
        );
        return true;
    }
    if !bundled_bootstrap().is_file() {
        eprintln!(
            "skipping p6_webview_view_e2e: bootstrap.js missing at {}",
            bundled_bootstrap().display(),
        );
        return true;
    }
    false
}

/// Synthetic extension whose `activate()` registers a webview view provider
/// for `alice.view.v`. The provider posts a greeting on resolve and echoes
/// any inbound message back out.
fn write_view_extension(root: &std::path::Path) -> PathBuf {
    let dist = root.join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(
        dist.join("main.js"),
        r#"
        exports.activate = async (ctx) => {
            const c = globalThis.cronymax;
            ctx.subscriptions.push(
                c.window.registerWebviewViewProvider("alice.view.v", {
                    resolveWebviewView(view) {
                        view.webview.onDidReceiveMessage((msg) => {
                            view.webview.postMessage({ type: "echo", received: msg });
                        });
                        view.webview.postMessage({ type: "greeting", text: "hi from host" });
                    },
                }),
            );
            return { ok: true };
        };
        exports.deactivate = async () => {};
        "#,
    )
    .unwrap();

    let manifest_path = root.join("cronymax-extension.json");
    let manifest = serde_json::json!({
        "id": "alice.view",
        "name": "View Ext",
        "version": "0.1.0",
        "publisher": "alice",
        "engines": { "cronymax": "^1.0" },
        "main": "./dist/main.js",
        "activationEvents": ["onStartup"],
        "contributes": {
            "cronymax.ui.sidebar.view": [
                { "id": "alice.view.v", "title": "Alice View", "entry": "./v.html", "target": "main" }
            ]
        }
    });
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    manifest_path
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
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn webview_view_provider_round_trips_through_real_host() {
    if skip_if_no_bundled_node() {
        return;
    }

    let td = TempDir::new().unwrap();
    let reg_root = td.path().join("registry");
    std::fs::create_dir_all(&reg_root).unwrap();
    let ext_root = reg_root.join("alice.view");
    std::fs::create_dir_all(&ext_root).unwrap();
    let manifest_path = write_view_extension(&ext_root);

    let mut registry = ExtensionRegistry::new(&reg_root);
    registry.refresh().expect("registry refresh");
    assert!(registry.get("alice.view").is_some());

    let runtime = ExtensionRuntime::new(registry);

    // Capture every WebviewEvent the runtime emits (extension → view).
    let events: Arc<Mutex<Vec<WebviewEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let cap = events.clone();
    runtime.set_webview_emitter(Arc::new(move |ev| cap.lock().unwrap().push(ev)));

    let storage_dir = td.path().join("storage");
    let global_dir = td.path().join("global");
    std::fs::create_dir_all(&storage_dir).unwrap();
    std::fs::create_dir_all(&global_dir).unwrap();

    let manifest_path_clone = manifest_path.clone();
    tokio::time::timeout(
        Duration::from_secs(10),
        runtime.activate("alice.view", move |_manifest, ext_dir| SpawnConfig {
            ext_id: "alice.view".into(),
            node_binary: bundled_node(),
            node_flags: vec!["--no-warnings".into()],
            bootstrap_js: bundled_bootstrap(),
            ext_dir,
            storage_dir: storage_dir.clone(),
            global_storage_dir: global_dir.clone(),
            workspace_dirs: Vec::new(),
            manifest_path: manifest_path_clone.clone(),
            max_restarts: 0,
            ping_interval: None,
            stdout_log: None,
            stderr_log: None,
        }),
    )
    .await
    .expect("activate did not return within 10s")
    .expect("activate");

    // registerWebviewViewProvider sent `sidebar/register`; wait for the
    // sidebar-view registry to learn the owner.
    assert!(
        wait_until(Duration::from_secs(3), || runtime.sidebars().len() == 1).await,
        "provider registration should land the view in the sidebar registry",
    );

    // (a) extension → view: resolving the view runs resolveWebviewView,
    // which posts a greeting.
    runtime
        .resolve_view("alice.view.v")
        .await
        .expect("resolve_view");

    assert!(
        wait_until(Duration::from_secs(3), || !events
            .lock()
            .unwrap()
            .is_empty())
        .await,
        "expected a greeting Message after resolve",
    );
    {
        let evs = events.lock().unwrap();
        let greeting = evs.iter().find_map(|e| match e {
            WebviewEvent::Message { panel_id, payload } if panel_id == "alice.view.v" => {
                Some(payload.clone())
            }
            _ => None,
        });
        let greeting = greeting.expect("a Message keyed by the viewId");
        assert_eq!(
            greeting.get("type").and_then(|v| v.as_str()),
            Some("greeting"),
        );
    }

    // (b) iframe → ext → view echo: forward a payload as the renderer
    // bridge would; the provider echoes it back out.
    let before = events.lock().unwrap().len();
    runtime
        .forward_panel_message(
            "alice.view.v",
            serde_json::json!({ "type": "ping", "n": 7 }),
        )
        .await
        .expect("forward_panel_message to view");

    assert!(
        wait_until(Duration::from_secs(3), || {
            events.lock().unwrap().len() > before
        })
        .await,
        "expected an echo Message after forwarding to the view",
    );
    {
        let evs = events.lock().unwrap();
        let echo = evs[before..].iter().find_map(|e| match e {
            WebviewEvent::Message { panel_id, payload } if panel_id == "alice.view.v" => {
                Some(payload.clone())
            }
            _ => None,
        });
        let echo = echo.expect("an echo Message keyed by the viewId");
        assert_eq!(echo.get("type").and_then(|v| v.as_str()), Some("echo"));
        let received = echo
            .get("received")
            .expect("echo wraps the received payload");
        assert_eq!(received.get("type").and_then(|v| v.as_str()), Some("ping"));
        assert_eq!(received.get("n").and_then(|v| v.as_i64()), Some(7));
    }

    runtime.deactivate("alice.view").await.expect("deactivate");
}
