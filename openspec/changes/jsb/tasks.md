## 1. Rust Runtime — Second Service Binding

- [x] 1.1 In `crony/src/bin/cronymax_runtime.rs`, add `GipsTransport::bind("ai.cronymax.runtime.renderer")` after the existing `bind_default()` call
- [x] 1.2 Call `bundle.runtime.attach_transport(renderer_transport)` with the new transport (shares the same `RuntimeAuthority`)
- [ ] 1.3 Verify both GIPS services appear in the Mach bootstrap namespace on startup (manual: `lsregister` / `bootstrap_look_up` probe)

## 2. CMake — Link crony into Helper Binary

- [x] 2.1 In `cmake/CronymaxApp.cmake`, add `Cronymax::Crony` to the `target_link_libraries` for `cronymax_app_helper`
- [x] 2.2 Add CoreFoundation, Security, SystemConfiguration framework links to the helper target (match the existing browser-process linkage)
- [ ] 2.3 Build `cronymax_app_helper` and confirm it compiles without errors; confirm no `BridgeHandler` / `SpaceManager` symbols leak in via `nm`

## 3. RenderApp — C++ Bridge Infrastructure

- [x] 3.1 Add `crony_client_t* renderer_client_` (atomic or mutex-protected), pump thread handle, and a shutdown flag to `RenderApp` in `app/browser/render_app.h`
- [x] 3.2 Implement `ConnectRuntimeClient(CefRefPtr<CefFrame> frame)` in `render_app.cc`: calls `crony_client_new("ai.cronymax.runtime.renderer")`, sends `Hello`, waits for `Welcome`
- [x] 3.3 Implement `StartPumpThread(CefRefPtr<CefFrame> frame)`: spawns a `std::thread` that loops on `crony_client_recv`, posts `RuntimeToClient` frames to the render thread via `CefPostTask(TID_RENDERER, ...)`; exits on recv error or shutdown flag
- [x] 3.4 In `OnContextCreated`, after the existing `CefMessageRouterRendererSide` init, call `ConnectRuntimeClient` + `StartPumpThread` for main-frame built-in pages only (check `frame->IsMain()` and built-in URL scheme)
- [x] 3.5 Implement `DisconnectRuntimeClient()`: sets shutdown flag, calls `crony_client_close`, joins pump thread, nulls `renderer_client_`
- [x] 3.6 Call `DisconnectRuntimeClient()` from `OnContextReleased` for cleanup

## 4. RenderApp — V8 Bindings (`window.__runtimeBridge`)

- [x] 4.1 Implement `send(method, params) → Promise` V8 binding: marshals call to render thread, dispatches `ClientToRuntime` via `renderer_client_`, resolves Promise from `RuntimeToClient` response
- [x] 4.2 Implement `subscribe(topic, callback) → unsubscribe fn` V8 binding: registers callback in a topic→callbacks map stored in the V8 context; invoked by pump thread dispatch
- [x] 4.3 Implement `reconnect()` V8 binding: calls `DisconnectRuntimeClient()` then `ConnectRuntimeClient()` + `StartPumpThread()`
- [x] 4.4 Register all three functions on `window.__runtimeBridge` object in `OnContextCreated`
- [x] 4.5 Ensure `send()` returns a rejected Promise (not throws) when `renderer_client_ == nullptr`

## 5. Reconnect on Space Switch

- [x] 5.1 In the built-in page JS (or a shared bridge utility), subscribe to `space.switch_loading` via `window.__aiDesktopDispatch` listener
- [x] 5.2 When `space.switch_loading: false` is received, call `window.__runtimeBridge.reconnect()`
- [ ] 5.3 Verify reconnect restores `send` and `subscribe` functionality after a space switch (manual test: switch spaces, send a `tools/list` message)

## 6. Testing & Verification

- [ ] 6.1 Write a manual test script: open built-in page, call `window.__runtimeBridge.send("tools/list", {})`, confirm response arrives without browser-process relay (add temporary log in `BridgeHandler::OnQuery` to confirm it's NOT hit)
- [ ] 6.2 Verify `window.__runtimeBridge.subscribe` delivers a runtime event (trigger a workspace event, check JS callback fires)
- [ ] 6.3 Verify no double-delivery: a runtime event dispatched via the renderer bridge should NOT also appear via `window.__aiDesktopDispatch` for the same topic
- [ ] 6.4 Verify space-switch reconnect: switch spaces twice in sequence, confirm bridge is functional after each switch
- [ ] 6.5 Verify `window.__runtimeBridge` is absent in a non-built-in frame (open an external URL in a webview, confirm object is undefined)
