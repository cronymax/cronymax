import { useCallback, useEffect, useRef, useState } from "react";
import Editor, { type OnMount } from "@monaco-editor/react";

import { bridge } from "@/bridge";

import { BLOCK_MARKER_REGEX } from "./blockIds";
import type { WorkbenchParams } from "./url";

// ---------------------------------------------------------------------------
// Source-mode editor (Group 4, change: document-wysiwyg).
//
// Plain-text Monaco view of the markdown including all `<!-- block:
// <uuid> -->` markers. Power-users edit raw bytes here; we deliberately
// do NOT run `assignMissingBlockIds` on save in source mode (the user
// is editing markers explicitly), matching the spec.
//
// Deep-link: `#block-<uuid>` scrolls Monaco to the line carrying the
// matching marker comment.

interface DocumentReadResponse {
  revision: number;
  content: string;
}

interface DocumentSubmitResponse {
  ok: true;
  revision: number;
  sha: string;
}

interface SaveStatus {
  state: "idle" | "saving" | "saved" | "error";
  message: string;
}

type IStandaloneCodeEditor = Parameters<OnMount>[0];

export function SourceEditor({ params }: { params: WorkbenchParams }) {
  const [content, setContent] = useState<string | null>(null);
  const [revision, setRevision] = useState<number>(0);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<SaveStatus>({
    state: "idle",
    message: "",
  });
  const editorRef = useRef<IStandaloneCodeEditor | null>(null);

  // Initial load.
  useEffect(() => {
    let cancelled = false;
    setContent(null);
    setError(null);
    void bridge
      .send("document.read", { flow: params.flow, name: params.doc })
      .then((res) => {
        if (cancelled) return;
        const r = res as DocumentReadResponse;
        setContent(r.content);
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
    const ed = editorRef.current;
    if (!ed) return;
    const text = ed.getValue();
    setStatus({ state: "saving", message: "Saving…" });
    try {
      const res = (await bridge.send("document.submit", {
        flow: params.flow,
        name: params.doc,
        content: text,
      })) as DocumentSubmitResponse;
      setRevision(res.revision);
      setStatus({ state: "saved", message: `Saved · rev ${res.revision}` });
      window.setTimeout(() => {
        setStatus((cur) =>
          cur.state === "saved" ? { state: "idle", message: "" } : cur,
        );
      }, 2500);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatus({ state: "error", message: `Save failed: ${msg}` });
    }
  }, [params.flow, params.doc, status.state]);

  // Cmd/Ctrl+S to save.
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      const isSave =
        (ev.metaKey || ev.ctrlKey) &&
        !ev.shiftKey &&
        ev.key.toLowerCase() === "s";
      if (!isSave) return;
      ev.preventDefault();
      void handleSave();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [handleSave]);

  const onMount: OnMount = useCallback(
    (ed) => {
      editorRef.current = ed;
      // Apply deep-link scroll once Monaco is ready.
      const blockId = params.blockId;
      if (!blockId) return;
      const model = ed.getModel();
      if (!model) return;
      const total = model.getLineCount();
      for (let line = 1; line <= total; line += 1) {
        const text = model.getLineContent(line);
        const m = BLOCK_MARKER_REGEX.exec(text);
        if (m && m[1] === blockId) {
          // Reveal a few lines below the marker (the actual block body).
          ed.revealLineInCenter(line + 1);
          ed.setPosition({ lineNumber: line + 1, column: 1 });
          break;
        }
      }
    },
    [params.blockId],
  );

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

  if (content === null) {
    return (
      <div className="flex h-full items-center justify-center p-6 text-sm text-gray-500">
        Loading…
      </div>
    );
  }

  return (
    <div className="relative flex h-full flex-col">
      <div className="flex-1 overflow-hidden">
        <Editor
          height="100%"
          defaultLanguage="markdown"
          defaultValue={content}
          onMount={onMount}
          options={{
            wordWrap: "on",
            minimap: { enabled: false },
            scrollBeyondLastLine: false,
            fontSize: 13,
            renderWhitespace: "selection",
          }}
        />
      </div>
      <div className="flex items-center justify-between border-t border-gray-200 bg-gray-50 px-3 py-1 text-xs text-gray-500">
        <div>rev {revision} · raw markdown</div>
        <div
          className={
            status.state === "error"
              ? "text-red-600"
              : status.state === "saved"
                ? "text-emerald-600"
                : "text-gray-500"
          }
        >
          {status.message}
        </div>
      </div>
    </div>
  );
}
