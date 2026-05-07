## Context

Built-in pages today receive runtime events via `BridgeHandler::SendEvent` → `frame->ExecuteJavaScript("window.__aiDesktopDispatch(...)")`. This path routes every event through the browser process, preventing renderer pages from sending messages to the runtime without going through `CefMessageRouter` / `OnQuery`. The runtime restarts on every space switch (`SpaceManager::SwitchTo` → `Stop()`+`Start()` on a background thread).

Key architectural constraints:
- Only **built-in pages** (renderer frames served from the app bundle) are in scope — sandboxed web content is explicitly excluded.
- The Rust runtime exposes a GIPS IPC service (`ai.cronymax.runtime`) exclusively to the browser process today. The `GipsTransport` in `boundary.rs` maintains a **single `ReturnPath` slot**, making it unsuitable for two concurrent clients on the same service.
- The CEF helper binary (`cronymax_app_helper`) currently does **not** link `libcrony.a`; only `libcef_dll_wrapper` and CEF system libs are linked.
- `spawn_session` in `crates/cronymax/src/protocol/session.rs` is explicitly designed for multiple dispatch sessions sharing one `RuntimeAuthority`.
- On macOS, `crony_client_new` / `send` / `recv` are thin wrappers over GIPS `Endpoint::connect` (Mach bootstrap lookup), so the renderer process can connect to a named service with no OS-level restriction.

## Goals / Non-Goals

**Goals:**
- Renderer built-in pages can call `window.__runtimeBridge.send(method, params)` to invoke runtime control methods directly.
- Renderer built-in pages can call `window.__runtimeBridge.subscribe(topic, cb)` to receive runtime events without the browser process relay.
- The bridge is reconnect-safe across space switches.
- No changes to the single-slot `GipsTransport` / `ReturnPath` — each direction gets its own transport.

**Non-Goals:**
- Sandboxed / third-party web content.
- Replacing the browser-process `RuntimeBridge` (browser process retains its own session).
- Windows / Linux (macOS only for now, Mach-port based GIPS).
- Shared-memory or streaming payloads (JSON messages only).

## Decisions

### D1: Two separate GIPS service names

**Decision**: The runtime binds a second service `ai.cronymax.runtime.renderer` specifically for renderer clients; browser process keeps `ai.cronymax.runtime`.

**Rationale**: `GipsTransport`'s `ReturnPath` holds one `Connection` slot. Two clients on the same service would race — whichever sent last owns the reply slot. A second service + second `attach_transport` call gives each client its own `GipsTransport` + `ResponseSink` in the dispatch loop. The `RuntimeAuthority` is shared (`Arc`) so both sessions operate on the same subscription bus without duplication.

**Alternatives considered**:
- *Extend `GipsTransport` to multi-slot (`HashMap<ClientId, Connection>`)* — functionally equivalent but larger surface change to `boundary.rs` and the dispatch protocol. Deferred; D1 is strictly additive.
- *Reuse browser process as relay* — rejected; the whole point of this change is to remove that relay.

### D2: Renderer-side client lifecycle in `RenderApp`

**Decision**: `RenderApp` holds a `crony_client_t*` and a `std::thread` pump thread. `OnContextCreated` starts the connection and injects `window.__runtimeBridge`. The pump thread loops on `crony_client_recv`, dispatching to JS via `frame->ExecuteJavaScript`. On recv error, the pump thread sets `renderer_client_ = nullptr` (atomic) and exits.

**Rationale**: `RenderApp::OnContextCreated` is the natural V8 injection point (already used for `CefMessageRouterRendererSide`). A dedicated pump thread mirrors how `RuntimeBridge` in the browser process works today (pump thread calling `crony_client_recv`).

**Alternatives considered**:
- *Poll from the CEF render thread* — would block or require non-blocking recv with a timer; more complex than a dedicated thread.
- *Use `OnContextReleased` only for cleanup* — pump thread must exit before `crony_client_close` is called to avoid use-after-free. Shutdown sequence: set stop flag → close client → join thread.

### D3: Reconnect on space switch via `space.switch_loading`

**Decision**: The renderer subscribes to `space.switch_loading` events still delivered via `window.__aiDesktopDispatch` (browser-process-originated, works even when runtime is down). On `space.switch_loading: false`, JS calls a `reconnect()` binding which: closes the old client, calls `crony_client_new`, sends `Hello`, waits for `Welcome`, re-subscribes, restarts the pump thread.

**Rationale**: `space.switch_loading: false` already means "the new runtime is ready" — `RuntimeBridge::Start()` only completes after the browser-process handshake succeeds. This gives a reliable "runtime is up" signal without polling. `__aiDesktopDispatch` is a browser-process JS eval and is unaffected by the runtime restart.

**Alternatives considered**:
- *`CefProcessMessage` from browser process after `Start()` succeeds* — cleaner signal but requires adding a new IPC path between processes; the `space.switch_loading: false` event is equivalent and already exists.
- *Pump thread detects error + polls for service availability* — avoids any dependency on the loading event, but introduces polling and a timing window where the service might not yet be registered.

### D4: CMake linking change for helper binary

**Decision**: Add `Cronymax::Crony` (the IMPORTED STATIC target pointing at `libcrony.a`) to `cronymax_app_helper` in `cmake/CronymaxApp.cmake`. Add CoreFoundation, Security, SystemConfiguration frameworks (already present for browser process; required by GIPS on macOS).

**Rationale**: `Cronymax::Crony` is a thin INTERFACE target — it does not pull in `BridgeHandler`, `SpaceManager`, or any browser-process C++ code. Only `crony.h` + `libcrony.a` are added to the helper.

### D5: JS API surface (`window.__runtimeBridge`)

**Decision**:
```ts
window.__runtimeBridge = {
  send(method: string, params: unknown): Promise<unknown>,
  subscribe(topic: string, callback: (payload: unknown) => void): () => void,
  reconnect(): void,  // called by space.switch_loading handler
}
```
`send` maps to a `cefQuery` call that the renderer-side C++ dispatches as a `ClientToRuntime` message and returns the `RuntimeToClient` response via the callback.  
`subscribe` registers a JS callback in a local `Map<topic, Set<callback>>`; the pump thread dispatches `RuntimeToClient` event frames to all registered callbacks for the matching topic.

**Rationale**: Mirrors the existing `window.__aiDesktopDispatch` pattern but bidirectional. `reconnect()` is exposed so the space-switch handler in JS can trigger reconnection without a separate `cefQuery` round-trip.

## Risks / Trade-offs

- **Runtime restart timing window**: Between `space.switch_loading: false` reaching the renderer and `crony_client_new` completing, a brief window exists where no runtime connection is active. Any `send()` calls during reconnect should queue or reject gracefully.  
  → Mitigation: `send()` returns a rejected Promise if `renderer_client_ == nullptr`; UI should disable runtime-dependent actions while `space.switch_loading: true`.

- **Multiple renderer frames**: If multiple built-in frames exist simultaneously (e.g., popover + main panel), each `OnContextCreated` call would try to open its own `crony_client_t` connection to `ai.cronymax.runtime.renderer`. `GipsTransport`'s single-slot would be contested.  
  → Mitigation: Scope the renderer-side client to the **main frame only** (`frame->IsMain()` check in `OnContextCreated`). Sub-frames share the parent frame's JS context via `window.__runtimeBridge`.

- **Pump thread and CEF render-thread safety**: `frame->ExecuteJavaScript` must be called on the CEF render thread, but the pump thread is a plain `std::thread`.  
  → Mitigation: Use `CefPostTask(TID_RENDERER, ...)` from the pump thread to marshal calls onto the render thread, matching the pattern already used elsewhere in `RenderApp`.

- **`crony_client_t` is not thread-safe for concurrent send+recv**: The C API wraps a Rust object internally protected by a mutex, but calling `send` and `recv` from different threads simultaneously requires care.  
  → Mitigation: Pump thread owns `recv`; JS `send` calls are marshalled from the render thread. These are serialized by the underlying Rust mutex.

## Open Questions

- Should `window.__runtimeBridge` be accessible from all built-in frames, or only specific origins? (Security hardening for future; current answer: main frame + same-origin sub-frames only.)
- Protocol versioning: should the renderer `Hello` message include a client-type identifier (`"renderer"`) so the runtime can distinguish browser-process vs renderer sessions in logs?
