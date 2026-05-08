import { browser } from "@/shells/bridge";
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
        : "bg-cronymax-float text-cronymax-title/70";

  const canReview = !!flowId && !!runId;

  return (
    <div className="self-start w-full max-w-[640px] rounded-lg border border-cronymax-border bg-cronymax-base p-3 text-sm">
      <div className="flex items-start justify-between gap-2">
        <div className="flex-1 min-w-0">
          <div className="font-mono text-xs opacity-70 truncate">
            {thread.doc_path ?? thread.doc_id}
          </div>
          <div className="mt-0.5 text-xs opacity-60">
            {thread.doc_type ?? "document"} · rev {thread.revision} ·{" "}
            {thread.producer ?? "unknown"}
          </div>
        </div>
        {verdict && (
          <span className={`rounded px-2 py-0.5 text-[10px] ${verdictBadge}`}>
            {verdict}
          </span>
        )}
      </div>
      <div className="mt-2 flex gap-2">
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
              <span className="font-mono opacity-70">{r.payload.reviewer}</span>{" "}
              · {r.payload.verdict}
              {r.payload.comment ? ` — ${r.payload.comment}` : ""}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

async function sendVerdict(
  thread: ThreadState,
  verdict: "approve" | "request_changes",
  flowId: string,
  runId: string,
) {
  const channel =
    verdict === "approve" ? "review.approve" : "review.request_changes";
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
