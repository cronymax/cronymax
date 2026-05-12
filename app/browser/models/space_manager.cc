#include "browser/models/space_manager.h"

#include <algorithm>
#include <chrono>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <random>
#include <sstream>

#include "event_bus/event_bus.h"
#include "platform/macos/notifications.h"
#include "workspace/workspace_layout.h"
// (task 4.1) agent_runtime.h and flow_runtime.h removed — run lifecycle
// now owned by the Rust runtime via GIPS / RuntimeProxy.

namespace cronymax {

namespace {

int64_t NowMs() {
  using namespace std::chrono;
  return duration_cast<milliseconds>(system_clock::now().time_since_epoch())
      .count();
}

// Generate a simple UUID-like id (not cryptographically strong — prototype).
std::string MakeId() {
  static std::mt19937_64 rng{std::random_device{}()};
  const uint64_t a = rng();
  const uint64_t b = rng();
  char buf[37];
  std::snprintf(
      buf, sizeof(buf), "%08x-%04x-%04x-%04x-%012llx",
      static_cast<uint32_t>(a >> 32), static_cast<uint32_t>((a >> 16) & 0xffff),
      static_cast<uint32_t>(a & 0xffff), static_cast<uint32_t>(b >> 48),
      static_cast<unsigned long long>(b & 0x0000ffffffffffff));
  return buf;
}

}  // namespace

TerminalSession* Space::FindTerminal(const std::string& tid) {
  for (auto& t : terminals)
    if (t->id == tid)
      return t.get();
  return nullptr;
}

TerminalSession* Space::ActiveTerminal() {
  if (!active_terminal_id.empty()) {
    if (auto* t = FindTerminal(active_terminal_id))
      return t;
  }
  if (!terminals.empty()) {
    active_terminal_id = terminals.front()->id;
    return terminals.front().get();
  }
  return nullptr;
}

TerminalSession* Space::CreateTerminal() {
  auto t = std::make_unique<TerminalSession>();
  t->id = "t" + std::to_string(next_terminal_seq);
  t->name = "Terminal " + std::to_string(next_terminal_seq);
  ++next_terminal_seq;
  active_terminal_id = t->id;
  terminals.push_back(std::move(t));
  return terminals.back().get();
}

bool Space::CloseTerminal(const std::string& tid) {
  for (auto it = terminals.begin(); it != terminals.end(); ++it) {
    if ((*it)->id == tid) {
      terminals.erase(it);
      if (active_terminal_id == tid) {
        active_terminal_id = terminals.empty() ? "" : terminals.front()->id;
      }
      return true;
    }
  }
  return false;
}

SpaceManager::SpaceManager() = default;
SpaceManager::~SpaceManager() = default;

// ---------------------------------------------------------------------------
// Init
// ---------------------------------------------------------------------------

bool SpaceManager::Init(const std::filesystem::path& db_path) {
  if (!store_.Open(db_path)) {
    return false;
  }

  // Initialise ProfileStore from the user home directory.
  const char* home = std::getenv("HOME");
  if (home && *home) {
    profile_store_ = ProfileStore(std::filesystem::path(home));
  } else {
    profile_store_ = ProfileStore(db_path.parent_path());
  }
  profile_store_.EnsureDefaultProfile();

  const auto rows = store_.ListSpaces();
  for (const auto& row : rows) {
    spaces_.push_back(InstantiateSpace(row));
  }

  if (spaces_.empty()) {
    return true;  // Caller should call CreateSpace for a default Space.
  }

  // Restore last-active Space (already sorted by last_active desc).
  // Use SwitchTo so per-Space registries (Phase A) and any switch
  // callbacks are invoked consistently.
  SwitchTo(spaces_.front()->id);
  return true;
}

// ---------------------------------------------------------------------------
// CreateSpace
// ---------------------------------------------------------------------------

// Name-from-basename overload (preferred for open-folder flow).
std::string SpaceManager::CreateSpace(const std::filesystem::path& root_path,
                                      const std::string& profile_id) {
  const std::string name = root_path.filename().string();
  return CreateSpace(name.empty() ? root_path.string() : name, root_path,
                     profile_id);
}

// Legacy overload: caller supplies name explicitly.
std::string SpaceManager::CreateSpace(const std::string& name,
                                      const std::filesystem::path& root_path,
                                      const std::string& profile_id) {
  if (!std::filesystem::is_directory(root_path)) {
    return {};
  }

  SpaceRow row;
  row.id = MakeId();
  row.name = name;
  row.root_path = root_path.string();
  row.profile_id = profile_id.empty() ? "default" : profile_id;
  row.created_at = NowMs();
  row.last_active = NowMs();

  if (!store_.CreateSpace(row)) {
    return {};
  }

  spaces_.push_back(InstantiateSpace(row));
  // If this is the first Space, activate it so registries (Phase A) and
  // ActiveSpace() are usable immediately.
  if (active_index_ < 0) {
    SwitchTo(row.id);
  }
  return row.id;
}

// ---------------------------------------------------------------------------
// SwitchTo
// ---------------------------------------------------------------------------

bool SpaceManager::SwitchTo(const std::string& space_id) {
  for (int i = 0; i < static_cast<int>(spaces_.size()); ++i) {
    if (spaces_[static_cast<size_t>(i)]->id == space_id) {
      const std::string old_id =
          (active_index_ >= 0) ? spaces_[static_cast<size_t>(active_index_)]->id
                               : "";
      active_index_ = i;
      store_.UpdateLastActive(space_id, NowMs());

      // Phase 3+4: registries and FsWatcher moved to Rust; only set up
      // the .cronymax/ skeleton and EventBus on first activation.
      Space* sp = spaces_[static_cast<size_t>(i)].get();
      if (!sp->event_bus) {
        WorkspaceLayout layout(sp->workspace_root);
        std::string err;
        layout.EnsureSkeleton(&err);  // best-effort; ignore err for now

        // EventBus: typed event store for the channel view, inbox, and
        // status dot. Borrows the SpaceStore's sqlite3 handle.
        // (task 4.1) FlowRuntime initialization removed; run lifecycle is
        // now owned by the Rust runtime over GIPS. EventBus is retained for
        // local events (events.append), inbox, and notification paths.
        sp->event_bus = std::make_unique<event_bus::EventBus>(
            &store_, sp->id, sp->workspace_root);

        // Migration marker: existing trace.jsonl files are TraceEvent-shaped
        // (legacy), not AppEvent-shaped, so a faithful replay is not
        // possible without a kind-mapping table. Write a marker so future
        // releases know not to re-attempt this migration. The legacy files
        // remain on disk and `rebuild_trace` can rewrite them from
        // EventBus going forward.
        {
          namespace fs = std::filesystem;
          const fs::path marker_dir =
              fs::path(sp->workspace_root) / ".cronymax" / "migrations";
          const fs::path marker = marker_dir / "event-bus-v1.done";
          std::error_code ec;
          if (!fs::exists(marker, ec)) {
            fs::create_directories(marker_dir, ec);
            std::ofstream m(marker);
            if (m.is_open()) {
              m << "event-bus v1: legacy trace.jsonl preserved on disk; "
                << "AppEvent stream is the new source of truth.\n";
            }
          }
        }

        // macOS native bridge: subscribe to needs-action events and
        // refresh the dock badge after every Append. Best-effort; on
        // non-Apple platforms these calls are no-ops.
        {
          event_bus::Scope all;  // empty scope = all events for this Space.
          auto* bus_ptr = sp->event_bus.get();
          bus_ptr->Subscribe(all, [bus_ptr](const event_bus::AppEvent& e) {
            const std::string kind = event_bus::AppEventKindToString(e.kind);
            if (bus_ptr->IsKindEnabledForNotifications(kind) &&
                platform::macos::IsNotificationAuthorized()) {
              std::string body;
              if (e.payload.is_object()) {
                if (e.payload.contains("body") && e.payload["body"].is_string())
                  body = e.payload["body"].get<std::string>();
                else if (e.payload.contains("message") &&
                         e.payload["message"].is_string())
                  body = e.payload["message"].get<std::string>();
              }
              if (body.empty())
                body = "(" + kind + ")";
              std::string deeplink = "cronymax://inbox/" + e.id;
              platform::macos::PostNotification(
                  /*title=*/kind, body, deeplink);
            }
            // Refresh dock badge from the inbox unread count after
            // every Append. Inexpensive single COUNT(*) query.
            event_bus::InboxQuery iq;
            iq.state = event_bus::InboxState::kUnread;
            iq.limit = 1;
            const auto inbox = bus_ptr->ListInbox(iq);
            platform::macos::SetDockBadgeCount(inbox.unread_count);
          });
        }
      }

      if (runtime_restart_callback_) {
        // Resolve the space's profile and trigger a runtime restart so the
        // new sandbox policy is applied (design decision D4).
        ProfileRecord profile;
        if (auto opt = profile_store_.Get(sp->profile_id)) {
          profile = *opt;
        } else {
          // Fall back to default profile if the referenced one is missing.
          if (auto def = profile_store_.Get("default")) {
            profile = *def;
          } else {
            profile.id = "default";
            profile.name = "Default";
            profile.allow_network = true;
          }
        }
        runtime_restart_callback_(sp->workspace_root.string(), profile);
      }

      if (switch_callback_) {
        switch_callback_(old_id, space_id);
      }
      return true;
    }
  }
  return false;
}

// ---------------------------------------------------------------------------
// DeleteSpace
// ---------------------------------------------------------------------------

bool SpaceManager::DeleteSpace(const std::string& space_id) {
  const int idx = [&]() -> int {
    for (int i = 0; i < static_cast<int>(spaces_.size()); ++i) {
      if (spaces_[static_cast<size_t>(i)]->id == space_id)
        return i;
    }
    return -1;
  }();

  if (idx < 0)
    return false;

  // If deleting the active Space and others exist, switch first.
  if (idx == active_index_ && spaces_.size() > 1) {
    const int next =
        (idx + 1) < static_cast<int>(spaces_.size()) ? idx + 1 : idx - 1;
    SwitchTo(spaces_[static_cast<size_t>(next)]->id);
  }

  store_.DeleteSpace(space_id);
  spaces_.erase(spaces_.begin() + idx);

  // Adjust active_index_ after removal.
  if (spaces_.empty()) {
    active_index_ = -1;
  } else if (active_index_ >= static_cast<int>(spaces_.size())) {
    active_index_ = static_cast<int>(spaces_.size()) - 1;
  }

  return true;
}

// ---------------------------------------------------------------------------
// Accessors
// ---------------------------------------------------------------------------

Space* SpaceManager::ActiveSpace() {
  if (active_index_ < 0 || active_index_ >= static_cast<int>(spaces_.size())) {
    return nullptr;
  }
  return spaces_[static_cast<size_t>(active_index_)].get();
}

const Space* SpaceManager::ActiveSpace() const {
  if (active_index_ < 0 || active_index_ >= static_cast<int>(spaces_.size())) {
    return nullptr;
  }
  return spaces_[static_cast<size_t>(active_index_)].get();
}

Space* SpaceManager::FindSpace(const std::string& space_id) {
  for (auto& sp : spaces_) {
    if (sp->id == space_id)
      return sp.get();
  }
  return nullptr;
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

std::unique_ptr<Space> SpaceManager::InstantiateSpace(const SpaceRow& row) {
  auto sp = std::make_unique<Space>();
  sp->id = row.id;
  sp->name = row.name;
  sp->profile_id = row.profile_id;
  sp->workspace_root = row.root_path;
  // (task 4.1) agent_runtime removed; run lifecycle is now owned by the
  // Rust runtime over GIPS. RuntimeBindingState is value-initialized.
  sp->CreateTerminal();  // start with one terminal
  return sp;
}

}  // namespace cronymax
