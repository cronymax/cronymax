#include "renderer/app.h"

#include <mutex>
#include <random>
#include <string>

#include "include/cef_process_message.h"
#include "include/cef_values.h"
#include "include/wrapper/cef_closure_task.h"

// JSON serialisation for runtime control requests.
#include "nlohmann/json.hpp"
#include "nlohmann/json_fwd.hpp"

namespace cronymax {

// CEF process-message names used on the renderer↔browser runtime channel.
// Renderer sends control requests; browser replies and pushes events.
static constexpr char kMsgRuntimeCtrl[] = "cronymax.runtime.ctrl";
static constexpr char kMsgRuntimeCtrlReply[] = "cronymax.runtime.ctrl.reply";
static constexpr char kMsgRuntimeEvent[] = "cronymax.runtime.event";
static constexpr char kMsgBrowserCtrl[] = "cronymax.browser.ctrl";
static constexpr char kMsgBrowserCtrlReply[] = "cronymax.browser.ctrl.reply";

// ---------------------------------------------------------------------------
// V8 ↔ nlohmann::json conversion helpers (renderer process only)
// ---------------------------------------------------------------------------

static nlohmann::json V8ToJson(CefRefPtr<CefV8Value> val, int depth = 0) {
  if (depth > 32 || !val)
    return nullptr;
  if (val->IsNull() || val->IsUndefined())
    return nullptr;
  if (val->IsBool())
    return val->GetBoolValue();
  if (val->IsInt())
    return val->GetIntValue();
  if (val->IsUInt())
    return val->GetUIntValue();
  if (val->IsDouble())
    return val->GetDoubleValue();
  if (val->IsString())
    return val->GetStringValue().ToString();
  if (val->IsArray()) {
    auto arr = nlohmann::json::array();
    const int len = val->GetArrayLength();
    for (int i = 0; i < len; ++i)
      arr.push_back(V8ToJson(val->GetValue(i), depth + 1));
    return arr;
  }
  if (val->IsObject()) {
    auto obj = nlohmann::json::object();
    std::vector<CefString> keys;
    val->GetKeys(keys);
    for (const auto& k : keys)
      obj[k.ToString()] = V8ToJson(val->GetValue(k), depth + 1);
    return obj;
  }
  return nullptr;
}

static CefRefPtr<CefV8Value> JsonToV8(const nlohmann::json& j) {
  if (j.is_null())
    return CefV8Value::CreateNull();
  if (j.is_boolean())
    return CefV8Value::CreateBool(j.get<bool>());
  if (j.is_number_integer())
    return CefV8Value::CreateInt(j.get<int>());
  if (j.is_number_unsigned())
    return CefV8Value::CreateUInt(j.get<unsigned>());
  if (j.is_number_float())
    return CefV8Value::CreateDouble(j.get<double>());
  if (j.is_string())
    return CefV8Value::CreateString(j.get<std::string>());
  if (j.is_array()) {
    auto arr = CefV8Value::CreateArray(static_cast<int>(j.size()));
    for (int i = 0; i < static_cast<int>(j.size()); ++i)
      arr->SetValue(i, JsonToV8(j[i]));
    return arr;
  }
  if (j.is_object()) {
    auto obj = CefV8Value::CreateObject(nullptr, nullptr);
    for (const auto& [k, v] : j.items())
      obj->SetValue(k, JsonToV8(v), V8_PROPERTY_ATTRIBUTE_NONE);
    return obj;
  }
  return CefV8Value::CreateNull();
}

static CefRefPtr<CefV8Value> BinaryToV8Json(CefRefPtr<CefBinaryValue> binary) {
  if (!binary || binary->GetSize() == 0)
    return CefV8Value::CreateNull();
  std::vector<uint8_t> bytes(binary->GetSize());
  binary->GetData(bytes.data(), bytes.size(), 0);
  auto j = nlohmann::json::from_msgpack(bytes, true, false);
  return j.is_discarded() ? CefV8Value::CreateNull() : JsonToV8(j);
}

// ---------------------------------------------------------------------------
// UUID v4 generator — used for correlation IDs
// ---------------------------------------------------------------------------

// static
std::string App::MakeId() {
  // RFC 4122 §4.4 — version 4 UUID from random bytes.
  static std::mutex rng_mu;
  static std::mt19937_64 rng{std::random_device{}()};
  uint8_t b[16];
  {
    std::lock_guard<std::mutex> g(rng_mu);
    uint64_t hi = rng(), lo = rng();
    for (int i = 0; i < 8; ++i)
      b[i] = static_cast<uint8_t>(hi >> (56 - 8 * i));
    for (int i = 0; i < 8; ++i)
      b[8 + i] = static_cast<uint8_t>(lo >> (56 - 8 * i));
  }
  b[6] = (b[6] & 0x0f) | 0x40;  // version 4
  b[8] = (b[8] & 0x3f) | 0x80;  // variant 10xx
  char buf[37];
  std::snprintf(
      buf, sizeof(buf),
      "%02x%02x%02x%02x-%02x%02x-%02x%02x-%02x%02x-%02x%02x%02x%02x%02x%02x",
      b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11],
      b[12], b[13], b[14], b[15]);
  return buf;
}

// ---------------------------------------------------------------------------
// V8 handler: window.cronymax.runtime.send(request) → Promise<string>
//
// `request` must be a ControlRequest object with a `kind` field.  The handler
// serializes it to JSON, wraps it in a cronymax.runtime.ctrl process message,
// and returns a Promise that resolves/rejects when the matching ctrl.reply
// message arrives from the browser process.
// ---------------------------------------------------------------------------

class RuntimeCtrlHandler : public CefV8Handler {
 public:
  explicit RuntimeCtrlHandler(App* app) : app_(app) {}

  bool Execute(const CefString& name,
               CefRefPtr<CefV8Value> object,
               const CefV8ValueList& arguments,
               CefRefPtr<CefV8Value>& retval,
               CefString& exception) override;

 private:
  App* app_;
  IMPLEMENT_REFCOUNTING(RuntimeCtrlHandler);
};

bool RuntimeCtrlHandler::Execute(const CefString& /*name*/,
                                 CefRefPtr<CefV8Value> /*object*/,
                                 const CefV8ValueList& arguments,
                                 CefRefPtr<CefV8Value>& retval,
                                 CefString& exception) {
  if (arguments.empty() || !arguments[0]->IsObject()) {
    exception = "cronymax.runtime.send: expected a ControlRequest object";
    return true;
  }

  auto context = CefV8Context::GetCurrentContext();

  // Serialise the ControlRequest via V8ToJson → msgpack (no JSON.stringify).
  const auto j = V8ToJson(arguments[0]);
  const auto bytes = nlohmann::json::to_msgpack(j);

  const std::string corr_id = App::MakeId();

  // Create a native V8 Promise.
  auto promise = CefV8Value::CreatePromise();
  if (!promise) {
    exception = "cronymax.runtime.send: failed to create Promise";
    return true;
  }

  // Register before sending to avoid a race with the reply.
  app_->pending_runtime_ctrl_callbacks_[corr_id] = promise;

  // Send ctrl process message: args[0]=corr_id, args[1]=msgpack bytes.
  auto msg = CefProcessMessage::Create(kMsgRuntimeCtrl);
  auto args = msg->GetArgumentList();
  args->SetString(0, corr_id);
  args->SetBinary(1, CefBinaryValue::Create(bytes.data(), bytes.size()));
  context->GetFrame()->SendProcessMessage(PID_BROWSER, msg);

  retval = promise;
  return true;
}

// ---------------------------------------------------------------------------
// V8 handler: window.cronymax.runtime.subscribe(topic, cb) → unsub fn
//
// Sends a Subscribe control request to the runtime. When the Subscribed
// response arrives, the callback is registered under the returned
// subscription UUID. Returns an unsubscribe function immediately.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// V8 handler: unsubscribe function returned by subscribe()
// ---------------------------------------------------------------------------

class RuntimeUnsubscribeHandler : public CefV8Handler {
 public:
  RuntimeUnsubscribeHandler(App* app, std::string corr_id)
      : app_(app), corr_id_(std::move(corr_id)) {}

  bool Execute(const CefString&,
               CefRefPtr<CefV8Value>,
               const CefV8ValueList&,
               CefRefPtr<CefV8Value>&,
               CefString&) override {
    auto sub_it = app_->corr_to_runtime_sub_id_.find(corr_id_);
    if (sub_it == app_->corr_to_runtime_sub_id_.end()) {
      // Subscribe reply not yet received — cancel the pending entry.
      app_->pending_runtime_sub_callbacks_.erase(corr_id_);
      return true;
    }
    const std::string sub_id = sub_it->second;
    app_->runtime_subscribers_.erase(sub_id);
    app_->corr_to_runtime_sub_id_.erase(sub_it);

    // Tell the browser process to unsubscribe from the runtime.
    auto context = CefV8Context::GetCurrentContext();
    if (context && context->GetFrame()) {
      nlohmann::json req;
      req["kind"] = "unsubscribe";
      req["subscription"] = sub_id;
      auto msg = CefProcessMessage::Create(kMsgRuntimeCtrl);
      auto args = msg->GetArgumentList();
      args->SetString(0, App::MakeId());  // one-way; no reply needed
      const auto bytes = nlohmann::json::to_msgpack(req);
      args->SetBinary(1, CefBinaryValue::Create(bytes.data(), bytes.size()));
      context->GetFrame()->SendProcessMessage(PID_BROWSER, msg);
    }
    return true;
  }

 private:
  App* app_;
  std::string corr_id_;
  IMPLEMENT_REFCOUNTING(RuntimeUnsubscribeHandler);
};

// ---------------------------------------------------------------------------
// V8 handler: window.cronymax.runtime.subscribe(topic, callback) → unsub fn
//
// Sends a subscribe control request to the browser process via process
// message.  Returns an unsubscribe function immediately; once the browser
// confirms the subscription, subsequent runtime events for that topic are
// delivered to `callback`.
// ---------------------------------------------------------------------------

class RuntimeSubscribeHandler : public CefV8Handler {
 public:
  explicit RuntimeSubscribeHandler(App* app) : app_(app) {}

  bool Execute(const CefString& name,
               CefRefPtr<CefV8Value> object,
               const CefV8ValueList& arguments,
               CefRefPtr<CefV8Value>& retval,
               CefString& exception) override;

 private:
  App* app_;
  IMPLEMENT_REFCOUNTING(RuntimeSubscribeHandler);
};

bool RuntimeSubscribeHandler::Execute(const CefString& /*name*/,
                                      CefRefPtr<CefV8Value> /*object*/,
                                      const CefV8ValueList& arguments,
                                      CefRefPtr<CefV8Value>& retval,
                                      CefString& exception) {
  if (arguments.size() < 2 || !arguments[0]->IsString() ||
      !arguments[1]->IsFunction()) {
    exception =
        "cronymax.runtime.subscribe: expected (topic: string, callback: "
        "function)";
    return true;
  }

  const std::string topic = arguments[0]->GetStringValue().ToString();
  const CefRefPtr<CefV8Value> callback = arguments[1];
  const std::string corr_id = App::MakeId();

  app_->pending_runtime_sub_callbacks_[corr_id] = callback;

  // Send subscribe request to browser: args[0]=corr_id, args[1]=msgpack bytes.
  nlohmann::json req;
  req["kind"] = "subscribe";
  req["topic"] = topic;
  auto msg = CefProcessMessage::Create(kMsgRuntimeCtrl);
  auto args = msg->GetArgumentList();
  args->SetString(0, corr_id);
  const auto bytes = nlohmann::json::to_msgpack(req);
  args->SetBinary(1, CefBinaryValue::Create(bytes.data(), bytes.size()));
  CefV8Context::GetCurrentContext()->GetFrame()->SendProcessMessage(PID_BROWSER,
                                                                    msg);

  // Return unsubscribe function immediately.
  retval = CefV8Value::CreateFunction(
      "unsubscribe", new RuntimeUnsubscribeHandler(app_, corr_id));
  return true;
}

// ---------------------------------------------------------------------------
// V8 handler: window.cronymax.browser.send(channel, payload) → Promise
//
// Uses the binary msgpack transport (cronymax.browser.send process message).
// ---------------------------------------------------------------------------

class BrowserCtrlHandler : public CefV8Handler {
 public:
  explicit BrowserCtrlHandler(App* app) : app_(app) {}

  bool Execute(const CefString& /*name*/,
               CefRefPtr<CefV8Value> /*object*/,
               const CefV8ValueList& arguments,
               CefRefPtr<CefV8Value>& retval,
               CefString& exception) override {
    if (arguments.size() < 2 || !arguments[0]->IsString()) {
      exception = "jsbSend requires (channel: string, payload: any)";
      return true;
    }
    const std::string channel = arguments[0]->GetStringValue().ToString();
    const auto j = V8ToJson(arguments[1]);
    const auto bytes = nlohmann::json::to_msgpack(j);

    auto context = CefV8Context::GetCurrentContext();

    // Create a native V8 Promise.
    auto promise = CefV8Value::CreatePromise();
    if (!promise) {
      exception = "jsbSend: failed to create Promise";
      return true;
    }
    retval = promise;

    const std::string corr_id = App::MakeId();
    app_->pending_browser_ctrl_callbacks_[corr_id] = promise;

    auto msg = CefProcessMessage::Create(kMsgBrowserCtrl);
    auto args = msg->GetArgumentList();
    args->SetString(0, corr_id);
    args->SetString(1, channel);
    args->SetBinary(2, CefBinaryValue::Create(bytes.data(), bytes.size()));
    context->GetFrame()->SendProcessMessage(PID_BROWSER, msg);
    return true;
  }

 private:
  App* app_;
  IMPLEMENT_REFCOUNTING(BrowserCtrlHandler);
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

  // Move cefQuery / cefQueryCancel from the window global into
  // window.cronymax.browser.query / .queryCancel, then delete the originals
  // so that only the cronymax namespace is exposed to JS.
  {
    CefRefPtr<CefV8Value> global = context->GetGlobal();
    CefRefPtr<CefV8Value> cronymax_obj = global->GetValue("cronymax");
    if (!cronymax_obj || !cronymax_obj->IsObject()) {
      cronymax_obj = CefV8Value::CreateObject(nullptr, nullptr);
      global->SetValue("cronymax", cronymax_obj, V8_PROPERTY_ATTRIBUTE_NONE);
    }
    CefRefPtr<CefV8Value> browser_obj = cronymax_obj->GetValue("browser");
    if (!browser_obj || !browser_obj->IsObject()) {
      browser_obj = CefV8Value::CreateObject(nullptr, nullptr);
      cronymax_obj->SetValue("browser", browser_obj,
                             V8_PROPERTY_ATTRIBUTE_NONE);
    }
    // Binary msgpack fast path: window.cronymax.browser.send
    browser_obj->SetValue(
        "send",
        CefV8Value::CreateFunction("send", new BrowserCtrlHandler(this)),
        V8_PROPERTY_ATTRIBUTE_NONE);
  }

  // Inject window.cronymax.runtime only into built-in main frames.
  if (!frame->IsMain())
    return;
  if (!IsBuiltinUrl(frame->GetURL()))
    return;

  main_context_ = context;

  // Ensure window.cronymax exists; bridge.ts adds .browser to the same object.
  CefRefPtr<CefV8Value> global = context->GetGlobal();
  CefRefPtr<CefV8Value> cronymax_obj = global->GetValue("cronymax");
  if (!cronymax_obj || !cronymax_obj->IsObject()) {
    cronymax_obj = CefV8Value::CreateObject(nullptr, nullptr);
    global->SetValue("cronymax", cronymax_obj, V8_PROPERTY_ATTRIBUTE_NONE);
  }

  // Build window.cronymax.runtime = { send, subscribe }
  // Both functions communicate with the Rust runtime via CEF process messages
  // (cronymax.runtime.ctrl) rather than a GIPS connection.  This sidesteps
  // the macOS sandbox restriction on Mach bootstrap lookups in renderer procs.
  CefRefPtr<CefV8Value> runtime_obj =
      CefV8Value::CreateObject(nullptr, nullptr);

  runtime_obj->SetValue(
      "send", CefV8Value::CreateFunction("send", new RuntimeCtrlHandler(this)),
      V8_PROPERTY_ATTRIBUTE_NONE);

  runtime_obj->SetValue("subscribe",
                        CefV8Value::CreateFunction(
                            "subscribe", new RuntimeSubscribeHandler(this)),
                        V8_PROPERTY_ATTRIBUTE_NONE);

  cronymax_obj->SetValue("runtime", runtime_obj, V8_PROPERTY_ATTRIBUTE_NONE);
}

void App::OnContextReleased(CefRefPtr<CefBrowser> browser,
                            CefRefPtr<CefFrame> frame,
                            CefRefPtr<CefV8Context> context) {
  render_message_router_->OnContextReleased(browser, frame, context);

  if (frame->IsMain()) {
    // Send explicit unsubscribe for every confirmed subscription so the
    // browser's renderer_subs_ is drained immediately.  Without this,
    // every navigation accumulates zombie entries in the browser process.
    for (const auto& [corr_id, sub_id] : corr_to_runtime_sub_id_) {
      nlohmann::json req;
      req["kind"] = "unsubscribe";
      req["subscription"] = sub_id;
      auto msg = CefProcessMessage::Create(kMsgRuntimeCtrl);
      auto margs = msg->GetArgumentList();
      margs->SetString(0, App::MakeId());  // one-way; no reply expected
      const auto bytes = nlohmann::json::to_msgpack(req);
      margs->SetBinary(1, CefBinaryValue::Create(bytes.data(), bytes.size()));
      frame->SendProcessMessage(PID_BROWSER, msg);
    }

    runtime_subscribers_.clear();
    pending_runtime_ctrl_callbacks_.clear();
    pending_runtime_sub_callbacks_.clear();
    corr_to_runtime_sub_id_.clear();
    pending_browser_ctrl_callbacks_.clear();
    main_context_ = nullptr;
  }
}

// ---------------------------------------------------------------------------
// Bridge — process message dispatch (render thread)
//
// cronymax.runtime.ctrl.reply  args[0]=corr_id, args[1]=response_json,
//                               args[2]=is_error (bool)
// cronymax.runtime.event        args[0]=sub_id, args[1]=event_envelope_json
// ---------------------------------------------------------------------------

bool App::OnProcessMessageReceived(CefRefPtr<CefBrowser> browser,
                                   CefRefPtr<CefFrame> frame,
                                   CefProcessId source_process,
                                   CefRefPtr<CefProcessMessage> message) {
  const std::string name = message->GetName().ToString();

  // ── Control reply ────────────────────────────────────────────────────────
  if (name == kMsgRuntimeCtrlReply) {
    auto msg_args = message->GetArgumentList();
    const std::string corr_id = msg_args->GetString(0).ToString();
    // args[1] is now a CefBinaryValue (msgpack-encoded response).
    const bool is_error = msg_args->GetBool(2);
    // args[3]=kind_hint, args[4]=sub_id_hint — injected by browser to avoid
    // a msgpack parse on the hot success path.
    const std::string kind =
        is_error ? std::string{} : msg_args->GetString(3).ToString();
    const std::string sub_id_hint = msg_args->GetString(4).ToString();

    // Subscribe confirmation: move callback to active subscribers map.
    auto sub_pending = pending_runtime_sub_callbacks_.find(corr_id);
    if (sub_pending != pending_runtime_sub_callbacks_.end()) {
      if (!is_error && kind == "subscribed" && !sub_id_hint.empty()) {
        runtime_subscribers_[sub_id_hint] = sub_pending->second;
        corr_to_runtime_sub_id_[corr_id] = sub_id_hint;
      }
      pending_runtime_sub_callbacks_.erase(sub_pending);
      return true;
    }

    // Regular request/response — resolve or reject the Promise.
    auto cb_it = pending_runtime_ctrl_callbacks_.find(corr_id);
    if (cb_it == pending_runtime_ctrl_callbacks_.end())
      return true;

    auto promise = cb_it->second;
    pending_runtime_ctrl_callbacks_.erase(cb_it);

    if (!main_context_)
      return true;
    main_context_->Enter();
    if (is_error) {
      std::string err_msg = "runtime error";
      if (auto bin = msg_args->GetBinary(1)) {
        std::vector<uint8_t> err_bytes(bin->GetSize());
        bin->GetData(err_bytes.data(), err_bytes.size(), 0);
        auto j = nlohmann::json::from_msgpack(err_bytes, true, false);
        if (!j.is_discarded()) {
          if (j.is_object() && j.contains("error"))
            err_msg = j["error"].value("message", j.dump());
          else
            err_msg = j.dump();
        }
      }
      promise->RejectPromise(err_msg);
    } else {
      auto v8_resp = BinaryToV8Json(msg_args->GetBinary(1));
      promise->ResolvePromise(v8_resp ? v8_resp : CefV8Value::CreateNull());
    }
    main_context_->Exit();
    return true;
  }

  // ── Browser JSB reply ────────────────────────────────────────────────────
  if (name == kMsgBrowserCtrlReply) {
    auto msg_args = message->GetArgumentList();
    const std::string corr_id = msg_args->GetString(0).ToString();
    const bool is_error = msg_args->GetBool(2);

    auto it = pending_browser_ctrl_callbacks_.find(corr_id);
    if (it == pending_browser_ctrl_callbacks_.end())
      return true;
    auto promise = it->second;
    pending_browser_ctrl_callbacks_.erase(it);

    if (!main_context_)
      return true;
    main_context_->Enter();
    if (is_error) {
      std::string err_msg = "browser ctrl error";
      if (auto bin = msg_args->GetBinary(1)) {
        std::vector<uint8_t> err_bytes(bin->GetSize());
        bin->GetData(err_bytes.data(), err_bytes.size(), 0);
        auto j = nlohmann::json::from_msgpack(err_bytes, true, false);
        if (!j.is_discarded()) {
          if (j.is_object() && j.contains("error"))
            err_msg = j["error"].value("message", j.dump());
          else
            err_msg = j.dump();
        }
      }
      promise->RejectPromise(err_msg);
    } else {
      auto v8_resp = BinaryToV8Json(msg_args->GetBinary(1));
      promise->ResolvePromise(v8_resp ? v8_resp : CefV8Value::CreateNull());
    }
    main_context_->Exit();
    return true;
  }

  // ── Runtime event ────────────────────────────────────────────────────────
  if (name == kMsgRuntimeEvent) {
    auto msg_args = message->GetArgumentList();
    const std::string sub_id = msg_args->GetString(0).ToString();
    // args[1] is already the inner event object — browser unwraps the envelope
    // so we can pass it directly to the JS callback without parse+dump.
    const std::string event_str = msg_args->GetString(1).ToString();

    auto it = runtime_subscribers_.find(sub_id);
    if (it == runtime_subscribers_.end() || !it->second ||
        !it->second->IsFunction())
      return true;

    if (!main_context_)
      return true;
    main_context_->Enter();
    CefV8ValueList v8args;
    v8args.push_back(CefV8Value::CreateString(event_str));
    it->second->ExecuteFunctionWithContext(main_context_, nullptr, v8args);
    main_context_->Exit();
    return true;
  }

  return render_message_router_->OnProcessMessageReceived(
      browser, frame, source_process, message);
}

}  // namespace cronymax
