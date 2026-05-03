#include "browser/bridge_handler.h"

#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <mutex>
#include <optional>
#include <set>
#include <sstream>

#include "document/document_store.h"
#include "document/review_store.h"
#include "document/reviews_state.h"
#include "event_bus/event_bus.h"
#include "flow/flow_runtime.h"
#include "flow/mention_parser.h"
#include "flow/trace_event.h"
#include "flow/trace_writer.h"
#include "flow/gitignore_helper.h"
#include "flow/workspace_layout.h"
#include "include/base/cef_callback.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {
namespace {

// ---------------------------------------------------------------------------
// JSON helpers (minimal, no external dependency)
// ---------------------------------------------------------------------------

std::string JsEscape(std::string_view value) {
  std::string out;
  out.reserve(value.size() + 2);
  for (char c : value) {
    switch (c) {
      case '\\': out += "\\\\"; break;
      case '"':  out += "\\\""; break;
      case '\n': out += "\\n";  break;
      case '\r': out += "\\r";  break;
      case '\t': out += "\\t";  break;
      default:   out += c;      break;
    }
  }
  return out;
}

std::string JsonString(std::string_view s) {
  return "\"" + JsEscape(s) + "\"";
}

// Extract value for "key" from a flat JSON object string.
std::string JsonGet(const std::string& json, std::string_view key) {
  const std::string search = "\"" + std::string(key) + "\"";
  const auto pos = json.find(search);
  if (pos == std::string::npos) return {};
  auto colon = json.find(':', pos + search.size());
  if (colon == std::string::npos) return {};
  auto vstart = json.find_first_not_of(" \t\r\n", colon + 1);
  if (vstart == std::string::npos) return {};
  if (json[vstart] == '"') {
    auto end = json.find('"', vstart + 1);
    while (end != std::string::npos && json[end - 1] == '\\') {
      end = json.find('"', end + 1);
    }
    if (end == std::string::npos) return {};
    return json.substr(vstart + 1, end - vstart - 1);
  }
  auto end = json.find_first_of(",}", vstart);
  return json.substr(vstart, end == std::string::npos ? std::string::npos
                                                       : end - vstart);
}

// Unescape a JSON string value (the raw substring extracted by JsonGet).
std::string JsonUnescape(const std::string& s) {
  std::string out;
  out.reserve(s.size());
  for (size_t i = 0; i < s.size(); ++i) {
    char c = s[i];
    if (c != '\\' || i + 1 >= s.size()) { out += c; continue; }
    char n = s[++i];
    switch (n) {
      case '"':  out += '"';  break;
      case '\\': out += '\\'; break;
      case '/':  out += '/';  break;
      case 'b':  out += '\b'; break;
      case 'f':  out += '\f'; break;
      case 'n':  out += '\n'; break;
      case 'r':  out += '\r'; break;
      case 't':  out += '\t'; break;
      case 'u': {
        if (i + 4 >= s.size()) break;
        unsigned cp = 0;
        for (int k = 0; k < 4; ++k) {
          char h = s[++i];
          cp <<= 4;
          if (h >= '0' && h <= '9') cp |= h - '0';
          else if (h >= 'a' && h <= 'f') cp |= h - 'a' + 10;
          else if (h >= 'A' && h <= 'F') cp |= h - 'A' + 10;
        }
        if (cp < 0x80) {
          out += static_cast<char>(cp);
        } else if (cp < 0x800) {
          out += static_cast<char>(0xC0 | (cp >> 6));
          out += static_cast<char>(0x80 | (cp & 0x3F));
        } else {
          out += static_cast<char>(0xE0 | (cp >> 12));
          out += static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
          out += static_cast<char>(0x80 | (cp & 0x3F));
        }
        break;
      }
      default: out += n; break;
    }
  }
  return out;
}

std::pair<std::string, std::string> SplitEnvelope(const std::string& request) {
  // Modern web bridge format: a JSON envelope
  // `{"channel":"...","payload":"<json-string>"}` where payload is itself a
  // JSON-encoded string. Detect this shape and decode.
  if (!request.empty() && request.front() == '{') {
    const std::string channel = JsonGet(request, "channel");
    if (!channel.empty()) {
      const std::string payload = JsonUnescape(JsonGet(request, "payload"));
      return {channel, payload};
    }
  }
  // Legacy format: "<channel>\n<payload>".
  const auto sep = request.find('\n');
  if (sep == std::string::npos) return {request, ""};
  return {request.substr(0, sep), request.substr(sep + 1)};
}

std::string SpaceToJson(const Space& sp) {
  return "{\"id\":" + JsonString(sp.id) +
         ",\"name\":" + JsonString(sp.name) +
         ",\"root_path\":" + JsonString(sp.workspace_root.string()) + "}";
}

}  // namespace

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

BridgeHandler::BridgeHandler(SpaceManager* space_manager)
    : space_manager_(space_manager) {}

BridgeHandler::~BridgeHandler() = default;

// ---------------------------------------------------------------------------
// OnQuery — route to subsystem handlers
// ---------------------------------------------------------------------------

bool BridgeHandler::OnQuery(CefRefPtr<CefBrowser> browser,
                            CefRefPtr<CefFrame> frame,
                            int64_t query_id,
                            const CefString& request,
                            bool persistent,
                            CefRefPtr<Callback> callback) {
  CEF_REQUIRE_UI_THREAD();
  (void)frame;
  (void)query_id;
  (void)persistent;

  const auto [channel, payload] = SplitEnvelope(request.ToString());

  if (channel.rfind("terminal.", 0) == 0)
    return HandleTerminal(browser, channel, payload, callback);
  if (channel.rfind("agent.registry.", 0) == 0)
    return HandleRegistry(channel, payload, callback);
  if (channel.rfind("agent.", 0) == 0)
    return HandleAgent(browser, channel, payload, callback);
  if (channel.rfind("space.", 0) == 0)
    return HandleSpace(browser, channel, payload, callback);
  if (channel.rfind("tool.", 0) == 0)
    return HandleTool(channel, payload, callback);
  if (channel.rfind("permission.", 0) == 0)
    return HandlePermission(channel, payload, callback);
  if (channel.rfind("llm.config", 0) == 0)
    return HandleLlmConfig(channel, payload, callback);
  if (channel.rfind("llm.providers", 0) == 0)
    return HandleLlmConfig(channel, payload, callback);
  if (channel.rfind("browser.", 0) == 0)
    return HandleBrowser(browser, channel, payload, callback);
  if (channel.rfind("shell.", 0) == 0)
    return HandleShell(channel, payload, callback);
  if (channel.rfind("theme.", 0) == 0)
    return HandleTheme(channel, payload, callback);
  if (channel.rfind("tab.", 0) == 0)
    return HandleTab(channel, payload, callback);

  if (channel.rfind("workspace.", 0) == 0)
    return HandleWorkspace(channel, payload, callback);

  // Phase A registries (read-only).
  if (channel.rfind("flow.", 0) == 0 ||
      channel.rfind("doc_type.", 0) == 0)
    return HandleRegistry(channel, payload, callback);

  if (channel.rfind("document.", 0) == 0)
    return HandleDocument(channel, payload, callback);

  if (channel.rfind("review.", 0) == 0)
    return HandleReview(channel, payload, callback);

  // agent-event-bus: typed event store, inbox, and notification prefs.
  if (channel.rfind("events.", 0) == 0)
    return HandleEvents(browser, channel, payload, callback);
  if (channel.rfind("inbox.", 0) == 0)
    return HandleInbox(channel, payload, callback);
  if (channel.rfind("notifications.", 0) == 0)
    return HandleNotifications(channel, payload, callback);

  callback->Failure(404, "unknown bridge channel");
  return true;
}

void BridgeHandler::OnQueryCanceled(CefRefPtr<CefBrowser> browser,
                                    CefRefPtr<CefFrame> frame,
                                    int64_t query_id) {
  (void)browser;
  (void)frame;
  (void)query_id;
}

// ---------------------------------------------------------------------------
// Terminal channels
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleTerminal(CefRefPtr<CefBrowser> browser,
                                   std::string_view channel,
                                   std::string_view payload,
                                   CefRefPtr<Callback> callback) {
  auto* sp = space_manager_->ActiveSpace();
  if (!sp) { callback->Failure(503, "no active space"); return true; }

  // Resolve a TerminalSession from optional "id" field; fall back to active.
  const std::string p_str(payload);
  auto resolve_terminal = [&](const std::string& payload_str) -> TerminalSession* {
    const std::string id = JsonGet(payload_str, "id");
    if (!id.empty()) return sp->FindTerminal(id);
    return sp->ActiveTerminal();
  };

  // List terminals for the active Space.
  if (channel == "terminal.list") {
    std::string json = "{\"active\":" + JsonString(sp->active_terminal_id) +
                       ",\"items\":[";
    for (size_t i = 0; i < sp->terminals.size(); ++i) {
      const auto& t = sp->terminals[i];
      if (i) json += ",";
      json += "{\"id\":" + JsonString(t->id) +
              ",\"name\":" + JsonString(t->name) + "}";
    }
    json += "]}";
    callback->Success(json);
    return true;
  }

  // Create a new terminal session (does NOT auto-start the PTY).
  if (channel == "terminal.new") {
    auto* t = sp->CreateTerminal();
    const std::string item =
        "{\"id\":" + JsonString(t->id) + ",\"name\":" + JsonString(t->name) + "}";
    if (shell_cbs_.broadcast_event) {
      shell_cbs_.broadcast_event("terminal.created", item);
      shell_cbs_.broadcast_event("terminal.switched",
                                 "{\"id\":" + JsonString(t->id) + "}");
    } else {
      SendEvent(browser, "terminal.created", item);
      SendEvent(browser, "terminal.switched",
                "{\"id\":" + JsonString(t->id) + "}");
    }
    callback->Success(item);
    return true;
  }

  // Switch the active terminal.
  if (channel == "terminal.switch") {
    const std::string id = JsonGet(p_str, "id");
    if (id.empty() || !sp->FindTerminal(id)) {
      callback->Failure(404, "no such terminal");
      return true;
    }
    sp->active_terminal_id = id;
    const std::string body = "{\"id\":" + JsonString(id) + "}";
    if (shell_cbs_.broadcast_event) {
      shell_cbs_.broadcast_event("terminal.switched", body);
    } else {
      SendEvent(browser, "terminal.switched", body);
    }
    callback->Success("ok");
    return true;
  }

  // Close a terminal session.
  if (channel == "terminal.close") {
    const std::string id = JsonGet(p_str, "id");
    if (!sp->CloseTerminal(id)) { callback->Failure(404, "no such terminal"); return true; }
    const std::string removed = "{\"id\":" + JsonString(id) + "}";
    if (shell_cbs_.broadcast_event) {
      shell_cbs_.broadcast_event("terminal.removed", removed);
    } else {
      SendEvent(browser, "terminal.removed", removed);
    }
    if (!sp->active_terminal_id.empty()) {
      const std::string sw =
          "{\"id\":" + JsonString(sp->active_terminal_id) + "}";
      if (shell_cbs_.broadcast_event) {
        shell_cbs_.broadcast_event("terminal.switched", sw);
      } else {
        SendEvent(browser, "terminal.switched", sw);
      }
    }
    callback->Success("ok");
    return true;
  }

  if (channel == "terminal.start") {
    auto* term = resolve_terminal(p_str);
    if (!term) { callback->Failure(404, "no such terminal"); return true; }
    auto* pty = term->pty.get();
    const std::string tid = term->id;
    if (!pty->running()) {
      const bool started = pty->Start(
          sp->workspace_root, "/bin/zsh",
          [this, browser, tid](std::string_view data) {
            const std::string payload =
                "{\"id\":" + JsonString(tid) +
                ",\"data\":" + JsonString(data) + "}";
            SendEvent(browser, "terminal.output", payload);
          },
          [this, browser, tid](int code) {
            const std::string payload =
                "{\"id\":" + JsonString(tid) +
                ",\"code\":" + std::to_string(code) + "}";
            SendEvent(browser, "terminal.exit", payload);
          });
      if (!started) { callback->Failure(500, "failed to start PTY"); return true; }
    }
    callback->Success("ok");
    return true;
  }

  if (channel == "terminal.input") {
    auto* term = resolve_terminal(p_str);
    if (!term) { callback->Failure(404, "no such terminal"); return true; }
    const std::string data = JsonUnescape(JsonGet(p_str, "data"));
    // Backward-compat: if no "data" field, treat the whole payload as input.
    const std::string& to_write = data.empty() ? p_str : data;
    if (term->pty->running()) term->pty->Write(to_write);
    callback->Success("ok");
    return true;
  }

  if (channel == "terminal.resize") {
    auto* term = resolve_terminal(p_str);
    if (!term) { callback->Failure(404, "no such terminal"); return true; }
    const std::string sc = JsonGet(p_str, "cols");
    const std::string sr = JsonGet(p_str, "rows");
    term->pty->Resize(sc.empty() ? 100 : std::stoi(sc), sr.empty() ? 30 : std::stoi(sr));
    callback->Success("ok");
    return true;
  }

  if (channel == "terminal.stop") {
    auto* term = resolve_terminal(p_str);
    if (!term) { callback->Failure(404, "no such terminal"); return true; }
    term->pty->Stop();
    callback->Success("ok");
    return true;
  }

  if (channel == "terminal.restart") {
    if (shell_cbs_.terminal_restart) shell_cbs_.terminal_restart();
    callback->Success("ok");
    return true;
  }

  if (channel == "terminal.block_save") {
    TerminalBlockRow row;
    row.space_id = JsonGet(p_str, "space_id");
    if (row.space_id.empty()) row.space_id = sp->id;
    row.command = JsonGet(p_str, "command");
    row.output = JsonGet(p_str, "output");
    const std::string ec = JsonGet(p_str, "exit_code");
    row.exit_code = ec.empty() ? -1 : std::stoi(ec);
    const std::string sa = JsonGet(p_str, "started_at");
    row.started_at = sa.empty() ? 0 : std::stoll(sa);
    const std::string ea = JsonGet(p_str, "ended_at");
    row.ended_at = ea.empty() ? 0 : std::stoll(ea);
    space_manager_->store().CreateBlock(row);
    callback->Success("ok");
    return true;
  }

  if (channel == "terminal.blocks_load") {
    const std::string sid =
        JsonGet(p_str, "space_id").empty() ? sp->id : JsonGet(p_str, "space_id");
    const auto blocks = space_manager_->store().ListBlocksForSpace(sid);
    std::string json = "[";
    for (size_t i = 0; i < blocks.size(); ++i) {
      const auto& b = blocks[i];
      if (i) json += ",";
      json += "{\"id\":" + std::to_string(b.id) +
              ",\"command\":" + JsonString(b.command) +
              ",\"output\":" + JsonString(b.output) +
              ",\"exit_code\":" + std::to_string(b.exit_code) +
              ",\"started_at\":" + std::to_string(b.started_at) +
              ",\"ended_at\":" + std::to_string(b.ended_at) + "}";
    }
    json += "]";
    callback->Success(json);
    return true;
  }

  callback->Failure(404, "unknown terminal channel");
  return true;
}

// ---------------------------------------------------------------------------
// Agent channels
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleAgent(CefRefPtr<CefBrowser> browser,
                                std::string_view channel,
                                std::string_view payload,
                                CefRefPtr<Callback> callback) {
  auto* sp = space_manager_->ActiveSpace();

  if (channel == "agent.run") {
    if (!sp) { callback->Failure(503, "no active space"); return true; }
    const auto result = sp->agent_runtime->RunPrototypeTask(std::string(payload));
    std::ostringstream out;
    out << result.final_message << "\n\nTrace:";
    for (const auto& ev : result.trace)
      out << "\n[" << ev.type << "] " << ev.message;
    if (result.ok) callback->Success(out.str());
    else           callback->Failure(500, out.str());
    return true;
  }

  if (channel == "agent.task_from_command") {
    SendEvent(browser, "agent.task_from_command", payload);
    callback->Success("ok");
    return true;
  }

  callback->Failure(404, "unknown agent channel");
  return true;
}

// ---------------------------------------------------------------------------
// Space channels
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleSpace(CefRefPtr<CefBrowser> browser,
                                std::string_view channel,
                                std::string_view payload,
                                CefRefPtr<Callback> callback) {
  if (channel == "space.list") {
    std::string json = "[";
    bool first = true;
    for (const auto& sp : space_manager_->spaces()) {
      if (!first) json += ",";
      json += SpaceToJson(*sp);
      first = false;
    }
    json += "]";
    callback->Success(json);
    return true;
  }

  if (channel == "space.create") {
    const std::string p(payload);
    const std::string name = JsonGet(p, "name");
    const std::string root = JsonGet(p, "root_path");
    if (name.empty() || root.empty()) {
      callback->Failure(400, "name and root_path required");
      return true;
    }
    const auto id = space_manager_->CreateSpace(name, root);
    if (id.empty()) {
      callback->Failure(500, "failed to create space (path may not exist)");
      return true;
    }
    for (const auto& s : space_manager_->spaces()) {
      if (s->id == id) {
        const std::string sj = SpaceToJson(*s);
        callback->Success(sj);
        SendEvent(browser, "space.created", sj);
        return true;
      }
    }
    callback->Success("{\"id\":" + JsonString(id) + "}");
    return true;
  }

  if (channel == "space.switch") {
    const std::string p(payload);
    const std::string id = JsonGet(p, "space_id");
    if (!space_manager_->SwitchTo(id)) {
      callback->Failure(404, "space not found");
      return true;
    }
    callback->Success("ok");
    return true;
  }

  if (channel == "space.delete") {
    const std::string p(payload);
    const std::string id = JsonGet(p, "space_id");
    if (!space_manager_->DeleteSpace(id)) {
      callback->Failure(404, "space not found");
      return true;
    }
    callback->Success("ok");
    SendEvent(browser, "space.deleted", "{\"space_id\":" + JsonString(id) + "}");
    return true;
  }

  callback->Failure(404, "unknown space channel");
  return true;
}

// ---------------------------------------------------------------------------
// Tool channels
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleTool(std::string_view channel,
                               std::string_view payload,
                               CefRefPtr<Callback> callback) {
  if (channel != "tool.exec") {
    callback->Failure(404, "unknown tool channel");
    return true;
  }
  auto* sp = space_manager_->ActiveSpace();
  if (!sp) { callback->Failure(503, "no active space"); return true; }

  const std::string p(payload);
  ToolCall call;
  call.name = JsonGet(p, "name");
  call.input = JsonGet(p, "input");
  if (call.name.empty()) { callback->Failure(400, "tool name required"); return true; }

  const auto result = sp->agent_runtime->tools().Invoke(call);
  if (result.ok) {
    callback->Success("{\"ok\":true,\"output\":" + JsonString(result.output) + "}");
  } else {
    callback->Failure(500, "{\"ok\":false,\"error\":" + JsonString(result.error) + "}");
  }
  return true;
}

// ---------------------------------------------------------------------------
// Permission channels
// ---------------------------------------------------------------------------

bool BridgeHandler::HandlePermission(std::string_view channel,
                                     std::string_view payload,
                                     CefRefPtr<Callback> callback) {
  if (channel == "permission.respond") {
    const std::string p(payload);
    const std::string rid = JsonGet(p, "request_id");
    const std::string dec = JsonGet(p, "decision");
    DeliverPermissionResponse(rid, dec == "allow");
    callback->Success("ok");
    return true;
  }
  callback->Failure(404, "unknown permission channel");
  return true;
}

void BridgeHandler::DeliverPermissionResponse(const std::string& request_id,
                                              bool allow) {
  std::lock_guard<std::mutex> lock(perm_mutex_);
  auto it = pending_permissions_.find(request_id);
  if (it != pending_permissions_.end()) {
    it->second(allow);
    pending_permissions_.erase(it);
  }
}

// ---------------------------------------------------------------------------
// LLM config channels
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleLlmConfig(std::string_view channel,
                                    std::string_view payload,
                                    CefRefPtr<Callback> callback) {
  if (channel == "llm.config.set") {
    const std::string p(payload);
    LlmConfig cfg;
    cfg.base_url = JsonGet(p, "base_url");
    cfg.api_key = JsonGet(p, "api_key");
    space_manager_->store().SetLlmConfig(cfg);
    callback->Success("ok");
    return true;
  }
  if (channel == "llm.config.get") {
    const auto cfg = space_manager_->store().GetLlmConfig();
    callback->Success("{\"base_url\":" + JsonString(cfg.base_url) +
                      ",\"api_key\":" + JsonString(cfg.api_key) + "}");
    return true;
  }
  if (channel == "llm.providers.get") {
    const std::string raw = space_manager_->store().GetKv("llm.providers");
    const std::string active =
        space_manager_->store().GetKv("llm.active_provider_id");
    callback->Success("{\"raw\":" + JsonString(raw) +
                      ",\"active_id\":" + JsonString(active) + "}");
    return true;
  }
  if (channel == "llm.providers.set") {
    const std::string p(payload);
    const std::string raw = JsonUnescape(JsonGet(p, "raw"));
    const std::string active = JsonUnescape(JsonGet(p, "active_id"));
    space_manager_->store().SetKv("llm.providers", raw);
    space_manager_->store().SetKv("llm.active_provider_id", active);
    callback->Success("{\"ok\":true}");
    return true;
  }
  callback->Failure(404, "unknown llm.config channel");
  return true;
}

// ---------------------------------------------------------------------------
// Browser channels
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleBrowser(CefRefPtr<CefBrowser> browser,
                                  std::string_view channel,
                                  std::string_view payload,
                                  CefRefPtr<Callback> callback) {
  (void)payload;
  if (channel == "browser.get_active_page") {
    if (!browser) { callback->Failure(503, "no browser"); return true; }
    const auto frame = browser->GetMainFrame();
    const std::string url = frame ? frame->GetURL().ToString() : "";
    callback->Success("{\"url\":" + JsonString(url) + ",\"text\":\"\"}");
    return true;
  }
  callback->Failure(404, "unknown browser channel");
  return true;
}

// ---------------------------------------------------------------------------
// SendEvent
// ---------------------------------------------------------------------------

void BridgeHandler::SendEvent(CefRefPtr<CefBrowser> browser,
                              std::string_view event,
                              std::string_view payload) {
  const std::string ev(event);
  const std::string pl(payload);

  auto dispatch = [ev, pl](CefRefPtr<CefBrowser> target) {
    if (!target) return;
    const auto frame = target->GetMainFrame();
    if (!frame) return;
    const std::string js =
        "window.__aiDesktopDispatch && window.__aiDesktopDispatch(" +
        ("\"" + ev + "\"") + "," + JsonString(pl) + ");";
    frame->ExecuteJavaScript(js, frame->GetURL(), 0);
  };

  if (!CefCurrentlyOn(TID_UI)) {
    CefPostTask(TID_UI,
                base::BindOnce([](CefRefPtr<CefBrowser> b, std::string e,
                                  std::string p) {
                                 if (!b) return;
                                 const auto frame = b->GetMainFrame();
                                 if (!frame) return;
                                 const std::string js =
                                     "window.__aiDesktopDispatch && "
                                     "window.__aiDesktopDispatch(\"" +
                                     e + "\"," + JsonString(p) + ");";
                                 frame->ExecuteJavaScript(js, frame->GetURL(), 0);
                               },
                               browser, ev, pl));
    return;
  }
  dispatch(browser);
}

// ---------------------------------------------------------------------------
// Shell channels (sidebar ↔ MainWindow tab / panel management)
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleShell(std::string_view channel,
                                std::string_view payload,
                                CefRefPtr<Callback> callback) {
  const std::string p(payload);

  if (channel == "shell.tabs_list") {
    if (!shell_cbs_.list_tabs) { callback->Success("{\"tabs\":[],\"active_tab_id\":-1}"); return true; }
    callback->Success(shell_cbs_.list_tabs());
    return true;
  }

  if (channel == "shell.tab_new") {
    if (!shell_cbs_.new_tab) { callback->Failure(503, "not available"); return true; }
    const std::string url = JsonGet(p, "url");
    callback->Success(shell_cbs_.new_tab(url.empty() ? "https://www.google.com" : url));
    return true;
  }

  if (channel == "shell.tab_switch") {
    const std::string sid = JsonGet(p, "id");
    if (sid.empty()) { callback->Success("ok"); return true; }
    // Try string-id (TabManager) first; fall back to legacy numeric.
    if (shell_cbs_.tab_activate_str && shell_cbs_.tab_activate_str(sid)) {
      callback->Success("ok");
      return true;
    }
    if (shell_cbs_.switch_tab) {
      char* end = nullptr;
      long v = std::strtol(sid.c_str(), &end, 10);
      if (end && end != sid.c_str() && *end == '\0') {
        shell_cbs_.switch_tab(static_cast<int>(v));
      }
    }
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.tab_close") {
    const std::string sid = JsonGet(p, "id");
    if (sid.empty()) { callback->Success("ok"); return true; }
    if (shell_cbs_.tab_close_str && shell_cbs_.tab_close_str(sid)) {
      callback->Success("ok");
      return true;
    }
    if (shell_cbs_.close_tab) {
      char* end = nullptr;
      long v = std::strtol(sid.c_str(), &end, 10);
      if (end && end != sid.c_str() && *end == '\0') {
        shell_cbs_.close_tab(static_cast<int>(v));
      }
    }
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.tab_open_singleton") {
    if (!shell_cbs_.tab_open_singleton) {
      callback->Failure(503, "not available");
      return true;
    }
    const std::string kind = JsonGet(p, "kind");
    callback->Success(shell_cbs_.tab_open_singleton(kind));
    return true;
  }

  if (channel == "shell.tab_new_kind") {
    if (!shell_cbs_.new_tab_kind) {
      callback->Failure(503, "not available");
      return true;
    }
    const std::string kind = JsonGet(p, "kind");
    const std::string out = shell_cbs_.new_tab_kind(kind);
    callback->Success(out.empty() ? "{}" : out);
    return true;
  }

  if (channel == "shell.show_panel") {
    // arc-style-tab-cards: panel switching is gone; the channel is
    // accepted as a no-op for renderer compatibility during the rollout.
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.navigate") {
    if (!shell_cbs_.navigate) { callback->Success("ok"); return true; }
    const std::string url = JsonGet(p, "url");
    if (!url.empty()) shell_cbs_.navigate(url);
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.go_back") {
    if (shell_cbs_.go_back) shell_cbs_.go_back();
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.go_forward") {
    if (shell_cbs_.go_forward) shell_cbs_.go_forward();
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.reload") {
    if (shell_cbs_.reload) shell_cbs_.reload();
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.popover_open") {
    if (!shell_cbs_.popover_open) { callback->Success("ok"); return true; }
    const std::string url = JsonGet(p, "url");
    shell_cbs_.popover_open(url.empty() ? "https://www.google.com" : url);
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.popover_close") {
    if (shell_cbs_.popover_close) shell_cbs_.popover_close();
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.popover_refresh") {
    if (shell_cbs_.popover_refresh) shell_cbs_.popover_refresh();
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.popover_open_as_tab") {
    if (shell_cbs_.popover_open_as_tab) shell_cbs_.popover_open_as_tab();
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.popover_navigate") {
    const std::string url = JsonUnescape(JsonGet(std::string(payload), "url"));
    if (!url.empty() && shell_cbs_.popover_navigate)
      shell_cbs_.popover_navigate(url);
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.window_drag") {
    if (shell_cbs_.window_drag) shell_cbs_.window_drag();
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.set_drag_regions") {
    // arc-style-tab-cards: native drag strips replace the JS-side region
    // pump; the channel is accepted as a no-op for renderer compatibility.
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.settings_popover_open") {
    // refine-ui-theme-layout: open Settings as a popover anchored at the
    // window. MainWindow resolves the URL via ResourceUrl().
    if (shell_cbs_.settings_popover_open) shell_cbs_.settings_popover_open();
    callback->Success("ok");
    return true;
  }

  callback->Failure(404, "unknown shell channel");
  return true;
}

// ---------------------------------------------------------------------------
// Theme channels (refine-ui-theme-layout)
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleTheme(std::string_view channel,
                                std::string_view payload,
                                CefRefPtr<Callback> callback) {
  const std::string p(payload);

  if (channel == "theme.get") {
    if (!theme_cbs_.get_mode) {
      callback->Success("{\"mode\":\"system\",\"resolved\":\"dark\"}");
      return true;
    }
    callback->Success(theme_cbs_.get_mode());
    return true;
  }

  if (channel == "theme.set") {
    const std::string mode = JsonGet(p, "mode");
    if (mode != "system" && mode != "light" && mode != "dark") {
      callback->Failure(400, "invalid mode");
      return true;
    }
    if (theme_cbs_.set_mode) theme_cbs_.set_mode(mode);
    callback->Success("ok");
    return true;
  }

  callback->Failure(404, "unknown theme channel");
  return true;
}

// ---------------------------------------------------------------------------
// Tab channels (arc-style-tab-cards Phase 2)
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleTab(std::string_view channel,
                              std::string_view payload,
                              CefRefPtr<Callback> callback) {
  const std::string p(payload);

  if (channel == "tab.set_toolbar_state") {
    if (!shell_cbs_.set_toolbar_state) { callback->Success("ok"); return true; }
    const std::string tab_id = JsonGet(p, "tabId");
    if (tab_id.empty()) {
      callback->Failure(400, "missing tabId");
      return true;
    }
    // Forward the entire "state" sub-object as raw JSON; the C++ side
    // re-extracts kind + per-kind fields via JsonGet (or the behavior's
    // own parser later).
    size_t spos = p.find("\"state\"");
    std::string state_json;
    if (spos != std::string::npos) {
      size_t obj = p.find('{', spos);
      if (obj != std::string::npos) {
        // Find matching closing brace (depth-tracked).
        int depth = 0;
        size_t end = obj;
        for (; end < p.size(); ++end) {
          if (p[end] == '{') ++depth;
          else if (p[end] == '}') { if (--depth == 0) { ++end; break; } }
        }
        state_json = p.substr(obj, end - obj);
      }
    }
    if (state_json.empty()) {
      callback->Failure(400, "missing state");
      return true;
    }
    if (!shell_cbs_.set_toolbar_state(tab_id, state_json)) {
      callback->Failure(409, "tab kind mismatch or unknown tab");
      return true;
    }
    callback->Success("ok");
    return true;
  }

  if (channel == "tab.set_chrome_theme") {
    if (!shell_cbs_.set_chrome_theme) { callback->Success("ok"); return true; }
    const std::string tab_id = JsonGet(p, "tabId");
    if (tab_id.empty()) {
      callback->Failure(400, "missing tabId");
      return true;
    }
    // color may be either a string or null. JsonGet returns empty for null.
    const std::string color = JsonGet(p, "color");
    if (!shell_cbs_.set_chrome_theme(tab_id, color)) {
      callback->Failure(404, "unknown tab");
      return true;
    }
    callback->Success("ok");
    return true;
  }

  callback->Failure(404, "unknown tab channel");
  return true;
}

// ---------------------------------------------------------------------------
// Workspace channels (.cronymax/ layout introspection)
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleWorkspace(std::string_view channel,
                                    std::string_view payload,
                                    CefRefPtr<Callback> callback) {
  (void)payload;
  auto* sp = space_manager_->ActiveSpace();
  if (!sp) { callback->Failure(503, "no active space"); return true; }

  if (channel == "workspace.layout") {
    WorkspaceLayout layout(sp->workspace_root);
    std::string err;
    const bool ensured = layout.EnsureSkeleton(&err);
    const int version = layout.ReadVersion();
    std::string json =
        "{\"root\":" + JsonString(layout.Root().string()) +
        ",\"cronymax_dir\":" + JsonString(layout.CronymaxDir().string()) +
        ",\"flows_dir\":" + JsonString(layout.FlowsDir().string()) +
        ",\"agents_dir\":" + JsonString(layout.AgentsDir().string()) +
        ",\"doc_types_dir\":" + JsonString(layout.DocTypesDir().string()) +
        ",\"conflicts_dir\":" + JsonString(layout.ConflictsDir().string()) +
        ",\"version\":" + std::to_string(version) +
        ",\"layout_version\":" + std::to_string(WorkspaceLayout::kLayoutVersion) +
        ",\"ensured\":" + (ensured ? "true" : "false");
    if (!ensured && !err.empty()) {
      json += ",\"error\":" + JsonString(err);
    }
    json += "}";
    callback->Success(json);
    return true;
  }

  if (channel == "workspace.gitignore_suggestions") {
    const auto missing = GitignoreHelper::MissingEntries(sp->workspace_root);
    std::string json = "{\"missing\":[";
    for (size_t i = 0; i < missing.size(); ++i) {
      if (i) json += ",";
      json += JsonString(missing[i]);
    }
    json += "]}";
    callback->Success(json);
    return true;
  }

  callback->Failure(404, "unknown workspace channel");
  return true;
}

// ---------------------------------------------------------------------------
// Registry channels (Phase A: read-only views over the per-Space registries)
//
//   agent.registry.list  → {agents:[{name, kind, llm}]}
//   agent.registry.load  → {name, kind, llm, system_prompt, memory_namespace,
//                            tools:[...]}  (payload: {name})
//   flow.list            → {flows:[{id, name, agents:[...], edge_count}]}
//   flow.load            → full FlowDefinition view  (payload: {id})
//   doc_type.list        → {doc_types:[{name, display_name, user_defined}]}
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleRegistry(std::string_view channel,
                                   std::string_view payload,
                                   CefRefPtr<Callback> callback) {
  auto* sp = space_manager_->ActiveSpace();
  if (!sp || !sp->agent_registry || !sp->flow_registry ||
      !sp->doc_type_registry) {
    callback->Failure(503, "registries not ready");
    return true;
  }

  // Lightweight payload extraction: payload looks like `{"name":"foo"}` or
  // `{"id":"bar"}`. Avoid pulling in a full JSON parser for these reads.
  auto extract_field = [](std::string_view body,
                          std::string_view key) -> std::string {
    auto pos = body.find("\"" + std::string(key) + "\"");
    if (pos == std::string_view::npos) return {};
    pos = body.find(':', pos);
    if (pos == std::string_view::npos) return {};
    pos = body.find('"', pos);
    if (pos == std::string_view::npos) return {};
    auto end = body.find('"', pos + 1);
    if (end == std::string_view::npos) return {};
    return std::string(body.substr(pos + 1, end - pos - 1));
  };

  if (channel == "agent.registry.list") {
    std::string json = "{\"agents\":[";
    bool first = true;
    for (const auto& name : sp->agent_registry->Names()) {
      const auto* def = sp->agent_registry->Get(name);
      if (!def) continue;
      if (!first) json += ",";
      first = false;
      json += "{\"name\":" + JsonString(name) +
              ",\"kind\":" + JsonString(def->kind()) +
              ",\"llm\":" + JsonString(def->llm()) + "}";
    }
    json += "]}";
    callback->Success(json);
    return true;
  }

  if (channel == "agent.registry.load") {
    auto name = extract_field(payload, "name");
    const auto* def = sp->agent_registry->Get(name);
    if (!def) {
      callback->Failure(404, "agent not found");
      return true;
    }
    std::string json = "{\"name\":" + JsonString(def->name()) +
                       ",\"kind\":" + JsonString(def->kind()) +
                       ",\"llm\":" + JsonString(def->llm()) +
                       ",\"system_prompt\":" + JsonString(def->system_prompt()) +
                       ",\"memory_namespace\":" +
                       JsonString(def->memory_namespace()) + ",\"tools\":[";
    bool first = true;
    for (const auto& t : def->tools()) {
      if (!first) json += ",";
      first = false;
      json += JsonString(t);
    }
    json += "]}";
    callback->Success(json);
    return true;
  }

  // -------------------------------------------------------------------------
  // agent.registry.save  payload {name, kind, llm, system_prompt,
  //                                memory_namespace, tools_csv}
  //   Writes a minimal `<name>.agent.yaml` to <workspace>/.cronymax/agents/
  //   then refreshes the registry. Idempotent — overwrites any existing
  //   file with the same basename.
  // agent.registry.delete payload {name}
  //   Deletes <workspace>/.cronymax/agents/<name>.agent.yaml then refreshes.
  // -------------------------------------------------------------------------
  if (channel == "agent.registry.save") {
    const std::string p(payload);
    const std::string name = JsonUnescape(JsonGet(p, "name"));
    const std::string kind = JsonUnescape(JsonGet(p, "kind"));
    const std::string llm = JsonUnescape(JsonGet(p, "llm"));
    const std::string system_prompt =
        JsonUnescape(JsonGet(p, "system_prompt"));
    const std::string memory_ns =
        JsonUnescape(JsonGet(p, "memory_namespace"));
    const std::string tools_csv = JsonUnescape(JsonGet(p, "tools_csv"));

    // Validate the basename — it becomes a filename, so reject anything
    // that could escape the agents directory or break YAML/file APIs.
    auto valid_name = [](const std::string& s) {
      if (s.empty() || s.size() > 64) return false;
      for (char c : s) {
        const bool ok = (c >= 'a' && c <= 'z') ||
                        (c >= 'A' && c <= 'Z') ||
                        (c >= '0' && c <= '9') || c == '_' || c == '-' ||
                        c == '.';
        if (!ok) return false;
      }
      return s.front() != '.' && s.find("..") == std::string::npos;
    };
    if (!valid_name(name)) {
      callback->Failure(400, "invalid agent name");
      return true;
    }
    const std::string normalized_kind =
        (kind == "reviewer") ? "reviewer" : "worker";

    WorkspaceLayout layout(sp->workspace_root);
    std::error_code ec;
    std::filesystem::create_directories(layout.AgentsDir(), ec);
    if (ec) {
      callback->Failure(500, "create agents dir failed: " + ec.message());
      return true;
    }
    const auto target = layout.AgentsDir() / (name + ".agent.yaml");

    // Build the YAML by hand. Quote scalars with single quotes (escape any
    // embedded single quote by doubling it). System prompt uses a literal
    // block scalar so newlines round-trip cleanly.
    auto sq = [](const std::string& s) {
      std::string out;
      out.reserve(s.size() + 2);
      out += '\'';
      for (char c : s) {
        if (c == '\'') out += "''";
        else out += c;
      }
      out += '\'';
      return out;
    };
    auto block = [](const std::string& s) {
      std::string out = "|\n";
      // Indent every line by two spaces; preserve empty lines.
      const std::string indent = "  ";
      bool at_line_start = true;
      for (char c : s) {
        if (at_line_start) {
          out += indent;
          at_line_start = false;
        }
        if (c == '\n') {
          out += '\n';
          at_line_start = true;
        } else {
          out += c;
        }
      }
      if (!out.empty() && out.back() != '\n') out += '\n';
      return out;
    };

    std::string yaml;
    yaml += "name: " + sq(name) + "\n";
    yaml += "kind: " + sq(normalized_kind) + "\n";
    yaml += "llm: " + sq(llm) + "\n";
    if (!memory_ns.empty()) {
      yaml += "memory_namespace: " + sq(memory_ns) + "\n";
    }
    yaml += "system_prompt: " + block(system_prompt);
    if (!tools_csv.empty()) {
      yaml += "tools:\n";
      std::string cur;
      auto flush_tool = [&]() {
        // Trim whitespace.
        size_t a = cur.find_first_not_of(" \t");
        size_t b = cur.find_last_not_of(" \t");
        if (a == std::string::npos) {
          cur.clear();
          return;
        }
        std::string tool = cur.substr(a, b - a + 1);
        if (!tool.empty()) yaml += "  - " + sq(tool) + "\n";
        cur.clear();
      };
      for (char c : tools_csv) {
        if (c == ',') flush_tool();
        else cur.push_back(c);
      }
      flush_tool();
    }

    std::ofstream out(target, std::ios::binary | std::ios::trunc);
    if (!out) {
      callback->Failure(500,
                        "open for write failed: " + target.string());
      return true;
    }
    out.write(yaml.data(), static_cast<std::streamsize>(yaml.size()));
    out.close();
    if (!out) {
      callback->Failure(500, "write failed: " + target.string());
      return true;
    }

    sp->agent_registry->Refresh();
    callback->Success("{\"ok\":true}");
    return true;
  }

  if (channel == "agent.registry.delete") {
    const std::string p(payload);
    const std::string name = JsonUnescape(JsonGet(p, "name"));
    auto valid_name = [](const std::string& s) {
      if (s.empty() || s.size() > 64) return false;
      for (char c : s) {
        const bool ok = (c >= 'a' && c <= 'z') ||
                        (c >= 'A' && c <= 'Z') ||
                        (c >= '0' && c <= '9') || c == '_' || c == '-' ||
                        c == '.';
        if (!ok) return false;
      }
      return s.front() != '.' && s.find("..") == std::string::npos;
    };
    if (!valid_name(name)) {
      callback->Failure(400, "invalid agent name");
      return true;
    }
    WorkspaceLayout layout(sp->workspace_root);
    const auto target = layout.AgentsDir() / (name + ".agent.yaml");
    std::error_code ec;
    if (!std::filesystem::remove(target, ec) || ec) {
      callback->Failure(404, "agent file not found: " + target.string());
      return true;
    }
    sp->agent_registry->Refresh();
    callback->Success("{\"ok\":true}");
    return true;
  }

  // -------------------------------------------------------------------------
  // space.profile.get  → current Space's persisted sandbox-rule overrides.
  // space.profile.set  payload {allow_network, extra_read_paths_nl,
  //                             extra_write_paths_nl, extra_deny_paths_nl}
  //   Newline-separated path lists; empty entries dropped. Persisted to
  //   <workspace>/.cronymax/space.profile.yaml. Stored as user intent —
  //   FileBroker enforcement plumbing is wired separately.
  // -------------------------------------------------------------------------
  if (channel == "space.profile.get" || channel == "space.profile.set") {
    WorkspaceLayout layout(sp->workspace_root);
    const auto profile_path = layout.CronymaxDir() / "space.profile.yaml";

    auto json_escape_path = [](const std::string& s) {
      std::string out;
      out.reserve(s.size() + 2);
      out += '"';
      for (char c : s) {
        if (c == '\\' || c == '"') {
          out += '\\';
          out += c;
        } else if (c == '\n') {
          out += "\\n";
        } else if (static_cast<unsigned char>(c) < 0x20) {
          char buf[8];
          std::snprintf(buf, sizeof(buf), "\\u%04x", c);
          out += buf;
        } else {
          out += c;
        }
      }
      out += '"';
      return out;
    };

    // Tiny YAML reader that pulls the four keys we wrote. Tolerates
    // missing file / missing keys — defaults to disabled + empty arrays.
    bool allow_network = false;
    std::vector<std::string> reads, writes, denies;
    {
      std::ifstream in(profile_path);
      if (in) {
        std::string line;
        std::vector<std::string>* current_list = nullptr;
        while (std::getline(in, line)) {
          // Strip trailing CR.
          if (!line.empty() && line.back() == '\r') line.pop_back();
          // List item under a previous key.
          if (current_list && line.size() >= 4 &&
              line.substr(0, 4) == "  - ") {
            std::string v = line.substr(4);
            // Strip surrounding single quotes if present.
            if (v.size() >= 2 && v.front() == '\'' && v.back() == '\'') {
              v = v.substr(1, v.size() - 2);
              std::string unq;
              for (size_t i = 0; i < v.size(); ++i) {
                if (v[i] == '\'' && i + 1 < v.size() && v[i + 1] == '\'') {
                  unq += '\'';
                  ++i;
                } else {
                  unq += v[i];
                }
              }
              v = unq;
            }
            if (!v.empty()) current_list->push_back(v);
            continue;
          }
          current_list = nullptr;
          auto colon = line.find(':');
          if (colon == std::string::npos) continue;
          std::string key = line.substr(0, colon);
          std::string val = line.substr(colon + 1);
          while (!val.empty() && (val.front() == ' ' || val.front() == '\t'))
            val.erase(val.begin());
          if (key == "allow_network") {
            allow_network = (val == "true");
          } else if (key == "extra_read_paths") {
            current_list = &reads;
          } else if (key == "extra_write_paths") {
            current_list = &writes;
          } else if (key == "extra_deny_paths") {
            current_list = &denies;
          }
        }
      }
    }

    if (channel == "space.profile.get") {
      auto emit_arr = [&](const std::vector<std::string>& v) {
        std::string s = "[";
        bool first = true;
        for (const auto& p : v) {
          if (!first) s += ",";
          first = false;
          s += json_escape_path(p);
        }
        s += "]";
        return s;
      };
      std::string json = "{";
      json += "\"space_id\":" + JsonString(sp->id) + ",";
      json += "\"space_name\":" + JsonString(sp->name) + ",";
      json += "\"workspace_root\":" + JsonString(sp->workspace_root.string()) +
              ",";
      json += std::string("\"allow_network\":") +
              (allow_network ? "true" : "false") + ",";
      json += "\"extra_read_paths\":" + emit_arr(reads) + ",";
      json += "\"extra_write_paths\":" + emit_arr(writes) + ",";
      json += "\"extra_deny_paths\":" + emit_arr(denies);
      json += "}";
      callback->Success(json);
      return true;
    }

    // space.profile.set: parse payload and write YAML.
    const std::string p(payload);
    auto get_bool = [&](const char* k) {
      const std::string needle = std::string("\"") + k + "\":";
      auto pos = p.find(needle);
      if (pos == std::string::npos) return false;
      pos += needle.size();
      while (pos < p.size() && (p[pos] == ' ' || p[pos] == '\t')) ++pos;
      return p.compare(pos, 4, "true") == 0;
    };
    const bool new_allow_network = get_bool("allow_network");
    const std::string reads_nl =
        JsonUnescape(JsonGet(p, "extra_read_paths_nl"));
    const std::string writes_nl =
        JsonUnescape(JsonGet(p, "extra_write_paths_nl"));
    const std::string denies_nl =
        JsonUnescape(JsonGet(p, "extra_deny_paths_nl"));

    auto split_lines = [](const std::string& s) {
      std::vector<std::string> out;
      std::string cur;
      for (char c : s) {
        if (c == '\n') {
          // Trim.
          size_t a = cur.find_first_not_of(" \t");
          size_t b = cur.find_last_not_of(" \t");
          if (a != std::string::npos)
            out.push_back(cur.substr(a, b - a + 1));
          cur.clear();
        } else if (c != '\r') {
          cur.push_back(c);
        }
      }
      size_t a = cur.find_first_not_of(" \t");
      size_t b = cur.find_last_not_of(" \t");
      if (a != std::string::npos) out.push_back(cur.substr(a, b - a + 1));
      return out;
    };
    auto sq = [](const std::string& s) {
      std::string out;
      out.reserve(s.size() + 2);
      out += '\'';
      for (char c : s) {
        if (c == '\'') out += "''";
        else out += c;
      }
      out += '\'';
      return out;
    };

    const auto new_reads = split_lines(reads_nl);
    const auto new_writes = split_lines(writes_nl);
    const auto new_denies = split_lines(denies_nl);

    std::error_code ec;
    std::filesystem::create_directories(layout.CronymaxDir(), ec);
    if (ec) {
      callback->Failure(500, "create cronymax dir failed: " + ec.message());
      return true;
    }

    std::string yaml;
    yaml += "# Per-Space sandbox-rule overrides. Edited via Config →\n";
    yaml += "# Workspace tab. Path lists supplement the default sandbox\n";
    yaml += "# allow/deny set; allow_network gates outbound traffic.\n";
    yaml += std::string("allow_network: ") +
            (new_allow_network ? "true" : "false") + "\n";
    auto emit_list = [&](const char* key,
                         const std::vector<std::string>& v) {
      yaml += std::string(key) + ":\n";
      for (const auto& path : v) yaml += "  - " + sq(path) + "\n";
    };
    emit_list("extra_read_paths", new_reads);
    emit_list("extra_write_paths", new_writes);
    emit_list("extra_deny_paths", new_denies);

    std::ofstream out(profile_path, std::ios::binary | std::ios::trunc);
    if (!out) {
      callback->Failure(500, "open for write failed: " + profile_path.string());
      return true;
    }
    out.write(yaml.data(), static_cast<std::streamsize>(yaml.size()));
    out.close();
    if (!out) {
      callback->Failure(500, "write failed: " + profile_path.string());
      return true;
    }
    callback->Success("{\"ok\":true}");
    return true;
  }

  if (channel == "flow.list") {
    std::string json = "{\"flows\":[";
    bool first = true;
    for (const auto& id : sp->flow_registry->Ids()) {
      const auto* f = sp->flow_registry->Get(id);
      if (!f) continue;
      if (!first) json += ",";
      first = false;
      json += "{\"id\":" + JsonString(id) +
              ",\"name\":" + JsonString(f->name()) +
              ",\"edge_count\":" + std::to_string(f->edges().size()) +
              ",\"agents\":[";
      bool ai = true;
      for (const auto& a : f->agents()) {
        if (!ai) json += ",";
        ai = false;
        json += JsonString(a);
      }
      json += "]}";
    }
    json += "]}";
    callback->Success(json);
    return true;
  }

  if (channel == "flow.load") {
    auto id = extract_field(payload, "id");
    const auto* f = sp->flow_registry->Get(id);
    if (!f) {
      callback->Failure(404, "flow not found");
      return true;
    }
    std::string json = "{\"id\":" + JsonString(id) +
                       ",\"name\":" + JsonString(f->name()) +
                       ",\"description\":" + JsonString(f->description()) +
                       ",\"max_review_rounds\":" +
                       std::to_string(f->max_review_rounds()) +
                       ",\"on_review_exhausted\":" +
                       JsonString(f->on_review_exhausted()) +
                       ",\"reviewer_enabled\":" +
                       (f->reviewer_enabled() ? "true" : "false") +
                       ",\"reviewer_timeout_secs\":" +
                       std::to_string(f->reviewer_timeout_secs()) +
                       ",\"agents\":[";
    bool ai = true;
    for (const auto& a : f->agents()) {
      if (!ai) json += ",";
      ai = false;
      json += JsonString(a);
    }
    json += "],\"edges\":[";
    bool ei = true;
    for (const auto& e : f->edges()) {
      if (!ei) json += ",";
      ei = false;
      json += "{\"from\":" + JsonString(e.from_agent) +
              ",\"to\":" + JsonString(e.to_agent) +
              ",\"port\":" + JsonString(e.port) +
              ",\"requires_human_approval\":" +
              (e.requires_human_approval ? "true" : "false") + "}";
    }
    json += "]}";
    callback->Success(json);
    return true;
  }

  if (channel == "doc_type.list") {
    std::string json = "{\"doc_types\":[";
    bool first = true;
    for (const auto& name : sp->doc_type_registry->Names()) {
      const auto* s = sp->doc_type_registry->Get(name);
      if (!s) continue;
      if (!first) json += ",";
      first = false;
      json +=
          "{\"name\":" + JsonString(name) +
          ",\"display_name\":" + JsonString(s->display_name()) +
          ",\"user_defined\":" +
          (sp->doc_type_registry->IsUserDefined(name) ? "true" : "false") +
          "}";
    }
    json += "]}";
    callback->Success(json);
    return true;
  }

  // -------------------------------------------------------------------------
  // FlowRuntime channels (Phase B Group 8). Owned by Space->flow_runtime.
  //   flow.run.start   payload {flow_id, initial_input?} \u2192 {run_id}
  //   flow.run.cancel  payload {run_id}                  \u2192 {ok:true}
  //   flow.run.status  payload {run_id}                  \u2192 FlowRunState JSON
  //   flow.run.list    no payload                        \u2192 {runs:[FlowRunState]}
  // Emits broadcast event `flow.run.changed` on every state transition.
  // -------------------------------------------------------------------------
  if (channel.rfind("flow.run.", 0) == 0) {
    if (!sp->flow_runtime) {
      callback->Failure(503, "flow runtime not ready");
      return true;
    }
    // Wire emitter once (idempotent: SetEventEmitter overwrites).
    sp->flow_runtime->SetEventEmitter(
        [this](const std::string& evt, const std::string& json) {
          shell_cbs_.broadcast_event(evt, json);
        });

    if (channel == "flow.run.start") {
      auto flow_id = extract_field(payload, "flow_id");
      if (flow_id.empty()) {
        callback->Failure(400, "flow_id required");
        return true;
      }
      auto initial_input = extract_field(payload, "initial_input");
      std::string err;
      const std::string run_id =
          sp->flow_runtime->StartRun(flow_id, initial_input, &err);
      if (run_id.empty()) {
        callback->Failure(400, err);
        return true;
      }
      // Persist active-run pointer for this Space (task 8.8).
      space_manager_->store().SetKv(
          "space:" + sp->id + ":active_run",
          flow_id + ":" + run_id);
      callback->Success("{\"run_id\":" + JsonString(run_id) + "}");
      return true;
    }
    if (channel == "flow.run.cancel") {
      auto run_id = extract_field(payload, "run_id");
      if (run_id.empty()) {
        callback->Failure(400, "run_id required");
        return true;
      }
      std::string err;
      if (!sp->flow_runtime->CancelRun(run_id, &err)) {
        callback->Failure(400, err);
        return true;
      }
      callback->Success("{\"ok\":true}");
      return true;
    }
    if (channel == "flow.run.status") {
      auto run_id = extract_field(payload, "run_id");
      auto state = sp->flow_runtime->GetRun(run_id);
      if (!state) {
        callback->Failure(404, "run not found");
        return true;
      }
      callback->Success(state->ToJson());
      return true;
    }
    if (channel == "flow.run.list") {
      std::string json = "{\"runs\":[";
      bool first = true;
      for (const auto& s : sp->flow_runtime->ListRuns()) {
        if (!first) json += ",";
        first = false;
        json += s->ToJson();
      }
      json += "]}";
      callback->Success(json);
      return true;
    }
  }

  // -------------------------------------------------------------------------
  // Trace event subscription. `event.subscribe` payload {run_id} attaches
  // the renderer to a Run's trace stream with replay-then-live semantics:
  // every line already in trace.jsonl is delivered before any live event,
  // and ordering across replay/live is preserved by the TraceWriter lock.
  // Each delivered event is broadcast on `flow.event` (the renderer
  // listens on a single global channel and filters by run_id locally).
  // -------------------------------------------------------------------------
  if (channel == "event.subscribe") {
    // DEPRECATED + DISABLED: the legacy TraceWriter→flow.event pipeline
    // was retired by the agent-event-bus change. New code must call
    // `events.subscribe` and listen on the `event` broadcast channel.
    static std::once_flag s_once;
    std::call_once(s_once, []() {
      fprintf(stderr,
              "[bridge_handler] deprecated channel 'event.subscribe' used; "
              "migrate to events.subscribe (returning 410)\n");
    });
    callback->Failure(410, "event.subscribe is deprecated; use events.subscribe");
    return true;
  }

  // -------------------------------------------------------------------------
  // mention.user_input — server-side @mention parser. Renderer sends the raw
  // user-typed text and the active flow id; we return the matched agent
  // names (and any unknown mentions) so the renderer can dispatch the
  // message to those agents. This keeps mention-parsing rules in one place
  // (MentionParser) and out of JS.
  //   payload: {flow_id, text}
  //   reply:   {mentions:[name], unknown:[name]}
  // -------------------------------------------------------------------------
  if (channel == "mention.user_input") {
    auto flow_id = extract_field(payload, "flow_id");
    auto text = extract_field(payload, "text");
    if (flow_id.empty()) {
      callback->Failure(400, "flow_id required");
      return true;
    }
    if (!sp->flow_registry) {
      callback->Failure(503, "flow registry not ready");
      return true;
    }
    const auto* flow = sp->flow_registry->Get(flow_id);
    if (!flow) {
      callback->Failure(404, "flow not found");
      return true;
    }
    // Build the set of agent names declared by the flow.
    std::set<std::string> known;
    for (const auto& a : flow->agents()) known.insert(a);
    auto parsed = MentionParser::Parse(text);
    std::string json = "{\"mentions\":[";
    bool first = true;
    std::vector<std::string> unknown;
    for (const auto& m : parsed) {
      if (known.count(m.name)) {
        if (!first) json += ",";
        first = false;
        json += JsonString(m.name);
      } else {
        unknown.push_back(m.name);
      }
    }
    json += "],\"unknown\":[";
    first = true;
    for (const auto& u : unknown) {
      if (!first) json += ",";
      first = false;
      json += JsonString(u);
    }
    json += "]}";
    callback->Success(json);
    return true;
  }

  callback->Failure(404, "unknown registry channel");
  return true;
}

// ---------------------------------------------------------------------------
// Document channels (Phase B): read/list current and historical revisions.
//
//   document.list      payload {flow}            \u2192 {docs:[{name, latest_revision}]}
//   document.read      payload {flow, name, [revision]} \u2192 {content, revision}
//   document.subscribe payload {flow}            \u2192 {ok:true} (acks subscribe;
//                                                  push notifications come
//                                                  via "document.changed"
//                                                  broadcast events emitted
//                                                  by the FlowRuntime in a
//                                                  later change set)
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleDocument(std::string_view channel,
                                   std::string_view payload,
                                   CefRefPtr<Callback> callback) {
  auto* sp = space_manager_->ActiveSpace();
  if (!sp) { callback->Failure(503, "no active space"); return true; }

  // Same lightweight extractor used by HandleRegistry. Keep it private to
  // each handler so we don't grow a JSON dependency for trivial reads.
  auto extract = [](std::string_view body, std::string_view key) -> std::string {
    auto pos = body.find("\"" + std::string(key) + "\"");
    if (pos == std::string_view::npos) return {};
    pos = body.find(':', pos);
    if (pos == std::string_view::npos) return {};
    pos = body.find('"', pos);
    if (pos == std::string_view::npos) return {};
    auto end = body.find('"', pos + 1);
    if (end == std::string_view::npos) return {};
    return std::string(body.substr(pos + 1, end - pos - 1));
  };

  const std::string flow_id = extract(payload, "flow");
  if (flow_id.empty()) {
    callback->Failure(400, "missing 'flow' in payload");
    return true;
  }
  // FlowRegistry is the source of truth for valid flow ids.
  if (!sp->flow_registry || !sp->flow_registry->Get(flow_id)) {
    callback->Failure(404, "unknown flow");
    return true;
  }

  WorkspaceLayout layout(sp->workspace_root);
  DocumentStore store(layout.FlowDir(flow_id));

  if (channel == "document.list") {
    const auto items = store.List();
    std::string json = "{\"docs\":[";
    bool first = true;
    for (const auto& d : items) {
      if (!first) json += ",";
      first = false;
      json += "{\"name\":" + JsonString(d.name) +
              ",\"latest_revision\":" + std::to_string(d.latest_revision) +
              ",\"size_bytes\":" + std::to_string(d.size_bytes) + "}";
    }
    json += "]}";
    callback->Success(json);
    return true;
  }

  if (channel == "document.read") {
    const std::string name = extract(payload, "name");
    if (name.empty()) {
      callback->Failure(400, "missing 'name' in payload");
      return true;
    }
    const std::string rev_str = extract(payload, "revision");
    std::string err;
    std::optional<std::string> content;
    int revision = 0;
    if (rev_str.empty()) {
      content = store.Read(name, &err);
      revision = store.LatestRevision(name);
    } else {
      // Exceptions are disabled in this build; manually validate digits.
      if (rev_str.find_first_not_of("0123456789") != std::string::npos) {
        callback->Failure(400, "bad 'revision' value");
        return true;
      }
      revision = std::atoi(rev_str.c_str());
      if (revision < 1) {
        callback->Failure(400, "bad 'revision' value");
        return true;
      }
      content = store.ReadRevision(name, revision, &err);
    }
    if (!content) {
      callback->Failure(404, err.empty() ? "document not found" : err);
      return true;
    }
    std::string json = "{\"revision\":" + std::to_string(revision) +
                       ",\"content\":" + JsonString(*content) + "}";
    callback->Success(json);
    return true;
  }

  if (channel == "document.subscribe") {
    // Subscription is implicit: the renderer listens for the
    // "document.changed" broadcast event. This call exists so the
    // renderer can confirm the channel/flow are valid before installing
    // its event listener, and so future per-flow watchers know which
    // flows have active subscribers.
    callback->Success("{\"ok\":true,\"event\":\"document.changed\"}");
    return true;
  }

  // document.submit { flow, name, content }
  //
  // User-initiated revision write from the workbench (WYSIWYG / Source
  // / Diff "save" paths). Mirrors the agent submit pathway in
  // FlowRuntime: writes a new revision via DocumentStore::Submit,
  // emits a `document_event` AppEvent so chat panels refresh, and
  // broadcasts `document.changed` so other workbench instances reload.
  if (channel == "document.submit") {
    const std::string name = extract(payload, "name");
    const std::string content = extract(payload, "content");
    if (name.empty()) {
      callback->Failure(400, "missing 'name' in payload");
      return true;
    }
    // `content` may legitimately be empty (e.g. user cleared the file),
    // so we do not reject empty content here.
    std::string err;
    auto wr = store.Submit(name, content,
                           std::chrono::milliseconds(2000), &err);
    if (wr.revision == 0) {
      callback->Failure(500, err.empty() ? "submit failed" : err);
      return true;
    }
    if (sp->event_bus) {
      event_bus::AppEvent e;
      e.kind = event_bus::AppEventKind::kDocumentEvent;
      e.space_id = sp->id;
      e.flow_id = flow_id;
      e.agent_id = "user";
      auto payload_obj = JsonValue::Object();
      payload_obj.as_object()["doc_id"] = JsonValue::String(name);
      payload_obj.as_object()["doc_path"] = JsonValue::String(name + ".md");
      payload_obj.as_object()["revision"] = JsonValue::Number(wr.revision);
      payload_obj.as_object()["sha256"] = JsonValue::String(wr.sha256_hex);
      payload_obj.as_object()["producer"] = JsonValue::String("user");
      payload_obj.as_object()["source"] = JsonValue::String("workbench_save");
      e.payload = std::move(payload_obj);
      sp->event_bus->Append(std::move(e));
    }
    if (shell_cbs_.broadcast_event) {
      std::string evt = "{\"flow\":" + JsonString(flow_id) +
                        ",\"name\":" + JsonString(name) +
                        ",\"revision\":" + std::to_string(wr.revision) + "}";
      shell_cbs_.broadcast_event("document.changed", evt);
    }
    std::string out_json = "{\"ok\":true,\"revision\":" +
                           std::to_string(wr.revision) +
                           ",\"sha\":" + JsonString(wr.sha256_hex) + "}";
    callback->Success(out_json);
    return true;
  }

  // document.suggestion.apply { flow, run_id, name, comment_id }
  //
  // Looks up the comment in `runs/<run_id>/reviews.json`, validates that
  // it carries both a non-empty `block_id` and `suggestion`, finds the
  // matching `<!-- block: <uuid> -->` marker in the current revision,
  // replaces the block's body with the suggestion text, and submits a
  // new revision via DocumentStore. On success the comment's
  // `resolved_in_rev` is updated. Stale-revision detection: the comment
  // is rejected if its anchor's revision is older than `current_revision`.
  if (channel == "document.suggestion.apply") {
    const std::string run_id = extract(payload, "run_id");
    const std::string name = extract(payload, "name");
    const std::string comment_id = extract(payload, "comment_id");
    if (run_id.empty() || name.empty() || comment_id.empty()) {
      callback->Failure(400, "missing 'run_id' / 'name' / 'comment_id'");
      return true;
    }
    for (char c : run_id) {
      if (!(std::isalnum(static_cast<unsigned char>(c)) ||
            c == '-' || c == '_')) {
        callback->Failure(400, "bad 'run_id' value");
        return true;
      }
    }
    auto run_dir = layout.FlowDir(flow_id) / "runs" / run_id;
    ReviewStore reviews(run_dir);
    ReviewsState rstate;
    std::string err;
    if (!reviews.Load(&rstate, &err)) {
      callback->Failure(500, err.empty() ? "load reviews failed" : err);
      return true;
    }
    auto doc_it = rstate.docs.find(name);
    if (doc_it == rstate.docs.end()) {
      callback->Failure(404, "document not in reviews.json");
      return true;
    }
    const DocComment* found = nullptr;
    for (const auto& c : doc_it->second.comments) {
      if (c.id == comment_id) { found = &c; break; }
    }
    if (!found) {
      callback->Failure(404, "comment not found");
      return true;
    }
    if (found->block_id.empty()) {
      callback->Failure(400, "comment_not_block_anchored");
      return true;
    }
    if (found->suggestion.empty()) {
      callback->Failure(400, "comment_has_no_suggestion");
      return true;
    }
    const std::string block_id = found->block_id;
    const std::string suggestion = found->suggestion;

    // Read the current document revision and locate the block marker.
    auto current = store.Read(name, &err);
    if (!current) {
      callback->Failure(404, err.empty() ? "document not found" : err);
      return true;
    }
    const int latest = store.LatestRevision(name);
    // Stale-revision guard: if the comment was created against an older
    // revision (its anchor encodes the rev at creation time as
    // "block=<uuid>"; the older form "rev=N ..." is the legacy fallback)
    // and the document has moved on with edits to the same block, we
    // can't safely apply blindly. We detect "stale" only via the legacy
    // anchor; pure block-id anchors are considered stable across edits
    // to other blocks.
    if (!found->legacy_anchor.empty()) {
      // Best-effort parse of "rev=<n> ..." from the legacy anchor.
      auto pos = found->legacy_anchor.find("rev=");
      if (pos != std::string::npos) {
        int legacy_rev = std::atoi(found->legacy_anchor.c_str() + pos + 4);
        if (legacy_rev > 0 && legacy_rev < latest) {
          callback->Failure(409, "stale_revision");
          return true;
        }
      }
    }

    // Locate `<!-- block: <uuid> -->` in current content. Same shape as
    // `web/src/workbench/blockIds.ts`.
    const std::string& md = *current;
    std::vector<std::string> lines;
    {
      std::string cur;
      for (char ch : md) {
        if (ch == '\n') { lines.push_back(std::move(cur)); cur.clear(); }
        else cur.push_back(ch);
      }
      lines.push_back(std::move(cur));
    }
    auto match_marker = [](const std::string& line) -> std::string {
      size_t p = 0;
      while (p < line.size() && (line[p] == ' ' || line[p] == '\t')) ++p;
      if (line.compare(p, 4, "<!--") != 0) return {};
      p += 4;
      while (p < line.size() && (line[p] == ' ' || line[p] == '\t')) ++p;
      if (line.compare(p, 6, "block:") != 0) return {};
      p += 6;
      while (p < line.size() && (line[p] == ' ' || line[p] == '\t')) ++p;
      size_t start = p;
      while (p < line.size() &&
             ((line[p] >= '0' && line[p] <= '9') ||
              (line[p] >= 'a' && line[p] <= 'f') ||
              (line[p] >= 'A' && line[p] <= 'F') || line[p] == '-')) {
        ++p;
      }
      if (p - start < 8) return {};
      return line.substr(start, p - start);
    };
    int marker_idx = -1;
    for (size_t i = 0; i < lines.size(); ++i) {
      if (match_marker(lines[i]) == block_id) {
        marker_idx = static_cast<int>(i);
        break;
      }
    }
    if (marker_idx < 0) {
      callback->Failure(409, "block_not_found_in_current_revision");
      return true;
    }
    int block_end = static_cast<int>(lines.size());
    for (size_t i = static_cast<size_t>(marker_idx) + 1; i < lines.size(); ++i) {
      if (!match_marker(lines[i]).empty()) {
        block_end = static_cast<int>(i);
        break;
      }
    }
    // Build the new content: keep lines [0..marker_idx], then suggestion
    // (trimmed of trailing newlines, plus a single blank separator), then
    // lines [block_end..end].
    std::string trimmed_suggestion = suggestion;
    while (!trimmed_suggestion.empty() &&
           trimmed_suggestion.back() == '\n') {
      trimmed_suggestion.pop_back();
    }
    std::string new_content;
    for (int i = 0; i <= marker_idx; ++i) {
      new_content += lines[static_cast<size_t>(i)];
      new_content += '\n';
    }
    new_content += trimmed_suggestion;
    new_content += "\n\n";
    for (size_t i = static_cast<size_t>(block_end); i < lines.size(); ++i) {
      new_content += lines[i];
      if (i + 1 < lines.size()) new_content += '\n';
    }

    // Write the new revision.
    auto wr = store.Submit(name, new_content,
                           std::chrono::milliseconds(2000), &err);
    if (wr.revision == 0) {
      callback->Failure(500, err.empty() ? "submit failed" : err);
      return true;
    }

    // Mark the original comment resolved.
    bool ok = reviews.Update(
        [&](ReviewsState& s) {
          auto it = s.docs.find(name);
          if (it == s.docs.end()) return false;
          for (auto& c : it->second.comments) {
            if (c.id == comment_id) {
              c.resolved_in_rev = wr.revision;
              break;
            }
          }
          it->second.current_revision = wr.revision;
          return true;
        },
        std::chrono::milliseconds(2000), &err);
    if (!ok) {
      // The new revision is on disk but the comment wasn't marked
      // resolved. Surface this as a soft 200 with a warning so the UI
      // can still refresh; the user can manually mark the comment.
      fprintf(stderr,
              "[bridge_handler] document.suggestion.apply: revision %d "
              "written but resolved_in_rev update failed: %s\n",
              wr.revision, err.c_str());
    }

    // Emit a `document_event` AppEvent so the channel view picks up the
    // new revision without polling. We synthesize it directly here
    // (FlowRuntime would normally do this when an agent submits; this
    // is the human-acted-on-suggestion equivalent).
    if (sp->event_bus) {
      event_bus::AppEvent e;
      e.kind = event_bus::AppEventKind::kDocumentEvent;
      e.space_id = sp->id;
      e.flow_id = flow_id;
      e.run_id = run_id;
      e.agent_id = "user";
      // doc payload mirrors the FlowRuntime emission shape.
      auto payload_obj = JsonValue::Object();
      payload_obj.as_object()["doc_id"] = JsonValue::String(name);
      payload_obj.as_object()["doc_path"] = JsonValue::String(name + ".md");
      payload_obj.as_object()["revision"] = JsonValue::Number(wr.revision);
      payload_obj.as_object()["sha256"] = JsonValue::String(wr.sha256_hex);
      payload_obj.as_object()["producer"] = JsonValue::String("user");
      payload_obj.as_object()["source"] = JsonValue::String("suggestion_apply");
      e.payload = std::move(payload_obj);
      // EventBus::Append fills in id + ts_ms for us.
      sp->event_bus->Append(std::move(e));
    }

    std::string out_json = "{\"ok\":true,\"new_revision\":" +
                           std::to_string(wr.revision) +
                           ",\"sha\":" + JsonString(wr.sha256_hex) + "}";
    callback->Success(out_json);
    return true;
  }

  callback->Failure(404, "unknown document channel");
  return true;
}

// ---------------------------------------------------------------------------
// HandleReview: review.list / review.comment / review.approve /
//               review.request_changes
//
// All channels require {flow, run_id} in the payload to locate
// `<workspace>/.cronymax/flows/<flow>/runs/<run_id>/reviews.json`.
// review.comment / approve / request_changes additionally require `name`
// (document name) and `body` (text). Mutations go through ReviewStore's
// atomic flock-protected update so concurrent writes don't lose data.
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleReview(std::string_view channel,
                                 std::string_view payload,
                                 CefRefPtr<Callback> callback) {
  auto* sp = space_manager_->ActiveSpace();
  if (!sp) { callback->Failure(503, "no active space"); return true; }

  auto extract = [](std::string_view body, std::string_view key) -> std::string {
    auto pos = body.find("\"" + std::string(key) + "\"");
    if (pos == std::string_view::npos) return {};
    pos = body.find(':', pos);
    if (pos == std::string_view::npos) return {};
    pos = body.find('"', pos);
    if (pos == std::string_view::npos) return {};
    auto end = body.find('"', pos + 1);
    if (end == std::string_view::npos) return {};
    return std::string(body.substr(pos + 1, end - pos - 1));
  };

  const std::string flow_id = extract(payload, "flow");
  const std::string run_id  = extract(payload, "run_id");
  if (flow_id.empty() || run_id.empty()) {
    callback->Failure(400, "missing 'flow' or 'run_id' in payload");
    return true;
  }
  if (!sp->flow_registry || !sp->flow_registry->Get(flow_id)) {
    callback->Failure(404, "unknown flow");
    return true;
  }
  // Defense in depth on run_id (used as a directory component).
  for (char c : run_id) {
    if (!(std::isalnum(static_cast<unsigned char>(c)) ||
          c == '-' || c == '_')) {
      callback->Failure(400, "bad 'run_id' value");
      return true;
    }
  }

  WorkspaceLayout layout(sp->workspace_root);
  auto run_dir = layout.FlowDir(flow_id) / "runs" / run_id;
  ReviewStore reviews(run_dir);

  if (channel == "review.list") {
    ReviewsState state;
    std::string err;
    if (!reviews.Load(&state, &err)) {
      callback->Failure(500, err.empty() ? "load failed" : err);
      return true;
    }
    callback->Success(state.ToJson());
    return true;
  }

  // Mutating channels require name + body.
  const std::string name = extract(payload, "name");
  if (name.empty()) {
    callback->Failure(400, "missing 'name' in payload");
    return true;
  }
  std::string body = extract(payload, "body");
  // Optional block-anchored fields (added by `change: document-wysiwyg`).
  // When `block_id` is present the comment is anchored to a specific
  // top-level block rather than a line range, and `anchor` is written
  // in the new `block=<uuid>` form. When `suggestion` is present the
  // comment carries a proposed replacement body the user can accept via
  // `document.suggestion.apply`.
  const std::string block_id = extract(payload, "block_id");
  const std::string suggestion = extract(payload, "suggestion");

  std::string kind;
  DocStatus next_status = DocStatus::kInReview;
  if (channel == "review.comment") {
    kind = "comment";
    if (body.empty()) {
      callback->Failure(400, "missing 'body' in payload");
      return true;
    }
  } else if (channel == "review.approve") {
    kind = "approve";
    next_status = DocStatus::kApproved;
    if (body.empty()) body = "approved";
  } else if (channel == "review.request_changes") {
    kind = "changes_requested";
    next_status = DocStatus::kChangesRequested;
    if (body.empty()) body = "changes requested";
  } else {
    callback->Failure(404, "unknown review channel");
    return true;
  }

  std::string err;
  // Generate a stable comment id from millisecond timestamp.
  auto now_ms =
      std::chrono::duration_cast<std::chrono::milliseconds>(
          std::chrono::system_clock::now().time_since_epoch()).count();
  bool ok = reviews.Update(
      [&](ReviewsState& s) {
        auto& doc = s.docs[name];
        DocComment c;
        c.id = "c-" + std::to_string(now_ms);
        c.author = "user";
        c.kind = kind;
        // Block-anchored comments use the new `block=<uuid>` anchor; pure
        // line/revision comments retain the legacy `rev=N` form so that
        // existing renderers and reviewer pipelines keep working.
        if (!block_id.empty()) {
          c.anchor = "block=" + block_id;
          c.block_id = block_id;
        } else {
          c.anchor = "rev=" + std::to_string(doc.current_revision);
        }
        if (!suggestion.empty()) c.suggestion = suggestion;
        c.body = body;
        c.created_at_ms = now_ms;
        doc.comments.push_back(std::move(c));
        if (kind != "comment") doc.status = next_status;
        return true;
      },
      std::chrono::milliseconds(2000), &err);
  if (!ok) {
    callback->Failure(500, err.empty() ? "update failed" : err);
    return true;
  }

  // Notify renderers that a document's review state changed. Future
  // FlowRuntime work will emit richer trace events; this lightweight
  // broadcast is enough for the chat panel to refresh.
  if (shell_cbs_.broadcast_event) {
    std::string evt = "{\"flow\":" + JsonString(flow_id) +
                      ",\"run_id\":" + JsonString(run_id) +
                      ",\"doc\":" + JsonString(name) +
                      ",\"kind\":" + JsonString(kind) + "}";
    shell_cbs_.broadcast_event("review.changed", evt);
  }
  callback->Success("{\"ok\":true}");
  return true;
}

// ===========================================================================
// agent-event-bus channels (events.*, inbox.*, notifications.*)
//
// All three handlers funnel through the same per-Space `event_bus::EventBus`
// owned by `Space::event_bus`. Live events are pushed to renderers as
// "event" broadcasts (see HandleEvents `events.subscribe`); each renderer
// filters by scope locally.
// ===========================================================================

namespace {

// Lightweight JSON-string field extraction used by these handlers (mirrors
// the lambda inside HandleRegistry; duplicated to avoid plumbing it
// through). For numeric fields, callers must parse separately.
std::string ExtractJsonString(std::string_view body, std::string_view key) {
  std::string needle = "\"" + std::string(key) + "\"";
  auto pos = body.find(needle);
  if (pos == std::string_view::npos) return {};
  pos = body.find(':', pos);
  if (pos == std::string_view::npos) return {};
  pos = body.find('"', pos);
  if (pos == std::string_view::npos) return {};
  auto end = body.find('"', pos + 1);
  if (end == std::string_view::npos) return {};
  return std::string(body.substr(pos + 1, end - pos - 1));
}

// Extract a non-negative integer field. Returns -1 when the field is
// missing or unparseable so callers can substitute defaults.
long long ExtractJsonInt(std::string_view body, std::string_view key) {
  std::string needle = "\"" + std::string(key) + "\"";
  auto pos = body.find(needle);
  if (pos == std::string_view::npos) return -1;
  pos = body.find(':', pos);
  if (pos == std::string_view::npos) return -1;
  ++pos;
  while (pos < body.size() && (body[pos] == ' ' || body[pos] == '\t')) ++pos;
  // We accept either bare number or quoted number (renderer convenience).
  if (pos < body.size() && body[pos] == '"') ++pos;
  size_t start = pos;
  while (pos < body.size() && body[pos] >= '0' && body[pos] <= '9') ++pos;
  if (pos == start) return -1;
  std::string n(body.substr(start, pos - start));
  return std::atoll(n.c_str());
}

std::string AppEventToJson(const event_bus::AppEvent& e) { return e.ToJson(); }

std::string InboxRowToJson(const event_bus::InboxRow& r) {
  std::string out =
      "{\"event_id\":" + JsonString(r.event_id) +
      ",\"state\":" + JsonString(event_bus::InboxStateToString(r.state)) +
      ",\"flow_id\":" + JsonString(r.flow_id) +
      ",\"kind\":" + JsonString(r.kind);
  if (r.snooze_until.has_value()) {
    out += ",\"snooze_until\":" + std::to_string(*r.snooze_until);
  }
  out += "}";
  return out;
}

// Track first-use deprecation warnings (one per process per channel).
std::mutex g_deprecation_mu;
std::set<std::string> g_deprecation_seen;

[[maybe_unused]] void LogDeprecationOnce(const std::string& tag) {
  std::lock_guard<std::mutex> g(g_deprecation_mu);
  if (g_deprecation_seen.insert(tag).second) {
    fprintf(stderr,
            "[bridge_handler] deprecated channel '%s' used; migrate to "
            "events.subscribe\n",
            tag.c_str());
  }
}

}  // namespace

bool BridgeHandler::HandleEvents(CefRefPtr<CefBrowser> browser,
                                 std::string_view channel,
                                 std::string_view payload,
                                 CefRefPtr<Callback> callback) {
  CEF_REQUIRE_UI_THREAD();
  auto* sp = space_manager_->ActiveSpace();
  if (!sp || !sp->event_bus) {
    callback->Failure(503, "event bus not ready");
    return true;
  }
  auto* bus = sp->event_bus.get();

  // events.list { flow_id?, run_id?, before_id?, limit? }
  if (channel == "events.list") {
    event_bus::ListQuery q;
    q.scope.flow_id = ExtractJsonString(payload, "flow_id");
    q.scope.run_id = ExtractJsonString(payload, "run_id");
    q.before_id = ExtractJsonString(payload, "before_id");
    long long lim = ExtractJsonInt(payload, "limit");
    if (lim > 0) q.limit = static_cast<int>(lim);
    auto res = bus->List(q);
    std::string out = "{\"events\":[";
    bool first = true;
    for (const auto& e : res.events) {
      if (!first) out += ",";
      first = false;
      out += AppEventToJson(e);
    }
    out += "],\"cursor\":" + JsonString(res.cursor) + "}";
    callback->Success(out);
    return true;
  }

  // events.subscribe { flow_id?, run_id? } — replay-then-live; broadcasts
  // every matching event on the global "event" channel (renderers filter
  // by scope locally). The token is captured in `browser_subs_` so it can
  // be released when the renderer closes.
  if (channel == "events.subscribe") {
    event_bus::Scope scope;
    scope.flow_id = ExtractJsonString(payload, "flow_id");
    scope.run_id = ExtractJsonString(payload, "run_id");
    auto cbs = shell_cbs_;
    auto token = bus->Subscribe(scope, [cbs](const event_bus::AppEvent& e) {
      if (cbs.broadcast_event) cbs.broadcast_event("event", e.ToJson());
    });
    if (browser) {
      const int bid = browser->GetIdentifier();
      std::lock_guard<std::mutex> g(browser_subs_mutex_);
      browser_subs_[bid].push_back([bus, token]() { bus->Unsubscribe(token); });
    }
    callback->Success("{\"ok\":true}");
    return true;
  }

  // events.append — text events only. Full payload is forwarded as-is into
  // AppEvent.payload after schema gating.
  if (channel == "events.append") {
    auto kind_str = ExtractJsonString(payload, "kind");
    if (kind_str != "text") {
      callback->Failure(400, "events.append only accepts kind=text");
      return true;
    }
    event_bus::AppEvent evt;
    evt.kind = event_bus::AppEventKind::kText;
    evt.space_id = sp->id;
    evt.flow_id = ExtractJsonString(payload, "flow_id");
    evt.run_id = ExtractJsonString(payload, "run_id");
    evt.agent_id = ExtractJsonString(payload, "agent_id");
    // Parse payload field as a JSON object, default to {body, mentions:[]}
    // built from the raw text/mentions fields when absent.
    JsonValue p_raw;
    std::string perr;
    bool have_payload = false;
    if (JsonValue::Parse(std::string(payload), &p_raw, &perr) &&
        p_raw.is_object()) {
      const auto& inner = p_raw.Get("payload");
      if (inner.is_object()) {
        evt.payload = inner;
        have_payload = true;
      }
    }
    if (!have_payload) {
      // Fall back to constructing { body, mentions:[] } from top-level fields.
      auto body = ExtractJsonString(payload, "body");
      JsonValue obj = JsonValue::Object();
      obj.as_object()["body"] = JsonValue::String(body);
      obj.as_object()["mentions"] = JsonValue::Array();
      evt.payload = std::move(obj);
    }
    auto id = bus->Append(std::move(evt));
    callback->Success("{\"id\":" + JsonString(id) + "}");
    return true;
  }

  callback->Failure(404, "unknown events.* channel");
  return true;
}

bool BridgeHandler::HandleInbox(std::string_view channel,
                                std::string_view payload,
                                CefRefPtr<Callback> callback) {
  CEF_REQUIRE_UI_THREAD();
  auto* sp = space_manager_->ActiveSpace();
  if (!sp || !sp->event_bus) {
    callback->Failure(503, "event bus not ready");
    return true;
  }
  auto* bus = sp->event_bus.get();

  if (channel == "inbox.list") {
    event_bus::InboxQuery q;
    q.flow_id = ExtractJsonString(payload, "flow_id");
    auto state_str = ExtractJsonString(payload, "state");
    if (!state_str.empty()) {
      event_bus::InboxState s;
      if (event_bus::InboxStateFromString(state_str, &s)) q.state = s;
    }
    long long lim = ExtractJsonInt(payload, "limit");
    if (lim > 0) q.limit = static_cast<int>(lim);
    auto res = bus->ListInbox(q);
    std::string out = "{\"rows\":[";
    bool first = true;
    for (const auto& r : res.rows) {
      if (!first) out += ",";
      first = false;
      out += InboxRowToJson(r);
    }
    out += "],\"unread_count\":" + std::to_string(res.unread_count) +
           ",\"needs_action_count\":" +
           std::to_string(res.needs_action_count) + "}";
    callback->Success(out);
    return true;
  }

  if (channel == "inbox.read" || channel == "inbox.unread" ||
      channel == "inbox.snooze") {
    auto event_id = ExtractJsonString(payload, "event_id");
    if (event_id.empty()) {
      callback->Failure(400, "event_id required");
      return true;
    }
    event_bus::InboxState target = event_bus::InboxState::kRead;
    std::optional<long long> snooze;
    if (channel == "inbox.unread") target = event_bus::InboxState::kUnread;
    if (channel == "inbox.snooze") {
      target = event_bus::InboxState::kSnoozed;
      long long until = ExtractJsonInt(payload, "snooze_until");
      if (until <= 0) {
        callback->Failure(400, "snooze_until required for inbox.snooze");
        return true;
      }
      snooze = until;
    }
    bool ok = bus->SetInboxState(event_id, target, snooze);
    if (!ok) {
      callback->Failure(404, "inbox row not found");
      return true;
    }
    callback->Success("{\"ok\":true}");
    return true;
  }

  callback->Failure(404, "unknown inbox.* channel");
  return true;
}

bool BridgeHandler::HandleNotifications(std::string_view channel,
                                        std::string_view payload,
                                        CefRefPtr<Callback> callback) {
  CEF_REQUIRE_UI_THREAD();
  auto* sp = space_manager_->ActiveSpace();
  if (!sp || !sp->event_bus) {
    callback->Failure(503, "event bus not ready");
    return true;
  }
  auto* bus = sp->event_bus.get();

  if (channel == "notifications.get_prefs") {
    auto kinds = bus->ListEnabledNotificationKinds();
    std::string out = "{\"enabled\":[";
    bool first = true;
    for (const auto& k : kinds) {
      if (!first) out += ",";
      first = false;
      out += JsonString(k);
    }
    out += "]}";
    callback->Success(out);
    return true;
  }

  if (channel == "notifications.set_kind_pref") {
    auto kind = ExtractJsonString(payload, "kind");
    if (kind.empty()) {
      callback->Failure(400, "kind required");
      return true;
    }
    // Accept either `"enabled":true` or `"enabled":false` literal.
    bool enabled = payload.find("\"enabled\":true") != std::string_view::npos;
    bus->SetKindNotificationEnabled(kind, enabled);
    callback->Success("{\"ok\":true}");
    return true;
  }

  callback->Failure(404, "unknown notifications.* channel");
  return true;
}

void BridgeHandler::OnBrowserClosed(int browser_id) {
  std::vector<std::function<void()>> cbs;
  {
    std::lock_guard<std::mutex> g(browser_subs_mutex_);
    auto it = browser_subs_.find(browser_id);
    if (it == browser_subs_.end()) return;
    cbs = std::move(it->second);
    browser_subs_.erase(it);
  }
  for (auto& f : cbs) f();
}

}  // namespace cronymax

