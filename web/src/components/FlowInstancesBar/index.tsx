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

import { useCallback, useEffect, useRef, useState } from "react";
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
  if (status === "human_review_pending") {
    return (
      <span className="inline-block h-2.5 w-2.5 rounded-full bg-amber-400 animate-pulse" title="Human review pending" />
    );
  }
  if (status === "running") {
    return (
      <svg
        className="inline-block h-3 w-3 animate-spin text-cronymax-primary"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={2}
        aria-label="Running"
      >
        <path strokeLinecap="round" strokeLinejoin="round" d="M4 12a8 8 0 018-8v4m8 4a8 8 0 01-8 8v-4" />
      </svg>
    );
  }
  if (status === "completed") {
    return (
      <svg
        className="inline-block h-3 w-3 text-green-400"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={2.5}
        aria-label="Completed"
      >
        <polyline points="20 6 9 17 4 12" />
      </svg>
    );
  }
  // failed / default
  return (
    <svg
      className="inline-block h-3 w-3 text-red-400"
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

function SubRunRow({ run }: { run: SubRunEntry }) {
  const label = run.agent_id ?? run.id.slice(0, 8);
  const statusColor =
    run.status === "succeeded"
      ? "text-green-400"
      : run.status === "failed" || run.status === "cancelled"
        ? "text-red-400"
        : run.status === "awaiting_review"
          ? "text-amber-400"
          : run.status === "running"
            ? "text-cronymax-primary"
            : "text-cronymax-caption";

  return (
    <div className="flex items-center gap-2 px-3 py-0.5">
      <span className="truncate text-[11px] text-cronymax-title font-mono w-28">{label}</span>
      <span className={`text-[10px] ${statusColor}`}>{run.status}</span>
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
    // Sort by insertion order (index)
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

        // We only care about runs for our session that belong to a flow run.
        // If the event doesn't carry session_id, we can't filter — include it
        // only if we already track this run.
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
          runsMap.set(runId, {
            id: runId,
            agent_id: agentId,
            status,
          });
          rebuildFlowRuns();
        }
      }
    });

    return () => off();
  }, [sessionId, rebuildFlowRuns]);

  // Don't render if there are no flow runs for this session.
  if (flowRuns.length === 0) return null;

  return (
    <div className="border-b border-cronymax-border bg-cronymax-float">
      {flowRuns.map((run) => (
        <div key={run.flow_run_id}>
          {/* Single-line run entry */}
          <button
            type="button"
            className="flex w-full items-center gap-2 px-3 py-1 text-left transition hover:bg-cronymax-hover"
            onClick={() => setExpandedId((prev) => (prev === run.flow_run_id ? null : run.flow_run_id))}
          >
            <StatusIcon status={run.status} />
            <span className="text-[11px] font-mono text-cronymax-caption">{run.flow_run_id.slice(0, 8)}</span>
            <span className="text-[11px] text-cronymax-caption">#{run.index}</span>
            <span
              className={
                "ml-auto text-[10px] " +
                (run.status === "human_review_pending"
                  ? "text-amber-400 font-semibold"
                  : run.status === "completed"
                    ? "text-green-400"
                    : run.status === "failed"
                      ? "text-red-400"
                      : "text-cronymax-caption")
              }
            >
              {statusLabel(run.status)}
            </span>
            <svg
              className={
                "h-3 w-3 shrink-0 text-cronymax-caption transition-transform " +
                (expandedId === run.flow_run_id ? "rotate-180" : "")
              }
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth={2}
            >
              <polyline points="6 9 12 15 18 9" />
            </svg>
          </button>

          {/* Expanded sub-run list */}
          {expandedId === run.flow_run_id && run.subRuns.length > 0 && (
            <div className="border-t border-cronymax-border/50 bg-cronymax-base pb-1">
              {run.subRuns.map((sub) => (
                <SubRunRow key={sub.id} run={sub} />
              ))}
            </div>
          )}
        </div>
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
