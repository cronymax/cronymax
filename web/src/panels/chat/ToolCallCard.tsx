import { useEffect, useState } from "react";
import type { ContentSegment } from "./store";

type ToolCallSegment = Extract<ContentSegment, { kind: "tool_call" }>;

interface Props {
  segment: ToolCallSegment;
}

function fmtDurationMs(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/** Animated spinner for running state. */
function Spinner() {
  const [frame, setFrame] = useState(0);
  const frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
  useEffect(() => {
    const id = setInterval(() => setFrame((f) => (f + 1) % frames.length), 80);
    return () => clearInterval(id);
  }, [frames.length]);
  return <span className="font-mono text-amber-400">{frames[frame]}</span>;
}

/**
 * Renders a tool call segment as an inline expandable card.
 *
 * - Running: tool name + animated spinner
 * - Done: tool name + ✓ glyph + duration (collapsed); args + result (expanded)
 * - Error: tool name + ✗ glyph; expanding shows error result
 */
export function ToolCallCard({ segment }: Props) {
  const [expanded, setExpanded] = useState(false);

  const isRunning = segment.status === "running";
  const isDone = segment.status === "done";
  const isError = segment.status === "error";

  const statusGlyph = isDone ? (
    <span className="text-green-400 font-mono text-[11px]">✓</span>
  ) : isError ? (
    <span className="text-red-400 font-mono text-[11px]">✗</span>
  ) : null;

  return (
    <div className="my-1 rounded border border-cronymax-border bg-cronymax-float overflow-hidden">
      {/* Card header — always visible */}
      <button
        type="button"
        disabled={isRunning}
        onClick={() => !isRunning && setExpanded((v) => !v)}
        className={`flex w-full items-center gap-2 px-2.5 py-1.5 text-left transition${
          isRunning ? " cursor-default" : " hover:bg-cronymax-border/20 cursor-pointer"
        }`}
      >
        {/* Running indicator */}
        {isRunning && <Spinner />}

        {/* Tool name */}
        <span className="flex-1 font-mono text-[11px] text-cronymax-caption truncate">{segment.tool}</span>

        {/* Status glyph */}
        {statusGlyph}

        {/* Duration */}
        {!isRunning && segment.durationMs != null && (
          <span className="shrink-0 font-mono text-[10px] text-cronymax-caption opacity-60">
            {fmtDurationMs(segment.durationMs)}
          </span>
        )}

        {/* Expand/collapse chevron */}
        {!isRunning && (
          <span className="shrink-0 text-[10px] text-cronymax-caption opacity-40">{expanded ? "▾" : "▸"}</span>
        )}
      </button>

      {/* Expanded args + result */}
      {expanded && !isRunning && (
        <div className="border-t border-cronymax-border px-2.5 py-2 space-y-2">
          <div>
            <div className="mb-0.5 text-[10px] font-semibold uppercase tracking-wide text-cronymax-caption opacity-60">
              Args
            </div>
            <pre className="max-h-[200px] overflow-y-auto rounded bg-cronymax-base px-2 py-1 font-mono text-[10px] text-cronymax-caption whitespace-pre-wrap break-all">
              {JSON.stringify(segment.args, null, 2)}
            </pre>
          </div>
          <div>
            <div className="mb-0.5 text-[10px] font-semibold uppercase tracking-wide text-cronymax-caption opacity-60">
              Result
            </div>
            <pre className="max-h-[200px] overflow-y-auto rounded bg-cronymax-base px-2 py-1 font-mono text-[10px] text-cronymax-caption whitespace-pre-wrap break-all">
              {segment.result != null ? JSON.stringify(segment.result, null, 2) : "(no result)"}
            </pre>
          </div>
        </div>
      )}
    </div>
  );
}
