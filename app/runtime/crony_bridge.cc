// runtime_bridge.cc — implementation of RuntimeBridge.
//
// See runtime_bridge.h for the full design rationale.
//
// Concurrency model recap:
//   * pump_thread_      — calls crony_client_recv in a tight loop; delivers
//     payloads to subscribers; exits when pump_stop_ is set or recv returns
//     CRONY_ERR_CLOSED/error.
//   * supervisor_thread_ — waits for the child PID to exit; if the bridge is
//     still meant to be running it triggers a restart (up to
//     kMaxRestartAttempts); sets status kFailed on giving up.
//   * Caller threads    — call Invoke() (serialized by send_mu_) and
//     Subscribe()/Unsubscribe().
//   * mu_               — guards status_, last_error_, client_, child_pid_,
//     runtime_binary_, service_name_, restart_count_.

#include "runtime/crony_bridge.h"

#include "runtime/layout_migrator.h"
#include "runtime/workspace_id.h"

#include <chrono>
#include <cstdlib>
#include <cstring>
#include <sstream>

#if defined(_WIN32)
#include <windows.h>
#elif defined(__APPLE__)
#include <mach-o/dyld.h>
#include <sys/wait.h>
#include <unistd.h>
#include <climits>
#include <csignal>
#else
#include <sys/wait.h>
#include <unistd.h>
#include <csignal>
#endif

#include <nlohmann/json.hpp>

// crony.h is brought in via the Cronymax::Crony interface include dir.
// It is already included transitively through runtime_bridge.h.

namespace cronymax {

namespace {

// ---------------------------------------------------------------------------
// GIPS service name the runtime child advertises.
// Must match boundary.rs DEFAULT_SERVICE_NAME.
// ---------------------------------------------------------------------------
constexpr const char* kDefaultServiceName = "ai.cronymax.runtime";

// How long to wait for the Hello/Welcome handshake before giving up.
constexpr auto kHandshakeTimeout = std::chrono::seconds(10);

// Small sleep between supervisor poll iterations when waiting for the child to
// appear in a failed state (avoids busy-wait while the child boots).
constexpr auto kSupervisorPollMs = std::chrono::milliseconds(200);

// ---------------------------------------------------------------------------
// Platform: spawn a child process.
// Returns true on success, fills in *pid / *handle.
// ---------------------------------------------------------------------------
#if defined(_WIN32)

bool SpawnProcess(const std::filesystem::path& binary,
                  const std::string& service_arg,
                  HANDLE* out_handle) {
  std::string cmd =
      "\"" + binary.string() + "\" --service \"" + service_arg + "\"";
  STARTUPINFOA si{};
  si.cb = sizeof(si);
  PROCESS_INFORMATION pi{};
  if (!CreateProcessA(nullptr, cmd.data(), nullptr, nullptr, FALSE, 0, nullptr,
                      nullptr, &si, &pi)) {
    return false;
  }
  CloseHandle(pi.hThread);
  *out_handle = pi.hProcess;
  return true;
}

void KillProcess(HANDLE handle) {
  if (handle && handle != INVALID_HANDLE_VALUE) {
    TerminateProcess(handle, 1);
    WaitForSingleObject(handle, 3000);
    CloseHandle(handle);
  }
}

// Returns true once the process has exited.
bool WaitForProcessExit(HANDLE handle, DWORD timeout_ms) {
  return WaitForSingleObject(handle, timeout_ms) == WAIT_OBJECT_0;
}

#else  // POSIX

bool SpawnProcess(const std::filesystem::path& binary,
                  const std::string& config_json,
                  int* out_pid) {
  // Create a pipe: the child reads its RuntimeConfig from the read end.
  int pipe_fds[2];
  if (::pipe(pipe_fds) != 0)
    return false;

  pid_t pid = fork();
  if (pid < 0) {
    close(pipe_fds[0]);
    close(pipe_fds[1]);
    return false;
  }
  if (pid == 0) {
    // Child: redirect stdin to the read end of the pipe.
    dup2(pipe_fds[0], STDIN_FILENO);
    close(pipe_fds[0]);
    close(pipe_fds[1]);
    const char* args[] = {binary.c_str(), nullptr};
    execv(binary.c_str(), const_cast<char* const*>(args));
    _exit(1);  // exec failed
  }

  // Parent: close read end, write config JSON, then close write end so
  // the child's read_to_string() sees EOF and proceeds.
  close(pipe_fds[0]);
  const char* p = config_json.data();
  ssize_t remaining = static_cast<ssize_t>(config_json.size());
  while (remaining > 0) {
    ssize_t n = write(pipe_fds[1], p, static_cast<size_t>(remaining));
    if (n <= 0)
      break;
    p += n;
    remaining -= n;
  }
  close(pipe_fds[1]);

  *out_pid = static_cast<int>(pid);
  return true;
}

void KillProcess(int pid) {
  if (pid > 0) {
    kill(pid, SIGTERM);
    // Give it 3 s to exit cleanly before forcing.
    for (int i = 0; i < 30; ++i) {
      std::this_thread::sleep_for(std::chrono::milliseconds(100));
      int status = 0;
      if (waitpid(pid, &status, WNOHANG) == pid)
        return;
    }
    kill(pid, SIGKILL);
    waitpid(pid, nullptr, 0);
  }
}

// Blocks until the child exits. Returns the wait status.
int WaitForProcessExit(int pid) {
  int status = 0;
  waitpid(pid, &status, 0);
  return status;
}

#endif  // platform

// ---------------------------------------------------------------------------
// Locate the crony binary.
//
// Search order (first existing path wins):
//   1. hint (passed from tests or command-line override)
//   2. Same directory as the hint path (if hint is a directory)
//   3. On POSIX: /proc/self/exe parent   — Linux dev builds
//   4. On macOS:  derived from __FILE__ at compile time (fallback only)
//   5. Relative to CWD                   — last resort for dev builds
// ---------------------------------------------------------------------------
std::filesystem::path FindRuntimeBinary(const std::filesystem::path& hint) {
  const char* binary_name =
#if defined(_WIN32)
      "crony.exe";
#else
      "crony";
#endif

  if (!hint.empty()) {
    auto candidate = hint / binary_name;
    if (std::filesystem::exists(candidate))
      return candidate;
    // hint might already BE the binary path.
    if (std::filesystem::exists(hint))
      return hint;
  }

#if defined(__linux__)
  {
    std::error_code ec;
    auto self = std::filesystem::read_symlink("/proc/self/exe", ec);
    if (!ec) {
      auto c = self.parent_path() / binary_name;
      if (std::filesystem::exists(c))
        return c;
      // ../Frameworks/ for any bundle layout that keeps linux helpers there
      auto c2 = self.parent_path() / ".." / "Frameworks" / binary_name;
      std::error_code ec2;
      auto c2c = std::filesystem::canonical(c2, ec2);
      if (!ec2 && std::filesystem::exists(c2c))
        return c2c;
    }
  }
#elif defined(__APPLE__)
  // On macOS the executable is Contents/MacOS/<name>; the runtime binary is
  // bundled at Contents/Frameworks/crony.
  // Use _NSGetExecutablePath to get the real executable path independent of
  // CWD.
  {
    char exe_buf[PATH_MAX];
    uint32_t exe_size = sizeof(exe_buf);
    if (_NSGetExecutablePath(exe_buf, &exe_size) == 0) {
      std::error_code ec;
      auto exe_path = std::filesystem::canonical(exe_buf, ec);
      if (!ec) {
        // Contents/MacOS/<exe> → ../Frameworks/<binary_name>
        auto c = exe_path.parent_path() / ".." / "Frameworks" / binary_name;
        auto cc = std::filesystem::weakly_canonical(c, ec);
        if (!ec && std::filesystem::exists(cc))
          return cc;
        // Also check same directory as executable (non-bundle dev layout).
        auto c2 = exe_path.parent_path() / binary_name;
        if (std::filesystem::exists(c2))
          return c2;
      }
    }
  }
#endif

  // CWD fallback.
  auto cwd_candidate = std::filesystem::current_path() / binary_name;
  if (std::filesystem::exists(cwd_candidate))
    return cwd_candidate;

  return {};
}

}  // namespace

// ---------------------------------------------------------------------------
// RuntimeBridgeStatusToString
// ---------------------------------------------------------------------------

const char* RuntimeBridgeStatusToString(RuntimeBridgeStatus s) {
  switch (s) {
    case RuntimeBridgeStatus::kStopped:
      return "stopped";
    case RuntimeBridgeStatus::kStarting:
      return "starting";
    case RuntimeBridgeStatus::kReady:
      return "ready";
    case RuntimeBridgeStatus::kRestarting:
      return "restarting";
    case RuntimeBridgeStatus::kFailed:
      return "failed";
  }
  return "unknown";
}

// ---------------------------------------------------------------------------
// RuntimeBridge
// ---------------------------------------------------------------------------

RuntimeBridge::RuntimeBridge() = default;

RuntimeBridge::~RuntimeBridge() {
  Stop();
}

// ---------- Start ----------

bool RuntimeBridge::Start(const std::filesystem::path& runtime_dir,
                          const std::filesystem::path& app_data_dir) {
  std::unique_lock lock(mu_);
  if (status_ == RuntimeBridgeStatus::kReady ||
      status_ == RuntimeBridgeStatus::kStarting) {
    return status_ == RuntimeBridgeStatus::kReady;
  }
  status_ = RuntimeBridgeStatus::kStarting;
  last_error_.clear();
  restart_count_ = 0;

  runtime_binary_ = FindRuntimeBinary(runtime_dir);
  if (!app_data_dir.empty()) {
    app_data_dir_ = app_data_dir;
  }
  if (runtime_binary_.empty()) {
    last_error_ = "crony binary not found";
    status_ = RuntimeBridgeStatus::kFailed;
    return false;
  }

  service_name_ = kDefaultServiceName;
  lock.unlock();

  // Run the one-shot layout migration before spawning the runtime.
  // This moves the legacy flat layout ($appDataDir/runtime-state.json) into
  // the profile-scoped layout ($appDataDir/profiles/default/) if needed.
  if (!app_data_dir_.empty()) {
    LayoutMigrator migrator(app_data_dir_);
    migrator
        .Run();  // non-fatal if it fails; runtime will start with empty state
  }

  if (!SpawnAndHandshake()) {
    return false;
  }

  // Start the supervisor (monitors child exit).
  supervisor_stop_.store(false);
  supervisor_thread_ = std::thread(&RuntimeBridge::SupervisorLoop, this);

  return true;
}

// Internal: spawn + handshake + start pump. Called from Start() and the
// supervisor restart path. mu_ must NOT be held on entry.
bool RuntimeBridge::SpawnAndHandshake() {
  std::filesystem::path bin;
  std::filesystem::path app_data;
  std::string profile_id;
  std::string memory_id;
  std::filesystem::path workspace_root;
  {
    std::lock_guard lock(mu_);
    bin = runtime_binary_;
    app_data = app_data_dir_;
    profile_id = profile_id_.empty() ? "default" : profile_id_;
    memory_id = memory_id_.empty() ? profile_id : memory_id_;
    workspace_root = workspace_root_;
  }

  // Kill any stale runtime from a previous crashed session.  If an old
  // crony is still alive and bound to the GIPS service, a new
  // handshake would fail with "Hello sent twice".  We terminate it
  // gracefully (SIGTERM, then SIGKILL) before spawning our own child.
#if !defined(_WIN32)
  {
    FILE* fp = popen("pgrep -x crony", "r");
    if (fp) {
      char buf[32];
      bool killed_any = false;
      while (fgets(buf, sizeof(buf), fp)) {
        int stale_pid = std::atoi(buf);
        if (stale_pid > 0) {
          ::kill(stale_pid, SIGTERM);
          killed_any = true;
        }
      }
      pclose(fp);
      if (killed_any) {
        // Give stale process time to exit cleanly before we try to bind
        // the GIPS service name.
        std::this_thread::sleep_for(std::chrono::milliseconds(400));
      }
    }
  }
#endif

  // Build the RuntimeConfig JSON that is piped to the child's stdin.
  // Storage layout:
  //   $appDataDir/<profile_id>/                     — CEF webview cache
  //   $appDataDir/cronymax/Profiles/<profile_id>/  — runtime profile data
  //   $appDataDir/cronymax/Memories/<memory_id>/   — runtime memory caches
  //   $appDataDir/cronymax/logs/                    — application logs
  std::error_code ec;
  const auto cronymax_dir = app_data / "cronymax";
  const auto profile_data = cronymax_dir / "Profiles" / profile_id;
  const auto profile_memory = cronymax_dir / "Memories" / memory_id;
  const auto webview_profile = app_data / profile_id;
  const auto logs_dir = cronymax_dir / "logs";
  const auto ws_id = workspace_root.empty() ? std::string("default")
                                            : WorkspaceId(workspace_root);
  const auto ws_cache = profile_data / "workspaces" / ws_id;

  // Ensure all required subdirectories exist before spawning.
  std::filesystem::create_directories(profile_data, ec);
  std::filesystem::create_directories(profile_memory, ec);
  std::filesystem::create_directories(logs_dir, ec);
  std::filesystem::create_directories(webview_profile, ec);
  std::filesystem::create_directories(ws_cache / "chats", ec);
  std::filesystem::create_directories(ws_cache / "pty", ec);
  std::filesystem::create_directories(profile_data / "cache", ec);

  nlohmann::json cfg;
  cfg["storage"]["workspace_roots"] = nlohmann::json::array();
  if (!workspace_root.empty())
    cfg["storage"]["workspace_roots"].push_back(workspace_root.string());
  cfg["storage"]["app_data_dir"] = profile_data.string();
  cfg["storage"]["workspace_cache_dir"] = ws_cache.string();
  cfg["storage"]["cache_dir"] = (profile_data / "cache").string();
  cfg["logging"]["log_dir"] = logs_dir.string();
  cfg["logging"]["filter"] = nullptr;
  cfg["host_protocol"]["major"] = 0;
  cfg["host_protocol"]["minor"] = 1;
  cfg["host_protocol"]["patch"] = 0;

  // Inject sandbox config if one has been set via SetSandboxConfig().
  {
    std::lock_guard lock(mu_);
    if (!sandbox_config_.is_null()) {
      cfg["sandbox"] = sandbox_config_;
    }
  }

  const std::string config_json = cfg.dump();

  // Spawn with config piped to stdin.
  if (!SpawnChild(bin, config_json)) {
    std::lock_guard lock(mu_);
    last_error_ = "failed to spawn crony";
    status_ = RuntimeBridgeStatus::kFailed;
    return false;
  }

  if (!WaitForHandshake()) {
    KillChild();
    return false;
  }

  // Start pump thread.
  pump_stop_.store(false);
  pump_thread_ = std::thread(&RuntimeBridge::PumpLoop, this);

  {
    std::lock_guard lock(mu_);
    status_ = RuntimeBridgeStatus::kReady;
  }
  return true;
}

// ---------- Stop ----------

void RuntimeBridge::Stop() {
  // Signal the supervisor to exit before the pump so it doesn't
  // trigger a restart after we close the client.
  supervisor_stop_.store(true);
  if (supervisor_thread_.joinable()) {
    supervisor_thread_.join();
  }

  // Signal the pump and close the gips handle (makes recv return
  // CRONY_ERR_CLOSED so the pump exits cleanly).
  pump_stop_.store(true);
  {
    std::lock_guard lock(mu_);
    if (client_) {
      crony_client_close(client_);
      client_ = nullptr;
    }
  }
  if (pump_thread_.joinable()) {
    pump_thread_.join();
  }

  KillChild();

  std::lock_guard lock(mu_);
  status_ = RuntimeBridgeStatus::kStopped;
}

// ---------- Invoke ----------

bool RuntimeBridge::Invoke(const std::string& json_envelope) {
  crony_client_t* c = nullptr;
  {
    std::lock_guard lock(mu_);
    if (status_ != RuntimeBridgeStatus::kReady || !client_) {
      last_error_ = "bridge not ready (status=" +
                    std::string(RuntimeBridgeStatusToString(status_)) + ")";
      return false;
    }
    c = client_;
  }

  std::lock_guard send_lock(send_mu_);
  char* err = nullptr;
  int rc = crony_client_send(
      c, reinterpret_cast<const uint8_t*>(json_envelope.data()),
      json_envelope.size(), &err);
  if (rc != CRONY_OK) {
    std::string msg = err ? err : "(gips send error)";
    crony_string_free(err);
    std::lock_guard lock(mu_);
    last_error_ = "send failed: " + msg;
    return false;
  }
  return true;
}

// ---------- Subscribe / Unsubscribe ----------

int64_t RuntimeBridge::Subscribe(RuntimeMessageCallback callback) {
  std::lock_guard lock(sub_mu_);
  int64_t tok = next_token_++;
  subscribers_.emplace_back(tok, std::move(callback));
  fprintf(stderr, "[RuntimeBridge::Subscribe] tok=%lld total_subs=%zu\n",
          (long long)tok, subscribers_.size());
  fflush(stderr);
  return tok;
}

void RuntimeBridge::Unsubscribe(int64_t token) {
  std::lock_guard lock(sub_mu_);
  subscribers_.erase(
      std::remove_if(subscribers_.begin(), subscribers_.end(),
                     [token](const auto& p) { return p.first == token; }),
      subscribers_.end());
}

// ---------- Diagnostics ----------

RuntimeBridgeStatus RuntimeBridge::Status() const {
  std::lock_guard lock(mu_);
  return status_;
}

std::string RuntimeBridge::LastError() const {
  std::lock_guard lock(mu_);
  return last_error_;
}

// ---------------------------------------------------------------------------
// Private — binary discovery
// ---------------------------------------------------------------------------

std::filesystem::path RuntimeBridge::LocateRuntimeBinary(
    const std::filesystem::path& hint) {
  return FindRuntimeBinary(hint);
}

// ---------------------------------------------------------------------------
// Private — process management
// ---------------------------------------------------------------------------

bool RuntimeBridge::SpawnChild(const std::filesystem::path& binary_path,
                               const std::string& config_json) {
#if defined(_WIN32)
  std::string svc;
  {
    std::lock_guard lock(mu_);
    svc = service_name_;
  }
  HANDLE handle = nullptr;
  if (!SpawnProcess(binary_path, svc, &handle))
    return false;
  std::lock_guard lock(mu_);
  child_process_ = handle;
#else
  int pid = -1;
  if (!SpawnProcess(binary_path, config_json, &pid))
    return false;
  std::lock_guard lock(mu_);
  child_pid_ = pid;
#endif
  return true;
}

void RuntimeBridge::KillChild() {
#if defined(_WIN32)
  HANDLE h = nullptr;
  {
    std::lock_guard lock(mu_);
    h = static_cast<HANDLE>(child_process_);
    child_process_ = nullptr;
  }
  if (h)
    KillProcess(h);
#else
  int pid = -1;
  {
    std::lock_guard lock(mu_);
    pid = child_pid_;
    child_pid_ = -1;
  }
  if (pid > 0)
    KillProcess(pid);
#endif
}

// ---------------------------------------------------------------------------
// Private — handshake
// ---------------------------------------------------------------------------

bool RuntimeBridge::WaitForHandshake() {
  std::string svc;
  {
    std::lock_guard lock(mu_);
    svc = service_name_;
  }

  // Poll for the child to register its GIPS service. The runtime binary
  // needs to parse its config from stdin, initialise the Rust async
  // executor, and call GipsTransport::bind_default() before the service
  // appears in the Mach bootstrap namespace. Retry for up to
  // kHandshakeTimeout instead of giving up after a single attempt.
  const auto deadline = std::chrono::steady_clock::now() + kHandshakeTimeout;
  crony_client_t* c = nullptr;
  while (!c && std::chrono::steady_clock::now() < deadline) {
    char* err = nullptr;
    c = crony_client_new(svc.c_str(), &err);
    if (!c) {
      crony_string_free(err);
      std::this_thread::sleep_for(std::chrono::milliseconds(200));
    }
  }
  if (!c) {
    std::lock_guard lock(mu_);
    last_error_ = "gips connect failed: timeout waiting for runtime service";
    status_ = RuntimeBridgeStatus::kFailed;
    return false;
  }

  // Build Hello envelope.
  // ClientToRuntime uses #[serde(tag = "tag", rename_all = "snake_case")],
  // so the internally-tagged format is {"tag":"hello", "protocol":…, …}.
  nlohmann::json hello;
  hello["tag"] = "hello";
  hello["protocol"]["major"] = 0;
  hello["protocol"]["minor"] = 1;
  hello["protocol"]["patch"] = 0;
  hello["client_name"] = "cronymax-host";
  hello["client_version"] = "0.0.0";
  std::string hello_str = hello.dump();

  char* err = nullptr;
  int rc =
      crony_client_send(c, reinterpret_cast<const uint8_t*>(hello_str.data()),
                        hello_str.size(), &err);
  if (rc != CRONY_OK) {
    std::string msg = err ? err : "(send error)";
    crony_string_free(err);
    crony_client_close(c);
    std::lock_guard lock(mu_);
    last_error_ = "handshake send failed: " + msg;
    status_ = RuntimeBridgeStatus::kFailed;
    return false;
  }

  // Wait for Welcome.
  uint8_t* buf = nullptr;
  size_t len = 0;
  err = nullptr;
  rc = crony_client_recv(c, &buf, &len, &err);
  if (rc != CRONY_OK) {
    std::string msg = err ? err : "(recv error)";
    crony_string_free(err);
    crony_client_close(c);
    std::lock_guard lock(mu_);
    last_error_ = "handshake recv failed: " + msg;
    status_ = RuntimeBridgeStatus::kFailed;
    return false;
  }

  std::string resp(reinterpret_cast<char*>(buf), len);
  crony_bytes_free(buf, len);

  try {
    auto j = nlohmann::json::parse(resp);
    if (j.value("tag", "") != "welcome") {
      crony_client_close(c);
      std::lock_guard lock(mu_);
      last_error_ = "handshake: unexpected reply: " + resp;
      status_ = RuntimeBridgeStatus::kFailed;
      return false;
    }
  } catch (const nlohmann::json::exception& ex) {
    crony_client_close(c);
    std::lock_guard lock(mu_);
    last_error_ =
        std::string("handshake: malformed Welcome JSON: ") + ex.what();
    status_ = RuntimeBridgeStatus::kFailed;
    return false;
  }

  std::lock_guard lock(mu_);
  client_ = c;
  return true;
}

// ---------------------------------------------------------------------------
// Private — recv pump
// ---------------------------------------------------------------------------

void RuntimeBridge::PumpLoop() {
  while (!pump_stop_.load()) {
    crony_client_t* c = nullptr;
    {
      std::lock_guard lock(mu_);
      c = client_;
    }
    if (!c)
      break;

    uint8_t* buf = nullptr;
    size_t len = 0;
    char* err = nullptr;
    // Use try_recv (non-blocking) so we never hold the endpoint lock
    // long enough to starve concurrent crony_client_send calls on other
    // threads. On CRONY_ERR_WOULD_BLOCK we sleep briefly and retry.
    int rc = crony_client_try_recv(c, &buf, &len, &err);

    if (rc == CRONY_ERR_CLOSED || pump_stop_.load()) {
      crony_string_free(err);
      break;
    }
    if (rc == CRONY_ERR_WOULD_BLOCK) {
      // No message ready yet — yield and poll again.
      std::this_thread::sleep_for(std::chrono::milliseconds(5));
      continue;
    }
    if (rc != CRONY_OK) {
      // Transient recv error — log and continue unless pump is stopping.
      crony_string_free(err);
      if (!pump_stop_.load()) {
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
      }
      continue;
    }

    std::string payload(reinterpret_cast<char*>(buf), len);
    crony_bytes_free(buf, len);

    DispatchPayload(payload);
  }
}

void RuntimeBridge::DispatchPayload(const std::string& payload) {
  std::lock_guard lock(sub_mu_);
  for (auto& [tok, cb] : subscribers_) {
    cb(payload);
  }
}

// ---------------------------------------------------------------------------
// Private — supervisor
// ---------------------------------------------------------------------------

void RuntimeBridge::SupervisorLoop() {
  while (!supervisor_stop_.load()) {
    // Wait for the child to exit.
#if defined(_WIN32)
    HANDLE h = nullptr;
    {
      std::lock_guard lock(mu_);
      h = static_cast<HANDLE>(child_process_);
    }
    if (!h || WaitForProcessExit(h, 200)) {
      // Child has exited or handle is gone.
      if (h) {
        std::lock_guard lock(mu_);
        CloseHandle(h);
        child_process_ = nullptr;
      }
#else
    int pid = -1;
    {
      std::lock_guard lock(mu_);
      pid = child_pid_;
    }
    if (pid > 0) {
      // Non-blocking check: has it exited?
      int status = 0;
      pid_t result = waitpid(pid, &status, WNOHANG);
      if (result == pid) {
        {
          std::lock_guard lock(mu_);
          child_pid_ = -1;
        }
#endif
      // Child exited while we're still meant to be running.
      if (supervisor_stop_.load())
        break;

      int attempts = 0;
      {
        std::lock_guard lock(mu_);
        if (status_ == RuntimeBridgeStatus::kStopped)
          break;
        attempts = ++restart_count_;
        if (attempts > kMaxRestartAttempts) {
          last_error_ = "runtime crashed too many times; giving up";
          status_ = RuntimeBridgeStatus::kFailed;
          break;
        }
        status_ = RuntimeBridgeStatus::kRestarting;
      }

      // Tear down the old pump and client before respawning.
      pump_stop_.store(true);
      {
        std::lock_guard lock(mu_);
        if (client_) {
          crony_client_close(client_);
          client_ = nullptr;
        }
      }
      if (pump_thread_.joinable())
        pump_thread_.join();

      // Notify subscribers (e.g. RuntimeProxy) that the runtime is about to
      // restart.  RuntimeProxy::HandleBridgeRestarting() drains its pending_
      // callbacks with errors so renderer Promises reject instead of hanging.
      DispatchPayload(R"({"tag":"bridge_restarting"})");

      if (!supervisor_stop_.load()) {
        SpawnAndHandshake();
      }
#if defined(_WIN32)
    }
  }
#else
      }  // waitpid matched
    }  // pid > 0
#endif

  std::this_thread::sleep_for(kSupervisorPollMs);
}
}

}  // namespace cronymax
