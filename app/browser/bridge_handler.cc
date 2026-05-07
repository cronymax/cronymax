#include "browser/bridge_handler.h"

#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <mutex>
#include <optional>
#include <set>
#include <sstream>
#include <unordered_map>
#include <sys/wait.h>
#include <unistd.h>

#include <nlohmann/json.hpp>

#include "event_bus/event_bus.h"
#include "event_bus/app_event.h"
#include "workspace/flow_yaml.h"
// (task 4.1) flow_runtime.h, trace_event.h, trace_writer.h removed —
// run lifecycle is now owned by the Rust runtime over GIPS.
// (task 5.0) sandbox, flow C++ modules removed — logic now lives in the
// Rust runtime (crates/cronymax).
// (Phase 2) file_broker.h removed — file I/O proxied to Rust FileBroker.
#include "common/types.h"
#include "common/path_utils.h"
#include "workspace/workspace_layout.h"
#include "include/base/cef_callback.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {
namespace {

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

std::pair<std::string, std::string> SplitEnvelope(const std::string& request) {
  // Modern web bridge format: a JSON envelope
  // `{"channel":"...","payload":"<json-string>"}` where payload is itself a
  // JSON-encoded string. Detect this shape and decode.
  if (!request.empty() && request.front() == '{') {
    auto env = nlohmann::json::parse(request, nullptr, false);
    if (!env.is_discarded() && env.is_object()) {
      const std::string channel = env.value("channel", std::string{});
      if (!channel.empty()) {
        std::string payload;
        if (env.contains("payload") && env["payload"].is_string())
          payload = env["payload"].get<std::string>();
        return {channel, payload};
      }
    }
  }
  // Legacy format: "<channel>\n<payload>".
  const auto sep = request.find('\n');
  if (sep == std::string::npos) return {request, ""};
  return {request.substr(0, sep), request.substr(sep + 1)};
}

std::string SpaceToJson(const Space& sp) {
  return nlohmann::json{
      {"id",         sp.id},
      {"name",       sp.name},
      {"root_path",  sp.workspace_root.string()},
      {"profile_id", sp.profile_id},
  }.dump();
}

// Shell execution helper — replaces the removed sandbox::SandboxLauncher.
// Runs `cmd` via /bin/sh -c in `cwd` and captures stdout/stderr.
// No sandbox policy enforcement; the Rust runtime is the authoritative
// capability gate for agent tool calls.
ExecResult RunShellCommand(const std::filesystem::path& cwd,
                           const std::string& cmd) {
  ExecResult result;
  int stdout_pipe[2] = {-1, -1};
  int stderr_pipe[2] = {-1, -1};
  if (pipe(stdout_pipe) != 0 || pipe(stderr_pipe) != 0) {
    result.stderr_data = "failed to create pipes";
    return result;
  }
  const pid_t pid = fork();
  if (pid < 0) {
    result.stderr_data = "failed to fork";
    close(stdout_pipe[0]); close(stdout_pipe[1]);
    close(stderr_pipe[0]); close(stderr_pipe[1]);
    return result;
  }
  if (pid == 0) {
    close(stdout_pipe[0]); close(stderr_pipe[0]);
    dup2(stdout_pipe[1], STDOUT_FILENO); close(stdout_pipe[1]);
    dup2(stderr_pipe[1], STDERR_FILENO); close(stderr_pipe[1]);
    if (!cwd.empty()) chdir(cwd.c_str());
    execl("/bin/sh", "/bin/sh", "-c", cmd.c_str(), nullptr);
    _exit(127);
  }
  close(stdout_pipe[1]); close(stderr_pipe[1]);
  auto read_fd = [](int fd) {
    std::string data;
    char buf[4096];
    ssize_t n;
    while ((n = read(fd, buf, sizeof(buf))) > 0)
      data.append(buf, static_cast<size_t>(n));
    close(fd);
    return data;
  };
  result.stdout_data = read_fd(stdout_pipe[0]);
  result.stderr_data = read_fd(stderr_pipe[0]);
  int status = 0;
  waitpid(pid, &status, 0);
  result.exit_code = WIFEXITED(status) ? WEXITSTATUS(status) : -1;
  return result;
}

// Extract a string field from a JSON payload using nlohmann::json (no-throw).
std::string ExtractJsonString(std::string_view payload, std::string_view key) {
  auto j = nlohmann::json::parse(payload, nullptr, /*allow_exceptions=*/false);
  if (j.is_discarded() || !j.is_object()) return {};
  auto it = j.find(std::string(key));
  if (it == j.end() || !it->is_string()) return {};
  return it->get<std::string>();
}

// Extract an integer field from a JSON payload using nlohmann::json (no-throw).
long long ExtractJsonInt(std::string_view payload, std::string_view key) {
  auto j = nlohmann::json::parse(payload, nullptr, /*allow_exceptions=*/false);
  if (j.is_discarded() || !j.is_object()) return 0;
  auto it = j.find(std::string(key));
  if (it == j.end() || !it->is_number_integer()) return 0;
  return it->get<long long>();
}

// Render an AppEvent as compact JSON for bridge serialisation.
std::string AppEventToJson(const event_bus::AppEvent& e) {
  return e.ToJson();
}

// Render an InboxRow as compact JSON for bridge serialisation.
std::string InboxRowToJson(const event_bus::InboxRow& r) {
  nlohmann::json j = {
      {"event_id", r.event_id},
      {"state",    event_bus::InboxStateToString(r.state)},
      {"flow_id",  r.flow_id},
      {"kind",     r.kind},
  };
  if (r.snooze_until.has_value())
    j["snooze_until"] = *r.snooze_until;
  return j.dump();
}

// Decode a standard base64 string to raw bytes (returned as std::string).
// Returns an empty string on malformed input.
std::string Base64Decode(const std::string& in) {
  static const int8_t kTable[256] = {
    -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
    -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
    -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,62,-1,-1,-1,63,
    52,53,54,55,56,57,58,59,60,61,-1,-1,-1,-1,-1,-1,
    -1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9,10,11,12,13,14,
    15,16,17,18,19,20,21,22,23,24,25,-1,-1,-1,-1,-1,
    -1,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,
    41,42,43,44,45,46,47,48,49,50,51,-1,-1,-1,-1,-1,
    -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
    -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
    -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
    -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
    -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
    -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
    -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
    -1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,
  };
  std::string out;
  out.reserve(in.size() * 3 / 4);
  int bits = 0, n = 0;
  for (unsigned char c : in) {
    int v = kTable[c];
    if (v < 0) continue;  // skip padding and whitespace
    bits = (bits << 6) | v;
    if (++n == 4) {
      out.push_back(static_cast<char>((bits >> 16) & 0xff));
      out.push_back(static_cast<char>((bits >>  8) & 0xff));
      out.push_back(static_cast<char>( bits        & 0xff));
      bits = 0; n = 0;
    }
  }
  if (n == 3) {
    bits <<= 6;
    out.push_back(static_cast<char>((bits >> 16) & 0xff));
    out.push_back(static_cast<char>((bits >>  8) & 0xff));
  } else if (n == 2) {
    bits <<= 12;
    out.push_back(static_cast<char>((bits >> 16) & 0xff));
  }
  return out;
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
  if (channel.rfind("permission.", 0) == 0)
    return HandlePermission(channel, payload, callback);
  if (channel.rfind("llm.config", 0) == 0)
    return HandleLlmConfig(channel, payload, callback);
  if (channel.rfind("llm.providers", 0) == 0)
    return HandleLlmConfig(channel, payload, callback);
  if (channel.rfind("browser.", 0) == 0)
    return HandleBrowser(browser, channel, payload, callback);
  if (channel.rfind("shell.", 0) == 0)
    return HandleShell(browser, channel, payload, callback);
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
  if (channel.rfind("profiles.", 0) == 0)
    return HandleProfiles(browser, channel, payload, callback);

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
  auto j = nlohmann::json::parse(payload, nullptr, false);
  auto resolve_terminal = [&]() -> TerminalSession* {
    const std::string id = j.is_object() ? j.value("id", std::string{}) : std::string{};
    if (!id.empty()) return sp->FindTerminal(id);
    return sp->ActiveTerminal();
  };

  // List terminals for the active Space.
  if (channel == "terminal.list") {
    nlohmann::json items = nlohmann::json::array();
    for (const auto& t : sp->terminals)
      items.push_back({{"id", t->id}, {"name", t->name}});
    callback->Success(nlohmann::json{
        {"active", sp->active_terminal_id}, {"items", items}
    }.dump());
    return true;
  }

  // Create a new terminal session (does NOT auto-start the PTY).
  if (channel == "terminal.new") {
    auto* t = sp->CreateTerminal();
    const std::string item    = nlohmann::json{{"id", t->id}, {"name", t->name}}.dump();
    const std::string switched = nlohmann::json{{"id", t->id}}.dump();
    if (shell_cbs_.broadcast_event) {
      shell_cbs_.broadcast_event("terminal.created", item);
      shell_cbs_.broadcast_event("terminal.switched", switched);
    } else {
      SendEvent(browser, "terminal.created", item);
      SendEvent(browser, "terminal.switched", switched);
    }
    callback->Success(item);
    return true;
  }

  // Switch the active terminal.
  if (channel == "terminal.switch") {
    const std::string id = j.is_object() ? j.value("id", std::string{}) : std::string{};
    if (id.empty() || !sp->FindTerminal(id)) {
      callback->Failure(404, "no such terminal");
      return true;
    }
    sp->active_terminal_id = id;
    const std::string body = nlohmann::json{{"id", id}}.dump();
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
    const std::string id = j.is_object() ? j.value("id", std::string{}) : std::string{};
    if (!sp->CloseTerminal(id)) { callback->Failure(404, "no such terminal"); return true; }
    const std::string removed = nlohmann::json{{"id", id}}.dump();
    if (shell_cbs_.broadcast_event) {
      shell_cbs_.broadcast_event("terminal.removed", removed);
    } else {
      SendEvent(browser, "terminal.removed", removed);
    }
    if (!sp->active_terminal_id.empty()) {
      const std::string sw = nlohmann::json{{"id", sp->active_terminal_id}}.dump();
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
    auto* term = resolve_terminal();
    if (!term) { callback->Failure(404, "no such terminal"); return true; }
    const std::string tid = term->id;
    if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return true; }
    int cols = 100, rows = 30;
    if (j.is_object()) {
      if (j.contains("cols") && j["cols"].is_number()) cols = j["cols"].get<int>();
      if (j.contains("rows") && j["rows"].is_number()) rows = j["rows"].get<int>();
    }
    runtime_proxy_->SendControl({
        {"kind", "terminal_start"},
        {"terminal_id", tid},
        {"workspace_root", sp->workspace_root.string()},
        {"shell", "/bin/zsh"},
        {"cols", cols},
        {"rows", rows},
    }, [this, tid, callback](nlohmann::json resp, bool is_error) {
        if (is_error) {
          callback->Failure(500, resp.value("error", nlohmann::json{})
                                     .value("message", "start failed"));
          return;
        }
        // Subscribe to the terminal's output topic so we can broadcast
        // terminal.output events to the renderer.
        runtime_proxy_->SendControl(
            {{"kind", "subscribe"}, {"topic", "terminal:" + tid}},
            [this, tid](nlohmann::json sub_resp, bool sub_err) {
                if (sub_err) return;
                runtime_proxy_->SubscribeEvents(
                    [this, tid](const nlohmann::json& msg) {
                        // Filter for this terminal's Raw events.
                        const auto& ev = msg.value("event", nlohmann::json::object());
                        const auto& pl = ev.value("payload", nlohmann::json::object());
                        if (pl.value("kind", "") != "raw") return;
                        const auto& d = pl.value("data", nlohmann::json::object());
                        if (d.value("id", "") != tid) return;
                        const std::string b64 = d.value("data", std::string{});
                        const std::string raw = Base64Decode(b64);
                        if (shell_cbs_.broadcast_event) {
                            shell_cbs_.broadcast_event(
                                "terminal.output",
                                nlohmann::json{{"id", tid}, {"data", raw}}.dump());
                        }
                    });
            });
        callback->Success("ok");
    });
    return true;
  }

  if (channel == "terminal.input") {
    auto* term = resolve_terminal();
    if (!term) { callback->Failure(404, "no such terminal"); return true; }
    std::string data = j.is_object() ? j.value("data", std::string{}) : std::string{};
    if (data.empty()) data = std::string(payload);
    if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return true; }
    runtime_proxy_->SendControl({
        {"kind", "terminal_input"},
        {"terminal_id", term->id},
        {"data", data},
    }, [callback](nlohmann::json, bool) { callback->Success("ok"); });
    return true;
  }

  if (channel == "terminal.resize") {
    auto* term = resolve_terminal();
    if (!term) { callback->Failure(404, "no such terminal"); return true; }
    int cols = 100, rows = 30;
    if (j.is_object()) {
      if (j.contains("cols") && j["cols"].is_number()) cols = j["cols"].get<int>();
      if (j.contains("rows") && j["rows"].is_number()) rows = j["rows"].get<int>();
    }
    if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return true; }
    runtime_proxy_->SendControl({
        {"kind", "terminal_resize"},
        {"terminal_id", term->id},
        {"cols", cols},
        {"rows", rows},
    }, [callback](nlohmann::json, bool) { callback->Success("ok"); });
    return true;
  }

  if (channel == "terminal.stop") {
    auto* term = resolve_terminal();
    if (!term) { callback->Failure(404, "no such terminal"); return true; }
    if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return true; }
    runtime_proxy_->SendControl({
        {"kind", "terminal_stop"},
        {"terminal_id", term->id},
    }, [callback](nlohmann::json, bool) { callback->Success("ok"); });
    return true;
  }

  if (channel == "terminal.restart") {
    if (shell_cbs_.terminal_restart) shell_cbs_.terminal_restart();
    callback->Success("ok");
    return true;
  }

  if (channel == "terminal.run") {
    auto* term = resolve_terminal();
    if (!term) { callback->Failure(404, "no such terminal"); return true; }
    const std::string command = j.is_object() ? j.value("command", std::string{}) : std::string{};
    if (command.empty()) { callback->Success("ok"); return true; }
    if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return true; }
    runtime_proxy_->SendControl({
        {"kind", "terminal_input"},
        {"terminal_id", term->id},
        {"data", command + "\n"},
    }, [callback](nlohmann::json, bool) { callback->Success("ok"); });
    return true;
  }

  if (channel == "terminal.block_save") {
    TerminalBlockRow row;
    row.space_id   = j.is_object() ? j.value("space_id", std::string{}) : std::string{};
    if (row.space_id.empty()) row.space_id = sp->id;
    row.command    = j.is_object() ? j.value("command",    std::string{}) : std::string{};
    row.output     = j.is_object() ? j.value("output",     std::string{}) : std::string{};
    if (j.is_object()) {
      if (j.contains("exit_code")  && j["exit_code"].is_number())  row.exit_code  = j["exit_code"].get<int>();
      if (j.contains("started_at") && j["started_at"].is_number()) row.started_at = j["started_at"].get<long long>();
      if (j.contains("ended_at")   && j["ended_at"].is_number())   row.ended_at   = j["ended_at"].get<long long>();
    }
    space_manager_->store().CreateBlock(row);
    callback->Success("ok");
    return true;
  }

  if (channel == "terminal.blocks_load") {
    const std::string sid = j.is_object() ? j.value("space_id", std::string{}) : std::string{};
    const std::string& effective_sid = sid.empty() ? sp->id : sid;
    const auto blocks = space_manager_->store().ListBlocksForSpace(effective_sid);
    nlohmann::json arr = nlohmann::json::array();
    for (const auto& b : blocks) {
      arr.push_back({
          {"id",         b.id},
          {"command",    b.command},
          {"output",     b.output},
          {"exit_code",  b.exit_code},
          {"started_at", b.started_at},
          {"ended_at",   b.ended_at},
      });
    }
    callback->Success(arr.dump());
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
    // MIGRATION (rust-runtime-cpp-cutover, task 3.1): forward to Rust
    // runtime via RuntimeProxy::SendControl(StartRun{...}).
    if (runtime_proxy_) {
      // Read LLM config from the active provider in llm.providers (new-style)
      // with fallback to the old-style individual keys (llm.base_url, llm.api_key).
      const auto llm_cfg = space_manager_->store().GetLlmConfig();
      std::string base_url      = llm_cfg.base_url;
      std::string api_key       = llm_cfg.api_key;
      std::string model         = "gpt-4o-mini";
      std::string provider_kind = "openai_compat";
      const std::string providers_raw =
          space_manager_->store().GetKv("llm.providers");
      const std::string active_id =
          space_manager_->store().GetKv("llm.active_provider_id");
      if (!providers_raw.empty() && !active_id.empty()) {
        auto pj = nlohmann::json::parse(providers_raw, nullptr, false);
        if (!pj.is_discarded() && pj.is_array()) {
          for (const auto& p : pj) {
            if (p.value("id", std::string{}) == active_id) {
              // "base_url" and "api_key" from provider override old-style keys.
              const std::string purl = p.value("base_url", std::string{});
              if (!purl.empty()) base_url = purl;
              // api_key may be JSON null (optional for local providers) —
              // p.value() throws type_error.302 on null, so check is_string first.
              if (const auto it = p.find("api_key"); it != p.end() && it->is_string()) {
                const std::string pkey = it->get<std::string>();
                if (!pkey.empty()) api_key = pkey;
              }
              // Provider stores the model as "default_model".
              const std::string pm = p.value("default_model", std::string{});
              if (!pm.empty()) model = pm;
              // Forward provider kind so the runtime can apply Copilot-specific
              // token exchange and required request headers.
              const std::string pk = p.value("kind", std::string{});
              if (!pk.empty()) provider_kind = pk;
              break;
            }
          }
        }
      }
      if (base_url.empty()) base_url = "https://api.openai.com/v1";
      nlohmann::json req = {
          {"kind", "start_run"},
          {"space_id", sp->id},
          {"payload", {
              {"task", std::string(payload)},
              {"workspace_root", sp->workspace_root.string()},
              {"llm", {
                  {"base_url",      base_url},
                  {"api_key",       api_key},
                  {"model",         model},
                  {"provider_kind", provider_kind}
              }}
          }}
      };
      runtime_proxy_->SendControl(std::move(req),
          [this, browser, callback](nlohmann::json resp, bool is_error) {
            if (is_error) {
              const std::string msg =
                  resp.value("error", nlohmann::json{})
                      .value("message", "runtime error");
              callback->Failure(500, msg);
              return;
            }
            const std::string run_id = resp.value("run_id", std::string{});
            // RunStarted carries a pre-created subscription so we can
            // register our event listener HERE — synchronously, before
            // any ReactLoop events can arrive — avoiding the race where
            // tokens/status events are emitted before events.subscribe
            // completes its own IPC round-trip.
            const std::string sub_id = resp.value("subscription", std::string{});
            // Register cleanup for start_run's Rust subscription so the
            // runtime knows the run is done when the browser closes.
            // We do NOT add a C++ event_subs_ entry here; the space-level
            // subscription from OnSpaceSwitch (Lambda S) fans out all events
            // to all panels — adding a per-run entry causes N-fold duplication
            // after N runs.
            if (!sub_id.empty()) {
              const int bid = browser ? browser->GetIdentifier() : 0;
              std::lock_guard<std::mutex> g(browser_subs_mutex_);
              browser_subs_[bid].push_back([this, sub_id]() {
                if (runtime_proxy_) {
                  nlohmann::json unsub = {
                      {"kind", "unsubscribe"}, {"subscription", sub_id}};
                  runtime_proxy_->SendControl(std::move(unsub),
                                              [](nlohmann::json, bool) {});
                }
              });
            }
            // Return the run_id as a bare JSON string so the frontend
            // receives it directly as a string (bridge_channels: res z.string()).
            callback->Success(nlohmann::json(run_id).dump());
          });
      return true;
    }
    callback->Failure(503, "runtime not available");
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
    nlohmann::json arr = nlohmann::json::array();
    for (const auto& sp : space_manager_->spaces()) {
      arr.push_back({
          {"id",        sp->id},
          {"name",      sp->name},
          {"root_path", sp->workspace_root.string()},
      });
    }
    callback->Success(arr.dump());
    return true;
  }

  if (channel == "space.create") {
    auto j = nlohmann::json::parse(payload, nullptr, false);
    const std::string root       = j.is_object() ? j.value("root_path",  std::string{}) : std::string{};
    const std::string profile_id = j.is_object() ? j.value("profile_id", std::string{"default"}) : std::string{"default"};
    if (root.empty()) {
      callback->Failure(400, "root_path required");
      return true;
    }
    const auto id = space_manager_->CreateSpace(std::filesystem::path(root), profile_id);
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
    callback->Success(nlohmann::json{{"id", id}}.dump());
    return true;
  }

  if (channel == "space.open_folder") {
    // Trigger a native folder picker. On selection the host emits
    // `space.folder_picked` with {path} so the frontend can show the
    // ProfilePickerOverlay before calling space.create.
    if (!shell_cbs_.run_file_dialog) {
      callback->Failure(501, "folder picker not available");
      return true;
    }
    // run_file_dialog is expected to call back with the selected path, or
    // empty string on cancel. It must emit space.folder_picked via broadcast_event.
    shell_cbs_.run_file_dialog(
        [this, browser = browser](const std::string& path) {
          if (path.empty()) return;  // cancelled
          if (shell_cbs_.broadcast_event) {
            const std::string evt =
                nlohmann::json{{"path", path}}.dump();
            shell_cbs_.broadcast_event("space.folder_picked", evt);
          }
        });
    callback->Success("ok");
    return true;
  }

  if (channel == "space.switch") {
    auto j = nlohmann::json::parse(payload, nullptr, false);
    const std::string id = j.is_object() ? j.value("space_id", std::string{}) : std::string{};
    if (!space_manager_->SwitchTo(id)) {
      callback->Failure(404, "space not found");
      return true;
    }
    callback->Success("ok");
    return true;
  }

  if (channel == "space.delete") {
    auto j = nlohmann::json::parse(payload, nullptr, false);
    const std::string id = j.is_object() ? j.value("space_id", std::string{}) : std::string{};
    if (!space_manager_->DeleteSpace(id)) {
      callback->Failure(404, "space not found");
      return true;
    }
    callback->Success("ok");
    SendEvent(browser, "space.deleted", nlohmann::json{{"space_id", id}}.dump());
    return true;
  }

  callback->Failure(404, "unknown space channel");
  return true;
}

// ---------------------------------------------------------------------------
// Permission channels
// ---------------------------------------------------------------------------

bool BridgeHandler::HandlePermission(std::string_view channel,
                                     std::string_view payload,
                                     CefRefPtr<Callback> callback) {
  if (channel == "permission.respond") {
    auto j = nlohmann::json::parse(payload, nullptr, false);
    const std::string rid = j.is_object() ? j.value("request_id", std::string{}) : std::string{};
    const std::string dec = j.is_object() ? j.value("decision",   std::string{}) : std::string{};
    const bool allow = (dec == "allow");

    // (task 3.3) Check for a pending runtime capability reply first.
    // The runtime's user_approval capability calls are stored here with the
    // capability correlation_id as the key (set by SetupCapabilityHandler).
    {
      RuntimeProxy::CapabilityReplyFn reply_fn;
      {
        std::lock_guard<std::mutex> g(cap_reply_mu_);
        auto it = pending_cap_replies_.find(rid);
        if (it != pending_cap_replies_.end()) {
          reply_fn = std::move(it->second);
          pending_cap_replies_.erase(it);
        }
      }
      if (reply_fn) {
        nlohmann::json resp;
        if (allow) {
          resp = {{"outcome", "ok"}};
        } else {
          resp = {{"outcome", "err"}, {"error",
              {{"code", "denied"}, {"message", "user denied permission"}}}};
        }
        reply_fn(std::move(resp));
        callback->Success("{\"ok\":true}");
        return true;
      }
    }

    // Fallback: legacy in-process permission delivery.
    DeliverPermissionResponse(rid, allow);
    callback->Success("{\"ok\":true}");
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

// (task 3.3 + 4.3) Install the capability handler on the RuntimeProxy.
// Handles all capability types that require host participation:
//   user_approval — shows a permission dialog to the user
//   shell         — executes a sandboxed shell command (scope-enforced)
//   filesystem    — reads or writes files (scope-enforced to workspace_root)
//   notify        — posts a native OS notification
//   browser       — 501 (not yet implemented)
//   secret        — 501 (not yet implemented)
void BridgeHandler::SetupCapabilityHandler() {
  if (!runtime_proxy_) return;
  runtime_proxy_->SetCapabilityHandler(
      [this](const std::string& corr_id, const nlohmann::json& request,
             RuntimeProxy::CapabilityReplyFn reply) {
        const std::string cap = request.value("capability", std::string{});
        const std::string space_id = request.value("space_id", std::string{});

        // Resolve the owning Space for scope enforcement.
        // FindSpace is private; iterate the public spaces() list instead.
        Space* sp = nullptr;
        if (space_id.empty()) {
          sp = space_manager_->ActiveSpace();
        } else {
          for (const auto& s : space_manager_->spaces()) {
            if (s->id == space_id) { sp = s.get(); break; }
          }
        }
        const std::filesystem::path workspace_root =
            sp ? sp->workspace_root : std::filesystem::path{};

        // ── user_approval ────────────────────────────────────────────────
        if (cap == "user_approval") {
          {
            std::lock_guard<std::mutex> g(cap_reply_mu_);
            pending_cap_replies_[corr_id] = std::move(reply);
          }
          if (shell_cbs_.broadcast_event) {
            nlohmann::json evt = {
                {"request_id", corr_id},
                {"run_id",     request.value("run_id",    std::string{})},
                {"review_id",  request.value("review_id", std::string{})},
                {"prompt",     request.value("prompt",    std::string{})},
            };
            shell_cbs_.broadcast_event("permission_request", evt.dump());
          }
          return;
        }

        // ── shell ────────────────────────────────────────────────────────
        if (cap == "shell") {
          if (workspace_root.empty()) {
            reply({{"outcome","err"},{"error",{{"code","no_space"},{"message","no active space for shell capability"}}}});
            return;
          }
          // Extract cwd; default to workspace_root if absent.
          std::filesystem::path cwd = workspace_root;
          const std::string cwd_str = request.value("cwd", std::string{});
          if (!cwd_str.empty()) {
            std::filesystem::path candidate(cwd_str);
            // (task 4.3) Scope enforcement: cwd must be within workspace_root.
            std::error_code ec;
            auto rel = std::filesystem::relative(candidate, workspace_root, ec);
            if (ec || rel.empty() || rel.native().substr(0,2) == "..") {
              reply({{"outcome","err"},{"error",{{"code","scope_violation"},
                  {"message","cwd is outside workspace root"}}}});
              return;
            }
            cwd = candidate;
          }
          // argv → single command string (join with spaces).
          std::string cmd;
          if (request.contains("argv") && request["argv"].is_array()) {
            for (const auto& a : request["argv"]) {
              if (!cmd.empty()) cmd += ' ';
              if (a.is_string()) cmd += a.get<std::string>();
            }
          }
          if (cmd.empty()) {
            reply({{"outcome","err"},{"error",{{"code","bad_request"},{"message","empty argv"}}}});
            return;
          }
          // Hard floor: block execution of sensitive system paths (task 7.3).
          // Extract the first token of cmd as the candidate executable path.
          const std::string first_token = cmd.substr(0, cmd.find(' '));
          if (!first_token.empty() && IsSensitivePath(std::filesystem::path(first_token))) {
            reply({{"outcome","err"},{"error",{{"code","permission_denied"},
                {"message","access to sensitive path denied"}}}});
            return;
          }
          const auto result = RunShellCommand(cwd, cmd);
          if (result.exit_code == 0) {
            reply({{"outcome","ok"},{"stdout",result.stdout_data},
                   {"stderr",result.stderr_data},{"exit_code",result.exit_code}});
          } else {
            reply({{"outcome","err"},{"error",{{"code","exec_failed"},
                {"message","command exited with code " + std::to_string(result.exit_code)},
                {"stdout",result.stdout_data},{"stderr",result.stderr_data},
                {"exit_code",result.exit_code}}}});
          }
          return;
        }

        // ── filesystem ───────────────────────────────────────────────────
        if (cap == "filesystem") {
          if (workspace_root.empty()) {
            reply({{"outcome","err"},{"error",{{"code","no_space"},{"message","no active space for filesystem capability"}}}});
            return;
          }
          const auto& op = request.value("op", nlohmann::json{});
          const std::string op_kind = op.value("kind", std::string{});
          if (op_kind == "read") {
            const std::string path_str = op.value("path", std::string{});
            if (path_str.empty()) {
              reply({{"outcome","err"},{"error",{{"code","bad_request"},{"message","path required"}}}});
              return;
            }
            // Hard floor: block reads to sensitive system paths (task 7.1).
            if (IsSensitivePath(std::filesystem::path(path_str))) {
              reply({{"outcome","err"},{"error",{{"code","permission_denied"},
                  {"message","access to sensitive path denied"}}}});
              return;
            }
            // Phase 2: proxy to Rust runtime FileRead.
            nlohmann::json req = {
                {"kind", "file_read"},
                {"workspace_root", workspace_root},
                {"path", path_str},
            };
            // reply_fn is captured by value — fire-and-forget via SendControl.
            auto reply_copy = reply;
            runtime_proxy_->SendControl(std::move(req),
                [reply_copy](nlohmann::json resp, bool is_error) {
                  if (is_error) {
                    reply_copy({{"outcome","err"},{"error",{{"code","read_failed"},{"message",
                        resp.value("error", nlohmann::json{}).value("message","read failed")}}}});
                    return;
                  }
                  const auto& p = resp.contains("payload") ? resp["payload"] : resp;
                  const std::string content = p.value("content", std::string{});
                  reply_copy({{"outcome","ok"},{"content", content}});
                });
            return;
          }
          if (op_kind == "write") {
            const std::string path_str = op.value("path", std::string{});
            const std::string content  = op.value("content", std::string{});
            if (path_str.empty()) {
              reply({{"outcome","err"},{"error",{{"code","bad_request"},{"message","path required"}}}});
              return;
            }
            // Hard floor: block writes to sensitive system paths (task 7.2).
            if (IsSensitivePath(std::filesystem::path(path_str))) {
              reply({{"outcome","err"},{"error",{{"code","permission_denied"},
                  {"message","access to sensitive path denied"}}}});
              return;
            }
            // Phase 2: proxy to Rust runtime FileWrite.
            nlohmann::json req = {
                {"kind", "file_write"},
                {"workspace_root", workspace_root},
                {"path", path_str},
                {"content", content},
            };
            auto reply_copy = reply;
            runtime_proxy_->SendControl(std::move(req),
                [reply_copy](nlohmann::json resp, bool is_error) {
                  if (is_error) {
                    reply_copy({{"outcome","err"},{"error",{{"code","write_failed"},{"message",
                        resp.value("error", nlohmann::json{}).value("message","write failed")}}}});
                    return;
                  }
                  reply_copy({{"outcome","ok"}});
                });
            return;
          }
          reply({{"outcome","err"},{"error",{{"code","unsupported"},
              {"message","unknown filesystem op"},{"op_kind",op_kind}}}});
          return;
        }

        // ── notify ───────────────────────────────────────────────────────
        if (cap == "notify") {
          const std::string title = request.value("title", std::string{});
          const std::string body  = request.value("body",  std::string{});
          if (shell_cbs_.broadcast_event) {
            nlohmann::json evt = {{"title",title},{"body",body},
                {"level",request.value("level","info")}};
            shell_cbs_.broadcast_event("notification", evt.dump());
          }
          reply({{"outcome","ok"}});
          return;
        }

        // ── unhandled capability ─────────────────────────────────────────
        nlohmann::json err_resp = {
            {"outcome", "err"},
            {"error", {{"code", "unsupported"},
                       {"message", "capability not supported by host"},
                       {"capability", cap}}},
        };
        reply(std::move(err_resp));
      });
}

// ---------------------------------------------------------------------------
// LLM config channels
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleLlmConfig(std::string_view channel,
                                    std::string_view payload,
                                    CefRefPtr<Callback> callback) {
  if (channel == "llm.config.set") {
    auto j = nlohmann::json::parse(payload, nullptr, false);
    LlmConfig cfg;
    cfg.base_url = j.is_object() ? j.value("base_url", std::string{}) : std::string{};
    cfg.api_key  = j.is_object() ? j.value("api_key",  std::string{}) : std::string{};
    space_manager_->store().SetLlmConfig(cfg);
    callback->Success("ok");
    return true;
  }
  if (channel == "llm.config.get") {
    const auto cfg = space_manager_->store().GetLlmConfig();
    callback->Success(nlohmann::json{{"base_url", cfg.base_url}, {"api_key", cfg.api_key}}.dump());
    return true;
  }
  if (channel == "llm.providers.get") {
    const std::string raw = space_manager_->store().GetKv("llm.providers");
    const std::string active =
        space_manager_->store().GetKv("llm.active_provider_id");
    callback->Success(nlohmann::json{{"raw", raw}, {"active_id", active}}.dump());
    return true;
  }
  if (channel == "llm.providers.set") {
    auto j = nlohmann::json::parse(payload, nullptr, false);
    const std::string raw    = j.is_object() ? j.value("raw",       std::string{}) : std::string{};
    const std::string active = j.is_object() ? j.value("active_id", std::string{}) : std::string{};
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
    callback->Success(nlohmann::json{{"url", url}, {"text", ""}}.dump());
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
        ("\"" + ev + "\"") + "," + nlohmann::json(pl).dump() + ");";
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
                                     e + "\"," + nlohmann::json(p).dump() + ");";
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

bool BridgeHandler::HandleShell(CefRefPtr<CefBrowser> browser,
                                std::string_view channel,
                                std::string_view payload,
                                CefRefPtr<Callback> callback) {
  auto j = nlohmann::json::parse(payload, nullptr, false);
  auto get = [&](const char* k) -> std::string {
    return j.is_object() ? j.value(k, std::string{}) : std::string{};
  };

  if (channel == "shell.tabs_list") {
    if (!shell_cbs_.list_tabs) { callback->Success("{\"tabs\":[],\"active_tab_id\":-1}"); return true; }
    callback->Success(shell_cbs_.list_tabs());
    return true;
  }

  if (channel == "shell.tab_new") {
    if (!shell_cbs_.new_tab) { callback->Failure(503, "not available"); return true; }
    const std::string url = get("url");
    callback->Success(shell_cbs_.new_tab(url.empty() ? "https://www.google.com" : url));
    return true;
  }

  if (channel == "shell.tab_switch") {
    const std::string sid = get("id");
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
    const std::string sid = get("id");
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
    const std::string kind = get("kind");
    callback->Success(shell_cbs_.tab_open_singleton(kind));
    return true;
  }

  if (channel == "shell.tab_new_kind") {
    if (!shell_cbs_.new_tab_kind) {
      callback->Failure(503, "not available");
      return true;
    }
    const std::string kind = get("kind");
    const std::string out = shell_cbs_.new_tab_kind(kind);
    callback->Success(out.empty() ? "{}" : out);
    return true;
  }

  if (channel == "shell.this_tab_id") {
    if (!shell_cbs_.this_tab_id) {
      callback->Success("{\"tabId\":\"\",\"meta\":{}}");
      return true;
    }
    const int bid = browser ? browser->GetIdentifier() : 0;
    callback->Success(shell_cbs_.this_tab_id(bid));
    return true;
  }

  if (channel == "shell.tab_set_meta") {
    const std::string key   = get("key");
    const std::string value = get("value");
    if (!key.empty() && shell_cbs_.tab_set_meta) {
      const int bid = browser ? browser->GetIdentifier() : 0;
      shell_cbs_.tab_set_meta(bid, key, value);
    }
    callback->Success("ok");
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
    const std::string url = get("url");
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
    const std::string url = get("url");
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
    const std::string url = get("url");
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
  if (channel == "theme.get") {
    if (!theme_cbs_.get_mode) {
      callback->Success("{\"mode\":\"system\",\"resolved\":\"dark\"}");
      return true;
    }
    callback->Success(theme_cbs_.get_mode());
    return true;
  }

  if (channel == "theme.set") {
    auto j = nlohmann::json::parse(payload, nullptr, false);
    const std::string mode = j.is_object() ? j.value("mode", std::string{}) : std::string{};
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
  if (channel == "tab.set_toolbar_state") {
    if (!shell_cbs_.set_toolbar_state) { callback->Success("ok"); return true; }
    auto j = nlohmann::json::parse(payload, nullptr, false);
    const std::string tab_id = j.is_object() ? j.value("tabId", std::string{}) : std::string{};
    if (tab_id.empty()) {
      callback->Failure(400, "missing tabId");
      return true;
    }
    // Forward the entire "state" sub-object as raw JSON.
    std::string state_json;
    if (j.is_object() && j.contains("state") && j["state"].is_object())
      state_json = j["state"].dump();
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
    auto j = nlohmann::json::parse(payload, nullptr, false);
    const std::string tab_id = j.is_object() ? j.value("tabId", std::string{}) : std::string{};
    if (tab_id.empty()) {
      callback->Failure(400, "missing tabId");
      return true;
    }
    // color may be either a string or null; treat null/absent as empty.
    std::string color;
    if (j.is_object() && j.contains("color") && j["color"].is_string())
      color = j["color"].get<std::string>();
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
    // Phase 2: proxy to Rust runtime.
    if (!runtime_proxy_) {
      callback->Failure(503, "runtime not available");
      return true;
    }
    nlohmann::json req = {
        {"kind", "workspace_layout"},
        {"workspace_root", sp->workspace_root.string()},
    };
    runtime_proxy_->SendControl(std::move(req),
        [callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            callback->Failure(500,
                resp.value("error", nlohmann::json{})
                    .value("message", "workspace_layout failed"));
            return;
          }
          // resp is the ControlResponse::Data payload
          const auto& p = resp.contains("payload") ? resp["payload"] : resp;
          callback->Success(p.dump());
        });
    return true;
  }

  if (channel == "workspace.gitignore_suggestions") {
    static const std::vector<std::string> kSuggested = {
        ".cronymax/flows/*/runs/*/trace.jsonl",
        ".cronymax/flows/*/runs/*/reviews.json",
    };
    const auto gitignore_path = sp->workspace_root / ".gitignore";
    std::string gitignore_content;
    {
      std::ifstream in(gitignore_path);
      if (in) {
        std::ostringstream ss;
        ss << in.rdbuf();
        gitignore_content = ss.str();
      }
    }
    nlohmann::json arr = nlohmann::json::array();
    for (const auto& entry : kSuggested) {
      if (gitignore_content.find(entry) == std::string::npos)
        arr.push_back(entry);
    }
    callback->Success(nlohmann::json{{"missing", arr}}.dump());
    return true;
  }

  if (channel == "workspace.prompts.list") {
    const auto prompts_dir = sp->workspace_root / ".cronymax" / "prompts";
    nlohmann::json arr = nlohmann::json::array();
    std::error_code ec;
    for (const auto& entry :
         std::filesystem::directory_iterator(prompts_dir, ec)) {
      const auto& p = entry.path();
      // Accept files named  <name>.prompt.md
      if (p.extension() == ".md" && p.stem().extension() == ".prompt") {
        std::ifstream f(p);
        if (f) {
          std::ostringstream ss;
          ss << f.rdbuf();
          const std::string name = p.stem().stem().string();
          arr.push_back({{"name", name}, {"content", ss.str()}});
        }
      }
    }
    callback->Success(nlohmann::json{{"prompts", arr}}.dump());
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
  if (!sp) {
    callback->Failure(503, "no active space");
    return true;
  }

  // Parse payload once for all channels that need it.
  auto jp = nlohmann::json::parse(payload, nullptr, false);
  auto extract_field = [&](std::string_view key) -> std::string {
    if (!jp.is_object()) return {};
    auto it = jp.find(std::string(key));
    if (it == jp.end() || !it->is_string()) return {};
    return it->get<std::string>();
  };

  // Helper: forward a control request to Rust and relay the payload back.
  auto send_ctl = [&](nlohmann::json req) {
    if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return; }
    runtime_proxy_->SendControl(std::move(req),
        [callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            callback->Failure(500, resp.value("error", nlohmann::json{})
                                       .value("message", "registry error"));
            return;
          }
          const auto& p = resp.contains("payload") ? resp["payload"] : resp;
          callback->Success(p.dump());
        });
  };

  // ── Phase 3: agent registry (proxied to Rust) ─────────────────────────

  if (channel == "agent.registry.list") {
    send_ctl({
        {"kind", "agent_registry_list"},
        {"workspace_root", sp->workspace_root.string()},
    });
    return true;
  }

  if (channel == "agent.registry.load") {
    const auto name = extract_field("name");
    if (name.empty()) { callback->Failure(400, "name required"); return true; }
    send_ctl({
        {"kind", "agent_registry_load"},
        {"workspace_root", sp->workspace_root.string()},
        {"name", name},
    });
    return true;
  }

  if (channel == "agent.registry.save") {
    const std::string name = jp.is_object() ? jp.value("name", std::string{}) : std::string{};
    const std::string yaml = jp.is_object() ? jp.value("yaml", std::string{}) : std::string{};
    if (name.empty()) { callback->Failure(400, "name required"); return true; }
    if (yaml.empty()) { callback->Failure(400, "yaml required"); return true; }
    if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return true; }
    runtime_proxy_->SendControl({
        {"kind", "agent_registry_save"},
        {"workspace_root", sp->workspace_root.string()},
        {"name", name},
        {"yaml", yaml},
    }, [callback](nlohmann::json resp, bool is_error) {
        if (is_error) {
          callback->Failure(500, resp.value("error", nlohmann::json{})
                                     .value("message", "save failed"));
          return;
        }
        callback->Success("{\"ok\":true}");
    });
    return true;
  }

  if (channel == "agent.registry.delete") {
    const std::string name = jp.is_object() ? jp.value("name", std::string{}) : std::string{};
    if (name.empty()) { callback->Failure(400, "name required"); return true; }
    if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return true; }
    runtime_proxy_->SendControl({
        {"kind", "agent_registry_delete"},
        {"workspace_root", sp->workspace_root.string()},
        {"name", name},
    }, [callback](nlohmann::json resp, bool is_error) {
        if (is_error) {
          callback->Failure(404, resp.value("error", nlohmann::json{})
                                     .value("message", "delete failed"));
          return;
        }
        callback->Success("{\"ok\":true}");
    });
    return true;
  }

  // space.profile.get and space.profile.set were removed in the
  // workspace-with-profile change. Profile management is now done
  // via the profiles.* bridge channels.

  if (channel == "flow.list") {
    // Phase 2: proxy to Rust runtime.
    if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return true; }
    nlohmann::json req = {
        {"kind", "flow_list"},
        {"workspace_root", sp->workspace_root.string()},
        {"builtin_flows_dir", space_manager_->builtin_flows_dir().string()},
    };
    runtime_proxy_->SendControl(std::move(req),
        [callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            callback->Failure(500,
                resp.value("error", nlohmann::json{}).value("message", "flow_list failed"));
            return;
          }
          const auto& p = resp.contains("payload") ? resp["payload"] : resp;
          callback->Success(p.dump());
        });
    return true;
  }

  if (channel == "flow.load") {
    // Phase 2: proxy to Rust runtime.
    if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return true; }
    const auto id = extract_field("id");
    if (id.empty()) { callback->Failure(400, "id required"); return true; }
    nlohmann::json req = {
        {"kind", "flow_load"},
        {"workspace_root", sp->workspace_root.string()},
        {"flow_id", id},
    };
    runtime_proxy_->SendControl(std::move(req),
        [callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            const auto err = resp.value("error", nlohmann::json{});
            callback->Failure(404, err.value("message", "flow not found"));
            return;
          }
          const auto& p = resp.contains("payload") ? resp["payload"] : resp;
          callback->Success(p.dump());
        });
    return true;
  }

  // flow.save  payload {flow_id, graph}  → {ok:bool, error?}
  if (channel == "flow.save") {
    // Phase 2: proxy to Rust runtime.
    if (!runtime_proxy_) {
      callback->Success(R"({"ok":false,"error":"runtime not available"})");
      return true;
    }
    const auto flow_id = extract_field("flow_id");
    if (flow_id.empty()) {
      callback->Success(R"({"ok":false,"error":"flow_id required"})");
      return true;
    }
    if (!jp.contains("graph") || jp["graph"].is_null()) {
      callback->Success(R"({"ok":false,"error":"graph required"})");
      return true;
    }
    nlohmann::json req = {
        {"kind", "flow_save"},
        {"workspace_root", sp->workspace_root.string()},
        {"flow_id", flow_id},
        {"graph", jp["graph"]},
    };
    runtime_proxy_->SendControl(std::move(req),
        [callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            const auto msg = resp.value("error", nlohmann::json{})
                                 .value("message", "flow_save failed");
            callback->Success(nlohmann::json{{"ok", false}, {"error", msg}}.dump());
            return;
          }
          callback->Success(R"({"ok":true})");
        });
    return true;
  }

  if (channel == "doc_type.list") {
    // Phase 3: proxy to Rust runtime.
    if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return true; }
    runtime_proxy_->SendControl({
        {"kind", "doc_type_list"},
        {"workspace_root", sp->workspace_root.string()},
        {"builtin_doc_types_dir", space_manager_->builtin_doc_types_dir().string()},
    }, [callback](nlohmann::json resp, bool is_error) {
        if (is_error) {
          callback->Failure(500, resp.value("error", nlohmann::json{})
                                     .value("message", "doc_type_list failed"));
          return;
        }
        const auto& p = resp.contains("payload") ? resp["payload"] : resp;
        callback->Success(p.dump());
    });
    return true;
  }

  // doc_type.load  payload {name}
  //   Returns the full schema details for a single doc type.
  if (channel == "doc_type.load") {
    const std::string name = jp.is_object() ? jp.value("name", std::string{}) : std::string{};
    if (name.empty()) { callback->Failure(400, "name required"); return true; }
    if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return true; }
    runtime_proxy_->SendControl({
        {"kind", "doc_type_load"},
        {"workspace_root", sp->workspace_root.string()},
        {"builtin_doc_types_dir", space_manager_->builtin_doc_types_dir().string()},
        {"name", name},
    }, [callback](nlohmann::json resp, bool is_error) {
        if (is_error) {
          callback->Failure(404, resp.value("error", nlohmann::json{})
                                     .value("message", "not found"));
          return;
        }
        const auto& p = resp.contains("payload") ? resp["payload"] : resp;
        callback->Success(p.dump());
    });
    return true;
  }

  // -------------------------------------------------------------------------
  // doc_type.save   payload {name, display_name, description?}
  // doc_type.delete payload {name}
  // -------------------------------------------------------------------------
  if (channel == "doc_type.save") {
    const std::string name         = jp.is_object() ? jp.value("name",         std::string{}) : std::string{};
    const std::string display_name = jp.is_object() ? jp.value("display_name", std::string{}) : std::string{};
    const std::string description  = jp.is_object() ? jp.value("description",  std::string{}) : std::string{};
    if (name.empty()) { callback->Failure(400, "name required"); return true; }
    if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return true; }
    runtime_proxy_->SendControl({
        {"kind", "doc_type_save"},
        {"workspace_root", sp->workspace_root.string()},
        {"name", name},
        {"display_name", display_name.empty() ? name : display_name},
        {"description", description},
    }, [callback](nlohmann::json resp, bool is_error) {
        if (is_error) {
          callback->Failure(500, resp.value("error", nlohmann::json{})
                                     .value("message", "save failed"));
          return;
        }
        callback->Success("{\"ok\":true}");
    });
    return true;
  }

  if (channel == "doc_type.delete") {
    const std::string name = jp.is_object() ? jp.value("name", std::string{}) : std::string{};
    if (name.empty()) { callback->Failure(400, "name required"); return true; }
    if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return true; }
    runtime_proxy_->SendControl({
        {"kind", "doc_type_delete"},
        {"workspace_root", sp->workspace_root.string()},
        {"name", name},
    }, [callback](nlohmann::json resp, bool is_error) {
        if (is_error) {
          callback->Failure(404, resp.value("error", nlohmann::json{})
                                     .value("message", "not found"));
          return;
        }
        callback->Success("{\"ok\":true}");
    });
    return true;
  }

  // -------------------------------------------------------------------------
  // -------------------------------------------------------------------------
  // FlowRuntime channels — rewired to RuntimeProxy (task 3.1).
  //   flow.run.start      payload {flow_id, initial_input?} → {run_id}
  //   flow.run.cancel     payload {run_id}                  → {ok:true}
  //   flow.run.pause      payload {run_id}                  → {ok:true}
  //   flow.run.resume     payload {run_id}                  → {ok:true}
  //   flow.run.post_input payload {run_id, input}           → {ok:true}
  //   flow.run.status     not available via direct query; use events.subscribe
  //   flow.run.list       not available via direct query; use events.subscribe
  // -------------------------------------------------------------------------
  if (channel.rfind("flow.run.", 0) == 0) {
    if (!runtime_proxy_) {
      callback->Failure(503, "runtime not available");
      return true;
    }
    if (!sp) { callback->Failure(503, "no active space"); return true; }

    if (channel == "flow.run.start") {
      const auto flow_id = extract_field("flow_id");
      if (flow_id.empty()) {
        callback->Failure(400, "flow_id required");
        return true;
      }
      const auto initial_input = extract_field("initial_input");
      nlohmann::json run_payload = {
          {"flow_id", flow_id},
          {"workspace_root", sp->workspace_root.string()},
      };
      if (!initial_input.empty()) run_payload["initial_input"] = initial_input;
      nlohmann::json req = {
          {"kind", "start_run"},
          {"space_id", sp->id},
          {"payload", std::move(run_payload)}
      };
      runtime_proxy_->SendControl(std::move(req),
          [callback](nlohmann::json resp, bool is_error) {
            if (is_error) {
              callback->Failure(500,
                  resp.value("error", nlohmann::json{})
                      .value("message", "start_run failed"));
              return;
            }
            const std::string run_id = resp.value("run_id", std::string{});
            callback->Success("{\"run_id\":" +
                              nlohmann::json(run_id).dump() + "}");
          });
      return true;
    }

    if (channel == "flow.run.cancel") {
      const auto run_id = extract_field("run_id");
      if (run_id.empty()) { callback->Failure(400, "run_id required"); return true; }
      nlohmann::json req = {{"kind", "cancel_run"}, {"run_id", run_id}};
      runtime_proxy_->SendControl(std::move(req),
          [callback](nlohmann::json resp, bool is_error) {
            callback->Success(is_error ? "{\"ok\":false}" : "{\"ok\":true}");
          });
      return true;
    }

    if (channel == "flow.run.pause") {
      const auto run_id = extract_field("run_id");
      if (run_id.empty()) { callback->Failure(400, "run_id required"); return true; }
      nlohmann::json req = {{"kind", "pause_run"}, {"run_id", run_id}};
      runtime_proxy_->SendControl(std::move(req),
          [callback](nlohmann::json resp, bool is_error) {
            callback->Success(is_error ? "{\"ok\":false}" : "{\"ok\":true}");
          });
      return true;
    }

    if (channel == "flow.run.resume") {
      const auto run_id = extract_field("run_id");
      if (run_id.empty()) { callback->Failure(400, "run_id required"); return true; }
      nlohmann::json req = {{"kind", "resume_run"}, {"run_id", run_id}};
      runtime_proxy_->SendControl(std::move(req),
          [callback](nlohmann::json resp, bool is_error) {
            callback->Success(is_error ? "{\"ok\":false}" : "{\"ok\":true}");
          });
      return true;
    }

    if (channel == "flow.run.post_input") {
      const auto run_id = extract_field("run_id");
      if (run_id.empty()) { callback->Failure(400, "run_id required"); return true; }
      nlohmann::json input_payload;
      if (jp.is_object())
        input_payload = jp.value("input", nlohmann::json{});
      nlohmann::json req = {
          {"kind", "post_input"},
          {"run_id", run_id},
          {"payload", std::move(input_payload)}
      };
      runtime_proxy_->SendControl(std::move(req),
          [callback](nlohmann::json resp, bool is_error) {
            callback->Success(is_error ? "{\"ok\":false}" : "{\"ok\":true}");
          });
      return true;
    }

    // flow.run.status / flow.run.list: query operations are served via
    // runtime event subscriptions once task 4.x lands.
    // TODO(4.2): implement once RuntimeToClient carries run state queries.
    callback->Failure(501,
        "flow.run.status and flow.run.list are not available via direct query; "
        "subscribe to runtime events instead");
    return true;
  }
  // -------------------------------------------------------------------------
  // mention.user_input — server-side @mention parser. Renderer sends the raw
  // user-typed text and the active flow id; we return the matched agent
  // names (and any unknown mentions) so the renderer can dispatch the
  // message to those agents.
  //   payload: {flow_id, text}
  //   reply:   {mentions:[name], unknown:[name]}
  // -------------------------------------------------------------------------
  if (channel == "mention.user_input") {
    const auto flow_id = extract_field("flow_id");
    const auto text    = extract_field("text");
    if (flow_id.empty()) {
      callback->Failure(400, "flow_id required");
      return true;
    }
    WorkspaceLayout layout(sp->workspace_root);
    std::set<std::string> known;
    const auto flow_yaml = layout.FlowFile(flow_id);
    std::error_code yaml_ec;
    if (std::filesystem::exists(flow_yaml, yaml_ec)) {
      for (const auto& agent_id : LoadFlowAgents(flow_yaml))
        known.insert(agent_id);
    }
    // Inline @mention parser: @[a-zA-Z_][a-zA-Z0-9_-]*
    nlohmann::json mentions = nlohmann::json::array();
    nlohmann::json unknown_arr = nlohmann::json::array();
    for (size_t i = 0; i < text.size(); ++i) {
      if (text[i] != '@') continue;
      if (i > 0 && (std::isalnum((unsigned char)text[i-1]) || text[i-1] == '_'))
        continue;
      size_t j = i + 1;
      if (j >= text.size() ||
          (!std::isalpha((unsigned char)text[j]) && text[j] != '_'))
        continue;
      while (j < text.size() &&
             (std::isalnum((unsigned char)text[j]) || text[j] == '_' ||
              text[j] == '-'))
        ++j;
      std::string name = text.substr(i + 1, j - i - 1);
      if (known.count(name)) mentions.push_back(name);
      else unknown_arr.push_back(name);
    }
    callback->Success(
        nlohmann::json{{"mentions", mentions}, {"unknown", unknown_arr}}.dump());
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
  if (!runtime_proxy_) { callback->Failure(503, "runtime not available"); return true; }

  auto jp = nlohmann::json::parse(payload, nullptr, false);
  auto extract = [&](std::string_view key) -> std::string {
    if (!jp.is_object()) return {};
    auto it = jp.find(std::string(key));
    if (it == jp.end() || !it->is_string()) return {};
    return it->get<std::string>();
  };

  const std::string flow_id = extract("flow");
  if (flow_id.empty()) {
    callback->Failure(400, "missing 'flow' in payload");
    return true;
  }
  const std::string workspace_root = sp->workspace_root.string();

  auto relay_payload = [callback](nlohmann::json resp, bool is_error) {
    if (is_error) {
      callback->Failure(500, resp.value("error", nlohmann::json{})
                                 .value("message", "document error"));
      return;
    }
    const auto& p = resp.contains("payload") ? resp["payload"] : resp;
    callback->Success(p.dump());
  };

  if (channel == "document.list") {
    runtime_proxy_->SendControl({
        {"kind",           "document_list"},
        {"workspace_root", workspace_root},
        {"flow_id",        flow_id},
    }, relay_payload);
    return true;
  }

  if (channel == "document.read") {
    const std::string name    = extract("name");
    const std::string rev_str = extract("revision");
    if (name.empty()) {
      callback->Failure(400, "missing 'name' in payload");
      return true;
    }
    nlohmann::json req = {
        {"kind",           "document_read"},
        {"workspace_root", workspace_root},
        {"flow_id",        flow_id},
        {"name",           name},
    };
    if (!rev_str.empty()) {
      if (rev_str.find_first_not_of("0123456789") != std::string::npos) {
        callback->Failure(400, "bad 'revision' value"); return true;
      }
      req["revision"] = std::atoi(rev_str.c_str());
    }
    runtime_proxy_->SendControl(std::move(req), relay_payload);
    return true;
  }

  if (channel == "document.subscribe") {
    // Subscribe to Rust runtime document events forwarded as "document.changed".
    {
      const std::string topic = "space/" + sp->id + "/document_events";
      nlohmann::json req_sub = {{"kind", "subscribe"}, {"topic", topic}};
      runtime_proxy_->SendControl(std::move(req_sub),
          [this](nlohmann::json resp, bool is_error) {
            if (is_error) return;
            runtime_proxy_->SubscribeEvents(
                [this](const nlohmann::json& event) {
                  if (shell_cbs_.broadcast_event)
                    shell_cbs_.broadcast_event("document.changed", event.dump());
                });
          });
    }
    callback->Success("{\"ok\":true,\"event\":\"document.changed\"}");
    return true;
  }

  if (channel == "document.submit") {
    const std::string name    = extract("name");
    const std::string content = jp.is_object() ? jp.value("content", std::string{}) : std::string{};
    if (name.empty()) { callback->Failure(400, "missing 'name' in payload"); return true; }
    runtime_proxy_->SendControl({
        {"kind",           "document_submit"},
        {"workspace_root", workspace_root},
        {"flow_id",        flow_id},
        {"name",           name},
        {"content",        content},
    }, [this, flow_id, name, callback](nlohmann::json resp, bool is_error) {
        if (is_error) {
          callback->Failure(500, resp.value("error", nlohmann::json{})
                                     .value("message", "submit failed"));
          return;
        }
        const auto& p = resp.contains("payload") ? resp["payload"] : resp;
        int rev = p.value("revision", 0);
        std::string sha = p.value("sha256", "");
        if (shell_cbs_.broadcast_event) {
          shell_cbs_.broadcast_event("document.changed",
              nlohmann::json{{"flow", flow_id}, {"name", name}, {"revision", rev}}.dump());
        }
        callback->Success(
            nlohmann::json{{"ok", true}, {"revision", rev}, {"sha", sha}}.dump());
    });
    return true;
  }

  if (channel == "document.suggestion.apply") {
    const std::string run_id     = extract("run_id");
    const std::string name       = extract("name");
    const std::string block_id   = extract("block_id");
    const std::string suggestion = jp.is_object() ? jp.value("suggestion", std::string{}) : std::string{};
    if (run_id.empty() || name.empty() || block_id.empty() || suggestion.empty()) {
      callback->Failure(400, "missing 'run_id', 'name', 'block_id', or 'suggestion'");
      return true;
    }
    runtime_proxy_->SendControl({
        {"kind",           "document_suggestion_apply"},
        {"workspace_root", workspace_root},
        {"flow_id",        flow_id},
        {"run_id",         run_id},
        {"name",           name},
        {"block_id",       block_id},
        {"suggestion",     suggestion},
    }, [this, flow_id, name, callback](nlohmann::json resp, bool is_error) {
        if (is_error) {
          callback->Failure(500, resp.value("error", nlohmann::json{})
                                     .value("message", "suggestion_apply failed"));
          return;
        }
        const auto& p = resp.contains("payload") ? resp["payload"] : resp;
        int rev = p.value("new_revision", 0);
        std::string sha = p.value("sha", "");
        if (shell_cbs_.broadcast_event) {
          shell_cbs_.broadcast_event("document.changed",
              nlohmann::json{{"flow", flow_id}, {"name", name}, {"revision", rev}}.dump());
        }
        callback->Success(
            nlohmann::json{{"ok", true}, {"new_revision", rev}, {"sha", sha}}.dump());
    });
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

  auto jp_rev = nlohmann::json::parse(payload, nullptr, false);
  auto extract = [&](std::string_view key) -> std::string {
    if (!jp_rev.is_object()) return {};
    auto it = jp_rev.find(std::string(key));
    if (it == jp_rev.end() || !it->is_string()) return {};
    return it->get<std::string>();
  };

  // review.list — forwarded to runtime via RuntimeProxy.
  if (channel == "review.list") {
    if (!runtime_proxy_) {
      callback->Failure(503, "runtime not connected");
      return true;
    }
    const std::string run_id_l = extract("run_id");
    if (run_id_l.empty()) {
      callback->Failure(400, "missing 'run_id' in payload");
      return true;
    }
    nlohmann::json req = {{"kind", "list_reviews"}, {"run_id", run_id_l}};
    runtime_proxy_->SendControl(std::move(req),
        [callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            callback->Failure(500,
                resp.value("error", nlohmann::json{})
                    .value("message", "list_reviews failed"));
            return;
          }
          callback->Success(resp.dump());
        });
    return true;
  }

  // Mutating review channels — forwarded to the runtime via RuntimeProxy.
  const std::string run_id    = extract("run_id");
  const std::string review_id = extract("review_id");
  const std::string body      = extract("body");

  if (channel == "review.approve") {
    if (!runtime_proxy_ || review_id.empty()) {
      callback->Failure(503, runtime_proxy_ ? "missing review_id" : "runtime not connected");
      return true;
    }
    nlohmann::json req = {
        {"kind",      "resolve_review"},
        {"run_id",    run_id},
        {"review_id", review_id},
        {"decision",  "approve"},
    };
    if (!body.empty()) req["notes"] = body;
    runtime_proxy_->SendControl(std::move(req),
        [callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            callback->Failure(500,
                resp.value("error", nlohmann::json{})
                    .value("message", "approve failed"));
            return;
          }
          callback->Success("{\"ok\":true}");
        });
    return true;
  }

  if (channel == "review.request_changes") {
    if (!runtime_proxy_ || review_id.empty()) {
      callback->Failure(503, runtime_proxy_ ? "missing review_id" : "runtime not connected");
      return true;
    }
    nlohmann::json req = {
        {"kind",      "resolve_review"},
        {"run_id",    run_id},
        {"review_id", review_id},
        {"decision",  "reject"},
    };
    if (!body.empty()) req["notes"] = body;
    runtime_proxy_->SendControl(std::move(req),
        [callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            callback->Failure(500,
                resp.value("error", nlohmann::json{})
                    .value("message", "request_changes failed"));
            return;
          }
          callback->Success("{\"ok\":true}");
        });
    return true;
  }

  if (channel == "review.comment") {
    if (!runtime_proxy_ || run_id.empty()) {
      callback->Failure(503, runtime_proxy_ ? "missing run_id" : "runtime not connected");
      return true;
    }
    nlohmann::json comment_payload = {{"comment", body}};
    if (!review_id.empty()) comment_payload["review_id"] = review_id;
    const std::string name = extract("name");
    if (!name.empty()) comment_payload["doc"] = name;
    nlohmann::json req = {
        {"kind",    "post_input"},
        {"run_id",  run_id},
        {"payload", std::move(comment_payload)},
    };
    runtime_proxy_->SendControl(std::move(req),
        [callback](nlohmann::json resp, bool is_error) {
          callback->Success("{\"ok\":true}");
        });
    return true;
  }

  callback->Failure(404, "unknown review channel");
  return true;
}


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
    nlohmann::json events_arr = nlohmann::json::array();
    for (const auto& e : res.events) events_arr.push_back(nlohmann::json::parse(AppEventToJson(e), nullptr, false));
    callback->Success(nlohmann::json{{"events", events_arr}, {"cursor", res.cursor}}.dump());
    return true;
  }

  // events.subscribe { flow_id?, run_id? } — replay-then-live.
  //
  // (task 3.2) If RuntimeProxy is connected, also subscribe to runtime events
  // so that runtime-emitted payloads are fanned out on the "event" broadcast
  // channel alongside local event_bus events.  Both subscriptions are cleaned
  // up when the browser closes.
  if (channel == "events.subscribe") {
    event_bus::Scope scope;
    scope.flow_id = ExtractJsonString(payload, "flow_id");
    scope.run_id = ExtractJsonString(payload, "run_id");
    auto cbs = shell_cbs_;
    // Local event_bus subscription (events from events.append, legacy paths).
    auto token = bus->Subscribe(scope, [cbs](const event_bus::AppEvent& e) {
      if (cbs.broadcast_event) cbs.broadcast_event("event", e.ToJson());
    });
    const int bid = browser ? browser->GetIdentifier() : 0;
    {
      std::lock_guard<std::mutex> g(browser_subs_mutex_);
      browser_subs_[bid].push_back([bus, token]() { bus->Unsubscribe(token); });
    }
    // NOTE: we intentionally do NOT add a runtime_proxy_ SubscribeEvents entry
    // here.  The start_run response already creates one Rust subscription;
    // creating a second subscription for the same run topic causes each event
    // to arrive twice on the Mach transport and be broadcast N×2 times.
    // The space-level subscription in OnSpaceSwitch (Lambda S) is the single
    // fan-out path for runtime events.
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
    bool have_payload = false;
    auto p_raw = nlohmann::json::parse(std::string(payload), nullptr, false);
    if (!p_raw.is_discarded() && p_raw.is_object() &&
        p_raw.contains("payload") && p_raw["payload"].is_object()) {
      evt.payload = p_raw["payload"];
      have_payload = true;
    }
    if (!have_payload) {
      // Fall back to constructing { body, mentions:[] } from top-level fields.
      auto body = ExtractJsonString(payload, "body");
      evt.payload = {{"body", body}, {"mentions", nlohmann::json::array()}};
    }
    auto id = bus->Append(std::move(evt));
    callback->Success(nlohmann::json{{"id", id}}.dump());
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
    nlohmann::json rows_arr = nlohmann::json::array();
    for (const auto& r : res.rows)
      rows_arr.push_back(nlohmann::json::parse(InboxRowToJson(r), nullptr, false));
    callback->Success(nlohmann::json{
        {"rows",               rows_arr},
        {"unread_count",       res.unread_count},
        {"needs_action_count", res.needs_action_count},
    }.dump());
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
    nlohmann::json enabled = nlohmann::json::array();
    for (const auto& k : kinds) enabled.push_back(k);
    callback->Success(nlohmann::json{{"enabled", enabled}}.dump());
    return true;
  }

  if (channel == "notifications.set_kind_pref") {
    auto kind = ExtractJsonString(payload, "kind");
    if (kind.empty()) {
      callback->Failure(400, "kind required");
      return true;
    }
    auto jp_n = nlohmann::json::parse(payload, nullptr, false);
    bool enabled = jp_n.is_object() ? jp_n.value("enabled", false) : false;
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

// (task 4.2) Called by MainWindow when the active Space changes.
// Tears down the outgoing space's runtime event subscription so stale
// events from the old space are not forwarded to the new space's renderers.
// Then auto-subscribes to the new space's runtime event stream so events
// arrive even before the renderer calls `events.subscribe`.
void BridgeHandler::OnSpaceSwitch(const std::string& old_space_id,
                                  const std::string& new_space_id) {
  if (!runtime_proxy_) return;

  // Tear down old space subscription.
  if (!old_space_id.empty()) {
    SpaceRuntimeSub old_sub;
    {
      std::lock_guard<std::mutex> g(space_subs_mu_);
      auto it = space_runtime_subs_.find(old_space_id);
      if (it != space_runtime_subs_.end()) {
        old_sub = it->second;
        space_runtime_subs_.erase(it);
      }
    }
    if (old_sub.ev_token >= 0)
      runtime_proxy_->UnsubscribeEvents(old_sub.ev_token);
    if (!old_sub.runtime_sub_id.empty()) {
      nlohmann::json req = {
          {"kind", "unsubscribe"},
          {"subscription", old_sub.runtime_sub_id},
      };
      runtime_proxy_->SendControl(std::move(req),
          [](nlohmann::json, bool) {});
    }
  }

  // Auto-subscribe to new space's event stream.
  if (!new_space_id.empty()) {
    nlohmann::json req = {
        {"kind",  "subscribe"},
        {"topic", "space/" + new_space_id + "/events"},
    };
    runtime_proxy_->SendControl(std::move(req),
        [this, new_space_id](nlohmann::json resp, bool is_error) {
          if (is_error) return;
          SpaceRuntimeSub sub;
          sub.runtime_sub_id = resp.value("subscription", std::string{});
          sub.ev_token = runtime_proxy_->SubscribeEvents(
              [this](const nlohmann::json& event) {
                if (shell_cbs_.broadcast_event)
                  shell_cbs_.broadcast_event("event", event.dump());
              });
          std::lock_guard<std::mutex> g(space_subs_mu_);
          space_runtime_subs_[new_space_id] = std::move(sub);
        });
  }
}

// ---------------------------------------------------------------------------
// Profiles channels
// ---------------------------------------------------------------------------

namespace {
// Serialize a ProfileRecord to a JSON object (not a dump string).
nlohmann::json ProfileRecordToJson(const ProfileRecord& r) {
  auto to_arr = [](const std::vector<std::string>& v) {
    auto arr = nlohmann::json::array();
    for (const auto& s : v) arr.push_back(s);
    return arr;
  };
  return nlohmann::json{
      {"id",                r.id},
      {"name",              r.name},
      {"allow_network",     r.allow_network},
      {"extra_read_paths",  to_arr(r.extra_read_paths)},
      {"extra_write_paths", to_arr(r.extra_write_paths)},
      {"extra_deny_paths",  to_arr(r.extra_deny_paths)},
  };
}
}  // namespace

bool BridgeHandler::HandleProfiles(CefRefPtr<CefBrowser> browser,
                                   std::string_view channel,
                                   std::string_view payload,
                                   CefRefPtr<Callback> callback) {
  ProfileStore& ps = space_manager_->profile_store();
  const auto jp = nlohmann::json::parse(payload, nullptr, false);

  if (channel == "profiles.list") {
    const auto records = ps.List();
    auto arr = nlohmann::json::array();
    for (const auto& r : records) arr.push_back(ProfileRecordToJson(r));
    callback->Success(arr.dump());
    return true;
  }

  if (channel == "profiles.create") {
    if (!jp.is_object()) { callback->Failure(400, "payload must be an object"); return true; }
    ProfileRules rules;
    rules.name          = jp.value("name",          std::string{});
    rules.allow_network = jp.value("allow_network",  true);
    if (rules.name.empty()) { callback->Failure(400, "name required"); return true; }
    if (jp.contains("extra_read_paths") && jp["extra_read_paths"].is_array())
      for (const auto& p : jp["extra_read_paths"]) if (p.is_string()) rules.extra_read_paths.push_back(p);
    if (jp.contains("extra_write_paths") && jp["extra_write_paths"].is_array())
      for (const auto& p : jp["extra_write_paths"]) if (p.is_string()) rules.extra_write_paths.push_back(p);
    if (jp.contains("extra_deny_paths") && jp["extra_deny_paths"].is_array())
      for (const auto& p : jp["extra_deny_paths"]) if (p.is_string()) rules.extra_deny_paths.push_back(p);

    std::string new_id;
    const auto err = ps.Create(rules, &new_id);
    if (err == ProfileStoreError::kAlreadyExists) { callback->Failure(409, "profile name already exists"); return true; }
    if (err == ProfileStoreError::kIoError)        { callback->Failure(500, "I/O error writing profile"); return true; }

    if (const auto rec = ps.Get(new_id)) {
      callback->Success(ProfileRecordToJson(*rec).dump());
    } else {
      callback->Success(nlohmann::json{{"id", new_id}}.dump());
    }
    return true;
  }

  if (channel == "profiles.update") {
    if (!jp.is_object()) { callback->Failure(400, "payload must be an object"); return true; }
    const std::string id = jp.value("id", std::string{});
    if (id.empty()) { callback->Failure(400, "id required"); return true; }
    ProfileRules rules;
    rules.name          = jp.value("name",          std::string{});
    rules.allow_network = jp.value("allow_network",  true);
    if (rules.name.empty()) { callback->Failure(400, "name required"); return true; }
    if (jp.contains("extra_read_paths") && jp["extra_read_paths"].is_array())
      for (const auto& p : jp["extra_read_paths"]) if (p.is_string()) rules.extra_read_paths.push_back(p);
    if (jp.contains("extra_write_paths") && jp["extra_write_paths"].is_array())
      for (const auto& p : jp["extra_write_paths"]) if (p.is_string()) rules.extra_write_paths.push_back(p);
    if (jp.contains("extra_deny_paths") && jp["extra_deny_paths"].is_array())
      for (const auto& p : jp["extra_deny_paths"]) if (p.is_string()) rules.extra_deny_paths.push_back(p);

    const auto err = ps.Update(id, rules);
    if (err == ProfileStoreError::kNotFound) { callback->Failure(404, "profile not found"); return true; }
    if (err == ProfileStoreError::kIoError)  { callback->Failure(500, "I/O error writing profile"); return true; }

    if (const auto rec = ps.Get(id)) {
      callback->Success(ProfileRecordToJson(*rec).dump());
    } else {
      callback->Success("{\"ok\":true}");
    }
    return true;
  }

  if (channel == "profiles.delete") {
    if (!jp.is_object()) { callback->Failure(400, "payload must be an object"); return true; }
    const std::string id = jp.value("id", std::string{});
    if (id.empty()) { callback->Failure(400, "id required"); return true; }

    // Collect the profile_id of every space so we can detect "in use".
    std::vector<std::string> space_profile_ids;
    for (const auto& s : space_manager_->spaces())
      space_profile_ids.push_back(s->profile_id);

    const auto err = ps.Delete(id, space_profile_ids);
    if (err == ProfileStoreError::kNotFound)       { callback->Failure(404, "profile not found"); return true; }
    if (err == ProfileStoreError::kCannotDeleteDefault) { callback->Failure(403, "cannot delete default profile"); return true; }
    if (err == ProfileStoreError::kInUse)          { callback->Failure(409, "profile is in use by one or more spaces"); return true; }
    if (err == ProfileStoreError::kIoError)        { callback->Failure(500, "I/O error deleting profile"); return true; }

    callback->Success("{\"ok\":true}");
    return true;
  }

  callback->Failure(404, "unknown profiles channel");
  return true;
}

}  // namespace cronymax

