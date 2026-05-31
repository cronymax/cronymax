// SPDX-License-Identifier: Apache-2.0
//
// Cronymax extension-host bootstrap — Node 26 entry point.
//
// Spawned by the Rust host (see `crates/cronymax/src/extensions/host/`)
// with these argv positions:
//
//   node --permission --no-warnings <allow-flags...> <this script>
//
// stdio layout (set by the parent):
//
//   fd 0 — stdin   (unused; closed by parent)
//   fd 1 — stdout  (pipe → output.log; extension's console.log lands here)
//   fd 2 — stderr  (pipe → host.log;   Node warnings + console.error)
//   fd 3 — pipe    (the MessagePack-RPC channel — see spec §6.1.1)
//
// The extension's `cronymax-extension.json` path is passed via the
// `CRONYMAX_EXTENSION_MANIFEST` env var; the workspace root via
// `CRONYMAX_WORKSPACE_DIR`; per-extension storage via
// `CRONYMAX_EXTENSION_STORAGE` / `CRONYMAX_EXTENSION_GLOBAL_STORAGE`.
//
// This file is intentionally dependency-light. The only npm import is
// `@msgpack/msgpack` (resolved against Node 26's bundled `node_modules`
// that ships next to the binary; see `P2-T01`).

const net = require("node:net");
const fs = require("node:fs");
const path = require("node:path");
const { Encoder, Decoder } = require("@msgpack/msgpack");

// ── 0. Read env -----------------------------------------------------------

const MANIFEST_PATH = process.env.CRONYMAX_EXTENSION_MANIFEST;
const EXT_DIR = process.env.CRONYMAX_EXTENSION_DIR;
const EXT_STORAGE = process.env.CRONYMAX_EXTENSION_STORAGE;
const EXT_GLOBAL_STORAGE = process.env.CRONYMAX_EXTENSION_GLOBAL_STORAGE;

// Workspace folders come in as a JSON array of canonical paths.
// Empty array = no workspace open. v1 alpha grants rw on every folder
// automatically — manifests don't need to declare {WORKSPACE}.
let WORKSPACE_FOLDERS = [];
try {
  WORKSPACE_FOLDERS = JSON.parse(process.env.CRONYMAX_WORKSPACE_FOLDERS || "[]");
  if (!Array.isArray(WORKSPACE_FOLDERS)) {
    WORKSPACE_FOLDERS = [];
  }
} catch {
  WORKSPACE_FOLDERS = [];
}

if (!MANIFEST_PATH || !EXT_DIR || !EXT_STORAGE || !EXT_GLOBAL_STORAGE) {
  process.stderr.write("[bootstrap] missing one of CRONYMAX_EXTENSION_{MANIFEST,DIR,STORAGE,GLOBAL_STORAGE}\n");
  process.exit(2);
}

const manifest = JSON.parse(fs.readFileSync(MANIFEST_PATH, "utf8"));
const EXT_ID = manifest.id;

// ── 1. RPC channel over fd 3 ---------------------------------------------

const rpcSocket = new net.Socket({ fd: 3 });
const encoder = new Encoder();
const _decoder = new Decoder();

function writeFrame(frame) {
  rpcSocket.write(encoder.encode(frame));
}

// Pending outbound requests: msgid → { resolve, reject }
const pending = new Map();
let nextMsgid = 1;

function rpcRequest(method, params) {
  const msgid = nextMsgid++;
  return new Promise((resolve, reject) => {
    pending.set(msgid, { resolve, reject });
    writeFrame([0, msgid, method, params]);
  });
}

function rpcNotify(method, params) {
  writeFrame([2, method, params]);
}

// In-flight inbound requests by msgid, so `$/cancel` can flip a flag
const inFlight = new Map();

const handlers = new Map();

function registerRpcHandler(method, fn) {
  handlers.set(method, fn);
}

function dispatchFrame(frame) {
  if (!Array.isArray(frame) || frame.length < 3) return;
  const type = frame[0];
  if (type === 0) {
    const [, msgid, method, params] = frame;
    const handler = handlers.get(method);
    if (!handler) {
      writeFrame([1, msgid, `method '${method}' not implemented`, null]);
      return;
    }
    const cancel = { cancelled: false };
    inFlight.set(msgid, cancel);
    Promise.resolve()
      .then(() => handler(params, cancel))
      .then(
        (result) => {
          inFlight.delete(msgid);
          writeFrame([1, msgid, null, result == null ? null : result]);
        },
        (err) => {
          inFlight.delete(msgid);
          const msg = err?.message ? String(err.message) : String(err);
          writeFrame([1, msgid, msg, null]);
        },
      );
  } else if (type === 1) {
    const [, msgid, err, result] = frame;
    const slot = pending.get(msgid);
    if (slot) {
      pending.delete(msgid);
      if (err) slot.reject(new Error(String(err)));
      else slot.resolve(result);
    }
  } else if (type === 2) {
    const [, method, params] = frame;
    if (method === "$/cancel" && Array.isArray(params) && params.length >= 1) {
      const target = params[0];
      const cancel = inFlight.get(target);
      if (cancel) cancel.cancelled = true;
      return;
    }
    // Other inbound notifies (e.g. extension/registerError) — dispatch
    // to the registered handler if one exists. Fire-and-forget: any
    // result or error is logged but never sent back, since notifies
    // have no response frame.
    const handler = handlers.get(method);
    if (handler) {
      Promise.resolve()
        .then(() => handler(params))
        .catch((err) => {
          const msg = err?.message ? String(err.message) : String(err);
          console.error(`[cronymax-bootstrap] notify handler '${method}' threw: ${msg}`);
        });
    }
  }
}

// Decode the bidirectional fd 3 stream. `Decoder.decodeMulti` doesn't
// stream across read boundaries on its own; accumulate chunks then decode
// as many frames as possible.
let inboundBuf = Buffer.alloc(0);
rpcSocket.on("data", (chunk) => {
  inboundBuf = inboundBuf.length === 0 ? chunk : Buffer.concat([inboundBuf, chunk]);
  // `Decoder.decodeMulti` returns a generator; iterating consumes each
  // frame in sequence and `decoder.bytePosition` (or `pos` on older
  // versions) tracks how much of the input has been consumed *up to and
  // including* the most recently yielded frame. Collect frames while
  // they decode cleanly; once the iterator throws (RangeError = need
  // more bytes), trim what we consumed and wait for the next chunk.
  const frames = [];
  let consumed = 0;
  try {
    const decoder = new Decoder();
    for (const frame of decoder.decodeMulti(inboundBuf)) {
      frames.push(frame);
      // `pos` is the canonical "next-byte" cursor on @msgpack/msgpack's
      // Decoder; it's updated after each successful decode.
      consumed = decoder.pos;
    }
  } catch (e) {
    // RangeError / "not enough data" means the next frame is partial;
    // anything we already pushed into `frames` is valid up to `consumed`.
    const s = String(e);
    if (!s.includes("not enough data") && !s.includes("RangeError")) {
      process.stderr.write(`[bootstrap] decode error: ${e}\n`);
    }
  }
  if (consumed > 0) {
    inboundBuf = inboundBuf.subarray(consumed);
  }
  for (const frame of frames) {
    try {
      dispatchFrame(frame);
    } catch (e) {
      process.stderr.write(`[bootstrap] dispatch error: ${e}\n`);
    }
  }
});

rpcSocket.on("error", (e) => {
  process.stderr.write(`[bootstrap] rpc socket error: ${e}\n`);
});

// ── 2. console.* intercept (EH Layer B) ----------------------------------
//
// Replace the global `console` so each call lands in `output.log` *and*
// (optionally) audit-routes via RPC. We don't hook `require` — spec §6.1
// forbids module-graph mutation; this is a plain global replacement.

const _originalConsole = {
  log: console.log.bind(console),
  info: console.info.bind(console),
  warn: console.warn.bind(console),
  error: console.error.bind(console),
  debug: console.debug.bind(console),
};

function fmtArgs(args) {
  return args
    .map((a) => {
      if (typeof a === "string") return a;
      try {
        return JSON.stringify(a);
      } catch {
        return String(a);
      }
    })
    .join(" ");
}

// v1 alpha: console output is captured by the host's stdout/stderr pipe
// into `output.log` / `host.log` — no extra RPC notify needed. The
// previous `log/console` and `log/error` audit channels were dropped
// with the permission-model removal.
for (const level of ["log", "info", "warn", "error", "debug"]) {
  console[level] = (...args) => {
    const line = fmtArgs(args);
    if (level === "warn" || level === "error") {
      process.stderr.write(`${line}\n`);
    } else {
      process.stdout.write(`${line}\n`);
    }
  };
}

// ── 3. Process-level error handlers --------------------------------------

let activated = false;

process.on("uncaughtException", (err) => {
  const phase = activated ? "running" : "activate";
  const stack = err?.stack ? String(err.stack) : String(err);
  process.stderr.write(`[bootstrap] uncaughtException (${phase}): ${stack}\n`);
});

process.on("unhandledRejection", (reason) => {
  const msg = reason?.message ? String(reason.message) : String(reason);
  process.stderr.write(`[bootstrap] unhandledRejection: ${msg}\n`);
});

// ── 4. SDK injection ------------------------------------------------------
//
// In production, the `@cronymax/extension` package's runtime entry resolves
// `require("@cronymax/extension")` to a copy that talks to fd 3. In the
// alpha, we inject a *minimal* in-process implementation via the
// `cronymax` global so a hello-world extension can be written without
// publishing the SDK first. The SDK proper (P2-T07) supersedes this.

const subscriptions = []; // Disposable list — ctx.subscriptions
const channels = new Map(); // name → OutputChannel
// Live AgentSession instances keyed by their `id` field. Populated when
// `agents/session.create:<providerId>` resolves; consumed by the
// per-session handlers (prompt / dispose / cancel / resolvePermission)
// registered below. Spec §4 — the wire format routes by sessionId; the
// per-provider scope only matters for session creation.
const agentSessions = new Map();

// Per-topic local handler tables for `cronymax.events.on(topic, handler)`.
// Each topic maps to a list of handler closures; `events/publish` notifies
// from the platform get fan-out here. Per-extension capability gating
// happens platform-side — by the time `events/publish` arrives the topic
// was already validated.
const eventHandlers = new Map(); // topic → Set<handler>
registerRpcHandler("events/publish", (params) => {
  // Wire: { topic, publisher, data }
  const topic = params?.topic;
  if (typeof topic !== "string") return;
  const handlers = eventHandlers.get(topic);
  if (!handlers || handlers.size === 0) return;
  for (const h of handlers) {
    Promise.resolve()
      .then(() => h(params.data, { publisher: params.publisher, topic }))
      .catch((err) => {
        const msg = err?.message ? String(err.message) : String(err);
        process.stderr.write(`[bootstrap] events handler for '${topic}' threw: ${msg}\n`);
      });
  }
});

function createOutputChannel(name, options) {
  const isLog = !!options?.log;
  // Server-side rpc to allocate the channel file lives in P2-T12 / the
  // SDK; here we just notify on each line so the platform writes the
  // log channel file.
  const send = (level, line) => {
    rpcNotify("log/channel", { channel: name, level, message: line });
  };
  const channel = {
    name,
    append(text) {
      send("info", String(text));
    },
    appendLine(text) {
      send("info", String(text));
    },
    show(_preserveFocus) {
      rpcNotify("window/showOutputChannel", { name });
    },
    clear() {
      return rpcRequest("log/channelClear", { channel: name });
    },
    dispose() {
      channels.delete(name);
    },
  };
  if (isLog) {
    for (const level of ["trace", "debug", "info", "warn", "error"]) {
      channel[level] = (msg, ...args) => {
        const line = args.length
          ? `${msg} ${fmtArgs(args)}`
          : typeof msg === "string"
            ? msg
            : msg?.stack
              ? msg.stack
              : String(msg);
        send(level, line);
      };
    }
  }
  channels.set(name, channel);
  return channel;
}

// ── Webview panel state ─────────────────────────────────────────────────
//
// `cronymax.window.createWebviewPanel(opts)` returns a panel object that
// holds the user's `onDidReceiveMessage` listeners locally. Inbound
// `webview/onDidReceiveMessage` / `webview/onDidChangeViewState` /
// `webview/onDidDispose` notifies arrive routed by `panelId`; the global
// handlers below look up the matching panel object and fan out.
const webviewPanels = new Map(); // panelId → live WebviewPanel impl

registerRpcHandler("webview/onDidReceiveMessage", (params) => {
  const panel = webviewPanels.get(params?.panelId);
  if (!panel) return;
  panel._fireMessage(params.payload);
});

registerRpcHandler("webview/onDidChangeViewState", (params) => {
  const panel = webviewPanels.get(params?.panelId);
  if (!panel) return;
  panel._setViewState({
    active: !!params.active,
    visible: !!params.visible,
  });
});

registerRpcHandler("webview/onDidDispose", (params) => {
  const panel = webviewPanels.get(params?.panelId);
  if (!panel) return;
  // The platform already removed the panel registry entry; just notify
  // the user listeners and clean up our local map.
  panel._fireDispose();
  webviewPanels.delete(params.panelId);
});

function buildWebviewPanel({ id, slot, url, title, ownerExtId }) {
  const msgListeners = new Set();
  const viewStateListeners = new Set();
  const disposeListeners = new Set();
  let active = false;
  let visible = false;
  let disposed = false;
  const fireSafe = (set, arg) => {
    for (const cb of set) {
      Promise.resolve()
        .then(() => cb(arg))
        .catch((err) => {
          const msg = err?.message ? String(err.message) : String(err);
          process.stderr.write(`[bootstrap] webview listener threw: ${msg}\n`);
        });
    }
  };
  const subscribable = (set) => (listener) => {
    if (typeof listener !== "function") {
      throw new TypeError("event listener must be a function");
    }
    set.add(listener);
    return { dispose: () => set.delete(listener) };
  };
  const panel = {
    get id() { return id; },
    get slot() { return slot; },
    get url() { return url; },
    get title() { return title; },
    get active() { return active; },
    get visible() { return visible; },
    onDidReceiveMessage: subscribable(msgListeners),
    onDidChangeViewState: subscribable(viewStateListeners),
    onDidDispose: subscribable(disposeListeners),
    async postMessage(payload) {
      if (disposed) {
        throw new Error(`webview panel '${id}' is disposed`);
      }
      rpcNotify("webview/postMessage", { panelId: id, payload });
    },
    async setHtml(_html) {
      // setHtml is reserved (IDL v1 includes it) but not yet plumbed
      // through to the renderer; until the iframe loader supports
      // inline HTML, throw rather than silently no-op.
      throw new Error("webview setHtml is not implemented in v1 alpha");
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      webviewPanels.delete(id);
      rpcNotify("webview/disposePanel", { panelId: id });
      fireSafe(disposeListeners, undefined);
    },
    _fireMessage(payload) {
      fireSafe(msgListeners, payload);
    },
    _setViewState(s) {
      active = s.active;
      visible = s.visible;
      fireSafe(viewStateListeners, { active, visible });
    },
    _fireDispose() {
      if (disposed) return;
      disposed = true;
      fireSafe(disposeListeners, undefined);
    },
  };
  panel._ownerExtId = ownerExtId;
  return panel;
}

// ── Webview view (operation-view) provider state ────────────────────────
//
// `cronymax.window.registerWebviewViewProvider(viewId, provider)` is the
// counterpart to `createWebviewPanel` for PLATFORM-OPENED views: the rail
// view contributed via `cronymax.ui.sidebar.view` whose iframe the
// platform mounts when the user clicks its icon. The extension registers a
// provider; when the view is shown the platform sends `webviewView/resolve`
// and we build a `WebviewView` handle and run `provider.resolveWebviewView`.
//
// Inbound (platform → ext): `webviewView/resolve` /
// `onDidReceiveMessage` / `onDidChangeVisibility` / `onDidDispose`.
// Outbound (ext → platform): `webviewView/postMessage` (from
// `view.webview.postMessage`) and `sidebar/register` (on provider
// registration, so the platform's sidebar-view registry knows the owner).
const viewProviders = new Map(); // viewId → WebviewViewProvider
const liveViews = new Map(); // viewId → live WebviewView impl

function fireSafeListeners(set, arg, label) {
  for (const cb of set) {
    Promise.resolve()
      .then(() => cb(arg))
      .catch((err) => {
        const msg = err?.message ? String(err.message) : String(err);
        process.stderr.write(`[bootstrap] ${label} listener threw: ${msg}\n`);
      });
  }
}

function makeSubscribable(set) {
  return (listener) => {
    if (typeof listener !== "function") {
      throw new TypeError("event listener must be a function");
    }
    set.add(listener);
    return { dispose: () => set.delete(listener) };
  };
}

function buildWebviewView({ viewId, visible }) {
  const msgListeners = new Set();
  const disposeListeners = new Set();
  const visibilityListeners = new Set();
  let isVisible = !!visible;
  let disposed = false;
  const webview = {
    onDidReceiveMessage: makeSubscribable(msgListeners),
    async postMessage(payload) {
      if (disposed) {
        throw new Error(`webview view '${viewId}' is disposed`);
      }
      rpcNotify("webviewView/postMessage", { viewId, payload });
    },
  };
  const view = {
    get viewId() { return viewId; },
    get visible() { return isVisible; },
    webview,
    onDidDispose: makeSubscribable(disposeListeners),
    onDidChangeVisibility: makeSubscribable(visibilityListeners),
    _fireMessage(payload) {
      fireSafeListeners(msgListeners, payload, "webview view");
    },
    _setVisible(next) {
      const nv = !!next;
      if (nv === isVisible) return;
      isVisible = nv;
      fireSafeListeners(visibilityListeners, undefined, "webview view visibility");
    },
    _fireDispose() {
      if (disposed) return;
      disposed = true;
      fireSafeListeners(disposeListeners, undefined, "webview view dispose");
    },
  };
  return view;
}

// Build (or rebuild) the live view for `viewId` and run the provider's
// resolveWebviewView. A fresh handle per resolve mirrors VS Code, where the
// webview is recreated each time the view is shown — our iframe reloads on
// every open, so its listeners reset and the old handle is no longer wired.
function resolveWebviewView(viewId, visible) {
  const provider = viewProviders.get(viewId);
  if (!provider) return false;
  const prev = liveViews.get(viewId);
  if (prev) prev._fireDispose();
  const view = buildWebviewView({ viewId, visible });
  liveViews.set(viewId, view);
  Promise.resolve()
    .then(() => provider.resolveWebviewView(view))
    .catch((err) => {
      const msg = err?.message ? String(err.message) : String(err);
      process.stderr.write(`[bootstrap] resolveWebviewView('${viewId}') threw: ${msg}\n`);
    });
  return true;
}

registerRpcHandler("webviewView/resolve", (params) => {
  const viewId = params?.viewId;
  if (typeof viewId !== "string") return;
  resolveWebviewView(viewId, params?.visible !== false);
});

registerRpcHandler("webviewView/onDidReceiveMessage", (params) => {
  const viewId = params?.viewId;
  if (typeof viewId !== "string") return;
  let view = liveViews.get(viewId);
  if (!view) {
    // The view iframe can post before the explicit resolve notify lands
    // (the rail's resolve request and the iframe's onload race). Resolve
    // implicitly so the provider gets its handle, then deliver.
    if (!resolveWebviewView(viewId, true)) return;
    view = liveViews.get(viewId);
  }
  view?._fireMessage(params.payload);
});

registerRpcHandler("webviewView/onDidChangeVisibility", (params) => {
  const view = liveViews.get(params?.viewId);
  if (!view) return;
  view._setVisible(!!params.visible);
});

registerRpcHandler("webviewView/onDidDispose", (params) => {
  const view = liveViews.get(params?.viewId);
  if (!view) return;
  view._fireDispose();
  liveViews.delete(params.viewId);
});

const cronymax = {
  ExtensionMode: { Production: 1, Development: 2, Test: 3 },
  window: {
    createOutputChannel,
    showInformationMessage(message) {
      return rpcRequest("window/showInformationMessage", { message });
    },
    showWarningMessage(message) {
      return rpcRequest("window/showWarningMessage", { message });
    },
    showErrorMessage(message) {
      return rpcRequest("window/showErrorMessage", { message });
    },
    // IDL: createWebviewPanel(opts): Promise<WebviewPanel>. Returns a
    // panel that fires `onDidReceiveMessage` for cefQuery-routed posts
    // from inside the iframe and `postMessage` to push payloads back.
    async createWebviewPanel(opts) {
      if (!opts || typeof opts.id !== "string" || typeof opts.entry !== "string") {
        throw new TypeError(
          "createWebviewPanel: { id, slot, title, entry } required",
        );
      }
      const resp = await rpcRequest("webview/createPanel", {
        panelId: opts.id,
        title: opts.title ?? "",
        slot: opts.slot ?? "sidebar",
        entry: opts.entry,
        retainContextWhenHidden: opts.retainContextWhenHidden ?? false,
      });
      // The platform echoes back the resolved url + slot so the SDK
      // can expose a final shape that matches what the renderer
      // actually mounted (the slot string is normalised against the
      // accepted IDL set).
      const panel = buildWebviewPanel({
        id: resp?.panelId ?? opts.id,
        slot: resp?.slot ?? opts.slot ?? "sidebar",
        url: resp?.url ?? `cronymax-webview://${EXT_ID}/${opts.entry.replace(/^\.\/+/, "")}`,
        title: opts.title ?? "",
        ownerExtId: EXT_ID,
      });
      webviewPanels.set(panel.id, panel);
      subscriptions.push({ dispose: () => panel.dispose() });
      return panel;
    },
    // IDL: registerWebviewViewProvider(viewId, provider): Disposable.
    // Backs the operation views contributed via `cronymax.ui.sidebar.view`.
    // Registration sends `sidebar/register` so the platform's sidebar-view
    // registry learns the owner (needed to route view messages); the
    // provider is invoked when the platform sends `webviewView/resolve`.
    registerWebviewViewProvider(viewId, provider) {
      if (typeof viewId !== "string" || !provider || typeof provider.resolveWebviewView !== "function") {
        throw new TypeError(
          "registerWebviewViewProvider(viewId, provider): provider must implement resolveWebviewView",
        );
      }
      viewProviders.set(viewId, provider);
      rpcNotify("sidebar/register", { viewId });
      const dispose = () => {
        viewProviders.delete(viewId);
        const live = liveViews.get(viewId);
        if (live) {
          live._fireDispose();
          liveViews.delete(viewId);
        }
        rpcNotify("sidebar/unregister", { viewId });
      };
      const sub = { dispose };
      subscriptions.push(sub);
      return sub;
    },
  },
  commands: {
    register(commandId, callback) {
      registerRpcHandler(`commands/execute:${commandId}`, async (params) => {
        return await callback(...(Array.isArray(params) ? params : []));
      });
      const dispose = () => rpcNotify("commands/unregister", { commandId });
      rpcNotify("commands/register", { commandId });
      const sub = { dispose };
      subscriptions.push(sub);
      return sub;
    },
    execute(commandId, ...args) {
      return rpcRequest("commands/executeOther", {
        commandId,
        args,
      });
    },
  },
  workspace: {
    workspaceFolders: WORKSPACE_FOLDERS.map((p) => ({ uri: `file://${p}` })),
    rootUri: WORKSPACE_FOLDERS.length > 0 ? `file://${WORKSPACE_FOLDERS[0]}` : undefined,
    getConfiguration(section) {
      return {
        get(key, fallback) {
          // Sync API in v1 isn't truly sync; this returns a Promise for
          // alpha. SDK in P2-T07 polishes to match VS Code semantics.
          return rpcRequest("workspace/config.get", { section, key, fallback });
        },
        async update(key, value) {
          return rpcRequest("workspace/config.update", { section, key, value });
        },
      };
    },
  },
  extensions: {
    getExtension(id) {
      return rpcRequest("extensions/getExtension", { id });
    },
  },
  events: {
    // Subscribe a handler to one topic. Local dispatch only — the
    // `events/subscribe` notify tells the platform to start forwarding
    // matching fires over `events/publish`. The first subscribe for a
    // topic kicks off the platform subscription; later subscribes from
    // the same extension stack into the local table (and the platform
    // re-registers idempotently, but only one wire subscribe per topic
    // is necessary).
    on(topic, handler) {
      if (typeof topic !== "string" || typeof handler !== "function") {
        throw new TypeError("cronymax.events.on: (topic, handler) required");
      }
      let bucket = eventHandlers.get(topic);
      const firstForTopic = !bucket || bucket.size === 0;
      if (!bucket) {
        bucket = new Set();
        eventHandlers.set(topic, bucket);
      }
      bucket.add(handler);
      if (firstForTopic) {
        rpcNotify("events/subscribe", { topic });
      }
      const dispose = () => {
        const b = eventHandlers.get(topic);
        if (!b) return;
        b.delete(handler);
        if (b.size === 0) {
          eventHandlers.delete(topic);
          rpcNotify("events/unsubscribe", { topic });
        }
      };
      const sub = { dispose };
      subscriptions.push(sub);
      return sub;
    },
    // Emit one event under the extension's publisher namespace. Modeled
    // as a request (not notify) so the returned Promise rejects with
    // capability errors — silent drops on emit would hide misconfigured
    // `capabilities.events.emit` manifests.
    emit(topic, data) {
      if (typeof topic !== "string") {
        return Promise.reject(new TypeError("cronymax.events.emit: topic must be a string"));
      }
      return rpcRequest("events/emit", { topic, data }).then(() => undefined);
    },
  },
  agents: {
    // Extensions call this from activate() to make a contributed
    // provider available to the chat panel / flow runtime. The platform
    // looks up the provider's metadata (label / icon / supports*) from
    // the manifest's contributes["cronymax.agents.provider"] entry; the
    // notify here just announces "the JS impl is live".
    registerProvider(providerId, impl) {
      registerRpcHandler(`agents/session.create:${providerId}`, async (params) => {
        const session = await impl.createSession(params);
        if (!session || typeof session.id !== "string" || !session.id) {
          throw new Error(
            `provider '${providerId}' createSession() must return AgentSession with a non-empty string id`,
          );
        }
        // Stash so the global session.prompt / dispose / cancel /
        // resolvePermission handlers (registered below) can find it.
        agentSessions.set(session.id, session);
        return { sessionId: session.id };
      });
      registerRpcHandler(`agents/enumerate:${providerId}`, async () => {
        if (typeof impl.enumerate !== "function") {
          return [];
        }
        const items = await impl.enumerate();
        return items ?? [];
      });
      const dispose = () => rpcNotify("agents/unregisterProvider", { providerId });
      rpcNotify("agents/registerProvider", { providerId });
      const sub = { dispose };
      subscriptions.push(sub);
      return sub;
    },
  },
  // NB: there is no `cronymax.renderers` namespace in v1. ContentRenderer
  // is iframe-hosted — handler code runs in a `cronymax-webview://<ext>/
  // <entry>?surface=renderer&id=<inst>` iframe and uses the
  // `acquireCronymaxRendererApi()` global (see cep-idl/v1/renderer-host.ts).
  // Renderer-only extensions may omit `manifest.main` entirely.
  sidebar: {
    // Extensions call this from activate() to make a sidebar view
    // available. Title / icon / entry come from the manifest.
    register(viewId, handler) {
      if (handler) {
        registerRpcHandler(`sidebar/view.message:${viewId}`, async (params) => handler(params));
      }
      const dispose = () => rpcNotify("sidebar/unregister", { viewId });
      rpcNotify("sidebar/register", { viewId });
      const sub = { dispose };
      subscriptions.push(sub);
      return sub;
    },
  },
};

// Make it reachable via `require("@cronymax/extension")` shim — the SDK
// (P2-T07) will replace this with a real npm package, but in v1 alpha
// it's just a property on the global object so user code can do:
//     const cronymax = require("@cronymax/extension");
// without anything fancy in the resolver.
globalThis.cronymax = cronymax;

// ── 5. activate / deactivate handlers ------------------------------------

registerRpcHandler("$/ping", async () => "pong");

// ── AgentSession plumbing (cep-idl/v1/agents.ts §AgentSession) -----------
//
// session.create is per-provider (registered by registerProvider above);
// the rest are global because they look up by sessionId. Iteration of
// session.prompt() events streams back as `agents/event` notifies; turn
// completion is signalled via `agents/turn.done`. The session.prompt
// request itself resolves only after the iterator finishes (or errors /
// cancels), so the platform can serialize turns on a single sessionId.

function buildCancellationToken(cancelFlag) {
  const cbs = [];
  let fired = false;
  const fireOnce = () => {
    if (fired) return;
    fired = true;
    for (const cb of cbs) {
      try {
        cb();
      } catch (e) {
        process.stderr.write(`[bootstrap] cancellation cb threw: ${e}\n`);
      }
    }
  };
  // Poll the in-flight cancel flag; flip onCancellationRequested
  // listeners on first observation. A 25ms cadence keeps prompt() iteration
  // responsive to user cancel without burning CPU on a tight loop.
  const poller = setInterval(() => {
    if (cancelFlag.cancelled) {
      clearInterval(poller);
      fireOnce();
    }
  }, 25);
  return {
    token: {
      get isCancellationRequested() {
        return cancelFlag.cancelled;
      },
      onCancellationRequested(cb) {
        if (cancelFlag.cancelled) {
          // Already cancelled — schedule callback on next tick.
          Promise.resolve().then(() => {
            try {
              cb();
            } catch (e) {
              process.stderr.write(`[bootstrap] cancellation cb threw (sync path): ${e}\n`);
            }
          });
          return {
            dispose() {
              /* no-op — already fired */
            },
          };
        }
        cbs.push(cb);
        return {
          dispose() {
            const i = cbs.indexOf(cb);
            if (i >= 0) cbs.splice(i, 1);
          },
        };
      },
    },
    dispose: () => clearInterval(poller),
  };
}

registerRpcHandler("agents/session.prompt", async (params, cancelFlag) => {
  const sessionId = params?.sessionId;
  if (typeof sessionId !== "string" || !sessionId) {
    throw new Error("agents/session.prompt: missing string sessionId");
  }
  const session = agentSessions.get(sessionId);
  if (!session) {
    throw new Error(`agents/session.prompt: unknown sessionId '${sessionId}'`);
  }
  const message = params?.message || { text: "" };
  const { token, dispose: disposeToken } = buildCancellationToken(cancelFlag);
  let sawDone = false;
  try {
    for await (const event of session.prompt(message, token)) {
      rpcNotify("agents/event", { sessionId, event });
      if (event && event.kind === "done") {
        sawDone = true;
        break;
      }
    }
    if (!sawDone) {
      // Iterator ended without emitting a done event — synthesize one so
      // the platform observes a well-formed turn boundary. cancelled wins
      // over end_turn if the cancel flag was raised mid-iteration.
      const stopReason = cancelFlag.cancelled ? "cancelled" : "end_turn";
      const synthetic = { kind: "done", stopReason };
      rpcNotify("agents/event", { sessionId, event: synthetic });
    }
  } catch (err) {
    const msg = err?.message ? String(err.message) : String(err);
    rpcNotify("agents/event", {
      sessionId,
      event: { kind: "done", stopReason: "error", errorMessage: msg },
    });
  } finally {
    disposeToken();
  }
  rpcNotify("agents/turn.done", { sessionId });
  return null;
});

registerRpcHandler("agents/session.resolvePermission", async (params) => {
  const sessionId = params?.sessionId;
  if (typeof sessionId !== "string" || !sessionId) {
    throw new Error("agents/session.resolvePermission: missing string sessionId");
  }
  const session = agentSessions.get(sessionId);
  if (!session) {
    throw new Error(`agents/session.resolvePermission: unknown sessionId '${sessionId}'`);
  }
  if (typeof session.resolvePermission !== "function") {
    throw new Error(`agents/session.resolvePermission: session '${sessionId}' has no resolvePermission()`);
  }
  await session.resolvePermission(params.requestId, params.decision);
  return null;
});

registerRpcHandler("agents/session.cancel", async (params) => {
  const sessionId = params?.sessionId;
  if (typeof sessionId !== "string" || !sessionId) {
    throw new Error("agents/session.cancel: missing string sessionId");
  }
  const session = agentSessions.get(sessionId);
  if (!session) return null;
  if (typeof session.cancel === "function") {
    await session.cancel();
  }
  return null;
});

registerRpcHandler("agents/session.dispose", async (params) => {
  const sessionId = params?.sessionId;
  if (typeof sessionId !== "string" || !sessionId) {
    throw new Error("agents/session.dispose: missing string sessionId");
  }
  const session = agentSessions.get(sessionId);
  if (!session) return null;
  agentSessions.delete(sessionId);
  if (typeof session.dispose === "function") {
    await session.dispose();
  }
  return null;
});

// Platform → extension reverse notify: a register call failed on the
// runtime side (id not declared in manifest / cross-extension collision
// / namespace-reserved / etc.). Surface as console.error so developers
// notice — silent drop hides bugs until users complain.
registerRpcHandler("extension/registerError", async (params) => {
  const ep = params?.ep ? String(params.ep) : "<unknown ep>";
  const id = params?.id ? String(params.id) : "<unknown id>";
  const reason = params?.reason ? String(params.reason) : "<no reason>";
  console.error(`[cronymax] register failed: ep=${ep} id=${id} reason=${reason}`);
});

registerRpcHandler("extension/activate", async () => {
  // The runtime never sends `extension/activate` to a main-less extension
  // (it doesn't even spawn a host for them in v1 — see P6.5 IDL D7), so
  // hitting an empty `manifest.main` here is a wire-protocol bug, not a
  // user-facing case. Throw a clear error instead of letting path.join
  // produce a confusing TypeError.
  if (!manifest.main) {
    throw new Error(
      `extension ${EXT_ID}: extension/activate received but manifest has no \`main\` (declarative-only extensions must not spawn a host)`,
    );
  }
  const mainPath = path.join(EXT_DIR, manifest.main);
  const userModule = require(mainPath);
  const activateFn = userModule.activate;
  if (typeof activateFn !== "function") {
    throw new Error(`extension ${EXT_ID}: no activate() export from ${manifest.main}`);
  }
  const ctx = {
    extensionId: EXT_ID,
    extensionPath: EXT_DIR,
    storagePath: EXT_STORAGE,
    globalStoragePath: EXT_GLOBAL_STORAGE,
    subscriptions,
    extensionMode: cronymax.ExtensionMode.Production,
  };
  const exports = await activateFn(ctx);
  activated = true;
  return exports == null ? null : exports;
});

registerRpcHandler("extension/deactivate", async () => {
  for (const sub of subscriptions.splice(0)) {
    try {
      const r = sub.dispose?.();
      if (r && typeof r.then === "function") await r;
    } catch (e) {
      process.stderr.write(`[bootstrap] dispose threw: ${e}\n`);
    }
  }
  channels.clear();
  activated = false;
  return null;
});

// Signal readiness so the platform can send `extension/activate`.
rpcNotify("$/ready", { ext_id: EXT_ID, pid: process.pid });
