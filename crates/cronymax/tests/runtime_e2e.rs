//! End-to-end runtime test (task 12.1) plus failure-mode coverage
//! (subset of task 12.2) exercised against the in-process Rust
//! runtime authority (no GIPS — that surface is covered by
//! `crony/tests/gips_transport.rs`).
//!
//! The scenarios here intentionally walk a complete run lifecycle —
//! start → tool dispatch → permission pause → resume → terminal
//! status — and then drop the authority and reconstruct a fresh one
//! from the same persistence file to assert restart rehydration
//! preserves both run state and history.

use std::sync::Arc;

use cronymax::{
    JsonFilePersistence, MemoryEntry, MemoryNamespaceId, PermissionState, Run,
    RuntimeAuthority, RuntimeError, Snapshot, Space,
};
use cronymax::runtime::SubscribeOutcome;
use cronymax::protocol::events::RuntimeEventPayload;
use tempfile::tempdir;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_run_lifecycle_round_trips_through_persistence() {
    let dir = tempdir().expect("tempdir");
    let persistence =
        Arc::new(JsonFilePersistence::under_app_data_dir(dir.path()));

    // ── First runtime instance: drive a complete lifecycle. ──────────
    let space_id;
    let run_id;
    let review_id;
    {
        let auth = RuntimeAuthority::rehydrate(persistence.clone())
            .expect("rehydrate fresh");

        let space = Space {
            id: cronymax::SpaceId::new(),
            name: "e2e".into(),
        };
        space_id = space.id;
        auth.upsert_space(space).expect("upsert space");

        // Subscribe before start so we observe the status event.
        let SubscribeOutcome { id: sub_id, mut receiver } =
            auth.subscribe("*");

        run_id = auth
            .start_run(space_id, None, serde_json::json!({"input": "hi"}))
            .expect("start run");

        // First event = run.started status. Drain a couple to be safe;
        // the Subscribe call itself may have queued internal frames.
        let mut saw_status = false;
        for _ in 0..4 {
            match tokio::time::timeout(
                std::time::Duration::from_millis(100),
                receiver.recv(),
            )
            .await
            {
                Ok(Some(evt)) => {
                    if matches!(
                        evt.payload,
                        RuntimeEventPayload::RunStatus { .. }
                    ) {
                        saw_status = true;
                        break;
                    }
                }
                _ => break,
            }
        }
        assert!(saw_status, "expected RunStatus event after start_run");
        auth.unsubscribe(sub_id);

        // Simulate the agent loop: append a tool-call history entry,
        // mark running, then open a permission review.
        auth.mark_run_running(run_id).expect("mark running");
        auth.append_history(
            run_id,
            serde_json::json!({"kind": "tool.call", "name": "shell.exec"}),
        )
        .expect("append history");

        let handle = auth.open_review_with_completion(
            run_id,
            serde_json::json!({"tool": "shell.exec", "cmd": "ls"}),
        )
        .expect("open review");
        review_id = handle.id;

        // Capability is still pending — resolve as Approved on behalf
        // of the host UI.
        auth.resolve_review(run_id, review_id, PermissionState::Approved, None)
            .expect("resolve review");
        let resolution = handle.completion.await.expect("completion");
        assert_eq!(resolution.decision, PermissionState::Approved);

        // Memory write so we can verify it persists across restart.
        auth.put_memory(
            MemoryNamespaceId::from("space:e2e/conv"),
            MemoryEntry {
                key: "last_user".into(),
                value: serde_json::json!("hello"),
                updated_at_ms: 1,
            },
        )
        .expect("put memory");

        auth.complete_run(run_id).expect("complete run");
    }

    // ── Second runtime instance: rehydrate, verify state. ────────────
    let auth = RuntimeAuthority::rehydrate(persistence.clone())
        .expect("rehydrate restart");
    let snap = auth.snapshot();
    assert!(
        snap.spaces.contains_key(&space_id),
        "space must survive restart"
    );
    let run = snap.runs.get(&run_id).expect("run must survive restart");
    assert!(
        matches!(run.status, cronymax::RunStatus::Succeeded),
        "succeeded run must rehydrate as Succeeded, got {:?}",
        run.status
    );
    assert!(
        !run.history.is_empty(),
        "appended history entry must survive restart"
    );
    let review = snap
        .reviews
        .get(&review_id)
        .expect("review must survive restart");
    assert_eq!(review.state, PermissionState::Approved);

    let mem = snap
        .memory
        .get(&MemoryNamespaceId::from("space:e2e/conv"))
        .expect("memory namespace must survive restart");
    assert!(mem.entries.contains_key("last_user"));

    // Schema version is stamped to the current authoritative version.
    assert_eq!(
        snap.schema_version,
        cronymax::runtime::state::SNAPSHOT_SCHEMA_VERSION
    );
}

// ── Failure-mode coverage (task 12.2 subset) ─────────────────────────

#[test]
fn corrupted_snapshot_file_surfaces_load_error() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("runtime-state.json");
    std::fs::write(&path, b"{ this is not valid json").expect("write");
    let persistence = Arc::new(JsonFilePersistence::new(&path));
    let err = RuntimeAuthority::rehydrate(persistence)
        .expect_err("corrupt snapshot must fail rehydrate");
    // We don't assert on the exact variant — only that startup
    // refuses to swallow corruption silently. Boot must fail cleanly.
    let msg = format!("{err}");
    assert!(
        msg.contains("serialization") || msg.contains("io") || msg.contains("snapshot"),
        "expected persistence-layer error, got: {msg}"
    );
    let _ = err;
    let _: RuntimeError; // type alias check
}

#[test]
fn future_schema_version_refuses_to_boot() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("runtime-state.json");
    let future = serde_json::json!({
        "schema_version": 9999,
        "spaces": {}, "agents": {}, "runs": {},
        "memory": {}, "reviews": {}
    });
    std::fs::write(&path, serde_json::to_vec(&future).unwrap()).expect("write");
    let persistence = Arc::new(JsonFilePersistence::new(&path));
    let err = RuntimeAuthority::rehydrate(persistence)
        .expect_err("future schema must refuse to boot");
    let msg = format!("{err}");
    assert!(
        msg.contains("schema") || msg.contains("understands"),
        "expected migration error, got: {msg}"
    );
}

// Suppress an unused-import warning when the `Run` and `Snapshot`
// imports are kept for future scenario expansion.
#[allow(dead_code)]
fn _type_anchors(_: Snapshot, _: Run) {}
