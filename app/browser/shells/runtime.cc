// app/browser/shells/bridge_runtime.cc
// Renderer↔browser runtime IPC: HandleRuntimeProcessMessage, SendRuntimeReply,
// SendRuntimeEvent.
//
// Note: subscribe/unsubscribe are handled as ordinary control requests here;
// JS manages subscription UUIDs and routes kMsgRuntimeEvent via
// window.cronymax.runtime.on (set by bridge.ts).

#include "browser/bridge_handler.h"

#include <nlohmann/json.hpp>

#include "include/base/cef_callback.h"
#include "include/cef_task.h"
#include "include/cef_values.h"
#include "include/wrapper/cef_closure_task.h"

namespace cronymax {

bool BridgeHandler::HandleRuntimeProcessMessage(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> frame,
    CefRefPtr<CefProcessMessage> message) {
  if (message->GetName() != kMsgRuntimeCtrl)
    return false;

  auto margs = message->GetArgumentList();
  const std::string corr_id = margs->GetString(0).ToString();

  nlohmann::json req;
  if (margs->GetSize() > 1) {
    auto binary = margs->GetBinary(1);
    if (binary && binary->GetSize() > 0) {
      std::vector<uint8_t> bytes(binary->GetSize());
      binary->GetData(bytes.data(), bytes.size(), 0);
      req =
          nlohmann::json::from_msgpack(bytes, true, /*allow_exceptions=*/false);
    }
  }

  if (!runtime_proxy_) {
    SendRuntimeReply(
        browser, corr_id,
        {{"kind", "err"}, {"error", {{"message", "runtime not available"}}}},
        true);
    return true;
  }

  if (req.is_discarded() || !req.is_object()) {
    SendRuntimeReply(
        browser, corr_id,
        {{"kind", "err"}, {"error", {{"message", "invalid msgpack payload"}}}},
        true);
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

      SendRuntimeReply(browser, corr_id, resp, false);
    });
    return true;
  }

  // ── Arbitrary control request ────────────────────────────────────────────
  EnrichRequest(kind, req);

  runtime_proxy_->SendControl(
      std::move(req),
      [this, corr_id, browser, kind](nlohmann::json resp, bool is_error) {
        if (!is_error && resp.contains("payload") && !resp["payload"].is_null())
          SendRuntimeReply(browser, corr_id, resp["payload"], false);
        else
          SendRuntimeReply(browser, corr_id, resp, is_error);

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

// SendRuntimeReply — sends a msgpack-encoded reply to the renderer.
void BridgeHandler::SendRuntimeReply(CefRefPtr<CefBrowser> browser,
                                     const std::string& corr_id,
                                     const nlohmann::json& response,
                                     bool is_error) {
  auto send = [browser, corr_id,
               resp_bytes = nlohmann::json::to_msgpack(response), is_error]() {
    auto msg = CefProcessMessage::Create(kMsgRuntimeCtrlReply);
    auto args = msg->GetArgumentList();
    args->SetString(0, corr_id);
    args->SetBinary(
        1, CefBinaryValue::Create(resp_bytes.data(), resp_bytes.size()));
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

// JSON optimization: send only the inner event object instead of the full
// envelope so the renderer can pass it to JS callbacks without parse+dump.
void BridgeHandler::SendRuntimeEvent(CefRefPtr<CefBrowser> browser,
                                     const std::string& sub_id,
                                     const nlohmann::json& event_envelope) {
  // Extract just the inner "event" object to avoid a parse+dump in the
  // renderer.  Fall back to full envelope if the field is absent.
  std::string event_str;
  if (event_envelope.contains("event") && event_envelope["event"].is_object())
    event_str = event_envelope["event"].dump();
  else
    event_str = event_envelope.dump();

  auto send = [browser, sub_id, event_str = std::move(event_str)]() {
    auto msg = CefProcessMessage::Create(kMsgRuntimeEvent);
    auto args = msg->GetArgumentList();
    args->SetString(0, sub_id);
    args->SetString(1, event_str);
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

}  // namespace cronymax
