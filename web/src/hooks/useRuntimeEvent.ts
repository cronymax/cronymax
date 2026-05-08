import { useEffect, useRef, useState } from "react";
import { browser, runtime } from "@/shells/bridge";

/**
 * Subscribe to a runtime topic via `window.cronymax.runtime.subscribe`.
 * Auto-unsubscribes on unmount. The handler is stable-referenced so closure
 * changes never force a resubscription.
 *
 * Automatically resubscribes after a space switch (runtime restart) by
 * watching the `space.switch_loading` bridge event.
 *
 * The callback receives the inner event object JSON string:
 *   `{ sequence, emitted_at_ms, payload: { ... } }`
 */
export function useRuntimeEvent(
  topic: string,
  handler: (eventJson: string) => void,
): void {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;

  // Bumped to zero when the runtime restarts after a space switch so the
  // subscription effect below re-runs and creates a fresh subscription on
  // the new runtime instance.
  const [epoch, setEpoch] = useState(0);

  useEffect(() => {
    return browser.on("space.switch_loading", ({ loading }) => {
      if (!loading) setEpoch((n) => n + 1);
    });
  }, []);

  useEffect(() => {
    const unsub = runtime.subscribe(topic, (ev) => handlerRef.current(ev));
    return () => unsub?.();
  }, [topic, epoch]);
}
