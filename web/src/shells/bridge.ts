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

import { z } from "zod";
import {
  Channels,
  Events,
  FastPathEvents,
  type ChannelName,
  type EventName,
  type EventPayloadOf,
  type RequestOf,
  type ResponseOf,
} from "./browser";

type AnyEventHandler = (payload: unknown) => void;
const subscribers = new Map<string, Set<AnyEventHandler>>();

function dispatch(event: string, rawPayload: unknown) {
  const handlers = subscribers.get(event);
  // eslint-disable-next-line no-console
  if (event === "event")
    console.log(
      "[bridge] dispatch 'event'",
      handlers?.size ?? 0,
      "handlers",
      rawPayload,
    );
  if (!handlers) return;
  let payload = rawPayload;
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
      console.error(
        `[bridge] inbound payload for "${event}" failed validation`,
        parsed.error.issues,
        rawPayload,
      );
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
}

function send<C extends ChannelName>(
  channel: C,
  payload?: RequestOf<C>,
): Promise<ResponseOf<C>> {
  const def = Channels[channel];
  if (!def) {
    return Promise.reject(new Error(`unknown channel: ${String(channel)}`));
  }

  const reqResult = def.req.safeParse(payload ?? undefined);
  if (!reqResult.success) {
    return Promise.reject(
      new Error(
        `bridge.send(${String(channel)}) payload invalid: ` +
          JSON.stringify(reqResult.error.issues),
      ),
    );
  }

  // C++ legacy expects {channel, payload} as an outer JSON envelope, where
  // payload itself is JSON-stringified. Match the existing wire format.
  const wirePayload =
    reqResult.data === undefined ? "" : JSON.stringify(reqResult.data);
  const request = JSON.stringify({ channel, payload: wirePayload });

  return new Promise((resolve, reject) => {
    const query = window.cronymax?.browser?.query;
    if (typeof query !== "function") {
      reject(
        new Error(
          "cronymax.browser.query not available (running outside CEF?)",
        ),
      );
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
        reject(
          new Error(`bridge ${String(channel)} failed [${code}]: ${message}`),
        );
      },
    });
  });
}

function on<E extends EventName>(
  event: E,
  handler: (payload: EventPayloadOf<E>) => void,
): () => void {
  let set = subscribers.get(event);
  if (!set) {
    set = new Set();
    subscribers.set(event, set);
  }
  const wrapped = handler as AnyEventHandler;
  set.add(wrapped);
  return () => {
    set?.delete(wrapped);
  };
}

// ---------------------------------------------------------------------------
export const browser = { send, on };

// Expose window.cronymax.browser so any built-in page can reach the
// browser-process IPC without importing this module.
// C++ (App::OnContextCreated) creates window.cronymax and pre-populates
// .browser.query / .browser.queryCancel; we spread both levels to preserve them.
window.cronymax = {
  ...window.cronymax,
  browser: {
    ...window.cronymax?.browser,
    ...browser,
    // C++ (bridge_handler.cc, main_window.cc) calls this to deliver events.
    onDispatch: (event: string, payload: unknown) => {
      dispatch(event, payload);
    },
  },
};

export type RuntimeControlRequest = Record<string, unknown>;
export const runtime = {
  /** Send a one-shot control request; resolves with the raw JSON reply string. */
  send(req: RuntimeControlRequest): Promise<string> {
    return (
      window.cronymax?.runtime?.send?.(req) ??
      Promise.reject(new Error("cronymax.runtime not available"))
    );
  },

  /**
   * Subscribe to a runtime topic.
   * Returns an unsubscribe function, or null if the runtime is unavailable.
   * The callback receives the inner event object JSON string
   * (i.e. {sequence, emitted_at_ms, payload:{...}}).
   */
  subscribe(
    topic: string,
    cb: (eventJson: string) => void,
  ): (() => void) | null {
    return window.cronymax?.runtime?.subscribe?.(topic, cb) ?? null;
  },
};
