#include "renderer/app.h"

#include "browser/webview_scheme.h"

#include <mutex>
#include <random>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

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
static constexpr char kMsgBrowserEvent[] = "cronymax.browser.event";
// Extension webview frame → browser process. Mirrors the constants in
// `browser/bridge_handler.h` but is declared locally so the renderer
// compilation unit doesn't pull in the full browser bridge header.
static constexpr char kMsgWebviewPost[] = "cronymax.webview.post";
static constexpr char kMsgWebviewDeliver[] = "cronymax.webview.deliver";
// Content-renderer iframe → browser process (P6.5). Same locality
// rationale as the webview constants above.
static constexpr char kMsgRendererSetHeight[] = "cronymax.renderer.setHeight";

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
// V8 handler: acquireCronymaxApi().postMessage(payload) → Promise<void>
//
// Installed onto cronymax-webview://<ext-id>/<entry>?surface=panel&id=<id>
// iframes only. Bridges to the browser process via the kMsgWebviewPost process
// message carrying the panelId and a msgpack-encoded payload. The browser-side
// BridgeHandler::HandleWebviewPost looks up the panel's owning extension
// (via the WebviewRegistry) and calls
// `ExtensionRuntime::forward_panel_message`, which lands a
// `webview/onDidReceiveMessage` notify on the extension's Node host.
//
// Per-panel state (panel_id, ext_id) is captured at construction time,
// derived from the frame URL at OnContextCreated time. The promise
// resolves as soon as the process message has been dispatched — the
// IDL's `postMessage()` contract is "queue the payload", not "wait for
// the extension to handle it".
// ---------------------------------------------------------------------------

class WebviewPostHandler : public CefV8Handler {
 public:
  WebviewPostHandler(std::string panel_id, std::string ext_id)
      : panel_id_(std::move(panel_id)), ext_id_(std::move(ext_id)) {}

  bool Execute(const CefString& /*name*/,
               CefRefPtr<CefV8Value> /*object*/,
               const CefV8ValueList& arguments,
               CefRefPtr<CefV8Value>& retval,
               CefString& exception) override {
    if (arguments.empty()) {
      exception = "postMessage requires a payload argument";
      return true;
    }
    auto context = CefV8Context::GetCurrentContext();
    auto promise = CefV8Value::CreatePromise();
    if (!promise) {
      exception = "postMessage: failed to create Promise";
      return true;
    }
    retval = promise;

    const auto j = V8ToJson(arguments[0]);
    const auto bytes = nlohmann::json::to_msgpack(j);

    auto msg = CefProcessMessage::Create(kMsgWebviewPost);
    auto args = msg->GetArgumentList();
    args->SetString(0, panel_id_);
    args->SetString(1, ext_id_);
    args->SetBinary(2, CefBinaryValue::Create(bytes.data(), bytes.size()));
    context->GetFrame()->SendProcessMessage(PID_BROWSER, msg);

    // Fire-and-forget on the renderer side; the IDL promises only
    // "queued for delivery", and there is no per-message ACK on the
    // wire that would let us reject on bridge errors.
    promise->ResolvePromise(CefV8Value::CreateUndefined());
    return true;
  }

 private:
  std::string panel_id_;
  std::string ext_id_;

  IMPLEMENT_REFCOUNTING(WebviewPostHandler);
};

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

void App::OnRegisterCustomSchemes(CefRawPtr<CefSchemeRegistrar> registrar) {
  cronymax::RegisterWebviewScheme(registrar);
}

// Check whether a frame URL belongs to built-in pages.
// Production: built-in panels are served from file:// (or a custom scheme),
// so any http(s):// URL is an external site that must not see the bridge.
// Dev (CRONYMAX_DEV=1): main_window.cc rewrites ResourceUrl() to
// http://localhost:5173/<path>; we mirror that exact prefix here so external
// tabs (https://example.com, …) still can't see the bridge.
//
// **Extension webview URLs** (`cronymax-webview://...`) are deliberately
// NOT classified as built-in — they get a separate, tightly-scoped
// `acquireCronymaxApi()` surface via [`IsExtensionWebviewUrl`], not the
// full `cronymax.runtime` / `cronymax.browser` IPC.
// CEF helper processes inherit the host's env, so getenv works here.
static bool IsBuiltinUrl(const CefString& url) {
  static const char* const kDevPanelPrefix = "http://localhost:5173/";
  static const char* const kExtSchemePrefix = "cronymax-webview://";
  static const bool dev_mode = [] {
    const char* v = std::getenv("CRONYMAX_DEV");
    return v && *v;
  }();
  std::string u = url.ToString();
  if (u.rfind(kExtSchemePrefix, 0) == 0)
    return false;  // separate surface
  if (dev_mode && u.rfind(kDevPanelPrefix, 0) == 0)
    return true;
  return u.rfind("https://", 0) != 0 && u.rfind("http://", 0) != 0;
}

// Returns true for `cronymax-webview://<ext-id>/<path>` frames — the
// extension-webview sandbox surface. Used to scope `acquireCronymaxApi`
// injection to those frames only.
static bool IsExtensionWebviewUrl(const CefString& url) {
  static const char* const kExtSchemePrefix = "cronymax-webview://";
  std::string u = url.ToString();
  return u.rfind(kExtSchemePrefix, 0) == 0;
}

// Parse `cronymax-webview://<ext-id>/<path>` and return the `<ext-id>`
// component. Returns empty string if the URL is malformed.
static std::string ExtractWebviewExtensionId(const CefString& url) {
  static const std::string kPrefix = "cronymax-webview://";
  std::string u = url.ToString();
  if (u.rfind(kPrefix, 0) != 0)
    return std::string();
  std::string tail = u.substr(kPrefix.size());
  size_t slash = tail.find('/');
  return (slash == std::string::npos) ? tail : tail.substr(0, slash);
}

void App::OnContextCreated(CefRefPtr<CefBrowser> browser,
                           CefRefPtr<CefFrame> frame,
                           CefRefPtr<CefV8Context> context) {
  render_message_router_->OnContextCreated(browser, frame, context);

  const CefString frame_url = frame->GetURL();

  // ── Extension webview iframe surface ──────────────────────────────────
  // Frames loaded from `cronymax-webview://...` get a tightly-scoped
  // `acquireCronymaxApi()` global instead of the full cronymax.runtime /
  // cronymax.browser surface. They can post messages to the owning
  // extension (and receive replies); they cannot reach any built-in IPC.
  if (IsExtensionWebviewUrl(frame_url)) {
    InjectAcquireCronymaxApi(frame, context);
    return;
  }

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

  // Capture the main-frame V8 context for every main frame so async replies
  // can enter it to resolve pending promises. Without this, browser-ctrl
  // replies (kMsgBrowserCtrlReply) for http://localhost dev URLs would early-
  // out at `if (!main_context_) return true;` and JS-side awaits would hang.
  if (!frame->IsMain())
    return;
  main_context_ = context;

  // Inject window.cronymax.runtime only into built-in main frames — external
  // pages (https://...) must not see the runtime IPC surface.
  if (!IsBuiltinUrl(frame_url))
    return;

  // Ensure window.cronymax exists; bridge.ts adds .browser to the same object.
  CefRefPtr<CefV8Value> global = context->GetGlobal();
  CefRefPtr<CefV8Value> cronymax_obj = global->GetValue("cronymax");
  if (!cronymax_obj || !cronymax_obj->IsObject()) {
    cronymax_obj = CefV8Value::CreateObject(nullptr, nullptr);
    global->SetValue("cronymax", cronymax_obj, V8_PROPERTY_ATTRIBUTE_NONE);
  }

  // Build window.cronymax.runtime = { send, on }
  // `send` communicates with the Rust runtime via CEF process messages
  // (cronymax.runtime.ctrl).  `on` is a JS-settable callback;
  // bridge.ts assigns the actual function and the C++ renderer calls it
  // when kMsgRuntimeEvent arrives.
  CefRefPtr<CefV8Value> runtime_obj =
      CefV8Value::CreateObject(nullptr, nullptr);

  runtime_obj->SetValue(
      "send", CefV8Value::CreateFunction("send", new RuntimeCtrlHandler(this)),
      V8_PROPERTY_ATTRIBUTE_NONE);

  // Placeholder; bridge.ts replaces this with the real dispatch function.
  runtime_obj->SetValue("on", CefV8Value::CreateNull(),
                        V8_PROPERTY_ATTRIBUTE_NONE);

  cronymax_obj->SetValue("runtime", runtime_obj, V8_PROPERTY_ATTRIBUTE_NONE);
}

void App::OnContextReleased(CefRefPtr<CefBrowser> browser,
                            CefRefPtr<CefFrame> frame,
                            CefRefPtr<CefV8Context> context) {
  render_message_router_->OnContextReleased(browser, frame, context);

  if (frame->IsMain()) {
    pending_runtime_ctrl_callbacks_.clear();
    pending_browser_ctrl_callbacks_.clear();
    main_context_ = nullptr;
  }

  // Drop any webview frame context whose V8 context we just lost. Frames
  // can outlive the panel registry entry (the renderer process may keep
  // serving an iframe after the platform disposed its registry row), so
  // we identify the frame by V8 context identity rather than panel id.
  for (auto it = webview_frames_.begin(); it != webview_frames_.end();) {
    if (it->second.context && it->second.context->IsSame(context)) {
      it = webview_frames_.erase(it);
    } else {
      ++it;
    }
  }
}

// ---------------------------------------------------------------------------
// Extension webview API injection.
//
// Builds a per-frame `acquireCronymaxApi` global. Each call returns the
// same panel-scoped object (a la VS Code's `acquireVsCodeApi()` model)
// — calling it twice in the same iframe throws, matching VS Code's
// "exactly one handle per iframe" convention.
//
// The handler classes are defined at file scope (not as locals of the
// injection function) because C++ doesn't permit one local class to
// name another that's declared later in the same function.
// ---------------------------------------------------------------------------

class WebviewStateHandler : public CefV8Handler {
 public:
  WebviewStateHandler(App* app, std::string panel_id, bool set)
      : app_(app), panel_id_(std::move(panel_id)), set_(set) {}
  bool Execute(const CefString& /*name*/,
               CefRefPtr<CefV8Value> /*object*/,
               const CefV8ValueList& arguments,
               CefRefPtr<CefV8Value>& retval,
               CefString& /*exception*/) override {
    auto it = app_->webview_frames_.find(panel_id_);
    if (it == app_->webview_frames_.end()) {
      retval = CefV8Value::CreateUndefined();
      return true;
    }
    if (set_) {
      it->second.state =
          arguments.empty() ? CefV8Value::CreateUndefined() : arguments[0];
      retval = CefV8Value::CreateUndefined();
    } else {
      retval =
          it->second.state ? it->second.state : CefV8Value::CreateUndefined();
    }
    return true;
  }

 private:
  App* app_;
  std::string panel_id_;
  bool set_;
  IMPLEMENT_REFCOUNTING(WebviewStateHandler);
};

class WebviewDisposeHandler : public CefV8Handler {
 public:
  WebviewDisposeHandler(App* app, std::string panel_id, int index)
      : app_(app), panel_id_(std::move(panel_id)), index_(index) {}
  bool Execute(const CefString&,
               CefRefPtr<CefV8Value>,
               const CefV8ValueList&,
               CefRefPtr<CefV8Value>&,
               CefString&) override {
    auto it = app_->webview_frames_.find(panel_id_);
    if (it != app_->webview_frames_.end() && it->second.on_message_handlers &&
        it->second.on_message_handlers->IsArray() &&
        index_ < it->second.on_message_handlers->GetArrayLength()) {
      it->second.on_message_handlers->SetValue(index_,
                                               CefV8Value::CreateNull());
    }
    return true;
  }

 private:
  App* app_;
  std::string panel_id_;
  int index_;
  IMPLEMENT_REFCOUNTING(WebviewDisposeHandler);
};

class WebviewOnMessageHandler : public CefV8Handler {
 public:
  WebviewOnMessageHandler(App* app, std::string panel_id)
      : app_(app), panel_id_(std::move(panel_id)) {}
  bool Execute(const CefString& /*name*/,
               CefRefPtr<CefV8Value> /*object*/,
               const CefV8ValueList& arguments,
               CefRefPtr<CefV8Value>& retval,
               CefString& exception) override {
    if (arguments.empty() || !arguments[0]->IsFunction()) {
      exception = "onDidReceiveMessage expects a function";
      return true;
    }
    int idx = 0;
    auto it = app_->webview_frames_.find(panel_id_);
    if (it != app_->webview_frames_.end() && it->second.on_message_handlers &&
        it->second.on_message_handlers->IsArray()) {
      idx = it->second.on_message_handlers->GetArrayLength();
      it->second.on_message_handlers->SetValue(idx, arguments[0]);
    }
    // Return a Disposable-shaped object so the SDK consumer can drop
    // listeners — we mark the slot null rather than splicing the array
    // so other indexes stay stable across dispose calls.
    auto disposable = CefV8Value::CreateObject(nullptr, nullptr);
    disposable->SetValue(
        "dispose",
        CefV8Value::CreateFunction(
            "dispose", new WebviewDisposeHandler(app_, panel_id_, idx)),
        V8_PROPERTY_ATTRIBUTE_NONE);
    retval = disposable;
    return true;
  }

 private:
  App* app_;
  std::string panel_id_;
  IMPLEMENT_REFCOUNTING(WebviewOnMessageHandler);
};

class WebviewAcquireHandler : public CefV8Handler {
 public:
  WebviewAcquireHandler(App* app, std::string panel_id, std::string ext_id)
      : app_(app), panel_id_(std::move(panel_id)), ext_id_(std::move(ext_id)) {}
  bool Execute(const CefString& /*name*/,
               CefRefPtr<CefV8Value> /*object*/,
               const CefV8ValueList& /*arguments*/,
               CefRefPtr<CefV8Value>& retval,
               CefString& exception) override {
    if (consumed_) {
      exception = "acquireCronymaxApi: already acquired in this frame";
      return true;
    }
    consumed_ = true;
    auto api = CefV8Value::CreateObject(nullptr, nullptr);
    api->SetValue(
        "postMessage",
        CefV8Value::CreateFunction("postMessage",
                                   new WebviewPostHandler(panel_id_, ext_id_)),
        V8_PROPERTY_ATTRIBUTE_NONE);
    api->SetValue(
        "setState",
        CefV8Value::CreateFunction(
            "setState", new WebviewStateHandler(app_, panel_id_, /*set=*/true)),
        V8_PROPERTY_ATTRIBUTE_NONE);
    api->SetValue("getState",
                  CefV8Value::CreateFunction(
                      "getState",
                      new WebviewStateHandler(app_, panel_id_, /*set=*/false)),
                  V8_PROPERTY_ATTRIBUTE_NONE);
    api->SetValue("onDidReceiveMessage",
                  CefV8Value::CreateFunction(
                      "onDidReceiveMessage",
                      new WebviewOnMessageHandler(app_, panel_id_)),
                  V8_PROPERTY_ATTRIBUTE_NONE);
    retval = api;
    return true;
  }

 private:
  App* app_;
  std::string panel_id_;
  std::string ext_id_;
  bool consumed_ = false;
  IMPLEMENT_REFCOUNTING(WebviewAcquireHandler);
};

// ---------------------------------------------------------------------------
// Content-renderer iframe V8 handlers (Phase 6.5)
// ---------------------------------------------------------------------------
//
// `cronymax-webview://<ext>/<entry>?surface=renderer&id=<inst>` iframes
// get a single V8 native primitive: `setHeight(px)` — JS reports the
// rendered content height through this so the cross-origin parent
// React surface can size the embedding `<iframe>` element.
//
// Everything else the IDL `RendererContext` / `RendererActivationApi`
// exposes (acquired-once `activate(fn)`, the postMessage-driven render
// dispatch loop) is built in pure JS inside the bootstrap shim injected
// via `frame->ExecuteJavaScript`. The split is deliberate: only
// IPC-crossing things go in V8.

class RendererSetHeightHandler : public CefV8Handler {
 public:
  RendererSetHeightHandler(std::string instance_id)
      : instance_id_(std::move(instance_id)) {}

  bool Execute(const CefString& /*name*/,
               CefRefPtr<CefV8Value> /*object*/,
               const CefV8ValueList& arguments,
               CefRefPtr<CefV8Value>& retval,
               CefString& exception) override {
    if (arguments.empty() ||
        (!arguments[0]->IsDouble() && !arguments[0]->IsInt() &&
         !arguments[0]->IsUInt())) {
      exception = "setHeight(px): expected a number";
      return true;
    }
    int px = 0;
    if (arguments[0]->IsInt()) {
      px = arguments[0]->GetIntValue();
    } else if (arguments[0]->IsUInt()) {
      px = static_cast<int>(arguments[0]->GetUIntValue());
    } else {
      // Round JS doubles defensively; CSS heights are integers in
      // practice and Rust on the other end of the bridge takes i32.
      const double d = arguments[0]->GetDoubleValue();
      if (d < 0) {
        px = 0;
      } else if (d > static_cast<double>(INT32_MAX)) {
        px = INT32_MAX;
      } else {
        px = static_cast<int>(d + 0.5);
      }
    }

    auto context = CefV8Context::GetCurrentContext();
    if (!context) {
      retval = CefV8Value::CreateUndefined();
      return true;
    }
    auto msg = CefProcessMessage::Create(kMsgRendererSetHeight);
    auto args = msg->GetArgumentList();
    args->SetString(0, instance_id_);
    args->SetInt(1, px);
    context->GetFrame()->SendProcessMessage(PID_BROWSER, msg);
    retval = CefV8Value::CreateUndefined();
    return true;
  }

 private:
  std::string instance_id_;
  IMPLEMENT_REFCOUNTING(RendererSetHeightHandler);
};

namespace {

// Percent-decode the small set Rust's `url_for_surface` encodes. Lenient:
// a malformed `%xx` triplet is passed through byte-for-byte (we don't have
// a richer error channel here, and the Rust-emitted URLs only ever contain
// uppercase hex from `format!("{:02X}", ...)`).
std::string PercentDecode(std::string_view raw) {
  const auto from_hex = [](char c) -> int {
    if (c >= '0' && c <= '9')
      return c - '0';
    if (c >= 'A' && c <= 'F')
      return 10 + (c - 'A');
    if (c >= 'a' && c <= 'f')
      return 10 + (c - 'a');
    return -1;
  };
  std::string out;
  out.reserve(raw.size());
  for (size_t i = 0; i < raw.size(); ++i) {
    if (raw[i] == '%' && i + 2 < raw.size()) {
      int hi = from_hex(raw[i + 1]);
      int lo = from_hex(raw[i + 2]);
      if (hi >= 0 && lo >= 0) {
        out.push_back(static_cast<char>((hi << 4) | lo));
        i += 2;
        continue;
      }
    }
    out.push_back(raw[i]);
  }
  return out;
}

// Parse `?k1=v1&k2=v2` query string into a vector of (key, decoded-value)
// pairs preserving order. Values are percent-decoded; keys are not (they
// are platform constants like `surface` / `id`).
//
// Whatever sits between '?' and '#' (or end-of-string) is treated as the
// query; pairs without '=' are kept with an empty value.
std::vector<std::pair<std::string, std::string>> ParseQuery(
    const std::string& url) {
  std::vector<std::pair<std::string, std::string>> out;
  const size_t qpos = url.find('?');
  if (qpos == std::string::npos)
    return out;
  size_t end = url.find('#', qpos);
  if (end == std::string::npos)
    end = url.size();
  size_t pos = qpos + 1;
  while (pos < end) {
    size_t amp = url.find('&', pos);
    if (amp == std::string::npos || amp > end)
      amp = end;
    const std::string_view pair_sv(url.data() + pos, amp - pos);
    if (!pair_sv.empty()) {
      const size_t eq = pair_sv.find('=');
      if (eq == std::string_view::npos) {
        out.emplace_back(std::string(pair_sv), std::string());
      } else {
        std::string key(pair_sv.substr(0, eq));
        std::string value = PercentDecode(pair_sv.substr(eq + 1));
        out.emplace_back(std::move(key), std::move(value));
      }
    }
    pos = amp + 1;
  }
  return out;
}

std::string LookupQuery(
    const std::vector<std::pair<std::string, std::string>>& q,
    std::string_view key) {
  for (const auto& [k, v] : q) {
    if (k == key)
      return v;
  }
  return {};
}

}  // namespace

void App::InjectAcquireCronymaxApi(CefRefPtr<CefFrame> frame,
                                   CefRefPtr<CefV8Context> context) {
  const std::string url = frame->GetURL().ToString();
  const std::string ext_id = ExtractWebviewExtensionId(url);
  if (ext_id.empty())
    return;

  // Parse the `?surface=<kind>&id=<value>` query the Rust-side `url_for_
  // surface` always emits. Two surfaces are recognised in v1:
  //
  //   * `panel`    → install `acquireCronymaxApi()` (P6 webview panels)
  //   * `renderer` → install `acquireCronymaxRendererApi()` (P6.5 content
  //                  renderer iframes; injection itself lands in T04)
  //
  // A missing / empty / unknown surface is REJECTED: we do not write the
  // frame into `webview_frames_` (an empty key would collide across all
  // such frames) and no SDK is injected. The iframe loads, but any attempt
  // to acquire the API throws ReferenceError — a louder, easier-to-debug
  // failure than silent cross-frame state corruption.
  const auto query = ParseQuery(url);
  const std::string surface = LookupQuery(query, "surface");
  const std::string id = LookupQuery(query, "id");

  if (surface.empty() || id.empty()) {
    // Malformed URL (no surface=... or no id=...). Do nothing — neither
    // SDK is injected and the frame is not registered.
    return;
  }

  if (surface == "panel") {
    WebviewFrameContext fc;
    fc.context = context;
    fc.panel_id = id;
    fc.ext_id = ext_id;
    fc.on_message_handlers = CefV8Value::CreateArray(0);
    fc.state = CefV8Value::CreateUndefined();
    webview_frames_[id] = std::move(fc);

    CefRefPtr<CefV8Value> global = context->GetGlobal();
    global->SetValue(
        "acquireCronymaxApi",
        CefV8Value::CreateFunction("acquireCronymaxApi",
                                   new WebviewAcquireHandler(this, id, ext_id)),
        V8_PROPERTY_ATTRIBUTE_NONE);
    return;
  }

  if (surface == "renderer") {
    // P6.5-T04: install `acquireCronymaxRendererApi()` for content-
    // renderer iframes. We expose ONE V8 native primitive
    // (`setHeight(px)`) plus metadata as a hidden
    // `__cronymax_renderer_native__` global; everything else (the
    // acquired-once `activate(fn)` wrapper and the postMessage-driven
    // render dispatch loop) is a JS bootstrap defined via
    // `frame->ExecuteJavaScript` below. Splitting that way keeps the
    // V8 surface to the bare minimum that actually needs to cross the
    // process boundary.
    CefRefPtr<CefV8Value> native = CefV8Value::CreateObject(nullptr, nullptr);
    native->SetValue("extensionId", CefV8Value::CreateString(ext_id),
                     V8_PROPERTY_ATTRIBUTE_READONLY);
    native->SetValue("instanceId", CefV8Value::CreateString(id),
                     V8_PROPERTY_ATTRIBUTE_READONLY);
    // Theme is constant for v1 alpha; the IDL `onDidChangeTheme` event
    // returns an immediately-disposable noop from the JS shim. Theme
    // sync will be wired in P6.5-T08 / a later task.
    native->SetValue("theme", CefV8Value::CreateString("light"),
                     V8_PROPERTY_ATTRIBUTE_READONLY);
    native->SetValue("setHeight",
                     CefV8Value::CreateFunction(
                         "setHeight", new RendererSetHeightHandler(id)),
                     V8_PROPERTY_ATTRIBUTE_READONLY);

    CefRefPtr<CefV8Value> global = context->GetGlobal();
    global->SetValue("__cronymax_renderer_native__", native,
                     V8_PROPERTY_ATTRIBUTE_DONTENUM);

    // JS bootstrap shim. Defines `acquireCronymaxRendererApi()` and
    // installs the message listener that drives `renderItem` /
    // `updateItem` / `disposeItem` from parent → iframe postMessage
    // (per P6.5 IDL D9: streaming payloads bypass Rust IPC).
    //
    // Message contract:
    //   { type: "cronymax:renderer:render",  request: RenderRequest }
    //   { type: "cronymax:renderer:update",  request: RenderRequest }
    //   { type: "cronymax:renderer:dispose", instanceId: string      }
    //
    // The parent React surface (P6.5-T08 `<ExtensionContentBlock>`)
    // posts these via `iframe.contentWindow.postMessage(msg,
    // "cronymax-webview://<ext>")`. We accept messages whose origin
    // begins with `cronymax-webview://` (or any messages if the
    // platform sends from a non-scheme origin, which it currently does
    // — chat React runs from `file://` or the built-in app chrome).
    static constexpr char kRendererBootstrap[] =
        R"((function () {
  var native = globalThis.__cronymax_renderer_native__;
  if (!native) return;

  var acquired = false;
  var activateFn = null;
  var rendererApi = null;
  var rendererApiPromise = null;
  var pendingMessages = [];

  function makeDisposable() { return { dispose: function () {} }; }

  function buildCtx() {
    return Object.freeze({
      rendererId: '',
      extensionId: native.extensionId,
      theme: native.theme,
      onDidChangeTheme: function () { return makeDisposable(); },
      setHeight: function (px) {
        try { native.setHeight(px | 0); } catch (e) { /* ignore */ }
      },
    });
  }

  globalThis.acquireCronymaxRendererApi = function () {
    if (acquired) {
      throw new Error('acquireCronymaxRendererApi: already acquired');
    }
    acquired = true;
    return Object.freeze({
      activate: function (fn) {
        if (activateFn) {
          throw new Error('RendererActivationApi.activate: already called');
        }
        if (typeof fn !== 'function') {
          throw new TypeError('activate(fn): fn must be a function');
        }
        activateFn = fn;
        flushPending();
      },
    });
  };

  function ensureRenderer() {
    if (rendererApi) return Promise.resolve(rendererApi);
    if (rendererApiPromise) return rendererApiPromise;
    if (!activateFn) return Promise.resolve(null);
    rendererApiPromise = Promise.resolve()
      .then(function () { return activateFn(buildCtx()); })
      .then(function (api) {
        rendererApi = api || {};
        return rendererApi;
      })
      .catch(function (e) {
        console.error('[cronymax renderer] activate() failed:', e);
        rendererApi = {};
        return rendererApi;
      });
    return rendererApiPromise;
  }

  function dispatch(msg) {
    if (!msg || typeof msg !== 'object') return;
    var t = msg.type;
    if (typeof t !== 'string') return;
    if (t === 'cronymax:renderer:dispose') {
      var inst = msg.instanceId || native.instanceId;
      if (rendererApi && typeof rendererApi.disposeItem === 'function') {
        try { rendererApi.disposeItem(inst); } catch (e) { console.error(e); }
      }
      return;
    }
    if (t !== 'cronymax:renderer:render' && t !== 'cronymax:renderer:update') {
      return;
    }
    ensureRenderer().then(function (api) {
      if (!api) return;  // activate not yet called — message already buffered
      var element = document.body;
      var token = Object.freeze({
        isCancellationRequested: false,
        onCancellationRequested: function () { return makeDisposable(); },
      });
      var fn = (t === 'cronymax:renderer:update' && typeof api.updateItem === 'function')
        ? api.updateItem
        : api.renderItem;
      if (typeof fn !== 'function') return;
      try { fn.call(api, element, msg.request, token); }
      catch (e) { console.error('[cronymax renderer]', e); }
    });
  }

  function flushPending() {
    var batch = pendingMessages;
    pendingMessages = [];
    for (var i = 0; i < batch.length; i++) dispatch(batch[i]);
  }

  window.addEventListener('message', function (event) {
    var data = event.data;
    if (!data || typeof data !== 'object' || typeof data.type !== 'string') {
      return;
    }
    if (!data.type.startsWith('cronymax:renderer:')) return;
    if (!activateFn) {
      pendingMessages.push(data);
      return;
    }
    dispatch(data);
  });
})();)";

    frame->ExecuteJavaScript(kRendererBootstrap,
                             "cronymax://renderer-bootstrap", 0);
    return;
  }

  // Unknown surface — same outcome as missing: no SDK, no registration.
}

void App::DispatchWebviewDelivery(const std::string& panel_id,
                                  const std::vector<uint8_t>& payload_msgpack) {
  auto it = webview_frames_.find(panel_id);
  if (it == webview_frames_.end())
    return;
  if (!it->second.context)
    return;
  it->second.context->Enter();
  auto j = nlohmann::json::from_msgpack(payload_msgpack, true, false);
  CefRefPtr<CefV8Value> payload =
      j.is_discarded() ? CefV8Value::CreateNull() : JsonToV8(j);
  auto handlers = it->second.on_message_handlers;
  if (handlers && handlers->IsArray()) {
    const int n = handlers->GetArrayLength();
    for (int i = 0; i < n; ++i) {
      auto h = handlers->GetValue(i);
      if (!h || !h->IsFunction())
        continue;
      CefV8ValueList args;
      args.push_back(payload);
      h->ExecuteFunctionWithContext(it->second.context, nullptr, args);
    }
  }
  it->second.context->Exit();
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
    // args[1] is a CefBinaryValue (msgpack-encoded response).
    const bool is_error = msg_args->GetBool(2);

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
          if (j.is_object() && j.contains("error")) {
            const auto& err_val = j["error"];
            if (err_val.is_object())
              err_msg = err_val.value("message", j.dump());
            else if (err_val.is_string())
              err_msg = err_val.get<std::string>();
            else
              err_msg = j.dump();
          } else {
            err_msg = j.dump();
          }
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
          if (j.is_object() && j.contains("error")) {
            const auto& err_val = j["error"];
            if (err_val.is_object())
              err_msg = err_val.value("message", j.dump());
            else if (err_val.is_string())
              err_msg = err_val.get<std::string>();
            else
              err_msg = j.dump();
          } else {
            err_msg = j.dump();
          }
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
  // Browser sends kMsgRuntimeEvent(sub_id, inner_event_json) for each active
  // renderer subscription.  Forward to window.cronymax.runtime.on so
  // JS can route events to the correct subscriber.
  // The inner event JSON is decoded to a V8 object so callers receive a
  // plain JS object rather than a raw string.
  if (name == kMsgRuntimeEvent) {
    auto msg_args = message->GetArgumentList();
    const std::string sub_id = msg_args->GetString(0).ToString();
    const std::string event_str = msg_args->GetString(1).ToString();

    if (!main_context_)
      return true;
    main_context_->Enter();
    auto global = main_context_->GetGlobal();
    auto cronymax = global->GetValue("cronymax");
    if (cronymax && cronymax->IsObject()) {
      auto rt = cronymax->GetValue("runtime");
      if (rt && rt->IsObject()) {
        auto on_dispatch = rt->GetValue("on");
        if (on_dispatch && on_dispatch->IsFunction()) {
          auto j = nlohmann::json::parse(event_str, nullptr, false);
          CefV8ValueList v8args;
          v8args.push_back(CefV8Value::CreateString(sub_id));
          v8args.push_back(j.is_discarded() ? CefV8Value::CreateNull()
                                            : JsonToV8(j));
          on_dispatch->ExecuteFunctionWithContext(main_context_, nullptr,
                                                  v8args);
        }
      }
    }
    main_context_->Exit();
    return true;
  }

  // ── Browser event ────────────────────────────────────────────────────────
  // Browser sends kMsgBrowserEvent(event_name, payload_json) via the
  // refactored BridgeHandler::SendEvent (replaces ExecuteJavaScript injection).
  // The payload JSON is decoded to a V8 object before forwarding.
  if (name == kMsgBrowserEvent) {
    auto msg_args = message->GetArgumentList();
    const std::string event = msg_args->GetString(0).ToString();
    const std::string payload_str = msg_args->GetString(1).ToString();

    if (!main_context_)
      return true;
    main_context_->Enter();
    auto global = main_context_->GetGlobal();
    auto cronymax = global->GetValue("cronymax");
    if (cronymax && cronymax->IsObject()) {
      auto browser_obj = cronymax->GetValue("browser");
      if (browser_obj && browser_obj->IsObject()) {
        auto on_dispatch = browser_obj->GetValue("on");
        if (on_dispatch && on_dispatch->IsFunction()) {
          auto j = nlohmann::json::parse(payload_str, nullptr, false);
          CefV8ValueList v8args;
          v8args.push_back(CefV8Value::CreateString(event));
          v8args.push_back(j.is_discarded() ? CefV8Value::CreateNull()
                                            : JsonToV8(j));
          on_dispatch->ExecuteFunctionWithContext(main_context_, nullptr,
                                                  v8args);
        }
      }
    }
    main_context_->Exit();
    return true;
  }

  // ── Webview delivery ───────────────────────────────────────────────────
  // Browser sends kMsgWebviewDeliver(panel_id, ext_id, payload_msgpack)
  // whenever an extension calls `panel.postMessage(...)`. Find the matching
  // iframe frame and fan out to its onDidReceiveMessage listeners.
  if (name == kMsgWebviewDeliver) {
    auto msg_args = message->GetArgumentList();
    const std::string panel_id = msg_args->GetString(0).ToString();
    auto bin = msg_args->GetBinary(2);
    if (!bin)
      return true;
    std::vector<uint8_t> bytes(bin->GetSize());
    bin->GetData(bytes.data(), bytes.size(), 0);
    DispatchWebviewDelivery(panel_id, bytes);
    return true;
  }

  return render_message_router_->OnProcessMessageReceived(
      browser, frame, source_process, message);
}

}  // namespace cronymax
