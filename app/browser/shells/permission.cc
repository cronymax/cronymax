// app/browser/shells/bridge_permission.cc
// permission.* channels, DeliverPermissionResponse, SetupCapabilityHandler.

#include "browser/bridge_handler.h"

#include <sys/wait.h>
#include <unistd.h>

#include <fstream>
#include <sstream>

#include <nlohmann/json.hpp>

#include "common/path_utils.h"
#include "common/types.h"

namespace cronymax {
namespace {

// Shell execution helper — runs `cmd` via /bin/sh -c in `cwd` and captures
// stdout/stderr.  Scope enforcement is the caller's responsibility.
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

}  // namespace

// ---------------------------------------------------------------------------
// RegisterPermissionHandlers — install browser.permission.* in BridgeRegistry.
// ---------------------------------------------------------------------------

void RegisterPermissionHandlers(BridgeRegistry& r, BridgeHandler* h) {
  r.add("browser.permission.respond", [h](BridgeCtx ctx) {
    const auto& j = ctx.payload;
    const std::string rid =
        j.is_object() ? j.value("request_id", std::string{}) : std::string{};
    const std::string dec =
        j.is_object() ? j.value("decision", std::string{}) : std::string{};
    const bool allow = (dec == "allow");

    // Check for a pending runtime capability reply first.
    {
      RuntimeProxy::CapabilityReplyFn reply_fn;
      {
        std::lock_guard<std::mutex> g(h->cap_reply_mu_);
        auto it = h->pending_cap_replies_.find(rid);
        if (it != h->pending_cap_replies_.end()) {
          reply_fn = std::move(it->second);
          h->pending_cap_replies_.erase(it);
        }
      }
      if (reply_fn) {
        nlohmann::json resp =
            allow ? nlohmann::json{{"outcome", "ok"}}
                  : nlohmann::json{{"outcome", "err"},
                                   {"error",
                                    {{"code", "denied"},
                                     {"message", "user denied permission"}}}};
        reply_fn(std::move(resp));
        ctx.callback->Success(nlohmann::json{{"ok", true}});
        return;
      }
    }

    // Fallback: legacy in-process permission delivery.
    h->DeliverPermissionResponse(rid, allow);
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });
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
  runtime_proxy_->SetCapabilityHandler(
      [this](const std::string& corr_id, const nlohmann::json& request,
             RuntimeProxy::CapabilityReplyFn reply) {
        const std::string cap = request.value("capability", std::string{});
        const std::string space_id = request.value("space_id", std::string{});

        // Resolve the owning Space for scope enforcement.
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
            // Scope enforcement: cwd must be within workspace_root.
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
            reply({{"outcome", "err"},
                   {"error",
                    {{"code", "bad_request"}, {"message", "empty argv"}}}});
            return;
          }
          // Block execution of sensitive system paths.
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
            reply(
                {{"outcome", "err"},
                 {"error",
                  {{"code", "no_space"},
                   {"message", "no active space for filesystem capability"}}}});
            return;
          }
          const auto& op = request.value("op", nlohmann::json{});
          const std::string op_kind = op.value("kind", std::string{});

          auto scope_check = [&](const std::string& path_str) -> bool {
            if (path_str.empty())
              return false;
            std::filesystem::path p(path_str);
            if (IsSensitivePath(p))
              return false;
            std::error_code ec;
            auto rel = std::filesystem::relative(p, workspace_root, ec);
            return !ec && !rel.empty() && rel.native().substr(0, 2) != "..";
          };

          if (op_kind == "read") {
            const std::string path_str = op.value("path", std::string{});
            if (!scope_check(path_str)) {
              reply({{"outcome", "err"},
                     {"error",
                      {{"code", "scope_violation"},
                       {"message",
                        "path is outside workspace root or sensitive"}}}});
              return;
            }
            std::ifstream f(path_str, std::ios::binary);
            if (!f) {
              reply({{"outcome", "err"},
                     {"error",
                      {{"code", "not_found"},
                       {"message", "file not found: " + path_str}}}});
              return;
            }
            std::ostringstream ss;
            ss << f.rdbuf();
            reply({{"outcome", "ok"}, {"content", ss.str()}});
            return;
          }

          if (op_kind == "write") {
            const std::string path_str = op.value("path", std::string{});
            if (!scope_check(path_str)) {
              reply({{"outcome", "err"},
                     {"error",
                      {{"code", "scope_violation"},
                       {"message",
                        "path is outside workspace root or sensitive"}}}});
              return;
            }
            const std::string content = op.value("content", std::string{});
            // Ensure parent directory exists.
            std::filesystem::path p(path_str);
            if (p.has_parent_path()) {
              std::error_code ec;
              std::filesystem::create_directories(p.parent_path(), ec);
              if (ec) {
                reply({{"outcome", "err"},
                       {"error",
                        {{"code", "io_error"},
                         {"message",
                          "failed to create directory: " + ec.message()}}}});
                return;
              }
            }
            std::ofstream out(path_str, std::ios::out | std::ios::trunc);
            if (!out) {
              reply({{"outcome", "err"},
                     {"error",
                      {{"code", "io_error"},
                       {"message",
                        "failed to open file for writing: " + path_str}}}});
              return;
            }
            out << content;
            out.close();
            reply({{"outcome", "ok"}});
            return;
          }

          reply({{"outcome", "err"},
                 {"error",
                  {{"code", "bad_request"},
                   {"message", "unknown filesystem op: " + op_kind}}}});
          return;
        }

        // ── notify ───────────────────────────────────────────────────────
        if (cap == "notify") {
          if (shell_cbs_.broadcast_event) {
            nlohmann::json evt = {
                {"title", request.value("title", std::string{})},
                {"body", request.value("body", std::string{})},
            };
            shell_cbs_.broadcast_event("notification", evt.dump());
          }
          reply({{"outcome", "ok"}});
          return;
        }

        // ── browser / secret — not yet implemented ───────────────────────
        reply({{"outcome", "err"},
               {"error",
                {{"code", "not_implemented"},
                 {"message", "capability not supported: " + cap}}}});
      });
}

}  // namespace cronymax
