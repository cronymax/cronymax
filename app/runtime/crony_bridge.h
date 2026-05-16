#pragma once

// crony_bridge.h — C++ side of the Rust runtime GIPS bridge.
//
// Implements tasks 1.4 and 1.5 of the `rust-runtime-cpp-cutover` change.
//
// RuntimeBridge owns two responsibilities:
//
//   1. RuntimeSupervisor   — finds the `crony` binary, spawns it as
//      a child process, performs the Hello/Welcome handshake, restarts it after
//      unexpected exit, and stops it during app shutdown.
//
//   2. RuntimeProxy        — owns a `crony_client_t` handle, a send mutex so
//      request threads don't interleave frames, and a recv pump thread that
//      dispatches incoming RuntimeToClient envelopes to registered
//      subscribers. Bridge handlers call Invoke() and Subscribe(); the pump
//      thread delivers replies and events.
//
// Usage (from MainWindow or AppDelegate):
//
//   RuntimeBridge bridge;
//   bridge.Start();                    // spawns child + handshake
//   bridge.Subscribe([](auto& env) {}); // register fanout callback
//   bridge.Invoke(envelope);           // blocking send
//   bridge.Stop();                     // clean shutdown
//
// Thread safety:
//   Start() and Stop() must be called on the same thread. Invoke() and
//   Subscribe() are safe to call from any thread after Start() returns.

#include <atomic>
#include <filesystem>
#include <functional>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include "crony.h"  // provided via Cronymax::Crony INTERFACE_INCLUDE_DIRECTORIES
#include "nlohmann/json.hpp"

namespace cronymax {

// ---------------------------------------------------------------------------
// RuntimeBridgeStatus
// ---------------------------------------------------------------------------

enum class RuntimeBridgeStatus {
  kStopped,     // initial / after Stop()
  kStarting,    // binary found, child spawned, handshake in progress
  kReady,       // handshake complete, Invoke() accepted
  kRestarting,  // child exited unexpectedly; respawn in progress
  kFailed,      // unrecoverable: binary not found, or repeated crash
};

const char* RuntimeBridgeStatusToString(RuntimeBridgeStatus s);

// ---------------------------------------------------------------------------
// RuntimeBridgeObserver
// ---------------------------------------------------------------------------

// Subscribers receive raw JSON payloads (RuntimeToClient envelopes). The
// callback fires from the recv pump thread — implementations must be
// thread-safe.
using RuntimeMessageCallback = std::function<void(std::string payload)>;

// ---------------------------------------------------------------------------
// RuntimeBridge
// ---------------------------------------------------------------------------

class RuntimeBridge {
 public:
  // Maximum number of consecutive child crashes before entering kFailed.
  static constexpr int kMaxRestartAttempts = 5;

  RuntimeBridge();
  ~RuntimeBridge();

  RuntimeBridge(const RuntimeBridge&) = delete;
  RuntimeBridge& operator=(const RuntimeBridge&) = delete;

  // ---------- lifecycle ----------

  // Locate the `cronymax-runtime` binary, spawn it, and perform the
  // Hello/Welcome handshake. Blocks until the handshake completes or fails.
  //
  // Returns true on success (status becomes kReady). On failure, status
  // becomes kFailed and the reason is available via LastError().
  //
  // `runtime_dir` overrides binary search (pass {} to auto-discover).
  // `app_data_dir` is the runtime's persistent-state root; used to build
  // the RuntimeConfig JSON passed to the child via stdin.
  bool Start(const std::filesystem::path& runtime_dir = {},
             const std::filesystem::path& app_data_dir = {});

  // Stop the recv pump, terminate the child process, and release the GIPS
  // handle. Blocks until the pump thread has exited. Safe to call from any
  // state; idempotent if already stopped.
  void Stop();

  // ---------- request side ----------

  // JSON-encode and send a ClientToRuntime envelope to the runtime. Thread
  // safe. Returns false (and sets LastError) if the bridge is not in kReady
  // state or if gips send fails.
  bool Invoke(const std::string& json_envelope);

  // ---------- event subscription ----------

  // Register a callback that receives every incoming RuntimeToClient JSON
  // payload. Returns a token that can be passed to Unsubscribe(). Thread safe.
  int64_t Subscribe(RuntimeMessageCallback callback);

  // Remove a subscription by token. Thread safe.
  void Unsubscribe(int64_t token);

  // Set the active profile context. Combined with workspace_root_, this
  // determines profile-scoped runtime/memory/cache paths in RuntimeConfig.
  // Thread safe; must be called before Start() or Stop()+Start() to take
  // effect.
  void SetProfileContext(std::string profile_id,
                         std::string memory_id,
                         std::filesystem::path workspace_root) {
    std::lock_guard<std::mutex> lock(mu_);
    profile_id_ = std::move(profile_id);
    memory_id_ = std::move(memory_id);
    workspace_root_ = std::move(workspace_root);
  }

  // Set the sandbox configuration that will be included in the next
  // RuntimeConfig JSON handed to the child process via stdin.
  // Thread safe; must be called before Start() or Stop()+Start() to take
  // effect.
  void SetSandboxConfig(nlohmann::json config) {
    std::lock_guard<std::mutex> lock(mu_);
    sandbox_config_ = std::move(config);
  }

  // ---------- diagnostics ----------

  RuntimeBridgeStatus Status() const;
  std::string LastError() const;

 private:
  // ---------- binary discovery ----------
  static std::filesystem::path LocateRuntimeBinary(
      const std::filesystem::path& hint);

  // ---------- process management ----------
  bool SpawnChild(const std::filesystem::path& binary_path,
                  const std::string& config_json);
  bool SpawnAndHandshake();  // spawn + handshake + start pump; called from
                             // Start() and supervisor
  void KillChild();
  bool WaitForHandshake();

  // ---------- recv pump ----------
  void PumpLoop();  // runs on pump_thread_
  void DispatchPayload(const std::string& payload);

  // ---------- supervisor ----------
  void SupervisorLoop();  // runs on supervisor_thread_

  // ---------- state ----------
  mutable std::mutex mu_;
  RuntimeBridgeStatus status_ = RuntimeBridgeStatus::kStopped;
  std::string last_error_;

  crony_client_t* client_ = nullptr;

  // Serialize concurrent Invoke() calls so send frames don't interleave.
  std::mutex send_mu_;

  // Native child process handle.
#if defined(_WIN32)
  void* child_process_ = nullptr;  // HANDLE; avoids <windows.h> here
#else
  int child_pid_ = -1;
#endif

  // Recv pump.
  std::thread pump_thread_;
  std::atomic<bool> pump_stop_{false};

  // Supervisor (monitors child exit and triggers restart).
  std::thread supervisor_thread_;
  std::atomic<bool> supervisor_stop_{false};

  // Subscription fanout.
  std::mutex sub_mu_;
  int64_t next_token_ = 1;
  std::vector<std::pair<int64_t, RuntimeMessageCallback>> subscribers_;

  // Restart accounting.
  int restart_count_ = 0;

  // Path of the runtime binary used in the current session (set once in
  // Start; reused by the supervisor on restart).
  std::filesystem::path runtime_binary_;

  // App-private data directory handed to the runtime as its persistence root.
  std::filesystem::path app_data_dir_;

  // Active profile context — used to derive profile/memory/workspace paths
  // for RuntimeConfig.
  std::string profile_id_ = "default";
  std::string memory_id_ = "default";
  std::filesystem::path workspace_root_;

  // Sandbox policy for the active workspace; serialized into the RuntimeConfig
  // JSON on each Start() / SpawnAndHandshake(). null_json = no sandbox section.
  nlohmann::json sandbox_config_ = nullptr;

  // Service name advertised by the current child (used to reconnect after
  // restart).
  std::string service_name_;
};

}  // namespace cronymax
