// SQLite persistence — long-term memory (FTS5), audit logs, budget tracking.
//
// Single database file at `~/.config/cronymax/cronymax.db` with WAL mode for
// concurrent read/write from UI + background tasks.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

// ─── Types ───────────────────────────────────────────────────────────────────

/// A long-term memory entry stored in SQLite with FTS5 full-text search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRow {
    pub id: i64,
    pub profile_id: String,
    pub content: String,
    pub tag: String,
    pub pinned: bool,
    pub token_count: i64,
    pub created_at: i64,
    pub last_used_at: i64,
    pub access_count: i64,
    /// Optional embedding vector (reserved for future semantic search).
    #[serde(skip)]
    pub embedding: Option<Vec<u8>>,
}

/// A sandbox audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub timestamp: i64,
    pub session_id: u32,
    pub action: String,
    pub detail: String,
    pub outcome: String,
    pub policy_name: Option<String>,
}

/// Budget usage snapshot for a period (day or session).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetRow {
    pub id: i64,
    pub profile_id: String,
    /// "session" or "daily"
    pub scope: String,
    /// Session ID (for session scope) or date string "YYYY-MM-DD" (for daily).
    pub scope_key: String,
    pub tokens_used: i64,
    pub turns_used: i64,
    pub updated_at: i64,
}

/// Onboarding wizard state for resumable multi-step channel setup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingWizardRow {
    pub id: i64,
    /// Current wizard step: "login", "create_app", "permissions", "callback".
    pub current_step: String,
    pub lark_app_id: Option<String>,
    pub oauth_token: Option<String>,
    pub tenant_id: Option<String>,
    pub is_admin: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

// ─── Database Store ──────────────────────────────────────────────────────────

/// Thread-safe SQLite database handle.
#[derive(Clone)]
pub struct DbStore {
    conn: Arc<Mutex<Connection>>,
}

impl DbStore {
    /// Open (or create) the database at the given path.
    pub fn open(path: &PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;

        // Enable WAL mode for concurrent readers.
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Open the database at the default config location.
    pub fn open_default() -> anyhow::Result<Self> {
        let path = crate::renderer::platform::config_dir().join("cronymax.db");
        Self::open(&path)
    }

    /// Get a lock on the underlying connection for direct queries.
    pub fn conn(
        &self,
    ) -> Result<
        std::sync::MutexGuard<'_, Connection>,
        std::sync::PoisonError<std::sync::MutexGuard<'_, Connection>>,
    > {
        self.conn.lock()
    }

    /// Run schema migrations.
    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;

        conn.execute_batch(
            "
            -- Long-term memory table
            CREATE TABLE IF NOT EXISTS memory (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                profile_id  TEXT    NOT NULL,
                content     TEXT    NOT NULL,
                tag         TEXT    NOT NULL DEFAULT 'general',
                pinned      INTEGER NOT NULL DEFAULT 0,
                token_count INTEGER NOT NULL DEFAULT 0,
                created_at  INTEGER NOT NULL,
                last_used_at INTEGER NOT NULL,
                access_count INTEGER NOT NULL DEFAULT 0,
                embedding   BLOB
            );

            -- FTS5 virtual table for full-text search on memory content
            CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
                content, tag,
                content=memory,
                content_rowid=id,
                tokenize='porter unicode61'
            );

            -- Triggers to keep FTS in sync
            CREATE TRIGGER IF NOT EXISTS memory_ai AFTER INSERT ON memory BEGIN
                INSERT INTO memory_fts(rowid, content, tag)
                VALUES (new.id, new.content, new.tag);
            END;
            CREATE TRIGGER IF NOT EXISTS memory_ad AFTER DELETE ON memory BEGIN
                INSERT INTO memory_fts(memory_fts, rowid, content, tag)
                VALUES ('delete', old.id, old.content, old.tag);
            END;
            CREATE TRIGGER IF NOT EXISTS memory_au AFTER UPDATE ON memory BEGIN
                INSERT INTO memory_fts(memory_fts, rowid, content, tag)
                VALUES ('delete', old.id, old.content, old.tag);
                INSERT INTO memory_fts(rowid, content, tag)
                VALUES (new.id, new.content, new.tag);
            END;

            -- Sandbox audit log
            CREATE TABLE IF NOT EXISTS audit_logs (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp   INTEGER NOT NULL,
                session_id  INTEGER NOT NULL,
                action      TEXT    NOT NULL,
                detail      TEXT    NOT NULL DEFAULT '',
                outcome     TEXT    NOT NULL DEFAULT 'ok',
                policy_name TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_logs(timestamp);
            CREATE INDEX IF NOT EXISTS idx_audit_session ON audit_logs(session_id);

            -- Budget usage tracking
            CREATE TABLE IF NOT EXISTS budget_usages (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                profile_id  TEXT    NOT NULL,
                scope       TEXT    NOT NULL,
                scope_key   TEXT    NOT NULL,
                tokens_used INTEGER NOT NULL DEFAULT 0,
                turns_used  INTEGER NOT NULL DEFAULT 0,
                updated_at  INTEGER NOT NULL,
                UNIQUE(profile_id, scope, scope_key)
            );
            CREATE INDEX IF NOT EXISTS idx_budget_profile ON budget_usages(profile_id, scope);

            -- Chat session persistence (for session history restore)
            CREATE TABLE IF NOT EXISTS chat_sessions (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id  INTEGER NOT NULL,
                profile_id  TEXT    NOT NULL,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL,
                title       TEXT    NOT NULL DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS chat_messages (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id  INTEGER NOT NULL REFERENCES chat_sessions(id),
                role        TEXT    NOT NULL,
                content     TEXT    NOT NULL,
                importance  TEXT    NOT NULL DEFAULT 'normal',
                token_count INTEGER NOT NULL DEFAULT 0,
                timestamp   INTEGER NOT NULL,
                tool_call_id TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_chat_msgs_session ON chat_messages(session_id);

            -- Starred blocks — user-highlighted messages that survive compaction
            CREATE TABLE IF NOT EXISTS starred_blocks (
                session_id  INTEGER NOT NULL,
                message_id  INTEGER NOT NULL,
                starred_at  INTEGER NOT NULL,
                PRIMARY KEY (session_id, message_id)
            );

            -- Onboarding wizard state persistence (resumable multi-step setup)
            CREATE TABLE IF NOT EXISTS onboarding_wizards (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                current_step TEXT NOT NULL DEFAULT 'login',
                lark_app_id TEXT,
                oauth_token TEXT,
                tenant_id   TEXT,
                is_admin    INTEGER NOT NULL DEFAULT 0,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL
            );
            ",
        )?;
        Ok(())
    }
}

mod memory;
pub mod memory_store;
mod operations;
pub use operations::WizardStepUpdate;
use operations::now_millis;
