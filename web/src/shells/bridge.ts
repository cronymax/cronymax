/**
 * Typed bridge to the C++ host (CEF).
 *
 * - `bridge.send(channel, payload?)` — request/response over window.cronymax.browser.query.
 *   Validates payload (req schema) before serializing and validates response
 *   (res schema) before resolving. Errors are surfaced as Error rejections.
 * - `bridge.on(event, handler)` — subscribes to a broadcast event delivered
 *   by C++ via the internal dispatch hook. Validates payload (event schema)
 *   before invoking handler unless the event is in FastPathEvents.
 *
 * Terminal I/O (input, run, resize, stop) is routed directly to the Rust
 * runtime via window.cronymax.runtime process messages; terminal output
 * is subscribed through the same channel and dispatched as bridge events.
 * The channel and event names are narrowed to the registry; `payload` and
 * the handler argument are inferred from the schemas.
 */

import type { z } from "zod";
import {
  type ChannelName,
  Channels,
  type EventName,
  type EventPayloadOf,
  Events,
  FastPathEvents,
  type RequestOf,
  type ResponseOf,
} from "./browser";

type AnyEventHandler = (payload: unknown) => void;
const browserSubscriptions = new Map<string, Set<AnyEventHandler>>();

// Expose window.cronymax.browser so any built-in page can reach the
// browser-process IPC without importing this module.
// C++ (App::OnContextCreated) creates window.cronymax and pre-populates
// .browser.query / .browser.queryCancel; we spread both levels to preserve them.
if (window.cronymax?.browser) {
  window.cronymax.browser.on = function dispatch(event: string, payload: unknown) {
    const handlers = browserSubscriptions.get(event);
    // eslint-disable-next-line no-console
    if (event === "event") {
      console.log("[bridge] dispatch 'event'", handlers?.size ?? 0, "handlers", payload);
    }
    if (!handlers) return;
    if (typeof payload === "string") {
      try {
        payload = JSON.parse(payload);
      } catch {
        // leave as string
      }
    }
    const eventDef = (Events as Record<string, z.ZodTypeAny>)[event];
    if (eventDef && !FastPathEvents.has(event as EventName)) {
      const parsed = eventDef.safeParse(payload);
      if (!parsed.success) {
        // eslint-disable-next-line no-console
        console.error(`[bridge] inbound payload for "${event}" failed validation`, parsed.error.issues, payload);
        return;
      }
      payload = parsed.data;
    }
    for (const h of handlers) {
      try {
        h(payload);
      } catch (err) {
        // eslint-disable-next-line no-console
        console.error(`[bridge] handler for ${event} threw`, err);
      }
    }
  };
}
// Set runtime.on separately to avoid TypeScript issues from spreading
// a potentially-undefined optional object (which would make `send` optional).
// C++ (OnContextCreated) already populated window.cronymax.runtime.send; we
// only add on so JS can receive kMsgRuntimeEvent forwards.
if (window.cronymax?.runtime) {
  window.cronymax.runtime.on = (subId: string, event: unknown) => {
    const cb = runtimeSubscriptions.get(subId);
    if (cb) cb(event);
  };
}

// ---------------------------------------------------------------------------
// Navigates the nested Channels tree by a dot-path to find the leaf ChannelDef.
function lookupChannel(path: string): { req: z.ZodTypeAny; res: z.ZodTypeAny } | undefined {
  // biome-ignore lint/suspicious/noExplicitAny: dynamic traversal of nested Channels tree
  let node: any = Channels;
  for (const part of path.split(".")) {
    if (typeof node !== "object" || node === null || !(part in node)) return undefined;
    node = node[part] as unknown;
  }
  return typeof node === "object" && node !== null && "req" in node && "res" in node
    ? (node as { req: z.ZodTypeAny; res: z.ZodTypeAny })
    : undefined;
}

// ---------------------------------------------------------------------------
export const browser = {
  send<C extends ChannelName>(channel: C, payload?: RequestOf<C>): Promise<ResponseOf<C>> {
    const def = lookupChannel(channel);
    if (!def) {
      return Promise.reject(new Error(`unknown channel: ${String(channel)}`));
    }

    const reqResult = def.req.safeParse(payload ?? undefined);
    if (!reqResult.success) {
      return Promise.reject(
        new Error(`bridge.send(${String(channel)}) payload invalid: ${JSON.stringify(reqResult.error.issues)}`),
      );
    }

    // ── Fast path: binary msgpack via jsbSend ────────────────────────────────
    const jsbSend = window.cronymax?.browser?.send;
    if (typeof jsbSend === "function") {
      return jsbSend(channel, reqResult.data ?? null).then((result) => {
        const resResult = def.res.safeParse(result);
        if (!resResult.success) {
          // eslint-disable-next-line no-console
          console.error(`[bridge] response for "${String(channel)}" failed validation`, resResult.error.issues, result);
          return result as ResponseOf<C>;
        }
        return resResult.data as ResponseOf<C>;
      });
    }

    // ── Fallback: cefQuery (JSON string transport) ───────────────────────────
    // Envelope: { channel, payload? } where payload is the raw parsed data (not
    // re-stringified). C++ SplitEnvelope accepts either a string or an inline
    // object for the payload field. Omit the key entirely when data is undefined.
    const request = JSON.stringify(reqResult.data !== undefined ? { channel, payload: reqResult.data } : { channel });

    return new Promise((resolve, reject) => {
      const query = window.cefQuery;
      if (typeof query !== "function") {
        reject(new Error("cronymax.browser.query not available (running outside CEF?)"));
        return;
      }
      query({
        request,
        onSuccess: (response) => {
          let parsed: unknown = response;
          if (response) {
            try {
              parsed = JSON.parse(response);
            } catch {
              parsed = response;
            }
          }
          const resResult = def.res.safeParse(parsed);
          if (!resResult.success) {
            // eslint-disable-next-line no-console
            console.error(
              `[bridge] response for "${String(channel)}" failed validation`,
              resResult.error.issues,
              parsed,
            );
            resolve(parsed as ResponseOf<C>);
            return;
          }
          resolve(resResult.data as ResponseOf<C>);
        },
        onFailure: (code, message) => {
          reject(new Error(`bridge ${String(channel)} failed [${code}]: ${message}`));
        },
      });
    });
  },
  on<E extends EventName>(event: E, handler: (payload: EventPayloadOf<E>) => void): () => void {
    let set = browserSubscriptions.get(event);
    if (!set) {
      set = new Set();
      browserSubscriptions.set(event, set);
    }
    const wrapped = handler as AnyEventHandler;
    set.add(wrapped);
    return () => {
      set?.delete(wrapped);
    };
  },
};

// ---------------------------------------------------------------------------
// Runtime event routing
//
// `runtime.on("*", cb)` — wildcard: routed via broadcast_event("event", ...)
//   which already broadcasts ALL runtime events from WireSpaceEventCallback.
//   The full envelope is parsed and the inner event object is delivered to cb.
//
// `runtime.on("topic", cb)` — topic-specific: sends a subscribe ctrl request
//   to get a subscription UUID, then routes kMsgRuntimeEvent arrivals via
//   window.cronymax.runtime.on (called by C++ renderer app.cc).
// ---------------------------------------------------------------------------

/** UUID → callback for topic-specific runtime subscriptions. */
const runtimeSubscriptions = new Map<string, (event: unknown) => void>();
/** Wildcard handlers registered via runtime.on("*", cb). */
const runtimeWildcard = new Set<(event: unknown) => void>();

// Route "event" broadcasts (from WireSpaceEventCallback) to wildcard handlers.
// rawPayload is already a parsed object from the dispatch() fast-path; extract
// the inner event object and forward it directly — no JSON round-trip needed.
browser.on("event", (rawPayload: unknown) => {
  if (runtimeWildcard.size === 0) return;
  const envelope = rawPayload as Record<string, unknown>;
  const innerEvent = envelope.event !== undefined ? envelope.event : envelope;
  for (const cb of runtimeWildcard) {
    try {
      cb(innerEvent);
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error("[bridge] runtime wildcard handler threw", err);
    }
  }
});

// ---------------------------------------------------------------------------
// window.cronymax.shells — nested path-accumulating proxy for all channels.
// Usage (browser channel): shells.browser.shell.popover_close()
// Usage (runtime channel): shells.runtime.terminal.resize({ terminal_id, cols, rows })
// ---------------------------------------------------------------------------

/**
 * A ShellNode is both callable (it invokes the channel at the accumulated
 * path) and traversable (property access appends another segment to the path).
 *
 * The index signature uses `any` (not `ShellNode`) so that TypeScript's
 * `noUncheckedIndexedAccess` rule doesn't add `| undefined` at every property
 * access in call sites — `any | undefined` collapses to `any`.
 */
// biome-ignore lint/suspicious/noExplicitAny: index signature uses `any` so noUncheckedIndexedAccess does not add `| undefined` at call sites
export type ShellNode = ((payload?: unknown) => Promise<unknown>) & { readonly [K: string]: any };

/**
 * Derive a callable tree type from the nested Channels structure.
 * Leaf ChannelDef nodes become typed callable functions; intermediate nodes
 * become plain objects with typed children.
 */
type CallableTree<T> = T extends { req: infer Req; res: infer Res }
  ? Req extends z.ZodTypeAny
    ? Res extends z.ZodTypeAny
      ? (payload?: z.input<Req>) => Promise<z.infer<Res>>
      : never
    : never
  : { readonly [K in keyof T]: CallableTree<T[K]> };

/**
 * Fully-typed surface for `shells`. Browser channels (from `Channels`) get
 * concrete argument and return types; the runtime sub-tree is left as `any`
 * because runtime IPC paths are not in the Channels registry.
 */
export type Shells = CallableTree<typeof Channels> & {
  // biome-ignore lint/suspicious/noExplicitAny: runtime channels go to Rust IPC, not the typed Channels registry
  runtime: any;
};

export function makeShellsProxy(): ShellNode {
  function makeNode(path: string): ShellNode {
    const invoke = (payload?: unknown): Promise<unknown> => {
      // Browser channel: path is a known Channels key.
      if (lookupChannel(path) !== undefined) {
        return browser.send(path as ChannelName, payload as never);
      }
      // Runtime channel: path starts with "runtime."; derive kind by
      // replacing "." separators with "_".
      if (path.startsWith("runtime.")) {
        const kind = path.slice("runtime.".length).replace(/\./g, "_");
        const req: Record<string, unknown> = {
          kind,
          ...(typeof payload === "object" && payload !== null ? (payload as Record<string, unknown>) : {}),
        };
        return window.cronymax?.runtime?.send?.(req) ?? Promise.reject(new Error("cronymax.runtime not available"));
      }
      return Promise.reject(new Error(`[shells] unknown path: ${path}`));
    };
    return new Proxy(invoke as ShellNode, {
      get(_target, prop: string | symbol) {
        if (typeof prop !== "string") return undefined;
        return makeNode(`${path}.${prop}`);
      },
    });
  }

  // Root proxy: first property access starts path accumulation.
  return new Proxy(Function.prototype as unknown as ShellNode, {
    get(_target, prop: string | symbol) {
      if (typeof prop !== "string") return undefined;
      return makeNode(prop);
    },
  });
}

export const shells: Shells = makeShellsProxy() as unknown as Shells;

// ---------------------------------------------------------------------------
// runtimeSend — typed helper for Rust-runtime IPC via the shells path scheme.
//
// Equivalent to shells.runtime.<path>(payload) but avoids TypeScript index-
// signature nullability issues from noUncheckedIndexedAccess.
//
// `path` is the dot-separated sub-path after "runtime."; the proxy derives
// the Rust `kind` by replacing all "." with "_" (e.g. "terminal.resize"
// → kind "terminal_resize").
// ---------------------------------------------------------------------------
export function runtimeSend(path: string, payload?: Record<string, unknown>): Promise<unknown> {
  const kind = path.replace(/\./g, "_");
  const req: Record<string, unknown> = { kind, ...payload };
  return (
    window.cronymax?.runtime?.send?.(req) ??
    Promise.reject(new Error(`cronymax.runtime not available (runtime.${path})`))
  );
}

export type RuntimeControlRequest = Record<string, unknown>;
export const runtime = {
  /** Send a one-shot control request; resolves with the decoded response object. */
  send(req: RuntimeControlRequest): Promise<unknown> {
    return window.cronymax?.runtime?.send?.(req) ?? Promise.reject(new Error("cronymax.runtime not available"));
  },

  /**
   * Subscribe to a runtime topic.
   *
   * - `"*"` — wildcard, receives all runtime events via the existing
   *   broadcast_event("event", ...) path. No runtime subscription needed.
   * - Any other topic — sends a subscribe ctrl request to the runtime and
   *   routes kMsgRuntimeEvent arrivals via window.cronymax.runtime.on.
   *
   * The callback receives the inner event object as a plain JS object
   * ({sequence, emitted_at_ms, payload:{...}}).
   * Returns an unsubscribe function, or null if the runtime is unavailable.
   */
  on(topic: string, cb: (event: unknown) => void): (() => void) | null {
    if (topic === "*") {
      runtimeWildcard.add(cb);
      return () => {
        runtimeWildcard.delete(cb);
      };
    }

    if (!window.cronymax?.runtime?.send) return null;

    let subId: string | null = null;
    let unsubscribed = false;

    (window.cronymax.runtime.send({ kind: "subscribe", topic }) as Promise<unknown>)
      .then((resp) => {
        const uuid = (resp as Record<string, unknown>).subscription as string | undefined;
        if (!uuid) return;
        if (unsubscribed) {
          // Caller already unsubscribed before confirm arrived — clean up.
          window.cronymax?.runtime?.send?.({ kind: "unsubscribe", subscription: uuid });
          return;
        }
        subId = uuid;
        runtimeSubscriptions.set(uuid, cb);
      })
      .catch(() => {
        // Subscription failed; nothing to clean up.
      });

    return () => {
      unsubscribed = true;
      if (subId) {
        runtimeSubscriptions.delete(subId);
        window.cronymax?.runtime?.send?.({ kind: "unsubscribe", subscription: subId });
        subId = null;
      }
    };
  },
};
