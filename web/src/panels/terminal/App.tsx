import { useEffect, useRef, useCallback, type FormEvent } from "react";
import { bridge } from "@/bridge";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";
import { Icon } from "@/shared/components/Icon";
import type { IconName } from "@/shared/icons";
import { useStore, type Block, type PaneState } from "./store";

// ── small helpers ──────────────────────────────────────────────────────

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(s < 10 ? 2 : 1)}s`;
  const m = Math.floor(s / 60);
  const rs = Math.round(s - m * 60);
  return `${m}m${rs}s`;
}

// Channel set for AI block-actions — keep in sync with action labels below.
type BlockAction = "Explain" | "Fix" | "Retry";

// ── components ─────────────────────────────────────────────────────────

function ActionBar({
  block,
  onAction,
}: {
  block: Block;
  onAction: (action: BlockAction) => void;
}) {
  const aiBtns: { icon: IconName; label: string; action: BlockAction }[] = [
    { icon: "sparkle", label: "Explain", action: "Explain" },
    { icon: "tools", label: "Fix", action: "Fix" },
    { icon: "refresh", label: "Retry", action: "Retry" },
  ];
  return (
    <div className="flex items-center justify-between gap-1.5 border-t border-[#2b3138] bg-[#111317] px-2.5 py-1.5">
      <div className="flex gap-1.5">
        {aiBtns.map(({ icon, label, action }) => (
          <button
            key={action}
            type="button"
            onClick={() => onAction(action)}
            className="inline-flex h-6 items-center gap-1 rounded border border-[rgba(123,140,255,0.35)] bg-[#1c222a] px-2.5 text-[11px] text-[#c7d2fe] transition hover:border-[#7b8cff] hover:bg-[rgba(123,140,255,0.12)] hover:text-white"
          >
            <Icon name={icon} size={12} aria-hidden="true" />
            {label}
          </button>
        ))}
      </div>
      <div className="flex gap-1.5">
        <button
          type="button"
          onClick={() =>
            navigator.clipboard.writeText(block.command).catch(() => {})
          }
          className="h-6 rounded border border-[#2f3a48] bg-transparent px-2.5 text-[11px] text-[#cbd5e1] transition hover:border-[#475569] hover:bg-[#252d38] hover:text-[#f8fafc]"
        >
          Copy cmd
        </button>
        <button
          type="button"
          onClick={() =>
            navigator.clipboard.writeText(block.output).catch(() => {})
          }
          className="h-6 rounded border border-[#2f3a48] bg-transparent px-2.5 text-[11px] text-[#cbd5e1] transition hover:border-[#475569] hover:bg-[#252d38] hover:text-[#f8fafc]"
        >
          Copy out
        </button>
      </div>
    </div>
  );
}

function CommandBlock({
  block,
  onAction,
}: {
  block: Block;
  onAction: (action: BlockAction) => void;
}) {
  const borderClass =
    block.status === "running"
      ? "border-l-[#fbbf24]"
      : block.status === "ok"
        ? "border-l-[#4ade80]"
        : "border-l-[#f87171]";
  const statusGlyph =
    block.status === "running" ? "●" : block.status === "ok" ? "✓" : "✗";
  const statusColor =
    block.status === "running"
      ? "text-[#fbbf24] animate-pulse"
      : block.status === "ok"
        ? "text-[#4ade80]"
        : "text-[#f87171]";
  const dur =
    block.endedAt !== null
      ? formatDuration(block.endedAt - block.startedAt)
      : "running…";
  return (
    <div
      className={`mx-2.5 my-1.5 overflow-hidden rounded-lg border border-[#2b3138] border-l-[3px] bg-[#15191f] transition-colors hover:border-[#475569] ${borderClass}`}
    >
      <div className="flex items-center gap-2.5 border-b border-[#2b3138] bg-[#1c222a] px-3 py-1.5 text-xs">
        <span className={`w-3 text-center text-[10px] ${statusColor}`}>
          {statusGlyph}
        </span>
        <span className="flex-1 truncate text-[#e8edf2] before:font-semibold before:text-[#8bd3dd] before:content-['$_']">
          {block.command || "(no command)"}
        </span>
        <span className="text-[11px] text-[#6b7280]">
          {new Date(block.startedAt).toLocaleTimeString()}
        </span>
        <span className="min-w-[60px] text-right font-mono text-[11px] tabular-nums text-[#6b7280]">
          {dur}
        </span>
        {block.exitCode !== null && (
          <span
            className={
              "rounded px-1.5 py-px text-[10px] font-semibold tracking-wider " +
              (block.exitCode === 0
                ? "bg-[rgba(74,222,128,0.15)] text-[#4ade80]"
                : "bg-[rgba(248,113,113,0.15)] text-[#f87171]")
            }
          >
            exit {block.exitCode}
          </span>
        )}
      </div>
      {block.output && (
        <pre className="m-0 max-h-[400px] overflow-auto whitespace-pre-wrap break-words px-3 py-2 text-xs leading-[1.5] text-[#d1d5db]">
          {block.output}
        </pre>
      )}
      <ActionBar block={block} onAction={onAction} />
    </div>
  );
}

function Pane({ tid, pane }: { tid: string; pane: PaneState }) {
  const paneRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom on output growth.
  useEffect(() => {
    const el = paneRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [pane.blocks, pane.rawOutput]);

  const dispatchAction = useCallback((block: Block, action: BlockAction) => {
    bridge
      .send("agent.task_from_command", {
        action,
        command: block.command,
        output: block.output.slice(0, 2000),
        exit_code: block.exitCode ?? -1,
      })
      .catch((e) => console.warn("agent dispatch failed", e));
  }, []);

  return (
    <div ref={paneRef} className="h-full overflow-y-auto" data-tid={tid}>
      {pane.restoredNotice && (
        <div className="px-3 py-1 text-center text-[11px] text-[#4a5568]">
          {pane.restoredNotice}
        </div>
      )}
      {pane.blocks.map((b) => (
        <CommandBlock
          key={b.id}
          block={b}
          onAction={(a) => dispatchAction(b, a)}
        />
      ))}
      {pane.rawOutput && (
        <pre className="m-0 whitespace-pre-wrap break-words px-3 py-1 text-xs leading-[1.5] text-[#9ea8b3]">
          {pane.rawOutput}
        </pre>
      )}
    </div>
  );
}

// ── App ────────────────────────────────────────────────────────────────

export function App() {
  const [state, dispatch] = useStore();
  const inputRef = useRef<HTMLInputElement>(null);
  // Track tids we've already issued terminal.start for, so React StrictMode
  // double-invokes don't fire it twice.
  const startedRef = useRef<Set<string>>(new Set());

  // ── start helper (idempotent) ────────────────────────────────────────
  const startTerminal = useCallback(
    async (tid: string) => {
      if (startedRef.current.has(tid)) return;
      startedRef.current.add(tid);
      dispatch({ type: "markStarted", tid });
      try {
        await bridge.send("terminal.start", { id: tid });
        try {
          const blocks = await bridge.send("terminal.blocks_load", {});
          const count = Array.isArray(blocks) ? blocks.length : 0;
          dispatch({ type: "markRestored", tid, count });
        } catch (err) {
          console.warn("terminal.blocks_load failed", err);
        }
      } catch (err) {
        console.warn("terminal.start failed", err);
        startedRef.current.delete(tid);
      }
    },
    [dispatch],
  );

  // ── initial load ────────────────────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    bridge
      .send("terminal.list")
      .then((res) => {
        if (cancelled) return;
        const items = res?.items ?? [];
        items.forEach((t) => dispatch({ type: "ensurePane", tid: t.id }));
        const initial = res?.active ?? items[0]?.id ?? null;
        if (initial) {
          dispatch({ type: "setActive", tid: initial });
          for (const t of items) startTerminal(t.id);
        }
      })
      .catch((e) => console.warn("terminal.list failed", e));
    return () => {
      cancelled = true;
    };
  }, [dispatch, startTerminal]);

  // ── persistence side effect: save pendingSave blocks ─────────────────
  useEffect(() => {
    for (const [tid, pane] of Object.entries(state.panes)) {
      for (const blk of pane.blocks) {
        if (!blk.pendingSave || blk.endedAt === null) continue;
        // Mark optimistically so we don't double-fire.
        dispatch({ type: "markSaved", tid, blockId: blk.id });
        bridge
          .send("terminal.block_save", {
            command: blk.command,
            output: blk.output,
            exit_code: blk.exitCode ?? -1,
            started_at: blk.startedAt,
            ended_at: blk.endedAt,
          })
          .catch((e) => console.warn("block_save failed", e));
      }
    }
  }, [state.panes, dispatch]);

  // ── bridge events ───────────────────────────────────────────────────
  useBridgeEvent("terminal.output", (p) => {
    if (!p?.id) return;
    dispatch({ type: "ensurePane", tid: p.id });
    dispatch({
      type: "output",
      tid: p.id,
      chunk: p.data ?? "",
      now: Date.now(),
    });
  });

  useBridgeEvent("terminal.exit", (p) => {
    if (!p?.id) return;
    dispatch({ type: "exit", tid: p.id, code: p.code ?? -1, now: Date.now() });
    startedRef.current.delete(p.id);
  });

  useBridgeEvent("terminal.created", (row) => {
    if (!row?.id) return;
    dispatch({ type: "ensurePane", tid: row.id });
    startTerminal(row.id);
  });

  useBridgeEvent("terminal.removed", (p) => {
    if (!p?.id) return;
    dispatch({ type: "removePane", tid: p.id });
    startedRef.current.delete(p.id);
  });

  useBridgeEvent("terminal.switched", (p) => {
    if (!p?.id) return;
    dispatch({ type: "setActive", tid: p.id });
    startTerminal(p.id);
    inputRef.current?.focus();
  });

  useBridgeEvent("terminal.restart_requested", () => {
    void doRestart();
  });

  // ── restart ─────────────────────────────────────────────────────────
  const doRestart = useCallback(async () => {
    const tid = state.activeTid;
    if (!tid) return;
    try {
      await bridge.send("terminal.stop", { id: tid });
    } catch {
      // ignore
    }
    startedRef.current.delete(tid);
    dispatch({ type: "restartClear", tid });
    void startTerminal(tid);
  }, [state.activeTid, dispatch, startTerminal]);

  // ── submit ──────────────────────────────────────────────────────────
  const onSubmit = useCallback(
    (e: FormEvent<HTMLFormElement>) => {
      e.preventDefault();
      const input = inputRef.current;
      if (!input) return;
      const command = input.value.trim();
      input.value = "";
      if (!command) return;
      const tid = state.activeTid;
      if (!tid) return;
      dispatch({ type: "submit", tid, command, now: Date.now() });
      bridge
        .send("terminal.input", { id: tid, data: command + "\n" })
        .catch((err) => console.warn("terminal.input failed", err));
    },
    [state.activeTid, dispatch],
  );

  // ── render ──────────────────────────────────────────────────────────
  const activeTid = state.activeTid;
  const activePane = activeTid ? state.panes[activeTid] : null;

  return (
    <main className="flex h-screen min-h-[220px] flex-col border-t border-[#2b3138] bg-[#111317] text-[#e8edf2]">
      <div className="flex-1 overflow-hidden py-2">
        {activeTid && activePane ? (
          <Pane tid={activeTid} pane={activePane} />
        ) : (
          <div className="flex h-full items-center justify-center text-xs text-[#6b7280]">
            No terminal yet — create one from the sidebar.
          </div>
        )}
      </div>
      <form
        onSubmit={onSubmit}
        className="m-0 flex items-center gap-2 border-t border-[#2b3138] bg-[#15191f] px-3 py-1.5"
      >
        <span className="text-[#8bd3dd]">$</span>
        <input
          ref={inputRef}
          autoFocus
          autoComplete="off"
          className="w-full border-0 bg-transparent font-mono text-[#f4f7fb] outline-none"
        />
      </form>
    </main>
  );
}
