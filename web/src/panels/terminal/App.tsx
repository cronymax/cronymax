/**
 * Terminal panel — classic xterm.js terminal (replaces Wrap-style block renderer).
 *
 * Supports interactive programs (vim, htop, etc.) because output is forwarded
 * raw without ANSI stripping.
 *
 * AI block-actions (Explain / Fix / Retry) now live in the chat panel via
 * $-mode shell blocks.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";
import { shells } from "@/shells/bridge";
import { terminal as rt_terminal } from "@/shells/runtime";
import { useStore } from "./store";
import { XtermPane } from "./XtermPane";

/** Shorten a filesystem path for display: abbreviate /Users/x/... → ~/... */
function abbreviatePath(p: string): string {
  return p.replace(/^\/Users\/[^/]+/, "~").replace(/^\/home\/[^/]+/, "~");
}

export function App() {
  const [state, dispatch] = useStore();
  const startedRef = useRef<Set<string>>(new Set());

  // ── start helper (idempotent) ────────────────────────────────────────
  const startTerminal = useCallback(
    async (tid: string) => {
      if (startedRef.current.has(tid)) return;
      startedRef.current.add(tid);
      dispatch({ type: "markStarted", tid });
      try {
        await rt_terminal.start(tid);
      } catch (err) {
        console.warn("terminal.start failed", err);
        startedRef.current.delete(tid);
      }
    },
    [dispatch],
  );

  // ── initial load ─────────────────────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    shells.browser.terminal
      .list()
      .then(async (res) => {
        if (cancelled) return;
        const items = res?.items ?? [];
        for (const t of items) dispatch({ type: "ensurePane", tid: t.id });
        const activeTid = res?.active ?? items[0]?.id ?? null;
        if (activeTid) {
          dispatch({ type: "setActive", tid: activeTid });
          for (const t of items) void startTerminal(t.id);
        } else {
          // No terminals exist yet — create one for this panel.
          try {
            const newTid = await shells.browser.terminal.new();
            const tid = typeof newTid === "string" ? newTid : (newTid as { id: string }).id;
            dispatch({ type: "ensurePane", tid });
            dispatch({ type: "setActive", tid });
            await startTerminal(tid);
          } catch (e) {
            console.warn("terminal.new failed", e);
          }
        }
      })
      .catch((e) => console.warn("terminal.list failed", e));
    return () => {
      cancelled = true;
    };
  }, [dispatch, startTerminal]);

  // ── bridge events ────────────────────────────────────────────────────
  useBridgeEvent("terminal.created", (row) => {
    if (!row?.id) return;
    dispatch({ type: "ensurePane", tid: row.id });
    void startTerminal(row.id);
  });

  useBridgeEvent("terminal.removed", (p) => {
    if (!p?.id) return;
    dispatch({ type: "removePane", tid: p.id });
    startedRef.current.delete(p.id);
  });

  useBridgeEvent("terminal.switched", (p) => {
    if (!p?.id) return;
    dispatch({ type: "setActive", tid: p.id });
    void startTerminal(p.id);
  });

  useBridgeEvent("terminal.restart_requested", () => {
    void doRestart();
  });

  // ── restart ──────────────────────────────────────────────────────────
  const doRestart = useCallback(async () => {
    const tid = state.activeTid;
    if (!tid) return;
    try {
      await rt_terminal.stop(tid);
    } catch {
      // ignore
    }
    startedRef.current.delete(tid);
    dispatch({ type: "removePane", tid });
    dispatch({ type: "ensurePane", tid });
    void startTerminal(tid);
  }, [state.activeTid, dispatch, startTerminal]);

  // ── CWD tracking (per-tid, persists across tab switches) ─────────────
  const cwdMapRef = useRef<Record<string, string>>({});
  const [activeCwd, setActiveCwd] = useState("");

  // Restore stored CWD when the active tab changes.
  useEffect(() => {
    setActiveCwd(cwdMapRef.current[state.activeTid ?? ""] ?? "");
  }, [state.activeTid]);

  const handleCwdChange = useCallback(
    (cwd: string) => {
      if (state.activeTid) cwdMapRef.current[state.activeTid] = cwd;
      setActiveCwd(cwd);
    },
    [state.activeTid],
  );

  // ── render ───────────────────────────────────────────────────────────
  const activeTid = state.activeTid;

  return (
    <main className={`flex h-screen flex-col bg-[#292929] text-cronymax-title`}>
      {/* Title bar — shows "Terminal" label + current working directory */}
      <div className="flex shrink-0 items-center gap-2 border-b border-cronymax-border bg-cronymax-float px-4 py-1.5">
        <span className="shrink-0 font-mono text-xs font-semibold text-cronymax-title">Terminal</span>
        {activeCwd && (
          <span className="truncate rounded bg-cronymax-float px-2 py-0.5 font-mono text-[12px] text-cronymax-title">
            {abbreviatePath(activeCwd)}
          </span>
        )}
      </div>

      {/* Terminal with padding so it doesn't bleed to the panel edges */}
      <div className="flex-1 overflow-hidden p-4">
        {activeTid ? (
          <XtermPane key={activeTid} tid={activeTid} onCwdChange={handleCwdChange} />
        ) : (
          <div className="flex h-full items-center justify-center text-xs text-cronymax-muted">
            No terminal yet — create one from the sidebar.
          </div>
        )}
      </div>
    </main>
  );
}
