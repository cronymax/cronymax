import { useCallback, useEffect, useRef, useState } from "react";
import type { z } from "zod";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { browser, shells } from "@/shells/bridge";
import type { AppEvent, InboxRowSchema } from "@/types/events";

type InboxRow = z.infer<typeof InboxRowSchema>;

interface SnoozeOption {
  label: string;
  ms: number;
}

const SNOOZE_OPTIONS: SnoozeOption[] = [
  { label: "1h", ms: 60 * 60 * 1000 },
  { label: "4h", ms: 4 * 60 * 60 * 1000 },
  { label: "Tomorrow", ms: 18 * 60 * 60 * 1000 },
];

const NEEDS_ACTION_KINDS = new Set(["review_event", "error", "handoff"]);

export function App() {
  const [rows, setRows] = useState<InboxRow[]>([]);
  const [stateFilter, setStateFilter] = useState<"unread" | "read" | "snoozed" | "all">("unread");
  const [unreadCount, setUnreadCount] = useState(0);
  const [needsActionCount, setNeedsActionCount] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const stateRef = useRef(stateFilter);
  stateRef.current = stateFilter;

  const refresh = useCallback(async () => {
    try {
      const res = await shells.browser.inbox.list({
        state: stateRef.current,
        limit: 200,
      });
      setRows(res.rows);
      setUnreadCount(res.unread_count);
      setNeedsActionCount(res.needs_action_count);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh, stateFilter]);

  // Refresh when relevant new events arrive.
  useEffect(() => {
    void shells.browser.events.subscribe({}).catch(() => {
      /* ignore */
    });
    const off = browser.on("event", (payload) => {
      const e = payload as AppEvent;
      if (NEEDS_ACTION_KINDS.has(e.kind)) {
        void refresh();
      }
    });
    return () => off();
  }, [refresh]);

  async function markRead(id: string) {
    try {
      await shells.browser.inbox.read({ event_id: id });
      void refresh();
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error("[inbox] read failed", err);
    }
  }

  async function snooze(id: string, ms: number) {
    try {
      await shells.browser.inbox.snooze({
        event_id: id,
        snooze_until: Date.now() + ms,
      });
      void refresh();
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error("[inbox] snooze failed", err);
    }
  }

  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      <header className="flex items-center gap-2 border-b border-border bg-card px-3 py-2">
        <div className="text-sm font-medium">Inbox</div>
        <div className="text-xs opacity-60">
          {unreadCount} unread · {needsActionCount} need action
        </div>
        <div className="flex-1" />
        <div className="flex overflow-hidden rounded border border-border">
          {(["unread", "read", "snoozed", "all"] as const).map((s) => (
            <button
              key={s}
              type="button"
              className={
                "px-2 py-1 text-xs " +
                (stateFilter === s ? "bg-primary text-primary-foreground" : "bg-card hover:bg-accent")
              }
              onClick={() => setStateFilter(s)}
            >
              {s}
            </button>
          ))}
        </div>
      </header>

      {error && <div className="border-b border-red-500/40 bg-red-900/30 px-3 py-1 text-xs text-red-200">{error}</div>}

      <div className="flex-1 overflow-y-auto">
        {rows.length === 0 && <div className="p-4 text-center text-xs opacity-60">(no items in “{stateFilter}”)</div>}
        {rows.map((row) => (
          <Row
            key={row.event_id}
            row={row}
            onRead={() => markRead(row.event_id)}
            onSnooze={(ms) => snooze(row.event_id, ms)}
          />
        ))}
      </div>
    </div>
  );
}

interface RowProps {
  row: InboxRow;
  onRead: () => void;
  onSnooze: (ms: number) => void;
}

function Row({ row, onRead, onSnooze }: RowProps) {
  return (
    <div className="flex items-start gap-2 border-b border-border px-3 py-2">
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2 text-xs">
          <span className="rounded bg-card px-1.5 py-0.5 font-mono">{row.kind || "event"}</span>
          <span className="opacity-60 font-mono truncate">{row.flow_id}</span>
          <span
            className={
              "rounded px-1.5 py-0.5 text-xs uppercase " +
              (row.state === "unread"
                ? "bg-amber-700/40 text-amber-200"
                : row.state === "snoozed"
                  ? "bg-card text-foreground/70"
                  : "bg-card text-foreground/50")
            }
          >
            {row.state}
          </span>
        </div>
        <div className="mt-1 font-mono text-xs opacity-50 truncate">id: {row.event_id}</div>
      </div>
      <div className="flex flex-col items-end gap-1">
        {row.state !== "read" && (
          <Button size="sm" onClick={onRead}>
            Acknowledge
          </Button>
        )}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="sm">
              Snooze ▾
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {SNOOZE_OPTIONS.map((opt) => (
              <DropdownMenuItem key={opt.label} onClick={() => onSnooze(opt.ms)}>
                {opt.label}
              </DropdownMenuItem>
            ))}
            <DropdownMenuItem
              onClick={() => {
                const hrs = window.prompt("Snooze for how many hours?", "8");
                const n = hrs ? Number(hrs) : Number.NaN;
                if (Number.isFinite(n) && n > 0) {
                  onSnooze(n * 60 * 60 * 1000);
                }
              }}
            >
              Custom…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  );
}
