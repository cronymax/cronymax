//! Unit tests for the agent middleware chain.

use cronymax::ai::context::{ChatMessage, MessageImportance, MessageRole};
use cronymax::ai::middleware::*;
use cronymax::ai::stream::ToolCallInfo;

fn msg(role: MessageRole, content: &str) -> ChatMessage {
    ChatMessage::new(role, content.to_string(), MessageImportance::Normal, 10)
}

fn sys(content: &str) -> ChatMessage {
    ChatMessage::new(
        MessageRole::System,
        content.to_string(),
        MessageImportance::System,
        10,
    )
}

// ── DanglingToolCallMiddleware ───────────────────────────────────────────────

#[test]
fn dangling_tool_call_injects_for_orphaned_calls() {
    let mw = DanglingToolCallMiddleware;
    let mut ctx = MiddlewareContext::new(0, 10, 1000, 128_000);

    let mut assistant = msg(MessageRole::Assistant, "Checking...");
    assistant.tool_calls = vec![
        ToolCallInfo {
            id: "c1".into(),
            function_name: "search".into(),
            arguments: "{}".into(),
        },
        ToolCallInfo {
            id: "c2".into(),
            function_name: "fetch".into(),
            arguments: "{}".into(),
        },
    ];

    let mut result = msg(MessageRole::Tool, "search done");
    result.tool_call_id = Some("c1".into());

    let mut messages = vec![
        sys("system"),
        msg(MessageRole::User, "hi"),
        assistant,
        result,
    ];

    mw.before_llm(&mut messages, &mut ctx);

    // c2 should have a placeholder.
    assert_eq!(messages.len(), 5);
    assert_eq!(messages[4].role, MessageRole::Tool);
    assert_eq!(messages[4].tool_call_id.as_deref(), Some("c2"));
}

#[test]
fn dangling_no_false_positives() {
    let mw = DanglingToolCallMiddleware;
    let mut ctx = MiddlewareContext::new(0, 10, 1000, 128_000);

    let mut assistant = msg(MessageRole::Assistant, "");
    assistant.tool_calls = vec![ToolCallInfo {
        id: "c1".into(),
        function_name: "s".into(),
        arguments: "{}".into(),
    }];

    let mut result = msg(MessageRole::Tool, "done");
    result.tool_call_id = Some("c1".into());

    let mut messages = vec![sys("s"), assistant, result];
    let orig_len = messages.len();

    mw.before_llm(&mut messages, &mut ctx);
    assert_eq!(messages.len(), orig_len);
}

// ── ContextSummarizationMiddleware ──────────────────────────────────────────

#[test]
fn summarization_downgrades_when_over_threshold() {
    let mw = ContextSummarizationMiddleware {
        trigger_ratio: 0.70,
        keep_recent: 2,
    };
    let mut ctx = MiddlewareContext::new(0, 10, 100_000, 128_000); // ~78%

    let mut messages = vec![
        sys("system"),
        msg(MessageRole::User, "old 1"),
        msg(MessageRole::Assistant, "old 2"),
        msg(MessageRole::User, "old 3"),
        msg(MessageRole::User, "recent"),
        msg(MessageRole::Assistant, "recent resp"),
    ];

    mw.before_llm(&mut messages, &mut ctx);

    // System untouched.
    assert_eq!(messages[0].importance, MessageImportance::System);
    // Old messages downgraded.
    assert_eq!(messages[1].importance, MessageImportance::Ephemeral);
    assert_eq!(messages[2].importance, MessageImportance::Ephemeral);
    assert_eq!(messages[3].importance, MessageImportance::Ephemeral);
    // Recent kept.
    assert_eq!(messages[4].importance, MessageImportance::Normal);
    assert_eq!(messages[5].importance, MessageImportance::Normal);
}

#[test]
fn summarization_skips_when_below_threshold() {
    let mw = ContextSummarizationMiddleware {
        trigger_ratio: 0.75,
        keep_recent: 2,
    };
    let mut ctx = MiddlewareContext::new(0, 10, 50_000, 128_000); // ~39%

    let mut messages = vec![
        sys("system"),
        msg(MessageRole::User, "msg"),
        msg(MessageRole::Assistant, "resp"),
    ];

    mw.before_llm(&mut messages, &mut ctx);

    // Nothing changed.
    assert_eq!(messages[1].importance, MessageImportance::Normal);
    assert_eq!(messages[2].importance, MessageImportance::Normal);
}

// ── ToolRoundGuardMiddleware ────────────────────────────────────────────────

#[test]
fn tool_guard_aborts_at_limit() {
    let mw = ToolRoundGuardMiddleware;
    let mut ctx = MiddlewareContext::new(10, 10, 1000, 128_000);
    let mut messages = vec![sys("s")];

    mw.before_llm(&mut messages, &mut ctx);
    assert!(ctx.abort);
}

#[test]
fn tool_guard_passes_below_limit() {
    let mw = ToolRoundGuardMiddleware;
    let mut ctx = MiddlewareContext::new(5, 10, 1000, 128_000);
    let mut messages = vec![sys("s")];

    mw.before_llm(&mut messages, &mut ctx);
    assert!(!ctx.abort);
}

// ── SubagentLimitMiddleware ─────────────────────────────────────────────────

#[test]
fn subagent_limit_truncates_excess() {
    let mw = SubagentLimitMiddleware { max_concurrent: 2 };
    let mut ctx = MiddlewareContext::new(0, 10, 1000, 128_000);
    let tcs = vec![
        ToolCallInfo {
            id: "1".into(),
            function_name: "a".into(),
            arguments: "{}".into(),
        },
        ToolCallInfo {
            id: "2".into(),
            function_name: "b".into(),
            arguments: "{}".into(),
        },
        ToolCallInfo {
            id: "3".into(),
            function_name: "c".into(),
            arguments: "{}".into(),
        },
    ];

    let outcome = mw.after_llm("", &tcs, &mut ctx);
    let overridden = outcome.override_tool_calls.unwrap();
    assert_eq!(overridden.len(), 2);
}

// ── Full Chain Integration ──────────────────────────────────────────────────

#[test]
fn chain_runs_all_middlewares() {
    let chain = MiddlewareChain::build_default(MiddlewareChainConfig::default());

    // Normal case — should proceed.
    let mut ctx = MiddlewareContext::new(0, 10, 50_000, 128_000);
    let mut messages = vec![sys("test"), msg(MessageRole::User, "hi")];

    assert!(chain.run_before_llm(&mut messages, &mut ctx));
    assert!(!ctx.abort);
}

#[test]
fn chain_aborts_on_round_limit() {
    let chain = MiddlewareChain::build_default(MiddlewareChainConfig::default());

    // At round limit — should abort.
    let mut ctx = MiddlewareContext::new(10, 10, 50_000, 128_000);
    let mut messages = vec![sys("test")];

    assert!(!chain.run_before_llm(&mut messages, &mut ctx));
    assert!(ctx.abort);
}

#[test]
fn chain_fixes_dangling_then_summarizes() {
    let chain = MiddlewareChain::build_default(MiddlewareChainConfig {
        summarization_trigger_ratio: 0.50,
        summarization_keep_recent: 2,
        max_concurrent_subagents: 3,
    });

    let mut ctx = MiddlewareContext::new(0, 10, 80_000, 128_000); // 62% > 50%

    let mut assistant = msg(MessageRole::Assistant, "");
    assistant.tool_calls = vec![ToolCallInfo {
        id: "orphan".into(),
        function_name: "x".into(),
        arguments: "{}".into(),
    }];

    let mut messages = vec![
        sys("system"),
        msg(MessageRole::User, "old"),
        msg(MessageRole::Assistant, "old resp"),
        assistant,
        // No tool result for "orphan" — dangling!
        msg(MessageRole::User, "recent"),
        msg(MessageRole::Assistant, "recent resp"),
    ];

    chain.run_before_llm(&mut messages, &mut ctx);

    // Dangling call should be fixed.
    assert!(
        messages
            .iter()
            .any(|m| m.tool_call_id.as_deref() == Some("orphan") && m.role == MessageRole::Tool)
    );
    // Old messages should be downgraded.
    assert_eq!(messages[1].importance, MessageImportance::Ephemeral);
}
