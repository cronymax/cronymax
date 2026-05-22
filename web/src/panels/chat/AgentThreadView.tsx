/**
 * AgentThreadView — full-screen view for a single supervisor-dispatched agent
 * sub-task.  Shows the agent's content stream, trace entries, and a critic
 * pass banner when a critic_result event arrives.
 *
 * supervisor-session-ux task 9.1
 */

import { useMemo } from "react";
import { Breadcrumb } from "./Breadcrumb";
import { CriticPassBanner } from "./CriticPassBanner";
import { type AgentThreadBlock, useStore } from "./store";

// ── Component ──────────────────────────────────────────────────────────────

interface Props {
  taskId: string;
}

export function AgentThreadView({ taskId }: Props) {
  const [state] = useStore();
  const blocks = state.blocks;

  const block = useMemo(
    () => blocks.find((b) => b.kind === "agent_thread" && b.taskId === taskId) as AgentThreadBlock | undefined,
    [blocks, taskId],
  );

  if (!block) {
    return (
      <div className="flex h-full flex-col">
        <Breadcrumb label="Agent thread" />
        <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">Thread not found.</div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <Breadcrumb label={block.agentName} />
      <div className="flex-1 overflow-y-auto px-4 py-4">
        <div className="mb-2 flex items-center gap-2">
          <span className="text-sm font-medium">{block.agentName}</span>
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
        </div>
        {block.summary && (
          <p className="mb-4 rounded-md bg-primary/10 px-3 py-2 text-sm text-foreground">{block.summary}</p>
        )}

        {/* Critic pass banners — task 9.3 + 9.4 */}
        {(block.criticResults ?? []).length > 0 && (
          <div className="mt-4 flex flex-col gap-2">
            {(block.criticResults ?? []).map((cr, i) => (
              <div key={i}>
                <CriticPassBanner passed={cr.passed} summary={cr.passed ? undefined : cr.summary} ts={cr.ts} />
                {!cr.passed && (
                  <p className="mt-1 text-xs text-muted-foreground">
                    → Revision {cr.revision}/{cr.maxRevisions} queued
                  </p>
                )}
              </div>
            ))}
          </div>
        )}

        {(block.criticResults ?? []).length === 0 && (
          <div className="text-sm text-muted-foreground italic">
            Agent run in progress. Streaming output will appear here.
          </div>
        )}
      </div>
    </div>
  );
}
