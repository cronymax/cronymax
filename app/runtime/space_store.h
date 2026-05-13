#pragma once

#include <cstdint>
#include <filesystem>
#include <functional>
#include <mutex>
#include <queue>
#include <string>
#include <thread>
#include <vector>

#include <sqlite3.h>

namespace cronymax {

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

struct SpaceRow {
  std::string id;
  std::string name;
  std::string root_path;
  std::string profile_id = "default";  // FK to ~/.cronymax/profiles/<id>.yaml
  int64_t created_at = 0;
  int64_t last_active = 0;
};

struct BrowserTabRow {
  int64_t id = 0;
  std::string space_id;
  std::string url;
  std::string title;
  bool is_pinned = false;
  int64_t last_accessed = 0;
};

struct TerminalBlockRow {
  int64_t id = 0;
  std::string space_id;
  std::string command;
  std::string output;
  int exit_code = -1;
  int64_t started_at = 0;
  int64_t ended_at = 0;
};

struct AgentTraceRow {
  int64_t id = 0;
  std::string space_id;
  std::string graph_id;
  std::string event_type;
  std::string payload;
  int64_t ts = 0;
};

struct LlmConfig {
  std::string base_url;
  std::string api_key;
};

// ---------------------------------------------------------------------------
// SpaceStore
// ---------------------------------------------------------------------------

class SpaceStore {
 public:
  SpaceStore();
  ~SpaceStore();

  SpaceStore(const SpaceStore&) = delete;
  SpaceStore& operator=(const SpaceStore&) = delete;

  // Open (or create) the database at the given path.
  bool Open(const std::filesystem::path& db_path);
  void Close();

  // Space CRUD
  bool CreateSpace(const SpaceRow& row);
  std::vector<SpaceRow> ListSpaces() const;
  bool UpdateLastActive(const std::string& space_id, int64_t ts);
  bool DeleteSpace(const std::string& space_id);

  // BrowserTab CRUD
  int64_t CreateTab(const BrowserTabRow& row);
  bool UpdateTab(const BrowserTabRow& row);
  bool DeleteTab(int64_t tab_id);
  std::vector<BrowserTabRow> ListTabsForSpace(
      const std::string& space_id) const;

  // TerminalBlock CRUD (writes are deferred to a background thread)
  // Returns placeholder id 0; actual id assigned by background writer.
  void CreateBlock(const TerminalBlockRow& row);
  void UpdateBlock(const TerminalBlockRow& row);
  std::vector<TerminalBlockRow> ListBlocksForSpace(const std::string& space_id,
                                                   int limit = 200) const;

  // AgentTrace append (also deferred to background thread)
  void AppendTrace(const AgentTraceRow& row);
  std::vector<AgentTraceRow> ListTracesForSpace(const std::string& space_id,
                                                int limit = 500) const;

  // LLM config
  bool SetLlmConfig(const LlmConfig& config);
  LlmConfig GetLlmConfig() const;

  // Generic kv_config helpers (used by FlowRuntime for the active-run
  // pointer per Space). Keys are expected to be namespaced by caller, e.g.
  // `space:<id>:active_run`.
  bool SetKv(const std::string& key, const std::string& value);
  std::string GetKv(const std::string& key) const;

  // Borrowed handle for subsystems (event_bus) that need to issue their
  // own SQL on the same database. The store retains ownership; callers
  // MUST NOT close the handle. Returns nullptr when not Open.
  sqlite3* raw_db() const { return db_; }

  // Mutex held during reads/writes from non-background callers. Subsystems
  // that share the connection must lock around their own statements.
  std::mutex& read_mutex() const { return read_mutex_; }

 private:
  void ApplySchema();

  // Background write queue
  void StartWriteThread();
  void StopWriteThread();
  void EnqueueWrite(std::function<void()> fn);
  void WriteLoop();

  sqlite3* db_ = nullptr;
  mutable std::mutex read_mutex_;

  std::thread write_thread_;
  std::mutex write_queue_mutex_;
  std::condition_variable write_cv_;
  std::queue<std::function<void()>> write_queue_;
  bool write_stop_ = false;
};

}  // namespace cronymax
