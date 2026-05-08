import { useRuntimeEvent } from "./useRuntimeEvent";

/**
 * Subscribe to PTY output for a specific terminal.
 * `onData` receives decoded UTF-8 output chunks.
 * Scoped to the given `tid` at the subscription level (no fan-out filtering).
 * Auto-unsubscribes on unmount and resubscribes after a space switch.
 */
export function useTerminalOutput(
  tid: string,
  onData: (data: string) => void,
): void {
  useRuntimeEvent(`terminal:${tid}`, (eventJson: string) => {
    try {
      const ev = JSON.parse(eventJson) as Record<string, unknown>;
      const pl = ev?.payload as Record<string, unknown> | undefined;
      if (pl?.kind !== "raw") return;
      const dataObj = pl?.data as Record<string, unknown> | undefined;
      const b64 = dataObj?.data as string | undefined;
      if (!b64) return;
      onData(atob(b64));
    } catch {
      // Ignore malformed events.
    }
  });
}
