#include "browser/bridge_handler.h"

#include <sys/wait.h>
#include <unistd.h>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <mutex>
#include <optional>
#include <sstream>
#include <unordered_map>
#include <unordered_set>

#include <nlohmann/json.hpp>

#include "event_bus/app_event.h"
#include "event_bus/event_bus.h"
// (task 4.1) flow_runtime.h, trace_event.h, trace_writer.h removed —
// run lifecycle is now owned by the Rust runtime over GIPS.
// (task 5.0) sandbox, flow C++ modules removed — logic now lives in the
// Rust runtime (crates/cronymax).
// (Phase 2) file_broker.h removed — file I/O proxied to Rust FileBroker.
// flow_yaml.h removed — mention parsing moved to Rust MentionParse IPC.
#include "common/path_utils.h"
#include "common/types.h"
#include "include/base/cef_callback.h"
#include "include/cef_process_message.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {
namespace {

// ---------------------------------------------------------------------------
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
  if (sep == std::string::npos)
    return {request, ""};
  return {request.substr(0, sep), request.substr(sep + 1)};
}

std::string SpaceToJson(const Space& sp) {
  return nlohmann::json{
      {"id", sp.id},
      {"name", sp.name},
      {"root_path", sp.workspace_root.string()},
      {"profile_id", sp.profile_id},
  }
      .dump();
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
    close(stdout_pipe[0]);
    close(stdout_pipe[1]);
    close(stderr_pipe[0]);
    close(stderr_pipe[1]);
    return result;
  }
  if (pid == 0) {
    close(stdout_pipe[0]);
    close(stderr_pipe[0]);
    dup2(stdout_pipe[1], STDOUT_FILENO);
    close(stdout_pipe[1]);
    dup2(stderr_pipe[1], STDERR_FILENO);
    close(stderr_pipe[1]);
    if (!cwd.empty())
      chdir(cwd.c_str());
    execl("/bin/sh", "/bin/sh", "-c", cmd.c_str(), nullptr);
    _exit(127);
  }
  close(stdout_pipe[1]);
  close(stderr_pipe[1]);
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
  if (j.is_discarded() || !j.is_object())
    return {};
  auto it = j.find(std::string(key));
  if (it == j.end() || !it->is_string())
    return {};
  return it->get<std::string>();
}

// Extract an integer field from a JSON payload using nlohmann::json (no-throw).
long long ExtractJsonInt(std::string_view payload, std::string_view key) {
  auto j = nlohmann::json::parse(payload, nullptr, /*allow_exceptions=*/false);
  if (j.is_discarded() || !j.is_object())
    return 0;
  auto it = j.find(std::string(key));
  if (it == j.end() || !it->is_number_integer())
    return 0;
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
      {"state", event_bus::InboxStateToString(r.state)},
      {"flow_id", r.flow_id},
      {"kind", r.kind},
  };
  if (r.snooze_until.has_value())
    j["snooze_until"] = *r.snooze_until;
  return j.dump();
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
  if (channel.rfind("flow.", 0) == 0 || channel.rfind("doc_type.", 0) == 0)
    return HandleRegistry(channel, payload, callback);

  if (channel.rfind("document.", 0) == 0)
    return HandleDocument(channel, payload, callback);

  if (channel.rfind("review.", 0) == 0)
    return HandleReview(channel, payload, callback);

  if (channel == "activity.snapshot")
    return HandleActivitySnapshot(channel, payload, callback);

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
  if (!sp) {
    callback->Failure(503, "no active space");
    return true;
  }

  // Resolve a TerminalSession from optional "id" field; fall back to active.
  auto j = nlohmann::json::parse(payload, nullptr, false);

  // List terminals for the active Space.
  if (channel == "terminal.list") {
    nlohmann::json items = nlohmann::json::array();
    for (const auto& t : sp->terminals)
      items.push_back({{"id", t->id}, {"name", t->name}});
    callback->Success(nlohmann::json{
        {"active", sp->active_terminal_id},
        {"items", items}}.dump());
    return true;
  }

  // Create a new terminal session (does NOT auto-start the PTY).
  if (channel == "terminal.new") {
    auto* t = sp->CreateTerminal();
    const std::string item =
        nlohmann::json{{"id", t->id}, {"name", t->name}}.dump();
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
    const std::string id =
        j.is_object() ? j.value("id", std::string{}) : std::string{};
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
    const std::string id =
        j.is_object() ? j.value("id", std::string{}) : std::string{};
    if (!sp->CloseTerminal(id)) {
      callback->Failure(404, "no such terminal");
      return true;
    }
    const std::string removed = nlohmann::json{{"id", id}}.dump();
    if (shell_cbs_.broadcast_event) {
      shell_cbs_.broadcast_event("terminal.removed", removed);
    } else {
      SendEvent(browser, "terminal.removed", removed);
    }
    if (!sp->active_terminal_id.empty()) {
      const std::string sw =
          nlohmann::json{{"id", sp->active_terminal_id}}.dump();
      if (shell_cbs_.broadcast_event) {
        shell_cbs_.broadcast_event("terminal.switched", sw);
      } else {
        SendEvent(browser, "terminal.switched", sw);
      }
    }
    callback->Success("ok");
    return true;
  }

  if (channel == "terminal.restart") {
    if (shell_cbs_.terminal_restart)
      shell_cbs_.terminal_restart();
    callback->Success("ok");
    return true;
  }

  if (channel == "terminal.block_save") {
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
    space_manager_->store().CreateBlock(row);
    callback->Success("ok");
    return true;
  }

  if (channel == "terminal.blocks_load") {
    const std::string sid =
        j.is_object() ? j.value("space_id", std::string{}) : std::string{};
    const std::string& effective_sid = sid.empty() ? sp->id : sid;
    const auto blocks =
        space_manager_->store().ListBlocksForSpace(effective_sid);
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
    const auto* active_sp = space_manager_->ActiveSpace();
    for (const auto& sp : space_manager_->spaces()) {
      arr.push_back({
          {"id", sp->id},
          {"name", sp->name},
          {"root_path", sp->workspace_root.string()},
          {"active", active_sp && sp->id == active_sp->id},
      });
    }
    callback->Success(arr.dump());
    return true;
  }

  if (channel == "space.create") {
    auto j = nlohmann::json::parse(payload, nullptr, false);
    const std::string root =
        j.is_object() ? j.value("root_path", std::string{}) : std::string{};
    const std::string profile_id =
        j.is_object() ? j.value("profile_id", std::string{"default"})
                      : std::string{"default"};
    if (root.empty()) {
      callback->Failure(400, "root_path required");
      return true;
    }
    const auto id =
        space_manager_->CreateSpace(std::filesystem::path(root), profile_id);
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

  if (channel == "space.switch") {
    auto j = nlohmann::json::parse(payload, nullptr, false);
    const std::string id =
        j.is_object() ? j.value("space_id", std::string{}) : std::string{};
    if (!space_manager_->SwitchTo(id)) {
      callback->Failure(404, "space not found");
      return true;
    }
    callback->Success("ok");
    return true;
  }

  if (channel == "space.delete") {
    auto j = nlohmann::json::parse(payload, nullptr, false);
    const std::string id =
        j.is_object() ? j.value("space_id", std::string{}) : std::string{};
    if (!space_manager_->DeleteSpace(id)) {
      callback->Failure(404, "space not found");
      return true;
    }
    callback->Success("ok");
    SendEvent(browser, "space.deleted",
              nlohmann::json{{"space_id", id}}.dump());
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
    const std::string rid =
        j.is_object() ? j.value("request_id", std::string{}) : std::string{};
    const std::string dec =
        j.is_object() ? j.value("decision", std::string{}) : std::string{};
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
          resp = {
              {"outcome", "err"},
              {"error",
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
  if (!runtime_proxy_)
    return;
  runtime_proxy_->SetCapabilityHandler([this](const std::string& corr_id,
                                              const nlohmann::json& request,
                                              RuntimeProxy::CapabilityReplyFn
                                                  reply) {
    const std::string cap = request.value("capability", std::string{});
    const std::string space_id = request.value("space_id", std::string{});

    // Resolve the owning Space for scope enforcement.
    // FindSpace is private; iterate the public spaces() list instead.
    Space* sp = nullptr;
    if (space_id.empty()) {
      sp = space_manager_->ActiveSpace();
    } else {
      for (const auto& s : space_manager_->spaces()) {
        if (s->id == space_id) {
          sp = s.get();
          break;
        }
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
            {"run_id", request.value("run_id", std::string{})},
            {"review_id", request.value("review_id", std::string{})},
            {"prompt", request.value("prompt", std::string{})},
        };
        shell_cbs_.broadcast_event("permission_request", evt.dump());
      }
      return;
    }

    // ── shell ────────────────────────────────────────────────────────
    if (cap == "shell") {
      if (workspace_root.empty()) {
        reply({{"outcome", "err"},
               {"error",
                {{"code", "no_space"},
                 {"message", "no active space for shell capability"}}}});
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
        if (ec || rel.empty() || rel.native().substr(0, 2) == "..") {
          reply({{"outcome", "err"},
                 {"error",
                  {{"code", "scope_violation"},
                   {"message", "cwd is outside workspace root"}}}});
          return;
        }
        cwd = candidate;
      }
      // argv → single command string (join with spaces).
      std::string cmd;
      if (request.contains("argv") && request["argv"].is_array()) {
        for (const auto& a : request["argv"]) {
          if (!cmd.empty())
            cmd += ' ';
          if (a.is_string())
            cmd += a.get<std::string>();
        }
      }
      if (cmd.empty()) {
        reply(
            {{"outcome", "err"},
             {"error", {{"code", "bad_request"}, {"message", "empty argv"}}}});
        return;
      }
      // Hard floor: block execution of sensitive system paths (task 7.3).
      // Extract the first token of cmd as the candidate executable path.
      const std::string first_token = cmd.substr(0, cmd.find(' '));
      if (!first_token.empty() &&
          IsSensitivePath(std::filesystem::path(first_token))) {
        reply({{"outcome", "err"},
               {"error",
                {{"code", "permission_denied"},
                 {"message", "access to sensitive path denied"}}}});
        return;
      }
      const auto result = RunShellCommand(cwd, cmd);
      if (result.exit_code == 0) {
        reply({{"outcome", "ok"},
               {"stdout", result.stdout_data},
               {"stderr", result.stderr_data},
               {"exit_code", result.exit_code}});
      } else {
        reply({{"outcome", "err"},
               {"error",
                {{"code", "exec_failed"},
                 {"message", "command exited with code " +
                                 std::to_string(result.exit_code)},
                 {"stdout", result.stdout_data},
                 {"stderr", result.stderr_data},
                 {"exit_code", result.exit_code}}}});
      }
      return;
    }

    // ── filesystem ───────────────────────────────────────────────────
    if (cap == "filesystem") {
      if (workspace_root.empty()) {
        reply({{"outcome", "err"},
               {"error",
                {{"code", "no_space"},
                 {"message", "no active space for filesystem capability"}}}});
        return;
      }
      const auto& op = request.value("op", nlohmann::json{});
      const std::string op_kind = op.value("kind", std::string{});
      if (op_kind == "read") {
        const std::string path_str = op.value("path", std::string{});
        if (path_str.empty()) {
          reply({{"outcome", "err"},
                 {"error",
                  {{"code", "bad_request"}, {"message", "path required"}}}});
          return;
        }
        // Hard floor: block reads to sensitive system paths (task 7.1).
        if (IsSensitivePath(std::filesystem::path(path_str))) {
          reply({{"outcome", "err"},
                 {"error",
                  {{"code", "permission_denied"},
                   {"message", "access to sensitive path denied"}}}});
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
        runtime_proxy_->SendControl(
            std::move(req), [reply_copy](nlohmann::json resp, bool is_error) {
              if (is_error) {
                reply_copy(
                    {{"outcome", "err"},
                     {"error",
                      {{"code", "read_failed"},
                       {"message", resp.value("error", nlohmann::json{})
                                       .value("message", "read failed")}}}});
                return;
              }
              const auto& p = resp.contains("payload") ? resp["payload"] : resp;
              const std::string content = p.value("content", std::string{});
              reply_copy({{"outcome", "ok"}, {"content", content}});
            });
        return;
      }
      if (op_kind == "write") {
        const std::string path_str = op.value("path", std::string{});
        const std::string content = op.value("content", std::string{});
        if (path_str.empty()) {
          reply({{"outcome", "err"},
                 {"error",
                  {{"code", "bad_request"}, {"message", "path required"}}}});
          return;
        }
        // Hard floor: block writes to sensitive system paths (task 7.2).
        if (IsSensitivePath(std::filesystem::path(path_str))) {
          reply({{"outcome", "err"},
                 {"error",
                  {{"code", "permission_denied"},
                   {"message", "access to sensitive path denied"}}}});
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
        runtime_proxy_->SendControl(
            std::move(req), [reply_copy](nlohmann::json resp, bool is_error) {
              if (is_error) {
                reply_copy(
                    {{"outcome", "err"},
                     {"error",
                      {{"code", "write_failed"},
                       {"message", resp.value("error", nlohmann::json{})
                                       .value("message", "write failed")}}}});
                return;
              }
              reply_copy({{"outcome", "ok"}});
            });
        return;
      }
      reply({{"outcome", "err"},
             {"error",
              {{"code", "unsupported"},
               {"message", "unknown filesystem op"},
               {"op_kind", op_kind}}}});
      return;
    }

    // ── notify ───────────────────────────────────────────────────────
    if (cap == "notify") {
      const std::string title = request.value("title", std::string{});
      const std::string body = request.value("body", std::string{});
      if (shell_cbs_.broadcast_event) {
        nlohmann::json evt = {{"title", title},
                              {"body", body},
                              {"level", request.value("level", "info")}};
        shell_cbs_.broadcast_event("notification", evt.dump());
      }
      reply({{"outcome", "ok"}});
      return;
    }

    // ── unhandled capability ─────────────────────────────────────────
    nlohmann::json err_resp = {
        {"outcome", "err"},
        {"error",
         {{"code", "unsupported"},
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
    cfg.base_url =
        j.is_object() ? j.value("base_url", std::string{}) : std::string{};
    cfg.api_key =
        j.is_object() ? j.value("api_key", std::string{}) : std::string{};
    space_manager_->store().SetLlmConfig(cfg);
    callback->Success("ok");
    return true;
  }
  if (channel == "llm.config.get") {
    const auto cfg = space_manager_->store().GetLlmConfig();
    callback->Success(nlohmann::json{
        {"base_url", cfg.base_url},
        {"api_key",
         cfg.api_key}}.dump());
    return true;
  }
  if (channel == "llm.providers.get") {
    const std::string raw = space_manager_->store().GetKv("llm.providers");
    const std::string active =
        space_manager_->store().GetKv("llm.active_provider_id");
    callback->Success(
        nlohmann::json{{"raw", raw}, {"active_id", active}}.dump());
    return true;
  }
  if (channel == "llm.providers.set") {
    auto j = nlohmann::json::parse(payload, nullptr, false);
    const std::string raw =
        j.is_object() ? j.value("raw", std::string{}) : std::string{};
    const std::string active =
        j.is_object() ? j.value("active_id", std::string{}) : std::string{};
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
    if (!browser) {
      callback->Failure(503, "no browser");
      return true;
    }
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
    if (!target)
      return;
    const auto frame = target->GetMainFrame();
    if (!frame)
      return;
    const std::string js = "window.cronymax?.browser?.onDispatch?.(" +
                           ("\"" + ev + "\"") + "," +
                           nlohmann::json(pl).dump() + ");";
    frame->ExecuteJavaScript(js, frame->GetURL(), 0);
  };

  if (!CefCurrentlyOn(TID_UI)) {
    CefPostTask(TID_UI,
                base::BindOnce(
                    [](CefRefPtr<CefBrowser> b, std::string e, std::string p) {
                      if (!b)
                        return;
                      const auto frame = b->GetMainFrame();
                      if (!frame)
                        return;
                      const std::string js =
                          "window.cronymax?.browser?.onDispatch?.(\"" + e +
                          "\"," + nlohmann::json(p).dump() + ");";
                      frame->ExecuteJavaScript(js, frame->GetURL(), 0);
                    },
                    browser, ev, pl));
    return;
  }
  dispatch(browser);
}

// ---------------------------------------------------------------------------
// HandleRuntimeProcessMessage / SendRuntimeReply / SendRuntimeEvent
//
// These implement the renderer↔browser bridge for window.cronymax.runtime.
// The renderer sends cronymax.runtime.ctrl process messages; the browser
// forwards them to the Rust runtime via RuntimeProxy and replies with
// cronymax.runtime.ctrl.reply.  Event subscriptions are forwarded back via
// cronymax.runtime.event.
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleRuntimeProcessMessage(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> frame,
    CefRefPtr<CefProcessMessage> message) {
  if (message->GetName() != "cronymax.runtime.ctrl")
    return false;

  auto margs = message->GetArgumentList();
  const std::string corr_id = margs->GetString(0).ToString();
  const std::string req_str = margs->GetString(1).ToString();

  if (!runtime_proxy_) {
    SendRuntimeReply(
        browser, corr_id,
        {{"kind", "err"}, {"error", {{"message", "runtime not available"}}}},
        true);
    return true;
  }

  auto req = nlohmann::json::parse(req_str, nullptr, false);
  if (req.is_discarded()) {
    SendRuntimeReply(
        browser, corr_id,
        {{"kind", "err"}, {"error", {{"message", "invalid JSON"}}}}, true);
    return true;
  }

  const std::string kind = req.value("kind", "");

  // ── Unsubscribe ─────────────────────────────────────────────────────────
  if (kind == "unsubscribe") {
    const std::string sub_id = req.value("subscription", std::string{});
    RendererSub sub;
    {
      std::lock_guard<std::mutex> g(renderer_subs_mu_);
      auto it = renderer_subs_.find(sub_id);
      if (it == renderer_subs_.end())
        return true;
      sub = it->second;
      renderer_subs_.erase(it);
    }
    if (sub.ev_token >= 0)
      runtime_proxy_->UnsubscribeEvents(sub.ev_token);
    if (!sub.runtime_sub_id.empty())
      runtime_proxy_->SendControl(
          {{"kind", "unsubscribe"}, {"subscription", sub_id}},
          [](nlohmann::json, bool) {});
    // One-way: no reply expected.
    return true;
  }

  // ── Subscribe ────────────────────────────────────────────────────────────
  if (kind == "subscribe") {
    runtime_proxy_->SendControl(req, [this, corr_id, browser](
                                         nlohmann::json resp, bool is_error) {
      if (is_error) {
        SendRuntimeReply(browser, corr_id, resp, true);
        return;
      }
      const std::string sub_id = resp.value("subscription", std::string{});
      if (sub_id.empty()) {
        SendRuntimeReply(browser, corr_id,
                         {{"kind", "err"},
                          {"error", {{"message", "missing subscription id"}}}},
                         true);
        return;
      }

      // Register a SubscribeEvents listener that forwards events to the
      // renderer via cronymax.runtime.event process messages.
      int64_t ev_token = runtime_proxy_->SubscribeEvents(
          [this, sub_id, browser](const nlohmann::json& envelope) {
            if (envelope.value("subscription", "") != sub_id)
              return;
            SendRuntimeEvent(browser, sub_id, envelope);
          });

      {
        std::lock_guard<std::mutex> g(renderer_subs_mu_);
        renderer_subs_[sub_id] = {ev_token, sub_id, browser};
      }

      // Reply: {kind:"subscribed", subscription: sub_id}
      SendRuntimeReply(browser, corr_id, resp, false);
    });
    return true;
  }

  // ── Arbitrary control request ────────────────────────────────────────────
  // Inject filesystem-dependent fields so the renderer doesn't need to know
  // the active workspace paths. Fields already present in req are kept as-is.
  {
    auto* sp = space_manager_->ActiveSpace();
    if (sp) {
      const std::string wroot = sp->workspace_root.string();
      // Channels that need workspace_root
      static const std::unordered_set<std::string> kNeedsWorkspace{
          "terminal_start",
          "agent_registry_list",
          "agent_registry_load",
          "agent_registry_save",
          "agent_registry_delete",
          "flow_list",
          "flow_load",
          "flow_save",
          "doc_type_list",
          "doc_type_load",
          "doc_type_save",
          "doc_type_delete",
          "start_run",
      };
      if (kNeedsWorkspace.count(kind) && !req.contains("workspace_root"))
        req["workspace_root"] = wroot;

      if (kind == "flow_list" && !req.contains("builtin_flows_dir"))
        req["builtin_flows_dir"] = space_manager_->builtin_flows_dir().string();

      if ((kind == "doc_type_list" || kind == "doc_type_load") &&
          !req.contains("builtin_doc_types_dir"))
        req["builtin_doc_types_dir"] =
            space_manager_->builtin_doc_types_dir().string();

      if (kind == "terminal_start") {
        if (!req.contains("shell"))
          req["shell"] = "/bin/zsh";
        if (!req.contains("cols"))
          req["cols"] = 100;
        if (!req.contains("rows"))
          req["rows"] = 30;
      }

      // start_run: also inject space_id and workspace_root into the nested
      // payload field (mirroring how HandleFlows builds the request).
      if (kind == "start_run") {
        if (!req.contains("space_id"))
          req["space_id"] = sp->id;
        if (!req.contains("payload") || !req["payload"].is_object())
          req["payload"] = nlohmann::json::object();
        if (!req["payload"].contains("workspace_root"))
          req["payload"]["workspace_root"] = wroot;

        // Agent run: payload.task present, but no payload.flow_id.
        // Inject LLM config from the active provider so the Rust runtime
        // can call the correct inference endpoint.
        if (req["payload"].contains("task") &&
            !req["payload"].contains("flow_id") &&
            !req["payload"].contains("llm")) {
          std::string base_url = "https://api.openai.com/v1";
          std::string api_key;
          std::string model = "gpt-4o-mini";
          std::string provider_kind = "openai_compat";
          // Prefer new-style provider list; fall back to legacy LlmConfig.
          const std::string providers_raw =
              space_manager_->store().GetKv("llm.providers");
          const std::string active_id =
              space_manager_->store().GetKv("llm.active_provider_id");
          if (!providers_raw.empty() && !active_id.empty()) {
            auto pj = nlohmann::json::parse(providers_raw, nullptr, false);
            if (!pj.is_discarded() && pj.is_array()) {
              for (const auto& p : pj) {
                if (p.value("id", std::string{}) == active_id) {
                  const std::string purl = p.value("base_url", std::string{});
                  if (!purl.empty())
                    base_url = purl;
                  if (const auto it = p.find("api_key");
                      it != p.end() && it->is_string()) {
                    const std::string pkey = it->get<std::string>();
                    if (!pkey.empty())
                      api_key = pkey;
                  }
                  const std::string pm =
                      p.value("default_model", std::string{});
                  if (!pm.empty())
                    model = pm;
                  const std::string pk = p.value("kind", std::string{});
                  if (!pk.empty())
                    provider_kind = pk;
                  break;
                }
              }
            }
          } else {
            const auto llm_cfg = space_manager_->store().GetLlmConfig();
            if (!llm_cfg.base_url.empty())
              base_url = llm_cfg.base_url;
            api_key = llm_cfg.api_key;
          }
          req["payload"]["llm"] = {
              {"base_url", base_url},
              {"api_key", api_key},
              {"model", model},
              {"provider_kind", provider_kind},
          };
          // If the frontend passed a model_override inside payload, use it
          // to override the provider's default_model while keeping other LLM
          // fields (base_url, api_key, provider_kind) from the active provider.
          if (req["payload"].contains("model_override")) {
            const std::string mo =
                req["payload"].value("model_override", std::string{});
            if (!mo.empty())
              req["payload"]["llm"]["model"] = mo;
            req["payload"].erase("model_override");
          }
        }
      }
    }
  }

  runtime_proxy_->SendControl(
      std::move(req),
      [this, corr_id, browser, kind](nlohmann::json resp, bool is_error) {
        // Unwrap payload envelope for channels that use it (mirrors C++ handler
        // convention: resp.payload if present, else resp).
        if (!is_error && resp.contains("payload") && !resp["payload"].is_null())
          SendRuntimeReply(browser, corr_id, resp["payload"], false);
        else
          SendRuntimeReply(browser, corr_id, resp, is_error);

        // For start_run: register the Rust subscription for cleanup when the
        // browser closes, mirroring what the legacy cefQuery agent.run handler
        // did.  This ensures the runtime subscription is unsubscribed if the
        // user closes the panel before the run completes.
        if (!is_error && kind == "start_run") {
          const std::string sub_id = resp.value("subscription", std::string{});
          if (!sub_id.empty() && browser) {
            const int bid = browser->GetIdentifier();
            std::lock_guard<std::mutex> g(browser_subs_mutex_);
            browser_subs_[bid].push_back([this, sub_id]() {
              if (runtime_proxy_) {
                nlohmann::json unsub = {{"kind", "unsubscribe"},
                                        {"subscription", sub_id}};
                runtime_proxy_->SendControl(std::move(unsub),
                                            [](nlohmann::json, bool) {});
              }
            });
          }
        }
      });
  return true;
}

void BridgeHandler::SendRuntimeReply(CefRefPtr<CefBrowser> browser,
                                     const std::string& corr_id,
                                     const nlohmann::json& response,
                                     bool is_error) {
  auto send = [browser, corr_id, resp_str = response.dump(), is_error]() {
    auto msg = CefProcessMessage::Create("cronymax.runtime.ctrl.reply");
    auto args = msg->GetArgumentList();
    args->SetString(0, corr_id);
    args->SetString(1, resp_str);
    args->SetBool(2, is_error);
    auto frame = browser->GetMainFrame();
    if (frame)
      frame->SendProcessMessage(PID_RENDERER, msg);
  };

  if (CefCurrentlyOn(TID_UI)) {
    send();
  } else {
    CefPostTask(TID_UI, base::BindOnce([](decltype(send) fn) { fn(); },
                                       std::move(send)));
  }
}

void BridgeHandler::SendRuntimeEvent(CefRefPtr<CefBrowser> browser,
                                     const std::string& sub_id,
                                     const nlohmann::json& event_envelope) {
  auto send = [browser, sub_id, env_str = event_envelope.dump()]() {
    auto msg = CefProcessMessage::Create("cronymax.runtime.event");
    auto args = msg->GetArgumentList();
    args->SetString(0, sub_id);
    args->SetString(1, env_str);
    auto frame = browser->GetMainFrame();
    if (frame)
      frame->SendProcessMessage(PID_RENDERER, msg);
  };

  if (CefCurrentlyOn(TID_UI)) {
    send();
  } else {
    CefPostTask(TID_UI, base::BindOnce([](decltype(send) fn) { fn(); },
                                       std::move(send)));
  }
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
    if (!shell_cbs_.list_tabs) {
      callback->Success("{\"tabs\":[],\"active_tab_id\":-1}");
      return true;
    }
    callback->Success(shell_cbs_.list_tabs());
    return true;
  }

  if (channel == "shell.tab_new") {
    if (!shell_cbs_.new_tab) {
      callback->Failure(503, "not available");
      return true;
    }
    const std::string url = get("url");
    callback->Success(
        shell_cbs_.new_tab(url.empty() ? "https://www.google.com" : url));
    return true;
  }

  if (channel == "shell.tab_switch") {
    const std::string sid = get("id");
    if (sid.empty()) {
      callback->Success("ok");
      return true;
    }
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
    if (sid.empty()) {
      callback->Success("ok");
      return true;
    }
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
    const std::string key = get("key");
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
    if (!shell_cbs_.navigate) {
      callback->Success("ok");
      return true;
    }
    const std::string url = get("url");
    if (!url.empty())
      shell_cbs_.navigate(url);
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.go_back") {
    if (shell_cbs_.go_back)
      shell_cbs_.go_back();
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.go_forward") {
    if (shell_cbs_.go_forward)
      shell_cbs_.go_forward();
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.reload") {
    if (shell_cbs_.reload)
      shell_cbs_.reload();
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.popover_open") {
    if (!shell_cbs_.popover_open) {
      callback->Success("ok");
      return true;
    }
    const std::string url = get("url");
    shell_cbs_.popover_open(url.empty() ? "https://www.google.com" : url);
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.open_external") {
    const std::string url = get("url");
    if (!url.empty() && shell_cbs_.open_external)
      shell_cbs_.open_external(url);
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.popover_close") {
    if (shell_cbs_.popover_close)
      shell_cbs_.popover_close();
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.popover_refresh") {
    if (shell_cbs_.popover_refresh)
      shell_cbs_.popover_refresh();
    callback->Success("ok");
    return true;
  }

  if (channel == "shell.popover_open_as_tab") {
    if (shell_cbs_.popover_open_as_tab)
      shell_cbs_.popover_open_as_tab();
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
    if (shell_cbs_.window_drag)
      shell_cbs_.window_drag();
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
    if (shell_cbs_.settings_popover_open)
      shell_cbs_.settings_popover_open();
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
    const std::string mode =
        j.is_object() ? j.value("mode", std::string{}) : std::string{};
    if (mode != "system" && mode != "light" && mode != "dark") {
      callback->Failure(400, "invalid mode");
      return true;
    }
    if (theme_cbs_.set_mode)
      theme_cbs_.set_mode(mode);
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
    if (!shell_cbs_.set_toolbar_state) {
      callback->Success("ok");
      return true;
    }
    auto j = nlohmann::json::parse(payload, nullptr, false);
    const std::string tab_id =
        j.is_object() ? j.value("tabId", std::string{}) : std::string{};
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
    if (!shell_cbs_.set_chrome_theme) {
      callback->Success("ok");
      return true;
    }
    auto j = nlohmann::json::parse(payload, nullptr, false);
    const std::string tab_id =
        j.is_object() ? j.value("tabId", std::string{}) : std::string{};
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
  if (!sp) {
    callback->Failure(503, "no active space");
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

  if (channel == "workspace.prompt.save") {
    auto jp2 = nlohmann::json::parse(payload, nullptr, false);
    if (!jp2.is_object()) {
      callback->Failure(400, "invalid payload");
      return true;
    }

    // Extract name and content fields.
    std::string name, content;
    {
      auto it = jp2.find("name");
      if (it == jp2.end() || !it->is_string()) {
        callback->Failure(400, "name required");
        return true;
      }
      name = it->get<std::string>();
    }
    {
      auto it = jp2.find("content");
      if (it == jp2.end() || !it->is_string()) {
        callback->Failure(400, "content required");
        return true;
      }
      content = it->get<std::string>();
    }

    // Validate name: reject path-traversal characters.
    if (name.empty() || name.find('/') != std::string::npos ||
        name.find('\\') != std::string::npos ||
        name.find("..") != std::string::npos ||
        name.find('\0') != std::string::npos) {
      callback->Success(
          nlohmann::json{{"ok", false}, {"error", "invalid name"}}.dump());
      return true;
    }

    const auto prompts_dir = sp->workspace_root / ".cronymax" / "prompts";
    std::error_code ec;
    std::filesystem::create_directories(prompts_dir, ec);
    if (ec) {
      callback->Success(
          nlohmann::json{{"ok", false}, {"error", ec.message()}}.dump());
      return true;
    }

    const auto target = prompts_dir / (name + ".prompt.md");
    std::ofstream f(target, std::ios::out | std::ios::trunc);
    if (!f) {
      callback->Success(nlohmann::json{
          {"ok", false}, {"error", "failed to open file for writing"}}
                            .dump());
      return true;
    }
    f << content;
    f.close();

    callback->Success(nlohmann::json{{"ok", true}}.dump());
    return true;
  }

  callback->Failure(404, "unknown workspace channel");
  return true;
}

// ---------------------------------------------------------------------------
// Registry channels
//
// All agent.registry.*, flow.*, doc_type.*, and flow.run.* channels are now
// handled via the direct renderer↔runtime IPC path
// (HandleRuntimeProcessMessage). The only remaining cefQuery channel here is
// mention.user_input, forwarded to the Rust runtime as "mention_parse".
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleRegistry(std::string_view channel,
                                   std::string_view payload,
                                   CefRefPtr<Callback> callback) {
  auto* sp = space_manager_->ActiveSpace();
  if (!sp) {
    callback->Failure(503, "no active space");
    return true;
  }

  auto jp = nlohmann::json::parse(payload, nullptr, false);
  auto extract_field = [&](std::string_view key) -> std::string {
    if (!jp.is_object())
      return {};
    auto it = jp.find(std::string(key));
    if (it == jp.end() || !it->is_string())
      return {};
    return it->get<std::string>();
  };

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
    if (flow_id.empty()) {
      callback->Failure(400, "flow_id required");
      return true;
    }
    if (!runtime_proxy_) {
      callback->Failure(503, "runtime not available");
      return true;
    }
    const std::string workspace_root = sp->workspace_root.string();
    const std::string text = extract_field("text");
    runtime_proxy_->SendControl(
        {
            {"kind", "mention_parse"},
            {"workspace_root", workspace_root},
            {"flow_id", flow_id},
            {"text", text},
        },
        [callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            callback->Failure(500,
                              resp.value("error", nlohmann::json{})
                                  .value("message", "mention parse error"));
            return;
          }
          const auto& p = resp.contains("payload") ? resp["payload"] : resp;
          callback->Success(p.dump());
        });
    return true;
  }

  callback->Failure(404, "unknown registry channel");
  return true;
}

// ---------------------------------------------------------------------------
// Document channels (Phase B): read/list current and historical revisions.
//
//   document.list      payload {flow}            \u2192 {docs:[{name,
//   latest_revision}]} document.read      payload {flow, name, [revision]}
//   \u2192 {content, revision} document.subscribe payload {flow} \u2192
//   {ok:true} (acks subscribe;
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
  if (!sp) {
    callback->Failure(503, "no active space");
    return true;
  }
  if (!runtime_proxy_) {
    callback->Failure(503, "runtime not available");
    return true;
  }

  auto jp = nlohmann::json::parse(payload, nullptr, false);
  auto extract = [&](std::string_view key) -> std::string {
    if (!jp.is_object())
      return {};
    auto it = jp.find(std::string(key));
    if (it == jp.end() || !it->is_string())
      return {};
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
    runtime_proxy_->SendControl(
        {
            {"kind", "document_list"},
            {"workspace_root", workspace_root},
            {"flow_id", flow_id},
        },
        relay_payload);
    return true;
  }

  if (channel == "document.read") {
    const std::string name = extract("name");
    const std::string rev_str = extract("revision");
    if (name.empty()) {
      callback->Failure(400, "missing 'name' in payload");
      return true;
    }
    nlohmann::json req = {
        {"kind", "document_read"},
        {"workspace_root", workspace_root},
        {"flow_id", flow_id},
        {"name", name},
    };
    if (!rev_str.empty()) {
      if (rev_str.find_first_not_of("0123456789") != std::string::npos) {
        callback->Failure(400, "bad 'revision' value");
        return true;
      }
      req["revision"] = std::atoi(rev_str.c_str());
    }
    runtime_proxy_->SendControl(std::move(req), relay_payload);
    return true;
  }

  if (channel == "document.subscribe") {
    // Subscribe to Rust runtime document events forwarded as
    // "document.changed".
    {
      const std::string topic = "space/" + sp->id + "/document_events";
      nlohmann::json req_sub = {{"kind", "subscribe"}, {"topic", topic}};
      runtime_proxy_->SendControl(
          std::move(req_sub), [this](nlohmann::json resp, bool is_error) {
            if (is_error)
              return;
            runtime_proxy_->SubscribeEvents([this](
                                                const nlohmann::json& event) {
              if (shell_cbs_.broadcast_event)
                shell_cbs_.broadcast_event("document.changed", event.dump());
            });
          });
    }
    callback->Success("{\"ok\":true,\"event\":\"document.changed\"}");
    return true;
  }

  if (channel == "document.submit") {
    const std::string name = extract("name");
    const std::string content =
        jp.is_object() ? jp.value("content", std::string{}) : std::string{};
    if (name.empty()) {
      callback->Failure(400, "missing 'name' in payload");
      return true;
    }
    runtime_proxy_->SendControl(
        {
            {"kind", "document_submit"},
            {"workspace_root", workspace_root},
            {"flow_id", flow_id},
            {"name", name},
            {"content", content},
        },
        [this, flow_id, name, callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            callback->Failure(500, resp.value("error", nlohmann::json{})
                                       .value("message", "submit failed"));
            return;
          }
          const auto& p = resp.contains("payload") ? resp["payload"] : resp;
          int rev = p.value("revision", 0);
          std::string sha = p.value("sha256", "");
          if (shell_cbs_.broadcast_event) {
            shell_cbs_.broadcast_event(
                "document.changed",
                nlohmann::json{
                    {"flow", flow_id}, {"name", name}, {"revision", rev}}
                    .dump());
          }
          callback->Success(nlohmann::json{
              {"ok", true},
              {"revision", rev},
              {"sha", sha}}.dump());
        });
    return true;
  }

  if (channel == "document.suggestion.apply") {
    const std::string run_id = extract("run_id");
    const std::string name = extract("name");
    const std::string block_id = extract("block_id");
    const std::string suggestion =
        jp.is_object() ? jp.value("suggestion", std::string{}) : std::string{};
    if (run_id.empty() || name.empty() || block_id.empty() ||
        suggestion.empty()) {
      callback->Failure(
          400, "missing 'run_id', 'name', 'block_id', or 'suggestion'");
      return true;
    }
    runtime_proxy_->SendControl(
        {
            {"kind", "document_suggestion_apply"},
            {"workspace_root", workspace_root},
            {"flow_id", flow_id},
            {"run_id", run_id},
            {"name", name},
            {"block_id", block_id},
            {"suggestion", suggestion},
        },
        [this, flow_id, name, callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            callback->Failure(500,
                              resp.value("error", nlohmann::json{})
                                  .value("message", "suggestion_apply failed"));
            return;
          }
          const auto& p = resp.contains("payload") ? resp["payload"] : resp;
          int rev = p.value("new_revision", 0);
          std::string sha = p.value("sha", "");
          if (shell_cbs_.broadcast_event) {
            shell_cbs_.broadcast_event(
                "document.changed",
                nlohmann::json{
                    {"flow", flow_id}, {"name", name}, {"revision", rev}}
                    .dump());
          }
          callback->Success(nlohmann::json{
              {"ok", true},
              {"new_revision", rev},
              {"sha", sha}}.dump());
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
  if (!sp) {
    callback->Failure(503, "no active space");
    return true;
  }

  auto jp_rev = nlohmann::json::parse(payload, nullptr, false);
  auto extract = [&](std::string_view key) -> std::string {
    if (!jp_rev.is_object())
      return {};
    auto it = jp_rev.find(std::string(key));
    if (it == jp_rev.end() || !it->is_string())
      return {};
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
    runtime_proxy_->SendControl(std::move(req), [callback](nlohmann::json resp,
                                                           bool is_error) {
      if (is_error) {
        callback->Failure(500, resp.value("error", nlohmann::json{})
                                   .value("message", "list_reviews failed"));
        return;
      }
      callback->Success(resp.dump());
    });
    return true;
  }

  // Mutating review channels — forwarded to the runtime via RuntimeProxy.
  const std::string run_id = extract("run_id");
  const std::string review_id = extract("review_id");
  const std::string body = extract("body");

  if (channel == "review.approve") {
    if (!runtime_proxy_ || review_id.empty()) {
      callback->Failure(
          503, runtime_proxy_ ? "missing review_id" : "runtime not connected");
      return true;
    }
    nlohmann::json req = {
        {"kind", "resolve_review"},
        {"run_id", run_id},
        {"review_id", review_id},
        {"decision", "approve"},
    };
    if (!body.empty())
      req["notes"] = body;
    runtime_proxy_->SendControl(
        std::move(req), [callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            callback->Failure(500, resp.value("error", nlohmann::json{})
                                       .value("message", "approve failed"));
            return;
          }
          callback->Success("{\"ok\":true}");
        });
    return true;
  }

  if (channel == "review.request_changes") {
    if (!runtime_proxy_ || review_id.empty()) {
      callback->Failure(
          503, runtime_proxy_ ? "missing review_id" : "runtime not connected");
      return true;
    }
    nlohmann::json req = {
        {"kind", "resolve_review"},
        {"run_id", run_id},
        {"review_id", review_id},
        {"decision", "reject"},
    };
    if (!body.empty())
      req["notes"] = body;
    runtime_proxy_->SendControl(std::move(req), [callback](nlohmann::json resp,
                                                           bool is_error) {
      if (is_error) {
        callback->Failure(500, resp.value("error", nlohmann::json{})
                                   .value("message", "request_changes failed"));
        return;
      }
      callback->Success("{\"ok\":true}");
    });
    return true;
  }

  if (channel == "review.comment") {
    if (!runtime_proxy_ || run_id.empty()) {
      callback->Failure(
          503, runtime_proxy_ ? "missing run_id" : "runtime not connected");
      return true;
    }
    nlohmann::json comment_payload = {{"comment", body}};
    if (!review_id.empty())
      comment_payload["review_id"] = review_id;
    const std::string name = extract("name");
    if (!name.empty())
      comment_payload["doc"] = name;
    nlohmann::json req = {
        {"kind", "post_input"},
        {"run_id", run_id},
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

// ---------------------------------------------------------------------------
// HandleActivitySnapshot: activity.snapshot
//
// Requests the full (runs + pending_reviews) snapshot for the active space
// from the runtime via ControlRequest::GetSpaceSnapshot.  Used by the
// Activity panel on mount to hydrate its initial state.
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleActivitySnapshot(std::string_view /*channel*/,
                                           std::string_view /*payload*/,
                                           CefRefPtr<Callback> callback) {
  auto* sp = space_manager_->ActiveSpace();
  if (!sp) {
    callback->Failure(503, "no active space");
    return true;
  }
  if (!runtime_proxy_) {
    callback->Failure(503, "runtime not connected");
    return true;
  }

  nlohmann::json req = {{"kind", "get_space_snapshot"}, {"space_id", sp->id}};
  runtime_proxy_->SendControl(std::move(req), [callback](nlohmann::json resp,
                                                         bool is_error) {
    if (is_error) {
      callback->Failure(500, resp.value("error", nlohmann::json{})
                                 .value("message", "get_space_snapshot failed"));
      return;
    }
    callback->Success(resp.dump());
  });
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
    if (lim > 0)
      q.limit = static_cast<int>(lim);
    auto res = bus->List(q);
    nlohmann::json events_arr = nlohmann::json::array();
    for (const auto& e : res.events)
      events_arr.push_back(
          nlohmann::json::parse(AppEventToJson(e), nullptr, false));
    callback->Success(
        nlohmann::json{{"events", events_arr}, {"cursor", res.cursor}}.dump());
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
      if (cbs.broadcast_event)
        cbs.broadcast_event("event", e.ToJson());
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
      if (event_bus::InboxStateFromString(state_str, &s))
        q.state = s;
    }
    long long lim = ExtractJsonInt(payload, "limit");
    if (lim > 0)
      q.limit = static_cast<int>(lim);
    auto res = bus->ListInbox(q);
    nlohmann::json rows_arr = nlohmann::json::array();
    for (const auto& r : res.rows)
      rows_arr.push_back(
          nlohmann::json::parse(InboxRowToJson(r), nullptr, false));
    callback->Success(nlohmann::json{
        {"rows", rows_arr},
        {"unread_count", res.unread_count},
        {"needs_action_count", res.needs_action_count},
    }
                          .dump());
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
    if (channel == "inbox.unread")
      target = event_bus::InboxState::kUnread;
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
    for (const auto& k : kinds)
      enabled.push_back(k);
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
    if (it == browser_subs_.end())
      return;
    cbs = std::move(it->second);
    browser_subs_.erase(it);
  }
  for (auto& f : cbs)
    f();
}

// (task 4.2) Called by MainWindow when the active Space changes.
// Tears down the outgoing space's runtime event subscription so stale
// events from the old space are not forwarded to the new space's renderers.
// Then auto-subscribes to the new space's runtime event stream so events
// arrive even before the renderer calls `events.subscribe`.
void BridgeHandler::OnSpaceSwitch(const std::string& old_space_id,
                                  const std::string& new_space_id) {
  if (!runtime_proxy_)
    return;

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
      runtime_proxy_->SendControl(std::move(req), [](nlohmann::json, bool) {});
    }
  }

  // Auto-subscribe to new space's event stream.
  if (!new_space_id.empty()) {
    nlohmann::json req = {
        {"kind", "subscribe"},
        {"topic", "space/" + new_space_id + "/events"},
    };
    runtime_proxy_->SendControl(
        std::move(req),
        [this, new_space_id](nlohmann::json resp, bool is_error) {
          if (is_error)
            return;
          SpaceRuntimeSub sub;
          sub.runtime_sub_id = resp.value("subscription", std::string{});
          sub.ev_token = runtime_proxy_->SubscribeEvents(
              [this, new_space_id](const nlohmann::json& event) {
                if (shell_cbs_.broadcast_event)
                  shell_cbs_.broadcast_event("event", event.dump());

                // (task 6.3) For file_edited, git_commit_created, git_pushed:
                // also write to the AppEvent bus so events.list/subscribe
                // picks them up in the channel panel.
                // Runtime events arrive as: { tag:"event", subscription, event:
                //   { kind, run_id, session_id, payload:{ ... } } }
                if (!event.contains("event") || !event["event"].is_object())
                  return;
                const auto& inner = event["event"];
                if (!inner.contains("payload") || !inner["payload"].is_object())
                  return;
                const auto& pl = inner["payload"];
                const std::string kind_str = pl.value("kind", std::string{});

                event_bus::AppEventKind target_kind;
                bool is_target = false;
                if (kind_str == "file_edited") {
                  target_kind = event_bus::AppEventKind::kFileEdited;
                  is_target = true;
                } else if (kind_str == "git_commit_created") {
                  target_kind = event_bus::AppEventKind::kGitCommitCreated;
                  is_target = true;
                } else if (kind_str == "git_pushed") {
                  target_kind = event_bus::AppEventKind::kGitPushed;
                  is_target = true;
                }

                if (!is_target)
                  return;

                Space* sp = nullptr;
                for (const auto& s : space_manager_->spaces()) {
                  if (s->id == new_space_id) {
                    sp = s.get();
                    break;
                  }
                }
                if (!sp || !sp->event_bus)
                  return;

                event_bus::AppEvent evt;
                evt.kind = target_kind;
                evt.space_id = new_space_id;
                evt.run_id = pl.value("run_id", std::string{});
                evt.session_id = pl.value("session_id", std::string{});

                // Build the payload from the Rust payload fields
                nlohmann::json payload_obj = nlohmann::json::object();
                if (kind_str == "file_edited") {
                  payload_obj["path"] = pl.value("path", std::string{});
                  payload_obj["diff"] = pl.value("diff", std::string{});
                  if (!evt.session_id.empty())
                    payload_obj["session_id"] = evt.session_id;
                } else if (kind_str == "git_commit_created") {
                  payload_obj["hash"] = pl.value("hash", std::string{});
                  payload_obj["message"] = pl.value("message", std::string{});
                  payload_obj["files_changed"] = pl.contains("files_changed")
                                                     ? pl["files_changed"]
                                                     : nlohmann::json::array();
                  if (!evt.session_id.empty())
                    payload_obj["session_id"] = evt.session_id;
                } else if (kind_str == "git_pushed") {
                  payload_obj["remote"] = pl.value("remote", std::string{});
                  payload_obj["branch"] = pl.value("branch", std::string{});
                  payload_obj["commits_pushed"] = pl.value("commits_pushed", 0);
                  if (!evt.session_id.empty())
                    payload_obj["session_id"] = evt.session_id;
                }
                evt.payload = std::move(payload_obj);
                sp->event_bus->Append(std::move(evt));
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
    for (const auto& s : v)
      arr.push_back(s);
    return arr;
  };
  return nlohmann::json{
      {"id", r.id},
      {"name", r.name},
      {"memory_id", r.memory_id},
      {"allow_network", r.allow_network},
      {"extra_read_paths", to_arr(r.extra_read_paths)},
      {"extra_write_paths", to_arr(r.extra_write_paths)},
      {"extra_deny_paths", to_arr(r.extra_deny_paths)},
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
    for (const auto& r : records)
      arr.push_back(ProfileRecordToJson(r));
    callback->Success(arr.dump());
    return true;
  }

  if (channel == "profiles.create") {
    if (!jp.is_object()) {
      callback->Failure(400, "payload must be an object");
      return true;
    }
    ProfileRules rules;
    rules.name = jp.value("name", std::string{});
    rules.memory_id = jp.value("memory_id", std::string{});
    rules.allow_network = jp.value("allow_network", true);
    if (rules.name.empty()) {
      callback->Failure(400, "name required");
      return true;
    }
    if (jp.contains("extra_read_paths") && jp["extra_read_paths"].is_array())
      for (const auto& p : jp["extra_read_paths"])
        if (p.is_string())
          rules.extra_read_paths.push_back(p);
    if (jp.contains("extra_write_paths") && jp["extra_write_paths"].is_array())
      for (const auto& p : jp["extra_write_paths"])
        if (p.is_string())
          rules.extra_write_paths.push_back(p);
    if (jp.contains("extra_deny_paths") && jp["extra_deny_paths"].is_array())
      for (const auto& p : jp["extra_deny_paths"])
        if (p.is_string())
          rules.extra_deny_paths.push_back(p);

    std::string new_id;
    const auto err = ps.Create(rules, &new_id);
    if (err == ProfileStoreError::kAlreadyExists) {
      callback->Failure(409, "profile name already exists");
      return true;
    }
    if (err == ProfileStoreError::kIoError) {
      callback->Failure(500, "I/O error writing profile");
      return true;
    }

    if (const auto rec = ps.Get(new_id)) {
      callback->Success(ProfileRecordToJson(*rec).dump());
    } else {
      callback->Success(nlohmann::json{{"id", new_id}}.dump());
    }
    return true;
  }

  if (channel == "profiles.update") {
    if (!jp.is_object()) {
      callback->Failure(400, "payload must be an object");
      return true;
    }
    const std::string id = jp.value("id", std::string{});
    if (id.empty()) {
      callback->Failure(400, "id required");
      return true;
    }
    ProfileRules rules;
    rules.name = jp.value("name", std::string{});
    rules.memory_id = jp.value("memory_id", std::string{});
    rules.allow_network = jp.value("allow_network", true);
    if (rules.name.empty()) {
      callback->Failure(400, "name required");
      return true;
    }
    if (jp.contains("extra_read_paths") && jp["extra_read_paths"].is_array())
      for (const auto& p : jp["extra_read_paths"])
        if (p.is_string())
          rules.extra_read_paths.push_back(p);
    if (jp.contains("extra_write_paths") && jp["extra_write_paths"].is_array())
      for (const auto& p : jp["extra_write_paths"])
        if (p.is_string())
          rules.extra_write_paths.push_back(p);
    if (jp.contains("extra_deny_paths") && jp["extra_deny_paths"].is_array())
      for (const auto& p : jp["extra_deny_paths"])
        if (p.is_string())
          rules.extra_deny_paths.push_back(p);

    const auto err = ps.Update(id, rules);
    if (err == ProfileStoreError::kNotFound) {
      callback->Failure(404, "profile not found");
      return true;
    }
    if (err == ProfileStoreError::kIoError) {
      callback->Failure(500, "I/O error writing profile");
      return true;
    }

    if (const auto rec = ps.Get(id)) {
      callback->Success(ProfileRecordToJson(*rec).dump());
    } else {
      callback->Success("{\"ok\":true}");
    }
    return true;
  }

  if (channel == "profiles.delete") {
    if (!jp.is_object()) {
      callback->Failure(400, "payload must be an object");
      return true;
    }
    const std::string id = jp.value("id", std::string{});
    if (id.empty()) {
      callback->Failure(400, "id required");
      return true;
    }

    // Collect the profile_id of every space so we can detect "in use".
    std::vector<std::string> space_profile_ids;
    for (const auto& s : space_manager_->spaces())
      space_profile_ids.push_back(s->profile_id);

    const auto err = ps.Delete(id, space_profile_ids);
    if (err == ProfileStoreError::kNotFound) {
      callback->Failure(404, "profile not found");
      return true;
    }
    if (err == ProfileStoreError::kCannotDeleteDefault) {
      callback->Failure(403, "cannot delete default profile");
      return true;
    }
    if (err == ProfileStoreError::kInUse) {
      callback->Failure(409, "profile is in use by one or more spaces");
      return true;
    }
    if (err == ProfileStoreError::kIoError) {
      callback->Failure(500, "I/O error deleting profile");
      return true;
    }

    callback->Success("{\"ok\":true}");
    return true;
  }

  if (channel == "profiles.check_paths") {
    // Accepts { paths: string[] }, returns { missing: string[] } — the subset
    // of input paths that do not exist as directories on the filesystem.
    const auto& jp = nlohmann::json::parse(payload, nullptr, false);
    if (!jp.is_object() || !jp.contains("paths") || !jp["paths"].is_array()) {
      callback->Failure(400, "paths array required");
      return true;
    }
    nlohmann::json missing = nlohmann::json::array();
    for (const auto& entry : jp["paths"]) {
      if (!entry.is_string())
        continue;
      const std::string p = entry.get<std::string>();
      if (p.empty())
        continue;
      std::error_code ec;
      if (!std::filesystem::exists(std::filesystem::path(p), ec)) {
        missing.push_back(p);
      }
    }
    callback->Success(nlohmann::json{{"missing", std::move(missing)}}.dump());
    return true;
  }

  callback->Failure(404, "unknown profiles channel");
  return true;
}

}  // namespace cronymax
