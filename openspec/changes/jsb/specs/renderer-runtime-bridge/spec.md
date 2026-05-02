## ADDED Requirements

### Requirement: Renderer service binding

The Rust runtime binary SHALL bind a second GIPS service named `ai.cronymax.runtime.renderer` on startup, separate from the existing `ai.cronymax.runtime` browser-process service. The two services SHALL share the same `RuntimeAuthority` via `attach_transport` called twice.

#### Scenario: Second service binds without conflicting with browser service

- **WHEN** `cronymax-runtime` starts
- **THEN** both `ai.cronymax.runtime` and `ai.cronymax.runtime.renderer` are registered in the Mach bootstrap namespace
- **THEN** each service operates with an independent `GipsTransport` and `ResponseSink`

#### Scenario: Renderer client connects independently

- **WHEN** a renderer process calls `crony_client_new("ai.cronymax.runtime.renderer")`
- **THEN** a new dispatch session is established on the renderer transport without affecting the browser-process session on `ai.cronymax.runtime`

---

### Requirement: Helper binary links crony client library

The `cronymax_app_helper` CEF helper binary SHALL be linked against `libcrony.a` (`Cronymax::Crony`) and the required macOS system frameworks (CoreFoundation, Security, SystemConfiguration). No browser-process C++ code (BridgeHandler, SpaceManager, RuntimeProxy) SHALL be introduced into the helper binary via this linkage.

#### Scenario: Helper binary compiles with crony.h available

- **WHEN** `cronymax_app_helper` is built
- **THEN** it compiles successfully with `crony.h` in the include path and `libcrony.a` linked
- **THEN** no symbols from `cronymax_native` (BridgeHandler, SpaceManager, etc.) are present in the helper binary

---

### Requirement: Runtime bridge JS object injection

`RenderApp::OnContextCreated` SHALL inject `window.__runtimeBridge` into the V8 context of main-frame built-in pages. The object SHALL expose three methods: `send(method, params)`, `subscribe(topic, callback)`, and `reconnect()`.

#### Scenario: Object is available on DOMContentLoaded

- **WHEN** a built-in main frame's V8 context is created
- **THEN** `window.__runtimeBridge` is defined and non-null
- **THEN** `typeof window.__runtimeBridge.send === 'function'`
- **THEN** `typeof window.__runtimeBridge.subscribe === 'function'`

#### Scenario: Object is NOT injected in non-main frames

- **WHEN** a sub-frame's V8 context is created
- **THEN** `window.__runtimeBridge` is not injected (sub-frames inherit from parent's `window`)

---

### Requirement: Renderer send (renderer → runtime)

`window.__runtimeBridge.send(method, params)` SHALL dispatch a `ClientToRuntime` message to the runtime via the renderer's `crony_client_t` and return a `Promise` that resolves with the `RuntimeToClient` response payload.

#### Scenario: Successful send and response

- **WHEN** JS calls `window.__runtimeBridge.send("tools/list", {})`
- **THEN** the message is delivered to the Rust runtime as a `ClientToRuntime` frame
- **THEN** the Promise resolves with the runtime's response payload
- **THEN** the browser process is NOT involved in this exchange

#### Scenario: Send while disconnected

- **WHEN** `renderer_client_` is null (runtime not yet connected or mid-reconnect)
- **THEN** `send()` returns a rejected Promise with an error indicating the bridge is not ready

---

### Requirement: Renderer subscribe (runtime → renderer)

`window.__runtimeBridge.subscribe(topic, callback)` SHALL register `callback` to be invoked whenever the runtime pushes a `RuntimeToClient` event matching `topic`. It SHALL return an unsubscribe function.

#### Scenario: Event delivered to subscriber

- **WHEN** the runtime emits an event on topic `"space/123/events"`
- **THEN** all JS callbacks registered via `subscribe("space/123/events", cb)` are invoked with the event payload
- **THEN** the event is NOT re-delivered via `window.__aiDesktopDispatch` for the same topic (no double-delivery)

#### Scenario: Unsubscribe stops delivery

- **WHEN** JS calls the unsubscribe function returned by `subscribe`
- **THEN** subsequent events on that topic do NOT invoke the previously registered callback

#### Scenario: Multiple subscribers on same topic

- **WHEN** two JS callbacks are registered for the same topic
- **THEN** both are invoked when an event arrives on that topic

---

### Requirement: Pump thread event dispatch

The renderer process SHALL run a dedicated pump thread that calls `crony_client_recv` in a loop and dispatches `RuntimeToClient` frames to the JS layer via `CefPostTask(TID_RENDERER, ...)` → `frame->ExecuteJavaScript`. The pump thread SHALL exit cleanly when a recv error occurs or when a shutdown flag is set.

#### Scenario: Pump thread routes events to JS

- **WHEN** the runtime pushes a `RuntimeToClient` event frame
- **THEN** the pump thread receives it and posts a task to the render thread
- **THEN** the render thread executes JS that routes the payload to registered `subscribe` callbacks

#### Scenario: Pump thread exits on connection error

- **WHEN** `crony_client_recv` returns an error (e.g., runtime process killed)
- **THEN** the pump thread sets `renderer_client_ = nullptr` (atomically) and exits
- **THEN** no further recv calls are made until reconnect

---

### Requirement: Reconnect on space switch

When the runtime restarts due to a space switch, the renderer SHALL reconnect the `crony_client_t` handle. Reconnection SHALL be triggered by the `space.switch_loading: false` event delivered via `window.__aiDesktopDispatch`. The reconnect SHALL: close the stale client, call `crony_client_new` on `ai.cronymax.runtime.renderer`, complete the `Hello`/`Welcome` handshake, re-subscribe to any active topics, and restart the pump thread.

#### Scenario: Bridge reconnects after space switch

- **WHEN** `space.switch_loading` transitions to `false` (new runtime is ready)
- **THEN** `window.__runtimeBridge.reconnect()` is called
- **THEN** a new `crony_client_t` is created and connected to `ai.cronymax.runtime.renderer`
- **THEN** the pump thread is restarted
- **THEN** `send()` and `subscribe()` resume working within one round-trip of reconnection

#### Scenario: Queued sends during reconnect are rejected

- **WHEN** JS calls `send()` while `renderer_client_` is null (mid-reconnect)
- **THEN** the Promise rejects immediately with a "bridge not ready" error
- **THEN** the bridge does NOT queue the call silently

---

### Requirement: `window.__runtimeBridge` scoped to built-in pages only

The injection SHALL only occur for frames whose URL matches the app's built-in page origin (e.g., `chrome://` / custom scheme used by the app). External URLs SHALL NOT receive the injection.

#### Scenario: Built-in page receives bridge

- **WHEN** `OnContextCreated` fires for a frame with the built-in page URL scheme
- **THEN** `window.__runtimeBridge` is injected

#### Scenario: External URL does not receive bridge

- **WHEN** `OnContextCreated` fires for a frame with an `https://` external URL
- **THEN** `window.__runtimeBridge` is NOT injected
