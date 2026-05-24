import React from "react";
import { RunRow } from "./RunRow";
import type { ActivityGroups, ReviewEntry, RunTreeNode } from "./useActivityFeed";

interface Props {
  groups: ActivityGroups;
  reviews: Map<string, ReviewEntry>;
  onReviewResolved: () => void;
}

function renderNode(
  node: RunTreeNode,
  reviews: Map<string, ReviewEntry>,
  onReviewResolved: () => void,
  depth = 0,
): React.ReactNode {
  const reviewId = node.run.pending_review_id;
  const review = reviewId ? reviews.get(reviewId) : undefined;
  return (
    <React.Fragment key={node.run.id}>
      <RunRow run={node.run} review={review} onReviewResolved={onReviewResolved} depth={depth} />
      {node.children.map((child) => renderNode(child, reviews, onReviewResolved, depth + 1))}
    </React.Fragment>
  );
}

export function ActivityTree({ groups, reviews, onReviewResolved }: Props) {
  const { chatRoots, flowRoots } = groups;

  if (chatRoots.length === 0 && flowRoots.size === 0) {
    return <div className="flex flex-1 items-center justify-center text-xs text-muted-foreground">No runs yet</div>;
  }

  return (
    <div className="flex-1 overflow-y-auto px-2 py-2 text-xs">
      {/* ── Chat section ──────────────────────────────────────────────── */}
      {chatRoots.length > 0 && (
        <div className="mb-3">
          <div className="px-2 py-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">💬 Chat</div>
          {chatRoots.map((node) => renderNode(node, reviews, onReviewResolved, 0))}
        </div>
      )}

      {/* ── Flows section ─────────────────────────────────────────────── */}
      {flowRoots.size > 0 && (
        <div>
          <div className="px-2 py-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">🔀 Flows</div>
          {[...flowRoots.entries()].map(([flowRunId, roots]) => (
            <div key={flowRunId} className="mb-2">
              <div className="truncate px-2 py-0.5 font-medium text-foreground">
                Flow run <span className="font-mono">{flowRunId.slice(0, 8)}</span>
                <span className="ml-1 font-normal text-muted-foreground">
                  · {roots.length} root{roots.length !== 1 ? "s" : ""}
                </span>
              </div>
              {roots.map((node) => renderNode(node, reviews, onReviewResolved, 0))}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
