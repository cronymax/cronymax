import { defaultValueCtx, editorViewCtx, Editor as MilkdownEditor, rootCtx, serializerCtx } from "@milkdown/core";
import { commonmark } from "@milkdown/preset-commonmark";
import { gfm } from "@milkdown/preset-gfm";
import { Milkdown, MilkdownProvider, useEditor } from "@milkdown/react";
import { nord } from "@milkdown/theme-nord";
import { useCallback, useEffect, useRef, useState } from "react";

import { shells } from "@/shells/bridge";

import { assignMissingBlockIds, formatBlockMarker, parseBlockComments, withBlockIds } from "./blockIds";
import type { WorkbenchParams } from "./url";

// ---------------------------------------------------------------------------
// WYSIWYG editor (Group 3, change: document-wysiwyg).
//
// Wraps `@milkdown/react` with the CommonMark + GFM + Nord stack. The
// editor's source-of-truth is the markdown text returned by Milkdown's
// serializer; on every save we run `assignMissingBlockIds` against that
// markdown to ensure every top-level block carries a `<!-- block: <uuid>
// -->` marker before the bytes hit DocumentStore.
//
// **CommonMark + HTML comments**: the Milkdown CommonMark preset stores
// HTML blocks (including `<!-- ... -->` comments) verbatim as
// `html_block` ProseMirror nodes, and the serializer round-trips them
// unchanged. We therefore do not need a remark plugin to preserve the
// markers — the only place we touch them is in `assignMissingBlockIds`,
// applied to the serialized markdown right before submit.
//
// **Save flow** (Cmd/Ctrl+S):
//   1. Pull current markdown via `editor.action(ctx => serializer(state.doc))`.
//   2. Pipe through `assignMissingBlockIds`.
//   3. Send `document.submit { flow, name, content }`.
//   4. Surface a transient toast: "Saving…" → "Saved · rev N".
//
// **Deep-link**: on mount we look at `location.hash` for `#block-<uuid>`
// and scroll the matching DOM node into view with a brief pulse-ring.
// Because Milkdown does not natively attach `data-block-id` attributes,
// we walk the rendered html_block nodes and look for the one whose text
// contains the matching marker comment.

interface SaveStatus {
  state: "idle" | "saving" | "saved" | "error";
  message: string;
}

function MilkdownInner(props: { initialMarkdown: string; onReady: (getMd: () => string) => void }) {
  const { initialMarkdown, onReady } = props;

  useEditor((root) => {
    const editor = MilkdownEditor.make()
      .config((ctx) => {
        ctx.set(rootCtx, root);
        ctx.set(defaultValueCtx, initialMarkdown);
        // No-op pass-through; reserved for future ProseMirror-level
        // block-id wiring (see blockIds.ts).
        withBlockIds(ctx);
      })
      .config(nord)
      .use(commonmark)
      .use(gfm);

    void editor.create().then(() => {
      onReady(() =>
        editor.action((ctx) => {
          const view = ctx.get(editorViewCtx);
          const serializer = ctx.get(serializerCtx);
          return serializer(view.state.doc);
        }),
      );
    });

    return editor;
  }, []);

  return <Milkdown />;
}

export function Editor({ params }: { params: WorkbenchParams }) {
  const [initial, setInitial] = useState<string | null>(null);
  const [revision, setRevision] = useState<number>(0);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<SaveStatus>({
    state: "idle",
    message: "",
  });
  const getMarkdownRef = useRef<() => string>(() => "");

  // Initial load.
  useEffect(() => {
    let cancelled = false;
    setInitial(null);
    setError(null);
    void shells.document
      .read({ flow: params.flow, name: params.doc })
      .then((r) => {
        if (cancelled) return;
        setInitial(r.content);
        setRevision(r.revision);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [params.flow, params.doc]);

  const handleSave = useCallback(async () => {
    if (status.state === "saving") return;
    setStatus({ state: "saving", message: "Saving…" });
    try {
      const raw = getMarkdownRef.current();
      // Mint markers for any blocks the user added that don't have one.
      // Idempotent on existing markers — see web/test/blockids.test.ts.
      const content = assignMissingBlockIds(raw);
      const res = await shells.document.submit({
        flow: params.flow,
        name: params.doc,
        content,
      });
      setRevision(res.revision);
      setStatus({ state: "saved", message: `Saved · rev ${res.revision}` });
      window.setTimeout(() => {
        setStatus((cur) => (cur.state === "saved" ? { state: "idle", message: "" } : cur));
      }, 2500);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatus({ state: "error", message: `Save failed: ${msg}` });
    }
  }, [params.flow, params.doc, status.state]);

  // Cmd/Ctrl+S to save.
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      const isSave = (ev.metaKey || ev.ctrlKey) && !ev.shiftKey && ev.key.toLowerCase() === "s";
      if (!isSave) return;
      ev.preventDefault();
      void handleSave();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [handleSave]);

  // Deep-link: scroll to the block whose marker matches `#block-<uuid>`.
  // Milkdown does not attach `data-block-id` attributes, so we walk the
  // rendered DOM under `.milkdown` and find the html_block whose text
  // contains the marker comment, then scroll the *next* sibling into
  // view (the marker block itself renders as an empty container).
  useEffect(() => {
    if (!initial) return;
    const blockId = params.blockId;
    if (!blockId) return;
    const handle = window.setTimeout(() => {
      const root = document.querySelector(".milkdown");
      if (!root) return;
      const marker = formatBlockMarker(blockId);
      const candidates = root.querySelectorAll<HTMLElement>("*");
      for (const el of candidates) {
        if (el.children.length === 0 && el.textContent && el.textContent.includes(marker)) {
          const target = el.closest(
            "[data-type='html-block'], p, h1, h2, h3, h4, h5, h6, blockquote, pre, ul, ol, table",
          );
          const focusEl = (target?.nextElementSibling ?? target) as HTMLElement | null;
          if (focusEl) {
            focusEl.scrollIntoView({ behavior: "smooth", block: "center" });
            focusEl.classList.add("ring-2", "ring-emerald-400", "rounded");
            window.setTimeout(() => {
              focusEl.classList.remove("ring-2", "ring-emerald-400", "rounded");
            }, 1500);
          }
          return;
        }
      }
    }, 200);
    return () => window.clearTimeout(handle);
  }, [initial, params.blockId]);

  if (error) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <div className="max-w-md rounded border border-red-200 bg-red-50 p-4 text-sm text-red-700">
          <div className="font-medium">Couldn't load document</div>
          <div className="mt-1 text-xs text-red-600">{error}</div>
        </div>
      </div>
    );
  }

  if (initial === null) {
    return <div className="flex h-full items-center justify-center p-6 text-sm text-gray-500">Loading…</div>;
  }

  const markerCount = parseBlockComments(initial).length;

  return (
    <div className="relative flex h-full flex-col">
      <MilkdownProvider>
        <div className="flex-1 overflow-auto bg-white">
          <MilkdownInner
            initialMarkdown={initial}
            onReady={(getMd) => {
              getMarkdownRef.current = getMd;
            }}
          />
        </div>
      </MilkdownProvider>
      <div className="flex items-center justify-between border-t border-gray-200 bg-gray-50 px-3 py-1 text-xs text-gray-500">
        <div>
          rev {revision} · {markerCount} block{markerCount === 1 ? "" : "s"}
        </div>
        <div
          className={
            status.state === "error" ? "text-red-600" : status.state === "saved" ? "text-emerald-600" : "text-gray-500"
          }
        >
          {status.message}
        </div>
      </div>
    </div>
  );
}
