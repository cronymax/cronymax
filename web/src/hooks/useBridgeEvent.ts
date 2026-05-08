import { useEffect, useRef } from "react";
import { browser } from "@/shells/bridge";
import type { EventName, EventPayloadOf } from "@/shells/browser";

/**
 * Subscribe to a broadcast event from the C++ host. Auto-unsubscribes on
 * unmount. The handler is wrapped so closure changes don't re-subscribe.
 */
export function useBridgeEvent<E extends EventName>(
  event: E,
  handler: (payload: EventPayloadOf<E>) => void,
): void {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;

  useEffect(() => {
    return browser.on(event, (payload) => {
      handlerRef.current(payload);
    });
  }, [event]);
}
