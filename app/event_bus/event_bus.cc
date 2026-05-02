#include "event_bus/event_bus.h"

#include <chrono>
#include <fstream>
#include <sqlite3.h>
#include <system_error>

#include <nlohmann/json.hpp>

#include "common/uuid_v7.h"
#include "workspace/space_store.h"

// MIGRATION (rust-runtime-migration, group 10): the host-owned event
// bus and the SQLite events table are being removed. Semantic events
// (run.*, agent.*, tool.*, review.*, document.*) become Rust runtime
// emissions consumed via `cronymax::RuntimeAuthority::run_history`
// and the runtime event subscription topology over GIPS. After
// cutover, this file persists only shell/UI metadata events (or is
// deleted entirely) — no new semantic event kinds may be added here.
// See `openspec/changes/rust-runtime-migration/tasks.md` 10.2.

namespace cronymax::event_bus {

namespace {

namespace fs = std::filesystem;

long long NowMs() {
  using namespace std::chrono;
  return duration_cast<milliseconds>(system_clock::now().time_since_epoch())
      .count();
}

void BindText(sqlite3_stmt* stmt, int idx, const std::string& s) {
  if (s.empty()) {
    sqlite3_bind_null(stmt, idx);
  } else {
    sqlite3_bind_text(stmt, idx, s.c_str(),
                      static_cast<int>(s.size()), SQLITE_TRANSIENT);
  }
}

std::string ColText(sqlite3_stmt* stmt, int idx) {
  const auto* p = sqlite3_column_text(stmt, idx);
  if (!p) return {};
  return reinterpret_cast<const char*>(p);
}

}  // namespace

const char* InboxStateToString(InboxState s) {
  switch (s) {
    case InboxState::kUnread: return "unread";
    case InboxState::kRead: return "read";
    case InboxState::kSnoozed: return "snoozed";
  }
  return "unread";
}

bool InboxStateFromString(const std::string& s, InboxState* out) {
  if (s == "unread") { *out = InboxState::kUnread; return true; }
  if (s == "read") { *out = InboxState::kRead; return true; }
  if (s == "snoozed") { *out = InboxState::kSnoozed; return true; }
  return false;
}

EventBus::EventBus(SpaceStore* store, std::string space_id,
                   fs::path space_root)
    : store_(store),
      space_id_(std::move(space_id)),
      space_root_(std::move(space_root)) {}

EventBus::~EventBus() = default;

bool EventBus::ScopeMatches(const Scope& s, const AppEvent& e) const {
  if (!s.flow_id.empty() && s.flow_id != e.flow_id) return false;
  if (!s.run_id.empty() && s.run_id != e.run_id) return false;
  return true;
}

bool EventBus::TriageNeedsAction(const AppEvent& e) const {
  switch (e.kind) {
    case AppEventKind::kReviewEvent: {
      return e.payload.contains("verdict") && e.payload["verdict"].is_string() &&
             e.payload["verdict"].get<std::string>() == "request_changes" &&
             e.payload.contains("reviewer") && e.payload["reviewer"].is_string() &&
             e.payload["reviewer"].get<std::string>() == "human";
    }
    case AppEventKind::kText: {
      if (!e.payload.contains("mentions") || !e.payload["mentions"].is_array())
        return false;
      for (const auto& v : e.payload["mentions"]) {
        if (v.is_string() && v.get<std::string>() == kCurrentUserId) return true;
      }
      return false;
    }
    case AppEventKind::kError: {
      return !(e.payload.contains("scope") && e.payload["scope"].is_string() &&
               e.payload["scope"].get<std::string>() == "tool");
    }
    case AppEventKind::kSystem: {
      return e.payload.contains("subkind") && e.payload["subkind"].is_string() &&
             e.payload["subkind"].get<std::string>() == "run_paused" &&
             e.payload.contains("cause") && e.payload["cause"].is_string() &&
             e.payload["cause"].get<std::string>() == "human_approval";
    }
    default:
      return false;
  }
}

void EventBus::WriteJsonlLine(const AppEvent& e) const {
  if (e.run_id.empty() || space_root_.empty()) return;
  std::error_code ec;
  // Layout matches src/flow/workspace_layout.cc:
  //   <space>/.cronymax/flows/<flow_id>/runs/<run_id>/trace.jsonl
  if (e.flow_id.empty()) return;
  const auto dir = space_root_ / ".cronymax" / "flows" / e.flow_id /
                   "runs" / e.run_id;
  fs::create_directories(dir, ec);
  std::ofstream out(dir / "trace.jsonl",
                    std::ios::binary | std::ios::app);
  if (!out) return;
  const auto line = e.ToJson() + "\n";
  out.write(line.data(), static_cast<std::streamsize>(line.size()));
}

std::string EventBus::Append(AppEvent evt) {
  if (evt.id.empty()) evt.id = MakeUuidV7();
  if (evt.ts_ms == 0) evt.ts_ms = NowMs();
  if (evt.space_id.empty()) evt.space_id = space_id_;

  // Single critical section: SQL insert + JSONL write + triage + dispatch.
  std::lock_guard<std::mutex> lock(mu_);

  if (auto* db = store_ ? store_->raw_db() : nullptr) {
    std::lock_guard<std::mutex> sl(store_->read_mutex());
    static constexpr const char* kInsert =
        "INSERT INTO events (id, ts_ms, space_id, flow_id, run_id, agent_id, "
        "kind, payload) VALUES (?,?,?,?,?,?,?,?);";
    sqlite3_stmt* stmt = nullptr;
    if (sqlite3_prepare_v2(db, kInsert, -1, &stmt, nullptr) == SQLITE_OK) {
      BindText(stmt, 1, evt.id);
      sqlite3_bind_int64(stmt, 2, evt.ts_ms);
      BindText(stmt, 3, evt.space_id);
      BindText(stmt, 4, evt.flow_id);
      BindText(stmt, 5, evt.run_id);
      BindText(stmt, 6, evt.agent_id);
      const std::string kind_s = AppEventKindToString(evt.kind);
      BindText(stmt, 7, kind_s);
      const std::string payload_s =
          evt.payload.is_object() ? evt.payload.dump() : "{}";
      BindText(stmt, 8, payload_s);
      sqlite3_step(stmt);
      sqlite3_finalize(stmt);
    }

    if (TriageNeedsAction(evt)) {
      static constexpr const char* kInbox =
          "INSERT OR IGNORE INTO inbox (event_id, state, flow_id, kind) "
          "VALUES (?, 'unread', ?, ?);";
      sqlite3_stmt* ist = nullptr;
      if (sqlite3_prepare_v2(db, kInbox, -1, &ist, nullptr) == SQLITE_OK) {
        BindText(ist, 1, evt.id);
        BindText(ist, 2, evt.flow_id);
        const std::string kind_s = AppEventKindToString(evt.kind);
        BindText(ist, 3, kind_s);
        sqlite3_step(ist);
        sqlite3_finalize(ist);
      }
    }
  }

  WriteJsonlLine(evt);

  for (const auto& s : subs_) {
    if (ScopeMatches(s.scope, evt)) s.cb(evt);
  }
  return evt.id;
}

ListResult EventBus::List(const ListQuery& q) const {
  ListResult out;
  auto* db = store_ ? store_->raw_db() : nullptr;
  if (!db) return out;
  const int limit = std::min(std::max(q.limit, 1), 1000);
  std::string sql =
      "SELECT id, ts_ms, space_id, flow_id, run_id, agent_id, kind, payload "
      "FROM events WHERE 1=1";
  if (!q.scope.flow_id.empty()) sql += " AND flow_id = ?";
  if (!q.scope.run_id.empty()) sql += " AND run_id = ?";
  if (!q.before_id.empty()) sql += " AND id < ?";
  sql += " ORDER BY id DESC LIMIT ?;";

  std::lock_guard<std::mutex> sl(store_->read_mutex());
  sqlite3_stmt* stmt = nullptr;
  if (sqlite3_prepare_v2(db, sql.c_str(), -1, &stmt, nullptr) != SQLITE_OK) {
    return out;
  }
  int idx = 1;
  if (!q.scope.flow_id.empty()) BindText(stmt, idx++, q.scope.flow_id);
  if (!q.scope.run_id.empty()) BindText(stmt, idx++, q.scope.run_id);
  if (!q.before_id.empty()) BindText(stmt, idx++, q.before_id);
  sqlite3_bind_int(stmt, idx, limit);

  while (sqlite3_step(stmt) == SQLITE_ROW) {
    AppEvent e;
    e.id = ColText(stmt, 0);
    e.ts_ms = sqlite3_column_int64(stmt, 1);
    e.space_id = ColText(stmt, 2);
    e.flow_id = ColText(stmt, 3);
    e.run_id = ColText(stmt, 4);
    e.agent_id = ColText(stmt, 5);
    AppEventKindFromString(ColText(stmt, 6), &e.kind);
    const std::string payload_str = ColText(stmt, 7);
    auto parsed = nlohmann::json::parse(payload_str, nullptr, false);
    e.payload = (parsed.is_object()) ? parsed : nlohmann::json::object();
    out.events.push_back(std::move(e));
  }
  sqlite3_finalize(stmt);
  if (!out.events.empty() && static_cast<int>(out.events.size()) == limit) {
    out.cursor = out.events.back().id;
  }
  return out;
}

EventBus::Token EventBus::Subscribe(const Scope& scope, Subscriber cb) {
  std::lock_guard<std::mutex> lock(mu_);
  // Replay under the same lock so live appends don't sneak between
  // replay and live attachment.
  ListQuery q;
  q.scope = scope;
  q.limit = 1000;
  // We need oldest-first replay; List() returns newest-first. Page until
  // empty, then reverse-emit in order.
  std::vector<AppEvent> buf;
  std::string before;
  while (true) {
    ListQuery pq = q;
    pq.before_id = before;
    auto page = List(pq);  // List takes its own SQL lock — safe (recursive)
    if (page.events.empty()) break;
    for (auto& e : page.events) buf.push_back(std::move(e));
    if (page.cursor.empty()) break;
    before = page.cursor;
    if (buf.size() > 10000) break;  // safety cap; UI paginates beyond
  }
  // Replay oldest-first.
  for (auto it = buf.rbegin(); it != buf.rend(); ++it) cb(*it);

  const Token tok = next_token_.fetch_add(1);
  subs_.push_back({tok, scope, std::move(cb)});
  return tok;
}

void EventBus::Unsubscribe(Token tok) {
  std::lock_guard<std::mutex> lock(mu_);
  for (auto it = subs_.begin(); it != subs_.end(); ++it) {
    if (it->token == tok) {
      subs_.erase(it);
      return;
    }
  }
}

InboxListResult EventBus::ListInbox(const InboxQuery& q) const {
  InboxListResult out;
  auto* db = store_ ? store_->raw_db() : nullptr;
  if (!db) return out;
  std::string sql =
      "SELECT event_id, state, snooze_until, flow_id, kind FROM inbox "
      "WHERE 1=1";
  if (q.state.has_value()) sql += " AND state = ?";
  if (!q.flow_id.empty()) sql += " AND flow_id = ?";
  sql += " ORDER BY event_id DESC LIMIT ?;";

  std::lock_guard<std::mutex> sl(store_->read_mutex());
  sqlite3_stmt* stmt = nullptr;
  if (sqlite3_prepare_v2(db, sql.c_str(), -1, &stmt, nullptr) != SQLITE_OK) {
    return out;
  }
  int idx = 1;
  if (q.state.has_value()) {
    BindText(stmt, idx++, InboxStateToString(*q.state));
  }
  if (!q.flow_id.empty()) BindText(stmt, idx++, q.flow_id);
  sqlite3_bind_int(stmt, idx, q.limit > 0 ? q.limit : 100);

  while (sqlite3_step(stmt) == SQLITE_ROW) {
    InboxRow r;
    r.event_id = ColText(stmt, 0);
    InboxStateFromString(ColText(stmt, 1), &r.state);
    if (sqlite3_column_type(stmt, 2) != SQLITE_NULL) {
      r.snooze_until = sqlite3_column_int64(stmt, 2);
    }
    r.flow_id = ColText(stmt, 3);
    r.kind = ColText(stmt, 4);
    out.rows.push_back(std::move(r));
  }
  sqlite3_finalize(stmt);

  // Counts (independent queries — keep pure).
  static constexpr const char* kUnreadCount =
      "SELECT COUNT(*) FROM inbox WHERE state='unread';";
  if (sqlite3_prepare_v2(db, kUnreadCount, -1, &stmt, nullptr) == SQLITE_OK) {
    if (sqlite3_step(stmt) == SQLITE_ROW) {
      out.unread_count = sqlite3_column_int(stmt, 0);
    }
    sqlite3_finalize(stmt);
  }
  out.needs_action_count = out.unread_count;
  return out;
}

bool EventBus::SetInboxState(const std::string& event_id, InboxState state,
                             std::optional<long long> snooze_until) {
  auto* db = store_ ? store_->raw_db() : nullptr;
  if (!db) return false;
  std::lock_guard<std::mutex> sl(store_->read_mutex());
  static constexpr const char* kUpdate =
      "UPDATE inbox SET state=?, snooze_until=? WHERE event_id=?;";
  sqlite3_stmt* stmt = nullptr;
  if (sqlite3_prepare_v2(db, kUpdate, -1, &stmt, nullptr) != SQLITE_OK) {
    return false;
  }
  const std::string ss = InboxStateToString(state);
  BindText(stmt, 1, ss);
  if (snooze_until.has_value()) {
    sqlite3_bind_int64(stmt, 2, *snooze_until);
  } else {
    sqlite3_bind_null(stmt, 2);
  }
  BindText(stmt, 3, event_id);
  const int rc = sqlite3_step(stmt);
  sqlite3_finalize(stmt);
  return rc == SQLITE_DONE && sqlite3_changes(db) > 0;
}

int EventBus::GarbageCollect(long long older_than_ms) {
  auto* db = store_ ? store_->raw_db() : nullptr;
  if (!db) return 0;
  std::lock_guard<std::mutex> sl(store_->read_mutex());
  // Promote snoozed-and-expired rows to unread first.
  static constexpr const char* kExpire =
      "UPDATE inbox SET state='unread', snooze_until=NULL "
      "WHERE state='snoozed' AND snooze_until IS NOT NULL "
      "AND snooze_until <= ?;";
  sqlite3_stmt* stmt = nullptr;
  if (sqlite3_prepare_v2(db, kExpire, -1, &stmt, nullptr) == SQLITE_OK) {
    sqlite3_bind_int64(stmt, 1, NowMs());
    sqlite3_step(stmt);
    sqlite3_finalize(stmt);
  }
  // Delete events older than threshold whose inbox row is missing or read.
  static constexpr const char* kDelete =
      "DELETE FROM events WHERE ts_ms < ? AND id NOT IN ("
      "  SELECT event_id FROM inbox WHERE state IN ('unread','snoozed')"
      ");";
  if (sqlite3_prepare_v2(db, kDelete, -1, &stmt, nullptr) != SQLITE_OK) {
    return 0;
  }
  sqlite3_bind_int64(stmt, 1, older_than_ms);
  sqlite3_step(stmt);
  sqlite3_finalize(stmt);
  return sqlite3_changes(db);
}

bool EventBus::IsKindEnabledForNotifications(const std::string& kind) const {
  auto* db = store_ ? store_->raw_db() : nullptr;
  if (!db) return true;
  std::lock_guard<std::mutex> sl(store_->read_mutex());
  static constexpr const char* kQ =
      "SELECT enabled FROM notification_prefs WHERE kind=?;";
  sqlite3_stmt* stmt = nullptr;
  if (sqlite3_prepare_v2(db, kQ, -1, &stmt, nullptr) != SQLITE_OK) return true;
  BindText(stmt, 1, kind);
  bool enabled = true;
  if (sqlite3_step(stmt) == SQLITE_ROW) {
    enabled = sqlite3_column_int(stmt, 0) != 0;
  }
  sqlite3_finalize(stmt);
  return enabled;
}

void EventBus::SetKindNotificationEnabled(const std::string& kind,
                                          bool enabled) {
  auto* db = store_ ? store_->raw_db() : nullptr;
  if (!db) return;
  std::lock_guard<std::mutex> sl(store_->read_mutex());
  static constexpr const char* kUp =
      "INSERT INTO notification_prefs (kind, enabled) VALUES (?, ?) "
      "ON CONFLICT(kind) DO UPDATE SET enabled=excluded.enabled;";
  sqlite3_stmt* stmt = nullptr;
  if (sqlite3_prepare_v2(db, kUp, -1, &stmt, nullptr) != SQLITE_OK) return;
  BindText(stmt, 1, kind);
  sqlite3_bind_int(stmt, 2, enabled ? 1 : 0);
  sqlite3_step(stmt);
  sqlite3_finalize(stmt);
}

std::vector<std::string> EventBus::ListEnabledNotificationKinds() const {
  std::vector<std::string> out;
  auto* db = store_ ? store_->raw_db() : nullptr;
  if (!db) return out;
  std::lock_guard<std::mutex> sl(store_->read_mutex());
  static constexpr const char* kQ =
      "SELECT kind FROM notification_prefs WHERE enabled=1;";
  sqlite3_stmt* stmt = nullptr;
  if (sqlite3_prepare_v2(db, kQ, -1, &stmt, nullptr) != SQLITE_OK) return out;
  while (sqlite3_step(stmt) == SQLITE_ROW) {
    out.push_back(ColText(stmt, 0));
  }
  sqlite3_finalize(stmt);
  return out;
}

}  // namespace cronymax::event_bus
