import { ApprovalCard } from "@/panels/chat/ApprovalCard";
import type { ReviewEntry, RunEntry } from "./useActivityFeed";

interface Props {
  run: RunEntry;
  review?: ReviewEntry;
  onReviewResolved: () => void;
}

const STATUS_BADGE: Record<string, string> = {
  running: "bg-amber-500/20 text-amber-300",
  pending: "bg-cronymax-border/30 text-cronymax-caption",
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

export function RunRow({ run, review, onReviewResolved }: Props) {
  const shortId = run.id.slice(0, 8);
  const badge = STATUS_BADGE[run.status] ?? "bg-cronymax-border/30 text-cronymax-caption";
  const label = STATUS_LABEL[run.status] ?? run.status;
  const totalTokens = run.input_tokens + run.output_tokens;

  return (
    <div className="ml-4 mb-1">
      <div className="flex items-center gap-2 px-2 py-1 rounded hover:bg-cronymax-hover text-xs">
        <span className="font-mono text-cronymax-caption">{shortId}</span>
        <span className={`rounded px-1.5 py-0.5 text-[10px] font-medium ${badge}`}>{label}</span>
        {run.turn_count > 0 && (
          <span className="text-cronymax-caption">
            {run.turn_count} turn{run.turn_count !== 1 ? "s" : ""}
          </span>
        )}
        {totalTokens > 0 && <span className="text-cronymax-caption">{formatTokens(totalTokens)} tok</span>}
        {run.total_duration_ms > 0 && <span className="text-cronymax-caption">{formatMs(run.total_duration_ms)}</span>}
      </div>

      {run.status === "awaiting_review" && review && review.state === "pending" && (
        <ApprovalCard
          runId={run.id}
          reviewId={review.id}
          toolName={review.request.tool_name ?? "unknown_tool"}
          args={review.request.arguments}
          onAllow={onReviewResolved}
          onDeny={onReviewResolved}
        />
      )}
    </div>
  );
}
