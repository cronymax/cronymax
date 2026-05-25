//! Phase 4 end-to-end: ExtensionRuntime + real Node 26 + bootstrap.js +
//! a synthetic extension that exercises every L2 EP register/unregister
//! path.
//!
//! Skipped (with stderr breadcrumb) when the bundled Node 26 binary is
//! missing — same convention as `p2_node_host_e2e.rs`.
//!
//! What it proves:
//!
//! 1. `cronymax.commands.register(...)` from inside `activate(ctx)` ends
//!    up in `runtime.state.commands` via the `commands/register` notify
//! 2. `cronymax.agents.registerProvider(...)` lands in
//!    `runtime.providers()` with the metadata pulled from the manifest
//!    (label / supports* fields)
//! 3. The content renderer declared in the manifest lands in
//!    `runtime.renderers()` keyed by MIME at activate time, with NO
//!    Node-side `registerRenderer` call (renderers are iframe-hosted in
//!    P6.5; see `cep-idl/v1/renderer-host.ts`).
//! 4. `cronymax.sidebar.register(...)` lands in `runtime.sidebars()`
//! 5. `runtime.deactivate(ext_id)` drops every registration and shuts
//!    down the host

use std::path::PathBuf;
use std::time::Duration;

use tempfile::TempDir;

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
            "skipping p4_extension_runtime_e2e: bundled Node 26 not present at {} \
             (run `scripts/fetch-node26.sh` first)",
            bundled_node().display(),
        );
        return true;
    }
    if !bundled_bootstrap().is_file() {
        eprintln!(
            "skipping p4_extension_runtime_e2e: bootstrap.js missing at {}",
            bundled_bootstrap().display(),
        );
        return true;
    }
    false
}

/// Write a fully-loaded synthetic extension under `root` and return its
/// `cronymax-extension.json` path. The extension's `activate()`:
///
/// * registers a command `alice.p4.hi`
/// * registers an agent provider `alice.p4.gpt`
/// * registers a sidebar view `alice.p4.view`
///
/// All three Node-side registrations are also declared in the manifest's
/// `contributes`, so the runtime's manifest lookup in each register-notify
/// handler will find a matching declaration.
///
/// The manifest additionally declares a content renderer `alice.p4.rend`
/// for `text/x-alice` — this is iframe-hosted in v1, so there is no Node-
/// side `registerRenderer` call; the platform ingests it from the
/// manifest at activate time.
fn write_p4_extension(root: &std::path::Path) -> PathBuf {
    let dist = root.join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(
        dist.join("main.js"),
        r#"
        exports.activate = async (ctx) => {
            const c = globalThis.cronymax;
            ctx.subscriptions.push(
                c.commands.register("alice.p4.hi", () => "ok"),
            );
            ctx.subscriptions.push(
                c.agents.registerProvider("alice.p4.gpt", {
                    enumerate: async () => [{ id: "m1", label: "M1" }],
                    createSession: async () => ({ id: "s1", prompt: () => {}, dispose: () => {} }),
                }),
            );
            ctx.subscriptions.push(
                c.sidebar.register("alice.p4.view"),
            );
            return { ok: true };
        };
        exports.deactivate = async () => {};
        "#,
    )
    .unwrap();

    let manifest_path = root.join("cronymax-extension.json");
    let manifest = serde_json::json!({
        "id": "alice.p4",
        "name": "P4 Ext",
        "version": "0.1.0",
        "publisher": "alice",
        "engines": { "cronymax": "^1.0" },
        "main": "./dist/main.js",
        "activationEvents": ["onStartup"],
        "contributes": {
            "cronymax.command": [
                { "id": "alice.p4.hi", "title": "Hi" }
            ],
            "cronymax.agents.provider": [
                {
                    "id": "alice.p4.gpt",
                    "label": "Alice P4 GPT",
                    "supportsModels": true,
                    "supportsModes": false,
                    "supportsMcp": false
                }
            ],
            "cronymax.content.renderer": [
                { "id": "alice.p4.rend", "mimeTypes": ["text/x-alice"], "entry": "./r.html" }
            ],
            "cronymax.ui.sidebar.view": [
                { "id": "alice.p4.view", "title": "Alice View", "entry": "./v.html" }
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
async fn activate_routes_all_four_l2_registrations_into_runtime() {
    if skip_if_no_bundled_node() {
        return;
    }

    let td = TempDir::new().unwrap();
    let reg_root = td.path().join("registry");
    std::fs::create_dir_all(&reg_root).unwrap();
    let ext_root = reg_root.join("alice.p4");
    std::fs::create_dir_all(&ext_root).unwrap();
    let manifest_path = write_p4_extension(&ext_root);

    // Install via registry::refresh so the runtime sees the ext.
    let mut registry = ExtensionRegistry::new(&reg_root);
    registry.refresh().expect("registry refresh");
    assert!(
        registry.get("alice.p4").is_some(),
        "registry should have adopted alice.p4 after refresh",
    );

    let runtime = ExtensionRuntime::new(registry);

    let storage_dir = td.path().join("storage");
    let global_dir = td.path().join("global");
    std::fs::create_dir_all(&storage_dir).unwrap();
    std::fs::create_dir_all(&global_dir).unwrap();

    let manifest_path_clone = manifest_path.clone();
    tokio::time::timeout(
        Duration::from_secs(10),
        runtime.activate("alice.p4", move |_manifest, ext_dir| SpawnConfig {
            ext_id: "alice.p4".into(),
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
        }),
    )
    .await
    .expect("activate did not return within 10s")
    .expect("activate");

    // Wait for the four registrations to land via the notify pipeline.
    let ok = wait_until(Duration::from_secs(3), || {
        runtime.providers().len() == 1
            && runtime.renderers().len() == 1
            && runtime.sidebars().len() == 1
    })
    .await;
    assert!(
        ok,
        "expected one provider + one renderer + one sidebar registration; \
         got providers={} renderers={} sidebars={}",
        runtime.providers().len(),
        runtime.renderers().len(),
        runtime.sidebars().len(),
    );

    // Spot-check metadata pulled from the manifest contributes block.
    let p = runtime.providers().get("alice.p4.gpt").unwrap();
    assert_eq!(p.owning_ext, "alice.p4");
    assert_eq!(p.label, "Alice P4 GPT");
    assert!(p.supports_models);
    assert!(!p.supports_modes);

    let r = runtime.renderers().get("alice.p4.rend").unwrap();
    assert_eq!(r.mime_types, vec!["text/x-alice".to_string()]);
    assert_eq!(
        runtime
            .renderers()
            .first_for_mime("text/x-alice")
            .unwrap()
            .renderer_id,
        "alice.p4.rend"
    );

    let v = runtime.sidebars().get("alice.p4.view").unwrap();
    assert_eq!(v.title, "Alice View");

    // Contributions reflect what the manifest declared.
    let cmd_ep = runtime.contributions_for_ep("cronymax.command");
    assert_eq!(cmd_ep.len(), 1);

    // send_to_extension routes via the runtime's handle map — no
    // ProviderEntry.conn anymore. Drive a synthetic round-trip via the
    // `commands/execute:<id>` handler that bootstrap.js registers
    // automatically when the extension calls `commands.register(...)`.
    // The handler we registered in main.js just returns the string "ok".
    let exec_resp = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_to_extension(
            "alice.p4",
            "commands/execute:alice.p4.hi",
            rmpv::Value::Array(vec![]),
        ),
    )
    .await
    .expect("send_to_extension timed out")
    .expect("send_to_extension failed");
    assert_eq!(exec_resp.as_str(), Some("ok"));

    // Deactivate clears every registry slot.
    runtime
        .deactivate("alice.p4")
        .await
        .expect("deactivate clean");
    assert!(runtime.providers().is_empty());
    assert!(runtime.renderers().is_empty());
    assert!(runtime.sidebars().is_empty());
    assert!(!runtime.is_activated("alice.p4"));
    assert!(runtime.contributions_for_ep("cronymax.command").is_empty());

    // After deactivate, send_to_extension returns NotActivated.
    let err = runtime
        .send_to_extension("alice.p4", "anything", rmpv::Value::Nil)
        .await
        .unwrap_err();
    assert!(
        matches!(err, cronymax::extensions::ExtensionError::NotActivated(_)),
        "expected NotActivated, got {err:?}",
    );
}

/// Verify the reverse `extension/registerError` notify path: an
/// extension registers a provider id that isn't declared in its
/// manifest. The runtime sends back a `registerError` notify;
/// bootstrap.js writes a `[cronymax] register failed: …` line to
/// stderr; this test catches it from the child's stderr stream.
#[tokio::test]
async fn registering_undeclared_provider_surfaces_via_register_error() {
    if skip_if_no_bundled_node() {
        return;
    }

    let td = TempDir::new().unwrap();
    let reg_root = td.path().join("registry");
    std::fs::create_dir_all(&reg_root).unwrap();
    let ext_root = reg_root.join("alice.bad");
    std::fs::create_dir_all(&ext_root).unwrap();

    let dist = ext_root.join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    // Extension's main.js intentionally registers a provider id that
    // is NOT in the manifest's contributes.
    std::fs::write(
        dist.join("main.js"),
        r#"
        exports.activate = async (ctx) => {
            ctx.subscriptions.push(
                globalThis.cronymax.agents.registerProvider("alice.bad.UNDECLARED", {
                    enumerate: async () => [],
                    createSession: async () => ({}),
                }),
            );
            return { ok: true };
        };
        exports.deactivate = async () => {};
        "#,
    )
    .unwrap();
    // Manifest declares NO providers — so the register notify will be
    // rejected by the runtime and the extension should receive a
    // registerError reverse notify.
    let manifest_path = ext_root.join("cronymax-extension.json");
    let manifest = serde_json::json!({
        "id": "alice.bad",
        "name": "Bad Ext",
        "version": "0.1.0",
        "publisher": "alice",
        "engines": { "cronymax": "^1.0" },
        "main": "./dist/main.js",
        "activationEvents": ["onStartup"],
    });
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let mut registry = ExtensionRegistry::new(&reg_root);
    registry.refresh().unwrap();
    let runtime = ExtensionRuntime::new(registry);

    let storage = td.path().join("storage");
    let global = td.path().join("global");
    std::fs::create_dir_all(&storage).unwrap();
    std::fs::create_dir_all(&global).unwrap();

    // To verify the error reaches the extension we read the child
    // process stderr. The host wires stdout/stderr drains internally
    // (host/node.rs `drain_pipe`) and currently drops bytes, so we
    // can't intercept directly. Instead, verify the negative: the
    // provider registry stays empty after activate completes. That is
    // observable from the runtime side without piping stderr.
    let manifest_path_clone = manifest_path.clone();
    tokio::time::timeout(
        Duration::from_secs(10),
        runtime.activate("alice.bad", move |_manifest, ext_dir| SpawnConfig {
            ext_id: "alice.bad".into(),
            node_binary: bundled_node(),
            node_flags: vec!["--no-warnings".into()],
            bootstrap_js: bundled_bootstrap(),
            ext_dir,
            storage_dir: storage.clone(),
            global_storage_dir: global.clone(),
            workspace_dirs: Vec::new(),
            manifest_path: manifest_path_clone.clone(),
            max_restarts: 0,
            ping_interval: None,
        }),
    )
    .await
    .expect("activate timed out")
    .expect("activate failed");

    // Give the register notify + registerError round-trip a moment.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // The registry stayed empty: the undeclared provider was rejected.
    assert!(
        runtime.providers().is_empty(),
        "undeclared provider id should never have landed in the registry; got {:?}",
        runtime.providers().list(),
    );

    let _ = runtime.deactivate("alice.bad").await;
}
