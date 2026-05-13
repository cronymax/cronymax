// app/browser/models/view_dispatcher.cc

#include "browser/models/view_dispatcher.h"

#include <cstdlib>
#include <string>
#include <vector>

#include <nlohmann/json.hpp>

#include "browser/bridge_handler.h"
#include "browser/client_handler.h"
#include "browser/models/view_context.h"
#if defined(__APPLE__)
#include "browser/platform/open_url_mac.h"
#endif
#include "browser/models/view_model.h"
#include "browser/tab/tab.h"
#include "browser/tab/tab_behavior.h"
#include "browser/tab/tab_manager.h"
#include "browser/tab/web_tab_behavior.h"
#include "include/cef_parser.h"
#include "include/cef_task.h"

namespace cronymax {

ViewDispatcher::ViewDispatcher(TabsContext* tabs_ctx,
                               SpaceContext* space_ctx,
                               OverlayActionContext* overlay_ctx,
                               ResourceContext* resource_ctx,
                               ClientHandler* client_handler,
                               ViewModel* model,
                               DispatcherHost host)
    : tabs_ctx_(tabs_ctx),
      space_ctx_(space_ctx),
      overlay_ctx_(overlay_ctx),
      resource_ctx_(resource_ctx),
      client_handler_(client_handler),
      model_(model),
      host_(std::move(host)) {}

// ---------------------------------------------------------------------------
// Wire — ShellCallbacks + ThemeCallbacks
// ---------------------------------------------------------------------------
void ViewDispatcher::Wire() {
  // ── Shell callbacks (TabManager-backed) ─────────────────────────────────
  ShellCallbacks sh;

  sh.list_tabs = [this]() -> std::string {
    nlohmann::json tabs_arr = nlohmann::json::array();
    for (const auto& s : model_->tabs_->Snapshot()) {
      nlohmann::json entry = {{"kind", TabKindToString(s.kind)},
                              {"id", s.id},
                              {"displayName", s.display_name}};
      if (s.kind == TabKind::kWeb) {
        Tab* t = model_->tabs_->Get(s.id);
        auto* wb = t ? static_cast<WebTabBehavior*>(t->behavior()) : nullptr;
        if (wb)
          entry["url"] = wb->current_url();
      }
      tabs_arr.push_back(std::move(entry));
    }
    nlohmann::json result = {{"tabs", std::move(tabs_arr)}};
    const std::string& aid = model_->tabs_->active_tab_id();
    result["activeTabId"] =
        aid.empty() ? nlohmann::json(nullptr) : nlohmann::json(aid);
    return result.dump();
  };

  sh.new_tab = [this](const std::string& url) -> std::string {
    const std::string raw = url.empty() ? "https://www.google.com" : url;
    const TabId id = tabs_ctx_->OpenWebTab(raw);
    if (id.empty())
      return "{}";
    const std::string final_url =
        raw.find("://") == std::string::npos ? "https://" + raw : raw;
    const std::string json = nlohmann::json{
        {"id", id},
        {"url", final_url},
        {"title", ""},
        {"is_pinned", false}}.dump();
    host_.push_to_sidebar("shell.tab_created", json);
    return json;
  };

  sh.navigate = [this](const std::string& url) {
    Tab* tab = model_->tabs_->Active();
    if (!tab || tab->kind() != TabKind::kWeb) {
      tabs_ctx_->OpenWebTab(url);
      return;
    }
    if (auto* wb = static_cast<WebTabBehavior*>(tab->behavior())) {
      wb->Navigate(url);
    }
  };

  sh.go_back = [this]() {
    Tab* tab = model_->tabs_->Active();
    if (!tab || tab->kind() != TabKind::kWeb)
      return;
    if (auto* wb = static_cast<WebTabBehavior*>(tab->behavior()))
      wb->GoBack();
  };

  sh.go_forward = [this]() {
    Tab* tab = model_->tabs_->Active();
    if (!tab || tab->kind() != TabKind::kWeb)
      return;
    if (auto* wb = static_cast<WebTabBehavior*>(tab->behavior()))
      wb->GoForward();
  };

  sh.popover_open = [this](const std::string& u) {
    overlay_ctx_->OpenPopover(u);
  };
  sh.popover_close = [this]() { overlay_ctx_->ClosePopover(); };
  sh.popover_refresh = [this]() { host_.popover_reload(); };

  sh.settings_popover_open = [this]() {
    overlay_ctx_->OpenPopover(
        resource_ctx_->ResourceUrl("panels/settings/index.html"));
  };

  sh.popover_open_as_tab = [this]() {
    const std::string url = host_.get_popover_url();
    if (url.empty())
      return;
    overlay_ctx_->ClosePopover();
    const TabId id = tabs_ctx_->OpenWebTab(url);
    if (id.empty())
      return;
    host_.push_to_sidebar(
        "shell.tab_created",
        nlohmann::json{
            {"id", id}, {"url", url}, {"title", ""}, {"is_pinned", false}}
            .dump());
  };

  sh.popover_navigate = [this](const std::string& url) {
    host_.popover_navigate_url(url);
  };

#if defined(__APPLE__)
  sh.open_external = [](const std::string& url) { OpenUrlExternal(url); };
#endif

  sh.reload = [this]() {
    Tab* tab = model_->tabs_->Active();
    if (!tab || tab->kind() != TabKind::kWeb)
      return;
    if (auto* wb = static_cast<WebTabBehavior*>(tab->behavior()))
      wb->Reload();
  };

  sh.terminal_restart = [this]() {
    host_.broadcast("terminal.restart_requested", "{}");
  };

  sh.window_drag = [this]() { host_.window_drag(); };

  sh.broadcast_event = [this](const std::string& ev, const std::string& body) {
    host_.broadcast(ev, body);
  };

  auto kind_from_string = [](const std::string& s, TabKind* out) -> bool {
    if (s == "web") {
      *out = TabKind::kWeb;
      return true;
    }
    if (s == "chat") {
      *out = TabKind::kChat;
      return true;
    }
    if (s == "terminal") {
      *out = TabKind::kTerminal;
      return true;
    }
    if (s == "settings") {
      *out = TabKind::kSettings;
      return true;
    }
    return false;
  };

  sh.tab_activate_str = [this](const std::string& tab_id) -> bool {
    Tab* tab = model_->tabs_->Get(tab_id);
    if (!tab)
      return false;
    model_->tabs_->Activate(tab_id);
    return true;
  };

  sh.tab_close_str = [this](const std::string& tab_id) -> bool {
    Tab* tab = model_->tabs_->Get(tab_id);
    if (!tab)
      return false;
    const int closed_browser_id = tab->browser_id();
    host_.remove_tab_card(tab_id);
    host_.persist_tab_closed(tab_id);
    if (closed_browser_id != 0 &&
        host_.get_popover_owner_browser_id() == closed_browser_id) {
      overlay_ctx_->ClosePopover();
    }
    model_->tabs_->Close(tab_id);
    host_.push_to_sidebar("shell.tab_closed",
                          nlohmann::json{{"id", tab_id}}.dump());
    if (model_->tabs_->active_tab_id().empty()) {
      const auto snap = model_->tabs_->Snapshot();
      if (!snap.empty())
        model_->tabs_->Activate(snap.front().id);
    }
    return true;
  };

  sh.tab_open_singleton =
      [this, kind_from_string](const std::string& kind_s) -> std::string {
    TabKind kind;
    if (!kind_from_string(kind_s, &kind)) {
      return "{\"tabId\":\"\",\"created\":false}";
    }
    if (!model_->tabs_->IsSingletonKind(kind)) {
      return "{\"tabId\":\"\",\"created\":false}";
    }
    bool created = false;
    TabId id = model_->tabs_->FindOrCreateSingleton(kind, &created);
    if (!id.empty())
      model_->tabs_->Activate(id);
    return nlohmann::json{{"tabId", id}, {"created", created}}.dump();
  };

  sh.new_tab_kind =
      [this, kind_from_string](const std::string& kind_s) -> std::string {
    TabKind kind;
    if (!kind_from_string(kind_s, &kind))
      return "{}";
    TabId id;
    if (kind == TabKind::kWeb) {
      id = tabs_ctx_->OpenWebTab("https://www.google.com");
    } else if (kind == TabKind::kTerminal || kind == TabKind::kChat) {
      id = model_->tabs_->Open(kind, OpenParams{});
      if (!id.empty())
        model_->tabs_->Activate(id);
    } else {
      return "{}";
    }
    if (id.empty())
      return "{}";
    int numeric = 0;
    static constexpr char kPrefix[] = "tab-";
    if (id.compare(0, sizeof(kPrefix) - 1, kPrefix) == 0) {
      numeric = std::atoi(id.c_str() + sizeof(kPrefix) - 1);
    }
    const std::string tab_url =
        (kind == TabKind::kWeb) ? "https://www.google.com" : "";
    host_.push_to_sidebar("shell.tab_created",
                          nlohmann::json{{"id", numeric},
                                         {"url", tab_url},
                                         {"title", ""},
                                         {"is_pinned", false}}
                              .dump());
    return nlohmann::json{{"tabId", id}, {"kind", kind_s}}.dump();
  };

  sh.set_toolbar_state = [this, kind_from_string](
                             const std::string& tab_id,
                             const std::string& state_json) -> bool {
    Tab* tab = model_->tabs_->Get(tab_id);
    if (!tab)
      return false;
    auto kind_at = state_json.find("\"kind\"");
    if (kind_at == std::string::npos)
      return false;
    auto colon = state_json.find(':', kind_at);
    if (colon == std::string::npos)
      return false;
    auto q1 = state_json.find('"', colon);
    if (q1 == std::string::npos)
      return false;
    auto q2 = state_json.find('"', q1 + 1);
    if (q2 == std::string::npos)
      return false;
    const std::string kind_s = state_json.substr(q1 + 1, q2 - q1 - 1);
    TabKind kind;
    if (!kind_from_string(kind_s, &kind))
      return false;
    if (kind != tab->kind())
      return false;
    tab->OnToolbarState(ToolbarState{kind, state_json});
    return true;
  };

  sh.set_chrome_theme = [this](const std::string& tab_id,
                               const std::string& css) -> bool {
    Tab* tab = model_->tabs_->Get(tab_id);
    if (!tab)
      return false;
    tab->SetChromeTheme(css);
    return true;
  };

  sh.this_tab_id = [this](int browser_id) -> std::string {
    Tab* t =
        model_->tabs_ ? model_->tabs_->FindByBrowserId(browser_id) : nullptr;
    nlohmann::json meta = nlohmann::json::object();
    if (t) {
      for (const auto& [k, v] : t->meta())
        meta[k] = v;
    }
    return nlohmann::json{{"tabId", t ? t->tab_id() : ""},
                          {"meta", std::move(meta)}}
        .dump();
  };

  sh.tab_set_meta = [this](int browser_id, const std::string& key,
                           const std::string& value) -> bool {
    Tab* t =
        model_->tabs_ ? model_->tabs_->FindByBrowserId(browser_id) : nullptr;
    if (!t)
      return false;
    t->SetMeta(key, value);
    host_.persist_sidebar_tabs();
    return true;
  };

  model_->tabs_->SetOnChange([this]() {
    const auto snap = model_->tabs_->Snapshot();
    {
      nlohmann::json tabs_arr = nlohmann::json::array();
      for (const auto& s : snap) {
        nlohmann::json entry = {{"kind", TabKindToString(s.kind)},
                                {"id", s.id},
                                {"displayName", s.display_name}};
        if (s.kind == TabKind::kWeb) {
          Tab* t = model_->tabs_->Get(s.id);
          auto* wb = t ? static_cast<WebTabBehavior*>(t->behavior()) : nullptr;
          if (wb)
            entry["url"] = wb->current_url();
        }
        tabs_arr.push_back(std::move(entry));
      }
      nlohmann::json list_snap = {{"tabs", std::move(tabs_arr)}};
      const std::string& aid = model_->tabs_->active_tab_id();
      list_snap["activeTabId"] =
          aid.empty() ? nlohmann::json(nullptr) : nlohmann::json(aid);
      host_.broadcast("shell.tabs_list", list_snap.dump());
    }

    if (!model_->tabs_->active_tab_id().empty()) {
      host_.broadcast(
          "shell.tab_activated",
          nlohmann::json{{"tabId", model_->tabs_->active_tab_id()}}.dump());
    }

    {
      Tab* active = model_->tabs_->Active();
      std::string url;
      int browser_id = 0;
      if (active) {
        browser_id = active->browser_id();
        if (active->kind() == TabKind::kWeb) {
          if (auto* wb = static_cast<WebTabBehavior*>(active->behavior()))
            url = wb->current_url();
        }
      }
      model_->NotifyActiveTabChanged(url, browser_id);
    }

    host_.persist_tab_titles_if_changed();
    host_.persist_sidebar_tabs();
  });

  sh.run_file_dialog = host_.run_file_dialog;

  client_handler_->SetShellCallbacks(std::move(sh));

  // ── Theme callbacks ──────────────────────────────────────────────────────
  ThemeCallbacks theme_cbs;
  theme_cbs.get_mode = [this]() -> std::string {
    return model_->ThemeStateJson(/*include_chrome=*/false);
  };
  theme_cbs.set_mode = [this](const std::string& mode) {
    host_.handle_theme_mode_change(mode);
  };
  client_handler_->SetThemeCallbacks(std::move(theme_cbs));
}

}  // namespace cronymax
