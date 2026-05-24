/**
 * FlowThreadCard — a compact inline card shown in the main chat timeline for
 * a supervisor-dispatched flow sub-task.  Clicking "→ thread" navigates into
 * the FlowThreadView for that task.
 *
 * In receipt mode (status done/failed/cancelled) the card collapses to show
 * outcome sections: PRODUCES, FILE CHANGES, APPROVALS, REVIEWS.
 *
 * supervisor-session-ux tasks 6.2, 15.1, 15.2, 15.3, 15.4, 15.6, 15.8
 */

import { CheckCircle, ChevronDown, ChevronRight, Clock, FileDiff, FileText, GitBranch, XCircle } from "lucide-react";
import React, { useCallback, useEffect, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { runtime, shells } from "@/shells/bridge";
import { ApprovalCard } from "./ApprovalCard";
import { ApprovalHistoryRow, type ResolvedReview } from "./ApprovalHistoryRow";
import { type FlowThreadBlock, type TaskStatus, useStore } from "./store";

// ── Helpers ────────────────────────────────────────────────────────────────

function isReceiptStatus(status: TaskStatus): boolean {
  return status === "succeeded" || status === "failed" || status === "cancelled";
}

function statusBadge(status: TaskStatus) {
  switch (status) {
    case "running":
      return (
        <Badge variant="secondary" className="gap-1">
          <Clock className="h-3 w-3 animate-pulse" />
          Running
        </Badge>
      );
    case "succeeded":
      return (
        <Badge variant="default" className="gap-1 bg-green-600/20 text-green-700 dark:text-green-400">
          <CheckCircle className="h-3 w-3" />
          Done
        </Badge>
      );
    case "failed":
      return (
        <Badge variant="destructive" className="gap-1">
          <XCircle className="h-3 w-3" />
          Failed
        </Badge>
      );
    case "cancelled":
      return (
        <Badge variant="outline" className="gap-1 text-muted-foreground">
          <XCircle className="h-3 w-3" />
          Cancelled
        </Badge>
      );
    case "pending":
      return (
        <Badge variant="outline" className="gap-1">
          <Clock className="h-3 w-3" />
          Pending
        </Badge>
      );
    case "awaiting_review":
      return (
        <Badge variant="outline" className="gap-1 bg-purple-500/20 text-purple-400">
          <Clock className="h-3 w-3" />
          Awaiting review
        </Badge>
      );
  }
}

// ── ProducedDoc row ────────────────────────────────────────────────────────

interface ProducedDoc {
  doc_type: string;
  path: string;
  revision: number;
}

// ── DocReviewDecision ──────────────────────────────────────────────────────

/** A resolved document review from FlowRuntime port state (task 15.8). */
export interface DocReviewDecision {
  /** Node id / port name (e.g. "pm-design/output"). */
  node_port: string;
  /** Readable document name / path. */
  doc_path: string;
  /** "approved" | "requested_changes" */
  verdict: string;
  /** Reviewer display name (agent or human). */
  reviewer: string | null;
  /** Short comment excerpt shown inline. */
  comment_excerpt: string | null;
  /** When the decision was recorded (ms epoch). */
  decided_at_ms: number;
}

// ── Sub-run (task tree) types ─────────────────────────────────────────────

interface SubRunEntry {
  id: string;
  agentId: string | null;
  goal: string | null;
  status: string;
  parentRunId: string | null;
}

interface FlowRunData {
  subRuns: SubRunEntry[];
  produces: ProducedDoc[];
  fileChanges: FileChange[];
  resolvedReviews: ResolvedReview[];
  pendingReviews: Array<{ reviewId: string; runId: string; toolName: string; args: unknown }>;
}

const EMPTY_FLOW_DATA: FlowRunData = {
  subRuns: [],
  produces: [],
  fileChanges: [],
  resolvedReviews: [],
  pendingReviews: [],
};

function parseRawStatus(raw: unknown): string {
  if (typeof raw === "string") return raw;
  if (typeof raw === "object" && raw !== null) {
    const s = (raw as Record<string, unknown>).status;
    if (typeof s === "string") return s;
  }
  return "pending";
}

/**
 * Fetches activity snapshot data for all child runs of `flowRunId`.
 * Re-fetches when status changes and polls every 5 s while the run is live.
 */
function useFlowRunData(flowRunId: string, status: TaskStatus): FlowRunData {
  const [data, setData] = useState<FlowRunData>(EMPTY_FLOW_DATA);

  const refresh = useCallback(() => {
    if (!flowRunId) return;
    shells.browser.activity
      .snapshot()
      .then((resp: { runs?: unknown[]; pending_reviews?: unknown[] }) => {
        const rawRuns = (resp.runs ?? []) as Array<Record<string, unknown>>;
        const childRuns = rawRuns.filter((r) => r.flow_run_id === flowRunId);

        const subRuns: SubRunEntry[] = childRuns.map((r) => ({
          id: (r.id as string) ?? "",
          agentId: (r.agent_id as string | null) ?? null,
          goal: (r.goal as string | null) ?? null,
          status: parseRawStatus(r.status),
          parentRunId: (r.parent_run_id as string | null) ?? null,
        }));

        const produces: ProducedDoc[] = [];
        const fileChanges: FileChange[] = [];
        const resolvedReviews: ResolvedReview[] = [];

        for (const r of childRuns) {
          if (Array.isArray(r.produces)) {
            for (const p of r.produces as Array<Record<string, unknown>>) {
              produces.push({
                doc_type: (p.doc_type as string) ?? "doc",
                path: (p.path as string) ?? "",
                revision: (p.revision as number) ?? 1,
              });
            }
          }
          if (Array.isArray(r.file_changes)) {
            for (const fc of r.file_changes as Array<Record<string, unknown>>) {
              fileChanges.push({
                path: (fc.path as string) ?? "",
                additions: (fc.additions as number) ?? 0,
                deletions: (fc.deletions as number) ?? 0,
              });
            }
          }
          if (Array.isArray(r.resolved_reviews)) {
            for (const rv of r.resolved_reviews as Array<Record<string, unknown>>) {
              const req = (rv.request as Record<string, unknown>) ?? {};
              resolvedReviews.push({
                review_id: (rv.review_id as string) ?? "",
                request: {
                  tool_name: req.tool_name as string | undefined,
                  arguments: req.arguments,
                },
                decision: (rv.decision as string) ?? "approved",
                notes: (rv.notes as string | null) ?? null,
                resolved_at_ms: (rv.resolved_at_ms as number) ?? 0,
              });
            }
          }
        }

        const childRunIds = new Set(childRuns.map((r) => r.id as string));
        const pendingReviews: FlowRunData["pendingReviews"] = [];
        for (const rv of (resp.pending_reviews ?? []) as Array<Record<string, unknown>>) {
          const runId = (rv.run_id as string) ?? "";
          if (childRunIds.has(runId)) {
            const req = (rv.request as Record<string, unknown>) ?? {};
            pendingReviews.push({
              reviewId: (rv.id as string) ?? "",
              runId,
              toolName: (req.tool_name as string) ?? "unknown_tool",
              args: req.arguments,
            });
          }
        }

        setData({ subRuns, produces, fileChanges, resolvedReviews, pendingReviews });
      })
      .catch(() => undefined);
  }, [flowRunId]);

  // Re-fetch when status changes (to populate on completion).
  useEffect(() => {
    refresh();
  }, [refresh, status]);

  // Poll while running so the task tree stays live.
  useEffect(() => {
    if (!flowRunId || isReceiptStatus(status)) return;
    const id = setInterval(refresh, 5000);
    return () => clearInterval(id);
  }, [flowRunId, status, refresh]);

  // Subscribe to the flow run's session topic for push updates.
  useEffect(() => {
    if (!flowRunId) return;
    const off = runtime.on(`run:${flowRunId}`, (raw: unknown) => {
      const ev = raw as Record<string, unknown>;
      const pl = (ev?.payload as Record<string, unknown> | undefined) ?? {};
      const kind = pl.kind as string | undefined;
      if (kind === "run_status" || kind === "trace") refresh();
    });
    return () => off?.();
  }, [flowRunId, refresh]);

  return data;
}

function ProducedDocRow({ doc, flowRunId }: { doc: ProducedDoc; flowRunId: string }) {
  const handleClick = useCallback(() => {
    const url = `web/document/workbench.html?flow=${encodeURIComponent(flowRunId)}&doc=${encodeURIComponent(doc.path)}&run_id=${encodeURIComponent(flowRunId)}`;
    window.open(url, "_blank", "popup,width=800,height=600");
  }, [doc.path, flowRunId]);

  return (
    <button
      type="button"
      className="flex w-full items-center gap-2 rounded px-1 py-0.5 text-left text-xs hover:bg-accent/50"
      onClick={handleClick}
    >
      <FileText className="h-3 w-3 shrink-0 text-blue-400" />
      <span className="truncate text-foreground/80">{doc.path}</span>
      <span className="ml-auto shrink-0 text-muted-foreground">
        {doc.doc_type}
        {doc.revision > 1 ? ` v${doc.revision}` : ""}
      </span>
    </button>
  );
}

// ── FileChange row ────────────────────────────────────────────────────────

interface FileChange {
  path: string;
  additions: number;
  deletions: number;
}

function FileChangeRow({ fc, runId }: { fc: FileChange; runId: string }) {
  const handleClick = useCallback(() => {
    const url = `web/viewer/file-view.html?run_id=${encodeURIComponent(runId)}&path=${encodeURIComponent(fc.path)}`;
    window.open(url, "_blank", "popup,width=900,height=700");
  }, [fc.path, runId]);

  return (
    <button
      type="button"
      className="flex w-full items-center gap-2 rounded px-1 py-0.5 text-left text-xs hover:bg-accent/50"
      onClick={handleClick}
    >
      <FileDiff className="h-3 w-3 shrink-0 text-muted-foreground" />
      <span className="truncate text-foreground/80">{fc.path}</span>
      <span className="ml-auto shrink-0 font-mono">
        {fc.additions > 0 && <span className="text-green-400">+{fc.additions}</span>}
        {fc.additions > 0 && fc.deletions > 0 && "/"}
        {fc.deletions > 0 && <span className="text-red-400">-{fc.deletions}</span>}
      </span>
    </button>
  );
}

// ── SubRunRow (task tree) ─────────────────────────────────────────────────

const SUB_STATUS_COLOR: Record<string, string> = {
  running: "text-amber-400",
  succeeded: "text-green-400",
  failed: "text-red-400",
  cancelled: "text-muted-foreground",
  pending: "text-muted-foreground",
  awaiting_review: "text-purple-400",
};

const SUB_STATUS_ICON: Record<string, string> = {
  running: "●",
  succeeded: "✓",
  failed: "✗",
  cancelled: "⊘",
  pending: "○",
  awaiting_review: "⏸",
};

function SubRunRow({ run, depth = 0 }: { run: SubRunEntry; depth?: number }) {
  const label = run.goal
    ? run.goal.length > 55
      ? `${run.goal.slice(0, 55)}…`
      : run.goal
    : (run.agentId ?? run.id.slice(0, 8));
  const color = SUB_STATUS_COLOR[run.status] ?? "text-muted-foreground";
  const icon = SUB_STATUS_ICON[run.status] ?? "○";
  return (
    <div className="flex items-center gap-1.5 py-0.5 text-xs" style={{ paddingLeft: `${depth * 12}px` }}>
      {depth > 0 && <span className="shrink-0 text-muted-foreground/40">└</span>}
      <span className={`shrink-0 font-mono ${color}`}>{icon}</span>
      <span className="flex-1 truncate text-foreground/80">{label}</span>
      {run.agentId && <span className="shrink-0 font-mono text-[10px] text-muted-foreground">{run.agentId}</span>}
    </div>
  );
}

// ── Receipt section ────────────────────────────────────────────────────────

interface ReceiptSectionProps {
  label: string;
  count: number;
  children: React.ReactNode;
}

function ReceiptSection({ label, count, children }: ReceiptSectionProps) {
  const [open, setOpen] = useState(false);
  if (count === 0) return null;
  return (
    <div className="mt-1.5">
      <button
        type="button"
        className="flex items-center gap-1 text-xs font-medium text-muted-foreground hover:text-foreground"
        onClick={() => setOpen((v) => !v)}
      >
        {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        {label}
        <span className="ml-1 rounded-full bg-muted px-1.5 py-0 text-[10px]">{count}</span>
      </button>
      {open && <div className="mt-0.5 pl-4">{children}</div>}
    </div>
  );
}

// ── Component ──────────────────────────────────────────────────────────────

export function FlowThreadCard({ block }: { block: FlowThreadBlock }) {
  const [, dispatch] = useStore();
  const [trajectoryExpanded, setTrajectoryExpanded] = useState(false);
  const data = useFlowRunData(block.flowRunId, block.status);

  const handleNavigate = useCallback(() => {
    dispatch({ type: "setThreadView", target: { taskId: block.taskId, kind: "flow" } });
    dispatch({ type: "setPinnedBlockId", blockId: block.id });
  }, [dispatch, block.taskId, block.id]);

  const elapsed =
    block.endedAt != null
      ? Math.round((block.endedAt - block.startedAt) / 1000)
      : Math.round((Date.now() - block.startedAt) / 1000);

  const receipt = isReceiptStatus(block.status);
  const totalAdditions = data.fileChanges.reduce((s, fc) => s + fc.additions, 0);
  const totalDeletions = data.fileChanges.reduce((s, fc) => s + fc.deletions, 0);

  // Build a simple parent→children map for the task tree.
  const rootRunIds = new Set(
    data.subRuns.filter((r) => !r.parentRunId || !data.subRuns.some((p) => p.id === r.parentRunId)).map((r) => r.id),
  );
  function renderSubRunTree(parentId: string | null, depth: number): React.ReactNode {
    return data.subRuns
      .filter((r) => (parentId === null ? rootRunIds.has(r.id) : r.parentRunId === parentId))
      .map((r) => (
        <React.Fragment key={r.id}>
          <SubRunRow run={r} depth={depth} />
          {renderSubRunTree(r.id, depth + 1)}
        </React.Fragment>
      ));
  }

  return (
    <div className="my-1 rounded-lg border border-border/60 bg-muted/30 px-3 py-2">
      {/* Header row */}
      <div className="flex items-center gap-2">
        <GitBranch className="h-4 w-4 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">{block.flowId}</span>
        {statusBadge(block.status)}
        <span className="text-xs text-muted-foreground">{elapsed}s</span>
        <Button size="sm" variant="ghost" className="shrink-0 text-xs" onClick={handleNavigate}>
          → thread
        </Button>
      </div>

      {block.description && !receipt && (
        <p className="mt-0.5 truncate text-xs text-muted-foreground">{block.description}</p>
      )}

      {/* Outcome sections — always visible while running, receipt mode when done */}
      {(receipt || data.subRuns.length > 0) && (
        <div className="mt-1">
          {/* Trajectory summary toggle (task 15.2) */}
          {receipt && (
            <>
              <button
                type="button"
                className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
                onClick={() => setTrajectoryExpanded((v) => !v)}
              >
                {trajectoryExpanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
                <span>Details</span>
              </button>
              {trajectoryExpanded && block.description && (
                <p className="mt-0.5 pl-4 text-xs text-muted-foreground">{block.description}</p>
              )}
            </>
          )}

          {/* TASKS — agent sub-run tree */}
          <ReceiptSection label="TASKS" count={data.subRuns.length}>
            {renderSubRunTree(null, 0)}
          </ReceiptSection>

          {/* PRODUCES (task 15.3) */}
          <ReceiptSection label="PRODUCES" count={data.produces.length}>
            {data.produces.map((doc, i) => (
              <ProducedDocRow key={i} doc={doc} flowRunId={block.flowRunId} />
            ))}
          </ReceiptSection>

          {/* FILE CHANGES (task 15.4) */}
          {(totalAdditions > 0 || totalDeletions > 0) && (
            <ReceiptSection
              label={`FILE CHANGES  +${totalAdditions} −${totalDeletions}`}
              count={data.fileChanges.length}
            >
              {data.fileChanges.map((fc, i) => (
                <FileChangeRow key={i} fc={fc} runId={block.parentRunId} />
              ))}
            </ReceiptSection>
          )}

          {/* APPROVALS (task 15.6) */}
          {(block.status === "awaiting_review" ||
            data.resolvedReviews.length > 0 ||
            data.pendingReviews.length > 0) && (
            <ReceiptSection label="APPROVALS" count={data.resolvedReviews.length + data.pendingReviews.length}>
              {data.pendingReviews.map((pr) => (
                <ApprovalCard
                  key={pr.reviewId}
                  runId={pr.runId}
                  reviewId={pr.reviewId}
                  toolName={pr.toolName}
                  args={pr.args}
                  onAllow={() => undefined}
                  onDeny={() => undefined}
                />
              ))}
              {data.resolvedReviews.map((rv) => (
                <ApprovalHistoryRow key={rv.review_id} review={rv} />
              ))}
            </ReceiptSection>
          )}
        </div>
      )}
    </div>
  );
}
