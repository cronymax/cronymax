import { DiffEditor } from "@monaco-editor/react";
import { useEffect, useState } from "react";

import { browser } from "@/shells/bridge";

import { stripBlockComments } from "./blockIds";
import type { WorkbenchParams } from "./url";

// ---------------------------------------------------------------------------
// Diff view (Group 6, change: document-wysiwyg).
//
// Side-by-side or inline Monaco DiffEditor over two revisions of the
// same document. URL params `from` / `to` select the revisions; if
// either is omitted we default to comparing `latest-1` vs `latest`.
//
// Block-marker comments are stripped from both sides before rendering
// so the diff focuses on visible content rather than marker churn.

interface DocumentReadResponse {
  revision: number;
  content: string;
}

interface DocumentListResponse {
  docs: Array<{ name: string; latest_revision: number }>;
}

export function DiffView({ params }: { params: WorkbenchParams }) {
  const [original, setOriginal] = useState<string | null>(null);
  const [modified, setModified] = useState<string | null>(null);
  const [fromRev, setFromRev] = useState<number>(0);
  const [toRev, setToRev] = useState<number>(0);
  const [error, setError] = useState<string | null>(null);
  const [renderSideBySide, setRenderSideBySide] = useState<boolean>(true);

  useEffect(() => {
    let cancelled = false;
    setOriginal(null);
    setModified(null);
    setError(null);

    async function load() {
      try {
        // Resolve from/to. When unset, list docs to find the latest
        // revision and default to (latest-1, latest).
        let from = params.from ?? 0;
        let to = params.to ?? 0;
        if (!from || !to) {
          const listed = (await browser.send("document.list", {
            flow: params.flow,
          })) as DocumentListResponse;
          const entry = listed.docs.find((d) => d.name === params.doc);
          const latest = entry?.latest_revision ?? 0;
          if (latest < 2) {
            if (cancelled) return;
            setError(`Diff requires at least two revisions; this document only has ${latest}.`);
            return;
          }
          from = from || latest - 1;
          to = to || latest;
        }
        const [a, b] = await Promise.all([
          browser.send("document.read", {
            flow: params.flow,
            name: params.doc,
            revision: from,
          }) as Promise<DocumentReadResponse>,
          browser.send("document.read", {
            flow: params.flow,
            name: params.doc,
            revision: to,
          }) as Promise<DocumentReadResponse>,
        ]);
        if (cancelled) return;
        setOriginal(stripBlockComments(a.content));
        setModified(stripBlockComments(b.content));
        setFromRev(a.revision);
        setToRev(b.revision);
      } catch (err) {
        if (cancelled) return;
        setError(err instanceof Error ? err.message : String(err));
      }
    }
    void load();
    return () => {
      cancelled = true;
    };
  }, [params.flow, params.doc, params.from, params.to]);

  if (error) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <div className="max-w-md rounded border border-red-200 bg-red-50 p-4 text-sm text-red-700">
          <div className="font-medium">Couldn't load diff</div>
          <div className="mt-1 text-xs text-red-600">{error}</div>
        </div>
      </div>
    );
  }

  if (original === null || modified === null) {
    return <div className="flex h-full items-center justify-center p-6 text-sm text-gray-500">Loading diff…</div>;
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-gray-200 bg-gray-50 px-3 py-1 text-xs text-gray-600">
        <div>
          rev {fromRev} → rev {toRev}
        </div>
        <button
          type="button"
          onClick={() => setRenderSideBySide((v) => !v)}
          className="rounded border border-gray-300 bg-white px-2 py-0.5 text-xs hover:bg-gray-100"
        >
          {renderSideBySide ? "Inline" : "Side-by-side"}
        </button>
      </div>
      <div className="flex-1 overflow-hidden">
        <DiffEditor
          height="100%"
          language="markdown"
          original={original}
          modified={modified}
          options={{
            readOnly: true,
            originalEditable: false,
            renderSideBySide,
            wordWrap: "on",
            minimap: { enabled: false },
            scrollBeyondLastLine: false,
            fontSize: 13,
          }}
        />
      </div>
    </div>
  );
}
