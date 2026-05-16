// app/browser/shells/bridge_ui.cc
// UI chrome channels: shell.*, theme.*, tab.*, browser.*, agent.*

#include "browser/bridge_handler.h"

#include <nlohmann/json.hpp>

namespace cronymax {

static inline nlohmann::json ParseShellCbResult(const std::string& s) {
  if (s.empty())
    return nlohmann::json::object();
  auto j = nlohmann::json::parse(s, nullptr, false);
  return j.is_discarded() ? nlohmann::json::object() : j;
}

// ---------------------------------------------------------------------------
// RegisterShellHandlers — install all UI chrome channels in the BridgeRegistry.
// ---------------------------------------------------------------------------

void RegisterShellHandlers(BridgeRegistry& r, BridgeHandler* h) {
  // ── agent.task_from_command ───────────────────────────────────────────────
  r.add("browser.agent.task_from_command", [h](BridgeCtx ctx) {
    h->SendBrowserEvent(ctx.browser, "agent.task_from_command",
                        ctx.payload.dump());
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.webview.get_active ───────────────────────────────────────────
  r.add("browser.webview.get_active", [](BridgeCtx ctx) {
    (void)ctx.payload;
    if (!ctx.browser) {
      ctx.callback->Failure(503, "no browser");
      return;
    }
    const auto frame = ctx.browser->GetMainFrame();
    const std::string url = frame ? frame->GetURL().ToString() : "";
    ctx.callback->Success(nlohmann::json{{"url", url}, {"text", ""}});
  });

  // ── browser.shell.tabs_list ───────────────────────────────────────────────
  r.add("browser.shell.tabs_list", [h](BridgeCtx ctx) {
    if (!h->shell_cbs_.list_tabs) {
      ctx.callback->Success(nlohmann::json{{"tabs", nlohmann::json::array()},
                                           {"active_tab_id", -1}});
      return;
    }
    ctx.callback->Success(ParseShellCbResult(h->shell_cbs_.list_tabs()));
  });

  // ── browser.shell.tab_new ─────────────────────────────────────────────────
  r.add("browser.shell.tab_new", [h](BridgeCtx ctx) {
    if (!h->shell_cbs_.new_tab) {
      ctx.callback->Failure(503, "not available");
      return;
    }
    const std::string url = ctx.payload.is_object()
                                ? ctx.payload.value("url", std::string{})
                                : std::string{};
    ctx.callback->Success(ParseShellCbResult(
        h->shell_cbs_.new_tab(url.empty() ? "https://www.google.com" : url)));
  });

  // ── browser.shell.tab_switch ──────────────────────────────────────────────
  r.add("browser.shell.tab_switch", [h](BridgeCtx ctx) {
    const std::string sid = ctx.payload.is_object()
                                ? ctx.payload.value("id", std::string{})
                                : std::string{};
    if (sid.empty()) {
      ctx.callback->Success(nlohmann::json{{"ok", true}});
      return;
    }
    if (h->shell_cbs_.tab_activate_str && h->shell_cbs_.tab_activate_str(sid)) {
      ctx.callback->Success(nlohmann::json{{"ok", true}});
      return;
    }
    if (h->shell_cbs_.switch_tab) {
      char* end = nullptr;
      long v = std::strtol(sid.c_str(), &end, 10);
      if (end && end != sid.c_str() && *end == '\0')
        h->shell_cbs_.switch_tab(static_cast<int>(v));
    }
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.tab_close ───────────────────────────────────────────────
  r.add("browser.shell.tab_close", [h](BridgeCtx ctx) {
    const std::string sid = ctx.payload.is_object()
                                ? ctx.payload.value("id", std::string{})
                                : std::string{};
    if (sid.empty()) {
      ctx.callback->Success(nlohmann::json{{"ok", true}});
      return;
    }
    if (h->shell_cbs_.tab_close_str && h->shell_cbs_.tab_close_str(sid)) {
      ctx.callback->Success(nlohmann::json{{"ok", true}});
      return;
    }
    if (h->shell_cbs_.close_tab) {
      char* end = nullptr;
      long v = std::strtol(sid.c_str(), &end, 10);
      if (end && end != sid.c_str() && *end == '\0')
        h->shell_cbs_.close_tab(static_cast<int>(v));
    }
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.tab_open_singleton ─────────────────────────────────────
  r.add("browser.shell.tab_open_singleton", [h](BridgeCtx ctx) {
    if (!h->shell_cbs_.tab_open_singleton) {
      ctx.callback->Failure(503, "not available");
      return;
    }
    const std::string kind = ctx.payload.is_object()
                                 ? ctx.payload.value("kind", std::string{})
                                 : std::string{};
    ctx.callback->Success(
        ParseShellCbResult(h->shell_cbs_.tab_open_singleton(kind)));
  });

  // ── browser.shell.tab_new_kind ────────────────────────────────────────────
  r.add("browser.shell.tab_new_kind", [h](BridgeCtx ctx) {
    if (!h->shell_cbs_.new_tab_kind) {
      ctx.callback->Failure(503, "not available");
      return;
    }
    const std::string kind = ctx.payload.is_object()
                                 ? ctx.payload.value("kind", std::string{})
                                 : std::string{};
    ctx.callback->Success(ParseShellCbResult(h->shell_cbs_.new_tab_kind(kind)));
  });

  // ── browser.shell.this_tab_id ─────────────────────────────────────────────
  r.add("browser.shell.this_tab_id", [h](BridgeCtx ctx) {
    if (!h->shell_cbs_.this_tab_id) {
      ctx.callback->Success(
          nlohmann::json{{"tabId", ""}, {"meta", nlohmann::json::object()}});
      return;
    }
    const int bid = ctx.browser ? ctx.browser->GetIdentifier() : 0;
    ctx.callback->Success(ParseShellCbResult(h->shell_cbs_.this_tab_id(bid)));
  });

  // ── browser.shell.tab_set_meta ────────────────────────────────────────────
  r.add("browser.shell.tab_set_meta", [h](BridgeCtx ctx) {
    const auto& j = ctx.payload;
    const std::string key =
        j.is_object() ? j.value("key", std::string{}) : std::string{};
    const std::string value =
        j.is_object() ? j.value("value", std::string{}) : std::string{};
    if (!key.empty() && h->shell_cbs_.tab_set_meta) {
      const int bid = ctx.browser ? ctx.browser->GetIdentifier() : 0;
      h->shell_cbs_.tab_set_meta(bid, key, value);
    }
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.show_panel ──────────────────────────────────────────────
  r.add("browser.shell.show_panel", [](BridgeCtx ctx) {
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.navigate ────────────────────────────────────────────────
  r.add("browser.shell.navigate", [h](BridgeCtx ctx) {
    if (!h->shell_cbs_.navigate) {
      ctx.callback->Success(nlohmann::json{{"ok", true}});
      return;
    }
    const std::string url = ctx.payload.is_object()
                                ? ctx.payload.value("url", std::string{})
                                : std::string{};
    if (!url.empty())
      h->shell_cbs_.navigate(url);
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.go_back ─────────────────────────────────────────────────
  r.add("browser.shell.go_back", [h](BridgeCtx ctx) {
    if (h->shell_cbs_.go_back)
      h->shell_cbs_.go_back();
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.go_forward ──────────────────────────────────────────────
  r.add("browser.shell.go_forward", [h](BridgeCtx ctx) {
    if (h->shell_cbs_.go_forward)
      h->shell_cbs_.go_forward();
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.reload ──────────────────────────────────────────────────
  r.add("browser.shell.reload", [h](BridgeCtx ctx) {
    if (h->shell_cbs_.reload)
      h->shell_cbs_.reload();
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.popover_open ────────────────────────────────────────────
  r.add("browser.shell.popover_open", [h](BridgeCtx ctx) {
    if (!h->shell_cbs_.popover_open) {
      ctx.callback->Success(nlohmann::json{{"ok", true}});
      return;
    }
    const std::string url = ctx.payload.is_object()
                                ? ctx.payload.value("url", std::string{})
                                : std::string{};
    h->shell_cbs_.popover_open(url.empty() ? "https://www.google.com" : url);
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.open_external ───────────────────────────────────────────
  r.add("browser.shell.open_external", [h](BridgeCtx ctx) {
    const std::string url = ctx.payload.is_object()
                                ? ctx.payload.value("url", std::string{})
                                : std::string{};
    if (!url.empty() && h->shell_cbs_.open_external)
      h->shell_cbs_.open_external(url);
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.popover_close ───────────────────────────────────────────
  r.add("browser.shell.popover_close", [h](BridgeCtx ctx) {
    if (h->shell_cbs_.popover_close)
      h->shell_cbs_.popover_close();
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.popover_refresh ─────────────────────────────────────────
  r.add("browser.shell.popover_refresh", [h](BridgeCtx ctx) {
    if (h->shell_cbs_.popover_refresh)
      h->shell_cbs_.popover_refresh();
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.popover_open_as_tab ────────────────────────────────────
  r.add("browser.shell.popover_open_as_tab", [h](BridgeCtx ctx) {
    if (h->shell_cbs_.popover_open_as_tab)
      h->shell_cbs_.popover_open_as_tab();
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.popover_navigate ───────────────────────────────────────
  r.add("browser.shell.popover_navigate", [h](BridgeCtx ctx) {
    const std::string url = ctx.payload.is_object()
                                ? ctx.payload.value("url", std::string{})
                                : std::string{};
    if (!url.empty() && h->shell_cbs_.popover_navigate)
      h->shell_cbs_.popover_navigate(url);
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.window_drag ─────────────────────────────────────────────
  r.add("browser.shell.window_drag", [h](BridgeCtx ctx) {
    if (h->shell_cbs_.window_drag)
      h->shell_cbs_.window_drag();
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.set_drag_regions ───────────────────────────────────────
  r.add("browser.shell.set_drag_regions", [](BridgeCtx ctx) {
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.shell.settings_popover_open ──────────────────────────────────
  r.add("browser.shell.settings_popover_open", [h](BridgeCtx ctx) {
    if (h->shell_cbs_.settings_popover_open)
      h->shell_cbs_.settings_popover_open();
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.theme.get ─────────────────────────────────────────────────────
  r.add("browser.theme.get", [h](BridgeCtx ctx) {
    if (!h->theme_cbs_.get_mode) {
      ctx.callback->Success(
          nlohmann::json{{"mode", "system"}, {"resolved", "dark"}});
      return;
    }
    ctx.callback->Success(ParseShellCbResult(h->theme_cbs_.get_mode()));
  });

  // ── browser.theme.set ─────────────────────────────────────────────────────
  r.add("browser.theme.set", [h](BridgeCtx ctx) {
    const auto& j = ctx.payload;
    const std::string mode =
        j.is_object() ? j.value("mode", std::string{}) : std::string{};
    if (mode != "system" && mode != "light" && mode != "dark") {
      ctx.callback->Failure(400, "invalid mode");
      return;
    }
    if (h->theme_cbs_.set_mode)
      h->theme_cbs_.set_mode(mode);
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.tab.set_toolbar_state ────────────────────────────────────────
  r.add("browser.tab.set_toolbar_state", [h](BridgeCtx ctx) {
    if (!h->shell_cbs_.set_toolbar_state) {
      ctx.callback->Success(nlohmann::json{{"ok", true}});
      return;
    }
    const auto& j = ctx.payload;
    const std::string tab_id =
        j.is_object() ? j.value("tabId", std::string{}) : std::string{};
    if (tab_id.empty()) {
      ctx.callback->Failure(400, "missing tabId");
      return;
    }
    std::string state_json;
    if (j.is_object() && j.contains("state") && j["state"].is_object())
      state_json = j["state"].dump();
    if (state_json.empty()) {
      ctx.callback->Failure(400, "missing state");
      return;
    }
    if (!h->shell_cbs_.set_toolbar_state(tab_id, state_json)) {
      ctx.callback->Failure(409, "tab kind mismatch or unknown tab");
      return;
    }
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── browser.tab.set_chrome_theme ─────────────────────────────────────────
  r.add("browser.tab.set_chrome_theme", [h](BridgeCtx ctx) {
    if (!h->shell_cbs_.set_chrome_theme) {
      ctx.callback->Success(nlohmann::json{{"ok", true}});
      return;
    }
    const auto& j = ctx.payload;
    const std::string tab_id =
        j.is_object() ? j.value("tabId", std::string{}) : std::string{};
    if (tab_id.empty()) {
      ctx.callback->Failure(400, "missing tabId");
      return;
    }
    std::string color;
    if (j.is_object() && j.contains("color") && j["color"].is_string())
      color = j["color"].get<std::string>();
    if (!h->shell_cbs_.set_chrome_theme(tab_id, color)) {
      ctx.callback->Failure(404, "unknown tab");
      return;
    }
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });
}

}  // namespace cronymax
