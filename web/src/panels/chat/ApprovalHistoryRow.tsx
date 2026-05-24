/**
 * ApprovalHistoryRow — compact read-only row for a historically resolved
 * permission review.  Renders the decision icon, tool name, argument preview,
 * and a relative timestamp.
 *
 * supervisor-session-ux task 15.7
 */

import { CheckCircle2, XCircle } from "lucide-react";
import { useMemo } from "react";

// ── Types ──────────────────────────────────────────────────────────────────

/** Mirrors the Rust ResolvedReview struct (from RunEntry snapshot). */
export interface ResolvedReview {
  review_id: string;
  request: {
    tool_name?: string;
    arguments?: unknown;
  };
  /** "approved" | "rejected" | "deferred" */
  decision: string;
  notes: string | null;
  resolved_at_ms: number;
}

// ── Helpers ────────────────────────────────────────────────────────────────

function relativeTime(ms: number): string {
  const diffSec = Math.round((Date.now() - ms) / 1000);
  if (diffSec < 60) return `${diffSec}s ago`;
  const diffMin = Math.round(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.round(diffMin / 60);
  return `${diffHr}h ago`;
}

function truncateArgs(args: unknown, maxLen = 80): string {
  try {
    const s = JSON.stringify(args);
    if (!s || s === "{}") return "";
    if (s.length <= maxLen) return s;
    return `${s.slice(0, maxLen)}…`;
  } catch {
    return "";
  }
}

// ── Component ──────────────────────────────────────────────────────────────

interface Props {
  review: ResolvedReview;
}

export function ApprovalHistoryRow({ review }: Props) {
  const isApproved = review.decision === "approved";
  const toolName = review.request.tool_name ?? "unknown_tool";
  const argsPreview = useMemo(() => truncateArgs(review.request.arguments), [review.request.arguments]);
  const time = useMemo(() => relativeTime(review.resolved_at_ms), [review.resolved_at_ms]);

  return (
    <div className="flex items-center gap-2 py-0.5 text-xs text-muted-foreground">
      {isApproved ? (
        <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-green-500" />
      ) : (
        <XCircle className="h-3.5 w-3.5 shrink-0 text-red-500" />
      )}
      <span className="font-mono text-foreground/80">{toolName}</span>
      {argsPreview && <span className="truncate opacity-60">{argsPreview}</span>}
      <span className="ml-auto shrink-0 tabular-nums">{time}</span>
    </div>
  );
}
