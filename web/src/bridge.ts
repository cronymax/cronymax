/**
 * Typed bridge to the C++ host (CEF).
 *
 * - `bridge.send(channel, payload?)` — request/response over window.cefQuery.
 *   Validates payload (req schema) before serializing and validates response
 *   (res schema) before resolving. Errors are surfaced as Error rejections.
 * - `bridge.on(event, handler)` — subscribes to a broadcast event delivered
 *   by C++ via the internal dispatch hook. Validates payload (event schema)
 *   before invoking handler unless the event is in FastPathEvents.
 *
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
} from "./bridge_channels";

declare global {
  interface Window {
    cefQuery?: (opts: {
      request: string;
      onSuccess: (response: string) => void;
      onFailure: (errorCode: number, errorMessage: string) => void;
      persistent?: boolean;
    }) => number;
    cronymax?: {
      send(method: string, params?: unknown): Promise<unknown>;
      subscribe(
        topic: string,
        callback: (payload: unknown) => void,
      ): () => void;
      reconnect(): void;
    };
  }
}

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
    if (typeof window.cefQuery !== "function") {
      reject(new Error("cefQuery not available (running outside CEF?)"));
      return;
    }
    window.cefQuery({
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

export const bridge = { send, on };

// C++ (bridge_handler.cc, main_window.cc) calls window.__aiDesktopDispatch to
// deliver broadcast events. Keep the assignment but don't expose it in the
// Window type — callers inside this module use bridge.on() instead.
(window as unknown as Record<string, unknown>)["__aiDesktopDispatch"] = (
  event: string,
  payload: unknown,
) => {
  dispatch(event, payload);
};

// Reconnect window.cronymax after a space switch.
// space.switch_loading is a browser-process-originated broadcast that arrives
// via __aiDesktopDispatch even while the Rust runtime is restarting.
on("space.switch_loading", ({ loading }: { loading: boolean }) => {
  if (!loading && typeof window.cronymax?.reconnect === "function") {
    window.cronymax.reconnect();
  }
});
