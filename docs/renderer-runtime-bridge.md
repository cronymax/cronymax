# Renderer–Runtime Direct IPC (JSB)

Design notes for the `jsb` change — a direct IPC channel between built-in renderer pages and the Rust runtime, bypassing the browser process for both directions.

See `openspec/changes/jsb/` for the full proposal, design decisions, spec, and task list.

---

## Current Event Delivery Path (before JSB)

Every runtime event takes a five-hop relay through the browser process:

```
Rust runtime
  → GipsTransport (ai.cronymax.runtime)
    → RuntimeProxy (browser process, C++)
      → BridgeHandler::SendEvent
        → frame->ExecuteJavaScript("window.__aiDesktopDispatch(...)")
          → Mojo IPC
            → Renderer V8 context
```

Renderer→runtime control likewise goes through `CefMessageRouter` / `BridgeHandler::OnQuery`.

---

## Target Architecture (Shape 3 — true direct IPC)

After JSB, built-in pages have their own direct channel:

```
Rust runtime
  ├── GipsTransport A  (ai.cronymax.runtime)          ← browser process only
  │     └── RuntimeAuthority (Arc, shared)
  └── GipsTransport B  (ai.cronymax.runtime.renderer) ← renderer process only
        └── RuntimeAuthority (Arc, shared)

Browser process (C++)                Renderer process (C++)
  RuntimeBridge ──────────────────     RenderApp
  crony_client_t → service A            crony_client_t → service B
                                         pump thread
                                         window.__runtimeBridge
                                           .send()      → ClientToRuntime
                                           .subscribe() ← RuntimeToClient
```

Both transports share the same `RuntimeAuthority` — they are separate `attach_transport` sessions on the same bus, so subscriptions and state are consistent across both clients.

---

## Why Two GIPS Service Names

GIPS supports multiple clients on one service name at the Mach level (each `Endpoint::connect()` creates a new port pair; the listener yields a separate `Pod` per client). However, `GipsTransport` in `crony/src/boundary.rs` maintains a **single `ReturnPath` slot**:

```
struct ReturnPath {
    connection: Option<Connection>,   // ← single slot, replaced on every accept
    pending: VecDeque<RuntimeToClient>,
    closed: bool,
}
```

`cache_connection()` unconditionally replaces the slot: `rp.connection = Some(connection)`. With two clients on the same service, whichever sent most recently owns the slot — event delivery becomes non-deterministic.

Two service names + two `attach_transport` calls give each client its own `GipsTransport` and `ResponseSink` in the dispatch loop → deterministic event delivery with no races.

---

## Runtime Restart Lifecycle

The runtime process is **killed and restarted on every space switch**, not just on first boot:

```
SpaceManager::SwitchTo(space_id)
  └── runtime_restart_callback_(workspace_root, profile)   [unconditional]
        └── (background thread)
              BroadcastToAllPanels("space.switch_loading", true)
              RuntimeBridge::Stop()          ← SIGTERM + wait; Mach service dies
              RuntimeBridge::Start()         ← SpawnAndHandshake; new Mach service
              BroadcastToAllPanels("space.switch_loading", false)
```

**Impact on the renderer client:** The renderer's `crony_client_t` handle becomes invalid when `Stop()` kills the runtime process. The pump thread detects `CRONY_ERR_CLOSED` and exits. The renderer must reconnect.

---

## Reconnect Mechanism

`space.switch_loading` is broadcast from the browser process via `window.__aiDesktopDispatch` — this is a local JS eval, independent of the runtime, so it arrives even while the runtime is down.

```
space.switch_loading: true   (runtime going down)
  → UI disables runtime-dependent actions
  → pump thread detects recv error → sets renderer_client_ = null → exits

space.switch_loading: false  (new runtime is ready)
  → window.__runtimeBridge.reconnect()
      DisconnectRuntimeClient()      ← close stale client, join old pump thread
      crony_client_new("ai.cronymax.runtime.renderer")
      Hello → Welcome handshake
      StartPumpThread()
      re-subscribe active topics
  → UI re-enables runtime-dependent actions
```

`space.switch_loading: false` is the reliable "runtime is ready" signal because `RuntimeBridge::Start()` completes only after the browser-process handshake with the new runtime succeeds.

---

## JS API Surface

```ts
interface RuntimeBridge {
  // Renderer → runtime: dispatch a control message, returns Promise<response>
  send(method: string, params: unknown): Promise<unknown>;

  // Runtime → renderer: register event callback, returns unsubscribe fn
  subscribe(topic: string, callback: (payload: unknown) => void): () => void;

  // Called by space.switch_loading handler to reconnect after space switch
  reconnect(): void;
}

declare const __runtimeBridge: RuntimeBridge; // injected by RenderApp::OnContextCreated
```

`send()` rejects immediately with `"bridge not ready"` if `renderer_client_` is null (mid-reconnect). `subscribe()` callbacks are stored in a `Map<topic, Set<cb>>` maintained in the renderer C++ layer and dispatched from the pump thread via `CefPostTask(TID_RENDERER, ...)`.

---

## Multi-Frame Constraint

`window.__runtimeBridge` is injected only in the **main frame** of built-in pages (`frame->IsMain()` + built-in URL scheme check in `OnContextCreated`). Sub-frames share the parent's `window` object. This prevents multiple simultaneous `crony_client_t` connections from different frames contesting `GipsTransport B`'s single `ReturnPath` slot.

---

## Implementation Entry Points

| Change                 | File                                                    |
| ---------------------- | ------------------------------------------------------- |
| Second service bind    | `crony/src/bin/cronymax_runtime.rs`                     |
| Helper binary linkage  | `cmake/CronymaxApp.cmake`                               |
| C++ bridge infra       | `app/browser/render_app.cc` / `render_app.h`            |
| V8 injection           | `RenderApp::OnContextCreated`                           |
| Space-switch reconnect | built-in page JS + `window.__runtimeBridge.reconnect()` |
