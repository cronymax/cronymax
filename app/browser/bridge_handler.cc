#include "browser/bridge_handler.h"

#include <chrono>
#include <mutex>
#include <optional>
#include <unordered_map>
#include <unordered_set>

#include <nlohmann/json.hpp>

#include "include/base/cef_callback.h"
#include "include/cef_process_message.h"
#include "include/cef_task.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {

// ---------------------------------------------------------------------------
// QueryBridgeCallback — wraps the legacy cefQuery callback.
// Success dumps the json object to a string before calling cefQuery's
// Success so the JS bridge receives the same wire format as before.
// ---------------------------------------------------------------------------

class QueryBridgeCallback : public BridgeCallback {
 public:
  explicit QueryBridgeCallback(
      CefRefPtr<CefMessageRouterBrowserSide::Handler::Callback> cb)
      : cb_(std::move(cb)) {}

  void Success(const nlohmann::json& response) override {
    cb_->Success(response.dump());
  }
  void Failure(int code, const std::string& message) override {
    cb_->Failure(code, message);
  }

 private:
  CefRefPtr<CefMessageRouterBrowserSide::Handler::Callback> cb_;
};

// Forward declaration — BinaryBridgeCallback needs BridgeHandler.
class BinaryBridgeCallback;

namespace {

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

// Returns {channel, payload_json} from a cefQuery request string.
std::pair<std::string, nlohmann::json> SplitEnvelope(
    const std::string& request) {
  if (!request.empty() && request.front() == '{') {
    auto env = nlohmann::json::parse(request, nullptr, false);
    if (!env.is_discarded() && env.is_object()) {
      const std::string channel = env.value("channel", std::string{});
      if (!channel.empty()) {
        nlohmann::json payload = nlohmann::json::object();
        if (env.contains("payload")) {
          if (env["payload"].is_string()) {
            // Legacy: payload was pre-stringified — parse it back.
            auto inner = nlohmann::json::parse(
                env["payload"].get<std::string>(), nullptr, false);
            if (!inner.is_discarded())
              payload = std::move(inner);
          } else if (!env["payload"].is_null()) {
            payload = env["payload"];  // Modern: inline object
          }
        }
        return {channel, std::move(payload)};
      }
    }
  }
  // Legacy format: "<channel>\n<payload-json>".
  const auto sep = request.find('\n');
  if (sep == std::string::npos)
    return {request, nlohmann::json::object()};
  const std::string payload_str = request.substr(sep + 1);
  auto payload = nlohmann::json::parse(payload_str, nullptr, false);
  return {request.substr(0, sep), payload.is_discarded()
                                      ? nlohmann::json::object()
                                      : std::move(payload)};
}

}  // namespace

// ---------------------------------------------------------------------------
// ControlEnricher — abstract base class for outgoing request enrichers.
// ---------------------------------------------------------------------------

class ControlEnricher {
 public:
  virtual ~ControlEnricher() = default;
  virtual void Enrich(const std::string& kind,
                      nlohmann::json& req,
                      SpaceManager* sm) = 0;
};

namespace {

class SpaceContextEnricher : public ControlEnricher {
 public:
  void Enrich(const std::string& kind,
              nlohmann::json& req,
              SpaceManager* sm) override {
    auto* sp = sm->ActiveSpace();
    if (!sp)
      return;
    const std::string wroot = sp->workspace_root.string();

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
        "session_list",
        "session_thread_inspect",
    };
    if (kNeedsWorkspace.count(kind) && !req.contains("workspace_root"))
      req["workspace_root"] = wroot;

    if (kind == "flow_list" && !req.contains("builtin_flows_dir"))
      req["builtin_flows_dir"] = sm->builtin_flows_dir().string();

    if ((kind == "doc_type_list" || kind == "doc_type_load") &&
        !req.contains("builtin_doc_types_dir"))
      req["builtin_doc_types_dir"] = sm->builtin_doc_types_dir().string();

    if (kind == "start_run") {
      if (!req.contains("space_id"))
        req["space_id"] = sp->id;
      if (!req.contains("payload") || !req["payload"].is_object())
        req["payload"] = nlohmann::json::object();
      if (!req["payload"].contains("workspace_root"))
        req["payload"]["workspace_root"] = wroot;
    }
  }
};

class LlmConfigEnricher : public ControlEnricher {
 public:
  void Enrich(const std::string& kind,
              nlohmann::json& req,
              SpaceManager* sm) override {
    if (kind != "start_run")
      return;
    if (!req.contains("payload") || !req["payload"].is_object())
      return;
    if (!req["payload"].contains("task"))
      return;

    std::string base_url = "https://api.openai.com/v1";
    std::string api_key;
    std::string model = "gpt-4o-mini";
    std::string provider_kind = "openai_compat";
    std::string reasoning_effort;
    std::string anthropic_effort;

    const std::string providers_raw = sm->store().GetKv("llm.providers");
    const std::string active_id = sm->store().GetKv("llm.active_provider_id");
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
            const std::string pm = p.value("default_model", std::string{});
            if (!pm.empty())
              model = pm;
            const std::string pk = p.value("kind", std::string{});
            if (!pk.empty())
              provider_kind = pk;
            if (const auto it = p.find("reasoning_effort");
                it != p.end() && it->is_string()) {
              reasoning_effort = it->get<std::string>();
            }
            if (const auto it = p.find("anthropic_effort");
                it != p.end() && it->is_string()) {
              anthropic_effort = it->get<std::string>();
            }
            break;
          }
        }
      }
    } else {
      const auto llm_cfg = sm->store().GetLlmConfig();
      if (!llm_cfg.base_url.empty())
        base_url = llm_cfg.base_url;
      api_key = llm_cfg.api_key;
    }
    // Ensure payload.llm exists, then merge: renderer-supplied fields win.
    if (!req["payload"].contains("llm") || !req["payload"]["llm"].is_object())
      req["payload"]["llm"] = nlohmann::json::object();
    auto& llm = req["payload"]["llm"];
    if (!llm.contains("base_url"))
      llm["base_url"] = base_url;
    if (!llm.contains("api_key"))
      llm["api_key"] = api_key;
    if (!llm.contains("model"))
      llm["model"] = model;
    if (!llm.contains("provider_kind"))
      llm["provider_kind"] = provider_kind;
    if (!llm.contains("reasoning_effort") && !reasoning_effort.empty())
      llm["reasoning_effort"] = reasoning_effort;
    if (!llm.contains("anthropic_effort") && !anthropic_effort.empty())
      llm["anthropic_effort"] = anthropic_effort;
    if (req["payload"].contains("model_override")) {
      const std::string mo =
          req["payload"].value("model_override", std::string{});
      if (!mo.empty())
        llm["model"] = mo;
      req["payload"].erase("model_override");
    }
  }
};

class TerminalDefaultsEnricher : public ControlEnricher {
 public:
  void Enrich(const std::string& kind,
              nlohmann::json& req,
              SpaceManager* /*sm*/) override {
    if (kind != "terminal_start")
      return;
    if (!req.contains("shell"))
      req["shell"] = "/bin/zsh";
    if (!req.contains("cols"))
      req["cols"] = 100;
    if (!req.contains("rows"))
      req["rows"] = 30;
  }
};

}  // namespace

void BridgeHandler::EnrichRequest(const std::string& kind,
                                  nlohmann::json& req) {
  for (auto& enricher : enrichers_)
    enricher->Enrich(kind, req, space_manager_);
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

BridgeHandler::BridgeHandler(SpaceManager* space_manager)
    : space_manager_(space_manager) {
  enrichers_.push_back(std::make_unique<SpaceContextEnricher>());
  enrichers_.push_back(std::make_unique<LlmConfigEnricher>());
  enrichers_.push_back(std::make_unique<TerminalDefaultsEnricher>());

  // Per-module self-registration: each shells/*.cc file defines its own
  // RegisterXxxHandlers function that adds all its channels to the registry.
  RegisterShellHandlers(registry_, this);
  RegisterTerminalHandlers(registry_, this);
  RegisterSpaceHandlers(registry_, this);
  RegisterContentHandlers(registry_, this);
  RegisterEventsHandlers(registry_, this);
  RegisterPermissionHandlers(registry_, this);
  RegisterWorkspaceHandlers(registry_, this);
}

BridgeHandler::~BridgeHandler() = default;

// ---------------------------------------------------------------------------
// OnQuery — delegate to the exact-channel registry
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

  auto [channel, payload] = SplitEnvelope(request.ToString());
  auto cb = std::make_shared<QueryBridgeCallback>(callback);
  BridgeCtx ctx{browser, std::string_view{channel}, std::move(payload),
                std::move(cb)};
  registry_.dispatch(channel, ctx);
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
// SendBrowserEvent — dispatch a browser-side event to a renderer frame.
// Uses a typed CefProcessMessage (kMsgBrowserEvent) instead of JS injection
// so the renderer can decode the payload into a V8 object directly.
//   args[0]: event name  (string)
//   args[1]: payload     (JSON string)
// ---------------------------------------------------------------------------

void BridgeHandler::SendBrowserEvent(CefRefPtr<CefBrowser> browser,
                                     std::string_view event,
                                     std::string_view payload) {
  const std::string ev(event);
  const std::string pl(payload);

  auto send = [browser, ev, pl]() {
    const auto frame = browser->GetMainFrame();
    if (!frame)
      return;
    auto msg = CefProcessMessage::Create(kMsgBrowserEvent);
    auto args = msg->GetArgumentList();
    args->SetString(0, ev);
    args->SetString(1, pl);
    frame->SendProcessMessage(PID_RENDERER, msg);
  };

  if (CefCurrentlyOn(TID_UI))
    send();
  else
    CefPostTask(TID_UI, base::BindOnce([](decltype(send) fn) { fn(); },
                                       std::move(send)));
}

// ---------------------------------------------------------------------------
// BinaryBridgeCallback — sends msgpack reply via CefProcessMessage.
// ---------------------------------------------------------------------------

class BinaryBridgeCallback : public BridgeCallback {
 public:
  BinaryBridgeCallback(BridgeHandler* handler,
                       CefRefPtr<CefBrowser> browser,
                       std::string corr_id)
      : handler_(handler),
        browser_(std::move(browser)),
        corr_id_(std::move(corr_id)) {}

  void Success(const nlohmann::json& response) override {
    handler_->SendBrowserCtrlReply(browser_, corr_id_, response, false);
  }
  void Failure(int code, const std::string& message) override {
    handler_->SendBrowserCtrlReply(
        browser_, corr_id_, nlohmann::json{{"error", message}, {"code", code}},
        true);
  }

 private:
  BridgeHandler* handler_;
  CefRefPtr<CefBrowser> browser_;
  std::string corr_id_;
};

// ---------------------------------------------------------------------------
// HandleBrowserJsbMessage — receive cronymax.browser.ctrl process message.
//   arg[0]: corr_id (string)
//   arg[1]: channel (string)
//   arg[2]: msgpack payload (binary)
// ---------------------------------------------------------------------------

bool BridgeHandler::HandleBrowserCtrlMessage(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> /*frame*/,
    CefRefPtr<CefProcessMessage> message) {
  CEF_REQUIRE_UI_THREAD();
  if (message->GetName() != kMsgBrowserCtrl)
    return false;

  auto margs = message->GetArgumentList();
  const std::string corr_id = margs->GetString(0).ToString();
  const std::string channel = margs->GetString(1).ToString();

  nlohmann::json payload = nlohmann::json::object();
  if (margs->GetSize() > 2) {
    auto binary = margs->GetBinary(2);
    if (binary && binary->GetSize() > 0) {
      std::vector<uint8_t> bytes(binary->GetSize());
      binary->GetData(bytes.data(), bytes.size(), 0);
      auto decoded =
          nlohmann::json::from_msgpack(bytes, true, /*allow_exceptions=*/false);
      if (!decoded.is_discarded())
        payload = std::move(decoded);
    }
  }

  auto cb = std::make_shared<BinaryBridgeCallback>(this, browser, corr_id);
  BridgeCtx ctx{browser, std::string_view{channel}, std::move(payload),
                std::move(cb)};
  registry_.dispatch(channel, ctx);
  return true;
}

// ---------------------------------------------------------------------------
// SendBrowserCtrlReply — send cronymax.browser.ctrl.reply to the renderer.
//   arg[0]: corr_id (string)
//   arg[1]: msgpack response (binary)
//   arg[2]: is_error (bool)
// ---------------------------------------------------------------------------

void BridgeHandler::SendBrowserCtrlReply(CefRefPtr<CefBrowser> browser,
                                         const std::string& corr_id,
                                         const nlohmann::json& response,
                                         bool is_error) {
  auto resp_bytes = nlohmann::json::to_msgpack(response);

  auto send = [browser, corr_id, resp_bytes = std::move(resp_bytes),
               is_error]() {
    auto msg = CefProcessMessage::Create(kMsgBrowserCtrlReply);
    auto args = msg->GetArgumentList();
    args->SetString(0, corr_id);
    args->SetBinary(
        1, CefBinaryValue::Create(resp_bytes.data(), resp_bytes.size()));
    args->SetBool(2, is_error);
    if (auto frame = browser->GetMainFrame())
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
// OnBrowserClosed — clean up per-browser event subscriptions
// ---------------------------------------------------------------------------

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

}  // namespace cronymax
