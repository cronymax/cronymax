/**
 * useStatusDotState — derive a 4-state status indicator from inbox + errors.
 *
 * Returns one of:
 *   off       — no events, no activity
 *   activity  — recent non-needs-action event broadcast in last 5s
 *   attention — needs_action_count > 0 or unread_count > 0
 *   error     — recent `error` event in last 30s
 *
 * Subscribes to `event` broadcasts to refresh in real time and polls
 * `inbox.list` (state=all) on a slow interval as a fallback.
 */

import { useEffect, useState } from "react";
import { bridge } from "@/bridge";
import type { AppEvent } from "@/types/events";

export type StatusDot = "off" | "activity" | "attention" | "error";

export function useStatusDotState(): StatusDot {
  const [unread, setUnread] = useState(0);
  const [needsAction, setNeedsAction] = useState(0);
  const [lastErrorAt, setLastErrorAt] = useState(0);
  const [lastActivityAt, setLastActivityAt] = useState(0);
  const [, tick] = useState(0);

  // Refresh derivations every 1s so the activity/error windows expire.
  useEffect(() => {
    const id = window.setInterval(() => tick((n) => n + 1), 1000);
    return () => window.clearInterval(id);
  }, []);

  // Poll inbox counts.
  useEffect(() => {
    let cancelled = false;
    async function refresh() {
      try {
        const res = await bridge.send("inbox.list", {
          state: "unread",
          limit: 1,
        });
        if (cancelled) return;
        setUnread(res.unread_count);
        setNeedsAction(res.needs_action_count);
      } catch {
        // ignore
      }
    }
    void refresh();
    const id = window.setInterval(refresh, 15000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  // Subscribe to event broadcasts.
  useEffect(() => {
    void bridge.send("events.subscribe", {}).catch(() => {});
    const off = bridge.on("event", (payload) => {
      const e = payload as AppEvent;
      const now = Date.now();
      if (e.kind === "error") setLastErrorAt(now);
      else setLastActivityAt(now);
      // Bump inbox counts heuristically; full refresh on next poll cycle.
      if (
        e.kind === "review_event" &&
        e.payload.verdict === "request_changes"
      ) {
        setNeedsAction((n) => n + 1);
      }
    });
    return () => off();
  }, []);

  const now = Date.now();
  if (now - lastErrorAt < 30_000) return "error";
  if (needsAction > 0 || unread > 0) return "attention";
  if (now - lastActivityAt < 5_000) return "activity";
  return "off";
}

const COLORS: Record<StatusDot, string> = {
  off: "bg-cronymax-title/20",
  activity: "bg-emerald-400 animate-pulse",
  attention: "bg-amber-400",
  error: "bg-red-500",
};

interface StatusDotProps {
  state?: StatusDot;
  onClick?: () => void;
  className?: string;
}

export function StatusDot({ state, onClick, className }: StatusDotProps) {
  const auto = useStatusDotState();
  const s = state ?? auto;
  return (
    <button
      type="button"
      aria-label={`status: ${s}`}
      title={`status: ${s}`}
      onClick={onClick}
      className={
        "h-1.5 w-1.5 rounded-full transition-colors " +
        COLORS[s] +
        (className ? " " + className : "")
      }
    />
  );
}
