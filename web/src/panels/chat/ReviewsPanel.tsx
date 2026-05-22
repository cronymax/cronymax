/**
 * ReviewsPanel — shows pending tool-approval requests and document reviews
 * across all active runs. Surfaces both `pending_reviews` from the activity
 * snapshot AND any agent runs in `awaiting_review` status (the latter handles
 * the case where the snapshot's in-memory review list was lost after an app
 * restart while the run's persisted state still records it as waiting).
 */

import { Check, ShieldAlert, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Alert, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { browser, runtime, shells } from "@/shells/bridge";

interface PendingItem {
  /** Specific review ID (from pending_reviews). Null when only the run is known. */
  review_id: string | null;
  run_id: string;
  tool_name: string;
  args: unknown;
}

interface Props {
  sessionId: string | null | undefined;
}

function parseRunStatus(raw: unknown): string {
  if (typeof raw === "string") return raw;
  if (typeof raw === "object" && raw !== null) {
    const s = (raw as Record<string, unknown>).status;
    if (typeof s === "string") return s;
  }
  return "";
}

export function ReviewsPanel({ sessionId }: Props) {
  const [items, setItems] = useState<PendingItem[]>([]);

  const loadSnapshot = useCallback(() => {
    shells.browser.activity
      .snapshot()
      .then((resp: { runs?: unknown[]; pending_reviews?: unknown[] }) => {
        const rawRuns = (resp.runs ?? []).map((r) => r as Record<string, unknown>);

        // Build the set of run_ids that belong to the current session.
        // When sessionId is null/undefined we show nothing (no active chat).
        // Step 1: runs that are directly owned by this session.
        const directSessionRunIds = new Set<string>(
          rawRuns
            .filter((r) => sessionId && (r.session_id as string | undefined) === sessionId)
            .map((r) => r.id as string)
            .filter(Boolean),
        );
        // Step 2: also include sub-agent runs whose flow_run_id is a session
        // run (flow agents are spawned without a session_id but do have a
        // flow_run_id pointing back to the originating flow run).
        const sessionRunIds = new Set<string>(directSessionRunIds);
        for (const r of rawRuns) {
          const flowRunId = r.flow_run_id as string | undefined;
          if (flowRunId && directSessionRunIds.has(flowRunId)) {
            const id = r.id as string;
            if (id) sessionRunIds.add(id);
          }
        }

        // ── 1. Parse explicit pending reviews ──────────────────────────
        const rawReviews = (resp.pending_reviews ?? []).map((r) => r as Record<string, unknown>);
        const reviewItems: PendingItem[] = rawReviews
          .map((r): PendingItem => {
            // Shape: { id, run_id, request: { kind, tool, arguments? }, state }
            const req = (r.request as Record<string, unknown>) ?? {};
            return {
              review_id: (r.id as string) ?? (r.review_id as string) ?? null,
              run_id: (r.run_id as string) ?? "",
              // Rust stores the tool name under "tool" (not "tool_name")
              tool_name: (req.tool as string) ?? (req.tool_name as string) ?? "tool review",
              args: req.arguments ?? req.args ?? r.args ?? {},
            };
          })
          .filter((r) => r.review_id && sessionRunIds.has(r.run_id));

        // ── 2. Find runs in awaiting_review not already covered ────────
        const coveredRunIds = new Set(reviewItems.map((r) => r.run_id));
        const awaitingItems: PendingItem[] = rawRuns
          .filter((r) => {
            const status = parseRunStatus(r.status);
            const runId = (r.id as string) ?? "";
            return status === "awaiting_review" && runId && !coveredRunIds.has(runId) && sessionRunIds.has(runId);
          })
          .map(
            (r): PendingItem => ({
              review_id: null,
              run_id: (r.id as string) ?? "",
              tool_name: "document review",
              args: {},
            }),
          );

        setItems([...reviewItems, ...awaitingItems]);
      })
      .catch(() => undefined);
  }, [sessionId]);

  // Load on mount
  useEffect(() => {
    loadSnapshot();
  }, [loadSnapshot]);

  // Listen for live review events and run status changes
  useEffect(() => {
    if (!sessionId) return;
    const unsubscribe = runtime.on(`session:${sessionId}`, (event: unknown) => {
      const ev = event as Record<string, unknown>;
      if (!ev) return;
      const payload = ev.payload as Record<string, unknown> | undefined;
      if (!payload) return;
      const kind = payload.kind as string | undefined;

      // New review request — add directly without snapshot round-trip
      if (kind === "permission_request") {
        // Refresh snapshot so session filtering is applied correctly.
        loadSnapshot();
        return;
      }

      // Review resolved — remove it
      if (kind === "trace") {
        const trace = payload.trace as Record<string, unknown> | undefined;
        if (trace?.kind === "review_resolved") {
          const reviewId = trace.review_id as string | undefined;
          if (reviewId) {
            setItems((prev) => prev.filter((r) => r.review_id !== reviewId));
          }
        }
        return;
      }

      // Run status transition — refresh snapshot to catch awaiting_review runs
      if (kind === "run_status") {
        const rawStatus = payload.status;
        const status =
          typeof rawStatus === "string"
            ? rawStatus
            : typeof rawStatus === "object" && rawStatus !== null
              ? ((rawStatus as Record<string, unknown>).status as string | undefined)
              : undefined;
        if (status === "awaiting_review" || status === "succeeded" || status === "failed" || status === "cancelled") {
          loadSnapshot();
        }
      }
    });
    return () => unsubscribe?.();
  }, [sessionId, loadSnapshot]);

  if (items.length === 0) return null;

  const handleApprove = (item: PendingItem) => {
    browser.send("review.approve", { run_id: item.run_id, review_id: item.review_id ?? "" }).catch(() => undefined);
    setItems((prev) =>
      prev.filter((r) => (item.review_id ? r.review_id !== item.review_id : r.run_id !== item.run_id)),
    );
  };

  const handleReject = (item: PendingItem) => {
    browser
      .send("review.request_changes", { run_id: item.run_id, review_id: item.review_id ?? "" })
      .catch(() => undefined);
    setItems((prev) =>
      prev.filter((r) => (item.review_id ? r.review_id !== item.review_id : r.run_id !== item.run_id)),
    );
  };

  return (
    <Alert className="mb-1 text-xs">
      <ShieldAlert />
      <AlertTitle className="flex items-center gap-2">
        <span>Pending reviews</span>
        <Badge variant="outline">{items.length}</Badge>
      </AlertTitle>

      <div className="mt-2 flex flex-col gap-1">
        {items.map((item, idx) => (
          <div
            key={item.review_id ?? `${item.run_id}-${idx}`}
            className="flex items-center gap-2 rounded bg-muted/40 px-2 py-1"
          >
            <span className="max-w-[120px] truncate font-mono font-medium text-foreground">{item.tool_name}</span>
            {item.review_id ? (
              <span className="flex-1 truncate text-muted-foreground">{formatArgs(item.args)}</span>
            ) : (
              <span className="flex-1 truncate font-mono text-[10px] text-muted-foreground">
                run {item.run_id.slice(0, 12)}
              </span>
            )}
            <Button size="xs" onClick={() => handleApprove(item)}>
              <Check data-icon="inline-start" />
              Allow
            </Button>
            <Button size="xs" variant="destructive" onClick={() => handleReject(item)}>
              <X data-icon="inline-start" />
              Deny
            </Button>
          </div>
        ))}
      </div>
    </Alert>
  );
}

function formatArgs(args: unknown): string {
  if (!args || typeof args !== "object") return "";
  const obj = args as Record<string, unknown>;
  const key = ["path", "command", "query", "name", "url", "file"].find((k) => typeof obj[k] === "string");
  if (key) return String(obj[key]).slice(0, 80);
  try {
    return JSON.stringify(args).slice(0, 80);
  } catch {
    return "";
  }
}
