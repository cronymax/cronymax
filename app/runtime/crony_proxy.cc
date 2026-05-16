// runtime_proxy.cc — RuntimeProxy implementation.
//
// Covers tasks 2.1 (request/reply correlation), 2.2 (event subscription
// fanout), and 2.3 (capability adapter boundary).

#include "runtime/crony_proxy.h"

#include <cstdlib>
#include <random>
#include <sstream>

namespace cronymax {

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

RuntimeProxy::~RuntimeProxy() {
  Detach();
}

void RuntimeProxy::Attach(RuntimeBridge* bridge) {
  // Idempotent: if already attached (e.g. supervisor restart), remove the
  // stale subscription before adding the new one.
  if (bridge_ && bridge_sub_token_ >= 0) {
    bridge_->Unsubscribe(bridge_sub_token_);
    bridge_sub_token_ = -1;
  }
  bridge_ = bridge;
  bridge_sub_token_ =
      bridge_->Subscribe([this](std::string payload) { OnPayload(payload); });
}

void RuntimeProxy::Detach() {
  if (bridge_ && bridge_sub_token_ >= 0) {
    bridge_->Unsubscribe(bridge_sub_token_);
    bridge_sub_token_ = -1;
  }
  bridge_ = nullptr;

  // Wake any blocked SendControlSync callers so they can observe the
  // detach and return false.
  std::lock_guard lock(pending_mu_);
  for (auto& [id, entry] : pending_) {
    if (entry.sync_done) {
      std::lock_guard sl(*entry.sync_mu);
      *entry.sync_done = true;
      *entry.sync_error = true;
      *entry.sync_result = {
          {"kind", "err"},
          {"error", {{"code", "detached"}, {"message", "proxy detached"}}}};
      entry.sync_cv->notify_all();
    }
    // Async callers: fire error callback on detach.
    if (entry.cb) {
      entry.cb(
          {{"kind", "err"},
           {"error", {{"code", "detached"}, {"message", "proxy detached"}}}},
          true);
    }
  }
  pending_.clear();
}

// ---------------------------------------------------------------------------
// UUID generation (no external dep)
//
// Generates a random UUID v4 string "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx"
// ---------------------------------------------------------------------------

std::string RuntimeProxy::NewCorrelationId() {
  // Use thread-local RNG seeded from std::random_device.
  thread_local std::mt19937_64 rng(std::random_device{}());
  std::uniform_int_distribution<uint64_t> dist;

  const uint64_t hi = dist(rng);
  const uint64_t lo = dist(rng);

  // Version 4: bits 76..79 = 0100; variant: bits 62..63 = 10.
  const uint64_t hi_v4 = (hi & 0xFFFFFFFFFFFF0FFFULL) | 0x0000000000004000ULL;
  const uint64_t lo_var = (lo & 0x3FFFFFFFFFFFFFFFULL) | 0x8000000000000000ULL;

  auto hex2 = [](uint8_t b) -> std::string {
    const char* digits = "0123456789abcdef";
    std::string s(2, '0');
    s[0] = digits[(b >> 4) & 0xF];
    s[1] = digits[b & 0xF];
    return s;
  };

  // Format: 8-4-4-4-12
  const auto b = [&](int shift_from_hi, bool use_lo = false) -> uint8_t {
    if (!use_lo)
      return static_cast<uint8_t>((hi_v4 >> shift_from_hi) & 0xFF);
    return static_cast<uint8_t>((lo_var >> shift_from_hi) & 0xFF);
  };

  return hex2(b(56)) + hex2(b(48)) + hex2(b(40)) + hex2(b(32)) + "-" +
         hex2(b(24)) + hex2(b(16)) + "-" + hex2(b(8)) + hex2(b(0)) + "-" +
         hex2(b(56, true)) + hex2(b(48, true)) + "-" + hex2(b(40, true)) +
         hex2(b(32, true)) + hex2(b(24, true)) + hex2(b(16, true)) +
         hex2(b(8, true)) + hex2(b(0, true));
}

// ---------------------------------------------------------------------------
// 2.1 — SendControl (async)
// ---------------------------------------------------------------------------

bool RuntimeProxy::SendControl(nlohmann::json request, ReplyCallback reply_cb) {
  if (!bridge_)
    return false;

  const std::string corr_id = NewCorrelationId();

  // Build ClientToRuntime::Control envelope.
  nlohmann::json envelope = {
      {"tag", "control"}, {"id", corr_id}, {"request", std::move(request)}};

  // Register before sending to avoid a race where the reply arrives before
  // we store the entry.
  {
    std::lock_guard lock(pending_mu_);
    PendingEntry entry;
    entry.cb = std::move(reply_cb);
    pending_.emplace(corr_id, std::move(entry));
  }

  if (!bridge_->Invoke(envelope.dump())) {
    // Remove the entry and fire the error callback inline.
    ReplyCallback cb;
    {
      std::lock_guard lock(pending_mu_);
      auto it = pending_.find(corr_id);
      if (it != pending_.end()) {
        cb = std::move(it->second.cb);
        pending_.erase(it);
      }
    }
    if (cb)
      cb({{"kind", "err"},
          {"error",
           {{"code", "send_failed"}, {"message", "bridge invoke failed"}}}},
         true);
    return false;
  }
  return true;
}

// ---------------------------------------------------------------------------
// 2.1 — SendControlSync (blocking)
// ---------------------------------------------------------------------------

bool RuntimeProxy::SendControlSync(nlohmann::json request,
                                   nlohmann::json* out_response,
                                   int timeout_ms) {
  if (!bridge_)
    return false;

  const std::string corr_id = NewCorrelationId();

  // Build envelope.
  nlohmann::json envelope = {
      {"tag", "control"}, {"id", corr_id}, {"request", std::move(request)}};

  // Sync state shared between this thread and the pump thread.
  auto sync_mu = std::make_shared<std::mutex>();
  auto sync_cv = std::make_shared<std::condition_variable>();
  auto sync_res = std::make_shared<nlohmann::json>();
  auto sync_err = std::make_shared<bool>(false);
  auto sync_done = std::make_shared<bool>(false);

  {
    std::lock_guard lock(pending_mu_);
    PendingEntry entry;
    entry.sync_mu = sync_mu;
    entry.sync_cv = sync_cv;
    entry.sync_result = sync_res;
    entry.sync_error = sync_err;
    entry.sync_done = sync_done;
    pending_.emplace(corr_id, std::move(entry));
  }

  if (!bridge_->Invoke(envelope.dump())) {
    std::lock_guard lock(pending_mu_);
    pending_.erase(corr_id);
    return false;
  }

  // Wait for reply or timeout.
  std::unique_lock ul(*sync_mu);
  const bool got = sync_cv->wait_for(ul, std::chrono::milliseconds(timeout_ms),
                                     [&] { return *sync_done; });
  if (!got || *sync_err)
    return false;
  if (out_response)
    *out_response = std::move(*sync_res);
  return true;
}

// ---------------------------------------------------------------------------
// 2.2 — Event subscription fanout
// ---------------------------------------------------------------------------

int64_t RuntimeProxy::SubscribeEvents(EventCallback cb) {
  std::lock_guard lock(event_mu_);
  int64_t tok = next_event_token_++;
  event_subs_.emplace_back(tok, std::move(cb));
  return tok;
}

void RuntimeProxy::UnsubscribeEvents(int64_t token) {
  std::lock_guard lock(event_mu_);
  event_subs_.erase(
      std::remove_if(event_subs_.begin(), event_subs_.end(),
                     [token](const auto& p) { return p.first == token; }),
      event_subs_.end());
}

// ---------------------------------------------------------------------------
// Raw send helper
// ---------------------------------------------------------------------------

bool RuntimeProxy::SendRaw(const std::string& envelope) {
  if (!bridge_)
    return false;
  return bridge_->Invoke(envelope);
}

// ---------------------------------------------------------------------------
// Pump-thread dispatch: OnPayload
// ---------------------------------------------------------------------------

void RuntimeProxy::OnPayload(const std::string& json_payload) {
  nlohmann::json msg;
  try {
    msg = nlohmann::json::parse(json_payload);
  } catch (...) {
    // Malformed — ignore.
    return;
  }

  const auto tag_it = msg.find("tag");
  if (tag_it == msg.end() || !tag_it->is_string())
    return;
  const std::string& tag = tag_it->get_ref<const std::string&>();

  if (tag == "control") {
    HandleControlReply(msg);
  } else if (tag == "event") {
    HandleEvent(msg);
  } else if (tag == "capability_call") {
    HandleCapabilityCall(msg);
  } else if (tag == "bridge_restarting") {
    HandleBridgeRestarting();
  } else if (tag == "ping") {
    // Keepalive probe from the runtime.  Reply with Pong so the transport's
    // idle timer is reset and the runtime does not terminate during long
    // agent tasks that produce no inbound control messages.
    if (bridge_)
      bridge_->Invoke(R"({"tag":"pong"})");
  }
  // "welcome" / "goodbye" / others: no action needed here; RuntimeBridge
  // has already handled the handshake.
}

// ---------------------------------------------------------------------------
// HandleControlReply
//
// RuntimeToClient::Control { "tag":"control", "id":"<uuid>",
//                             "response": { "kind":"...", ... } }
// ---------------------------------------------------------------------------

void RuntimeProxy::HandleControlReply(const nlohmann::json& msg) {
  const auto id_it = msg.find("id");
  if (id_it == msg.end() || !id_it->is_string())
    return;
  const std::string& corr_id = id_it->get_ref<const std::string&>();

  PendingEntry entry;
  {
    std::lock_guard lock(pending_mu_);
    auto it = pending_.find(corr_id);
    if (it == pending_.end())
      return;  // stale / duplicate
    entry = std::move(it->second);
    pending_.erase(it);
  }

  const auto resp_it = msg.find("response");
  const nlohmann::json response =
      (resp_it != msg.end()) ? *resp_it : nlohmann::json{};

  const bool is_error = response.value("kind", std::string{}) == "err";

  // Sync variant: signal the waiting thread.
  if (entry.sync_done) {
    std::lock_guard sl(*entry.sync_mu);
    *entry.sync_result = response;
    *entry.sync_error = is_error;
    *entry.sync_done = true;
    entry.sync_cv->notify_all();
    return;
  }

  // Async variant: call the callback.
  if (entry.cb) {
    entry.cb(response, is_error);
  }
}

// ---------------------------------------------------------------------------
// HandleBridgeRestarting
//
// Called (via the sentinel payload "bridge_restarting") by RuntimeBridge's
// supervisor thread just before spawning a new crony child. Drains all
// in-flight pending_ callbacks with an error so that renderer Promises
// reject immediately rather than hanging forever.  Also fires restart_cb_
// so that BridgeHandler can clear stale renderer subscriptions.
// ---------------------------------------------------------------------------

void RuntimeProxy::HandleBridgeRestarting() {
  std::unordered_map<std::string, PendingEntry> pending_snapshot;
  {
    std::lock_guard lock(pending_mu_);
    pending_snapshot = std::move(pending_);
    pending_.clear();
  }

  const nlohmann::json err_resp = {
      {"kind", "err"},
      {"error",
       {{"code", "runtime_restarted"},
        {"message", "runtime restarted; retrying"}}}};

  for (auto& [id, entry] : pending_snapshot) {
    if (entry.sync_done) {
      std::lock_guard sl(*entry.sync_mu);
      *entry.sync_result = err_resp;
      *entry.sync_error = true;
      *entry.sync_done = true;
      entry.sync_cv->notify_all();
    } else if (entry.cb) {
      entry.cb(err_resp, /*is_error=*/true);
    }
  }

  // Notify the bridge handler (or whoever registered) to clear stale state.
  if (restart_cb_)
    restart_cb_();
}

// ---------------------------------------------------------------------------
// HandleEvent
//
// RuntimeToClient::Event { "tag":"event", "subscription":"<uuid>",
//                           "event": { ... } }
// ---------------------------------------------------------------------------

void RuntimeProxy::HandleEvent(const nlohmann::json& msg) {
  const auto ev_it = msg.find("event");
  if (ev_it == msg.end())
    return;

  // Copy subscriber list under lock, then call without lock so
  // subscribers can themselves call UnsubscribeEvents.
  std::vector<std::pair<int64_t, EventCallback>> subs;
  {
    std::lock_guard lock(event_mu_);
    subs = event_subs_;
  }
  // Diagnostic: log event kind and subscriber count to stderr.
  {
    const auto& ev = *ev_it;
    const auto& pl = ev.value("payload", nlohmann::json::object());
    fprintf(stderr, "[RuntimeProxy::HandleEvent] kind=%s subs=%zu\n",
            pl.value("kind", "?").c_str(), subs.size());
    fflush(stderr);
  }
  // Pass the full outer envelope (tag, subscription, event) so
  // subscribers can forward it verbatim to the frontend which expects
  // { tag:"event", subscription, event:{sequence,...,payload:{...}} }.
  for (auto& [tok, cb] : subs) {
    cb(msg);
  }
}

// ---------------------------------------------------------------------------
// HandleCapabilityCall  (task 2.3)
//
// RuntimeToClient::CapabilityCall { "tag":"capability_call",
//                                   "id":"<uuid>",
//                                   "request": { "capability":"...", ...} }
//
// The capability handler (installed by the host via SetCapabilityHandler)
// is responsible for executing the capability and calling the reply
// functor exactly once.  If no handler is installed, we reply with
// CapabilityError::Unsupported.
// ---------------------------------------------------------------------------

void RuntimeProxy::HandleCapabilityCall(const nlohmann::json& msg) {
  const auto id_it = msg.find("id");
  if (id_it == msg.end() || !id_it->is_string())
    return;
  const std::string corr_id = id_it->get_ref<const std::string&>();

  const auto req_it = msg.find("request");
  const nlohmann::json request =
      (req_it != msg.end()) ? *req_it : nlohmann::json{};

  // Capture corr_id and bridge pointer for the reply functor.
  auto reply_fn = [this, corr_id](nlohmann::json response) {
    // Build ClientToRuntime::CapabilityReply.
    const nlohmann::json envelope = {{"tag", "capability_reply"},
                                     {"id", corr_id},
                                     {"response", std::move(response)}};
    SendRaw(envelope.dump());
  };

  CapabilityHandler handler;
  {
    std::lock_guard lock(cap_mu_);
    handler = capability_handler_;
  }

  if (!handler) {
    // No adapter installed: reply unsupported.
    const std::string cap_name = request.value("capability", "unknown");
    reply_fn({{"outcome", "err"},
              {"error", {{"code", "unsupported"}, {"capability", cap_name}}}});
    return;
  }

  handler(corr_id, request, std::move(reply_fn));
}

}  // namespace cronymax
