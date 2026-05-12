#ifndef CRONYMAX_EVENT_BUS_APP_EVENT_H_
#define CRONYMAX_EVENT_BUS_APP_EVENT_H_

#include <string>

#include <nlohmann/json.hpp>

namespace cronymax::event_bus {

// Closed enum mirroring web/src/shared/types/events.ts. Adding a new kind
// here requires bumping the renderer Zod union as well — that's intentional
// (the wire format is the contract). Tags are serialised as the snake_case
// string of the enumerator (without the leading "k").
enum class AppEventKind {
  kText,
  kAgentStatus,
  kDocumentEvent,
  kReviewEvent,
  kHandoff,
  kError,
  kSystem,
  kFileEdited,
  kGitCommitCreated,
  kGitPushed,
};

const char* AppEventKindToString(AppEventKind k);
bool AppEventKindFromString(const std::string& s, AppEventKind* out);

// One event in the typed stream.
//
// `payload` is a free-form JSON object whose required fields depend on
// `kind`; the schema is enforced at the renderer with Zod. The C++ side
// trusts well-formedness because every emitter is in-tree.
struct AppEvent {
  std::string id;          // UUIDv7 (sortable)
  long long ts_ms = 0;     // unix milliseconds
  std::string space_id;
  std::string flow_id;     // empty when not scoped
  std::string run_id;      // empty when not scoped
  std::string agent_id;    // empty when not scoped
  std::string session_id;  // empty when not in a session
  AppEventKind kind = AppEventKind::kSystem;
  nlohmann::json payload = nlohmann::json::object();  // object

  // Render the event as compact JSON (no trailing newline).
  std::string ToJson() const;

  // Parse from compact JSON. Returns true on success; *err populated on
  // failure if non-null. The function is permissive about missing optional
  // fields but requires `id`, `ts_ms`, `kind`, `payload`.
  static bool ParseJson(const std::string& text, AppEvent* out,
                        std::string* err);
};

}  // namespace cronymax::event_bus

#endif  // CRONYMAX_EVENT_BUS_APP_EVENT_H_
