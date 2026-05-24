/**
 * StickyActiveThread — a sticky banner shown above the prompt editor when
 * there is a pinned thread block (agent or flow).  Provides a quick "→ thread"
 * button to navigate into the thread view without scrolling up in the timeline.
 *
 * supervisor-session-ux task 6.5
 */

import { Bot, GitBranch, X } from "lucide-react";
import { useCallback } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { type AgentThreadBlock, type Block, type FlowThreadBlock, useStore } from "./store";

// ── Component ──────────────────────────────────────────────────────────────

export function StickyActiveThread() {
  const [state, dispatch] = useStore();
  const pinnedBlockId = state.pinnedBlockId;
  const blocks = state.blocks;

  const block: Block | undefined = pinnedBlockId ? blocks.find((b) => b.id === pinnedBlockId) : undefined;

  const handleNavigate = useCallback(() => {
    if (!block) return;
    if (block.kind === "agent_thread") {
      dispatch({ type: "setThreadView", target: { taskId: block.taskId, kind: "agent" } });
    } else if (block.kind === "flow_thread") {
      dispatch({ type: "setThreadView", target: { taskId: block.taskId, kind: "flow" } });
    }
  }, [dispatch, block]);

  const handleDismiss = useCallback(() => {
    dispatch({ type: "setPinnedBlockId", blockId: null });
  }, [dispatch]);

  if (!block || (block.kind !== "agent_thread" && block.kind !== "flow_thread")) return null;

  const agentBlock = block.kind === "agent_thread" ? (block as AgentThreadBlock) : null;
  const flowBlock = block.kind === "flow_thread" ? (block as FlowThreadBlock) : null;

  const isRunning = block.status === "running";

  return (
    <div className="flex items-center gap-2 rounded-md border border-border/60 bg-muted/40 px-3 py-1.5 text-sm">
      {agentBlock ? (
        <Bot className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      ) : (
        <GitBranch className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      )}
      <span className="truncate text-muted-foreground">{agentBlock ? agentBlock.agentName : flowBlock?.flowId}</span>
      {isRunning && (
        <Badge variant="secondary" className="shrink-0 gap-1 text-xs">
          <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-current" />
          running
        </Badge>
      )}
      <Button size="sm" variant="ghost" className="ml-auto shrink-0 px-2 text-xs" onClick={handleNavigate}>
        → thread
      </Button>
      <Button
        size="icon"
        variant="ghost"
        className="h-5 w-5 shrink-0"
        onClick={handleDismiss}
        aria-label="Dismiss sticky thread"
      >
        <X className="h-3 w-3" />
      </Button>
    </div>
  );
}
