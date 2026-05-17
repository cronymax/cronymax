import { useMemo } from "react";
import type { TraceEntry } from "./store";

interface LiveTask {
  toolCallId: string;
  tool: string;
  args: unknown;
  /** undefined = still running */
  result?: unknown;
  startedAt: number;
  finishedAt?: number;
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
            terminal: e.terminal,
          });
        }
      }
    }
    // Sort: running first, then by start time descending
    return [...map.values()].sort((a, b) => {
      const aRunning = a.finishedAt === undefined ? 1 : 0;
      const bRunning = b.finishedAt === undefined ? 1 : 0;
      if (aRunning !== bRunning) return bRunning - aRunning;
      return b.startedAt - a.startedAt;
    });
  }, [traceEntries]);

  if (!isStreaming || tasks.length === 0) return null;

  const visible = tasks.slice(0, 5);

  return (
    <div className="flex flex-col gap-0.5 px-1 py-1">
      {visible.map((task) => {
        const running = task.finishedAt === undefined;
        return (
          <div
            key={task.toolCallId}
            className={`flex items-center gap-1.5 rounded px-2 py-1 text-[11px] transition ${
              running ? "bg-primary/10 text-primary" : "bg-muted/30 text-muted-foreground"
            }`}
          >
            {running ? <span className="animate-pulse">⟳</span> : <span className="text-green-500">✓</span>}
            <span className="font-mono font-medium truncate max-w-[120px]">{task.tool}</span>
            <span className="flex-1 truncate opacity-60">{formatArgs(task.args)}</span>
            {!running && task.finishedAt && (
              <span className="shrink-0 tabular-nums opacity-50">
                {((task.finishedAt - task.startedAt) / 1000).toFixed(1)}s
              </span>
            )}
          </div>
        );
      })}
    </div>
  );
}

function formatArgs(args: unknown): string {
  if (!args || typeof args !== "object") return "";
  const obj = args as Record<string, unknown>;
  // Show the most meaningful arg: path / command / query / name / url
  const key = ["path", "command", "query", "name", "url", "file"].find((k) => typeof obj[k] === "string");
  if (key) return String(obj[key]).slice(0, 60);
  const first = Object.values(obj).find((v) => typeof v === "string");
  if (typeof first === "string") return first.slice(0, 60);
  return "";
}
