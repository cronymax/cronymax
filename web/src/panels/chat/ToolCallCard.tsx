import { Check, ChevronDown, Loader2, X } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import type { ContentSegment } from "./store";

type ToolCallSegment = Extract<ContentSegment, { kind: "tool_call" }>;

interface Props {
  segment: ToolCallSegment;
}

function fmtDurationMs(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/**
 * Renders a tool call segment as an inline expandable card.
 *
 * - Running: tool name + animated spinner
 * - Done: tool name + ✓ glyph + duration (collapsed); args + result (expanded)
 * - Error: tool name + ✗ glyph; expanding shows error result
 */
export function ToolCallCard({ segment }: Props) {
  const [open, setOpen] = useState(false);

  const isRunning = segment.status === "running";
  const isDone = segment.status === "done";
  const isError = segment.status === "error";

  const statusIcon = isDone ? (
    <Check className="size-3 text-primary" />
  ) : isError ? (
    <X className="size-3 text-destructive" />
  ) : null;

  return (
    <Collapsible
      open={open && !isRunning}
      onOpenChange={(v) => !isRunning && setOpen(v)}
      className="my-1 overflow-hidden rounded border border-border bg-card"
    >
      <CollapsibleTrigger asChild disabled={isRunning}>
        <Button
          variant="ghost"
          className="flex h-auto w-full items-center justify-start gap-2 rounded-none px-2.5 py-1.5 hover:bg-muted/40"
        >
          {isRunning && <Loader2 className="size-3 animate-spin text-amber-500" />}
          <span className="flex-1 truncate text-left font-mono text-xs text-muted-foreground">{segment.tool}</span>
          {statusIcon}
          {!isRunning && segment.durationMs != null && (
            <span className="shrink-0 font-mono text-xs text-muted-foreground opacity-60">
              {fmtDurationMs(segment.durationMs)}
            </span>
          )}
          {!isRunning && (
            <ChevronDown
              className="size-3 shrink-0 text-muted-foreground opacity-40 transition-transform data-[state=closed]:-rotate-90"
              data-state={open ? "open" : "closed"}
            />
          )}
        </Button>
      </CollapsibleTrigger>

      <CollapsibleContent>
        <div className="flex flex-col gap-2 border-t border-border px-2.5 py-2">
          <div>
            <div className="mb-0.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground opacity-60">
              Args
            </div>
            <pre className="max-h-[200px] overflow-y-auto whitespace-pre-wrap break-all rounded bg-background px-2 py-1 font-mono text-xs text-muted-foreground">
              {JSON.stringify(segment.args, null, 2)}
            </pre>
          </div>
          <div>
            <div className="mb-0.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground opacity-60">
              Result
            </div>
            <pre className="max-h-[200px] overflow-y-auto whitespace-pre-wrap break-all rounded bg-background px-2 py-1 font-mono text-xs text-muted-foreground">
              {segment.result != null ? JSON.stringify(segment.result, null, 2) : "(no result)"}
            </pre>
          </div>
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
