import { ExternalLink } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ApprovalCard } from "@/panels/chat/ApprovalCard";
import { shells } from "@/shells/bridge";
import type { ReviewEntry, RunEntry } from "./useActivityFeed";

interface Props {
  run: RunEntry;
  review?: ReviewEntry;
  onReviewResolved: () => void;
  depth?: number;
}

const STATUS_BADGE: Record<string, string> = {
  running: "bg-amber-500/20 text-amber-300",
  pending: "bg-muted/50 text-muted-foreground",
  succeeded: "bg-green-500/20 text-green-400",
  failed: "bg-red-500/20 text-red-400",
  cancelled: "bg-red-500/10 text-red-500/70",
  awaiting_review: "bg-purple-500/20 text-purple-300",
  paused: "bg-blue-500/20 text-blue-300",
};

const STATUS_LABEL: Record<string, string> = {
  running: "running",
  pending: "pending",
  succeeded: "done",
  failed: "failed",
  cancelled: "cancelled",
  awaiting_review: "awaiting review",
  paused: "paused",
};

function formatTokens(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

function formatMs(ms: number): string {
  if (ms >= 60000) return `${Math.round(ms / 60000)}m`;
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`;
  return `${ms}ms`;
}

export function RunRow({ run, review, onReviewResolved, depth = 0 }: Props) {
  const shortId = run.id.slice(0, 8);
  const badge = STATUS_BADGE[run.status] ?? "bg-muted/50 text-muted-foreground";
  const label = STATUS_LABEL[run.status] ?? run.status;
  const totalTokens = run.input_tokens + run.output_tokens;
  const indentPx = depth * 16;

  // Outcome-first label: goal or agent_id fallback (task 16.3)
  const primaryLabel = run.goal
    ? run.goal.length > 60
      ? run.goal.slice(0, 60) + "…"
      : run.goal
    : (run.agent_id ?? shortId);
  const tooltipParts: string[] = [];
  if (run.agent_id) tooltipParts.push(`agent: ${run.agent_id}`);
  tooltipParts.push(`id: ${shortId}`);
  if (run.turn_count > 0) tooltipParts.push(`${run.turn_count} turn${run.turn_count !== 1 ? "s" : ""}`);
  if (totalTokens > 0) tooltipParts.push(`${formatTokens(totalTokens)} tok`);
  if (run.total_duration_ms > 0) tooltipParts.push(formatMs(run.total_duration_ms));

  return (
    <div className="mb-1" style={{ paddingLeft: `${indentPx}px` }}>
      <div
        className="flex items-center gap-2 px-2 py-1 rounded hover:bg-accent/50 text-xs"
        title={tooltipParts.join(" · ")}
      >
        {depth > 0 && <span className="text-muted-foreground/40 shrink-0">└</span>}
        <span className="flex-1 truncate text-foreground/90">{primaryLabel}</span>
        <span className={`rounded px-1.5 py-0.5 text-xs font-medium shrink-0 ${badge}`}>{label}</span>
        {run.produces_count > 0 && (
          <span className="rounded px-1.5 py-0.5 bg-blue-500/15 text-blue-300 shrink-0">
            {run.produces_count} doc{run.produces_count !== 1 ? "s" : ""}
          </span>
        )}
        {(run.file_change_additions > 0 || run.file_change_deletions > 0) && (
          <span className="font-mono shrink-0 text-muted-foreground">
            {run.file_change_additions > 0 && <span className="text-green-400">+{run.file_change_additions}</span>}
            {run.file_change_additions > 0 && run.file_change_deletions > 0 && "/"}
            {run.file_change_deletions > 0 && <span className="text-red-400">-{run.file_change_deletions}</span>}
          </span>
        )}
      </div>

      {run.status === "awaiting_review" && review && review.state === "pending" && (
        <div className="ml-3 mb-1">
          {run.session_id && (
            <div className="mb-1 flex justify-end">
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="h-6 gap-1 px-2 text-xs text-muted-foreground hover:text-foreground"
                onClick={() => {
                  if (run.session_id) {
                    shells.browser.shell.tab_switch({ id: run.session_id }).catch(() => undefined);
                  }
                }}
              >
                <ExternalLink className="h-3 w-3" />
                View ↗
              </Button>
            </div>
          )}
          <ApprovalCard
            runId={run.id}
            reviewId={review.id}
            toolName={review.request.tool_name ?? "unknown_tool"}
            args={review.request.arguments}
            onAllow={onReviewResolved}
            onDeny={onReviewResolved}
          />
        </div>
      )}
    </div>
  );
}
