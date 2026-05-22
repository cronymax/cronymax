/**
 * FlowGanttChart — Gantt-style timeline for a flow run.
 *
 * Shows one horizontal bar per agent, where the x-axis is wall-clock time
 * (relative to the first agent's startedAt) and the y-axis lists agent names.
 * Bars grow live while agents are still running (1-second tick).
 *
 * FlowTaskTree — compact collapsible tree listing agent status + elapsed time.
 * Rendered above the Gantt chart in the thread view.
 *
 * Both components consume `nodeConversations` directly from the Redux store
 * (already populated by the child-session subscription in App.tsx), so no
 * additional snapshot fetch or runtime subscription is needed.
 */

import { Activity, ChevronDown, GitBranch } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import type { NodeConversation, StatusKind } from "./store";

// ── Shared helpers ────────────────────────────────────────────────────────

function isActiveStatus(status: StatusKind): boolean {
  return status === "running" || status === "awaiting_review";
}

function isTerminalStatus(status: StatusKind): boolean {
  return status === "succeeded" || status === "failed";
}

function statusTextClass(status: StatusKind): string {
  switch (status) {
    case "running":
      return "text-primary";
    case "awaiting_review":
      return "text-amber-400";
    case "succeeded":
      return "text-green-400";
    case "failed":
      return "text-red-400";
    default:
      return "text-muted-foreground/50";
  }
}

function formatDuration(startMs: number, endMs: number): string {
  const diff = Math.max(0, Math.round((endMs - startMs) / 1000));
  if (diff < 60) return `${diff}s`;
  const m = Math.floor(diff / 60);
  const s = diff % 60;
  return s > 0 ? `${m}m ${s}s` : `${m}m`;
}

/** Sort NodeConversation[] by startedAt ascending (null last). */
function sortedAgents(conversations: Record<string, NodeConversation>): NodeConversation[] {
  return Object.values(conversations).sort((a, b) => {
    if (a.startedAt === null) return 1;
    if (b.startedAt === null) return -1;
    return a.startedAt - b.startedAt;
  });
}

// ── StatusDot ─────────────────────────────────────────────────────────────

function StatusDot({ status }: { status: StatusKind }) {
  if (status === "running") {
    return <span className="inline-flex h-2 w-2 rounded-full bg-primary animate-pulse shrink-0" />;
  }
  if (status === "awaiting_review") {
    return <span className="inline-flex h-2 w-2 rounded-full bg-amber-400 animate-pulse shrink-0" />;
  }
  if (status === "succeeded") {
    return (
      <svg
        className="h-3 w-3 text-green-400 shrink-0"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={2.5}
        aria-label="Succeeded"
      >
        <polyline points="20 6 9 17 4 12" />
      </svg>
    );
  }
  if (status === "failed") {
    return (
      <svg
        className="h-3 w-3 text-red-400 shrink-0"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={2.5}
        aria-label="Failed"
      >
        <line x1="18" y1="6" x2="6" y2="18" />
        <line x1="6" y1="6" x2="18" y2="18" />
      </svg>
    );
  }
  // pending
  return (
    <span className="inline-flex h-2 w-2 rounded-full border border-muted-foreground/30 bg-muted-foreground/10 shrink-0" />
  );
}

// ── FlowTaskTree ──────────────────────────────────────────────────────────

/**
 * Compact collapsible tree panel showing each agent's status and elapsed time.
 * Place above the Gantt chart in the thread view.
 */
export function FlowTaskTree({
  conversations,
  flowId,
}: {
  conversations: Record<string, NodeConversation>;
  flowId?: string;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const [, setTick] = useState(0);

  const agents = sortedAgents(conversations);
  const hasActive = agents.some((a) => isActiveStatus(a.status));

  // Tick every second while agents are running so elapsed times update live.
  useEffect(() => {
    if (!hasActive) return;
    const id = setInterval(() => setTick((t) => t + 1), 1000);
    return () => clearInterval(id);
  }, [hasActive]);

  if (agents.length === 0) return null;

  const now = Date.now();
  const doneCount = agents.filter((a) => isTerminalStatus(a.status)).length;

  return (
    <Collapsible
      open={!collapsed}
      onOpenChange={(v) => setCollapsed(!v)}
      className="rounded-lg border border-border bg-card text-xs"
    >
      <CollapsibleTrigger asChild>
        <Button variant="ghost" size="sm" className="group/collapse w-full justify-between rounded-lg px-3">
          <span className="flex items-center gap-2">
            <GitBranch />
            <span className="font-semibold">{flowId ?? "flow"}</span>
          </span>
          <span className="flex items-center gap-1.5 text-muted-foreground">
            <span className="text-[10px]">
              {doneCount}/{agents.length} done
            </span>
            <ChevronDown className="transition-transform duration-200 group-data-[state=open]/collapse:rotate-180" />
          </span>
        </Button>
      </CollapsibleTrigger>

      <CollapsibleContent className="overflow-hidden data-[state=open]:animate-collapsible-down data-[state=closed]:animate-collapsible-up">
        <div className="border-t border-border px-3 py-2 flex flex-col gap-1">
          {agents.map((agent, i) => {
            const isLast = i === agents.length - 1;
            const endMs = agent.endedAt ?? (isActiveStatus(agent.status) ? now : null);
            const elapsed = agent.startedAt && endMs ? formatDuration(agent.startedAt, endMs) : null;

            return (
              <div key={agent.runId} className="flex items-center gap-2 py-0.5">
                <span className="w-3 text-center font-mono text-muted-foreground select-none shrink-0">
                  {isLast ? "└" : "├"}
                </span>
                <StatusDot status={agent.status} />
                <span className="font-medium text-foreground truncate">{agent.agentId}</span>
                <span className={cn("text-[10px]", statusTextClass(agent.status))}>
                  {agent.status.replace(/_/g, " ")}
                </span>
                {elapsed && (
                  <span className="ml-auto font-mono text-[10px] text-muted-foreground shrink-0">{elapsed}</span>
                )}
              </div>
            );
          })}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}

// ── FlowGanttChart ────────────────────────────────────────────────────────

/** Pastel bar color per status. */
function barColorClass(status: StatusKind): string {
  switch (status) {
    case "running":
      return "bg-primary/60";
    case "awaiting_review":
      return "bg-amber-400/60";
    case "succeeded":
      return "bg-green-500/55";
    case "failed":
      return "bg-red-500/55";
    default:
      return "bg-muted-foreground/15";
  }
}

/**
 * Gantt-chart style trajectory panel.
 *
 * X-axis — wall-clock seconds relative to the first agent's start.
 * Y-axis — one row per agent, ordered by startedAt.
 * Bars   — span from startedAt to endedAt (or now for active agents).
 *
 * Grows live while any agent is running (1-second timer tick).
 */
export function FlowGanttChart({
  conversations,
  selectedFlow,
}: {
  conversations: Record<string, NodeConversation>;
  selectedFlow?: string;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const [, setTick] = useState(0);

  const agents = sortedAgents(conversations).filter((a) => a.startedAt !== null);
  const hasActive = agents.some((a) => isActiveStatus(a.status));

  // 1-second timer to animate running bars.
  useEffect(() => {
    if (!hasActive) return;
    const id = setInterval(() => setTick((t) => t + 1), 1000);
    return () => clearInterval(id);
  }, [hasActive]);

  if (agents.length === 0) return null;

  const now = Date.now();
  const minStart = agents[0]!.startedAt!;
  const maxEnd = Math.max(
    minStart + 1000,
    ...agents.map((a) => {
      if (isActiveStatus(a.status)) return now;
      return a.endedAt ?? a.startedAt! + 500;
    }),
  );
  const totalSpan = Math.max(maxEnd - minStart, 1);
  const totalSecs = Math.round(totalSpan / 1000);

  // Build evenly-spaced time-axis ticks (4–6 ticks).
  const rawInterval = totalSecs <= 10 ? 2 : totalSecs <= 30 ? 5 : totalSecs <= 120 ? 30 : totalSecs <= 300 ? 60 : 120;
  const ticks: number[] = [];
  for (let t = 0; t <= totalSecs; t += rawInterval) ticks.push(t);
  if (ticks[ticks.length - 1] !== totalSecs) ticks.push(totalSecs);

  return (
    <Collapsible
      open={!collapsed}
      onOpenChange={(v) => setCollapsed(!v)}
      className="rounded-lg border border-border bg-card text-xs"
    >
      {/* Header */}
      <CollapsibleTrigger asChild>
        <Button variant="ghost" size="sm" className="group/collapse w-full justify-between rounded-lg px-3">
          <span className="flex items-center gap-2">
            <Activity />
            <span className="font-semibold truncate">{selectedFlow ?? "trajectory"}</span>
          </span>
          <span className="flex items-center gap-1.5 text-muted-foreground">
            <span className="font-mono text-[10px]">{totalSecs}s</span>
            <ChevronDown className="transition-transform duration-200 group-data-[state=open]/collapse:rotate-180" />
          </span>
        </Button>
      </CollapsibleTrigger>

      {/* Gantt body */}
      <CollapsibleContent className="overflow-hidden data-[state=open]:animate-collapsible-down data-[state=closed]:animate-collapsible-up">
        <div className="border-t border-border px-3 py-2.5">
          {/* Agent rows */}
          <div className="flex flex-col gap-1.5">
            {agents.map((agent) => {
              const startMs = agent.startedAt!;
              const endMs = isActiveStatus(agent.status) ? now : (agent.endedAt ?? startMs + 500);

              const leftPct = ((startMs - minStart) / totalSpan) * 100;
              const widthPct = Math.max(0.3, ((endMs - startMs) / totalSpan) * 100);

              const isActive = isActiveStatus(agent.status);
              const elapsed = formatDuration(startMs, endMs);

              return (
                <div key={agent.runId} className="flex items-center gap-2 h-5">
                  {/* Agent label — fixed-width right-aligned */}
                  <span
                    className="w-[72px] shrink-0 text-right text-xs text-muted-foreground truncate"
                    title={agent.agentId}
                  >
                    {agent.agentId}
                  </span>

                  {/* Timeline bar area */}
                  <div className="flex-1 relative h-3 rounded-sm bg-muted/40 overflow-hidden">
                    <div
                      className={cn(
                        "absolute top-0 h-full rounded-sm",
                        barColorClass(agent.status),
                        isActive && "animate-pulse",
                      )}
                      style={{ left: `${leftPct}%`, width: `${widthPct}%` }}
                      title={`${agent.agentId}: ${agent.status} (${elapsed})`}
                    />
                  </div>

                  {/* Elapsed — fixed-width right of bar */}
                  <span className="w-9 shrink-0 text-right font-mono text-[10px] text-muted-foreground">{elapsed}</span>
                </div>
              );
            })}
          </div>

          {/* Time axis */}
          <div className="flex items-start gap-2 mt-1.5">
            <span className="w-[72px] shrink-0" />
            <div className="flex-1 relative h-3">
              {ticks.map((t) => {
                const leftPct = totalSecs > 0 ? (t / totalSecs) * 100 : 0;
                return (
                  <span
                    key={t}
                    className="absolute text-[9px] text-muted-foreground/70 font-mono select-none"
                    style={{
                      left: `${leftPct}%`,
                      transform: t === 0 ? "none" : t === totalSecs ? "translateX(-100%)" : "translateX(-50%)",
                    }}
                  >
                    {t}s
                  </span>
                );
              })}
            </div>
            <span className="w-9 shrink-0" />
          </div>
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
