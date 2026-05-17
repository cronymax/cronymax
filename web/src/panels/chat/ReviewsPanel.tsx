/**
 * ReviewsPanel — shows pending tool-approval requests and document reviews
 * across all active runs. Surfaces both `pending_reviews` from the activity
 * snapshot AND any agent runs in `awaiting_review` status (the latter handles
 * the case where the snapshot's in-memory review list was lost after an app
 * restart while the run's persisted state still records it as waiting).
 */

import { useCallback, useEffect, useState } from "react";
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

export function ReviewsPanel({ sessionId: _sessionId }: Props) {
  const [items, setItems] = useState<PendingItem[]>([]);

  const loadSnapshot = useCallback(() => {
    shells.browser.activity
      .snapshot()
      .then((resp: { runs?: unknown[]; pending_reviews?: unknown[] }) => {
        // ── 1. Parse explicit pending reviews ──────────────────────────
        const rawReviews = (resp.pending_reviews ?? []).map((r) => r as Record<string, unknown>);
        const reviewItems: PendingItem[] = rawReviews
          .map((r): PendingItem => {
            // Shape: { id, run_id, request: { tool_name?, arguments? }, state }
            const req = (r.request as Record<string, unknown>) ?? {};
            return {
              review_id: (r.id as string) ?? (r.review_id as string) ?? null,
              run_id: (r.run_id as string) ?? "",
              tool_name: (req.tool_name as string) ?? (r.tool_name as string) ?? "tool review",
              args: req.arguments ?? req.args ?? r.args ?? {},
            };
          })
          .filter((r) => r.review_id);

        // ── 2. Find runs in awaiting_review not already covered ────────
        const coveredRunIds = new Set(reviewItems.map((r) => r.run_id));
        const rawRuns = (resp.runs ?? []).map((r) => r as Record<string, unknown>);
        const awaitingItems: PendingItem[] = rawRuns
          .filter((r) => {
            const status = parseRunStatus(r.status);
            const runId = (r.id as string) ?? "";
            return status === "awaiting_review" && runId && !coveredRunIds.has(runId);
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
  }, []);

  // Load on mount
  useEffect(() => {
    loadSnapshot();
  }, [loadSnapshot]);

  // Listen for live review events and run status changes
  useEffect(() => {
    const unsub = runtime.on("*", (event: unknown) => {
      const ev = event as Record<string, unknown>;
      if (!ev) return;
      const payload = ev.payload as Record<string, unknown> | undefined;
      if (!payload) return;
      const kind = payload.kind as string | undefined;

      // New review request — add directly without snapshot round-trip
      if (kind === "permission_request") {
        const reviewId = payload.review_id as string | undefined;
        const runId = payload.run_id as string | undefined;
        const req = (payload.request as Record<string, unknown> | undefined) ?? {};
        if (reviewId && runId) {
          setItems((prev) => {
            if (prev.some((r) => r.review_id === reviewId)) return prev;
            return [
              ...prev,
              {
                review_id: reviewId,
                run_id: runId,
                tool_name: (req.tool_name as string) ?? "tool review",
                args: req.arguments ?? req.args ?? {},
              },
            ];
          });
        }
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
    return () => unsub?.();
  }, [loadSnapshot]);

  if (items.length === 0) return null;

  const handleApprove = (item: PendingItem) => {
    const payload = item.review_id ? { review_id: item.review_id } : { run_id: item.run_id };
    browser.send("review.approve", payload).catch(() => undefined);
    setItems((prev) =>
      prev.filter((r) => (item.review_id ? r.review_id !== item.review_id : r.run_id !== item.run_id)),
    );
  };

  const handleReject = (item: PendingItem) => {
    const payload = item.review_id ? { review_id: item.review_id } : { run_id: item.run_id };
    browser.send("review.request_changes", payload).catch(() => undefined);
    setItems((prev) =>
      prev.filter((r) => (item.review_id ? r.review_id !== item.review_id : r.run_id !== item.run_id)),
    );
  };

  return (
    <div className="mx-3 mb-1 flex flex-col gap-1 rounded-md border border-amber-400/40 bg-amber-500/10 px-2 py-2">
      <div className="mb-1 text-xs font-semibold text-amber-600 dark:text-amber-400">
        Pending Reviews ({items.length})
      </div>
      {items.map((item, idx) => (
        <div
          key={item.review_id ?? `${item.run_id}-${idx}`}
          className="flex items-center gap-2 rounded bg-background/50 px-2 py-1 text-xs"
        >
          <span className="font-mono font-medium text-foreground truncate max-w-[120px]">{item.tool_name}</span>
          {item.review_id ? (
            <span className="flex-1 truncate text-muted-foreground">{formatArgs(item.args)}</span>
          ) : (
            <span className="flex-1 truncate text-muted-foreground font-mono text-[10px]">
              run {item.run_id.slice(0, 12)}
            </span>
          )}
          <button
            type="button"
            onClick={() => handleApprove(item)}
            className="shrink-0 rounded bg-green-500/20 px-2 py-0.5 text-green-700 dark:text-green-300 hover:bg-green-500/40 transition"
          >
            ✓ Allow
          </button>
          <button
            type="button"
            onClick={() => handleReject(item)}
            className="shrink-0 rounded bg-red-500/20 px-2 py-0.5 text-red-700 dark:text-red-300 hover:bg-red-500/40 transition"
          >
            ✗ Deny
          </button>
        </div>
      ))}
    </div>
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
