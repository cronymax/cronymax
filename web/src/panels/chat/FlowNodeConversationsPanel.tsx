/**
 * FlowNodeConversationsPanel — shows the per-node agent conversation stream
 * for the currently active flow run.
 *
 * Renders as a collapsible panel above the prompt editor (mounted by App.tsx
 * in the floating stack).  A horizontal tab strip lets the user pick a node;
 * the body shows ContentStreamView + TraceViewer for that node's sub-run.
 *
 * Data flow:
 *   App ─► FlowTrajectoryDiagram (useFlowNodeConversations hook)
 *        ─► onConversationsUpdate ─► App state
 *        ─► FlowNodeConversationsPanel (receives conversations as prop)
 */

import { ChevronDown, ChevronUp, MessageSquare } from "lucide-react";
import { useState } from "react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import { ContentStreamView } from "./ContentStreamView";
import type { NodeConversation, StatusKind, TraceEntry } from "./store";
import { TraceViewer } from "./TraceViewer";

// ── Status dot ────────────────────────────────────────────────────────────

function StatusDot({ status }: { status: StatusKind }) {
  if (status === "running") {
    return <span className="inline-flex h-1.5 w-1.5 rounded-full bg-primary animate-pulse shrink-0" />;
  }
  if (status === "awaiting_review") {
    return <span className="inline-flex h-1.5 w-1.5 rounded-full bg-amber-400 animate-pulse shrink-0" />;
  }
  if (status === "succeeded") {
    return (
      <svg
        className="h-2.5 w-2.5 text-green-400 shrink-0"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={2.5}
        aria-label="Succeeded"
      >
        <polyline points="20 6 9 17 4 12" />
      </svg>
    );
  }
  if (status === "failed") {
    return (
      <svg
        className="h-2.5 w-2.5 text-red-400 shrink-0"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={2.5}
        aria-label="Failed"
      >
        <line x1="18" y1="6" x2="6" y2="18" />
        <line x1="6" y1="6" x2="18" y2="18" />
      </svg>
    );
  }
  return <span className="inline-flex h-1.5 w-1.5 rounded-full bg-muted-foreground/30 shrink-0" />;
}

// ── Reviews section ────────────────────────────────────────────────────────

function ReviewsSection({ reviews }: { reviews: Extract<TraceEntry, { kind: "approval_resolved" }>[] }) {
  if (reviews.length === 0) return null;
  return (
    <div className="border-t border-border/50 px-3 py-2">
      <p className="mb-1.5 text-xs font-medium text-muted-foreground">Reviews</p>
      <div className="flex flex-col gap-1">
        {reviews.map((r) => (
          <div key={r.reviewId} className="flex items-center gap-2 text-xs">
            <span
              className={cn(
                "rounded px-1.5 py-0.5 font-mono text-xs font-medium",
                r.decision === "approve" ? "bg-green-500/15 text-green-400" : "bg-red-500/15 text-red-400",
              )}
            >
              {r.decision === "approve" ? "approved" : "denied"}
            </span>
            <span className="font-mono text-muted-foreground/70">{r.reviewId.slice(0, 8)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ── NodeConversationView ───────────────────────────────────────────────────

function NodeConversationView({ conv }: { conv: NodeConversation }) {
  const isStreaming = conv.status === "running";
  const reviews = conv.traceEntries.filter(
    (e): e is Extract<TraceEntry, { kind: "approval_resolved" }> => e.kind === "approval_resolved",
  );

  return (
    <div className="flex flex-col overflow-hidden">
      {/* Main conversation stream */}
      <div className="max-h-[280px] overflow-y-auto px-3 py-2 text-sm">
        <ContentStreamView segments={conv.contentStream} isStreaming={isStreaming} />
      </div>

      {/* Trace entries */}
      {conv.traceEntries.length > 0 && (
        <div className="border-t border-border/50 px-3 py-1.5">
          <TraceViewer entries={conv.traceEntries} startExpanded={isStreaming} />
        </div>
      )}

      {/* Reviews */}
      <ReviewsSection reviews={reviews} />
    </div>
  );
}

// ── Main panel ────────────────────────────────────────────────────────────

interface Props {
  /** Ordered node names — determines tab order. Derived from topology. */
  nodeOrder: string[];
  conversations: Map<string, NodeConversation>;
  selectedNodeId: string | null;
  onSelectNode: (agentName: string | null) => void;
}

export function FlowNodeConversationsPanel({ nodeOrder, conversations, selectedNodeId, onSelectNode }: Props) {
  const [collapsed, setCollapsed] = useState(false);

  // Only show nodes that have a conversation entry.
  const visibleNodes = nodeOrder.filter((n) => conversations.has(n));
  if (visibleNodes.length === 0) return null;

  const activeCount = visibleNodes.filter((n) => {
    const s = conversations.get(n)?.status;
    return s === "running" || s === "awaiting_review";
  }).length;

  // Determine the active tab (auto-select first running/active node if none chosen).
  const effectiveSelected =
    selectedNodeId && conversations.has(selectedNodeId)
      ? selectedNodeId
      : (visibleNodes.find((n) => {
          const s = conversations.get(n)?.status;
          return s === "running" || s === "awaiting_review";
        }) ??
        visibleNodes[0] ??
        null);

  const selectedConv = effectiveSelected ? conversations.get(effectiveSelected) : null;

  return (
    <Collapsible
      open={!collapsed}
      onOpenChange={() => setCollapsed((c) => !c)}
      className="mx-0 rounded-lg border border-border/50 bg-card/95 shadow-sm backdrop-blur-sm overflow-hidden text-xs transition-all"
    >
      {/* Header */}
      <CollapsibleTrigger className="flex w-full items-center gap-2 px-2.5 py-1.5 transition-colors hover:bg-accent/50">
        <MessageSquare className="h-3 w-3 shrink-0 text-muted-foreground" />
        <span className="font-medium text-muted-foreground">Node conversations</span>
        {activeCount > 0 && (
          <span className="rounded-full bg-primary/20 px-1.5 py-0.5 text-xs font-medium text-primary">
            {activeCount} active
          </span>
        )}
        <span className="flex-1" />
        {collapsed ? <ChevronDown className="h-3 w-3" /> : <ChevronUp className="h-3 w-3" />}
      </CollapsibleTrigger>

      <CollapsibleContent className="border-t border-border/50 bg-background">
        {/* Tab strip */}
        <div className="flex items-center gap-1 overflow-x-auto border-b border-border/50 px-2 py-1.5 min-w-max">
          {visibleNodes.map((agentName) => {
            const conv = conversations.get(agentName)!;
            const isSelected = agentName === effectiveSelected;
            return (
              <button
                key={agentName}
                type="button"
                onClick={() => onSelectNode(isSelected ? null : agentName)}
                className={cn(
                  "flex items-center gap-1.5 rounded px-2 py-1 font-medium transition-colors",
                  isSelected ? "bg-primary/15 text-foreground" : "text-muted-foreground hover:bg-muted/60",
                )}
              >
                <StatusDot status={conv.status} />
                <span className="max-w-[80px] truncate">{agentName}</span>
              </button>
            );
          })}
        </div>

        {/* Body */}
        {selectedConv && <NodeConversationView conv={selectedConv} />}
      </CollapsibleContent>
    </Collapsible>
  );
}
