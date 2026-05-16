import { RunRow } from "./RunRow";
import type { ActivityGroups, ReviewEntry } from "./useActivityFeed";

interface Props {
  groups: ActivityGroups;
  reviews: Map<string, ReviewEntry>;
  onReviewResolved: () => void;
}

export function ActivityTree({ groups, reviews, onReviewResolved }: Props) {
  const { chatGroups, flowGroups } = groups;

  if (chatGroups.size === 0 && flowGroups.size === 0) {
    return <div className="flex flex-1 items-center justify-center text-xs text-muted-foreground">No runs yet</div>;
  }

  return (
    <div className="flex-1 overflow-y-auto px-2 py-2 text-xs">
      {/* ── Chat section ──────────────────────────────────────────────── */}
      {chatGroups.size > 0 && (
        <div className="mb-3">
          <div className="px-2 py-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">💬 Chat</div>
          {[...chatGroups.entries()].map(([sessionId, runs]) => (
            <div key={sessionId} className="mb-2">
              <div className="truncate px-2 py-0.5 font-medium text-foreground">
                {sessionId.slice(0, 12)}
                <span className="ml-1 font-normal text-muted-foreground">
                  · {runs.length} run{runs.length !== 1 ? "s" : ""}
                </span>
              </div>
              {runs.map((run) => {
                const reviewId = run.pending_review_id;
                const review = reviewId ? reviews.get(reviewId) : undefined;
                return <RunRow key={run.id} run={run} review={review} onReviewResolved={onReviewResolved} />;
              })}
            </div>
          ))}
        </div>
      )}

      {/* ── Flows section ─────────────────────────────────────────────── */}
      {flowGroups.size > 0 && (
        <div>
          <div className="px-2 py-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">🔀 Flows</div>
          {[...flowGroups.entries()].map(([flowRunId, runs]) => (
            <div key={flowRunId} className="mb-2">
              <div className="truncate px-2 py-0.5 font-medium text-foreground">
                Flow run <span className="font-mono">{flowRunId.slice(0, 8)}</span>
                <span className="ml-1 font-normal text-muted-foreground">
                  · {runs.length} agent{runs.length !== 1 ? "s" : ""}
                </span>
              </div>
              {runs.map((run) => {
                const reviewId = run.pending_review_id;
                const review = reviewId ? reviews.get(reviewId) : undefined;
                return <RunRow key={run.id} run={run} review={review} onReviewResolved={onReviewResolved} />;
              })}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
