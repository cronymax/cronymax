/**
 * FlowInstancesBar — compact status bar above the chat prompt showing all
 * active flow runs associated with the current chat session.
 *
 * Data source: `shells.browser.activity.snapshot()` filtered by `sessionId`,
 * updated reactively via `browser.on("event", ...)` run_status events.
 *
 * Each entry shows: `{flow_run_id_short} #{n} [status_icon]`
 * Clicking an entry expands a per-run summary listing the sub-agent runs
 * grouped under that flow run.
 */

import { AlertCircle, Check, ChevronDown, ChevronUp, Clock, Loader2, ShieldAlert } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { browser, shells } from "@/shells/bridge";

// ── Types ─────────────────────────────────────────────────────────────────

interface SubRunEntry {
  id: string;
  agent_id: string | null;
  status: string;
}

interface FlowRunEntry {
  flow_run_id: string;
  /** Sequence number among all flow runs in this session (1-based) */
  index: number;
  /** Aggregate status derived from all sub-runs */
  status: "running" | "human_review_pending" | "completed" | "failed";
  subRuns: SubRunEntry[];
}

type AggregateStatus = FlowRunEntry["status"];

// ── Helpers ───────────────────────────────────────────────────────────────

function computeStatus(subRuns: SubRunEntry[]): AggregateStatus {
  if (subRuns.length === 0) return "running";
  const statuses = subRuns.map((r) => r.status);
  if (statuses.some((s) => s === "awaiting_review")) return "human_review_pending";
  if (statuses.some((s) => s === "running" || s === "pending")) return "running";
  if (statuses.every((s) => s === "succeeded")) return "completed";
  return "failed";
}

function StatusIcon({ status }: { status: AggregateStatus }) {
  if (status === "human_review_pending") return <ShieldAlert className="h-3.5 w-3.5 text-amber-400 animate-pulse" />;
  if (status === "running") return <Loader2 className="h-3.5 w-3.5 animate-spin text-primary" />;
  if (status === "completed") return <Check className="h-3.5 w-3.5 text-green-500" />;
  return <AlertCircle className="h-3.5 w-3.5 text-destructive" />;
}

function SubRunStatusIcon({ status }: { status: string }) {
  if (status === "awaiting_review") return <Clock className="h-3 w-3 text-amber-400" />;
  if (status === "running" || status === "pending") return <Loader2 className="h-3 w-3 animate-spin text-primary" />;
  if (status === "succeeded") return <Check className="h-3 w-3 text-green-500" />;
  if (status === "failed" || status === "cancelled") return <AlertCircle className="h-3 w-3 text-destructive" />;
  return <div className="h-3 w-3 rounded-full bg-muted-foreground/40" />;
}

function SubRunRow({ run }: { run: SubRunEntry }) {
  const label = run.agent_id ?? run.id.slice(0, 8);
  return (
    <div className="flex items-center gap-2 px-3 py-0.5">
      <SubRunStatusIcon status={run.status} />
      <span className="truncate font-mono text-xs text-foreground w-28">{label}</span>
      <Badge variant="outline" className="ml-auto text-xs px-1.5 py-0 h-4 font-normal">
        {run.status}
      </Badge>
    </div>
  );
}

// ── Component ─────────────────────────────────────────────────────────────

interface Props {
  /** The chat session ID — used to filter runs by session. */
  sessionId: string | null | undefined;
}

export function FlowInstancesBar({ sessionId }: Props) {
  const [flowRuns, setFlowRuns] = useState<FlowRunEntry[]>([]);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  // We maintain a ref for the sub-run map so event handlers can access
  // it without stale closures.
  const subRunsRef = useRef<Map<string, Map<string, SubRunEntry>>>(new Map());
  const flowRunOrderRef = useRef<Map<string, number>>(new Map());

  const rebuildFlowRuns = useCallback(() => {
    const entries: FlowRunEntry[] = [];
    for (const [flowRunId, runsMap] of subRunsRef.current.entries()) {
      const subRuns = Array.from(runsMap.values());
      const index = flowRunOrderRef.current.get(flowRunId) ?? entries.length + 1;
      entries.push({
        flow_run_id: flowRunId,
        index,
        status: computeStatus(subRuns),
        subRuns,
      });
    }
    entries.sort((a, b) => a.index - b.index);
    setFlowRuns(entries);
  }, []);

  // ── Load initial state from activity snapshot ─────────────────────────
  useEffect(() => {
    if (!sessionId) return;

    shells.browser.activity
      .snapshot()
      .then((resp: { runs?: unknown[]; pending_reviews?: unknown[] }) => {
        const rawRuns = resp.runs ?? [];
        let seq = 0;
        for (const raw of rawRuns) {
          const r = raw as Record<string, unknown>;
          const runSessionId = (r.session_id as string | null) ?? null;
          const flowRunId = (r.flow_run_id as string | null) ?? null;
          if (runSessionId !== sessionId || !flowRunId) continue;

          if (!flowRunOrderRef.current.has(flowRunId)) {
            seq += 1;
            flowRunOrderRef.current.set(flowRunId, seq);
            subRunsRef.current.set(flowRunId, new Map());
          }
          const runsMap = subRunsRef.current.get(flowRunId)!;
          const runId = (r.id as string) ?? "";
          runsMap.set(runId, {
            id: runId,
            agent_id: (r.agent_id as string | null) ?? null,
            status: parseStatus(r.status),
          });
        }
        rebuildFlowRuns();
      })
      .catch(() => undefined);
  }, [sessionId, rebuildFlowRuns]);

  // ── Subscribe to live run_status events ───────────────────────────────
  useEffect(() => {
    if (!sessionId) return;

    const off = browser.on("event", (raw: unknown) => {
      const ev = raw as Record<string, unknown> | null;
      if (!ev || ev.tag !== "event") return;
      const inner = (ev.event as Record<string, unknown> | undefined) ?? {};
      const pl = (inner.payload as Record<string, unknown> | undefined) ?? {};
      const kind = pl.kind as string | undefined;

      if (kind === "run_status") {
        const runId = (pl.run_id as string | undefined) ?? "";
        const status = (pl.status as string | undefined) ?? "pending";
        const flowRunId = (pl.flow_run_id as string | undefined) ?? null;
        const agentId = (pl.agent_id as string | undefined) ?? null;
        const evSessionId = (pl.session_id as string | undefined) ?? null;

        if (flowRunId) {
          const alreadyTracked = subRunsRef.current.has(flowRunId);
          const sessionMatch = evSessionId ? evSessionId === sessionId : alreadyTracked;
          if (!sessionMatch) return;

          if (!subRunsRef.current.has(flowRunId)) {
            const nextSeq = subRunsRef.current.size + 1;
            flowRunOrderRef.current.set(flowRunId, nextSeq);
            subRunsRef.current.set(flowRunId, new Map());
          }
          const runsMap = subRunsRef.current.get(flowRunId)!;
          runsMap.set(runId, { id: runId, agent_id: agentId, status });
          rebuildFlowRuns();
        }
      }
    });

    return () => off();
  }, [sessionId, rebuildFlowRuns]);

  if (flowRuns.length === 0) return null;

  return (
    <div className="rounded-lg border border-border bg-card shadow-sm overflow-hidden">
      {flowRuns.map((run) => (
        <Collapsible
          key={run.flow_run_id}
          open={expandedId === run.flow_run_id}
          onOpenChange={(open) => setExpandedId(open ? run.flow_run_id : null)}
        >
          <CollapsibleTrigger className="flex w-full items-center gap-2 px-3 py-1.5 text-left transition-colors hover:bg-accent/50">
            <StatusIcon status={run.status} />
            <span className="font-mono text-xs text-muted-foreground">{run.flow_run_id.slice(0, 8)}</span>
            <Badge variant="outline" className="text-xs px-1.5 py-0 h-4 font-normal">
              #{run.index}
            </Badge>
            <Badge
              variant="outline"
              className={
                "ml-auto text-xs px-1.5 py-0 h-4 font-medium " +
                (run.status === "human_review_pending"
                  ? "border-amber-500/50 text-amber-400"
                  : run.status === "completed"
                    ? "border-green-500/50 text-green-500"
                    : run.status === "failed"
                      ? "border-destructive/50 text-destructive"
                      : "text-muted-foreground")
              }
            >
              {statusLabel(run.status)}
            </Badge>
            {expandedId === run.flow_run_id ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
          </CollapsibleTrigger>

          <CollapsibleContent className="border-t border-border/50 bg-background pb-1">
            {run.subRuns.map((sub) => (
              <SubRunRow key={sub.id} run={sub} />
            ))}
          </CollapsibleContent>
        </Collapsible>
      ))}
    </div>
  );
}

// ── Helpers ───────────────────────────────────────────────────────────────

function parseStatus(raw: unknown): string {
  if (typeof raw === "string") return raw;
  if (typeof raw === "object" && raw !== null) {
    const s = (raw as Record<string, unknown>).status;
    if (typeof s === "string") return s;
  }
  return "pending";
}

function statusLabel(status: AggregateStatus): string {
  switch (status) {
    case "human_review_pending":
      return "Review pending";
    case "running":
      return "Running";
    case "completed":
      return "Done";
    case "failed":
      return "Failed";
  }
}
