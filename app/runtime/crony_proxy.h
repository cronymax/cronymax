#pragma once

// runtime_proxy.h — request/reply correlation and event fanout on top
// of RuntimeBridge.
//
// Implements tasks 2.1, 2.2, and 2.3 of `rust-runtime-cpp-cutover`.
//
// RuntimeProxy sits between BridgeHandler and RuntimeBridge:
//
//   BridgeHandler ──────────────┐
//      SendControl(req, cb)     │
//                               ▼
//                       RuntimeProxy
//                           │
//                    RuntimeBridge::Invoke()
//                           │
//                    (recv pump thread)
//                           │
//           ┌───────────────┼───────────────────┐
//           ▼               ▼                   ▼
//     reply callback   event fanout        capability
//     (by corr_id)     to subscribers       adapter
//
// Wire format (from cronymax protocol/envelope.rs):
//
//   ClientToRuntime::Control
//     { "tag": "control", "id": "<uuid>",
//       "request": { "kind": "ping" | "start_run" | ... } }
//
//   RuntimeToClient::Control
//     { "tag": "control", "id": "<uuid>",
//       "response": { "kind": "pong" | "run_started" | ... } }
//
//   RuntimeToClient::Event
//     { "tag": "event",
//       "subscription": "<uuid>",
//       "event": { "sequence": N, "emitted_at_ms": N,
//                  "payload": { "kind": "...", ... } } }
//
//   RuntimeToClient::CapabilityCall
//     { "tag": "capability_call", "id": "<uuid>",
//       "request": { "capability": "user_approval" | "shell" | ... } }
//
//   ClientToRuntime::CapabilityReply
//     { "tag": "capability_reply", "id": "<uuid>",
//       "response": { "outcome": "ok" | "err", ... } }
//
// Thread safety:
//   SendControl() and SubscribeEvents() are safe to call from any thread.
//   Reply callbacks fire on the recv pump thread — implementations must
//   be thread-safe (capture CefRefPtr<Callback> and post to UI thread if
//   needed).

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <functional>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>
#include <vector>

#include <nlohmann/json.hpp>

#include "runtime/crony_bridge.h"

namespace cronymax {

// ---------------------------------------------------------------------------
// RuntimeProxy
// ---------------------------------------------------------------------------

class RuntimeProxy {
 public:
  RuntimeProxy() = default;
  ~RuntimeProxy();

  RuntimeProxy(const RuntimeProxy&) = delete;
  RuntimeProxy& operator=(const RuntimeProxy&) = delete;

  // Attach to a RuntimeBridge (must be called before any request is sent).
  // The bridge must outlive this proxy. Registers a pump subscriber.
  void Attach(RuntimeBridge* bridge);

  // Detach: unregister the pump subscriber. Safe to call from any thread.
  void Detach();

  // -------------------------------------------------------------------------
  // 2.1 — Request/reply (task 2.1)
  //
  // Build a ClientToRuntime::Control envelope with a freshly-minted
  // CorrelationId, send it via RuntimeBridge::Invoke(), and deliver the
  // corresponding RuntimeToClient::Control reply to `reply_cb` when it
  // arrives from the pump thread.
  //
  // `reply_cb(response_json, /*is_error=*/bool)` is called exactly once:
  //   - with the parsed "response" sub-object on success
  //   - with an error JSON object if Invoke() fails or the status is an
  //     error response ("kind":"err")
  //
  // Returns false immediately if the bridge is not ready.
  // -------------------------------------------------------------------------
  using ReplyCallback =
      std::function<void(nlohmann::json response, bool is_error)>;

  bool SendControl(nlohmann::json request, ReplyCallback reply_cb);

  // Synchronous variant: blocks the calling thread up to `timeout_ms`.
  // Stores the response in *out_response. Returns false on timeout or error.
  bool SendControlSync(nlohmann::json request,
                       nlohmann::json* out_response,
                       int timeout_ms = 10'000);

  // -------------------------------------------------------------------------
  // 2.2 — Event subscription fanout (task 2.2)
  //
  // Register for all RuntimeToClient::Event messages delivered to this
  // proxy. The callback fires on the recv pump thread.
  // -------------------------------------------------------------------------
  using EventCallback = std::function<void(const nlohmann::json& event)>;

  int64_t SubscribeEvents(EventCallback cb);
  void UnsubscribeEvents(int64_t token);

  // -------------------------------------------------------------------------
  // 2.3 — Capability adapter boundary (task 2.3)
  //
  // When the runtime issues a CapabilityCall the proxy invokes this
  // handler (on the pump thread). The handler MUST call
  // reply(response_json) exactly once; the proxy then sends a
  // ClientToRuntime::CapabilityReply back to the runtime.
  //
  // The handler is installed by the host (MainWindow / BridgeHandler) to
  // route to the PermissionBroker and platform adapters.
  // -------------------------------------------------------------------------
  using CapabilityReplyFn = std::function<void(nlohmann::json response)>;
  using CapabilityHandler =
      std::function<void(const std::string& correlation_id,
                         const nlohmann::json& request,
                         CapabilityReplyFn reply)>;

  void SetCapabilityHandler(CapabilityHandler h) {
    std::lock_guard lock(cap_mu_);
    capability_handler_ = std::move(h);
  }

 private:
  // Called by the RuntimeBridge pump for every received message.
  void OnPayload(const std::string& json_payload);

  // Dispatch helpers.
  void HandleControlReply(const nlohmann::json& msg);
  void HandleEvent(const nlohmann::json& msg);
  void HandleCapabilityCall(const nlohmann::json& msg);

  // Send a raw JSON envelope via the bridge (takes send_mu_ if needed but
  // RuntimeBridge::Invoke already serialises internally).
  bool SendRaw(const std::string& envelope);

  // Generate a new UUID v4 string (16 random bytes → hex).
  static std::string NewCorrelationId();

  RuntimeBridge* bridge_ = nullptr;
  int64_t bridge_sub_token_ = -1;

  // Pending reply callbacks: correlation_id → callback + cv for sync variant.
  struct PendingEntry {
    ReplyCallback cb;
    // For sync variant: cv + result storage.
    std::shared_ptr<std::mutex> sync_mu;
    std::shared_ptr<std::condition_variable> sync_cv;
    std::shared_ptr<nlohmann::json> sync_result;
    std::shared_ptr<bool> sync_error;
    std::shared_ptr<bool> sync_done;
  };
  std::mutex pending_mu_;
  std::unordered_map<std::string, PendingEntry> pending_;

  // Event subscribers.
  std::mutex event_mu_;
  int64_t next_event_token_ = 1;
  std::vector<std::pair<int64_t, EventCallback>> event_subs_;

  // Capability handler.
  std::mutex cap_mu_;
  CapabilityHandler capability_handler_;
};

}  // namespace cronymax
