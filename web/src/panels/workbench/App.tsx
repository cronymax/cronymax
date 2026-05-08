import { useCallback, useEffect, useState } from "react";
import {
  readParams,
  setMode,
  type WorkbenchMode,
  type WorkbenchParams,
} from "./url";
import { browser } from "@/shells/bridge";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";

import { CommentRail } from "./CommentRail";

/**
 * Top-level workbench shell. Renders a header with the doc title + mode
 * toggle and lazy-mounts the active mode's editor. Heavy editor bundles
 * (Milkdown, Monaco) are dynamically imported on first use of the mode
 * so that other panels never pay their cost.
 */
export function App() {
  const [params, setParams] = useState<WorkbenchParams>(() => readParams());

  // Re-read params on hashchange (deep links) and popstate.
  useEffect(() => {
    const onChange = () => setParams(readParams());
    window.addEventListener("hashchange", onChange);
    window.addEventListener("popstate", onChange);
    return () => {
      window.removeEventListener("hashchange", onChange);
      window.removeEventListener("popstate", onChange);
    };
  }, []);

  const switchMode = (next: WorkbenchMode) => {
    setMode(next);
    setParams(readParams());
  };

  if (!params.flow || !params.doc) {
    return (
      <div className="p-6 text-sm text-red-600">
        Missing <code>flow</code> or <code>doc</code> URL parameter.
      </div>
    );
  }

  return (
    <div className="flex h-screen flex-col bg-white text-gray-900">
      <Header
        flow={params.flow}
        doc={params.doc}
        mode={params.mode}
        onSwitch={switchMode}
      />
      <div className="flex min-h-0 flex-1">
        <main className="min-h-0 flex-1 overflow-hidden">
          <ModeView params={params} />
        </main>
        <RailHost params={params} />
      </div>
    </div>
  );
}

/**
 * Loads the active document's markdown so the rail can detect
 * orphaned comments (block_ids that no longer exist in the doc).
 * Re-fetches when document_event AppEvents fire for our flow.
 */
function RailHost({ params }: { params: WorkbenchParams }) {
  const [content, setContent] = useState<string>("");

  const refresh = useCallback(async () => {
    if (!params.flow || !params.doc) return;
    try {
      const res = (await browser.send("document.read", {
        flow: params.flow,
        name: params.doc,
      })) as { content: string };
      setContent(res.content ?? "");
    } catch {
      // Best-effort: leave content empty so all comments appear orphaned.
      setContent("");
    }
  }, [params.flow, params.doc]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useBridgeEvent("event", (raw: unknown) => {
    const evt = raw as {
      kind?: string;
      flow_id?: string;
      subject?: string;
    } | null;
    if (!evt) return;
    if (evt.flow_id && evt.flow_id !== params.flow) return;
    if (evt.kind === "document_event") void refresh();
  });

  const onJump = (blockId: string) => {
    const url = new URL(window.location.href);
    url.hash = `#block-${blockId}`;
    window.history.replaceState(null, "", url.toString());
    // Trigger a hashchange so editors re-scan.
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  };

  return (
    <CommentRail
      params={params}
      currentMarkdown={content}
      onJumpToBlock={onJump}
    />
  );
}

function Header({
  flow,
  doc,
  mode,
  onSwitch,
}: {
  flow: string;
  doc: string;
  mode: WorkbenchMode;
  onSwitch: (m: WorkbenchMode) => void;
}) {
  const tabClass = (m: WorkbenchMode) =>
    `px-3 py-1 text-xs font-medium rounded ${
      mode === m
        ? "bg-gray-900 text-white"
        : "bg-gray-100 text-gray-700 hover:bg-gray-200"
    }`;
  return (
    <header className="flex items-center justify-between border-b border-gray-200 px-4 py-2">
      <div className="text-sm">
        <span className="font-semibold">{doc}</span>
        <span className="ml-2 text-gray-500">· {flow}</span>
      </div>
      <div className="flex gap-1">
        <button
          className={tabClass("wysiwyg")}
          onClick={() => onSwitch("wysiwyg")}
        >
          WYSIWYG
        </button>
        <button
          className={tabClass("source")}
          onClick={() => onSwitch("source")}
        >
          Source
        </button>
        <button className={tabClass("diff")} onClick={() => onSwitch("diff")}>
          Diff
        </button>
      </div>
    </header>
  );
}

/**
 * Lazy-mounts the editor for the active mode. Each mode has its own
 * dynamic import so the bundle for one mode is never paid for another.
 */
function ModeView({ params }: { params: WorkbenchParams }) {
  if (params.mode === "wysiwyg") {
    return <LazyWysiwyg params={params} />;
  }
  if (params.mode === "source") {
    return <LazySource params={params} />;
  }
  return <LazyDiff params={params} />;
}

import { lazy, Suspense } from "react";

const LazyWysiwygInner = lazy(() =>
  import("./Editor").then((m) => ({ default: m.Editor })),
);
const LazySourceInner = lazy(() =>
  import("./SourceEditor").then((m) => ({ default: m.SourceEditor })),
);
const LazyDiffInner = lazy(() =>
  import("./DiffView").then((m) => ({ default: m.DiffView })),
);

function LazyWysiwyg({ params }: { params: WorkbenchParams }) {
  return (
    <Suspense fallback={<Loading label="Loading editor…" />}>
      <LazyWysiwygInner params={params} />
    </Suspense>
  );
}

function LazySource({ params }: { params: WorkbenchParams }) {
  return (
    <Suspense fallback={<Loading label="Loading source editor…" />}>
      <LazySourceInner params={params} />
    </Suspense>
  );
}

function LazyDiff({ params }: { params: WorkbenchParams }) {
  return (
    <Suspense fallback={<Loading label="Loading diff…" />}>
      <LazyDiffInner params={params} />
    </Suspense>
  );
}

function Loading({ label }: { label: string }) {
  return (
    <div className="flex h-full items-center justify-center text-sm text-gray-500">
      {label}
    </div>
  );
}
