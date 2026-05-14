// app/browser/bridge_registry.cc
// Exact-channel dispatch table for BridgeHandler::OnQuery.

#include "browser/bridge_registry.h"

namespace cronymax {

void BridgeRegistry::add(std::string channel, HandlerFn fn) {
  table_[std::move(channel)] = std::move(fn);
}

bool BridgeRegistry::dispatch(std::string_view channel, BridgeCtx ctx) const {
  auto it = table_.find(std::string(channel));
  if (it == table_.end()) {
    ctx.callback->Failure(404, "unknown bridge channel");
    return false;
  }
  it->second(ctx);
  return true;
}

}  // namespace cronymax
