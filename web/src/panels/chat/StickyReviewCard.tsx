/**
 * StickyReviewCard — compact sticky card displayed above the prompt editor
 * when the active flow run has pending human document reviews.
 *
 * supervisor-session-ux task 10.1
 *
 * Currently a simplified version of FlowDocReviewPanel — shows a count badge
 * and a quick-navigate button into the FlowThreadView for inline review.
 */

import { ShieldAlert } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

interface Props {
  /** Number of pending doc reviews across all active flow runs. */
  pendingCount: number;
  /** Called when the user clicks to view the reviews. */
  onReview: () => void;
}

export function StickyReviewCard({ pendingCount, onReview }: Props) {
  if (pendingCount === 0) return null;

  return (
    <div className="flex items-center gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-1.5 text-sm text-amber-700 dark:text-amber-400">
      <ShieldAlert className="h-4 w-4 shrink-0" />
      <span>
        {pendingCount} pending review{pendingCount !== 1 ? "s" : ""}
      </span>
      <Badge variant="outline" className="ml-1 text-xs">
        {pendingCount}
      </Badge>
      <Button
        size="sm"
        variant="ghost"
        className="ml-auto shrink-0 text-xs text-amber-700 hover:text-amber-900 dark:text-amber-400"
        onClick={onReview}
      >
        Review →
      </Button>
    </div>
  );
}
