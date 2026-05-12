#ifndef CRONYMAX_EVENT_BUS_EVENT_BUS_H_
#define CRONYMAX_EVENT_BUS_EVENT_BUS_H_

#include <atomic>
#include <cstdint>
#include <filesystem>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <vector>

#include "event_bus/app_event.h"

struct sqlite3;

namespace cronymax {
class SpaceStore;
}

namespace cronymax::event_bus {

// Subscription scope for live + replay queries.
struct Scope {
  std::string flow_id;  // empty = any
  std::string run_id;   // empty = any
};

// Pagination for List().
struct ListQuery {
  Scope scope;
  std::string before_id;  // empty = newest first
  int limit = 200;        // hard-capped at 1000
};

struct ListResult {
  std::vector<AppEvent> events;
  std::string cursor;  // empty when no more
};

// Inbox row materialised by the triage step.
enum class InboxState { kUnread, kRead, kSnoozed };

const char* InboxStateToString(InboxState s);
bool InboxStateFromString(const std::string& s, InboxState* out);

struct InboxRow {
  std::string event_id;
  InboxState state = InboxState::kUnread;
  std::optional<long long> snooze_until;  // unix ms
  std::string flow_id;                    // for filter
  std::string kind;                       // for filter
};

struct InboxQuery {
  std::optional<InboxState> state;  // nullopt = any
  std::string flow_id;              // empty = any
  int limit = 100;
};

struct InboxListResult {
  std::vector<InboxRow> rows;
  int unread_count = 0;
  int needs_action_count = 0;  // == rows for default state filter
};

// Identifier used to identify the user when triaging @-mentions in text
// events. Currently always "me" — future work may surface multiple
// participants per Space.
constexpr const char* kCurrentUserId = "me";

// Single per-Space event bus. Owns nothing; borrows the SQLite handle from
// `SpaceStore` and writes a sibling JSONL file under each run's directory
// (when run_id is set).
class EventBus {
 public:
  using Subscriber = std::function<void(const AppEvent&)>;
  using Token = std::uint64_t;

  // `space_root` is the workspace directory (where `.cronymax/` lives).
  // The bus uses it to derive the per-run trace.jsonl path.
  EventBus(SpaceStore* store,
           std::string space_id,
           std::filesystem::path space_root);
  ~EventBus();

  EventBus(const EventBus&) = delete;
  EventBus& operator=(const EventBus&) = delete;

  // Append an event. Performs (atomic-under-mutex):
  //   1. INSERT into events table
  //   2. Append JSONL line to runs/<run_id>/trace.jsonl when run_id set
  //   3. Triage → INSERT/UPDATE inbox row when needs_action
  //   4. Dispatch to live subscribers
  // Fills in `evt.id` (UUIDv7) and `evt.ts_ms` (now) when caller leaves
  // them empty/zero. Returns the (possibly-generated) id.
  std::string Append(AppEvent evt);

  // Page through persisted events. Newest-first; pass the previous result's
  // `cursor` as `before_id` to get the next page.
  ListResult List(const ListQuery& q) const;

  // Replay-then-live subscription. The callback fires synchronously with
  // every persisted event matching `scope` (oldest first) before this
  // function returns; subsequent live events are dispatched on the
  // calling thread of `Append`. Returns a token for Unsubscribe.
  Token Subscribe(const Scope& scope, Subscriber cb);
  void Unsubscribe(Token tok);

  // Inbox queries.
  InboxListResult ListInbox(const InboxQuery& q) const;
  bool SetInboxState(const std::string& event_id,
                     InboxState state,
                     std::optional<long long> snooze_until = std::nullopt);

  // Garbage-collect events older than `older_than_ms` whose inbox row is
  // either absent or has state='read'. Returns the row count deleted.
  int GarbageCollect(long long older_than_ms);

  // Notification preferences (kind → enabled). Default is enabled when no
  // row exists.
  bool IsKindEnabledForNotifications(const std::string& kind) const;
  void SetKindNotificationEnabled(const std::string& kind, bool enabled);
  std::vector<std::string> ListEnabledNotificationKinds() const;

 private:
  struct SubEntry {
    Token token;
    Scope scope;
    Subscriber cb;
  };

  bool ScopeMatches(const Scope& s, const AppEvent& e) const;
  bool TriageNeedsAction(const AppEvent& e) const;
  void WriteJsonlLine(const AppEvent& e) const;

  SpaceStore* store_;
  std::string space_id_;
  std::filesystem::path space_root_;

  mutable std::mutex mu_;  // guards subs_, append-side ordering
  std::vector<SubEntry> subs_;
  std::atomic<Token> next_token_{1};
};

}  // namespace cronymax::event_bus

#endif  // CRONYMAX_EVENT_BUS_EVENT_BUS_H_
