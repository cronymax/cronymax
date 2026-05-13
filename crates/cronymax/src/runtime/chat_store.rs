//! Per-session chat file storage.
//!
//! Each session's data lives in two files under
//! `workspace_cache_dir/chats/<session_id>/`:
//!
//! * `meta.json`      — lightweight metadata (id, name, agent_id, timestamps…)
//! * `history.jsonl`  — append-only LLM context window; one JSON object per line
//!
//! ## Write discipline
//!
//! * `write_meta` is atomic: writes to a sibling `.tmp` file and renames over
//!   the target (same-directory rename is atomic on all supported filesystems).
//! * `append_turns` opens the file with `O_APPEND` and writes one JSON line per
//!   `ChatMessage`. Partial last lines (e.g. from a kill mid-write) are silently
//!   skipped by `load_history`.
//!
//! ## Thread safety
//!
//! `ChatStore` is `Clone + Send + Sync`. Multiple concurrent callers may call
//! `append_turns` on different session IDs safely (each session has its own
//! file). Concurrent appends to the *same* session from different threads are
//! race-free at the OS level thanks to `O_APPEND`, but the resulting line order
//! is non-deterministic; callers should avoid that pattern.

use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::llm::ChatMessage;
use crate::runtime::state::SessionId;

// ---------------------------------------------------------------------------
// SessionMeta
// ---------------------------------------------------------------------------

/// Lightweight metadata stored in `meta.json` alongside `history.jsonl`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_namespace: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

// ---------------------------------------------------------------------------
// ChatStoreError
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ChatStoreError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// ChatStore
// ---------------------------------------------------------------------------

/// File-backed store for per-session chat metadata and history.
///
/// Construct with `ChatStore::new(workspace_cache_dir)` — the root is
/// `<workspace_cache_dir>/chats/`.
#[derive(Clone, Debug)]
pub struct ChatStore {
    /// `<workspace_cache_dir>/chats/`
    root: PathBuf,
}

impl ChatStore {
    /// Create a `ChatStore` rooted at `<workspace_cache_dir>/chats/`.
    pub fn new(workspace_cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            root: workspace_cache_dir.into().join("chats"),
        }
    }

    /// Path to the session directory.
    fn session_dir(&self, session_id: &SessionId) -> PathBuf {
        self.root.join(session_id.0.as_str())
    }

    fn meta_path(&self, session_id: &SessionId) -> PathBuf {
        self.session_dir(session_id).join("meta.json")
    }

    fn history_path(&self, session_id: &SessionId) -> PathBuf {
        self.session_dir(session_id).join("history.jsonl")
    }

    // ── public API ──────────────────────────────────────────────────────────

    /// Atomically write (or overwrite) `meta.json` for `session_id`.
    ///
    /// Uses a sibling `.tmp` file + rename so crashes never leave a
    /// half-written `meta.json`.
    pub fn write_meta(
        &self,
        session_id: &SessionId,
        meta: &SessionMeta,
    ) -> Result<(), ChatStoreError> {
        let dir = self.session_dir(session_id);
        fs::create_dir_all(&dir)?;
        let path = self.meta_path(session_id);
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(meta)?;
        write_atomic(&tmp, &path, &bytes)?;
        Ok(())
    }

    /// Load `meta.json` for `session_id`. Returns `None` if the file does
    /// not exist.
    pub fn load_meta(&self, session_id: &SessionId) -> Option<SessionMeta> {
        let bytes = fs::read(self.meta_path(session_id)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Append `turns` to `history.jsonl` using `O_APPEND`.
    ///
    /// Each `ChatMessage` is serialized as a single-line JSON object followed
    /// by a newline. The append is O_APPEND so it is safe to call from
    /// multiple threads targeting different sessions concurrently.
    pub fn append_turns(
        &self,
        session_id: &SessionId,
        turns: &[ChatMessage],
    ) -> Result<(), ChatStoreError> {
        if turns.is_empty() {
            return Ok(());
        }
        let dir = self.session_dir(session_id);
        fs::create_dir_all(&dir)?;
        let path = self.history_path(session_id);
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        for turn in turns {
            let mut line = serde_json::to_string(turn)?;
            line.push('\n');
            file.write_all(line.as_bytes())?;
        }
        file.flush()?;
        Ok(())
    }

    /// Load all turns from `history.jsonl`.
    ///
    /// Skips the last line if it fails JSON parsing — this is the truncation
    /// guard for a process killed mid-write. All prior lines are returned.
    pub fn load_history(&self, session_id: &SessionId) -> Vec<ChatMessage> {
        let path = self.history_path(session_id);
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        let reader = io::BufReader::new(file);
        let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();
        let n = lines.len();
        let mut turns = Vec::with_capacity(n);
        for (i, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ChatMessage>(line) {
                Ok(msg) => turns.push(msg),
                Err(_) if i == n - 1 => {
                    // Last line failed to parse — treat as a truncated write,
                    // silently skip.
                    tracing::warn!(
                        session_id = %session_id,
                        "chat_store: skipping truncated last line in history.jsonl"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        session_id = %session_id,
                        line = i,
                        error = %e,
                        "chat_store: skipping malformed line in history.jsonl"
                    );
                }
            }
        }
        turns
    }

    /// Returns true if the session directory exists (i.e. the session has
    /// been written to at least once).
    pub fn session_exists(&self, session_id: &SessionId) -> bool {
        self.session_dir(session_id).exists()
    }

    /// Returns a reference to the root `chats/` directory.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

// ---------------------------------------------------------------------------
// Helper: atomic write via temp-file + rename
// ---------------------------------------------------------------------------

fn write_atomic(tmp: &Path, target: &Path, bytes: &[u8]) -> io::Result<()> {
    {
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(tmp, target)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ChatRole;
    use tempfile::tempdir;

    fn sid(s: &str) -> SessionId {
        SessionId(s.to_owned())
    }

    fn msg(role: ChatRole, text: &str) -> ChatMessage {
        match role {
            ChatRole::User => ChatMessage::user(text),
            ChatRole::Assistant => ChatMessage::assistant_text(text),
            ChatRole::System => ChatMessage::system(text),
            ChatRole::Tool => ChatMessage {
                role,
                content: Some(text.to_owned()),
                tool_calls: Vec::new(),
                tool_call_id: None,
                name: None,
            },
        }
    }

    // ── round-trip ──────────────────────────────────────────────────────────

    #[test]
    fn round_trip_meta() {
        let dir = tempdir().unwrap();
        let store = ChatStore::new(dir.path());
        let session = sid("sess-1");
        let meta = SessionMeta {
            id: "sess-1".to_owned(),
            name: Some("My Chat".to_owned()),
            created_at_ms: 1000,
            updated_at_ms: 2000,
            ..Default::default()
        };
        store.write_meta(&session, &meta).unwrap();
        let loaded = store.load_meta(&session).unwrap();
        assert_eq!(loaded.id, "sess-1");
        assert_eq!(loaded.name.as_deref(), Some("My Chat"));
    }

    #[test]
    fn round_trip_history() {
        let dir = tempdir().unwrap();
        let store = ChatStore::new(dir.path());
        let session = sid("sess-2");
        let turns = vec![
            msg(ChatRole::User, "hello"),
            msg(ChatRole::Assistant, "hi there"),
        ];
        store.append_turns(&session, &turns).unwrap();
        let loaded = store.load_history(&session);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].role, ChatRole::User);
        assert_eq!(loaded[1].role, ChatRole::Assistant);
    }

    #[test]
    fn append_is_cumulative() {
        let dir = tempdir().unwrap();
        let store = ChatStore::new(dir.path());
        let session = sid("sess-3");
        store
            .append_turns(&session, &[msg(ChatRole::User, "first")])
            .unwrap();
        store
            .append_turns(&session, &[msg(ChatRole::Assistant, "second")])
            .unwrap();
        let loaded = store.load_history(&session);
        assert_eq!(loaded.len(), 2);
    }

    // ── truncation recovery ─────────────────────────────────────────────────

    #[test]
    fn truncated_last_line_is_skipped() {
        let dir = tempdir().unwrap();
        let store = ChatStore::new(dir.path());
        let session = sid("sess-4");

        // Write one valid turn.
        store
            .append_turns(&session, &[msg(ChatRole::User, "good line")])
            .unwrap();

        // Manually append a truncated (invalid JSON) last line.
        let history_path = store.history_path(&session);
        let mut f = OpenOptions::new().append(true).open(&history_path).unwrap();
        f.write_all(b"{\"role\":\"user\",\"content\":\"TRUN")
            .unwrap();
        drop(f);

        let loaded = store.load_history(&session);
        // Only the valid line should come back.
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].role, ChatRole::User);
    }

    // ── missing session ─────────────────────────────────────────────────────

    #[test]
    fn missing_session_returns_empty() {
        let dir = tempdir().unwrap();
        let store = ChatStore::new(dir.path());
        let history = store.load_history(&sid("nonexistent"));
        assert!(history.is_empty());
        assert!(store.load_meta(&sid("nonexistent")).is_none());
    }

    // ── session_exists ───────────────────────────────────────────────────────

    #[test]
    fn session_exists_after_write() {
        let dir = tempdir().unwrap();
        let store = ChatStore::new(dir.path());
        let session = sid("sess-5");
        assert!(!store.session_exists(&session));
        store.write_meta(&session, &SessionMeta::default()).unwrap();
        assert!(store.session_exists(&session));
    }
}
