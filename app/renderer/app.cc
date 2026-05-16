#include "renderer/app.h"

#include <mutex>
#include <random>
#include <string>

#include "include/cef_process_message.h"
#include "include/wrapper/cef_closure_task.h"

// JSON serialisation for runtime control requests.
#include "nlohmann/json.hpp"

namespace cronymax {

// CEF process-message names used on the renderer↔browser runtime channel.
// Renderer sends control requests; browser replies and pushes events.
static constexpr char kMsgCtrl[] = "cronymax.runtime.ctrl";
static constexpr char kMsgCtrlReply[] = "cronymax.runtime.ctrl.reply";
static constexpr char kMsgEvent[] = "cronymax.runtime.event";

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
// serialises it to JSON, wraps it in a cronymax.runtime.ctrl process message,
// and returns a Promise that resolves/rejects when the matching ctrl.reply
// message arrives from the browser process.
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
  App* app_;
  IMPLEMENT_REFCOUNTING(SendHandler);
};

bool SendHandler::Execute(const CefString& /*name*/,
                          CefRefPtr<CefV8Value> /*object*/,
                          const CefV8ValueList& arguments,
                          CefRefPtr<CefV8Value>& retval,
                          CefString& exception) {
  if (arguments.empty() || !arguments[0]->IsObject()) {
    exception = "cronymax.runtime.send: expected a ControlRequest object";
    return true;
  }

  auto context = CefV8Context::GetCurrentContext();

  // Serialise the ControlRequest object to JSON via JSON.stringify.
  std::string request_json;
  {
    auto json_obj = context->GetGlobal()->GetValue("JSON");
    if (json_obj && json_obj->IsObject()) {
      auto stringify_fn = json_obj->GetValue("stringify");
      if (stringify_fn && stringify_fn->IsFunction()) {
        CefV8ValueList sargs;
        sargs.push_back(arguments[0]);
        auto result =
            stringify_fn->ExecuteFunctionWithContext(context, json_obj, sargs);
        if (result && result->IsString())
          request_json = result->GetStringValue().ToString();
      }
    }
  }
  if (request_json.empty()) {
    exception = "cronymax.runtime.send: failed to serialise request";
    return true;
  }

  const std::string corr_id = App::MakeId();

  // Create a JS Promise so the caller can await the reply.
  CefRefPtr<CefV8Value> eval_retval;
  CefRefPtr<CefV8Exception> eval_exc;
  bool eval_ok = context->Eval(
      "(function(){var r,j;"
      "var p=new Promise(function(res,rej){r=res;j=rej;});"
      "return[p,r,j];})()",
      CefString(), 0, eval_retval, eval_exc);
  if (!eval_ok || !eval_retval || !eval_retval->IsArray()) {
    exception = "cronymax.runtime.send: failed to create Promise";
    return true;
  }
  auto promise_val = eval_retval->GetValue(0);
  auto resolve_fn = eval_retval->GetValue(1);
  auto reject_fn = eval_retval->GetValue(2);

  // Register before sending to avoid a race with the reply.
  app_->pending_callbacks_[corr_id] = {resolve_fn, reject_fn};

  // Send ctrl process message: args[0]=corr_id, args[1]=request_json.
  auto msg = CefProcessMessage::Create(kMsgCtrl);
  auto args = msg->GetArgumentList();
  args->SetString(0, corr_id);
  args->SetString(1, request_json);
  context->GetFrame()->SendProcessMessage(PID_BROWSER, msg);

  retval = promise_val;
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

class UnsubHandler : public CefV8Handler {
 public:
  UnsubHandler(App* app, std::string corr_id)
      : app_(app), corr_id_(std::move(corr_id)) {}

  bool Execute(const CefString&,
               CefRefPtr<CefV8Value>,
               const CefV8ValueList&,
               CefRefPtr<CefV8Value>&,
               CefString&) override {
    auto sub_it = app_->corr_to_sub_id_.find(corr_id_);
    if (sub_it == app_->corr_to_sub_id_.end()) {
      // Subscribe reply not yet received — cancel the pending entry.
      app_->pending_sub_callbacks_.erase(corr_id_);
      return true;
    }
    const std::string sub_id = sub_it->second;
    app_->subscribers_.erase(sub_id);
    app_->corr_to_sub_id_.erase(sub_it);

    // Tell the browser process to unsubscribe from the runtime.
    auto context = CefV8Context::GetCurrentContext();
    if (context && context->GetFrame()) {
      nlohmann::json req;
      req["kind"] = "unsubscribe";
      req["subscription"] = sub_id;
      auto msg = CefProcessMessage::Create(kMsgCtrl);
      auto args = msg->GetArgumentList();
      args->SetString(0, App::MakeId());  // one-way; no reply needed
      args->SetString(1, req.dump());
      context->GetFrame()->SendProcessMessage(PID_BROWSER, msg);
    }
    return true;
  }

 private:
  App* app_;
  std::string corr_id_;
  IMPLEMENT_REFCOUNTING(UnsubHandler);
};

// ---------------------------------------------------------------------------
// V8 handler: window.cronymax.runtime.subscribe(topic, callback) → unsub fn
//
// Sends a subscribe control request to the browser process via process
// message.  Returns an unsubscribe function immediately; once the browser
// confirms the subscription, subsequent runtime events for that topic are
// delivered to `callback`.
// ---------------------------------------------------------------------------

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

  app_->pending_sub_callbacks_[corr_id] = callback;

  // Send subscribe request to browser: args[0]=corr_id, args[1]=request_json.
  nlohmann::json req;
  req["kind"] = "subscribe";
  req["topic"] = topic;
  auto msg = CefProcessMessage::Create(kMsgCtrl);
  auto args = msg->GetArgumentList();
  args->SetString(0, corr_id);
  args->SetString(1, req.dump());
  CefV8Context::GetCurrentContext()->GetFrame()->SendProcessMessage(PID_BROWSER,
                                                                    msg);

  // Return unsubscribe function immediately.
  retval = CefV8Value::CreateFunction("unsubscribe",
                                      new UnsubHandler(app_, corr_id));
  return true;
}

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
    auto query_fn = global->GetValue("cefQuery");
    if (query_fn && query_fn->IsFunction()) {
      browser_obj->SetValue("query", query_fn, V8_PROPERTY_ATTRIBUTE_NONE);
    }
    auto cancel_fn = global->GetValue("cefQueryCancel");
    if (cancel_fn && cancel_fn->IsFunction()) {
      browser_obj->SetValue("queryCancel", cancel_fn,
                            V8_PROPERTY_ATTRIBUTE_NONE);
    }
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
      "send", CefV8Value::CreateFunction("send", new SendHandler(this)),
      V8_PROPERTY_ATTRIBUTE_NONE);

  runtime_obj->SetValue(
      "subscribe",
      CefV8Value::CreateFunction("subscribe", new SubscribeHandler(this)),
      V8_PROPERTY_ATTRIBUTE_NONE);

  cronymax_obj->SetValue("runtime", runtime_obj, V8_PROPERTY_ATTRIBUTE_NONE);
}

void App::OnContextReleased(CefRefPtr<CefBrowser> browser,
                            CefRefPtr<CefFrame> frame,
                            CefRefPtr<CefV8Context> context) {
  render_message_router_->OnContextReleased(browser, frame, context);

  if (frame->IsMain()) {
    subscribers_.clear();
    pending_callbacks_.clear();
    pending_sub_callbacks_.clear();
    corr_to_sub_id_.clear();
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
  if (name == kMsgCtrlReply) {
    auto msg_args = message->GetArgumentList();
    const std::string corr_id = msg_args->GetString(0).ToString();
    const std::string resp_str = msg_args->GetString(1).ToString();
    const bool is_error = msg_args->GetBool(2);

    auto j = nlohmann::json::parse(resp_str, nullptr, false);
    if (j.is_discarded())
      j = nlohmann::json::object();

    const std::string kind = is_error ? std::string{} : j.value("kind", "");

    // Subscribe confirmation: move callback to active subscribers map.
    auto sub_pending = pending_sub_callbacks_.find(corr_id);
    if (sub_pending != pending_sub_callbacks_.end()) {
      if (!is_error && kind == "subscribed") {
        const std::string sub_id = j.value("subscription", std::string{});
        if (!sub_id.empty()) {
          subscribers_[sub_id] = sub_pending->second;
          corr_to_sub_id_[corr_id] = sub_id;
        }
      }
      pending_sub_callbacks_.erase(sub_pending);
      return true;
    }

    // Regular request/response — resolve or reject the Promise.
    auto cb_it = pending_callbacks_.find(corr_id);
    if (cb_it == pending_callbacks_.end())
      return true;

    auto [resolve_fn, reject_fn] = cb_it->second;
    pending_callbacks_.erase(cb_it);

    if (!main_context_)
      return true;
    main_context_->Enter();
    CefV8ValueList v8args;
    if (is_error) {
      const std::string msg = j.value(
          "message",
          j.value("error", nlohmann::json{}).value("message", "runtime error"));
      v8args.push_back(CefV8Value::CreateString(msg));
      reject_fn->ExecuteFunctionWithContext(main_context_, nullptr, v8args);
    } else {
      v8args.push_back(CefV8Value::CreateString(resp_str));
      resolve_fn->ExecuteFunctionWithContext(main_context_, nullptr, v8args);
    }
    main_context_->Exit();
    return true;
  }

  // ── Runtime event ────────────────────────────────────────────────────────
  if (name == kMsgEvent) {
    auto msg_args = message->GetArgumentList();
    const std::string sub_id = msg_args->GetString(0).ToString();
    const std::string event_str = msg_args->GetString(1).ToString();

    auto it = subscribers_.find(sub_id);
    if (it == subscribers_.end() || !it->second || !it->second->IsFunction())
      return true;

    // Pass the inner event object (sequence, emitted_at_ms, payload) to the
    // JS callback — same shape the caller expects from a real GIPS event.
    auto j = nlohmann::json::parse(event_str, nullptr, false);
    const auto& event_obj = j.is_discarded() ? j : j.value("event", j);

    if (!main_context_)
      return true;
    main_context_->Enter();
    CefV8ValueList v8args;
    v8args.push_back(CefV8Value::CreateString(event_obj.dump()));
    it->second->ExecuteFunctionWithContext(main_context_, nullptr, v8args);
    main_context_->Exit();
    return true;
  }

  return render_message_router_->OnProcessMessageReceived(
      browser, frame, source_process, message);
}

}  // namespace cronymax
