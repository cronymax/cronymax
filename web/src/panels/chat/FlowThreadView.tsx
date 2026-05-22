/**
 * FlowThreadView — full-screen view for a supervisor-dispatched flow sub-task.
 * Shows:
 *  - Breadcrumb navigation back to the main timeline
 *  - BlackboardPanel with current blackboard entries and inject UI
 *  - FlowNodeConversationsPanel for per-node agent conversations
 *  - ExecutionCanvas placeholder (for flow graph visualization)
 *
 * supervisor-session-ux tasks 8.1, 8.4, 8.5
 */

import { useMemo, useState } from "react";
import { BlackboardPanel } from "./BlackboardPanel";
import { Breadcrumb } from "./Breadcrumb";
import { ExecutionCanvas } from "./ExecutionCanvas";
import { FlowDocReviewPanel } from "./FlowDocReviewPanel";
import { type FlowThreadBlock, useStore } from "./store";

// ── Component ──────────────────────────────────────────────────────────────

interface Props {
  taskId: string;
}

export function FlowThreadView({ taskId }: Props) {
  const [state, dispatch] = useStore();
  const blocks = state.blocks;
  const [showCanvas, setShowCanvas] = useState(false);

  const block = useMemo(
    () => blocks.find((b) => b.kind === "flow_thread" && b.taskId === taskId) as FlowThreadBlock | undefined,
    [blocks, taskId],
  );

  if (!block) {
    return (
      <div className="flex h-full flex-col">
        <Breadcrumb label="Flow thread" />
        <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">Thread not found.</div>
      </div>
    );
  }

  // Find the parent ConversationBlock for dispatching blackboard injection records (task 8.7).
  const parentConvBlockId = useMemo(
    () =>
      blocks.find((b) => b.kind === "conversation" && b.flowThread?.childSessionId === block.childSessionId)?.id ??
      null,
    [blocks, block.childSessionId],
  );

  function handleBlackboardInjected(key: string) {
    if (!parentConvBlockId) return;
    dispatch({ type: "recordBlackboardInjection", blockId: parentConvBlockId, key });
  }

  const label = block.flowId || "Flow thread";

  return (
    <div className="flex h-full flex-col">
      <Breadcrumb label={label} />
      <div className="flex flex-1 flex-col gap-4 overflow-y-auto px-4 py-4">
        {/* Status badge + canvas toggle */}
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium">{label}</span>
          <span
            className={`rounded-full px-2 py-0.5 text-xs font-medium ${
              block.status === "running"
                ? "bg-amber-500/20 text-amber-600"
                : block.status === "succeeded"
                  ? "bg-green-500/20 text-green-600"
                  : block.status === "failed"
                    ? "bg-red-500/20 text-red-600"
                    : "bg-muted text-muted-foreground"
            }`}
          >
            {block.status}
          </span>
          {block.flowId && (
            <button
              type="button"
              onClick={() => setShowCanvas((v) => !v)}
              className="ml-auto rounded px-2 py-0.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
            >
              {showCanvas ? "← thread" : "→ canvas"}
            </button>
          )}
        </div>

        {/* Execution canvas — toggle with the button above */}
        {showCanvas && block.flowId && (
          <div className="h-80 overflow-hidden rounded-md border">
            <ExecutionCanvas flowId={block.flowId} childSessionId={block.childSessionId} />
          </div>
        )}

        {block.description && (
          <p className="rounded-md bg-muted/40 px-3 py-2 text-sm text-muted-foreground">{block.description}</p>
        )}

        {/* Blackboard — human-injectable key-value store */}
        {block.flowRunId && <BlackboardPanel flowRunId={block.flowRunId} onInjected={handleBlackboardInjected} />}

        {/* Pending doc reviews for this flow run */}
        {block.flowRunId && <FlowDocReviewPanel sessionId={block.childSessionId} showHistory={false} />}
      </div>
    </div>
  );
}
