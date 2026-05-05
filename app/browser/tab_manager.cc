// Copyright (c) 2026.

#include "browser/tab_manager.h"

#include <cassert>
#include <cstdlib>
#include <sstream>
#include <utility>

#include "browser/tab_behavior.h"
#include "browser/tab_behaviors/simple_tab_behavior.h"
#include "browser/tab_behaviors/web_tab_behavior.h"

namespace cronymax {

namespace {
std::string DisplayNameFor(const Tab* tab) {
  if (!tab) return {};
  if (tab->kind() == TabKind::kWeb) {
    if (auto* wb = static_cast<WebTabBehavior*>(tab->behavior())) {
      if (!wb->current_title().empty()) return wb->current_title();
      if (!wb->current_url().empty()) return wb->current_url();
    }
  } else {
    if (auto* sb = static_cast<SimpleTabBehavior*>(tab->behavior())) {
      if (!sb->display_name().empty()) return sb->display_name();
    }
  }
  return std::string("Tab ") + tab->tab_id();
}

// native-title-bar: kinds whose tabs auto-number when opened with empty
// display_name. Web stays opportunistically named from page title.
bool IsAutoNumberedKind(TabKind k) {
  return k == TabKind::kTerminal || k == TabKind::kChat;
}

const char* AutoNumberPrefix(TabKind k) {
  switch (k) {
    case TabKind::kTerminal: return "Terminal";
    case TabKind::kChat:     return "Chat";
    default:                 return "Tab";
  }
}
}  // namespace

TabManager::TabManager() = default;
TabManager::~TabManager() = default;

void TabManager::RegisterSingletonKind(TabKind kind) {
  singleton_kinds_registered_[kind] = true;
}

bool TabManager::IsSingletonKind(TabKind kind) const {
  auto it = singleton_kinds_registered_.find(kind);
  return it != singleton_kinds_registered_.end() && it->second;
}

TabId TabManager::Open(TabKind kind, const OpenParams& params) {
  // native-title-bar: auto-number terminal/chat tabs when no explicit
  // display name was supplied.
  OpenParams effective = params;
  if (effective.display_name.empty() && IsAutoNumberedKind(kind)) {
    int max_n = 0;
    const std::string prefix = std::string(AutoNumberPrefix(kind)) + " ";
    for (const auto& t : tabs_) {
      if (t->kind() != kind) continue;
      auto* sb = static_cast<SimpleTabBehavior*>(t->behavior());
      if (!sb) continue;
      const std::string& name = sb->display_name();
      if (name.compare(0, prefix.size(), prefix) != 0) continue;
      const char* p = name.c_str() + prefix.size();
      char* end = nullptr;
      long v = std::strtol(p, &end, 10);
      if (end != p && v > max_n) max_n = static_cast<int>(v);
    }
    effective.display_name = prefix + std::to_string(max_n + 1);
  }

  std::unique_ptr<TabBehavior> behavior = MakeBehavior(kind, effective);
  if (!behavior) {
    // Phase 1: no behaviors registered yet; surface the missing factory
    // explicitly so later phases catch the wiring.
    return {};
  }
  TabId id = NewId();
  auto tab = std::make_unique<Tab>(id, kind, std::move(behavior));
  // Apply seed metadata from open params (e.g. "chat_id" for restored tabs).
  for (const auto& [k, v] : effective.meta) {
    tab->SetMeta(k, v);
  }
  tab->Build();
  tabs_.push_back(std::move(tab));

  auto it = singleton_kinds_registered_.find(kind);
  if (it != singleton_kinds_registered_.end() && it->second) {
    singletons_[kind] = id;
  }
  if (on_change_) on_change_();
  return id;
}

TabId TabManager::FindOrCreateSingleton(TabKind kind, bool* out_created) {
  [[maybe_unused]] auto reg = singleton_kinds_registered_.find(kind);
  assert(reg != singleton_kinds_registered_.end() && reg->second &&
         "FindOrCreateSingleton called for an unregistered kind");
  auto existing = singletons_.find(kind);
  if (existing != singletons_.end()) {
    if (out_created) *out_created = false;
    return existing->second;
  }
  TabId id = Open(kind, OpenParams{});
  if (out_created) *out_created = !id.empty();
  return id;
}

void TabManager::Activate(const TabId& id) {
  if (!Get(id)) {
    return;
  }
  active_tab_id_ = id;
  if (on_change_) on_change_();
  // Host-panel swap wired in Phase 4+.
}

void TabManager::Close(const TabId& id) {
  for (auto it = tabs_.begin(); it != tabs_.end(); ++it) {
    if ((*it)->tab_id() == id) {
      TabKind kind = (*it)->kind();
      tabs_.erase(it);
      auto sit = singletons_.find(kind);
      if (sit != singletons_.end() && sit->second == id) {
        singletons_.erase(sit);
      }
      if (active_tab_id_ == id) {
        active_tab_id_.clear();
      }
      if (on_change_) on_change_();
      return;
    }
  }
}

Tab* TabManager::Get(const TabId& id) {
  for (auto& t : tabs_) {
    if (t->tab_id() == id) return t.get();
  }
  return nullptr;
}

const Tab* TabManager::Get(const TabId& id) const {
  for (const auto& t : tabs_) {
    if (t->tab_id() == id) return t.get();
  }
  return nullptr;
}

Tab* TabManager::Active() {
  return active_tab_id_.empty() ? nullptr : Get(active_tab_id_);
}

Tab* TabManager::FindByBrowserId(int browser_id) {
  if (browser_id == 0) return nullptr;
  for (auto& t : tabs_) {
    if (t->browser_id() == browser_id) return t.get();
  }
  return nullptr;
}

std::vector<TabSummary> TabManager::Snapshot() const {
  std::vector<TabSummary> out;
  out.reserve(tabs_.size());
  for (const auto& t : tabs_) {
    out.push_back(
        TabSummary{t->tab_id(), t->kind(), DisplayNameFor(t.get()), t->meta()});
  }
  return out;
}

void TabManager::SetTabMeta(const TabId& id, const std::string& key,
                            const std::string& value) {
  if (Tab* t = Get(id)) t->SetMeta(key, value);
}

std::string TabManager::GetTabMeta(const TabId& id,
                                   const std::string& key) const {
  if (const Tab* t = Get(id)) return t->GetMeta(key);
  return {};
}

std::unique_ptr<TabBehavior> TabManager::MakeBehavior(TabKind kind,
                                                     const OpenParams& params) {
  auto resolve_url = [&](const char* fallback) -> std::string {
    if (!params.url.empty()) return params.url;
    auto it = kind_content_urls_.find(kind);
    if (it != kind_content_urls_.end() && !it->second.empty()) {
      return it->second;
    }
    return fallback;
  };
  switch (kind) {
    case TabKind::kWeb: {
      if (!client_handler_) return nullptr;
      const std::string url = resolve_url("https://www.google.com");
      return std::make_unique<WebTabBehavior>(client_handler_, url);
    }
    case TabKind::kTerminal:
      if (!client_handler_) return nullptr;
      return std::make_unique<SimpleTabBehavior>(
          client_handler_, kind, std::string("\xEE\x9C\x80"),
          params.display_name.empty() ? std::string("Terminal")
                                      : params.display_name,
          resolve_url("about:blank"));
    case TabKind::kChat:
      if (!client_handler_) return nullptr;
      return std::make_unique<SimpleTabBehavior>(
          client_handler_, kind, std::string("\xF0\x9F\x92\xAC"),
          params.display_name.empty() ? std::string("Chat")
                                      : params.display_name,
          resolve_url("about:blank"));
    case TabKind::kSettings:
      if (!client_handler_) return nullptr;
      return std::make_unique<SimpleTabBehavior>(
          client_handler_, kind, std::string("\xE2\x9A\x99"),
          "Settings", resolve_url("about:blank"));
  }
  return nullptr;
}

TabId TabManager::NewId() {
  std::ostringstream os;
  os << "tab-" << next_id_seq_++;
  return os.str();
}

}  // namespace cronymax
