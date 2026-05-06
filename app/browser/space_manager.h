#pragma once

#include <filesystem>
#include <functional>
#include <memory>
#include <string>
#include <vector>

#include "agent/tool_registry.h"
#include "document/agent_registry.h"
#include "document/doc_type_registry.h"
#include "event_bus/event_bus.h"
#include "workspace/fs_watcher.h"
#include "terminal/pty_session.h"
#include "workspace/space_store.h"

namespace cronymax {

// Per-Space browser state (view handles managed by MainWindow; stored here for
// switching logic).
struct SpaceBrowserState {
  std::string active_tab_id;  // SQLite row id as string
};

// One PTY-backed terminal session within a Space.
struct TerminalSession {
  std::string id;    // unique within Space, e.g. "t1"
  std::string name;  // display label, e.g. "Terminal 1"
  std::unique_ptr<PtySession> pty;
};

// One Space = one Workspace context owning all runtime resources.
struct Space {
  std::string id;
  std::string name;
  std::filesystem::path workspace_root;

  std::vector<std::unique_ptr<TerminalSession>> terminals;
  std::string active_terminal_id;
  int next_terminal_seq = 1;

  SpaceBrowserState browser_state;

  // Per-Space orchestration registries (Phase A: read-only). Lazily
  // populated by SpaceManager when the Space is first activated.
  std::unique_ptr<AgentRegistry> agent_registry;
  std::unique_ptr<DocTypeRegistry> doc_type_registry;
  std::unique_ptr<FsWatcher> fs_watcher;

  // Per-Space typed event store (agent-event-bus). Lazily initialised on
  // first activation. Borrows SpaceStore's sqlite3 handle.
  // Retained for local events (events.append) and inbox/notification paths.
  std::unique_ptr<event_bus::EventBus> event_bus;

  // (task 4.1) Runtime binding state — replaces the removed agent_runtime
  // and flow_runtime unique_ptrs. Holds runtime-side subscription handles
  // and the tool registry used for direct tool invocations from the renderer.
  // Once task 6.1 lands (agent_runtime.* deleted) this section is the
  // sole source of Space-level orchestration identity.
  struct RuntimeBindingState {
    // RuntimeProxy event subscription token (set by events.subscribe)
    int64_t event_sub_token = -1;
    // Runtime-side subscription IDs for active event streams.
    std::vector<std::string> runtime_sub_ids;
    // Tool registry for direct tool.exec invocations (renderer debug path).
    // Empty until populated by tool-scope enforcement (task 4.3).
    ToolRegistry tool_registry;
  };
  RuntimeBindingState runtime_binding;

  TerminalSession* FindTerminal(const std::string& tid);
  TerminalSession* ActiveTerminal();
  TerminalSession* CreateTerminal();  // appends, sets active
  bool CloseTerminal(const std::string& tid);  // false if not found
};

// Callback invoked (on the caller's thread) when the active Space changes.
using SpaceSwitchCallback = std::function<void(const std::string& old_id,
                                               const std::string& new_id)>;

class SpaceManager {
 public:
  SpaceManager();
  ~SpaceManager();

  SpaceManager(const SpaceManager&) = delete;
  SpaceManager& operator=(const SpaceManager&) = delete;

  // Open DB, load persisted Spaces, restore last-active Space.
  // Returns false if the database cannot be opened.
  bool Init(const std::filesystem::path& db_path);

  // Path to bundled built-in doc-type YAMLs. Must be set before any Space
  // is activated, otherwise built-ins won't be available. Caller passes
  // (typically) `<bundle>/Contents/Resources/builtin-doc-types/`.
  void SetBuiltinDocTypesDir(std::filesystem::path dir) {
    builtin_doc_types_dir_ = std::move(dir);
  }

  // Create a new Space. Returns the new space_id on success, empty on error.
  std::string CreateSpace(const std::string& name,
                          const std::filesystem::path& root_path);

  // Switch the active Space. Returns false if space_id not found.
  bool SwitchTo(const std::string& space_id);

  // Delete a Space. If it is active, switches to the most-recently-active
  // remaining Space first.
  bool DeleteSpace(const std::string& space_id);

  // Returns the active Space, or nullptr if none.
  Space* ActiveSpace();
  const Space* ActiveSpace() const;

  // All loaded Spaces (ordered by last_active desc).
  const std::vector<std::unique_ptr<Space>>& spaces() const { return spaces_; }

  // Register a callback for Space switches.
  void SetSwitchCallback(SpaceSwitchCallback cb) {
    switch_callback_ = std::move(cb);
  }

  SpaceStore& store() { return store_; }
  const SpaceStore& store() const { return store_; }

 private:
  Space* FindSpace(const std::string& space_id);
  std::unique_ptr<Space> InstantiateSpace(const SpaceRow& row);

  SpaceStore store_;
  std::vector<std::unique_ptr<Space>> spaces_;
  int active_index_ = -1;
  SpaceSwitchCallback switch_callback_;
  std::filesystem::path builtin_doc_types_dir_;
};

}  // namespace cronymax
