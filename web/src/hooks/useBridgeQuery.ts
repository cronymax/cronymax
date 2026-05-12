import { useCallback, useEffect, useState } from "react";
import { browser } from "@/shells/bridge";
import type { ChannelName, RequestOf, ResponseOf } from "@/shells/browser";

export interface BridgeQueryResult<T> {
  data: T | undefined;
  error: Error | undefined;
  loading: boolean;
  send: () => void;
}

/**
 * Send a request to the C++ host on mount and whenever `send()` is called.
 * `payload` is treated as a stable reference for the lifetime of the hook;
 * call `send()` to refetch.
 */
export function useBridgeQuery<C extends ChannelName>(
  channel: C,
  payload?: RequestOf<C>,
): BridgeQueryResult<ResponseOf<C>> {
  const [data, setData] = useState<ResponseOf<C> | undefined>(undefined);
  const [error, setError] = useState<Error | undefined>(undefined);
  const [loading, setLoading] = useState(true);
  const [trigger, setTrigger] = useState(0);

  const refetch = useCallback(() => setTrigger((n) => n + 1), []);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(undefined);
    browser
      .send(channel, payload as RequestOf<C>)
      .then((res) => {
        if (cancelled) return;
        setData(res);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setError(err instanceof Error ? err : new Error(String(err)));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [channel, trigger]);

  return { data, error, loading, send: refetch };
}
