#pragma once

#include <functional>
#include <memory>
#include <string>
#include <string_view>
#include <unordered_map>

#include <nlohmann/json.hpp>

#include "include/cef_browser.h"

namespace cronymax {

// ---------------------------------------------------------------------------
// BridgeCallback — abstract response interface passed to every handler.
//
// Two concrete implementations live in bridge_handler.cc:
//   QueryBridgeCallback  — wraps the legacy cefQuery callback (dumps JSON)
//   BinaryBridgeCallback — sends msgpack CefBinaryValue process message
// ---------------------------------------------------------------------------

class BridgeCallback {
 public:
  virtual ~BridgeCallback() = default;
  virtual void Success(const nlohmann::json& response) = 0;
  virtual void Failure(int error_code, const std::string& error_message) = 0;
};

// ---------------------------------------------------------------------------
// BridgeCtx — per-call context passed to every registered handler.
// ---------------------------------------------------------------------------

struct BridgeCtx {
  CefRefPtr<CefBrowser> browser;
  std::string_view channel;
  nlohmann::json payload;  // pre-parsed; empty object if none
  std::shared_ptr<BridgeCallback> callback;
};

using HandlerFn = std::function<void(BridgeCtx)>;

// ---------------------------------------------------------------------------
// BridgeRegistry — exact-channel dispatch table.
// ---------------------------------------------------------------------------

class BridgeRegistry {
 public:
  // Register an exact channel with its handler function.
  // Calling add() for a channel that is already registered overwrites it.
  void add(std::string channel, HandlerFn fn);

  // Dispatch an incoming channel to its registered handler.
  // Returns true if a handler was found and invoked.
  // Returns false (and calls ctx.callback->Failure(404, ...)) if unknown.
  bool dispatch(std::string_view channel, BridgeCtx ctx) const;

 private:
  std::unordered_map<std::string, HandlerFn> table_;
};

}  // namespace cronymax
