## Why

Built-in pages currently receive runtime events only through `window.__aiDesktopDispatch`, a string-eval path that routes every message through the browser process (`BridgeHandler → ExecuteJavaScript`). This adds a hop, prevents built-in pages from sending control messages directly to the Rust runtime, and couples event delivery to the browser process's threading model. A direct renderer↔runtime IPC channel eliminates that intermediary for built-in pages, enabling cleaner bidirectional communication and reducing latency.

## What Changes

- The `cronymax-runtime` Rust binary binds a **second GIPS service** (`ai.cronymax.runtime.renderer`) dedicated to renderer clients, alongside the existing `ai.cronymax.runtime` browser-process service.
- A second `attach_transport` call on the shared `RuntimeAuthority` connects the renderer service to the same runtime session bus.
- The CEF helper binary (`cronymax_app_helper`) is linked against `Cronymax::Crony` (libcrony.a + crony.h), enabling it to call `crony_client_new` / `crony_client_send` / `crony_client_recv`.
- `RenderApp::OnContextCreated` injects a `window.__runtimeBridge` JS object exposing `send(method, params)` and `subscribe(topic, callback)` bindings backed by a `crony_client_t` handle.
- A pump thread in the renderer process forwards incoming `RuntimeToClient` frames to the JS layer via `frame->ExecuteJavaScript`.
- On space switch (`space.switch_loading: false`), the renderer reconnects: old `crony_client_t` is closed, a new one is opened against the freshly started runtime process.

## Capabilities

### New Capabilities

- `renderer-runtime-bridge`: Direct IPC channel between built-in renderer pages and the Rust runtime — JS `send` (renderer→runtime control) and `subscribe` (runtime→renderer events), bypassing the browser process for both directions.

### Modified Capabilities

## Impact

- **`crony/src/bin/cronymax_runtime.rs`**: add second `GipsTransport::bind` + `attach_transport` call
- **`crony/src/boundary.rs`**: no changes needed (existing single-slot model is correct per-transport)
- **`cmake/CronymaxApp.cmake`** / **`CMakeLists.txt`**: link `Cronymax::Crony` into `cronymax_app_helper`
- **`app/browser/render_app.cc` / `render_app.h`**: inject `window.__runtimeBridge` in `OnContextCreated`; manage `crony_client_t` lifecycle and pump thread
- **`app/browser/render_app.h`**: add `renderer_client_`, pump thread, reconnect flag
- **`web/src/`** (built-in pages): migrate from `window.__aiDesktopDispatch` (for runtime events) to `window.__runtimeBridge.subscribe`; use `window.__runtimeBridge.send` for control messages
- **Dependencies**: `cronymax_app_helper` now requires `libcrony.a` + CoreFoundation/Security/SystemConfiguration frameworks (already used by the browser process)
