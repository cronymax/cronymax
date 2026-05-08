/**
 * XtermPane — mounts an xterm.js Terminal into a React ref.
 *
 * Wires:
 *   terminal:<tid> runtime event  →  xterm.write (raw bytes, no stripping)
 *   xterm.onData                  →  terminal.input
 *   ResizeObserver                →  fitAddon.fit() + terminal.resize
 */
import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { terminal } from "@/shells/runtime";
import { useTerminalOutput } from "@/hooks/useTerminalOutput";

interface Props {
  tid: string;
  onCwdChange?: (cwd: string) => void;
}

export function XtermPane({ tid, onCwdChange }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);

  // Mount xterm inside a RAF so the browser has finished laying out the
  // flex container.  Calling term.open() before the container has non-zero
  // dimensions produces an unresponsive terminal (onData never fires).
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const term = new Terminal({
      allowProposedApi: true,
      fontFamily: "Menlo, Monaco, 'Courier New', monospace",
      fontSize: 12,
      theme: {
        background: "#111317",
        foreground: "#e8edf2",
        cursor: "#e8edf2",
        selectionBackground: "#264f78",
      },
      cursorBlink: true,
      scrollback: 5000,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);

    let rafId: ReturnType<typeof requestAnimationFrame>;
    let disposed = false;

    rafId = requestAnimationFrame(() => {
      if (disposed) return;
      term.open(container);
      try {
        fit.fit();
        terminal.resize(tid, term.cols, term.rows);
      } catch {
        /* ignore */
      }

      term.focus();

      termRef.current = term;
      fitRef.current = fit;
    });

    // Input: xterm → pty
    const disposeData = term.onData((data) => {
      terminal.input(tid, data);
    });

    // Resize: container → fit → pty
    const ro = new ResizeObserver(() => {
      if (!fitRef.current || !termRef.current) return;
      try {
        fitRef.current.fit();
        const { cols, rows } = termRef.current;
        terminal.resize(tid, cols, rows);
      } catch {
        // ignore if terminal unmounted
      }
    });
    ro.observe(container);

    return () => {
      disposed = true;
      cancelAnimationFrame(rafId);
      disposeData.dispose();
      ro.disconnect();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
    // tid should be stable; re-mounting on tid change is intentional
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tid]);

  // Output: pty → xterm (raw, no stripping).
  // Also scan for OSC 7 (file://hostname/path) and OSC 1337 (CurrentDir=path)
  // sequences in the raw stream to extract CWD. Parsing here — rather than
  // via term.parser.registerOscHandler — catches sequences that arrive before
  // the RAF fires and works with any shell/prompt that emits either format.
  useTerminalOutput(tid, (data) => {
    termRef.current?.write(data);

    // OSC 7: ESC ] 7 ; <uri> BEL-or-ST
    // URI form: file://[host]/path  or just a bare path
    const osc7 = /\x1b]7;([^\x07\x1b]*)(?:\x07|\x1b\\)/g;
    let m: RegExpExecArray | null;
    while ((m = osc7.exec(data)) !== null) {
      const raw = m[1]!;
      try {
        const url = new URL(raw);
        onCwdChange?.(decodeURIComponent(url.pathname));
      } catch {
        if (raw) onCwdChange?.(raw);
      }
    }

    // OSC 1337 CurrentDir= (iTerm2 / many prompts)
    const osc1337 = /\x1b]1337;CurrentDir=([^\x07\x1b]*)(?:\x07|\x1b\\)/g;
    while ((m = osc1337.exec(data)) !== null) {
      const path = m[1]!;
      if (path) onCwdChange?.(path);
    }
  });

  return (
    <div
      ref={containerRef}
      className="h-full w-full overflow-hidden"
      onClick={() => termRef.current?.focus()}
    />
  );
}
