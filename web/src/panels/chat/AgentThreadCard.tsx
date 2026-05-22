/**
 * AgentThreadCard — a compact inline card shown in the main chat timeline for
 * a supervisor-dispatched agent sub-task.  Clicking "→ thread" navigates into
 * the AgentThreadView for that task.
 *
 * In receipt mode (succeeded/failed/cancelled) the card collapses to a
 * single-line output summary with an expand toggle.
 *
 * supervisor-session-ux tasks 6.1, 15.9
 */

import { Bot, CheckCircle, ChevronDown, ChevronRight, Clock, XCircle } from "lucide-react";
import { useCallback, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { type AgentThreadBlock, type TaskStatus, useStore } from "./store";

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

// ── Component ──────────────────────────────────────────────────────────────

interface Props {
  block: AgentThreadBlock;
}

export function AgentThreadCard({ block }: Props) {
  const [, dispatch] = useStore();
  const [expanded, setExpanded] = useState(false);

  const handleNavigate = useCallback(() => {
    dispatch({ type: "setThreadView", target: { taskId: block.taskId, kind: "agent" } });
    dispatch({ type: "setPinnedBlockId", blockId: block.id });
  }, [dispatch, block.taskId, block.id]);

  const elapsed =
    block.endedAt != null
      ? Math.round((block.endedAt - block.startedAt) / 1000)
      : Math.round((Date.now() - block.startedAt) / 1000);

  const receipt = isReceiptStatus(block.status);

  // Receipt mode: single-line collapsed by default (task 15.9)
  if (receipt) {
    return (
      <div className="my-1 rounded-lg border border-border/60 bg-muted/30 px-3 py-2">
        <div className="flex items-center gap-2">
          <Bot className="h-4 w-4 shrink-0 text-muted-foreground" />
          <button
            type="button"
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
            onClick={() => setExpanded((v) => !v)}
          >
            {expanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
          </button>
          <span className="min-w-0 flex-1 truncate text-xs text-foreground/80">
            {block.agentName}
            {block.summary ? ` — ${block.summary}` : ""}
          </span>
          {statusBadge(block.status)}
          <span className="shrink-0 text-xs text-muted-foreground">{elapsed}s</span>
          <Button size="sm" variant="ghost" className="shrink-0 text-xs" onClick={handleNavigate}>
            → thread
          </Button>
        </div>
        {expanded && block.summary && <p className="mt-1 pl-6 text-xs text-muted-foreground">{block.summary}</p>}
      </div>
    );
  }

  return (
    <div className="my-1 flex items-start gap-3 rounded-lg border border-border/60 bg-muted/30 px-3 py-2">
      <Bot className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium text-foreground">{block.agentName}</span>
          {statusBadge(block.status)}
          <span className="ml-auto text-xs text-muted-foreground">{elapsed}s</span>
        </div>
        {block.summary && <p className="mt-0.5 truncate text-xs text-muted-foreground">{block.summary}</p>}
      </div>
      <Button size="sm" variant="ghost" className="shrink-0 text-xs" onClick={handleNavigate}>
        → thread
      </Button>
    </div>
  );
}
