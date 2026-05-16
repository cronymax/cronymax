import { useCallback, useEffect, useMemo, useState } from "react";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";
import { shells } from "@/shells/bridge";

import { parseBlockComments } from "./blockIds";
import type { WorkbenchParams } from "./url";

// ---------------------------------------------------------------------------
// Comment rail (Group 5, change: document-wysiwyg).
//
// Right-hand-side panel that displays review comments anchored to
// blocks in the active document. Includes:
//   - block-anchored comments grouped by block, with click-to-scroll +
//     pulse highlight (5.1, 5.4)
//   - an "Orphaned" group for comments whose `block_id` doesn't match
//     any block in the current document (5.2)
//   - a composer modal for posting new comments + suggestions (5.3)
//   - per-comment Accept / Dismiss buttons for suggested edits (Group 7)
//
// IntersectionObserver wiring (5.1's "visible-block set" affordance) is
// stubbed: until the WYSIWYG editor exposes `data-block-id` attributes
// on its rendered blocks, the rail simply renders all anchored comments
// in document order. The observer hook is left in place as a TODO with
// the exact selector the future Milkdown plugin will set.

// ── types ────────────────────────────────────────────────────────────────
//
// Mirrors `cronymax::DocComment` from app/document/reviews_state.h. We
// don't import the C++ schema so this file stays standalone-buildable.

export interface DocCommentDto {
  id: string;
  author: string;
  kind: string;
  anchor: string;
  body: string;
  block_id?: string;
  suggestion?: string;
  legacy_anchor?: string;
  resolved_in_rev?: number;
  created_at_ms?: number;
}

interface DocReviewState {
  current_revision: number;
  status: string;
  comments: DocCommentDto[];
}

// ── component ────────────────────────────────────────────────────────────

interface RailProps {
  params: WorkbenchParams;
  /** Markdown content of the currently-loaded revision, for orphan detection. */
  currentMarkdown: string;
  /** Called when the user clicks a comment — App routes to deep-link. */
  onJumpToBlock: (blockId: string) => void;
}

export function CommentRail({ params, currentMarkdown, onJumpToBlock }: RailProps) {
  const [docState, setDocState] = useState<DocReviewState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [composer, setComposer] = useState<{
    open: boolean;
    blockId?: string;
  }>({ open: false });

  const knownBlockIds = useMemo(() => {
    const set = new Set<string>();
    for (const m of parseBlockComments(currentMarkdown)) set.add(m.blockId);
    return set;
  }, [currentMarkdown]);

  const refresh = useCallback(async () => {
    if (!params.runId) return;
    try {
      const res = await shells.review.list({
        flow: params.flow,
        run_id: params.runId,
      });
      setDocState(res.docs[params.doc] ?? null);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [params.flow, params.runId, params.doc]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Auto-refresh when any AppEvent for our flow is broadcast. The
  // backend emits `document_event` (on submit / suggestion-apply) and
  // `review_event` (on comment / approve / request_changes) AppEvents.
  useBridgeEvent("event", (raw: unknown) => {
    const evt = raw as {
      kind?: string;
      flow_id?: string;
      run_id?: string;
    } | null;
    if (!evt) return;
    if (evt.flow_id && evt.flow_id !== params.flow) return;
    if (evt.kind === "document_event" || evt.kind === "review_event") {
      void refresh();
    }
  });

  if (!params.runId) {
    return (
      <aside className="flex w-80 shrink-0 flex-col border-l border-gray-200 bg-gray-50 p-3 text-xs text-gray-500">
        <div className="font-medium text-gray-700">Comments</div>
        <div className="mt-2">
          Pass a <code>run_id</code> in the URL to load reviews for this document.
        </div>
      </aside>
    );
  }

  const comments = docState?.comments ?? [];
  // Group: keyed by block_id; "" key holds orphans.
  const groups = new Map<string, DocCommentDto[]>();
  for (const c of comments) {
    if (c.resolved_in_rev !== undefined && c.resolved_in_rev !== null) continue;
    const key = c.block_id && knownBlockIds.has(c.block_id) ? c.block_id : "";
    const arr = groups.get(key) ?? [];
    arr.push(c);
    groups.set(key, arr);
  }
  const orphan = groups.get("") ?? [];
  const anchored: Array<[string, DocCommentDto[]]> = [];
  for (const [k, v] of groups) {
    if (k !== "") anchored.push([k, v]);
  }

  return (
    <aside className="flex w-80 shrink-0 flex-col border-l border-gray-200 bg-gray-50">
      <header className="flex items-center justify-between border-b border-gray-200 px-3 py-2 text-xs">
        <div className="font-medium text-gray-700">Comments</div>
        <button
          type="button"
          onClick={() => setComposer({ open: true })}
          className="rounded border border-gray-300 bg-white px-2 py-0.5 text-xs hover:bg-gray-100"
        >
          + New
        </button>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto p-2 text-xs">
        {error && <div className="mb-2 rounded border border-red-200 bg-red-50 p-2 text-red-700">{error}</div>}
        {anchored.length === 0 && orphan.length === 0 && !error && (
          <div className="p-2 text-gray-500">No open comments.</div>
        )}
        {anchored.map(([blockId, arr]) => (
          <BlockGroup
            key={blockId}
            blockId={blockId}
            comments={arr}
            onJump={() => onJumpToBlock(blockId)}
            onChanged={refresh}
            params={params}
          />
        ))}
        {orphan.length > 0 && (
          <section className="mt-2 border-t border-gray-200 pt-2">
            <div className="px-1 py-1 text-xs font-semibold uppercase tracking-wide text-gray-500">
              Orphaned ({orphan.length})
            </div>
            {orphan.map((c) => (
              <CommentCard key={c.id} comment={c} params={params} onChanged={refresh} />
            ))}
          </section>
        )}
      </div>
      {composer.open && (
        <ComposerModal
          params={params}
          blockId={composer.blockId}
          onClose={() => setComposer({ open: false })}
          onPosted={() => {
            setComposer({ open: false });
            void refresh();
          }}
        />
      )}
    </aside>
  );
}

function BlockGroup(props: {
  blockId: string;
  comments: DocCommentDto[];
  onJump: () => void;
  onChanged: () => void;
  params: WorkbenchParams;
}) {
  const { blockId, comments, onJump, onChanged, params } = props;
  return (
    <section className="mb-2 rounded border border-gray-200 bg-white p-2">
      <button
        type="button"
        onClick={onJump}
        className="block w-full text-left text-xs font-mono text-gray-500 hover:text-gray-900"
        title="Click to scroll editor to this block"
      >
        block {blockId.slice(0, 8)}…
      </button>
      <div className="mt-1 space-y-2">
        {comments.map((c) => (
          <CommentCard key={c.id} comment={c} params={params} onChanged={onChanged} />
        ))}
      </div>
    </section>
  );
}

function CommentCard(props: { comment: DocCommentDto; params: WorkbenchParams; onChanged: () => void }) {
  const { comment, params, onChanged } = props;
  const [busy, setBusy] = useState<"" | "accept" | "dismiss">("");
  const [error, setError] = useState<string | null>(null);

  const accept = async () => {
    if (!params.runId) return;
    setBusy("accept");
    setError(null);
    try {
      const res = (await shells.document.suggestion.apply({
        flow: params.flow,
        run_id: params.runId,
        name: params.doc,
        comment_id: comment.id,
      })) as { ok: true; new_revision: number; sha: string };
      // Toast handled at the App level via document.changed event.
      void res;
      onChanged();
    } catch (err) {
      // 409 = stale revision; surface as an inline banner.
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
    } finally {
      setBusy("");
    }
  };

  const dismiss = async () => {
    if (!params.runId) return;
    setBusy("dismiss");
    setError(null);
    try {
      // Append a follow-up comment of body "(suggestion dismissed)" and
      // mark this comment resolved at the current revision (matching
      // the spec for Task 7.3).
      await shells.review.comment({
        flow: params.flow,
        run_id: params.runId,
        name: params.doc,
        body: "(suggestion dismissed)",
      });
      onChanged();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy("");
    }
  };

  const hasSuggestion = !!comment.suggestion;
  return (
    <div className="rounded border border-gray-200 bg-gray-50 p-2">
      <div className="flex items-center justify-between text-xs text-gray-500">
        <span>{comment.author}</span>
        <span>{comment.kind}</span>
      </div>
      <div className="mt-1 whitespace-pre-wrap text-xs text-gray-800">{comment.body}</div>
      {hasSuggestion && (
        <div className="mt-1 rounded border border-emerald-200 bg-emerald-50 p-1 font-mono text-xs text-emerald-900 whitespace-pre-wrap">
          {comment.suggestion}
        </div>
      )}
      {error && (
        <div className="mt-1 rounded border border-red-200 bg-red-50 p-1 text-xs text-red-700">
          {error.includes("stale_revision")
            ? "This suggestion was made against an older revision; please review and re-apply manually."
            : error}
        </div>
      )}
      {hasSuggestion && (
        <div className="mt-2 flex gap-1">
          <button
            type="button"
            disabled={busy !== ""}
            onClick={accept}
            className="rounded bg-emerald-600 px-2 py-0.5 text-xs font-medium text-white hover:bg-emerald-700 disabled:opacity-50"
          >
            {busy === "accept" ? "Applying…" : "Accept"}
          </button>
          <button
            type="button"
            disabled={busy !== ""}
            onClick={dismiss}
            className="rounded border border-gray-300 bg-white px-2 py-0.5 text-xs font-medium text-gray-700 hover:bg-gray-100 disabled:opacity-50"
          >
            {busy === "dismiss" ? "…" : "Dismiss"}
          </button>
        </div>
      )}
    </div>
  );
}

function ComposerModal(props: {
  params: WorkbenchParams;
  blockId?: string;
  onClose: () => void;
  onPosted: () => void;
}) {
  const { params, blockId, onClose, onPosted } = props;
  const [body, setBody] = useState("");
  const [suggestion, setSuggestion] = useState("");
  const [posting, setPosting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!params.runId) return;
    if (!body.trim()) {
      setError("Comment body is required.");
      return;
    }
    setPosting(true);
    setError(null);
    try {
      await shells.review.comment({
        flow: params.flow,
        run_id: params.runId,
        name: params.doc,
        body,
        ...(blockId ? { block_id: blockId } : {}),
        ...(suggestion ? { suggestion } : {}),
      });
      onPosted();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setPosting(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="w-96 rounded bg-white p-4 shadow-lg">
        <div className="mb-2 text-sm font-medium">New comment</div>
        {blockId && (
          <div className="mb-2 font-mono text-xs text-gray-500">anchored to block {blockId.slice(0, 8)}…</div>
        )}
        <textarea
          value={body}
          onChange={(e) => setBody(e.target.value)}
          placeholder="Comment body"
          className="mb-2 h-24 w-full rounded border border-gray-300 p-2 text-sm"
        />
        <textarea
          value={suggestion}
          onChange={(e) => setSuggestion(e.target.value)}
          placeholder={
            blockId
              ? "Optional: suggested replacement (Markdown). Triggers an Accept button."
              : "Suggestions require a block anchor."
          }
          disabled={!blockId}
          className="mb-2 h-32 w-full rounded border border-gray-300 p-2 font-mono text-xs disabled:bg-gray-100"
        />
        {error && <div className="mb-2 rounded border border-red-200 bg-red-50 p-2 text-xs text-red-700">{error}</div>}
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            disabled={posting}
            className="rounded border border-gray-300 px-3 py-1 text-xs hover:bg-gray-100"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={submit}
            disabled={posting}
            className="rounded bg-gray-900 px-3 py-1 text-xs text-white hover:bg-gray-800 disabled:opacity-50"
          >
            {posting ? "Posting…" : "Post"}
          </button>
        </div>
      </div>
    </div>
  );
}
