#include "renderer/app.h"

#include <chrono>
#include <functional>
#include <string>
#include <thread>

#include "include/base/cef_callback.h"
#include "include/cef_task.h"
#include "include/wrapper/cef_closure_task.h"

// JSON parsing for Hello/Welcome handshake and event dispatch.
#include "nlohmann/json.hpp"

namespace cronymax {

// GIPS service name that renderer clients connect to.
static constexpr char kRendererServiceName[] = "ai.cronymax.runtime.renderer";

// How long to poll for the renderer service before giving up.
static constexpr auto kHandshakeTimeout = std::chrono::seconds(10);

// ---------------------------------------------------------------------------
// V8 handler: window.cronymax.send(method, params) → Promise
// ---------------------------------------------------------------------------

class SendHandler : public CefV8Handler {
 public:
  explicit SendHandler(App* app) : app_(app) {}

  bool Execute(const CefString& name,
               CefRefPtr<CefV8Value> object,
               const CefV8ValueList& arguments,
               CefRefPtr<CefV8Value>& retval,
               CefString& exception) override;

 private:
  App* app_;  // not owned; outlived by V8 context
  IMPLEMENT_REFCOUNTING(SendHandler);
};

bool SendHandler::Execute(const CefString& /*name*/,
                          CefRefPtr<CefV8Value> /*object*/,
                          const CefV8ValueList& arguments,
                          CefRefPtr<CefV8Value>& retval,
                          CefString& exception) {
  // Resolve/reject callbacks are captured into the Promise.
  auto context = CefV8Context::GetCurrentContext();

  crony_client_t* c = app_->renderer_client_.load();
  if (!c) {
    // Bridge not ready: return a rejected Promise.
    auto global = context->GetGlobal();
    auto promise_ctor = global->GetValue("Promise");
    if (promise_ctor && promise_ctor->IsFunction()) {
      CefRefPtr<CefV8Value> reject_fn;
      // Build Promise.reject(new Error("bridge not ready"))
      auto error_ctor = global->GetValue("Error");
      CefV8ValueList error_args;
      error_args.push_back(CefV8Value::CreateString("__runtimeBridge: not connected"));
      auto err_obj = error_ctor->ExecuteFunction(nullptr, error_args);
      CefV8ValueList reject_args;
      reject_args.push_back(err_obj ? err_obj : CefV8Value::CreateString("bridge not ready"));
      auto reject_method = promise_ctor->GetValue("reject");
      if (reject_method && reject_method->IsFunction()) {
        retval = reject_method->ExecuteFunctionWithContext(context, promise_ctor, reject_args);
        return true;
      }
    }
    exception = "__runtimeBridge: not connected";
    return true;
  }

  std::string method;
  std::string params_json = "{}";
  if (!arguments.empty() && arguments[0]->IsString()) {
    method = arguments[0]->GetStringValue().ToString();
  }
  if (arguments.size() > 1) {
    // Serialise params via JavaScript's JSON.stringify (handles any V8 value).
    auto json_obj = context->GetGlobal()->GetValue("JSON");
    if (json_obj && json_obj->IsObject()) {
      auto stringify_fn = json_obj->GetValue("stringify");
      if (stringify_fn && stringify_fn->IsFunction()) {
        CefV8ValueList sargs;
        sargs.push_back(arguments[1]);
        auto result =
            stringify_fn->ExecuteFunctionWithContext(context, json_obj, sargs);
        if (result && result->IsString()) {
          params_json = result->GetStringValue().ToString();
        }
      }
    }
  }

  // Build ClientToRuntime envelope: {"tag":"invoke","method":..,"params":..}
  nlohmann::json env;
  env["tag"] = "invoke";
  env["method"] = method;
  auto parsed_params =
      nlohmann::json::parse(params_json, nullptr, /*allow_exceptions=*/false);
  env["params"] = parsed_params.is_discarded() ? nlohmann::json(nullptr)
                                               : parsed_params;
  std::string payload = env.dump();

  // Send synchronously (crony_client_send is fast for small JSON payloads).
  char* err = nullptr;
  int rc = crony_client_send(
      c,
      reinterpret_cast<const uint8_t*>(payload.data()),
      payload.size(),
      &err);
  if (rc != CRONY_OK) {
    std::string msg = err ? err : "send error";
    crony_string_free(err);
    exception = "cronymax: send failed: " + msg;
    return true;
  }

  // For now return undefined; async response arrives via the pump thread
  // and is dispatched to subscribers. Full Promise resolution (correlating
  // request/response IDs) can be added incrementally.
  retval = CefV8Value::CreateUndefined();
  return true;
}

// ---------------------------------------------------------------------------
// V8 handler: window.cronymax.subscribe(topic, cb) → unsub fn
// ---------------------------------------------------------------------------

// UnsubHandler is at file scope (not a local class) so that the
// friend class declaration in App can grant it private access.
class UnsubHandler : public CefV8Handler {
 public:
  UnsubHandler(App* app, std::string topic, CefRefPtr<CefV8Value> cb)
      : app_(app), topic_(std::move(topic)), cb_(std::move(cb)) {}
  bool Execute(const CefString&, CefRefPtr<CefV8Value>, const CefV8ValueList&,
               CefRefPtr<CefV8Value>&, CefString&) override {
    auto it = app_->subscribers_.find(topic_);
    if (it != app_->subscribers_.end()) {
      it->second.erase(cb_);
      if (it->second.empty()) {
        app_->subscribers_.erase(it);
      }
    }
    return true;
  }
 private:
  App* app_;
  std::string topic_;
  CefRefPtr<CefV8Value> cb_;
  IMPLEMENT_REFCOUNTING(UnsubHandler);
};

class SubscribeHandler : public CefV8Handler {
 public:
  explicit SubscribeHandler(App* app) : app_(app) {}

  bool Execute(const CefString& name,
               CefRefPtr<CefV8Value> object,
               const CefV8ValueList& arguments,
               CefRefPtr<CefV8Value>& retval,
               CefString& exception) override;

 private:
  App* app_;
  IMPLEMENT_REFCOUNTING(SubscribeHandler);
};

bool SubscribeHandler::Execute(const CefString& /*name*/,
                               CefRefPtr<CefV8Value> /*object*/,
                               const CefV8ValueList& arguments,
                               CefRefPtr<CefV8Value>& retval,
                               CefString& exception) {
  if (arguments.size() < 2 || !arguments[0]->IsString() || !arguments[1]->IsFunction()) {
    exception = "cronymax.subscribe: expected (topic: string, callback: function)";
    return true;
  }

  std::string topic = arguments[0]->GetStringValue().ToString();
  CefRefPtr<CefV8Value> callback = arguments[1];

  app_->subscribers_[topic].insert(callback);

  retval = CefV8Value::CreateFunction(
      "unsubscribe", new UnsubHandler(app_, topic, callback));
  return true;
}

// ---------------------------------------------------------------------------
// V8 handler: window.cronymax.reconnect()
// ---------------------------------------------------------------------------

class ReconnectHandler : public CefV8Handler {
 public:
  ReconnectHandler(App* app, CefRefPtr<CefFrame> frame)
      : app_(app), frame_(frame) {}

  bool Execute(const CefString& name,
               CefRefPtr<CefV8Value> object,
               const CefV8ValueList& arguments,
               CefRefPtr<CefV8Value>& retval,
               CefString& exception) override {
    app_->DisconnectRuntimeClient();
    // connect + pump run in background; render thread is not blocked.
    app_->StartPumpThread(frame_);
    retval = CefV8Value::CreateUndefined();
    return true;
  }

 private:
  App* app_;
  CefRefPtr<CefFrame> frame_;
  IMPLEMENT_REFCOUNTING(ReconnectHandler);
};

// ---------------------------------------------------------------------------
// App implementation
// ---------------------------------------------------------------------------

App::App() {
  CefMessageRouterConfig config;
  config.js_query_function = "cefQuery";
  config.js_cancel_function = "cefQueryCancel";
  render_message_router_ = CefMessageRouterRendererSide::Create(config);
}

// Check whether a frame URL belongs to built-in pages (non-http/https origins).
static bool IsBuiltinUrl(const CefString& url) {
  std::string u = url.ToString();
  // Built-in pages use file:// or a custom scheme; external pages use https://.
  return u.rfind("https://", 0) != 0 && u.rfind("http://", 0) != 0;
}

void App::OnContextCreated(CefRefPtr<CefBrowser> browser,
                                 CefRefPtr<CefFrame> frame,
                                 CefRefPtr<CefV8Context> context) {
  render_message_router_->OnContextCreated(browser, frame, context);

  // Inject window.cronymax only into built-in main frames.
  if (!frame->IsMain()) return;
  if (!IsBuiltinUrl(frame->GetURL())) return;

  main_context_ = context;

  // Kick off connect + pump entirely in a background thread so the render
  // thread returns immediately and the page can paint.
  StartPumpThread(frame);

  // Build window.cronymax = { send, subscribe, reconnect }
  CefRefPtr<CefV8Value> global = context->GetGlobal();
  CefRefPtr<CefV8Value> bridge = CefV8Value::CreateObject(nullptr, nullptr);

  bridge->SetValue(
      "send",
      CefV8Value::CreateFunction("send", new SendHandler(this)),
      V8_PROPERTY_ATTRIBUTE_NONE);

  bridge->SetValue(
      "subscribe",
      CefV8Value::CreateFunction("subscribe", new SubscribeHandler(this)),
      V8_PROPERTY_ATTRIBUTE_NONE);

  bridge->SetValue(
      "reconnect",
      CefV8Value::CreateFunction("reconnect", new ReconnectHandler(this, frame)),
      V8_PROPERTY_ATTRIBUTE_NONE);

  global->SetValue("cronymax", bridge, V8_PROPERTY_ATTRIBUTE_NONE);
}

void App::OnContextReleased(CefRefPtr<CefBrowser> browser,
                                  CefRefPtr<CefFrame> frame,
                                  CefRefPtr<CefV8Context> context) {
  render_message_router_->OnContextReleased(browser, frame, context);

  if (frame->IsMain()) {
    subscribers_.clear();
    main_context_ = nullptr;
    DisconnectRuntimeClient();
  }
}

bool App::OnProcessMessageReceived(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> frame,
    CefProcessId source_process,
    CefRefPtr<CefProcessMessage> message) {
  return render_message_router_->OnProcessMessageReceived(
      browser, frame, source_process, message);
}

// ---------------------------------------------------------------------------
// Bridge — connect
// ---------------------------------------------------------------------------

bool App::ConnectRuntimeClient() {
  const auto deadline = std::chrono::steady_clock::now() + kHandshakeTimeout;
  crony_client_t* c = nullptr;
  while (!c && std::chrono::steady_clock::now() < deadline) {
    // Bail early if DisconnectRuntimeClient() was called while we were polling.
    if (pump_stop_.load()) return false;
    char* err = nullptr;
    c = crony_client_new(kRendererServiceName, &err);
    if (!c) {
      crony_string_free(err);
      std::this_thread::sleep_for(std::chrono::milliseconds(200));
    }
  }
  if (!c) return false;

  // Send Hello.
  nlohmann::json hello;
  hello["tag"] = "hello";
  hello["protocol"]["major"] = 0;
  hello["protocol"]["minor"] = 1;
  hello["protocol"]["patch"] = 0;
  hello["client_name"] = "cronymax-renderer";
  hello["client_version"] = "0.0.0";
  std::string hello_str = hello.dump();

  char* err = nullptr;
  int rc = crony_client_send(
      c,
      reinterpret_cast<const uint8_t*>(hello_str.data()),
      hello_str.size(),
      &err);
  if (rc != CRONY_OK) {
    crony_string_free(err);
    crony_client_close(c);
    return false;
  }

  // Receive Welcome.
  uint8_t* buf = nullptr;
  size_t len = 0;
  err = nullptr;
  rc = crony_client_recv(c, &buf, &len, &err);
  if (rc != CRONY_OK) {
    crony_string_free(err);
    crony_client_close(c);
    return false;
  }
  std::string resp(reinterpret_cast<char*>(buf), len);
  crony_bytes_free(buf, len);

  auto j = nlohmann::json::parse(resp, nullptr, /*allow_exceptions=*/false);
  if (j.is_discarded() || j.value("tag", "") != "welcome") {
    crony_client_close(c);
    return false;
  }

  renderer_client_.store(c);
  return true;
}

// ---------------------------------------------------------------------------
// Bridge — pump thread
// ---------------------------------------------------------------------------

void App::StartPumpThread(CefRefPtr<CefFrame> frame) {
  pump_stop_.store(false);
  pump_thread_ = std::thread([this, frame]() {
    // Connect phase: runs off the render thread so it can block freely.
    if (!ConnectRuntimeClient()) return;

    // Pump phase.
    while (!pump_stop_.load()) {
      crony_client_t* c = renderer_client_.load();
      if (!c) break;

      uint8_t* buf = nullptr;
      size_t len = 0;
      char* err = nullptr;
      int rc = crony_client_try_recv(c, &buf, &len, &err);

      if (rc == CRONY_ERR_CLOSED || pump_stop_.load()) {
        crony_string_free(err);
        renderer_client_.store(nullptr);
        break;
      }
      if (rc == CRONY_ERR_WOULD_BLOCK) {
        std::this_thread::sleep_for(std::chrono::milliseconds(5));
        continue;
      }
      if (rc != CRONY_OK) {
        crony_string_free(err);
        if (pump_stop_.load()) break;
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
        continue;
      }

      std::string payload(reinterpret_cast<char*>(buf), len);
      crony_bytes_free(buf, len);

      // Marshal dispatch to the render thread.
      CefPostTask(TID_RENDERER,
                  base::BindOnce(&App::DispatchEvent,
                                 base::Unretained(this), payload));
    }
  });
}

// ---------------------------------------------------------------------------
// Bridge — disconnect
// ---------------------------------------------------------------------------

void App::DisconnectRuntimeClient() {
  pump_stop_.store(true);
  crony_client_t* c = renderer_client_.exchange(nullptr);
  if (c) {
    crony_client_close(c);
  }
  if (pump_thread_.joinable()) {
    pump_thread_.join();
  }
  pump_stop_.store(false);
}

// ---------------------------------------------------------------------------
// Bridge — event dispatch (render thread)
// ---------------------------------------------------------------------------

void App::DispatchEvent(const std::string& payload) {
  if (!main_context_) return;

  auto j = nlohmann::json::parse(payload, nullptr, /*allow_exceptions=*/false);
  if (j.is_discarded()) return;
  // RuntimeToClient events carry a "topic" field.
  std::string topic = j.value("topic", "");
  if (topic.empty()) return;

  auto it = subscribers_.find(topic);
  if (it == subscribers_.end() || it->second.empty()) return;

  main_context_->Enter();

  // Pass the raw JSON payload string to each subscriber callback.
  CefV8ValueList args;
  args.push_back(CefV8Value::CreateString(payload));
  for (auto& cb : it->second) {
    if (cb->IsFunction()) {
      cb->ExecuteFunctionWithContext(main_context_, nullptr, args);
    }
  }

  main_context_->Exit();
}

}  // namespace cronymax


