import { useEffect, useMemo, useRef } from "react";
import { flowRun } from "@/shells/runtime";
import { useEventStream } from "./hooks/useEventStream";
import { TextBubble } from "./components/TextBubble";
import { DocumentCard } from "./components/DocumentCard";
import { ReviewReply } from "./components/ReviewReply";
import { RunDivider } from "./components/RunDivider";
import { ErrorBanners } from "./components/ErrorBanner";
import { Composer } from "./components/Composer";

function readScopeFromUrl() {
  const p = new URLSearchParams(window.location.search);
  return {
    flow_id: p.get("flow") ?? p.get("flow_id") ?? "",
    run_id: p.get("run") ?? p.get("run_id") ?? undefined,
  };
}

export function App() {
  const scope = useMemo(readScopeFromUrl, []);
  const { state } = useEventStream(scope);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [state.timeline.length]);

  const renderedDocs = new Set<string>();

  return (
    <div className="flex h-screen flex-col bg-cronymax-body text-cronymax-title">
      <header className="flex items-center gap-2 border-b border-cronymax-border bg-cronymax-base px-3 py-2">
        <div className="text-sm font-medium">Channel</div>
        <div className="text-xs opacity-60 font-mono">
          {scope.flow_id || "(no flow)"}
          {scope.run_id ? ` · ${scope.run_id}` : ""}
        </div>
        <RunPill run={state.run} />
        <div className="flex-1" />
        {state.run.active && state.run.run_id && (
          <button
            type="button"
            className="rounded bg-red-700/60 px-2 py-1 text-xs hover:bg-red-700"
            onClick={async () => {
              try {
                await flowRun.cancel(state.run.run_id!);
              } catch (err) {
                // eslint-disable-next-line no-console
                console.error("[channel] flow.run.cancel failed", err);
              }
            }}
          >
            Cancel
          </button>
        )}
      </header>

      <ErrorBanners errors={state.errors} />

      <div ref={scrollRef} className="flex-1 overflow-y-auto px-3 py-2">
        <div className="mx-auto flex max-w-3xl flex-col gap-2">
          {state.loading && (
            <div className="self-center text-xs opacity-60">Loading…</div>
          )}
          {state.error && (
            <div className="self-center text-xs text-red-300">
              Failed to load: {state.error}
            </div>
          )}
          {state.timeline.map((e) => {
            if (e.kind === "text") {
              return <TextBubble key={e.id} event={e} />;
            }
            if (e.kind === "document_event") {
              if (renderedDocs.has(e.payload.doc_id)) return null;
              renderedDocs.add(e.payload.doc_id);
              const thread = state.threads.get(e.payload.doc_id);
              if (!thread) return null;
              return (
                <DocumentCard
                  key={e.payload.doc_id}
                  thread={thread}
                  flowId={scope.flow_id}
                  runId={scope.run_id ?? state.run.run_id ?? undefined}
                />
              );
            }
            if (e.kind === "review_event") {
              // Reviews already shown inside DocumentCard; show a compact log line too.
              return <ReviewReply key={e.id} event={e} />;
            }
            if (e.kind === "system") {
              return <RunDivider key={e.id} event={e} />;
            }
            if (e.kind === "agent_status") {
              return (
                <div
                  key={e.id}
                  className="self-start text-[11px] font-mono opacity-60"
                >
                  {e.agent_id ?? "agent"} · {e.payload.status}
                  {e.payload.reason ? ` (${e.payload.reason})` : ""}
                </div>
              );
            }
            if (e.kind === "handoff") {
              return (
                <div
                  key={e.id}
                  className="self-center text-[11px] font-mono opacity-70"
                >
                  {e.payload.from_agent} → {e.payload.to_agent} ·{" "}
                  {e.payload.port}
                </div>
              );
            }
            return null;
          })}
        </div>
      </div>

      <Composer flowId={scope.flow_id} runId={scope.run_id} />
    </div>
  );
}

function RunPill({
  run,
}: {
  run: { active: boolean; last_subkind: string | null };
}) {
  if (!run.last_subkind) return null;
  const cls = run.active
    ? "bg-emerald-700/40 text-emerald-200"
    : "bg-cronymax-float text-cronymax-title/70";
  return (
    <span className={`rounded px-2 py-0.5 text-[10px] uppercase ${cls}`}>
      {run.last_subkind.replace("_", " ")}
    </span>
  );
}
