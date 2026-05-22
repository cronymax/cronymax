//! Integration tests for [`AgentRunner`] (tasks 11.1 and 11.2).
//!
//! These tests exercise `spawn_agent` and `spawn_chat` using in-process
//! doubles — `MockLlmFactory` + `FakeCapabilityFactory` — so no real
//! LLM or filesystem gating occurs.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tempfile::tempdir;

use cronymax::capability::factory::FakeCapabilityFactory;
use cronymax::capability::tier::SandboxTier;
use cronymax::flow::runtime::{InvocationContext, InvocationTrigger};
use cronymax::flow::FlowRuntimeRegistry;
use cronymax::llm::messages::FinishReason;
use cronymax::llm::mock::{MockLlmFactory, MockScript};
use cronymax::runtime::run_context::RunContext;
use cronymax::runtime::services::RuntimeServices;
use cronymax::runtime::AgentRunner;
use cronymax::{InMemoryPersistence, RuntimeAuthority, Space, SpaceId};

fn make_authority_with_space() -> (SpaceId, RuntimeAuthority) {
    let persistence = Arc::new(InMemoryPersistence::default());
    let auth = RuntimeAuthority::rehydrate(persistence).expect("rehydrate");
    let space = Space {
        id: SpaceId::new(),
        name: "test".into(),
        compaction_threshold_pct: 80,
        compaction_recency_turns: 6,
    };
    let space_id = space.id;
    auth.upsert_space(space).expect("upsert space");
    (space_id, auth)
}

fn run_ctx_no_flow(space_id: SpaceId, workspace_root: std::path::PathBuf) -> RunContext {
    let (doc_tx, _doc_rx) = tokio::sync::mpsc::channel(1);
    RunContext {
        space_id,
        workspace_root,
        flow_id: None,
        flow_run_id: None,
        session_id: None,
        flow_runtime: None,
        doc_tx,
        llm_config: cronymax::llm::LlmConfig::OpenAi {
            base_url: "http://localhost".into(),
            api_key: Some("mock-key".into()),
            model: "mock-model".into(),
        },
        sandbox_tier: SandboxTier::Trusted,
        workspace_cache_dir: None,
    }
}

/// 11.1 — `spawn_agent` with `MockLlmFactory` + `FakeCapabilityFactory` completes
/// without network calls and creates an authority run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_agent_completes_without_network() {
    let dir = tempdir().expect("tempdir");
    let (space_id, auth) = make_authority_with_space();

    let mock_llm = MockLlmFactory::new();
    mock_llm.provider().push(
        MockScript::new()
            .delta("Hello from mock LLM")
            .done(FinishReason::Stop),
    );

    let services = Arc::new(RuntimeServices {
        authority: auth.clone(),
        flow_registry: Arc::new(FlowRuntimeRegistry::default()),
        llm_factory: Arc::new(mock_llm.clone()),
        capability_factory: Arc::new(FakeCapabilityFactory),
        terminal_managers: Arc::new(Mutex::new(HashMap::new())),
        memory_manager: None,
    });

    let runner = AgentRunner::new(services);
    let run_ctx = run_ctx_no_flow(space_id, dir.path().to_path_buf());

    let trigger = InvocationTrigger {
        kind: "fresh_start".into(),
        approved_port: None,
        from_node: None,
        reviewer_doc_path: None,
    };
    let inv_ctx = InvocationContext::build("node-1", "agent-1", trigger, vec![], vec![]);

    runner.spawn_agent(run_ctx, "agent-1".into(), inv_ctx);

    // Give the spawned tokio task time to finish.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // The agent_runner creates its own authority run synchronously (start_run)
    // before spawning the async task, so there should be at least one run.
    let snap = auth.snapshot();
    assert!(
        !snap.runs.is_empty(),
        "expected at least one run after spawn_agent"
    );

    // Verify the mock LLM was called (no network): the provider should have
    // seen exactly one request (the entry turn).
    let requests = mock_llm.provider().requests();
    assert_eq!(requests.len(), 1, "expected exactly one LLM request");
}

/// 11.2 — `spawn_chat` session bind/resolve round-trip via `RuntimeAuthority`.
///
/// `spawn_chat` calls `authority.bind_session(flow_run_id, session_id)`
/// synchronously before spawning the async task, so `resolve_session` must
/// return the correct value without any sleep.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_chat_binds_session_in_authority() {
    let dir = tempdir().expect("tempdir");
    let (space_id, auth) = make_authority_with_space();

    let mock_llm = MockLlmFactory::new();
    // Queue a script so the spawned task doesn't hang waiting for LLM output.
    mock_llm.provider().push(
        MockScript::new()
            .delta("Chat response")
            .done(FinishReason::Stop),
    );

    let services = Arc::new(RuntimeServices {
        authority: auth.clone(),
        flow_registry: Arc::new(FlowRuntimeRegistry::default()),
        llm_factory: Arc::new(mock_llm),
        capability_factory: Arc::new(FakeCapabilityFactory),
        terminal_managers: Arc::new(Mutex::new(HashMap::new())),
        memory_manager: None,
    });

    let runner = AgentRunner::new(services);
    let flow_run_id = "test-flow-run-42".to_owned();
    let session_id = "sess-xyz".to_owned();

    let (doc_tx, _doc_rx) = tokio::sync::mpsc::channel(1);
    let run_ctx = RunContext {
        space_id,
        workspace_root: dir.path().to_path_buf(),
        flow_id: Some("my-flow".into()),
        flow_run_id: Some(flow_run_id.clone()),
        session_id: None,
        flow_runtime: None,
        doc_tx,
        llm_config: cronymax::llm::LlmConfig::OpenAi {
            base_url: "http://localhost".into(),
            api_key: Some("mock-key".into()),
            model: "mock-model".into(),
        },
        sandbox_tier: SandboxTier::Trusted,
        workspace_cache_dir: None,
    };

    runner.spawn_chat(run_ctx, session_id.clone(), "Hello".into());

    // bind_session is called synchronously before tokio::spawn, so resolve_session
    // should work immediately.
    let resolved = auth.resolve_session(&flow_run_id);
    assert_eq!(
        resolved.as_deref(),
        Some(session_id.as_str()),
        "resolve_session should return the session_id bound by spawn_chat"
    );
}

/// 4.9 — Supervisor loop calls `invoke_agent`, child completes, result appears
/// in Supervisor history.
///
/// Strategy:
/// * Push a script for the Supervisor's first turn that emits a tool call to
///   `invoke_agent`.
/// * Push a script for the Supervisor's second turn (after child completes)
///   that stops cleanly.
/// * For the child agent, push a single-turn script that stops cleanly.
/// * After `spawn_chat` completes, verify:
///   - at least 2 LLM calls from the Supervisor (tool call + final answer),
///   - the Supervisor's history contains a `tool` message with the child result.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn supervisor_invoke_agent_result_appears_in_history() {
    use cronymax::llm::messages::FinishReason;

    let dir = tempdir().expect("tempdir");

    // Create a __chat__.agent.yaml with kind: supervisor so the Supervisor
    // tools are registered by spawn_chat.
    let agents_dir = dir.path().join(".cronymax").join("agents");
    std::fs::create_dir_all(&agents_dir).expect("agents dir");
    std::fs::write(
        agents_dir.join("__chat__.agent.yaml"),
        "name: __chat__\nkind: supervisor\n",
    )
    .expect("write chat agent yaml");

    let (space_id, auth) = make_authority_with_space();

    let mock_llm = MockLlmFactory::new();

    // Supervisor turn 1: calls invoke_agent
    mock_llm.provider().push(
        MockScript::new()
            .tool_call(
                0,
                "call-1",
                "invoke_agent",
                r#"{"agent_id":"worker","goal":"do the work"}"#,
            )
            .done(FinishReason::ToolCalls),
    );
    // Child agent: single stop turn
    mock_llm.provider().push(
        MockScript::new()
            .delta("Child output done")
            .done(FinishReason::Stop),
    );
    // Supervisor turn 2: sees tool result, stops
    mock_llm
        .provider()
        .push(MockScript::new().delta("All done").done(FinishReason::Stop));

    let services = Arc::new(RuntimeServices {
        authority: auth.clone(),
        flow_registry: Arc::new(FlowRuntimeRegistry::default()),
        llm_factory: Arc::new(mock_llm.clone()),
        capability_factory: Arc::new(FakeCapabilityFactory),
        terminal_managers: Arc::new(Mutex::new(HashMap::new())),
        memory_manager: None,
    });

    let runner = AgentRunner::new(services);
    let (doc_tx, _doc_rx) = tokio::sync::mpsc::channel(1);
    let run_ctx = RunContext {
        space_id,
        workspace_root: dir.path().to_path_buf(),
        flow_id: None,
        flow_run_id: Some("sup-flow-run".into()),
        session_id: None,
        flow_runtime: None,
        doc_tx,
        llm_config: cronymax::llm::LlmConfig::OpenAi {
            base_url: "http://localhost".into(),
            api_key: Some("mock-key".into()),
            model: "mock-model".into(),
        },
        sandbox_tier: SandboxTier::Trusted,
        workspace_cache_dir: None,
    };

    let session_id = "sup-session".to_owned();
    runner.spawn_chat(run_ctx, session_id.clone(), "Delegate task".into());

    // Wait enough time for Supervisor + child to complete.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // The Supervisor should have made at least 2 LLM calls (tool call + final).
    // The child should have made 1 call.
    let requests = mock_llm.provider().requests();
    assert!(
        requests.len() >= 2,
        "expected at least 2 LLM requests (supervisor tool-call + final), got {}",
        requests.len()
    );

    // The second supervisor turn (index >= 2) must have a tool result message.
    let has_tool_result = requests
        .iter()
        .skip(1)
        .any(|req| req.messages.iter().any(|m| m.tool_call_id.is_some()));
    assert!(
        has_tool_result,
        "expected a tool result message in a subsequent supervisor turn"
    );
}
