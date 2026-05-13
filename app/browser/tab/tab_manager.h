// Copyright (c) 2026.
//
// TabManager — owns every Tab. Replaces (in later phases) BrowserManager
// and the per-kind singleton view members on MainWindow.
//
// Phase 1 skeleton: storage + public API stubs. Activation does not yet
// swap visible cards into a host panel — that wiring lands in later phases.

#pragma once

#include <functional>
#include <map>
#include <memory>
#include <string>
#include <vector>

#include "browser/models/view_context.h"
#include "browser/tab/tab.h"
#include "include/cef_request_context.h"

namespace cronymax {

class ClientHandler;

// Per-kind opaque parameters for Open(). Concrete fields are added per kind
// in later phases (e.g. WebOpenParams.url).
struct OpenParams {
  std::string title;     // optional display label
  std::string url;       // web tabs only; ignored otherwise
  std::string raw_json;  // free-form per-kind extras (TBD)
  // native-title-bar: explicit display name override. When empty AND the
  // kind is in the auto-numbered set ({kTerminal, kChat}), TabManager::Open
  // assigns "<KindDisplayName> N".
  std::string display_name;
  // Optional seed metadata (e.g. "chat_id" for restored chat tabs).
  std::map<std::string, std::string> meta;
};

// Lightweight per-tab snapshot used by `shell.tabs_list` events. Phase 2
// adds the bridge serialization; Phase 1 just exposes the struct.
struct TabSummary {
  TabId id;
  TabKind kind;
  std::string display_name;  // pulled from behavior in later phases
  std::map<std::string, std::string> meta;  // arbitrary per-tab metadata
};

class TabBehavior;

class TabManager {
 public:
  TabManager(ThemeContext* theme_ctx);
  ~TabManager();

  TabManager(const TabManager&) = delete;
  TabManager& operator=(const TabManager&) = delete;

  // Inject the shared ClientHandler so per-kind behaviors (web, etc.) can
  // create CefBrowserViews bound to the app's single Client. Optional in
  // unit tests; required for any kind that hosts a browser.
  void SetClientHandler(ClientHandler* client_handler) {
    client_handler_ = client_handler;
  }

  // Set the profile-scoped CefRequestContext used for all subsequently
  // created browser views.  Call this once at startup and again on every
  // profile switch so that new tabs inherit the active profile's context.
  void SetRequestContext(CefRefPtr<CefRequestContext> ctx) {
    request_context_ = std::move(ctx);
  }

  // Register a kind as a singleton. FindOrCreateSingleton requires this.
  void RegisterSingletonKind(TabKind kind);
  // Returns true iff `kind` was registered as a singleton via
  // RegisterSingletonKind. Used by the bridge dispatcher to reject
  // shell.tab_open_singleton calls for multi-instance kinds.
  bool IsSingletonKind(TabKind kind) const;

  // Bind the content URL used when opening a singleton tab of `kind` (or
  // any non-web kind opened with empty params.url). Required for
  // SimpleTabBehavior-backed kinds (terminal/chat/agent/graph).
  void SetKindContentUrl(TabKind kind, std::string url) {
    kind_content_urls_[kind] = std::move(url);
  }

  // Create a new tab of `kind`. Returns the new tab's id. Phase 1 returns
  // empty string for kinds that have no behavior factory yet.
  TabId Open(TabKind kind, const OpenParams& params);

  // Returns existing singleton id when present, else creates a new one.
  // `out_created` is set to true iff a new tab was created. Asserts the
  // kind has been registered as a singleton.
  TabId FindOrCreateSingleton(TabKind kind, bool* out_created = nullptr);

  // Activate `id`. In Phase 1 this only updates `active_tab_id_`; the
  // host-panel swap is wired in later phases.
  void Activate(const TabId& id);

  // Close `id`. Removes from storage and clears any singleton index entry.
  // No-op if `id` is unknown.
  void Close(const TabId& id);

  Tab* Get(const TabId& id);
  const Tab* Get(const TabId& id) const;

  // Convenience: active tab pointer or nullptr.
  Tab* Active();
  // Find a tab by underlying browser identifier (web kind only). Returns
  // nullptr if no tab hosts a browser with this id.
  Tab* FindByBrowserId(int browser_id);

  std::vector<TabSummary> Snapshot() const;

  // Set/get arbitrary metadata on a tab (e.g. "chat_id" for chat tabs).
  // No-op / empty if the tab doesn't exist.
  void SetTabMeta(const TabId& id,
                  const std::string& key,
                  const std::string& value);
  std::string GetTabMeta(const TabId& id, const std::string& key) const;

  const TabId& active_tab_id() const { return active_tab_id_; }
  size_t size() const { return tabs_.size(); }

  // Phase 2: emitter hook. Fired after Open / Activate / Close mutate state.
  // The owner (MainWindow) uses this to broadcast `shell.tabs_list` and
  // `shell.tab_activated` events to all renderers.
  using ChangeCallback = std::function<void()>;
  void SetOnChange(ChangeCallback cb) { on_change_ = std::move(cb); }

 private:
  // Construct the per-kind behavior. Returns nullptr for kinds whose
  // behavior class has not yet been added (Phases 3-8).
  std::unique_ptr<TabBehavior> MakeBehavior(TabKind kind,
                                            const OpenParams& params);

  TabId NewId();

  std::vector<std::unique_ptr<Tab>> tabs_;
  TabId active_tab_id_;
  std::map<TabKind, TabId> singletons_;
  std::map<TabKind, bool> singleton_kinds_registered_;
  std::map<TabKind, std::string> kind_content_urls_;
  uint64_t next_id_seq_ = 1;
  ChangeCallback on_change_;
  ClientHandler* client_handler_ = nullptr;
  ThemeContext* theme_ctx_ = nullptr;
  CefRefPtr<CefRequestContext> request_context_;
};

}  // namespace cronymax
