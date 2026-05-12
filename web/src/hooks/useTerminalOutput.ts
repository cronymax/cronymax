import { useRuntimeEvent } from "./useRuntimeEvent";

/**
 * Decode a base64 string into a Uint8Array of raw bytes.
 * Using Uint8Array lets xterm.js decode UTF-8 properly instead of
 * treating the binary data as Latin-1 text (which garbles multi-byte chars).
 */
function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

/**
 * Subscribe to PTY output for a specific terminal.
 * `onData` receives raw UTF-8 bytes as a Uint8Array — pass directly to
 * xterm's `write()` so multi-byte characters render correctly.
 * Scoped to the given `tid` at the subscription level (no fan-out filtering).
 * Auto-unsubscribes on unmount and resubscribes after a space switch.
 */
export function useTerminalOutput(tid: string, onData: (data: Uint8Array) => void): void {
  useRuntimeEvent(`terminal:${tid}`, (eventJson: string) => {
    try {
      const ev = JSON.parse(eventJson) as Record<string, unknown>;
      const pl = ev?.payload as Record<string, unknown> | undefined;
      if (pl?.kind !== "raw") return;
      const dataObj = pl?.data as Record<string, unknown> | undefined;
      const b64 = dataObj?.data as string | undefined;
      if (!b64) return;
      onData(base64ToBytes(b64));
    } catch {
      // Ignore malformed events.
    }
  });
}
