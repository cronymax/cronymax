import { useEffect, useState } from "react";
import { Streamdown } from "streamdown";
import { browser, shells } from "@/shells/bridge";
import type { ThreadState } from "../hooks/useEventStream";

interface Props {
  thread: ThreadState;
  flowId: string;
  runId?: string;
}

export function DocumentCard({ thread, flowId, runId }: Props) {
  const lastReview = thread.reviews[thread.reviews.length - 1];
  const verdict = lastReview?.payload.verdict;
  const verdictBadge =
    verdict === "approve"
      ? "bg-green-700/40 text-green-200"
      : verdict === "request_changes"
        ? "bg-amber-700/40 text-amber-200"
        : "bg-card text-foreground/70";

  const canReview = !!flowId && !!runId;

  const [expanded, setExpanded] = useState(false);
  const [content, setContent] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  // Load the document content when the preview is expanded for the first time.
  useEffect(() => {
    if (!expanded) return;
    if (content !== null || loading) return;
    if (!flowId || !thread.doc_path) return;

    setLoading(true);
    setLoadError(null);
    shells.document
      .read({ flow: flowId, name: thread.doc_path })
      .then((res) => {
        setContent(res.content ?? "");
      })
      .catch((err: unknown) => {
        setLoadError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        setLoading(false);
      });
  }, [expanded, flowId, thread.doc_path, content, loading]);

  return (
    <div className="self-start w-full max-w-[640px] rounded-lg border border-border bg-background text-sm">
      {/* Header */}
      <div className="flex items-start justify-between gap-2 p-3">
        <div className="flex-1 min-w-0">
          <div className="font-mono text-xs opacity-70 truncate">{thread.doc_path ?? thread.doc_id}</div>
          <div className="mt-0.5 text-xs opacity-60">
            {thread.doc_type ?? "document"} · rev {thread.revision} · {thread.producer ?? "unknown"}
          </div>
        </div>
        <div className="flex items-center gap-2">
          {verdict && <span className={`rounded px-2 py-0.5 text-xs ${verdictBadge}`}>{verdict}</span>}
          <button
            type="button"
            className="rounded bg-card px-2 py-0.5 text-xs hover:bg-accent border border-border"
            onClick={() => setExpanded((v) => !v)}
          >
            {expanded ? "Hide preview" : "Preview"}
          </button>
        </div>
      </div>

      {/* Markdown preview */}
      {expanded && (
        <div className="border-t border-border px-3 py-2 max-h-[480px] overflow-y-auto">
          {loading && <div className="text-xs opacity-60 py-2">Loading…</div>}
          {loadError && <div className="text-xs text-red-400 py-2">{loadError}</div>}
          {!loading && !loadError && content !== null && (
            <div className="prose prose-invert prose-sm max-w-none text-foreground">
              <Streamdown mode="static">{content}</Streamdown>
            </div>
          )}
        </div>
      )}

      {/* Actions */}
      <div className="flex flex-col gap-1 px-3 pb-3">
        <div className="flex gap-2 mt-2">
          <button
            type="button"
            disabled={!canReview}
            className="rounded bg-green-700/60 px-2 py-1 text-xs hover:bg-green-700 disabled:opacity-40"
            onClick={() => sendVerdict(thread, "approve", flowId, runId!)}
          >
            Approve
          </button>
          <button
            type="button"
            disabled={!canReview}
            className="rounded bg-amber-700/60 px-2 py-1 text-xs hover:bg-amber-700 disabled:opacity-40"
            onClick={() => sendVerdict(thread, "request_changes", flowId, runId!)}
          >
            Request changes
          </button>
        </div>
        {thread.reviews.length > 0 && (
          <div className="mt-2 space-y-1">
            {thread.reviews.map((r) => (
              <div key={r.id} className="text-xs opacity-80">
                <span className="font-mono opacity-70">{r.payload.reviewer}</span> · {r.payload.verdict}
                {r.payload.comment ? ` — ${r.payload.comment}` : ""}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

async function sendVerdict(thread: ThreadState, verdict: "approve" | "request_changes", flowId: string, runId: string) {
  const channel = verdict === "approve" ? "review.approve" : "review.request_changes";
  try {
    await browser.send(channel, {
      flow: flowId,
      run_id: runId,
      name: thread.doc_path ?? thread.doc_id,
    });
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error(`[channel] ${channel} failed`, err);
  }
}
