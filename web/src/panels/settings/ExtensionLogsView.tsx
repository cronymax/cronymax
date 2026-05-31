/**
 * Settings → Extensions → per-extension Logs view (P9-T04b).
 *
 * Reached from the Extensions tab's per-row "Logs" button; replaces the list
 * until dismissed. A channel dropdown (the `stdout`/`stderr` console fallbacks
 * plus any `createOutputChannel` channels), an optional level filter (channel
 * logs only — they carry levels), and a time-window filter feed a tailing,
 * auto-scrolling log view. Data comes from the `extension.log_*` control
 * requests; the view polls every 2s while open so live output appears.
 */
import { ArrowLeft, Copy, Loader2, RefreshCw, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Caption } from "@/components/ui/typography";
import { cn } from "@/lib/utils";
import { extensionRegistry, type LogChannelInfo, type LogEntry } from "@/shells/runtime";

const TIME_WINDOWS = [
  { id: "all", label: "All time", ms: 0 },
  { id: "5m", label: "Last 5 min", ms: 5 * 60_000 },
  { id: "1h", label: "Last hour", ms: 60 * 60_000 },
  { id: "24h", label: "Last 24h", ms: 24 * 60 * 60_000 },
];

const LEVELS = ["all", "trace", "debug", "info", "warn", "error"];

const LEVEL_CLASS: Record<string, string> = {
  error: "text-destructive",
  warn: "text-amber-600 dark:text-amber-400",
  info: "text-foreground",
  debug: "text-muted-foreground",
  trace: "text-muted-foreground",
};

function fmtTime(ms?: number): string {
  if (!ms) return "";
  const d = new Date(ms);
  const p = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

export function ExtensionLogsView({ ext, onBack }: { ext: { id: string; name: string }; onBack: () => void }) {
  const [channels, setChannels] = useState<LogChannelInfo[]>([]);
  const [channel, setChannel] = useState<string>("stdout");
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [structured, setStructured] = useState(false);
  const [truncated, setTruncated] = useState(false);
  const [level, setLevel] = useState("all");
  const [timeWindow, setTimeWindow] = useState("all");
  const [refreshing, setRefreshing] = useState(false);

  const scrollRef = useRef<HTMLDivElement | null>(null);
  // Track whether the user is pinned to the bottom so auto-scroll doesn't
  // yank them up when they've scrolled back to read older lines.
  const atBottomRef = useRef(true);

  // Channel list — load once per extension. `stdout`/`stderr` always present.
  useEffect(() => {
    void (async () => {
      try {
        const { channels } = await extensionRegistry.logChannels(ext.id);
        setChannels(channels);
      } catch {
        /* ignore — keep the stdout/stderr defaults */
      }
    })();
  }, [ext.id]);

  const sinceMs = useMemo(() => {
    const w = TIME_WINDOWS.find((t) => t.id === timeWindow);
    return w && w.ms > 0 ? Date.now() - w.ms : undefined;
  }, [timeWindow]);

  const refetch = useCallback(async () => {
    try {
      const res = await extensionRegistry.logRead(ext.id, channel, { sinceMs });
      setStructured(res.structured);
      setTruncated(res.truncated);
      setEntries(res.entries);
    } catch {
      /* ignore transient runtime errors; the next poll retries */
    }
  }, [ext.id, channel, sinceMs]);

  // Initial + filter-change load, then poll every 2s while open.
  useEffect(() => {
    void refetch();
    const id = window.setInterval(() => void refetch(), 2000);
    return () => window.clearInterval(id);
  }, [refetch]);

  // Level filter is client-side and applies only to structured channel logs.
  const shown = useMemo(() => {
    if (!structured || level === "all") return entries;
    return entries.filter((e) => (e.level ?? "info") === level);
  }, [entries, structured, level]);

  // Auto-scroll to the newest line when pinned to the bottom.
  useEffect(() => {
    const el = scrollRef.current;
    if (el && atBottomRef.current) el.scrollTop = el.scrollHeight;
  }, [shown]);

  const onScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    atBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  }, []);

  const onManualRefresh = useCallback(async () => {
    setRefreshing(true);
    try {
      await refetch();
    } finally {
      setRefreshing(false);
    }
  }, [refetch]);

  const onClear = useCallback(async () => {
    try {
      await extensionRegistry.logClear(ext.id, channel);
      setEntries([]);
    } catch {
      /* ignore */
    }
  }, [ext.id, channel]);

  const onCopy = useCallback(() => {
    const text = shown
      .map((e) => {
        const ts = fmtTime(e.t);
        const lvl = e.level ? `[${e.level}] ` : "";
        return `${ts ? `${ts} ` : ""}${lvl}${e.text}`;
      })
      .join("\n");
    void navigator.clipboard?.writeText(text).catch(() => undefined);
  }, [shown]);

  return (
    <div className="flex h-full flex-col">
      {/* header */}
      <div className="flex shrink-0 items-center gap-2 px-4 pt-4 pb-2">
        <Button variant="ghost" size="icon" onClick={onBack} title="Back to extensions">
          <ArrowLeft className="size-4" />
        </Button>
        <span className="truncate text-sm font-medium text-foreground">Logs · {ext.name}</span>
      </div>

      {/* filters */}
      <div className="flex shrink-0 flex-wrap items-center gap-2 px-4 pb-2">
        <Select value={channel} onValueChange={setChannel}>
          <SelectTrigger className="h-7 w-[220px] text-xs">
            <SelectValue placeholder="Channel" />
          </SelectTrigger>
          <SelectContent>
            {channels.map((c) => (
              <SelectItem key={c.id} value={c.id} className="text-xs">
                {c.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <Select value={level} onValueChange={setLevel} disabled={!structured}>
          <SelectTrigger
            className="h-7 w-[110px] text-xs"
            title={structured ? "Level" : "Levels apply to channel logs only"}
          >
            <SelectValue placeholder="Level" />
          </SelectTrigger>
          <SelectContent>
            {LEVELS.map((l) => (
              <SelectItem key={l} value={l} className="text-xs">
                {l === "all" ? "All levels" : l}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <Select value={timeWindow} onValueChange={setTimeWindow}>
          <SelectTrigger className="h-7 w-[120px] text-xs">
            <SelectValue placeholder="Time" />
          </SelectTrigger>
          <SelectContent>
            {TIME_WINDOWS.map((t) => (
              <SelectItem key={t.id} value={t.id} className="text-xs">
                {t.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <Button
          variant="ghost"
          size="icon"
          className="ml-auto"
          onClick={() => void onManualRefresh()}
          title="Refresh now"
        >
          {refreshing ? <Loader2 className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
        </Button>
      </div>

      {/* log body */}
      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="mx-4 flex-1 overflow-y-auto rounded-md border border-border bg-muted/30 p-2 font-mono text-[11px] leading-relaxed"
      >
        {shown.length === 0 ? (
          <div className="flex h-full items-center justify-center text-muted-foreground">No log output.</div>
        ) : (
          <>
            {truncated && (
              <div className="mb-1 text-[10px] italic text-muted-foreground">
                …earlier lines truncated (showing most recent)
              </div>
            )}
            {shown.map((e, i) => (
              <div key={`${e.t ?? 0}-${i}`} className="whitespace-pre-wrap break-words">
                {e.t ? <span className="text-muted-foreground">{fmtTime(e.t)} </span> : null}
                {e.level ? (
                  <span className={cn("font-medium", LEVEL_CLASS[e.level] ?? "text-foreground")}>[{e.level}] </span>
                ) : null}
                <span className="text-foreground">{e.text}</span>
              </div>
            ))}
          </>
        )}
      </div>

      {/* footer actions */}
      <div className="flex shrink-0 items-center justify-between gap-2 px-4 py-3">
        <Caption>{structured ? "Structured channel (NDJSON)" : "Raw console output"}</Caption>
        <div className="flex items-center gap-1.5">
          <Button variant="outline" size="sm" onClick={onCopy} disabled={shown.length === 0}>
            <Copy className="size-3.5" />
            Copy
          </Button>
          <Button variant="outline" size="sm" onClick={() => void onClear()} title="Truncate this channel's log file">
            <Trash2 className="size-3.5" />
            Clear
          </Button>
        </div>
      </div>
    </div>
  );
}
