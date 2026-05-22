import { Check, ChevronDown, Loader2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import type { TraceEntry } from "./store";

interface LiveTask {
  toolCallId: string;
  tool: string;
  args: unknown;
  /** undefined = still running */
  result?: unknown;
  startedAt: number;
  finishedAt?: number;
  /** Runtime-measured duration in ms (preferred over finishedAt - startedAt). */
  durationMs?: number;
  terminal: boolean;
}

/**
 * Shows the most-recently-active tool calls for the currently streaming block.
 *
 * Rules:
 *   - Only visible while `isStreaming` is true.
 *   - Shows up to 5 most recent tasks (running first, then completed).
 *   - Completed tasks fade after a short delay (handled by parent if desired).
 */
export function LiveTasksView({ traceEntries, isStreaming }: { traceEntries: TraceEntry[]; isStreaming: boolean }) {
  const tasks = useMemo<LiveTask[]>(() => {
    const map = new Map<string, LiveTask>();
    for (const e of traceEntries) {
      if (e.kind === "tool_start") {
        map.set(e.toolCallId, {
          toolCallId: e.toolCallId,
          tool: e.tool,
          args: e.args,
          startedAt: e.ts,
          terminal: false,
        });
      } else if (e.kind === "tool_done") {
        const existing = map.get(e.toolCallId);
        if (existing) {
          map.set(e.toolCallId, {
            ...existing,
            result: e.result,
            finishedAt: e.ts,
            durationMs: e.durationMs,
            terminal: e.terminal,
          });
        }
      }
    }
    // Sort: running first, then by start time descending.
    return [...map.values()].sort((a, b) => {
      const aRunning = a.finishedAt === undefined ? 1 : 0;
      const bRunning = b.finishedAt === undefined ? 1 : 0;
      if (aRunning !== bRunning) return bRunning - aRunning;
      return b.startedAt - a.startedAt;
    });
  }, [traceEntries]);

  const hasRunningTask = tasks.some((t) => t.finishedAt === undefined);

  // Latest trace event timestamp — used to compute how long we've been waiting.
  const latestTs = useMemo(
    () => (traceEntries.length > 0 ? Math.max(...traceEntries.map((e) => e.ts)) : 0),
    [traceEntries],
  );

  // Tick every second while streaming so the elapsed display updates.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!isStreaming) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [isStreaming]);

  // Track which completed task rows the user has expanded.
  const [expandedIds, setExpandedIds] = useState<Set<string>>(() => new Set());

  const thinkingMs = isStreaming && !hasRunningTask && latestTs > 0 ? now - latestTs : 0;

  if (!isStreaming) return null;
  // Render if we have tasks to show OR if the thinking indicator is active.
  if (tasks.length === 0 && thinkingMs <= 5000) return null;

  const visible = tasks.slice(0, 5);

  function toggleExpand(id: string) {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function fmtElapsed(ms: number): string {
    const s = Math.floor(ms / 1000);
    if (s < 60) return `${s}s`;
    return `${Math.floor(s / 60)}m ${s % 60}s`;
  }

  return (
    <div className="flex flex-col gap-0.5 px-1 py-1">
      {thinkingMs > 5000 && (
        <div className="flex items-center gap-1.5 rounded px-2 py-1 text-[11px] text-amber-500/80">
          <Loader2 className="size-3 shrink-0 animate-spin" />
          <span className="font-mono">Thinking… {fmtElapsed(thinkingMs)}</span>
        </div>
      )}
      {visible.map((task) => {
        const running = task.finishedAt === undefined;
        const expanded = !running && expandedIds.has(task.toolCallId);
        return (
          <div key={task.toolCallId} className={cn("overflow-hidden rounded", !running && "border border-border/50")}>
            {/* Header row */}
            <div
              role={running ? undefined : "button"}
              tabIndex={running ? undefined : 0}
              onClick={running ? undefined : () => toggleExpand(task.toolCallId)}
              onKeyDown={
                running
                  ? undefined
                  : (e) => {
                      if (e.key === "Enter" || e.key === " ") toggleExpand(task.toolCallId);
                    }
              }
              className={cn(
                "flex items-center gap-1.5 px-2 py-1 text-[11px] transition",
                running ? "bg-primary/10 text-primary" : "bg-muted/30 text-muted-foreground",
                !running && "cursor-pointer select-none hover:bg-muted/50",
              )}
            >
              {running ? (
                <Loader2 className="size-3 shrink-0 animate-spin" />
              ) : (
                <Check className="size-3 shrink-0 text-primary" />
              )}
              <span className="max-w-[120px] truncate font-mono font-medium">{task.tool}</span>
              <span className="flex-1 truncate opacity-60">{formatArgs(task.args)}</span>
              {!running && (task.durationMs != null || task.finishedAt) && (
                <span className="shrink-0 tabular-nums opacity-50">
                  {task.durationMs != null
                    ? task.durationMs < 1000
                      ? `${task.durationMs}ms`
                      : `${(task.durationMs / 1000).toFixed(1)}s`
                    : `${((task.finishedAt! - task.startedAt) / 1000).toFixed(1)}s`}
                </span>
              )}
              {!running && (
                <ChevronDown
                  className={cn("size-3 shrink-0 opacity-40 transition-transform", expanded && "rotate-180")}
                />
              )}
            </div>
            {/* Expanded args + result */}
            {expanded && (
              <div className="flex flex-col gap-2 border-t border-border/50 bg-background/50 px-2.5 py-2">
                <div>
                  <div className="mb-0.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground opacity-60">
                    Args
                  </div>
                  <pre className="max-h-[150px] overflow-y-auto whitespace-pre-wrap break-all rounded bg-background px-2 py-1 font-mono text-[10px] text-muted-foreground">
                    {JSON.stringify(parseArgsValue(task.args), null, 2)}
                  </pre>
                </div>
                <div>
                  <div className="mb-0.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground opacity-60">
                    Result
                  </div>
                  <pre className="max-h-[150px] overflow-y-auto whitespace-pre-wrap break-all rounded bg-background px-2 py-1 font-mono text-[10px] text-muted-foreground">
                    {task.result != null ? JSON.stringify(task.result, null, 2) : "(no result)"}
                  </pre>
                </div>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function formatArgs(args: unknown): string {
  // tool_start "arguments" arrives as a raw JSON string from the runtime;
  // parse it so we can extract meaningful fields.
  let value: unknown = args;
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (trimmed.startsWith("{")) {
      try {
        value = JSON.parse(trimmed);
      } catch {
        return trimmed.slice(0, 60);
      }
    } else {
      return trimmed.slice(0, 60);
    }
  }
  if (!value || typeof value !== "object") return "";
  const obj = value as Record<string, unknown>;
  // Show the most meaningful arg: path / command / query / name / url.
  const key = ["path", "file_path", "command", "query", "name", "url", "file"].find((k) => typeof obj[k] === "string");
  if (key) return String(obj[key]).slice(0, 60);
  const first = Object.values(obj).find((v) => typeof v === "string");
  if (typeof first === "string") return first.slice(0, 60);
  return "";
}

/** Parse a raw JSON string arg into an object (or return the value as-is). */
function parseArgsValue(args: unknown): unknown {
  if (typeof args === "string") {
    const trimmed = args.trim();
    if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
      try {
        return JSON.parse(trimmed);
      } catch {
        /* fall through */
      }
    }
  }
  return args;
}
