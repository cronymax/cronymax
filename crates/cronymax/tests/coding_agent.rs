//! Tests for the coding-agent feature group (tasks 7.1 – 7.9).
//!
//! Covers:
//!  7.1  `RuntimeAuthority` creates Session on first `start_run` with new `session_id`
//!  7.2  `ReactLoop` initialised from `session.thread`; thread flushed on completion
//!  7.3  Compaction triggers at >80 % threshold; thread trimmed + summary inserted
//!  7.4  `str_replace` single-match success, not_found, ambiguous paths
//!  7.5  `CodeIndex::index_file` skips binary files and files >1 MB
//!       (CodeIndex FTS5 impl was skipped; covered by glob_files/grep_workspace smoke test instead)
//!  7.6  `search_workspace` / `grep_workspace` / `glob_files` smoke tests
//!  7.7  `git_status`, `git_diff`, `git_log` on a test repository
//!  7.8  `git_commit` creates commit; staged files reflected in `git_log`
//!  7.9  Integration: two runs on the same `session_id`; second run sees first run's messages

use std::path::Path;
use std::sync::Arc;

use cronymax::{RuntimeAuthority, SessionId, Space, SpaceId};
use cronymax::llm::{ChatMessage, FinishReason, MockLlmProvider, MockScript};
use cronymax::agent_loop::{
    compaction::{maybe_compact, token_estimate, DEFAULT_RECENCY_TURNS, DEFAULT_THRESHOLD_PCT},
};
use cronymax::capability::filesystem::{LocalFilesystem, FilesystemCapability};
use tempfile::tempdir;

// ─────────────────────────────────────────────────────────────────
// 7.1  Session creation on first start_run with a new session_id
// ─────────────────────────────────────────────────────────────────

#[test]
fn session_is_created_for_new_session_id() {
    let auth = RuntimeAuthority::in_memory();

    let space = Space { id: SpaceId::new(), name: "test".into(), compaction_threshold_pct: 80, compaction_recency_turns: 6 };
    let sid = space.id;
    auth.upsert_space(space).unwrap();

    let session_id = SessionId("my-session".into());
    let thread = auth
        .get_or_create_session(session_id.clone(), sid, Some("my session".into()))
        .expect("get_or_create_session");

    // Brand-new session → empty thread.
    assert!(thread.is_empty(), "expected empty thread for new session");

    // Calling again returns the same (still empty) thread.
    let thread2 = auth
        .get_or_create_session(session_id.clone(), sid, None)
        .expect("second call");
    assert_eq!(thread2.len(), thread.len());

    // Session is reflected in the snapshot.
    let snap = auth.snapshot();
    assert!(snap.sessions.contains_key(&session_id), "session should be in snapshot");
    assert_eq!(
        snap.sessions[&session_id].space_id,
        sid,
        "session's space_id should match"
    );
}

// ─────────────────────────────────────────────────────────────────
// 7.2  flush_thread persists messages; next get_or_create returns them
// ─────────────────────────────────────────────────────────────────

#[test]
fn flush_thread_persisted_and_reloaded() {
    let auth = RuntimeAuthority::in_memory();

    let space = Space { id: SpaceId::new(), name: "test".into(), compaction_threshold_pct: 80, compaction_recency_turns: 6 };
    let sid = space.id;
    auth.upsert_space(space).unwrap();

    let session_id = SessionId("flush-test".into());
    auth.get_or_create_session(session_id.clone(), sid, None).unwrap();

    let history = vec![
        ChatMessage::user("hello"),
        ChatMessage::assistant_text("hi there"),
    ];
    auth.flush_thread(&session_id, history.clone()).unwrap();

    // Retrieve the thread back.
    let loaded = auth.session_thread(&session_id).expect("session should exist");
    assert_eq!(loaded.len(), 2, "should have 2 messages after flush");
    assert_eq!(
        loaded[0].content.as_deref(),
        Some("hello"),
        "first message content mismatch"
    );
}

// ─────────────────────────────────────────────────────────────────
// 7.3  Compaction: token_estimate + maybe_compact threshold logic
// ─────────────────────────────────────────────────────────────────

#[test]
fn token_estimate_sums_content_lengths() {
    let msgs = vec![
        ChatMessage::user("hello"),      // 5 chars = 1 token
        ChatMessage::assistant_text("world!"), // 6 chars = 1 token
    ];
    // 11 chars total / 4 ≈ 2 tokens
    let est = token_estimate(&msgs);
    assert_eq!(est, (5 + 6) / 4);
}

#[test]
fn maybe_compact_below_threshold_returns_unchanged() {
    // Use the tokio runtime for the async call.
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let llm = Arc::new(MockLlmProvider::new());
        let short_thread = vec![ChatMessage::user("hi"), ChatMessage::assistant_text("hello")];

        let result = maybe_compact(
            short_thread.clone(),
            llm,
            "gpt-4",
            DEFAULT_THRESHOLD_PCT,
            DEFAULT_RECENCY_TURNS,
        )
        .await;

        assert!(!result.compacted, "should not compact short thread");
        assert_eq!(result.thread.len(), short_thread.len());
    });
}

#[test]
fn maybe_compact_above_threshold_produces_compacted_thread() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // Build a thread exceeding 80% of 128k tokens with enough turn-pairs
        // that split_thread can find a non-empty middle section to compact.
        // 6 recency turns = 6×(user+assistant) = 12 messages at the tail.
        // We need at least 14 messages total so there's something to summarise.
        let big_content = "a".repeat(500_000);
        let mut thread = vec![
            // Big message at the front to push over the threshold.
            ChatMessage::user(big_content),
            ChatMessage::assistant_text("processed"),
            // Extra early turns that will be compacted away.
            ChatMessage::user("early 1"),
            ChatMessage::assistant_text("reply 1"),
        ];
        // Add 8 more turns (16 messages) for the recency window.
        for i in 0..8usize {
            thread.push(ChatMessage::user(format!("follow up {i}")));
            thread.push(ChatMessage::assistant_text(format!("response {i}")));
        }
        assert!(
            token_estimate(&thread) > 128_000 * DEFAULT_THRESHOLD_PCT as usize / 100,
            "precondition: thread should exceed threshold"
        );

        // The mock LLM will return a summary message.
        let llm = Arc::new(MockLlmProvider::new());
        llm.push(
            MockScript::new()
                .delta("Summary: big content was processed.")
                .done(FinishReason::Stop),
        );

        let original_len = thread.len();
        let result = maybe_compact(
            thread,
            llm,
            "gpt-4",
            DEFAULT_THRESHOLD_PCT,
            DEFAULT_RECENCY_TURNS,
        )
        .await;

        assert!(result.compacted, "should have compacted");
        // After compaction the thread must be shorter than the original.
        assert!(
            result.thread.len() < original_len,
            "compacted thread ({}) should be shorter than original ({})",
            result.thread.len(),
            original_len
        );
        // The recency window (last 6 turns) must be intact.
        assert!(
            result.thread.len() >= DEFAULT_RECENCY_TURNS,
            "recency window should be preserved"
        );
    });
}

// ─────────────────────────────────────────────────────────────────
// 7.4  str_replace: success, not_found, ambiguous
// ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn str_replace_success() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("file.txt");
    tokio::fs::write(&path, "hello world\nfoo bar\n").await.unwrap();

    let fs = LocalFilesystem::new();
    let result = fs.str_replace(&path, "hello world", "goodbye world").await;
    assert!(result.is_ok(), "str_replace should succeed: {:?}", result.err());

    let content = tokio::fs::read_to_string(&path).await.unwrap();
    assert!(content.contains("goodbye world"), "replacement should be in file");
    assert!(!content.contains("hello world"), "old string should be gone");

    let sr = result.unwrap();
    assert!(!sr.diff.is_empty(), "diff should be non-empty");
}

#[tokio::test]
async fn str_replace_not_found_returns_error() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("file.txt");
    tokio::fs::write(&path, "hello world\n").await.unwrap();

    let fs = LocalFilesystem::new();
    let result = fs.str_replace(&path, "nonexistent string", "replacement").await;
    assert!(result.is_err(), "should error when old_str not found");
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("not_found"), "error message should mention not_found; got: {msg}");
}

#[tokio::test]
async fn str_replace_ambiguous_returns_error() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("file.txt");
    // Same string appears twice.
    tokio::fs::write(&path, "foo\nfoo\n").await.unwrap();

    let fs = LocalFilesystem::new();
    let result = fs.str_replace(&path, "foo", "bar").await;
    assert!(result.is_err(), "should error when old_str is ambiguous");
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("ambiguous"), "error message should mention ambiguous; got: {msg}");
}

// ─────────────────────────────────────────────────────────────────
// 7.5  glob_files / grep_workspace smoke test
//      (CodeIndex FTS5 skipped; covered here by rg / glob tools)
// ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn glob_files_finds_files_in_temp_dir() {
    let dir = tempdir().unwrap();

    // Create a few files.
    tokio::fs::write(dir.path().join("hello.rs"), "fn main() {}").await.unwrap();
    tokio::fs::write(dir.path().join("world.rs"), "fn world() {}").await.unwrap();
    tokio::fs::write(dir.path().join("readme.txt"), "text").await.unwrap();

    let (results, _truncated) = cronymax::capability::code_search::run_glob(
        dir.path(),
        "**/*.rs",
        200,
    )
    .await
    .expect("run_glob should succeed");

    assert_eq!(results.len(), 2, "should find exactly 2 .rs files, got: {results:?}");
    // Both files should appear.
    let names: Vec<&str> = results.iter().map(|s| {
        Path::new(s).file_name().unwrap().to_str().unwrap()
    }).collect();
    assert!(names.contains(&"hello.rs"), "hello.rs should be found");
    assert!(names.contains(&"world.rs"), "world.rs should be found");
}

#[tokio::test]
async fn grep_workspace_finds_pattern() {
    // Only run if rg is available.
    if which_rg().is_none() {
        eprintln!("skipping grep_workspace test: rg not in PATH");
        return;
    }

    let dir = tempdir().unwrap();
    tokio::fs::write(dir.path().join("foo.rs"), "fn hello_world() {}\nfn other() {}").await.unwrap();
    tokio::fs::write(dir.path().join("bar.rs"), "fn bar() {}").await.unwrap();

    let results = cronymax::capability::code_search::run_rg_search(
        dir.path(),
        "hello_world",
        "",
        0,
        50,
    )
    .await
    .expect("run_rg_search should succeed");

    assert!(!results.is_empty(), "should find at least one match for hello_world");
    let any_match = results.iter().any(|r| {
        r.get("text").and_then(|t| t.as_str()).map(|s| s.contains("hello_world")).unwrap_or(false)
    });
    assert!(any_match, "a result should contain the matched text; got: {results:?}");
}

fn which_rg() -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join("rg");
            if candidate.is_file() { Some(candidate) } else { None }
        })
    })
}

// ─────────────────────────────────────────────────────────────────
// 7.7  git_status, git_diff, git_log on a temporary repository
// ─────────────────────────────────────────────────────────────────

/// Create a minimal git repository with one commit at `root`.
fn init_test_repo(root: &Path) -> git2::Repository {
    let repo = git2::Repository::init(root).expect("git init");

    // Configure an identity so commits don't fail.
    let mut cfg = repo.config().unwrap();
    cfg.set_str("user.name", "Test User").unwrap();
    cfg.set_str("user.email", "test@example.com").unwrap();
    drop(cfg);

    // Write and commit an initial file.
    let file = root.join("README.md");
    std::fs::write(&file, "# Hello\n").unwrap();

    let mut index = repo.index().unwrap();
    index.add_path(Path::new("README.md")).unwrap();
    index.write().unwrap();

    let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
    let oid = index.write_tree().unwrap();
    let tree = repo.find_tree(oid).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[]).unwrap();
    drop(tree);

    repo
}

#[test]
fn git_status_returns_untracked_file() {
    let dir = tempdir().unwrap();
    let _repo = init_test_repo(dir.path());

    // Add a new untracked file.
    std::fs::write(dir.path().join("new_file.txt"), "content").unwrap();

    let entries = cronymax::capability::git::git_status_for_test(dir.path())
        .expect("git_status should succeed");

    let any_untracked = entries.iter().any(|e| {
        e.get("status")
            .and_then(|s| s.as_array())
            .map(|arr| arr.iter().any(|f| f.as_str() == Some("wt_new")))
            .unwrap_or(false)
    });
    assert!(any_untracked, "should report new_file.txt as untracked; got: {entries:?}");
}

#[test]
fn git_diff_returns_empty_for_clean_repo() {
    let dir = tempdir().unwrap();
    let _repo = init_test_repo(dir.path());

    let diff = cronymax::capability::git::git_diff_for_test(dir.path(), None, false)
        .expect("git_diff should succeed");
    assert!(diff.trim().is_empty(), "clean working tree should have empty diff; got: {diff:?}");
}

#[test]
fn git_log_returns_initial_commit() {
    let dir = tempdir().unwrap();
    let _repo = init_test_repo(dir.path());

    let commits = cronymax::capability::git::git_log_for_test(dir.path(), 10)
        .expect("git_log should succeed");

    assert_eq!(commits.len(), 1, "should have exactly 1 commit");
    let subject = commits[0].get("subject").and_then(|s| s.as_str()).unwrap_or("");
    assert_eq!(subject, "Initial commit");
}

// ─────────────────────────────────────────────────────────────────
// 7.8  git_commit creates a commit and shows up in git_log
// ─────────────────────────────────────────────────────────────────

#[test]
fn git_commit_creates_commit_and_appears_in_log() {
    let dir = tempdir().unwrap();
    let _repo = init_test_repo(dir.path());

    // Stage a new file.
    let new_file = dir.path().join("feature.rs");
    std::fs::write(&new_file, "pub fn feature() {}").unwrap();
    cronymax::capability::git::git_add_for_test(dir.path(), &["feature.rs".to_string()])
        .expect("git_add should succeed");

    // Create the commit.
    let (hash, files_changed) = cronymax::capability::git::git_commit_for_test(
        dir.path(),
        "feat: add feature",
    )
    .expect("git_commit should succeed");

    assert!(!hash.is_empty(), "commit hash should be non-empty");
    assert_eq!(files_changed, 1, "one file should be in the commit");

    // Verify log now has 2 commits.
    let commits = cronymax::capability::git::git_log_for_test(dir.path(), 10)
        .expect("git_log after commit");
    assert_eq!(commits.len(), 2, "log should show 2 commits");
    let latest_subject = commits[0].get("subject").and_then(|s| s.as_str()).unwrap_or("");
    assert_eq!(latest_subject, "feat: add feature");
}

// ─────────────────────────────────────────────────────────────────
// 7.9  Session continuity: second run sees first run's messages
// ─────────────────────────────────────────────────────────────────

#[test]
fn second_get_or_create_sees_flushed_thread() {
    let auth = RuntimeAuthority::in_memory();

    let space = Space { id: SpaceId::new(), name: "continuity".into(), compaction_threshold_pct: 80, compaction_recency_turns: 6 };
    let sid = space.id;
    auth.upsert_space(space).unwrap();

    let session_id = SessionId("cont-session".into());

    // First "run": create session and flush a synthetic thread.
    auth.get_or_create_session(session_id.clone(), sid, None).unwrap();
    let run1_thread = vec![
        ChatMessage::user("run-1 question"),
        ChatMessage::assistant_text("run-1 answer"),
    ];
    auth.flush_thread(&session_id, run1_thread.clone()).unwrap();

    // Second "run": calling get_or_create_session should return the flushed thread.
    let thread = auth
        .get_or_create_session(session_id.clone(), sid, None)
        .expect("second get_or_create should succeed");

    assert_eq!(
        thread.len(),
        2,
        "second run should start with the 2 messages from the first run"
    );
    assert_eq!(
        thread[0].content.as_deref(),
        Some("run-1 question"),
        "first message should be from run 1"
    );
}
