/**
 * FlowDocReviewPanel — floating panel surfaced above the prompt editor that
 * shows all pending human document reviews from active flow runs for the
 * current chat session.
 *
 * Data source: queries `flowRun.getPendingReviews()` for each known flow run
 * and refreshes whenever `flow.run.changed` or `run_status` events arrive.
 *
 * Each pending review shows:
 *  - The node/port identifier
 *  - The document markdown content (collapsible, with text selection → comments)
 *  - Approve button
 *  - Request Changes button (expands a comment panel with optional inline selections)
 *
 * Returns null when there are no pending reviews.
 */

import { Check, ChevronDown, Clock, MessageSquare, Plus, Send, ShieldAlert, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Textarea } from "@/components/ui/textarea";
import { runtime } from "@/shells/bridge";
import {
  type FlowDocReview,
  type FlowReviewComment,
  flowRun,
  type SessionPendingActionsResponse,
} from "@/shells/runtime";

// ── Types ─────────────────────────────────────────────────────────────────

type ReviewItem = FlowDocReview;

/** A comment entry attached to an optional text selection. */
interface SelectionComment {
  id: string;
  /** The selected text snippet (empty = general comment) */
  selectedText: string;
  /** Character offsets within the document body [start, end] */
  range: [number, number] | null;
  comment: string;
}

type ReviewVerdict = "approved" | "changes_requested";

interface ResolvedReview {
  item: ReviewItem;
  verdict: ReviewVerdict;
  resolvedAt: number;
}

interface ReviewCardProps {
  item: ReviewItem;
  onApproved: (item: ReviewItem) => void;
  onChangesRequested: (item: ReviewItem) => void;
}

// ── ReviewCard ─────────────────────────────────────────────────────────────

function ReviewCard({ item, onApproved, onChangesRequested }: ReviewCardProps) {
  const [expanded, setExpanded] = useState(false);
  const [showRequestPanel, setShowRequestPanel] = useState(false);
  const [comments, setComments] = useState<SelectionComment[]>([]);
  const [generalComment, setGeneralComment] = useState("");
  const [busy, setBusy] = useState(false);
  const [pendingSelection, setPendingSelection] = useState<{ text: string; range: [number, number] } | null>(null);
  const preRef = useRef<HTMLPreElement>(null);
  const selectionTooltipRef = useRef<HTMLDivElement>(null);

  const docName = item.port || item.node_id;
  const producerLabel = item.node_id || "agent";

  async function handleApprove() {
    if (busy) return;
    setBusy(true);
    try {
      await flowRun.approve(item.flow_run_id, item.node_id, item.port);
      onApproved(item);
    } catch {
      setBusy(false);
    }
  }

  async function handleRequestChanges() {
    if (busy) return;
    // Collect all comments: selection-anchored + general
    const allComments: FlowReviewComment[] = [
      ...comments.map((c) => ({
        message: c.selectedText
          ? `[Selected: "${c.selectedText.slice(0, 80)}${c.selectedText.length > 80 ? "…" : ""}"]\n${c.comment}`
          : c.comment,
      })),
    ];
    if (generalComment.trim()) {
      allComments.push({ message: generalComment.trim() });
    }
    setBusy(true);
    try {
      await flowRun.requestChanges(item.flow_run_id, item.node_id, item.port, allComments);
      onChangesRequested(item);
    } catch {
      setBusy(false);
    }
  }

  // ── Text selection handling ──────────────────────────────────────────────

  function handleMouseUp() {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || !preRef.current) return;
    const selStr = sel.toString().trim();
    if (!selStr) return;

    // Compute character offsets relative to the pre element's text content
    const range = sel.getRangeAt(0);
    const preText = preRef.current.textContent ?? "";

    // Walk DOM to find the start/end offsets within preText
    let startOffset = 0;
    let endOffset = 0;
    let found = false;

    function walkNode(node: Node, offset: number): number {
      if (node.nodeType === Node.TEXT_NODE) {
        const len = (node.textContent ?? "").length;
        if (!found && node === range.startContainer) {
          startOffset = offset + range.startOffset;
        }
        if (node === range.endContainer) {
          endOffset = offset + range.endOffset;
          found = true;
        }
        return offset + len;
      }
      let cur = offset;
      for (const child of Array.from(node.childNodes)) {
        cur = walkNode(child, cur);
      }
      return cur;
    }
    walkNode(preRef.current, 0);

    if (startOffset >= 0 && endOffset > startOffset && endOffset <= preText.length) {
      setPendingSelection({ text: selStr, range: [startOffset, endOffset] });
    }
  }

  function addSelectionComment() {
    if (!pendingSelection) return;
    setComments((prev) => [
      ...prev,
      {
        id: crypto.randomUUID(),
        selectedText: pendingSelection.text,
        range: pendingSelection.range,
        comment: "",
      },
    ]);
    setPendingSelection(null);
    setShowRequestPanel(true);
    // Clear browser selection
    window.getSelection()?.removeAllRanges();
  }

  function updateComment(id: string, text: string) {
    setComments((prev) => prev.map((c) => (c.id === id ? { ...c, comment: text } : c)));
  }

  function removeComment(id: string) {
    setComments((prev) => prev.filter((c) => c.id !== id));
  }

  // Dismiss pending selection if user clicks elsewhere
  useEffect(() => {
    if (!pendingSelection) return;
    function onMouseDown(e: MouseEvent) {
      if (selectionTooltipRef.current && selectionTooltipRef.current.contains(e.target as Node)) {
        return;
      }
      setPendingSelection(null);
    }
    document.addEventListener("mousedown", onMouseDown);
    return () => document.removeEventListener("mousedown", onMouseDown);
  }, [pendingSelection]);

  // ── Highlighted ranges ───────────────────────────────────────────────────

  /** Render document content with commented ranges highlighted. */
  function renderContent(text: string) {
    // Gather all selection ranges
    const ranges: Array<{ start: number; end: number }> = comments
      .filter((c) => c.range !== null)
      .map((c) => ({ start: c.range![0], end: c.range![1] }))
      .sort((a, b) => a.start - b.start);

    if (ranges.length === 0) return text;

    const parts: React.ReactNode[] = [];
    let cursor = 0;
    for (const r of ranges) {
      if (r.start > cursor) parts.push(text.slice(cursor, r.start));
      parts.push(
        <mark key={`${r.start}-${r.end}`} className="bg-amber-400/25 text-inherit rounded-sm">
          {text.slice(r.start, r.end)}
        </mark>,
      );
      cursor = r.end;
    }
    if (cursor < text.length) parts.push(text.slice(cursor));
    return parts;
  }

  const hasCommentContent = comments.length > 0 || generalComment.trim().length > 0;

  return (
    <Collapsible
      open={expanded}
      onOpenChange={setExpanded}
      className="rounded-lg border border-border bg-card shadow-sm overflow-hidden"
    >
      {/* ── Header ─────────────────────────────────────────────────────── */}
      <CollapsibleTrigger className="flex w-full items-center gap-2 px-3 py-1.5 text-left transition-colors hover:bg-accent/50">
        <ShieldAlert className="h-4 w-4 shrink-0 text-amber-400 animate-pulse" />
        <div className="flex-1 min-w-0">
          <p className="text-sm font-semibold text-foreground leading-tight truncate">
            Review: <span className="text-amber-500">{docName}</span>
          </p>
          <p className="text-xs text-muted-foreground leading-tight mt-0.5">
            from <span className="font-medium">{producerLabel}</span>
            <Badge variant="outline" className="ml-1.5 text-xs px-1.5 py-0 h-4 font-normal opacity-60">
              #{item.flow_run_id.slice(0, 8)}
            </Badge>
          </p>
        </div>
        {item.content && <ChevronDown className="h-4 w-4 transition-transform duration-200" />}
      </CollapsibleTrigger>

      {/* ── Document content ────────────────────────────────────────────── */}
      {item.content && (
        <CollapsibleContent className="border-t border-border/50 bg-background pb-1">
          <div className="relative">
            {pendingSelection && (
              <div ref={selectionTooltipRef} className="absolute z-20 top-2 right-2">
                <Button
                  size="sm"
                  variant="secondary"
                  className="h-7 gap-1.5 px-2.5 text-xs shadow-md"
                  onClick={addSelectionComment}
                >
                  <MessageSquare className="h-3 w-3" />
                  Comment on selection
                </Button>
              </div>
            )}
            <div className="max-h-72 overflow-y-auto">
              <pre
                ref={preRef}
                className="whitespace-pre-wrap break-words text-sm text-foreground/90 font-mono leading-relaxed px-4 py-3 select-text cursor-text"
                onMouseUp={handleMouseUp}
              >
                {renderContent(item.content)}
              </pre>
            </div>
            {comments.length > 0 && (
              <p className="px-4 pb-2 text-xs text-muted-foreground">
                {comments.length} selection comment{comments.length > 1 ? "s" : ""} added
              </p>
            )}
          </div>
        </CollapsibleContent>
      )}

      {/* ── Request-changes panel ────────────────────────────────────────── */}
      {showRequestPanel && (
        <div className="border-t border-border bg-muted/20 px-3.5 py-3 space-y-2.5">
          {comments.map((c) => (
            <div key={c.id} className="rounded-md border border-border bg-background p-2.5 space-y-1.5">
              {c.selectedText && (
                <div className="flex items-start gap-1.5">
                  <div className="mt-0.5 h-3 w-0.5 shrink-0 rounded-full bg-amber-400" />
                  <p className="text-xs text-muted-foreground italic line-clamp-2">
                    &ldquo;{c.selectedText.slice(0, 120)}
                    {c.selectedText.length > 120 ? "…" : ""}&rdquo;
                  </p>
                  <Button
                    type="button"
                    size="icon"
                    variant="ghost"
                    className="ml-auto h-5 w-5 shrink-0 text-muted-foreground"
                    onClick={() => removeComment(c.id)}
                    aria-label="Remove comment"
                  >
                    <X className="h-3 w-3" />
                  </Button>
                </div>
              )}
              <Textarea
                className="text-sm resize-none min-h-[52px] bg-transparent border-0 border-b border-border rounded-none px-0 focus-visible:ring-0 placeholder:text-muted-foreground/60"
                placeholder={c.selectedText ? "Comment on this selection…" : "Add a comment…"}
                value={c.comment}
                onChange={(e) => updateComment(c.id, e.target.value)}
                autoFocus
              />
            </div>
          ))}

          <Textarea
            className="text-sm resize-none min-h-[64px]"
            placeholder="Overall feedback (optional)…"
            value={generalComment}
            onChange={(e) => setGeneralComment(e.target.value)}
          />

          {!expanded && (
            <CollapsibleTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-6 gap-1.5 px-0 text-xs text-muted-foreground hover:text-foreground"
              >
                <Plus className="h-3 w-3" />
                Expand document to select and comment on text
              </Button>
            </CollapsibleTrigger>
          )}
        </div>
      )}

      {/* ── Actions ─────────────────────────────────────────────────────── */}
      <div className="flex items-center gap-2 border-t border-border px-3.5 py-2 bg-muted/20">
        <Button
          size="sm"
          className="h-7 gap-1.5 px-3 text-xs bg-emerald-600 hover:bg-emerald-500 text-white border-0 font-medium"
          onClick={handleApprove}
          disabled={busy}
        >
          <Check className="h-3.5 w-3.5" />
          Approve
        </Button>

        {showRequestPanel ? (
          <>
            <Button
              size="sm"
              variant="outline"
              className="h-7 gap-1.5 px-3 text-xs font-medium"
              onClick={handleRequestChanges}
              disabled={busy || !hasCommentContent}
            >
              <Send className="h-3 w-3" />
              Submit feedback
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="h-7 px-2 text-xs text-muted-foreground hover:text-foreground ml-auto"
              onClick={() => {
                setShowRequestPanel(false);
                setComments([]);
                setGeneralComment("");
              }}
              disabled={busy}
            >
              Cancel
            </Button>
          </>
        ) : (
          <Button
            size="sm"
            variant="outline"
            className="h-7 gap-1.5 px-3 text-xs font-medium"
            onClick={() => setShowRequestPanel(true)}
            disabled={busy}
          >
            <MessageSquare className="h-3.5 w-3.5" />
            Request Changes
          </Button>
        )}
      </div>
    </Collapsible>
  );
}

// ── Main Component ─────────────────────────────────────────────────────────

interface Props {
  sessionId: string | null | undefined;
}

export function FlowDocReviewPanel({ sessionId }: Props) {
  // Set of flow_run_ids we know about for this session
  const flowRunIdsRef = useRef<Set<string>>(new Set());
  const [reviews, setReviews] = useState<ReviewItem[]>([]);
  const [resolvedReviews, setResolvedReviews] = useState<ResolvedReview[]>([]);
  const [historyOpen, setHistoryOpen] = useState(false);

  /**
   * Session-scoped scan: returns all pending reviews for this session's
   * flow runs. Replaces the workspace-wide scan so reviews from other
   * sessions are never shown here.
   */
  const refreshAll = useCallback(async () => {
    if (!sessionId) return;
    const result: SessionPendingActionsResponse = await flowRun
      .getSessionPendingActions(sessionId)
      .catch(() => ({ doc_reviews: [] as ReviewItem[], approvals: [] }));
    for (const pr of result.doc_reviews) {
      if (pr.flow_run_id) flowRunIdsRef.current.add(pr.flow_run_id);
    }
    setReviews(result.doc_reviews);
  }, [sessionId]);

  // Fetch pending reviews from all known flow runs and merge into state
  const refresh = useCallback(async () => {
    const ids = Array.from(flowRunIdsRef.current);
    // Fall back to workspace-wide scan when we have no known run IDs
    if (ids.length === 0) {
      await refreshAll();
      return;
    }

    const results = await Promise.allSettled(
      ids.map((id) => flowRun.getPendingReviews(id).then((r) => ({ id, reviews: r.pending_reviews }))),
    );

    const merged: ReviewItem[] = [];
    for (const result of results) {
      if (result.status !== "fulfilled") continue;
      for (const pr of result.value.reviews) {
        merged.push(pr);
      }
    }

    setReviews(merged);
  }, [refreshAll]);

  // Remove an item optimistically from the local state and record in history
  const removeItem = useCallback(
    (item: ReviewItem, verdict: ReviewVerdict) => {
      setReviews((prev) =>
        prev.filter((r) => !(r.flow_run_id === item.flow_run_id && r.node_id === item.node_id && r.port === item.port)),
      );
      setResolvedReviews((prev) => [{ item, verdict, resolvedAt: Date.now() }, ...prev]);
      // Schedule a follow-up refresh to pick up any new reviews that might have been queued
      setTimeout(refresh, 1500);
    },
    [refresh],
  );

  // Listen for run_status / flow events to trigger refresh via session subscription.
  // Uses runtime.on("session:{id}") which delivers the inner event directly as
  // { sequence, emitted_at_ms, payload: { kind, ... } }.
  useEffect(() => {
    if (!sessionId) return;

    const unsubscribe = runtime.on(`session:${sessionId}`, (raw: unknown) => {
      const ev = raw as Record<string, unknown> | null;
      if (!ev) return;
      const pl = (ev.payload as Record<string, unknown> | undefined) ?? {};
      const kind = pl.kind as string | undefined;

      if (kind === "run_status") {
        // Any run_status (awaiting_review, succeeded, failed, cancelled) may
        // indicate new or resolved document reviews for this session.
        void refreshAll();
      }

      // flow.run.changed arrives as a Raw payload: { kind: "raw", data: { event: "flow.run.changed", ... } }
      if (kind === "raw") {
        const rawData = (pl.data as Record<string, unknown> | undefined) ?? {};
        const rawEvent = rawData.event as string | undefined;
        if (rawEvent === "flow.run.changed" || rawEvent === "flow_run_changed") {
          void refresh();
        }
        if (rawEvent === "session.pending_actions_ready") {
          void refreshAll();
        }
      } else if (kind === "flow.run.changed" || kind === "flow_run_changed") {
        // Future path where the event kind is promoted directly.
        void refresh();
      }
    });

    return () => unsubscribe?.();
  }, [sessionId, refresh, refreshAll]);

  // On mount (and when sessionId changes): do a session-scoped scan so the
  // panel shows pending reviews even after an app restart when no events
  // have fired and the in-memory activity snapshot is empty.
  useEffect(() => {
    if (!sessionId) return;
    refreshAll();
  }, [sessionId, refreshAll]);

  if (reviews.length === 0 && resolvedReviews.length === 0) return null;

  return (
    <div className="flex flex-col gap-2 px-3 py-2">
      {reviews.length > 0 && (
        <>
          <p className="text-xs font-medium text-muted-foreground px-0.5">Pending Reviews ({reviews.length})</p>
          {reviews.map((item) => (
            <ReviewCard
              key={`${item.flow_run_id}:${item.node_id}:${item.port}`}
              item={item}
              onApproved={(it) => removeItem(it, "approved")}
              onChangesRequested={(it) => removeItem(it, "changes_requested")}
            />
          ))}
        </>
      )}

      {resolvedReviews.length > 0 && (
        <Collapsible
          open={historyOpen}
          onOpenChange={setHistoryOpen}
          className="rounded-lg border border-border bg-card shadow-sm overflow-hidden"
        >
          <CollapsibleTrigger className="flex w-full items-center gap-1.5 px-0.5 py-1 text-left">
            <Clock className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            <span className="flex-1 text-xs font-medium text-muted-foreground">
              Review History ({resolvedReviews.length})
            </span>
            <ChevronDown
              className={`h-3.5 w-3.5 text-muted-foreground transition-transform duration-200 ${
                historyOpen ? "rotate-180" : ""
              }`}
            />
          </CollapsibleTrigger>
          <CollapsibleContent className="flex flex-col gap-1.5 pt-1">
            {resolvedReviews.map((entry, idx) => {
              const docName = entry.item.port || entry.item.node_id;
              const approved = entry.verdict === "approved";
              const timeLabel = new Date(entry.resolvedAt).toLocaleTimeString([], {
                hour: "2-digit",
                minute: "2-digit",
              });
              return (
                <div
                  key={`${entry.item.flow_run_id}:${entry.item.node_id}:${entry.item.port}:${idx}`}
                  className="flex items-center gap-2 rounded-md border border-border bg-card/50 px-3 py-1.5"
                >
                  {approved ? (
                    <Check className="h-3.5 w-3.5 shrink-0 text-emerald-500" />
                  ) : (
                    <MessageSquare className="h-3.5 w-3.5 shrink-0 text-amber-400" />
                  )}
                  <span className="flex-1 min-w-0 text-xs text-foreground/80 truncate">{docName}</span>
                  <Badge
                    variant="outline"
                    className={`text-xs px-1.5 py-0 h-4 font-normal shrink-0 ${
                      approved ? "border-emerald-500/40 text-emerald-500" : "border-amber-400/40 text-amber-400"
                    }`}
                  >
                    {approved ? "approved" : "changes requested"}
                  </Badge>
                  <span className="text-xs text-muted-foreground shrink-0">{timeLabel}</span>
                </div>
              );
            })}
          </CollapsibleContent>
        </Collapsible>
      )}
    </div>
  );
}
