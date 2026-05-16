#include "runtime/space_store.h"

#include <cassert>
#include <chrono>
#include <sstream>

namespace cronymax {

namespace {

int64_t NowMs() {
  using namespace std::chrono;
  return duration_cast<milliseconds>(system_clock::now().time_since_epoch())
      .count();
}

// Helper: bind text (treats empty string as NULL for optional fields)
void BindText(sqlite3_stmt* stmt, int idx, const std::string& value) {
  sqlite3_bind_text(stmt, idx, value.c_str(), static_cast<int>(value.size()),
                    SQLITE_TRANSIENT);
}

}  // namespace

// ---------------------------------------------------------------------------
// Construction / destruction
// ---------------------------------------------------------------------------

SpaceStore::SpaceStore() = default;

SpaceStore::~SpaceStore() {
  Close();
}

bool SpaceStore::Open(const std::filesystem::path& db_path) {
  std::filesystem::create_directories(db_path.parent_path());

  const int rc = sqlite3_open(db_path.c_str(), &db_);
  if (rc != SQLITE_OK) {
    db_ = nullptr;
    return false;
  }

  // Enable WAL mode for concurrent reads during background writes.
  sqlite3_exec(db_, "PRAGMA journal_mode=WAL;", nullptr, nullptr, nullptr);
  sqlite3_exec(db_, "PRAGMA foreign_keys=ON;", nullptr, nullptr, nullptr);

  ApplySchema();
  StartWriteThread();
  return true;
}

void SpaceStore::Close() {
  StopWriteThread();
  if (db_) {
    sqlite3_close(db_);
    db_ = nullptr;
  }
}

void SpaceStore::ApplySchema() {
  const char* sql = R"(
    CREATE TABLE IF NOT EXISTS spaces (
      id          TEXT PRIMARY KEY,
      name        TEXT NOT NULL,
      root_path   TEXT NOT NULL,
      created_at  INTEGER NOT NULL DEFAULT 0,
      last_active INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS browser_tabs (
      id            INTEGER PRIMARY KEY AUTOINCREMENT,
      space_id      TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
      url           TEXT NOT NULL DEFAULT '',
      title         TEXT NOT NULL DEFAULT '',
      is_pinned     INTEGER NOT NULL DEFAULT 0,
      last_accessed INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS terminal_blocks (
      id         INTEGER PRIMARY KEY AUTOINCREMENT,
      space_id   TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
      command    TEXT NOT NULL DEFAULT '',
      output     TEXT NOT NULL DEFAULT '',
      exit_code  INTEGER NOT NULL DEFAULT -1,
      started_at INTEGER NOT NULL DEFAULT 0,
      ended_at   INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS agent_graphs (
      id         TEXT PRIMARY KEY,
      space_id   TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
      name       TEXT NOT NULL DEFAULT '',
      graph_json TEXT NOT NULL DEFAULT '{}',
      created_at INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS agent_traces (
      id         INTEGER PRIMARY KEY AUTOINCREMENT,
      space_id   TEXT NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
      graph_id   TEXT NOT NULL DEFAULT '',
      event_type TEXT NOT NULL DEFAULT '',
      payload    TEXT NOT NULL DEFAULT '{}',
      ts         INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS kv_config (
      key   TEXT PRIMARY KEY,
      value TEXT NOT NULL DEFAULT ''
    );

    -- agent-event-bus: typed event store + inbox triage materialisation.
    -- Append-only; UUIDv7 ids are sortable so pagination uses id < cursor.
    CREATE TABLE IF NOT EXISTS events (
      id        TEXT PRIMARY KEY,
      ts_ms     INTEGER NOT NULL,
      space_id  TEXT NOT NULL,
      flow_id   TEXT,
      run_id    TEXT,
      agent_id  TEXT,
      kind      TEXT NOT NULL,
      payload   TEXT NOT NULL DEFAULT '{}'
    );
    CREATE INDEX IF NOT EXISTS events_run_ts ON events (run_id, ts_ms);
    CREATE INDEX IF NOT EXISTS events_flow_ts ON events (flow_id, ts_ms);
    CREATE INDEX IF NOT EXISTS events_kind_idx ON events (kind);

    CREATE TABLE IF NOT EXISTS inbox (
      event_id     TEXT PRIMARY KEY REFERENCES events(id) ON DELETE CASCADE,
      state        TEXT NOT NULL DEFAULT 'unread',
      snooze_until INTEGER,
      flow_id      TEXT,
      kind         TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX IF NOT EXISTS inbox_state_idx ON inbox (state);
    CREATE INDEX IF NOT EXISTS inbox_flow_idx ON inbox (flow_id);

    CREATE TABLE IF NOT EXISTS notification_prefs (
      kind    TEXT PRIMARY KEY,
      enabled INTEGER NOT NULL DEFAULT 1
    );

    -- Workspace code index: FTS5 full-text search over file content.
    -- Each row represents one file; the content column holds the raw text.
    CREATE VIRTUAL TABLE IF NOT EXISTS code_index USING fts5(
      path UNINDEXED,
      content,
      tokenize = 'trigram'
    );

    -- Metadata table used by the incremental indexer to skip files that
    -- haven't changed since the last index pass.
    CREATE TABLE IF NOT EXISTS code_index_meta (
      path        TEXT PRIMARY KEY,
      mtime_ns    INTEGER NOT NULL DEFAULT 0,
      size_bytes  INTEGER NOT NULL DEFAULT 0,
      indexed_at  INTEGER NOT NULL DEFAULT 0
    );
  )";
  sqlite3_exec(db_, sql, nullptr, nullptr, nullptr);

  // Additive migration: add profile_id if the column doesn't exist yet.
  // SQLite ignores ALTER TABLE ADD COLUMN when the column already exists only
  // from SQLite 3.37+; use a try-and-ignore approach for older versions.
  sqlite3_exec(db_,
               "ALTER TABLE spaces ADD COLUMN "
               "profile_id TEXT NOT NULL DEFAULT 'default';",
               nullptr, nullptr,
               nullptr);  // error ignored — column may already exist
}

// ---------------------------------------------------------------------------
// Background write thread
// ---------------------------------------------------------------------------

void SpaceStore::StartWriteThread() {
  write_stop_ = false;
  write_thread_ = std::thread([this] { WriteLoop(); });
}

void SpaceStore::StopWriteThread() {
  {
    std::lock_guard<std::mutex> lock(write_queue_mutex_);
    write_stop_ = true;
  }
  write_cv_.notify_all();
  if (write_thread_.joinable()) {
    write_thread_.join();
  }
}

void SpaceStore::EnqueueWrite(std::function<void()> fn) {
  {
    std::lock_guard<std::mutex> lock(write_queue_mutex_);
    write_queue_.push(std::move(fn));
  }
  write_cv_.notify_one();
}

void SpaceStore::WriteLoop() {
  while (true) {
    std::function<void()> fn;
    {
      std::unique_lock<std::mutex> lock(write_queue_mutex_);
      write_cv_.wait(lock,
                     [this] { return write_stop_ || !write_queue_.empty(); });
      if (write_stop_ && write_queue_.empty()) {
        break;
      }
      fn = std::move(write_queue_.front());
      write_queue_.pop();
    }
    if (fn) {
      fn();
    }
  }
}

// ---------------------------------------------------------------------------
// Space CRUD
// ---------------------------------------------------------------------------

bool SpaceStore::CreateSpace(const SpaceRow& row) {
  if (!db_)
    return false;
  const char* sql =
      "INSERT OR IGNORE INTO spaces (id, name, root_path, profile_id, "
      "created_at, "
      "last_active) VALUES (?,?,?,?,?,?);";
  sqlite3_stmt* stmt = nullptr;
  sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
  BindText(stmt, 1, row.id);
  BindText(stmt, 2, row.name);
  BindText(stmt, 3, row.root_path);
  BindText(stmt, 4, row.profile_id.empty() ? "default" : row.profile_id);
  sqlite3_bind_int64(stmt, 5, row.created_at ? row.created_at : NowMs());
  sqlite3_bind_int64(stmt, 6, row.last_active ? row.last_active : NowMs());
  const int rc = sqlite3_step(stmt);
  sqlite3_finalize(stmt);
  return rc == SQLITE_DONE;
}

std::vector<SpaceRow> SpaceStore::ListSpaces() const {
  std::lock_guard<std::mutex> lock(read_mutex_);
  std::vector<SpaceRow> result;
  if (!db_)
    return result;
  const char* sql =
      "SELECT id, name, root_path, profile_id, created_at, last_active FROM "
      "spaces "
      "ORDER BY last_active DESC;";
  sqlite3_stmt* stmt = nullptr;
  sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
  while (sqlite3_step(stmt) == SQLITE_ROW) {
    SpaceRow row;
    row.id = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 0));
    row.name = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 1));
    row.root_path = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 2));
    const unsigned char* pid = sqlite3_column_text(stmt, 3);
    row.profile_id = pid ? reinterpret_cast<const char*>(pid) : "default";
    row.created_at = sqlite3_column_int64(stmt, 4);
    row.last_active = sqlite3_column_int64(stmt, 5);
    result.push_back(std::move(row));
  }
  sqlite3_finalize(stmt);
  return result;
}

bool SpaceStore::UpdateLastActive(const std::string& space_id, int64_t ts) {
  if (!db_)
    return false;
  const char* sql = "UPDATE spaces SET last_active=? WHERE id=?;";
  sqlite3_stmt* stmt = nullptr;
  sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
  sqlite3_bind_int64(stmt, 1, ts ? ts : NowMs());
  BindText(stmt, 2, space_id);
  const int rc = sqlite3_step(stmt);
  sqlite3_finalize(stmt);
  return rc == SQLITE_DONE;
}

bool SpaceStore::DeleteSpace(const std::string& space_id) {
  if (!db_)
    return false;
  // Cascades delete child rows (foreign_keys=ON).
  const char* sql = "DELETE FROM spaces WHERE id=?;";
  sqlite3_stmt* stmt = nullptr;
  sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
  BindText(stmt, 1, space_id);
  const int rc = sqlite3_step(stmt);
  sqlite3_finalize(stmt);
  return rc == SQLITE_DONE;
}

// ---------------------------------------------------------------------------
// BrowserTab CRUD
// ---------------------------------------------------------------------------

int64_t SpaceStore::CreateTab(const BrowserTabRow& row) {
  if (!db_)
    return 0;
  const char* sql =
      "INSERT INTO browser_tabs (space_id, url, title, is_pinned, "
      "last_accessed) VALUES (?,?,?,?,?);";
  sqlite3_stmt* stmt = nullptr;
  sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
  BindText(stmt, 1, row.space_id);
  BindText(stmt, 2, row.url);
  BindText(stmt, 3, row.title);
  sqlite3_bind_int(stmt, 4, row.is_pinned ? 1 : 0);
  sqlite3_bind_int64(stmt, 5, row.last_accessed ? row.last_accessed : NowMs());
  sqlite3_step(stmt);
  sqlite3_finalize(stmt);
  return sqlite3_last_insert_rowid(db_);
}

bool SpaceStore::UpdateTab(const BrowserTabRow& row) {
  if (!db_)
    return false;
  const char* sql =
      "UPDATE browser_tabs SET url=?, title=?, is_pinned=?, last_accessed=? "
      "WHERE id=?;";
  sqlite3_stmt* stmt = nullptr;
  sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
  BindText(stmt, 1, row.url);
  BindText(stmt, 2, row.title);
  sqlite3_bind_int(stmt, 3, row.is_pinned ? 1 : 0);
  sqlite3_bind_int64(stmt, 4, row.last_accessed ? row.last_accessed : NowMs());
  sqlite3_bind_int64(stmt, 5, row.id);
  const int rc = sqlite3_step(stmt);
  sqlite3_finalize(stmt);
  return rc == SQLITE_DONE;
}

bool SpaceStore::DeleteTab(int64_t tab_id) {
  if (!db_)
    return false;
  const char* sql = "DELETE FROM browser_tabs WHERE id=?;";
  sqlite3_stmt* stmt = nullptr;
  sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
  sqlite3_bind_int64(stmt, 1, tab_id);
  const int rc = sqlite3_step(stmt);
  sqlite3_finalize(stmt);
  return rc == SQLITE_DONE;
}

std::vector<BrowserTabRow> SpaceStore::ListTabsForSpace(
    const std::string& space_id) const {
  std::lock_guard<std::mutex> lock(read_mutex_);
  std::vector<BrowserTabRow> result;
  if (!db_)
    return result;
  const char* sql =
      "SELECT id, space_id, url, title, is_pinned, last_accessed "
      "FROM browser_tabs WHERE space_id=? ORDER BY is_pinned DESC, id ASC;";
  sqlite3_stmt* stmt = nullptr;
  sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
  BindText(stmt, 1, space_id);
  while (sqlite3_step(stmt) == SQLITE_ROW) {
    BrowserTabRow row;
    row.id = sqlite3_column_int64(stmt, 0);
    row.space_id = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 1));
    row.url = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 2));
    row.title = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 3));
    row.is_pinned = sqlite3_column_int(stmt, 4) != 0;
    row.last_accessed = sqlite3_column_int64(stmt, 5);
    result.push_back(std::move(row));
  }
  sqlite3_finalize(stmt);
  return result;
}

// ---------------------------------------------------------------------------
// TerminalBlock (background writes)
// ---------------------------------------------------------------------------

void SpaceStore::CreateBlock(const TerminalBlockRow& row) {
  EnqueueWrite([this, row] {
    if (!db_)
      return;
    const char* sql =
        "INSERT INTO terminal_blocks (space_id, command, output, exit_code, "
        "started_at, ended_at) VALUES (?,?,?,?,?,?);";
    sqlite3_stmt* stmt = nullptr;
    sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
    BindText(stmt, 1, row.space_id);
    BindText(stmt, 2, row.command);
    BindText(stmt, 3, row.output);
    sqlite3_bind_int(stmt, 4, row.exit_code);
    sqlite3_bind_int64(stmt, 5, row.started_at);
    sqlite3_bind_int64(stmt, 6, row.ended_at);
    sqlite3_step(stmt);
    sqlite3_finalize(stmt);
  });
}

void SpaceStore::UpdateBlock(const TerminalBlockRow& row) {
  EnqueueWrite([this, row] {
    if (!db_)
      return;
    const char* sql =
        "UPDATE terminal_blocks SET output=?, exit_code=?, ended_at=? WHERE "
        "id=?;";
    sqlite3_stmt* stmt = nullptr;
    sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
    BindText(stmt, 1, row.output);
    sqlite3_bind_int(stmt, 2, row.exit_code);
    sqlite3_bind_int64(stmt, 3, row.ended_at);
    sqlite3_bind_int64(stmt, 4, row.id);
    sqlite3_step(stmt);
    sqlite3_finalize(stmt);
  });
}

std::vector<TerminalBlockRow> SpaceStore::ListBlocksForSpace(
    const std::string& space_id,
    int limit) const {
  std::lock_guard<std::mutex> lock(read_mutex_);
  std::vector<TerminalBlockRow> result;
  if (!db_)
    return result;
  const char* sql =
      "SELECT id, space_id, command, output, exit_code, started_at, ended_at "
      "FROM terminal_blocks WHERE space_id=? ORDER BY id DESC LIMIT ?;";
  sqlite3_stmt* stmt = nullptr;
  sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
  BindText(stmt, 1, space_id);
  sqlite3_bind_int(stmt, 2, limit);
  while (sqlite3_step(stmt) == SQLITE_ROW) {
    TerminalBlockRow row;
    row.id = sqlite3_column_int64(stmt, 0);
    row.space_id = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 1));
    row.command = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 2));
    row.output = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 3));
    row.exit_code = sqlite3_column_int(stmt, 4);
    row.started_at = sqlite3_column_int64(stmt, 5);
    row.ended_at = sqlite3_column_int64(stmt, 6);
    result.push_back(std::move(row));
  }
  sqlite3_finalize(stmt);
  // Reverse so oldest first.
  std::reverse(result.begin(), result.end());
  return result;
}

// ---------------------------------------------------------------------------
// AgentTrace (background writes)
// ---------------------------------------------------------------------------

void SpaceStore::AppendTrace(const AgentTraceRow& row) {
  EnqueueWrite([this, row] {
    if (!db_)
      return;
    const char* sql =
        "INSERT INTO agent_traces (space_id, graph_id, event_type, payload, "
        "ts) VALUES (?,?,?,?,?);";
    sqlite3_stmt* stmt = nullptr;
    sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
    BindText(stmt, 1, row.space_id);
    BindText(stmt, 2, row.graph_id);
    BindText(stmt, 3, row.event_type);
    BindText(stmt, 4, row.payload);
    sqlite3_bind_int64(stmt, 5, row.ts ? row.ts : NowMs());
    sqlite3_step(stmt);
    sqlite3_finalize(stmt);
  });
}

std::vector<AgentTraceRow> SpaceStore::ListTracesForSpace(
    const std::string& space_id,
    int limit) const {
  std::lock_guard<std::mutex> lock(read_mutex_);
  std::vector<AgentTraceRow> result;
  if (!db_)
    return result;
  const char* sql =
      "SELECT id, space_id, graph_id, event_type, payload, ts "
      "FROM agent_traces WHERE space_id=? ORDER BY id DESC LIMIT ?;";
  sqlite3_stmt* stmt = nullptr;
  sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
  BindText(stmt, 1, space_id);
  sqlite3_bind_int(stmt, 2, limit);
  while (sqlite3_step(stmt) == SQLITE_ROW) {
    AgentTraceRow row;
    row.id = sqlite3_column_int64(stmt, 0);
    row.space_id = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 1));
    row.graph_id = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 2));
    row.event_type =
        reinterpret_cast<const char*>(sqlite3_column_text(stmt, 3));
    row.payload = reinterpret_cast<const char*>(sqlite3_column_text(stmt, 4));
    row.ts = sqlite3_column_int64(stmt, 5);
    result.push_back(std::move(row));
  }
  sqlite3_finalize(stmt);
  std::reverse(result.begin(), result.end());
  return result;
}

// ---------------------------------------------------------------------------
// LLM config
// ---------------------------------------------------------------------------

bool SpaceStore::SetLlmConfig(const LlmConfig& config) {
  if (!db_)
    return false;
  const char* sql =
      "INSERT OR REPLACE INTO kv_config (key, value) VALUES (?,?);";
  auto exec = [&](const std::string& key, const std::string& val) {
    sqlite3_stmt* stmt = nullptr;
    sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
    BindText(stmt, 1, key);
    BindText(stmt, 2, val);
    sqlite3_step(stmt);
    sqlite3_finalize(stmt);
  };
  exec("llm.base_url", config.base_url);
  exec("llm.api_key", config.api_key);
  return true;
}

LlmConfig SpaceStore::GetLlmConfig() const {
  std::lock_guard<std::mutex> lock(read_mutex_);
  LlmConfig config;
  if (!db_)
    return config;
  const char* sql =
      "SELECT key, value FROM kv_config WHERE key IN "
      "('llm.base_url','llm.api_key');";
  sqlite3_stmt* stmt = nullptr;
  sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);
  while (sqlite3_step(stmt) == SQLITE_ROW) {
    const std::string key =
        reinterpret_cast<const char*>(sqlite3_column_text(stmt, 0));
    const std::string val =
        reinterpret_cast<const char*>(sqlite3_column_text(stmt, 1));
    if (key == "llm.base_url")
      config.base_url = val;
    else if (key == "llm.api_key")
      config.api_key = val;
  }
  sqlite3_finalize(stmt);
  return config;
}

bool SpaceStore::SetKv(const std::string& key, const std::string& value) {
  if (!db_)
    return false;
  const char* sql =
      "INSERT OR REPLACE INTO kv_config (key, value) VALUES (?,?);";
  sqlite3_stmt* stmt = nullptr;
  if (sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr) != SQLITE_OK) {
    return false;
  }
  BindText(stmt, 1, key);
  BindText(stmt, 2, value);
  const bool ok = sqlite3_step(stmt) == SQLITE_DONE;
  sqlite3_finalize(stmt);
  return ok;
}

std::string SpaceStore::GetKv(const std::string& key) const {
  std::lock_guard<std::mutex> lock(read_mutex_);
  if (!db_)
    return std::string();
  const char* sql = "SELECT value FROM kv_config WHERE key = ?;";
  sqlite3_stmt* stmt = nullptr;
  if (sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr) != SQLITE_OK) {
    return std::string();
  }
  BindText(stmt, 1, key);
  std::string out;
  if (sqlite3_step(stmt) == SQLITE_ROW) {
    const auto* txt =
        reinterpret_cast<const char*>(sqlite3_column_text(stmt, 0));
    if (txt)
      out = txt;
  }
  sqlite3_finalize(stmt);
  return out;
}

}  // namespace cronymax
