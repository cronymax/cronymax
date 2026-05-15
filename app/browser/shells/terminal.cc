// app/browser/shells/bridge_terminal.cc
// terminal.* bridge channels — list, new, switch, close, blocks.

#include "browser/bridge_handler.h"

#include <nlohmann/json.hpp>

namespace cronymax {

// ---------------------------------------------------------------------------
// RegisterTerminalHandlers — install browser.terminal.* in the BridgeRegistry.
// NOTE: terminal.restart is intentionally omitted — it moves to runtime.*.
// ---------------------------------------------------------------------------

void RegisterTerminalHandlers(BridgeRegistry& r, BridgeHandler* h) {
  r.add("browser.terminal.list", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    nlohmann::json items = nlohmann::json::array();
    for (const auto& t : sp->terminals)
      items.push_back({{"id", t->id}, {"name", t->name}});
    ctx.callback->Success(
        nlohmann::json{{"active", sp->active_terminal_id}, {"items", items}});
  });

  r.add("browser.terminal.new", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    auto* t = sp->CreateTerminal();
    const auto item = nlohmann::json{{"id", t->id}, {"name", t->name}};
    const auto switched = nlohmann::json{{"id", t->id}};
    if (h->shell_cbs_.broadcast_event) {
      h->shell_cbs_.broadcast_event("terminal.created", item.dump());
      h->shell_cbs_.broadcast_event("terminal.switched", switched.dump());
    } else {
      h->SendBrowserEvent(ctx.browser, "terminal.created", item.dump());
      h->SendBrowserEvent(ctx.browser, "terminal.switched", switched.dump());
    }
    ctx.callback->Success(item);
  });

  r.add("browser.terminal.switch", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    const auto& j = ctx.payload;
    const std::string id =
        j.is_object() ? j.value("id", std::string{}) : std::string{};
    if (id.empty() || !sp->FindTerminal(id)) {
      ctx.callback->Failure(404, "no such terminal");
      return;
    }
    sp->active_terminal_id = id;
    const std::string body = nlohmann::json{{"id", id}}.dump();
    if (h->shell_cbs_.broadcast_event)
      h->shell_cbs_.broadcast_event("terminal.switched", body);
    else
      h->SendBrowserEvent(ctx.browser, "terminal.switched", body);
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  r.add("browser.terminal.close", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    const auto& j = ctx.payload;
    const std::string id =
        j.is_object() ? j.value("id", std::string{}) : std::string{};
    if (!sp->CloseTerminal(id)) {
      ctx.callback->Failure(404, "no such terminal");
      return;
    }
    const std::string removed = nlohmann::json{{"id", id}}.dump();
    if (h->shell_cbs_.broadcast_event)
      h->shell_cbs_.broadcast_event("terminal.removed", removed);
    else
      h->SendBrowserEvent(ctx.browser, "terminal.removed", removed);
    if (!sp->active_terminal_id.empty()) {
      const std::string sw =
          nlohmann::json{{"id", sp->active_terminal_id}}.dump();
      if (h->shell_cbs_.broadcast_event)
        h->shell_cbs_.broadcast_event("terminal.switched", sw);
      else
        h->SendBrowserEvent(ctx.browser, "terminal.switched", sw);
    }
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  r.add("browser.terminal.blocks_load", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    const auto& j = ctx.payload;
    const std::string sid =
        j.is_object() ? j.value("space_id", std::string{}) : std::string{};
    const std::string& effective_sid = sid.empty() ? sp->id : sid;
    const auto blocks =
        h->space_manager_->store().ListBlocksForSpace(effective_sid);
    nlohmann::json arr = nlohmann::json::array();
    for (const auto& b : blocks) {
      arr.push_back({
          {"id", b.id},
          {"command", b.command},
          {"output", b.output},
          {"exit_code", b.exit_code},
          {"started_at", b.started_at},
          {"ended_at", b.ended_at},
      });
    }
    ctx.callback->Success(arr);
  });

  r.add("browser.terminal.block_save", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    const auto& j = ctx.payload;
    TerminalBlockRow row;
    row.space_id =
        j.is_object() ? j.value("space_id", std::string{}) : std::string{};
    if (row.space_id.empty())
      row.space_id = sp->id;
    row.command =
        j.is_object() ? j.value("command", std::string{}) : std::string{};
    row.output =
        j.is_object() ? j.value("output", std::string{}) : std::string{};
    if (j.is_object()) {
      if (j.contains("exit_code") && j["exit_code"].is_number())
        row.exit_code = j["exit_code"].get<int>();
      if (j.contains("started_at") && j["started_at"].is_number())
        row.started_at = j["started_at"].get<long long>();
      if (j.contains("ended_at") && j["ended_at"].is_number())
        row.ended_at = j["ended_at"].get<long long>();
    }
    h->space_manager_->store().CreateBlock(row);
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });
}

}  // namespace cronymax
